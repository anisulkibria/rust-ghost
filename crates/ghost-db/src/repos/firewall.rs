//! Every query behind the firewall, and the cache the middleware reads.
//!
//! This is the Security module's one table, and the reason it needed one:
//! unlike the posture report — computed from sessions, keys, staff and
//! settings that already exist — a list of barred addresses is not derivable
//! from anything. Somebody has to write it down.
//!
//! ## The cache, and why there is one
//!
//! The firewall is consulted on **every request**, including requests that
//! will be refused, which is exactly when a site is under the kind of load
//! that makes a firewall worth having. A query per request would mean a
//! database round trip per request — so the rules are held in an
//! [`arc_swap`-style](RuleCache) `RwLock<Arc<Vec<Rule>>>` and the middleware
//! reads a clone of the `Arc`.
//!
//! The cache is reloaded on every write from the admin screen, and at boot.
//! It is **not** reloaded on a timer: a rule added by editing the database by
//! hand will not take effect until a restart, which is the honest trade and
//! is written on the admin screen. The alternative — polling — costs a query
//! per interval forever to serve a case nobody has.
//!
//! Expired rules are dropped as the cache is built, so "is this rule still in
//! force" is answered once per reload rather than once per request.
//!
//! ## Hit counting
//!
//! A rule that refuses somebody gets `hits` incremented, so the screen can
//! show which rules are doing anything and which are stale. This is one write
//! per *refused* request — never on the path of a request that is allowed,
//! which is almost all of them. A refused request has already cost the
//! attacker more than it costs us.
use super::dberr;
use crate::Db;
use ghost_core::firewall::{Action, Pattern, Rule, Scope, MAX_RULES};
use ghost_core::{dates, ids, Result};
use std::sync::{Arc, RwLock};

/// A rule as it is stored, with the columns the screen shows that the
/// matching does not need.
#[derive(Debug, Clone)]
pub struct StoredRule {
    pub id: String,
    pub action: String,
    pub scope: String,
    pub pattern: String,
    pub note: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub hits: i64,
    pub last_hit_at: Option<String>,
}

impl StoredRule {
    /// The matching form, or `None` if the stored pattern will not parse.
    ///
    /// A row whose pattern is unreadable is **dropped**, not guessed at. It
    /// cannot match anything sensibly, and the two ways to guess are both
    /// wrong: treating it as "matches nothing" silently disables a block
    /// somebody is relying on, and treating it as "matches everything" takes
    /// the site down. Dropping it is the first of those, but visibly — the
    /// screen shows the row with its pattern struck through, so the thing
    /// that is not working is on the page rather than in a log.
    pub fn as_rule(&self) -> Option<Rule> {
        Some(Rule {
            id: self.id.clone(),
            action: Action::parse(&self.action),
            scope: Scope::parse(&self.scope),
            pattern: Pattern::parse(&self.pattern)?,
            note: self.note.clone(),
        })
    }

    /// Whether this row's pattern is readable at all. The screen uses it to
    /// mark a row that is in the table but not in force.
    pub fn is_valid(&self) -> bool {
        Pattern::parse(&self.pattern).is_some()
    }

    pub fn is_expired(&self, now: &str) -> bool {
        self.expires_at.as_deref().is_some_and(|at| !at.is_empty() && at <= now)
    }
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: String,
    action: String,
    scope: String,
    pattern: String,
    note: Option<String>,
    created_at: String,
    expires_at: Option<String>,
    hits: Option<i64>,
    last_hit_at: Option<String>,
}

impl From<RuleRow> for StoredRule {
    fn from(r: RuleRow) -> StoredRule {
        StoredRule {
            id: r.id,
            action: r.action,
            scope: r.scope,
            pattern: r.pattern,
            note: r.note.unwrap_or_default(),
            created_at: r.created_at,
            expires_at: r.expires_at,
            hits: r.hits.unwrap_or(0),
            last_hit_at: r.last_hit_at,
        }
    }
}

const COLS: &str =
    "id, action, scope, pattern, note, created_at, expires_at, hits, last_hit_at";

