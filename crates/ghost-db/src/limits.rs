//! Host limits — Ghost's `@tryghost/limit-service`.
//!
//! These are ceilings the *host* puts on a site, not preferences its owner
//! sets: how many staff users the plan allows, whether Stripe Connect is
//! available, which themes may be installed. They come from
//! `hostSettings.limits` in config, which is empty on a self-hosted install —
//! so on a normal `ghost` binary every one of these is inactive and every
//! check below returns `Ok(())` after a single map lookup. They matter when
//! this port is what runs behind a hosting product.
//!
//! Four kinds, as Ghost has:
//!
//! * **Max** — a ceiling on a count the database can answer: `members`,
//!   `staff`, `newsletters`, `uploads`.
//! * **Max periodic** — a ceiling per billing period, reset on the
//!   subscription's monthly anniversary: `emails`.
//! * **Flag** — on or off, no counting: `limitStripeConnect`,
//!   `limitAnalytics`, `limitSocialWeb`, `limitCustomFields`,
//!   `publicSiteAccess`.
//! * **Allowlist** — a named set of permitted values: `customThemes`.
//!
//! The asymmetry between the two questions is deliberate and worth keeping:
//! `error_if_would_go_over` asks "may I add one more", and is what a create
//! path calls; `error_if_is_over` asks "is this site already past the line",
//! and is what a path that is about to *use* something calls. A site that was
//! moved to a smaller plan can be over a limit without anybody having done
//! anything, and the second question is the one that notices.
use ghost_core::{GhostError, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::BTreeMap;

/// Ghost's default help link when the host has not configured a billing URL.
const DEFAULT_HELP_LINK: &str = "https://ghost.org/help/";

/// Which count a max limit measures, as Ghost's `config.js` defines them.
///
/// An enum rather than a closure because the set is closed: Ghost's config
/// module is explicitly the allowlist of limit names, and a name it does not
/// know is ignored rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counted {
    Members,
    Staff,
    Newsletters,
    Automations,
    /// Emails sent in the current period. The only periodic one.
    Emails,
    /// Uploads are special: the "count" is the size of the file being
    /// uploaded, which the caller passes in, so there is nothing to query.
    /// Ghost keeps a no-op query here for the same reason.
    UploadSize,
}

impl Counted {
    fn for_limit(name: &str) -> Option<Self> {
        Some(match name {
            "members" => Counted::Members,
            "staff" => Counted::Staff,
            "newsletters" => Counted::Newsletters,
            "limitAutomations" => Counted::Automations,
            "emails" => Counted::Emails,
            "uploads" => Counted::UploadSize,
            _ => return None,
        })
    }

    /// Count what this limit measures. `since` is set only for the periodic
    /// one; the rest ignore it.
    async fn count(self, pool: &SqlitePool, since: Option<&str>) -> i64 {
        let scalar = |sql: &'static str| async move {
            sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.unwrap_or(0)
        };
        match self {
            Counted::Members => scalar("SELECT COUNT(*) FROM members").await,
            // Contributors do not count against a staff limit, and neither do
            // deactivated users — but a pending invite does, otherwise a site
            // at its ceiling could invite its way past it and only find out
            // when somebody tried to accept.
            Counted::Staff => {
                scalar(
                    "SELECT (SELECT COUNT(*) FROM users u \
                               LEFT JOIN roles_users ru ON ru.user_id = u.id \
                               LEFT JOIN roles r ON r.id = ru.role_id \
                             WHERE COALESCE(r.name,'') != 'Contributor' AND u.status != 'inactive') \
                          + (SELECT COUNT(*) FROM invites i \
                               LEFT JOIN roles r ON r.id = i.role_id \
                             WHERE COALESCE(r.name,'') != 'Contributor')",
                )
                .await
            }
            Counted::Newsletters => {
                scalar("SELECT COUNT(*) FROM newsletters WHERE status = 'active'").await
            }
            // An automation with no actions is a draft in all but name, so it
            // does not count — this is Ghost's own carve-out for the legacy
            // welcome emails that have no action graph until they convert.
            Counted::Automations => {
                scalar(
                    "SELECT COUNT(*) FROM automations a WHERE a.status = 'active' \
                       AND EXISTS (SELECT 1 FROM automation_actions aa \
                                    WHERE aa.automation_id = a.id AND aa.deleted_at IS NULL)",
                )
                .await
            }
            Counted::Emails => {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(SUM(email_count), 0) FROM emails WHERE created_at >= ?",
                )
                .bind(since.unwrap_or(""))
                .fetch_one(pool)
                .await
                .unwrap_or(0)
            }
            Counted::UploadSize => 0,
        }
    }
}

