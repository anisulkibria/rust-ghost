//! The Security module's rules: the switches it owns, and the posture checks
//! it reports.
//!
//! Three halves, all pure, so the middleware that sets the headers, the
//! rate-limiter that decides whose address it is counting, the admin report
//! and the tests all answer the same questions the same way:
//!
//! * **[`flag`] and the tables around it** — the `security_*` settings, their
//!   types and their defaults. Unlike the SEO module, two of these defaults
//!   are deliberately *not* the behaviour the site had before: see
//!   "Why some defaults change behaviour" below.
//! * **[`response_headers`]** — exactly which response headers the admin and
//!   the public site send, derived from those settings. A pure function of
//!   the switches and the site's scheme, so the thing that is hardest to test
//!   in a browser is the thing that is easiest to test here.
//! * **[`audit`]** — the posture checks behind the Security report. Each one
//!   is a rule about a single fact of the site's configuration, with a
//!   severity and a sentence an administrator can act on, and a link to the
//!   thing that fixes it.
//!
//! Nothing here reads the database, knows what a session is, or touches an
//! HTTP type. It takes a [`Posture`] of already-loaded facts, which is what
//! makes the thresholds testable and keeps them in one place rather than
//! scattered through the page that renders them.
//!
//! ## This module is a trust boundary
//!
//! [`trust_proxy`] decides whether `X-Forwarded-For` is believed. Get that
//! wrong in the permissive direction on a directly-exposed server and every
//! brute-force budget in the application becomes advisory: an attacker sets a
//! fresh header value per request and each guess lands on a key nobody has
//! ever used. Get it wrong in the restrictive direction behind a reverse
//! proxy and one budget is shared by the whole internet, which locks the site
//! out of itself. There is no default that is right for both, which is why it
//! is a setting and why the report says out loud which way it is set.
//!
//! [`private_fetch_blocked`] is the other one: it decides whether the server
//! will fetch a URL that resolves inside its own network. See
//! `ghost_api::mentions::is_public_host` for the predicate itself, which this
//! only switches on and off.
//!
//! ## Why some defaults change behaviour
//!
//! The SEO module's rule is that installing it changes nothing until somebody
//! moves a switch. That rule is right for a module whose settings are
//! editorial preferences and wrong for one whose settings are defences: a
//! hardening switch that ships off is hardening nobody has.
//!
//! So the line drawn here is **can this break a working site**:
//!
//! * `security_headers` ships **on**. The headers it adds to admin responses
//!   (`nosniff`, `SAMEORIGIN`, `frame-ancestors 'self'`, a referrer policy)
//!   describe how the admin is already used. The admin is never framed by
//!   another origin and never relies on content sniffing, so there is no
//!   configuration in which these break it.
//! * `security_private_fetch` ships **blocked**. The only thing it stops is
//!   the server fetching a bookmark or embed from its own network, which the
//!   webmention receiver has refused to do since it shipped.
//! * `security_session_days` ships at **7**, which is the `Max-Age` the
//!   session cookie has always carried. A browser was already dropping the
//!   cookie at seven days; what changes is that the row behind it now expires
//!   too, so a session id copied out of the database stops working on the same
//!   schedule as the cookie it came from.
//! * `security_hsts` and `security_session_idle_minutes` ship **off**. HSTS is
//!   a promise a browser holds for two years and cannot be withdrawn quickly;
//!   an idle timeout signs people out of a tab they left open. Both are real
//!   improvements and both are somebody's choice, not ours.
//! * `security_trust_proxy` ships **on**, which *is* the old behaviour, and is
//!   the one case where the permissive default wins: turning it off on a site
//!   behind a proxy locks every visitor into one shared budget, and we cannot
//!   tell from inside the process whether there is a proxy. The report flags
//!   it instead.