/// Every rule, expired ones included, for the admin screen.
///
/// Ordered allow-first then by pattern, which is the order they are evaluated
/// in and so the order they make sense read in: a reader working out why an
/// address gets through sees the allow that let it before the block that
/// would have stopped it.
pub async fn all(db: &Db) -> Result<Vec<StoredRule>> {
    let rows: Vec<RuleRow> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM firewall_rules ORDER BY action ASC, scope ASC, pattern ASC"
    ))
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(StoredRule::from).collect())
}

/// Every rule that is in force now: not expired, and with a readable pattern.
pub async fn live(db: &Db) -> Result<Vec<Rule>> {
    let now = dates::now_db();
    let rows: Vec<RuleRow> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM firewall_rules \
         WHERE expires_at IS NULL OR expires_at = '' OR expires_at > ? \
         ORDER BY action ASC LIMIT ?"
    ))
    .bind(&now)
    .bind(MAX_RULES as i64)
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(StoredRule::from).filter_map(|r| r.as_rule()).collect())
}

pub async fn find(db: &Db, id: &str) -> Result<Option<StoredRule>> {
    let row: Option<RuleRow> = sqlx::query_as(&format!("SELECT {COLS} FROM firewall_rules WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(dberr)?;
    Ok(row.map(StoredRule::from))
}

pub async fn count(db: &Db) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM firewall_rules").fetch_one(db).await.map_err(dberr)
}

/// What a new rule is made of. A typed struct rather than eight arguments,
/// so a call site cannot swap `scope` and `action` and still compile.
pub struct NewRule<'a> {
    pub action: Action,
    pub scope: Scope,
    pub pattern: Pattern,
    pub note: &'a str,
    pub created_by: Option<&'a str>,
    /// Absolute instant, already computed — never an interval for the
    /// database to add, which does not work across both engines through the
    /// Any driver.
    pub expires_at: Option<String>,
}

/// Add a rule, replacing any existing rule for the same scope and pattern.
///
/// Replacing rather than erroring on the unique index: saving an allow for a
/// range that is already blocked at the same scope is somebody changing their
/// mind, and two contradictory rules for one range is not a state the screen
/// should be able to produce. The pattern is stored as
/// `Pattern`'s normalized form, which is what makes the index mean "one rule
/// per range" rather than "one rule per spelling".
pub async fn upsert(db: &Db, rule: &NewRule<'_>) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    let pattern = rule.pattern.to_string();

    // Delete-then-insert rather than an ON CONFLICT upsert: the id changes
    // when a rule is replaced, which is correct — it is a new decision with a
    // new author and a fresh hit count, not an edit of the old one — and it
    // keeps this one statement pair identical on both engines.
    sqlx::query("DELETE FROM firewall_rules WHERE scope = ? AND pattern = ?")
        .bind(rule.scope.as_str())
        .bind(&pattern)
        .execute(db)
        .await
        .map_err(dberr)?;

    sqlx::query(
        "INSERT INTO firewall_rules (id, action, scope, pattern, note, created_at, created_by, expires_at, hits) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
    )
    .bind(&id)
    .bind(rule.action.as_str())
    .bind(rule.scope.as_str())
    .bind(&pattern)
    .bind(rule.note)
    .bind(&now)
    .bind(rule.created_by)
    .bind(rule.expires_at.as_deref())
    .execute(db)
    .await
    .map_err(dberr)?;
    Ok(id)
}

pub async fn delete(db: &Db, id: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM firewall_rules WHERE id = ?")
        .bind(id)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected() > 0)
}

/// Drop every rule. The way out of a firewall somebody has tangled.
pub async fn delete_all(db: &Db) -> Result<u64> {
    let done = sqlx::query("DELETE FROM firewall_rules").execute(db).await.map_err(dberr)?;
    Ok(done.rows_affected())
}

/// Remove rules whose expiry has passed. Called at boot beside the other
/// sweeps; reads already filter them, so this only keeps the table tidy.
pub async fn sweep_expired(db: &Db) -> Result<u64> {
    let done = sqlx::query(
        "DELETE FROM firewall_rules WHERE expires_at IS NOT NULL AND expires_at <> '' AND expires_at <= ?",
    )
    .bind(dates::now_db())
    .execute(db)
    .await
    .map_err(dberr)?;
    Ok(done.rows_affected())
}