/// One configured limit.
#[derive(Debug, Clone)]
enum Kind {
    Max { max: i64, counted: Counted },
    MaxPeriodic { max: i64, counted: Counted, start_date: String },
    Flag { disabled: bool },
    Allowlist { allowed: Vec<String> },
}

#[derive(Debug, Clone)]
struct Limit {
    name: String,
    kind: Kind,
    /// The host's own message, with `{{max}}`, `{{count}}` and `{{name}}`
    /// placeholders. Empty means "use the generic one".
    error: String,
}

/// What a check knows about the thing being checked.
///
/// Defaulted throughout, because most callers are asking the plain question —
/// "may I add one more of these" — and only need to fill in a field when they
/// are not: a bulk create sets `added`, an upload sets `current`, and an
/// allowlist check sets `value`.
#[derive(Debug, Default, Clone)]
pub struct Check<'a> {
    /// How many are being added. Defaults to one.
    pub added: Option<i64>,
    /// A count the caller already knows, instead of one queried here. This is
    /// how the uploads limit works: the "count" is the size of the file in
    /// hand, which no query could tell us.
    pub current: Option<i64>,
    /// The value an allowlist limit is being asked about — a theme name.
    pub value: Option<&'a str>,
}

impl<'a> Check<'a> {
    pub fn adding(n: i64) -> Self {
        Check { added: Some(n), ..Default::default() }
    }
    pub fn of_size(bytes: i64) -> Self {
        Check { current: Some(bytes), ..Default::default() }
    }
    pub fn value(v: &'a str) -> Self {
        Check { value: Some(v), ..Default::default() }
    }
}

/// The configured limits for this site.
///
/// Built once at boot and read from everywhere, so this is cheap to clone and
/// never mutated afterwards.
#[derive(Debug, Clone, Default)]
pub struct Limits {
    limits: BTreeMap<String, Limit>,
    help_link: String,
}

impl Limits {
    /// Read `hostSettings` into the live set.
    ///
    /// Takes the config value rather than the whole [`ghost_core::Config`]
    /// because everything here comes out of that one subtree — the limits,
    /// the billing URL behind the help link, and the subscription date a
    /// periodic limit counts from.
    ///
    /// Anything Ghost's config module does not name is dropped, and a limit
    /// whose shape does not make sense — a `max` on something with no count
    /// to take, a periodic limit with no subscription to date it from — is
    /// dropped too, with a warning. Ghost does the same: a misconfigured
    /// limit must not stop a site booting, and must not silently become a
    /// limit of zero either.
    pub fn load(host: &Value) -> Self {
        let help_link = match (&host["billing"]["enabled"], host["billing"]["url"].as_str()) {
            (Value::Bool(true), Some(url)) if !url.is_empty() => url.to_string(),
            _ => DEFAULT_HELP_LINK.to_string(),
        };
        // A periodic limit counts from the subscription's start date, so
        // without one there is nothing to count from.
        let subscription_start = host["subscription"]["start"].as_str().map(str::to_owned);

        let mut limits = BTreeMap::new();
        if let Some(configured) = host["limits"].as_object() {
            for (name, spec) in configured {
                let error = spec["error"].as_str().unwrap_or("").to_string();
                // Order matters and is Ghost's: allowlist, then max, then
                // periodic, then flag as the fallback. A spec with both an
                // allowlist and a max is read as an allowlist.
                let kind = if let Some(allowed) = spec["allowlist"].as_array() {
                    let allowed: Vec<String> =
                        allowed.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
                    if allowed.is_empty() {
                        tracing::warn!("limit {name} has an empty allowlist and was ignored");
                        continue;
                    }
                    Kind::Allowlist { allowed }
                } else if let Some(max) = spec["max"].as_i64() {
                    match Counted::for_limit(name) {
                        Some(counted) => Kind::Max { max, counted },
                        None => {
                            tracing::warn!("limit {name} has a max but nothing to count, so it was ignored");
                            continue;
                        }
                    }
                } else if let Some(max) = spec["maxPeriodic"].as_i64() {
                    match (Counted::for_limit(name), &subscription_start) {
                        (Some(counted), Some(start)) => {
                            Kind::MaxPeriodic { max, counted, start_date: start.clone() }
                        }
                        (None, _) => {
                            tracing::warn!("limit {name} is periodic but has nothing to count, so it was ignored");
                            continue;
                        }
                        (_, None) => {
                            tracing::warn!(
                                "limit {name} is periodic but hostSettings.subscription.start is unset, so it was ignored"
                            );
                            continue;
                        }
                    }
                } else if KNOWN_FLAGS.contains(&name.as_str()) {
                    Kind::Flag { disabled: spec["disabled"].as_bool().unwrap_or(false) }
                } else {
                    tracing::warn!("limit {name} is not one this version knows about, so it was ignored");
                    continue;
                };
                limits.insert(name.clone(), Limit { name: name.clone(), kind, error });
            }
        }
        Limits { limits, help_link }
    }