use serde_json::Value;

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// Every `security_*` switch, with its default.
///
/// Read the module doc for why two of these are not the site's prior
/// behaviour. The tuple is `(key, default)`; the admin screen's labels live
/// with the screen, because they are copy and these are rules.
pub const BOOL_SETTINGS: &[(&str, bool)] = &[
    // `nosniff`, `X-Frame-Options: SAMEORIGIN`, `frame-ancestors 'self'` and
    // a referrer policy on admin responses. On: see the module doc.
    ("security_headers", true),
    // `Strict-Transport-Security` on an https site. Off: a browser holds the
    // promise for two years and a site that later loses its certificate is
    // unreachable, not merely insecure.
    ("security_hsts", false),
    // Believe `X-Forwarded-For`. On, because it is the old behaviour and the
    // restrictive default breaks a proxied site badly. The report flags it.
    ("security_trust_proxy", true),
    // Refuse to fetch a URL that resolves inside this server's own network —
    // link previews, embeds, and anything else that takes a URL and goes and
    // reads it.
    ("security_private_fetch", true),
    // Sign every staff account out of every *other* session when its password
    // changes. On, and matching what two of the three password paths already
    // did; the switch exists so the behaviour is visible, not so it is
    // optional in any sense we recommend.
    ("security_revoke_on_password_change", true),
];

/// How long a staff session may live, in days, however much it is used.
///
/// The offered values, and the default, which is the `Max-Age` the session
/// cookie has always carried. `0` is not offered: a session that never
/// expires is not a policy anybody should be able to choose from a dropdown.
pub const SESSION_DAYS: &[(&str, &str)] = &[
    ("1", "1 day"),
    ("7", "7 days (matches the session cookie)"),
    ("14", "14 days"),
    ("30", "30 days"),
    ("90", "90 days"),
];

pub const DEFAULT_SESSION_DAYS: i64 = 7;

/// How long a staff session may sit unused before it is dropped, in minutes.
///
/// `0` means no idle timeout, which is the default and the behaviour the site
/// had. The shortest offered is fifteen minutes: anything less signs people
/// out while they are reading something.
pub const SESSION_IDLE_MINUTES: &[(&str, &str)] = &[
    ("0", "Never — only the maximum age applies"),
    ("15", "15 minutes"),
    ("60", "1 hour"),
    ("240", "4 hours"),
    ("1440", "24 hours"),
    ("10080", "7 days"),
];

/// The referrer policy admin responses advertise, when the headers are on.
///
/// Narrower than the public site's list in [`crate::seo`] on purpose: an
/// admin URL can carry a post id or a member id in its path, and the only
/// values here are the ones that never send a path to another origin.
pub const ADMIN_REFERRER_POLICIES: &[(&str, &str)] = &[
    ("same-origin", "Send nothing to other sites (recommended)"),
    ("strict-origin-when-cross-origin", "Send only the origin to other sites"),
    ("no-referrer", "Never send a referrer, even within this site"),
];

pub const DEFAULT_ADMIN_REFERRER_POLICY: &str = "same-origin";

/// The shortest staff password the site will accept.
///
/// Ten is what every password path in the application already enforces, so it
/// is the default. The higher values exist because a site with one
/// administrator and a password manager has no reason to stop at ten.
pub const PASSWORD_LENGTHS: &[(&str, &str)] = &[
    ("10", "10 characters (Ghost's minimum)"),
    ("12", "12 characters"),
    ("16", "16 characters"),
    ("20", "20 characters"),
];

pub const DEFAULT_PASSWORD_LENGTH: i64 = 10;

/// Hard floor on the password length, whatever is stored.
///
/// A stored value below this is a corrupted or hand-edited row, and honouring
/// it would weaken every password path in the application at once. Clamping
/// rather than erroring, because the alternative is a site that cannot set a
/// password at all.
pub const PASSWORD_LENGTH_FLOOR: i64 = 10;
pub const PASSWORD_LENGTH_CEILING: i64 = 128;

/// `Strict-Transport-Security` max-age, in seconds. Two years, which is what
/// the preload list requires and what Ghost's own `force_ssl` sends.
pub const HSTS_MAX_AGE: i64 = 63_072_000;

/// The default for one boolean setting, or `false` for a key that is not one.
pub fn bool_default(key: &str) -> bool {
    BOOL_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, d)| *d).unwrap_or(false)
}

/// Read one boolean `security_*` setting out of a JSON object, falling back to
/// the key's default.
///
/// Both shapes are accepted because the settings table has held both over its
/// life: a real JSON `true`/`false`, and the string `"true"`/`"false"` that
/// older rows and form posts produce. A key that is absent, null or
/// unrecognised gets the *default* rather than `false` — which for this
/// module means a missing row leaves the defences on, not off.
pub fn flag(container: &Value, key: &str) -> bool {
    match container.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => bool_default(key),
        },
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or_else(|| bool_default(key)),
        _ => bool_default(key),
    }
}