/// Count one refusal against a rule.
///
/// Errors are swallowed: a request that has already been refused must not
/// then fail differently because the statistics could not be written.
pub async fn record_hit(db: &Db, id: &str) {
    let _ = sqlx::query("UPDATE firewall_rules SET hits = hits + 1, last_hit_at = ? WHERE id = ?")
        .bind(dates::now_db())
        .bind(id)
        .execute(db)
        .await;
}

// ---------------------------------------------------------------------------
// the in-memory cache the middleware reads
// ---------------------------------------------------------------------------

/// The live rules, held in memory for the middleware.
///
/// See the module doc for why this exists and why it is not polled. Cloning
/// the `Arc` is what the middleware does per request; the `RwLock` is only
/// contended on a reload, which happens when somebody saves a rule.
#[derive(Clone, Default)]
pub struct RuleCache(Arc<RwLock<Arc<Vec<Rule>>>>);

impl std::fmt::Debug for RuleCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RuleCache({} rules)", self.rules().len())
    }
}

impl RuleCache {
    /// An empty cache, for a process that has not loaded yet and for tests.
    pub fn empty() -> RuleCache {
        RuleCache(Arc::new(RwLock::new(Arc::new(Vec::new()))))
    }

    /// A cache over rules already in hand, with no database behind it.
    pub fn from_rules(rules: Vec<Rule>) -> RuleCache {
        RuleCache(Arc::new(RwLock::new(Arc::new(rules))))
    }

    /// Load from the database. Call at boot and after every write.
    pub async fn load(db: &Db) -> Result<RuleCache> {
        Ok(RuleCache::from_rules(live(db).await?))
    }

    /// Re-read the rules. Logs rather than returns on failure: a save that
    /// succeeded must not report an error because the cache could not be
    /// refreshed, and the next reload or restart will pick it up.
    pub async fn reload(&self, db: &Db) {
        match live(db).await {
            Ok(rules) => *self.0.write().unwrap() = Arc::new(rules),
            Err(e) => tracing::error!("firewall: could not reload the rules: {}", e.message()),
        }
    }

    /// The current rules. Cheap — one `Arc` clone.
    pub fn rules(&self) -> Arc<Vec<Rule>> {
        self.0.read().unwrap().clone()
    }

    /// Whether there are any rules at all, so the middleware can return
    /// before parsing an address when the firewall is empty.
    pub fn is_empty(&self) -> bool {
        self.0.read().unwrap().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(pattern: &str, expires: Option<&str>) -> StoredRule {
        StoredRule {
            id: "r1".into(),
            action: "block".into(),
            scope: "site".into(),
            pattern: pattern.into(),
            note: String::new(),
            created_at: "2026-11-16 10:00:00".into(),
            expires_at: expires.map(str::to_string),
            hits: 0,
            last_hit_at: None,
        }
    }

    #[test]
    fn a_row_whose_pattern_will_not_parse_is_dropped_rather_than_guessed_at() {
        // The two ways to guess are both wrong: "matches nothing" silently
        // disables a block somebody relies on, "matches everything" takes the
        // site down. Dropping it is the first, but visibly — `is_valid` is
        // what puts it on the screen struck through.
        let bad = stored("not-an-address", None);
        assert!(bad.as_rule().is_none());
        assert!(!bad.is_valid());

        let good = stored("203.0.113.0/24", None);
        assert!(good.as_rule().is_some());
        assert!(good.is_valid());
    }

    #[test]
    fn expiry_is_a_string_comparison_on_the_schemas_own_date_format() {
        let r = stored("203.0.113.4", Some("2026-11-16 09:00:00"));
        assert!(r.is_expired("2026-11-16 10:00:00"));
        assert!(!r.is_expired("2026-11-16 08:00:00"));
        // A rule with no expiry never expires, and neither does one whose
        // column is blank — which is what an empty form field writes.
        assert!(!stored("203.0.113.4", None).is_expired("2099-01-01 00:00:00"));
        assert!(!stored("203.0.113.4", Some("")).is_expired("2099-01-01 00:00:00"));
    }

    #[test]
    fn the_cache_hands_out_what_it_was_given() {
        let empty = RuleCache::empty();
        assert!(empty.is_empty());
        assert!(empty.rules().is_empty());

        let one = RuleCache::from_rules(vec![stored("203.0.113.4", None).as_rule().unwrap()]);
        assert!(!one.is_empty());
        assert_eq!(one.rules().len(), 1);
    }
}