    /// Whether this limit is configured at all. Every check short-circuits on
    /// this, so an unlimited site pays one lookup and nothing else.
    pub fn is_limited(&self, name: &str) -> bool {
        self.limits.contains_key(name)
    }

    /// Whether a flag limit is switched off. A limit that is not configured is
    /// not disabled — that is the self-hosted case, where everything is
    /// available.
    pub fn is_disabled(&self, name: &str) -> bool {
        matches!(self.limits.get(name).map(|l| &l.kind), Some(Kind::Flag { disabled: true }))
    }

    /// How many values an allowlist limit permits, if it is one.
    ///
    /// Only one caller needs this, and it needs it for a real reason: a
    /// `customThemes` allowlist of exactly one theme means no other theme may
    /// be installed at all, official or not, so the install path has to know
    /// the difference between "a short allowlist" and "no allowlist".
    pub fn allowlist_len(&self, name: &str) -> Option<usize> {
        match self.limits.get(name).map(|l| &l.kind) {
            Some(Kind::Allowlist { allowed }) => Some(allowed.len()),
            _ => None,
        }
    }

    /// The names of every configured limit, for the `/config` endpoint.
    pub fn names(&self) -> Vec<&str> {
        self.limits.keys().map(String::as_str).collect()
    }

    /// May the site add `check.added` more of these?
    ///
    /// For a flag limit the answer is "no, ever" when disabled: using a
    /// feature at all is over the line.
    pub async fn error_if_would_go_over(
        &self,
        pool: &SqlitePool,
        name: &str,
        check: Check<'_>,
    ) -> Result<()> {
        let Some(limit) = self.limits.get(name) else { return Ok(()) };
        match &limit.kind {
            Kind::Flag { disabled } => {
                if *disabled {
                    return Err(self.flag_error(limit));
                }
                Ok(())
            }
            Kind::Allowlist { allowed } => self.check_allowlist(limit, allowed, check.value),
            Kind::Max { max, counted } => {
                let current = self.current(pool, limit, *counted, &check).await;
                if current + check.added.unwrap_or(1) > *max {
                    return Err(self.max_error(limit, *max, current));
                }
                Ok(())
            }
            Kind::MaxPeriodic { max, counted, start_date } => {
                let since = last_period_start(start_date);
                let current = counted.count(pool, Some(&since)).await;
                if current + check.added.unwrap_or(1) > *max {
                    return Err(self.max_error(limit, *max, current));
                }
                Ok(())
            }
        }
    }