/// Read a number setting, reduced to one of the values the screen offered.
///
/// Anything else — a hand-edited row, a forged post that got past the form —
/// becomes `default`. This is the same argument the SEO module's
/// `referrer_policy` makes: a value we never offered has never been thought
/// about, and a session lifetime nobody chose is worse than the one we did.
pub fn choice(container: &Value, key: &str, offered: &[(&str, &str)], default: i64) -> i64 {
    let raw = match container.get(key) {
        Some(Value::Number(n)) => n.as_i64().map(|i| i.to_string()),
        Some(Value::String(s)) => Some(s.trim().to_string()),
        _ => None,
    };
    raw.filter(|v| offered.iter().any(|(value, _)| value == v))
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

/// How long a session may live, in seconds.
pub fn session_max_age_secs(container: &Value) -> i64 {
    choice(container, "security_session_days", SESSION_DAYS, DEFAULT_SESSION_DAYS) * 86_400
}

/// How long a session may sit unused, in seconds. `0` means no idle timeout.
pub fn session_idle_secs(container: &Value) -> i64 {
    choice(container, "security_session_idle_minutes", SESSION_IDLE_MINUTES, 0) * 60
}

/// The shortest password the site accepts, clamped to a sane band whatever is
/// stored. See [`PASSWORD_LENGTH_FLOOR`].
pub fn min_password_length(container: &Value) -> usize {
    choice(container, "security_min_password_length", PASSWORD_LENGTHS, DEFAULT_PASSWORD_LENGTH)
        .clamp(PASSWORD_LENGTH_FLOOR, PASSWORD_LENGTH_CEILING) as usize
}

/// The admin's referrer policy, reduced to a value we offer.
pub fn admin_referrer_policy(container: &Value) -> &'static str {
    container
        .get("security_admin_referrer_policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .and_then(|v| ADMIN_REFERRER_POLICIES.iter().find(|(value, _)| *value == v).map(|(value, _)| *value))
        .unwrap_or(DEFAULT_ADMIN_REFERRER_POLICY)
}

/// Whether `X-Forwarded-For` and `X-Real-IP` are believed.
///
/// The one question in this module that the rate limiter asks on every
/// authentication attempt, which is why it is a named function rather than a
/// `flag` call at the call site: there is exactly one place to change what it
/// means, and exactly one name to grep for.
pub fn trust_proxy(container: &Value) -> bool {
    flag(container, "security_trust_proxy")
}

/// Whether the server refuses to fetch URLs inside its own network.
pub fn private_fetch_blocked(container: &Value) -> bool {
    flag(container, "security_private_fetch")
}

// ---------------------------------------------------------------------------
// response headers
// ---------------------------------------------------------------------------

/// Which surface a response is for. The two get different headers because
/// they have different jobs: the admin is a single-origin application nobody
/// should ever embed, and the public site is a publication whose embeds and
/// framed cards are the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Anything under `/ghost/`.
    Admin,
    /// The published site, its feeds and its public APIs.
    Public,
}