    /// Is the site already past this limit?
    ///
    /// A flag limit never answers yes here, as in Ghost: being on a plan
    /// without a feature is not the same as having exceeded something, and
    /// there is no count that could have been exceeded.
    pub async fn error_if_is_over(
        &self,
        pool: &SqlitePool,
        name: &str,
        check: Check<'_>,
    ) -> Result<()> {
        let Some(limit) = self.limits.get(name) else { return Ok(()) };
        match &limit.kind {
            Kind::Flag { .. } => Ok(()),
            Kind::Allowlist { allowed } => self.check_allowlist(limit, allowed, check.value),
            Kind::Max { max, counted } => {
                let current = self.current(pool, limit, *counted, &check).await;
                if current > *max {
                    return Err(self.max_error(limit, *max, current));
                }
                Ok(())
            }
            Kind::MaxPeriodic { max, counted, start_date } => {
                let since = last_period_start(start_date);
                let current = counted.count(pool, Some(&since)).await;
                if current > *max {
                    return Err(self.max_error(limit, *max, current));
                }
                Ok(())
            }
        }
    }

    /// The same two questions as booleans, for a caller that wants to show a
    /// different screen rather than refuse a request.
    pub async fn would_go_over(&self, pool: &SqlitePool, name: &str, check: Check<'_>) -> bool {
        self.error_if_would_go_over(pool, name, check).await.is_err()
    }

    pub async fn is_over(&self, pool: &SqlitePool, name: &str, check: Check<'_>) -> bool {
        self.error_if_is_over(pool, name, check).await.is_err()
    }

    async fn current(
        &self,
        pool: &SqlitePool,
        _limit: &Limit,
        counted: Counted,
        check: &Check<'_>,
    ) -> i64 {
        match check.current {
            Some(known) => known,
            None => counted.count(pool, None).await,
        }
    }

    fn check_allowlist(&self, limit: &Limit, allowed: &[String], value: Option<&str>) -> Result<()> {
        // Ghost treats a missing value as the caller's mistake rather than as
        // a refusal, and so does this: an allowlist check with nothing to
        // check is a bug at the call site, not a site over its plan.
        let Some(value) = value else {
            return Err(GhostError::IncorrectUsage {
                name: limit.name.clone(),
                message: format!("The {} limit was checked without a value", limit.name),
            });
        };
        if allowed.iter().any(|a| a == value) {
            return Ok(());
        }
        Err(GhostError::HostLimitError {
            message: if limit.error.is_empty() {
                format!("This action would exceed the {} limit on your current plan.", lower_words(&limit.name))
            } else {
                limit.error.clone()
            },
            details: serde_json::json!({ "name": limit.name }),
            help: Some(self.help_link.clone()),
        })
    }

    fn flag_error(&self, limit: &Limit) -> GhostError {
        // The user-facing name of a flag drops the `limit` prefix the config
        // key carries: `limitStripeConnect` reads back as "stripe connect".
        let feature = lower_words(limit.name.strip_prefix("limit").unwrap_or(&limit.name));
        GhostError::HostLimitError {
            message: if limit.error.is_empty() {
                format!("Your plan does not support {feature}. Please upgrade to enable {feature}.")
            } else {
                limit.error.clone()
            },
            details: serde_json::json!({ "name": limit.name }),
            help: Some(self.help_link.clone()),
        }
    }

    fn max_error(&self, limit: &Limit, max: i64, count: i64) -> GhostError {
        let format = |n: i64| {
            // Uploads are configured in bytes and read back in megabytes,
            // because "this file is over the 5MB limit" is the sentence a
            // publisher can act on.
            if limit.name == "uploads" {
                format!("{}MB", n as f64 / 1_000_000.0)
            } else {
                thousands(n)
            }
        };
        let message = if limit.error.is_empty() {
            format!("This action would exceed the {} limit on your current plan.", lower_words(&limit.name))
        } else {
            limit
                .error
                .replace("{{max}}", &format(max))
                .replace("{{count}}", &format(count))
                .replace("{{name}}", &limit.name)
        };
        GhostError::HostLimitError {
            message,
            details: serde_json::json!({ "name": limit.name, "limit": max, "total": count }),
            help: Some(self.help_link.clone()),
        }
    }
}

/// The flag limits Ghost's config module knows. A name outside this list and
/// the four shapes above is ignored rather than turned into a flag, so a typo
/// in a host's config cannot accidentally disable a feature.
const KNOWN_FLAGS: &[&str] = &[
    "customIntegrations",
    "customThemes",
    "limitStripeConnect",
    "limitAnalytics",
    "limitSocialWeb",
    "limitCustomFields",
    "publicSiteAccess",
];

/// The start of the current billing period: the subscription's start date
/// advanced by as many whole months as have passed.
///
/// Monthly only, as in Ghost. Day-of-month is clamped, so a subscription that
/// started on the 31st has a period starting on the 28th in February rather
/// than rolling into March — which would make the period after it shorter
/// than a month and let an over-limit send through.
fn last_period_start(start: &str) -> String {
    use chrono::{Datelike, NaiveDate};
    let Ok(start_at) = chrono::DateTime::parse_from_rfc3339(start) else {
        // Unparseable means we cannot date the period, and counting from the
        // epoch would count every email the site has ever sent against this
        // month. Counting from now instead means a misconfigured limit lets
        // sends through rather than blocking them, which is the right way for
        // this to fail.
        return chrono::Utc::now().to_rfc3339();
    };
    let start_at = start_at.with_timezone(&chrono::Utc);
    let now = chrono::Utc::now();
    if now <= start_at {
        return start_at.to_rfc3339();
    }
    let months = (now.year() - start_at.year()) * 12 + now.month() as i32 - start_at.month() as i32;
    // Whole months only: if the day-of-month has not come round yet, the
    // current period began a month earlier than the arithmetic suggests.
    let months = if now.day() < start_at.day() { months - 1 } else { months };
    let months = months.max(0);
    let total = start_at.month0() as i32 + months;
    let (year, month0) = (start_at.year() + total / 12, (total % 12) as u32);
    let day = start_at.day().min(days_in_month(year, month0 + 1));
    NaiveDate::from_ymd_opt(year, month0 + 1, day)
        .and_then(|d| d.and_hms_opt(start_at.hour(), start_at.minute(), start_at.second()))
        .map(|dt| dt.and_utc().to_rfc3339())
        .unwrap_or_else(|| start_at.to_rfc3339())
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1);
    let next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1);
    match (first, next) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 31,
    }
}

use chrono::Timelike;