/// The security headers a response should carry, as `(name, value)` pairs.
///
/// Returns an empty list when `security_headers` is off, except for HSTS,
/// which is its own switch: an administrator who wants only the transport
/// promise and none of the rest can have exactly that.
///
/// `https` is whether *the site's own URL* is https, not whether this
/// particular request arrived over TLS. HSTS on an http site is either
/// ignored or, worse, honoured and locks the site out of itself, so it is
/// sent only when the site says it is https.
///
/// What is deliberately *not* here:
///
/// * A full `Content-Security-Policy` for the admin. The admin's pages carry
///   inline styles and inline event wiring throughout, so a real policy needs
///   either nonces on every page or `unsafe-inline`, and the second is a
///   policy that stops nothing. `frame-ancestors` is the one directive that
///   works regardless and it is here.
/// * `X-XSS-Protection`. Every browser that implemented it has removed it,
///   and the one value that was ever safe (`0`) is the default anyway.
pub fn response_headers(
    settings: &Value,
    surface: Surface,
    https: bool,
) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();

    if flag(settings, "security_headers") {
        // Both surfaces: a response's declared type is the only type it is.
        // The media library serves visitor-supplied files from the site's own
        // origin, so a `.txt` that a sniffer decides is HTML would be stored
        // XSS on the publication.
        out.push(("x-content-type-options", "nosniff".into()));

        match surface {
            Surface::Admin => {
                // Two spellings of one rule, because `X-Frame-Options` is
                // what an older browser obeys and `frame-ancestors` is what a
                // current one does. Clickjacking the admin is the attack:
                // the sign-in form and every destructive button in the
                // application are behind this origin.
                out.push(("x-frame-options", "SAMEORIGIN".into()));
                out.push(("content-security-policy", "frame-ancestors 'self'".into()));
                out.push(("referrer-policy", admin_referrer_policy(settings).into()));
                // No Flash, no Silverlight, no crossdomain.xml. Vestigial,
                // free, and a scanner will ask for it.
                out.push(("x-permitted-cross-domain-policies", "none".into()));
                // An admin page is a view of private data; a shared cache
                // holding one is the whole problem.
                out.push(("cache-control", "no-store, max-age=0".into()));
            }
            Surface::Public => {
                // Nothing framing-related: a publication's pages are embedded
                // on purpose, and its own Portal runs in an iframe. The
                // public site's referrer policy is the SEO module's setting,
                // which is about what other sites learn, and is already sent.
            }
        }
    }

    if flag(settings, "security_hsts") && https {
        out.push((
            "strict-transport-security",
            format!("max-age={HSTS_MAX_AGE}; includeSubDomains"),
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// the posture audit
// ---------------------------------------------------------------------------

/// How much a finding costs. Ordered so a sort puts the worst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing, costs nothing today.
    Note,
    /// A defence that is weaker than it should be.
    Warning,
    /// A defence that is absent or bypassable.
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "Critical",
            Severity::Warning => "Warning",
            Severity::Note => "Note",
        }
    }

    /// The stylesheet tint for the row's icon.
    pub fn tint(self) -> &'static str {
        match self {
            Severity::Critical => "red",
            Severity::Warning => "amber",
            Severity::Note => "slate",
        }
    }
}

/// One thing the report says about the site.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    /// A stable code, used as the row's anchor and in the tests. Never shown.
    pub code: &'static str,
    pub title: &'static str,
    /// One or two sentences saying what is true and what it costs. Plain
    /// text: the admin escapes it at the point of output.
    pub detail: String,
    /// Where the thing that fixes it lives, as `(label, path)`.
    pub fix: Option<(&'static str, &'static str)>,
}

/// Every fact the posture checks read, lifted out of the database and the
/// settings so [`audit`] is a pure function of them.
///
/// Deliberately flat and deliberately boring: a field here is something that
/// was *measured*, and the judgement about it is in [`audit`] and nowhere
/// else. Several of these are counts rather than lists because the report
/// shows a number and the list lives on the screen that owns it.
#[derive(Debug, Clone, Default)]
pub struct Posture {
    /// Whether the site's own URL is https.
    pub https: bool,
    /// Whether this is a development build, where http is expected.
    pub development: bool,
    pub headers_on: bool,
    pub hsts_on: bool,
    pub trust_proxy: bool,
    pub private_fetch_blocked: bool,
    pub mfa_required: bool,
    pub session_days: i64,
    pub session_idle_minutes: i64,
    pub min_password_length: usize,
    /// Staff accounts with the Owner or Administrator role.
    pub admin_count: i64,
    /// Staff accounts that are suspended but still hold a live session. Zero
    /// unless something has gone wrong — see the module's own doc.
    pub suspended_with_sessions: i64,
    /// Live staff sessions.
    pub session_count: i64,
    /// Live staff sessions older than `session_days`, which the expiry should
    /// have taken and has not.
    pub stale_sessions: i64,
    /// Staff invitations that have not been accepted and have not expired.
    pub open_invites: i64,
    /// Staff invitations past their expiry that are still in the table.
    pub expired_invites: i64,
    /// Admin API keys, which act as a staff user.
    pub admin_api_keys: i64,
    /// Content API keys, which read published content.
    pub content_api_keys: i64,
    /// Addresses currently serving a brute-force lockout.
    pub lockouts: i64,
    /// Whether the site is behind its access code.
    pub is_private: bool,
    /// Whether a custom integration can write (any admin key exists at all).
    pub webhooks: i64,
}