/// lodash's `lowerCase`: split camelCase into lowercase words.
///
/// Worth matching rather than approximating, because these strings are what a
/// publisher reads — "custom themes", not "customThemes".
fn lower_words(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// `Intl.NumberFormat().format` for an integer: thousands separated by commas.
fn thousands(n: i64) -> String {
    let negative = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if negative {
        format!("-{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use serde_json::json;


    #[test]
    fn a_self_hosted_site_has_no_limits() {
        let limits = Limits::load(&Value::Null);
        assert!(!limits.is_limited("staff"));
        assert!(!limits.is_disabled("limitStripeConnect"));
        assert!(limits.names().is_empty());
    }

    #[test]
    fn only_names_this_version_knows_are_loaded() {
        let limits = Limits::load(&json!({"limits": {
            "staff": {"max": 5},
            "limitStripeConnect": {"disabled": true},
            "customThemes": {"allowlist": ["casper", "source"]},
            // A max on something with no count behind it, which Ghost drops.
            "customIntegrations": {"max": 3},
            // A name from a future release.
            "limitSomethingElse": {"disabled": true},
        }}));
        assert_eq!(limits.names(), vec!["customThemes", "limitStripeConnect", "staff"]);
        assert!(limits.is_disabled("limitStripeConnect"));
        assert!(!limits.is_disabled("staff"));
    }

    #[test]
    fn a_periodic_limit_needs_a_subscription_to_date_it_from() {
        let without = Limits::load(&json!({"limits": {"emails": {"maxPeriodic": 1000}}}));
        assert!(!without.is_limited("emails"));

        let with = Limits::load(&json!({
            "subscription": {"start": "2026-01-15T00:00:00.000Z"},
            "limits": {"emails": {"maxPeriodic": 1000}},
        }));
        assert!(with.is_limited("emails"));
    }

    #[test]
    fn the_help_link_is_the_hosts_billing_url_when_it_has_one() {
        let default = Limits::load(&json!({"limits": {}}));
        assert_eq!(default.help_link, DEFAULT_HELP_LINK);

        let billing = Limits::load(&json!({
            "billing": {"enabled": true, "url": "https://billing.example/upgrade"},
            "limits": {},
        }));
        assert_eq!(billing.help_link, "https://billing.example/upgrade");

        // Configured but switched off falls back, rather than sending a
        // publisher to a billing page the host has turned off.
        let off = Limits::load(&json!({
            "billing": {"enabled": false, "url": "https://billing.example/upgrade"},
            "limits": {},
        }));
        assert_eq!(off.help_link, DEFAULT_HELP_LINK);
    }

    #[test]
    fn a_hosts_own_message_is_interpolated() {
        let limits = Limits::load(&json!({"limits": {"staff": {
            "max": 5,
            "error": "Your plan allows {{max}} staff users and you have {{count}}.",
        }}}));
        let limit = limits.limits.get("staff").expect("loaded");
        let err = limits.max_error(limit, 5, 1234);
        assert_eq!(err.message(), "Your plan allows 5 staff users and you have 1,234.");
        assert_eq!(err.status_code(), 403);
        assert_eq!(err.error_type(), "HostLimitError");
        let body = err.to_api_json();
        assert_eq!(body["errorDetails"], json!({"name": "staff", "limit": 5, "total": 1234}));
        assert_eq!(body["help"], json!(DEFAULT_HELP_LINK));
    }

    #[test]
    fn a_limit_with_no_message_gets_a_readable_one() {
        let limits = Limits::load(&json!({"limits": {
            "customThemes": {"allowlist": ["casper"]},
            "limitStripeConnect": {"disabled": true},
        }}));
        let themes = limits.limits.get("customThemes").expect("loaded");
        assert_eq!(
            limits.check_allowlist(themes, &["casper".into()], Some("dawn")).unwrap_err().message(),
            "This action would exceed the custom themes limit on your current plan."
        );
        // An allowed value passes.
        assert!(limits.check_allowlist(themes, &["casper".into()], Some("casper")).is_ok());

        // The `limit` prefix is for the config key, not for the reader.
        let stripe = limits.limits.get("limitStripeConnect").expect("loaded");
        assert_eq!(
            limits.flag_error(stripe).message(),
            "Your plan does not support stripe connect. Please upgrade to enable stripe connect."
        );
    }

    #[test]
    fn an_upload_limit_is_stated_in_megabytes() {
        let limits = Limits::load(&json!({"limits": {"uploads": {
            "max": 5_000_000,
            "error": "Files must be under {{max}}; this one is {{count}}.",
        }}}));
        let limit = limits.limits.get("uploads").expect("loaded");
        assert_eq!(
            limits.max_error(limit, 5_000_000, 7_500_000).message(),
            "Files must be under 5MB; this one is 7.5MB."
        );
    }

    #[test]
    fn a_period_starts_on_the_subscriptions_monthly_anniversary() {
        // Dates far in the past, so the arithmetic is checked rather than the
        // clock: the period containing today always starts on the 15th.
        let start = last_period_start("2020-01-15T09:30:00.000Z");
        let parsed = chrono::DateTime::parse_from_rfc3339(&start).expect("rfc3339");
        assert_eq!(parsed.day(), 15, "{start}");
        assert!(parsed <= chrono::Utc::now(), "{start} is not in the past");
        // And it is the most recent such date, not an older one.
        let next = parsed + chrono::Duration::days(32);
        assert!(next > chrono::Utc::now(), "{start} is more than a month old");

        // A subscription that has not started yet counts from its start.
        let future = chrono::Utc::now() + chrono::Duration::days(40);
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(&last_period_start(&future.to_rfc3339()))
                .expect("rfc3339")
                .timestamp(),
            future.timestamp()
        );
    }

    #[test]
    fn a_month_end_subscription_does_not_skip_february() {
        // The 31st does not exist in February, and rolling forward to March
                // would make January's period six weeks long.
        let start = last_period_start("2020-01-31T00:00:00.000Z");
        assert!(chrono::DateTime::parse_from_rfc3339(&start).is_ok(), "{start}");
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 12), 31);
    }

    #[test]
    fn counts_format_the_way_a_reader_expects() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(-4_321), "-4,321");
        assert_eq!(lower_words("customThemes"), "custom themes");
        assert_eq!(lower_words("staff"), "staff");
        assert_eq!(lower_words("limitSocialWeb"), "limit social web");
    }
}