/// The posture checks, worst first.
///
/// Ordered by what it costs to get wrong rather than by subject, so the top of
/// the list is always the next thing to do. Two conventions worth knowing:
///
/// * A check that is *good news* produces no finding. The report draws the
///   green lines from the [`Posture`] directly; this function only ever says
///   what is wrong, which is what keeps "how many findings" a meaningful
///   number.
/// * No check here is phrased as an accusation. A private site, a site with
///   one administrator and a site with no API keys are all legitimate; where
///   the honest answer is "this is a choice, and here is what it costs", the
///   finding is a [`Severity::Note`] and says so.
pub fn audit(p: &Posture) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut add = |severity: Severity, code: &'static str, title: &'static str, detail: String, fix: Option<(&'static str, &'static str)>| {
        out.push(Finding { severity, code, title, detail, fix });
    };

    // --- transport -------------------------------------------------------
    // First, because every other defence in the list is carried over it. A
    // session cookie on a cleartext site is readable by anyone on the path,
    // and no setting on this page changes that.
    if !p.https && !p.development {
        add(
            Severity::Critical,
            "no_https",
            "This site is served over http",
            "The admin session cookie, every password typed into the sign-in form and every page of private \
             data travel in cleartext, readable by anything between the browser and this server. Nothing else \
             on this page can compensate for it."
                .into(),
            Some(("General settings", "/ghost/settings/general/")),
        );
    } else if p.https && !p.hsts_on {
        add(
            Severity::Warning,
            "no_hsts",
            "HSTS is off",
            "The site is https, but a browser arriving at the http URL will still try http first — and that \
             first request is the one an attacker on the network gets to answer. HSTS tells the browser never \
             to try again."
                .into(),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    if !p.headers_on {
        add(
            Severity::Warning,
            "no_headers",
            "Security response headers are off",
            "Admin pages are sent without `X-Frame-Options`, `frame-ancestors` or `nosniff`, so another site \
             can frame the admin and a mistyped content type can be reinterpreted as HTML."
                .into(),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    // --- the rate limiter's idea of who you are --------------------------
    if p.trust_proxy {
        add(
            Severity::Critical,
            "trust_proxy",
            "Forwarded client addresses are believed",
            "Every brute-force budget is counted per client address, and the address is read from the \
             `X-Forwarded-For` header. Behind a reverse proxy that is correct and necessary. Served directly \
             to the internet it means a caller can send a fresh header on every request and never spend a \
             budget at all — sign-in, the emailed code, password resets and the access code all become \
             unlimited. Turn it off unless there is a proxy in front of this server."
                .into(),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    // --- authentication --------------------------------------------------
    if !p.mfa_required {
        add(
            Severity::Warning,
            "no_mfa",
            "Two-factor sign-in is not required",
            "A staff password is the whole of a login. With this on, a correct password only buys an \
             unverified session until a six-digit code emailed to the account comes back."
                .into(),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    if p.min_password_length <= DEFAULT_PASSWORD_LENGTH as usize && p.admin_count > 1 {
        add(
            Severity::Note,
            "password_length",
            "Staff passwords may be ten characters",
            format!(
                "Ten is Ghost's own minimum and this site has {} accounts that can change settings. A longer \
                 minimum costs a password manager nothing.",
                p.admin_count,
            ),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    // --- sessions --------------------------------------------------------
    if p.suspended_with_sessions > 0 {
        add(
            Severity::Critical,
            "suspended_sessions",
            "A suspended account still holds a live session",
            format!(
                "{n} suspended staff {account} still {have} a session that has not been revoked. A suspension \
                 stops a new sign-in; it does not reach into a browser that is already signed in. Revoke the \
                 sessions below.",
                n = p.suspended_with_sessions,
                account = if p.suspended_with_sessions == 1 { "account" } else { "accounts" },
                have = if p.suspended_with_sessions == 1 { "has" } else { "have" },
            ),
            Some(("Sessions", "/ghost/security/#sessions")),
        );
    }

    if p.stale_sessions > 0 {
        add(
            Severity::Warning,
            "stale_sessions",
            "Sessions older than the maximum age are still in the table",
            format!(
                "{n} {session} older than {days} days. They are refused at every guard, so nobody is signed in \
                 on one — but the rows should have been swept and have not been.",
                n = p.stale_sessions,
                session = if p.stale_sessions == 1 { "session is" } else { "sessions are" },
                days = p.session_days,
            ),
            Some(("Sessions", "/ghost/security/#sessions")),
        );
    }

    if p.session_idle_minutes == 0 && p.session_days >= 7 {
        add(
            Severity::Note,
            "no_idle_timeout",
            "Sessions never time out from disuse",
            format!(
                "A session lives {days} days whether it is used or not, so a browser left signed in on a \
                 shared machine stays signed in for {days} days.",
                days = p.session_days,
            ),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    // --- the ways in that are not a password -----------------------------
    if p.expired_invites > 0 {
        add(
            Severity::Note,
            "expired_invites",
            "Expired staff invitations are still listed",
            format!(
                "{n} staff {invite} past {its} expiry. They cannot be redeemed, but they keep an address on \
                 the invitation list long after anybody meant to invite it.",
                n = p.expired_invites,
                invite = if p.expired_invites == 1 { "invitation is" } else { "invitations are" },
                its = if p.expired_invites == 1 { "its" } else { "their" },
            ),
            Some(("Staff", "/ghost/staffs/")),
        );
    }

    if p.admin_api_keys > 0 {
        add(
            Severity::Note,
            "admin_api_keys",
            "Admin API keys can do everything a staff account can",
            format!(
                "{n} Admin API {key} in use. Each one signs its own tokens and is not covered by two-factor \
                 sign-in, a session expiry or a password change — the only way to withdraw one is to delete \
                 or regenerate it.",
                n = p.admin_api_keys,
                key = if p.admin_api_keys == 1 { "key is" } else { "keys are" },
            ),
            Some(("Integrations", "/ghost/integrations/")),
        );
    }

    if !p.private_fetch_blocked {
        add(
            Severity::Critical,
            "private_fetch",
            "The server will fetch URLs inside its own network",
            "Bookmark and embed cards have the server read a URL an author typed. With this off, that URL may \
             be `http://169.254.169.254/` or a database on localhost, and part of the answer comes back into \
             the editor. Any signed-in staff member, down to a Contributor, can ask."
                .into(),
            Some(("Security settings", "/ghost/settings/security/")),
        );
    }

    if p.admin_count == 1 {
        add(
            Severity::Note,
            "single_admin",
            "One account can change everything",
            "There is one Owner or Administrator. That is a reasonable way to run a site and it is also a \
             single account whose compromise is the whole site, with nobody else able to lock it out."
                .into(),
            Some(("Staff", "/ghost/staffs/")),
        );
    }

    out.sort_by(|a, b| b.severity.cmp(&a.severity));
    out
}

/// A single number for the top of the report: 100 minus what the findings
/// cost, floored at zero.
///
/// Weights, not a formula anybody should read meaning into beyond the
/// ordering: a critical finding is a defence that is absent or bypassable, a
/// warning is one that is weaker than it should be, and a note costs nothing
/// today. The number exists so a change is visible at a glance; the findings
/// are the content.
pub fn score(findings: &[Finding]) -> i64 {
    let cost: i64 = findings
        .iter()
        .map(|f| match f.severity {
            Severity::Critical => 25,
            Severity::Warning => 10,
            Severity::Note => 2,
        })
        .sum();
    (100 - cost).max(0)
}

/// The word for a score, for the ring's label and its tint.
pub fn grade(score: i64) -> (&'static str, &'static str) {
    match score {
        90..=100 => ("Strong", "green"),
        70..=89 => ("Reasonable", "blue"),
        40..=69 => ("Weak", "amber"),
        _ => ("At risk", "red"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The settings of a site that has never seen this module: no rows at all.
    fn absent() -> Value {
        json!({})
    }

    /// A site with every switch at its secure setting.
    fn hardened() -> Value {
        json!({
            "security_headers": true,
            "security_hsts": true,
            "security_trust_proxy": false,
            "security_private_fetch": true,
            "security_session_days": "7",
            "security_session_idle_minutes": "240",
            "security_min_password_length": "16",
            "security_admin_referrer_policy": "same-origin",
        })
    }

    #[test]
    fn a_site_with_no_security_rows_still_has_the_defences_on() {
        // The whole argument for defaulting `flag` to the key's default
        // rather than to false: a database that predates the module must not
        // read as "everything off".
        let s = absent();
        assert!(flag(&s, "security_headers"));
        assert!(private_fetch_blocked(&s));
        assert!(trust_proxy(&s), "the one permissive default, and it is deliberate");
        assert!(!flag(&s, "security_hsts"));
        assert_eq!(session_max_age_secs(&s), 7 * 86_400);
        assert_eq!(session_idle_secs(&s), 0);
        assert_eq!(min_password_length(&s), 10);
    }

    #[test]
    fn a_forged_session_lifetime_collapses_to_the_default() {
        // These come back from a form post. A lifetime nobody offered has
        // never been thought about; a password floor nobody offered could be
        // one character.
        for hostile in ["99999", "0", "-1", "7; DROP TABLE sessions", "", "7.5"] {
            let s = json!({ "security_session_days": hostile, "security_min_password_length": hostile });
            assert_eq!(session_max_age_secs(&s), 7 * 86_400, "{hostile}");
            assert_eq!(min_password_length(&s), 10, "{hostile}");
        }
    }

    #[test]
    fn the_password_floor_holds_against_a_hand_edited_row() {
        // `choice` would already refuse "4", but the clamp is the belt: this
        // value reaches every password path in the application.
        let s = json!({ "security_min_password_length": 4 });
        assert_eq!(min_password_length(&s), 10);
    }

    #[test]
    fn an_unoffered_referrer_policy_does_not_reach_a_response() {
        for hostile in ["unsafe-url", "", "same-origin\r\nSet-Cookie: x=1", "nonsense"] {
            let s = json!({ "security_admin_referrer_policy": hostile });
            assert_eq!(admin_referrer_policy(&s), "same-origin", "{hostile}");
        }
        let s = json!({ "security_admin_referrer_policy": "no-referrer" });
        assert_eq!(admin_referrer_policy(&s), "no-referrer");
    }

    #[test]
    fn admin_responses_refuse_to_be_framed_and_are_never_cached() {
        let headers = response_headers(&absent(), Surface::Admin, true);
        let get = |name: &str| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str());
        assert_eq!(get("x-frame-options"), Some("SAMEORIGIN"));
        assert_eq!(get("content-security-policy"), Some("frame-ancestors 'self'"));
        assert_eq!(get("x-content-type-options"), Some("nosniff"));
        assert_eq!(get("referrer-policy"), Some("same-origin"));
        assert_eq!(get("cache-control"), Some("no-store, max-age=0"));
    }

    #[test]
    fn the_public_site_is_not_told_it_cannot_be_framed() {
        // A publication's pages are embedded on purpose and Portal is an
        // iframe. Only the sniffing rule is shared.
        let headers = response_headers(&absent(), Surface::Public, true);
        let names: Vec<&str> = headers.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"x-content-type-options"));
        assert!(!names.contains(&"x-frame-options"));
        assert!(!names.contains(&"content-security-policy"));
    }

    #[test]
    fn the_public_site_is_told_not_to_sniff_uploaded_files() {
        // The reason the public surface gets anything at all: `/content/files/`,
        // `/content/media/` and `/content/images/` serve files somebody
        // uploaded, from the site's own origin. A `.txt` that a sniffing
        // browser decides is HTML is stored XSS on the publication.
        let headers = response_headers(&absent(), Surface::Public, true);
        assert_eq!(
            headers.iter().find(|(n, _)| *n == "x-content-type-options").map(|(_, v)| v.as_str()),
            Some("nosniff"),
        );
    }

    #[test]
    fn hsts_is_never_sent_over_cleartext() {
        // Honoured, it would make an http-only site unreachable rather than
        // merely insecure.
        let s = json!({ "security_hsts": true });
        let names = |https| -> Vec<&str> {
            response_headers(&s, Surface::Admin, https).into_iter().map(|(n, _)| n).collect()
        };
        assert!(names(true).contains(&"strict-transport-security"));
        assert!(!names(false).contains(&"strict-transport-security"));
    }

    #[test]
    fn hsts_can_be_had_without_the_rest() {
        let s = json!({ "security_headers": false, "security_hsts": true });
        let headers = response_headers(&s, Surface::Admin, true);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "strict-transport-security");
    }

    #[test]
    fn turning_the_headers_off_sends_nothing() {
        let s = json!({ "security_headers": false, "security_hsts": false });
        assert!(response_headers(&s, Surface::Admin, true).is_empty());
        assert!(response_headers(&s, Surface::Public, true).is_empty());
    }

    /// The posture of a site that has done everything right.
    fn good_posture() -> Posture {
        Posture {
            https: true,
            headers_on: true,
            hsts_on: true,
            trust_proxy: false,
            private_fetch_blocked: true,
            mfa_required: true,
            session_days: 7,
            session_idle_minutes: 240,
            min_password_length: 16,
            admin_count: 2,
            ..Posture::default()
        }
    }

    #[test]
    fn a_hardened_site_has_nothing_to_report() {
        let findings = audit(&good_posture());
        assert!(findings.is_empty(), "unexpected findings: {:?}", findings.iter().map(|f| f.code).collect::<Vec<_>>());
        assert_eq!(score(&findings), 100);
        assert_eq!(grade(100).0, "Strong");
    }

    #[test]
    fn cleartext_and_a_forwarded_header_are_both_critical() {
        // The two findings that make everything else on the page advisory,
        // and the two that must be at the top of it.
        let p = Posture { https: false, trust_proxy: true, ..good_posture() };
        let findings = audit(&p);
        let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
        assert!(codes.contains(&"no_https"));
        assert!(codes.contains(&"trust_proxy"));
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[1].severity, Severity::Critical);
    }

    #[test]
    fn a_development_site_is_not_scolded_for_http() {
        let p = Posture { https: false, development: true, ..good_posture() };
        let codes: Vec<&str> = audit(&p).iter().map(|f| f.code).collect();
        assert!(!codes.contains(&"no_https"));
        // And it is not told to turn on HSTS it cannot use either.
        assert!(!codes.contains(&"no_hsts"));
    }

    #[test]
    fn a_suspended_account_holding_a_session_is_the_worst_finding() {
        let p = Posture { suspended_with_sessions: 1, ..good_posture() };
        let findings = audit(&p);
        assert_eq!(findings[0].code, "suspended_sessions");
        assert_eq!(findings[0].severity, Severity::Critical);
        // Singular, because the plural reads as a bug report about itself.
        assert!(findings[0].detail.contains("1 suspended staff account still has"));
    }

    #[test]
    fn findings_are_ordered_worst_first() {
        let p = Posture {
            https: false,
            headers_on: false,
            trust_proxy: true,
            private_fetch_blocked: false,
            mfa_required: false,
            admin_count: 1,
            min_password_length: 10,
            session_days: 7,
            session_idle_minutes: 0,
            ..Posture::default()
        };
        let findings = audit(&p);
        let severities: Vec<Severity> = findings.iter().map(|f| f.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(severities, sorted);
        // Not an exact number — the weights are an ordering, not a formula —
        // but a site with three bypassable defences cannot read as anything
        // other than the worst grade.
        assert!(score(&findings) < 40, "scored {}", score(&findings));
        assert_eq!(grade(score(&findings)).0, "At risk");
    }

    #[test]
    fn one_administrator_is_a_note_and_not_a_mistake() {
        // The report's job is to say what is true, not to argue. A one-person
        // site is a legitimate way to run a publication.
        let p = Posture { admin_count: 1, ..good_posture() };
        let findings = audit(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "single_admin");
        assert_eq!(findings[0].severity, Severity::Note);
    }

    #[test]
    fn the_score_never_goes_below_zero() {
        let findings = vec![
            Finding { severity: Severity::Critical, code: "a", title: "", detail: String::new(), fix: None };
            10
        ];
        assert_eq!(score(&findings), 0);
    }

    #[test]
    fn every_finding_that_can_be_fixed_links_to_the_thing_that_fixes_it() {
        // Rule 8's intent: no link leads to a refusal, and no finding leads
        // nowhere. Every path named here is an admin page an administrator
        // can open.
        let p = Posture {
            https: false,
            trust_proxy: true,
            private_fetch_blocked: false,
            mfa_required: false,
            admin_count: 1,
            expired_invites: 2,
            admin_api_keys: 1,
            stale_sessions: 3,
            suspended_with_sessions: 1,
            session_days: 7,
            min_password_length: 10,
            ..Posture::default()
        };
        for f in audit(&p) {
            let (_, path) = f.fix.unwrap_or_else(|| panic!("{} has no fix", f.code));
            assert!(path.starts_with("/ghost/"), "{}: {path}", f.code);
        }
    }

    #[test]
    fn hardened_settings_read_back_as_hardened() {
        let s = hardened();
        assert!(!trust_proxy(&s));
        assert_eq!(session_idle_secs(&s), 240 * 60);
        assert_eq!(min_password_length(&s), 16);
    }
}
