//! Every query behind the Security module, plus the two sweeps that enforce
//! the session policy.
//!
//! The module owns **no table of its own**. Everything it reports already
//! exists and already has an owner: `sessions` is who is signed in, `brute` is
//! who is locked out, `api_keys` and `webhooks` are the ways in that are not a
//! password, `invites` is who has been offered one, `users` is who there is,
//! and `actions` is the audit log. A security report that kept its own copy of
//! any of that would be a second answer to a question the database already
//! answers, and the two would disagree the first time somebody changed
//! something by another route. That is the same argument
//! [`super::seo`] makes about posts, and the reason this module's migration
//! creates only `settings` rows.
//!
//! Two things here are not reads:
//!
//! * [`expire_sessions`] applies the maximum age and the idle timeout by
//!   deleting rows. It is called from the session guard, so the policy is
//!   enforced on the request that would otherwise have used an expired
//!   session, and separately from the periodic sweep so the table does not
//!   keep rows nobody asks about.
//! * [`revoke_session`] and [`revoke_sessions_for_user`] are what the report's
//!   buttons do.
//!
//! ## Personal data
//!
//! The session list names staff members and the times they signed in, and the
//! lockout list holds IP addresses. Both are personal data about staff rather
//! than about members, and the screen that shows them is Administrator-only
//! for that reason as well as for the obvious one. Nothing here reads a
//! member.
use super::dberr;
use crate::Db;
use ghost_core::{dates, Result};

/// The most rows any one list on the report shows. A site with ten thousand
/// lockouts has a problem the report should name as a number, not paint as a
/// table.
pub const LIST_LIMIT: i64 = 200;

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

/// One live staff session, as the report lists it.
#[derive(Debug, Clone)]
pub struct Session {
    /// The `session_id`, which is also the cookie value. **Never rendered**:
    /// the report shows [`Self::short`] and posts the full value only back to
    /// the revoke endpoint, over the same origin. Putting it on the page
    /// would make a screenshot of the Security screen a working login.
    pub session_id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_email: String,
    pub user_slug: String,
    /// `active` or `inactive`. A session on an `inactive` user is the finding
    /// `suspended_sessions` is about.
    pub user_status: String,
    pub role: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    /// Whether the session has passed its emailed code, when 2FA is on. An
    /// unverified session is not a login and resolves to nobody.
    pub verified: bool,
}

impl Session {
    /// The first six characters of the id, so two sessions belonging to one
    /// person can be told apart on the page without the page carrying either
    /// of them in full.
    ///
    /// Six hex characters out of thirty-two: enough to distinguish the
    /// handful of sessions one account holds, far too few to guess the rest.
    pub fn short(&self) -> String {
        self.session_id.chars().take(6).collect()
    }

    /// When this session was last used, falling back to when it was created —
    /// `updated_at` is only written when the session data changes, which for
    /// a session that never needed a code is never.
    pub fn last_active(&self) -> &str {
        self.updated_at.as_deref().filter(|s| !s.is_empty()).unwrap_or(&self.created_at)
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: String,
    user_id: String,
    name: Option<String>,
    email: Option<String>,
    slug: Option<String>,
    status: Option<String>,
    role: Option<String>,
    session_data: String,
    created_at: String,
    updated_at: Option<String>,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Session {
        // Sessions minted before 2FA existed carry no `verified` key, and
        // `repos::sessions::load` treats those as verified. Reading it the
        // same way here is what keeps the report from listing every old
        // session as a half-finished sign-in.
        let verified = serde_json::from_str::<serde_json::Value>(&r.session_data)
            .ok()
            .and_then(|d| d.get("verified").and_then(serde_json::Value::as_bool))
            .unwrap_or(true);
        Session {
            session_id: r.session_id,
            user_id: r.user_id,
            user_name: r.name.unwrap_or_default(),
            user_email: r.email.unwrap_or_default(),
            user_slug: r.slug.unwrap_or_default(),
            user_status: r.status.unwrap_or_else(|| "active".into()),
            role: r.role.unwrap_or_default(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            verified,
        }
    }
}

/// Every live staff session, most recently used first.
///
/// Joined to `users` rather than looked up per row, because the point of the
/// list is "who is signed in" and a session whose user has been deleted is
/// not an answer to that — the `INNER JOIN` drops it, and
/// [`expire_sessions`] is what eventually removes the row.
pub async fn sessions(db: &Db, limit: i64) -> Result<Vec<Session>> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT s.session_id, s.user_id, u.name, u.email, u.slug, u.status, r.name AS role, \
                s.session_data, s.created_at, s.updated_at \
         FROM sessions s \
         JOIN users u ON u.id = s.user_id \
         LEFT JOIN roles_users ru ON ru.user_id = u.id \
         LEFT JOIN roles r ON r.id = ru.role_id \
         ORDER BY COALESCE(s.updated_at, s.created_at) DESC, s.session_id \
         LIMIT ?",
    )
    .bind(limit.clamp(1, LIST_LIMIT))
    .fetch_all(db)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(Session::from).collect())
}

/// Drop one session. Returns whether a row was actually removed, so the
/// screen can say "revoked" rather than "revoked" over nothing.
pub async fn revoke_session(db: &Db, session_id: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM sessions WHERE session_id = ?")
        .bind(session_id)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected() > 0)
}

/// Drop every session a user holds, optionally sparing one.
///
/// `keep` is the caller's own session, for the administrator who is revoking
/// their own other sessions and does not expect to be signed out by it.
/// Delegates nothing to [`super::sessions::destroy_for_user`] only because
/// this returns the count, which the flash message says out loud.
pub async fn revoke_sessions_for_user(db: &Db, user_id: &str, keep: Option<&str>) -> Result<u64> {
    let done = match keep {
        Some(keep) => sqlx::query("DELETE FROM sessions WHERE user_id = ? AND session_id <> ?")
            .bind(user_id)
            .bind(keep)
            .execute(db)
            .await,
        None => sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(user_id).execute(db).await,
    }
    .map_err(dberr)?;
    Ok(done.rows_affected())
}

/// Drop every session except the caller's. The "sign out everywhere else"
/// button, across all staff.
pub async fn revoke_all_sessions(db: &Db, keep: Option<&str>) -> Result<u64> {
    let done = match keep {
        Some(keep) => {
            sqlx::query("DELETE FROM sessions WHERE session_id <> ?").bind(keep).execute(db).await
        }
        None => sqlx::query("DELETE FROM sessions").execute(db).await,
    }
    .map_err(dberr)?;
    Ok(done.rows_affected())
}

/// Apply the session policy: delete anything past its maximum age, and —
/// when an idle timeout is set — anything unused for longer than that.
///
/// Returns how many rows went, which the sweep logs and the report does not
/// show (a number that is almost always zero is noise on a page).
///
/// ## Why the cutoff is computed in Rust
///
/// `datetime('now', ?)` with a bound modifier does not work on MySQL through
/// the Any driver, and the two engines spell interval arithmetic differently.
/// Both store these columns as the same `YYYY-MM-DD HH:MM:SS` text, so a
/// string comparison against a timestamp built here is identical on both and
/// needs no dialect switch. See the repo-layer notes in
/// `docs/module-pattern.md`.
///
/// ## Why `updated_at` is coalesced
///
/// It is written only when a session's data changes — accepting an emailed
/// code, and nothing else. A session that never needed a code has a null
/// `updated_at` for its whole life, so treating null as "never used" would
/// expire every ordinary session the moment an idle timeout was switched on.
/// Falling back to `created_at` makes the idle clock start at sign-in, which
/// is the only reading that is both safe and true.
pub async fn expire_sessions(db: &Db, max_age_secs: i64, idle_secs: i64) -> Result<u64> {
    let mut removed = 0u64;
    let now = chrono::Utc::now();

    if max_age_secs > 0 {
        let cutoff = dates::to_db_string(&(now - chrono::Duration::seconds(max_age_secs)));
        let done = sqlx::query("DELETE FROM sessions WHERE created_at < ?")
            .bind(&cutoff)
            .execute(db)
            .await
            .map_err(dberr)?;
        removed += done.rows_affected();
    }

    if idle_secs > 0 {
        let cutoff = dates::to_db_string(&(now - chrono::Duration::seconds(idle_secs)));
        let done = sqlx::query("DELETE FROM sessions WHERE COALESCE(updated_at, created_at) < ?")
            .bind(&cutoff)
            .execute(db)
            .await
            .map_err(dberr)?;
        removed += done.rows_affected();
    }

    Ok(removed)
}

/// Drop sessions whose user no longer exists.
///
/// `sessions.user_id` has no foreign key — it predates this port — so
/// deleting a staff member leaves their sessions behind. [`sessions`] already
/// hides them, but a row that resolves to nobody is still a row that
/// `repos::sessions::get_user` will happily return a user id from, and the
/// guard above it then fails to find a user and treats the request as signed
/// out. That is the right outcome by accident rather than by design, and the
/// row should not be there.
pub async fn expire_orphan_sessions(db: &Db) -> Result<u64> {
    let done = sqlx::query("DELETE FROM sessions WHERE user_id NOT IN (SELECT id FROM users)")
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected())
}

/// How many live sessions belong to a suspended account.
///
/// The one number on the report that should always be zero: suspending an
/// account revokes its sessions, so anything here is either a suspension that
/// happened before that was true or a bug. Counted rather than assumed for
/// exactly that reason.
pub async fn suspended_with_sessions(db: &Db) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(DISTINCT s.user_id) FROM sessions s \
         JOIN users u ON u.id = s.user_id WHERE u.status <> 'active'",
    )
    .fetch_one(db)
    .await
    .map_err(dberr)
}

/// How many sessions are older than `max_age_secs` — which the expiry should
/// have taken already, so a non-zero answer is a sweep that is not running.
pub async fn stale_session_count(db: &Db, max_age_secs: i64) -> Result<i64> {
    if max_age_secs <= 0 {
        return Ok(0);
    }
    let cutoff =
        dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::seconds(max_age_secs)));
    sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE created_at < ?")
        .bind(&cutoff)
        .fetch_one(db)
        .await
        .map_err(dberr)
}

// ---------------------------------------------------------------------------
// lockouts
// ---------------------------------------------------------------------------

/// One row of the `brute` table, as the report lists it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Lockout {
    /// The budget key: `user_login:1.2.3.4`, `someone@example.com reset`, and
    /// so on. It can contain an email address, so it is escaped at output
    /// like everything else and the screen is Administrator-only.
    pub key: String,
    /// Recorded attempts. Not "failures" for every budget — the login budget
    /// counts attempts and clears on success, which is express-brute's shape.
    pub count: Option<i64>,
    /// Unix milliseconds. Compared against the clock in Rust rather than in
    /// SQL, because `brute` stores epoch millis as `BIGINT` and the rest of
    /// the schema stores text dates; mixing the two in one query is how a
    /// dialect bug gets in.
    #[sqlx(rename = "lastRequest")]
    pub last_request: Option<i64>,
    #[sqlx(rename = "firstRequest")]
    pub first_request: Option<i64>,
    pub lifetime: Option<i64>,
}

impl Lockout {
    /// What this key is protecting, read off the key's own shape.
    ///
    /// The key is built by the caller that charges the budget, so this is a
    /// presentation concern and not a rule: an unrecognised shape is shown as
    /// itself rather than guessed at.
    pub fn kind(&self) -> &'static str {
        let key = self.key.as_str();
        if key.starts_with("user_login:") {
            "Staff sign-in"
        } else if key.starts_with("user_verification:") {
            "Two-factor code"
        } else if key.starts_with("send_verification_code:") {
            "Code resend"
        } else if key.starts_with("private_block:") {
            "Site access code"
        } else if key.ends_with("reset") || key.starts_with("/authentication/password_reset") {
            // Two shapes for one thing: `<email>reset` is the per-account
            // budget and `/authentication/password_reset:<ip>` the per-address
            // one. Both are needed and neither alone stops both attack
            // shapes — see `ghost_api::brute`'s module doc.
            "Password reset"
        } else if key.ends_with("login") {
            "Member sign-in"
        } else if key.starts_with("/members/api/") {
            "Member magic link"
        } else if key.starts_with("content_api_key:") {
            "Content API key"
        } else if key.starts_with('/') {
            "Endpoint"
        } else {
            "Other"
        }
    }
}

/// Every live `brute` record, busiest first.
///
/// "Live" means `lastRequest` is inside the record's own `lifetime`; an
/// expired record is a row the sweep has not reached yet and is not a lockout.
/// Whether a live record is *currently* blocking depends on the budget it was
/// charged against, which this table does not store — the report says how
/// many attempts are on record and when the last one was, which is the part
/// that is true without knowing the budget.
pub async fn lockouts(db: &Db, limit: i64) -> Result<Vec<Lockout>> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query_as(
        "SELECT `key`, count, lastRequest, firstRequest, lifetime FROM brute \
         WHERE lastRequest IS NOT NULL AND lifetime IS NOT NULL \
           AND lastRequest + (lifetime * 1000) >= ? \
         ORDER BY count DESC, lastRequest DESC LIMIT ?",
    )
    .bind(now)
    .bind(limit.clamp(1, LIST_LIMIT))
    .fetch_all(db)
    .await
    .map_err(dberr)
}

/// How many live records have spent their free allowance.
///
/// `free_retries` differs per budget and the table does not record which
/// budget a key belongs to, so this uses the *smallest* free allowance any
/// authentication budget has (four). It is therefore an upper bound on "how
/// many addresses are being made to wait", and the report says so rather than
/// claiming a precision it does not have.
pub async fn lockout_count(db: &Db) -> Result<i64> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM brute \
         WHERE lastRequest IS NOT NULL AND lifetime IS NOT NULL \
           AND lastRequest + (lifetime * 1000) >= ? AND count > 4",
    )
    .bind(now)
    .fetch_one(db)
    .await
    .map_err(dberr)
}

/// Clear one lockout, for the administrator who has just locked themselves
/// out and is reading this page from another machine.
pub async fn clear_lockout(db: &Db, key: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM brute WHERE `key` = ?")
        .bind(key)
        .execute(db)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected() > 0)
}

/// Clear every lockout. Deliberately a separate, differently-labelled action
/// from clearing one: it hands every address that has been guessing a fresh
/// allowance.
pub async fn clear_all_lockouts(db: &Db) -> Result<u64> {
    let done = sqlx::query("DELETE FROM brute").execute(db).await.map_err(dberr)?;
    Ok(done.rows_affected())
}

// ---------------------------------------------------------------------------
// the counts the posture is built from
// ---------------------------------------------------------------------------

/// The counts [`ghost_core::security::Posture`] needs, in one struct so the
/// admin page does not run eleven awaits in a row and then have to be read
/// to find out what they were.
#[derive(Debug, Clone, Default)]
pub struct Counts {
    pub admins: i64,
    pub sessions: i64,
    pub suspended_with_sessions: i64,
    pub stale_sessions: i64,
    pub open_invites: i64,
    pub expired_invites: i64,
    pub admin_api_keys: i64,
    pub content_api_keys: i64,
    pub lockouts: i64,
    pub webhooks: i64,
}

/// Everything the posture is judged on, measured.
///
/// `max_age_secs` is passed in rather than read here because the setting is
/// the admin's to know about and the repo's job is only to count against it.
pub async fn counts(db: &Db, max_age_secs: i64) -> Result<Counts> {
    let scalar = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql).fetch_one(db).await.map_err(dberr)
    };

    // Expiry is milliseconds since the epoch in `invites`, matching Ghost.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let open_invites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invites WHERE expires > ?")
        .bind(now_ms)
        .fetch_one(db)
        .await
        .map_err(dberr)?;
    let expired_invites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invites WHERE expires <= ?")
        .bind(now_ms)
        .fetch_one(db)
        .await
        .map_err(dberr)?;

    Ok(Counts {
        admins: scalar(
            "SELECT COUNT(DISTINCT u.id) FROM users u \
             JOIN roles_users ru ON ru.user_id = u.id JOIN roles r ON r.id = ru.role_id \
             WHERE u.status = 'active' AND r.name IN ('Owner', 'Administrator')",
        )
        .await?,
        sessions: scalar("SELECT COUNT(*) FROM sessions").await?,
        suspended_with_sessions: suspended_with_sessions(db).await?,
        stale_sessions: stale_session_count(db, max_age_secs).await?,
        open_invites,
        expired_invites,
        admin_api_keys: scalar("SELECT COUNT(*) FROM api_keys WHERE type = 'admin'").await?,
        content_api_keys: scalar("SELECT COUNT(*) FROM api_keys WHERE type = 'content'").await?,
        lockouts: lockout_count(db).await?,
        webhooks: scalar("SELECT COUNT(*) FROM webhooks").await?,
    })
}

// ---------------------------------------------------------------------------
// the audit log, filtered to the events that are about access
// ---------------------------------------------------------------------------

/// One line of the security activity list.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub event: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub actor_name: Option<String>,
    pub actor_slug: Option<String>,
    pub created_at: String,
}

/// The `actions` rows that are about who can get in, newest first.
///
/// The `actions` table is the whole site's audit log — it records a post being
/// published as readily as a staff member being added. The report shows only
/// the resource types that change access, because a security screen that
/// scrolls past forty post edits to reach one role change is a screen nobody
/// reads.
///
/// The list is a fixed allowlist rather than a filter the page can set: a
/// query parameter that chose the resource type would be a query parameter
/// that reached a `LIKE`, and there is no version of that worth having here.
pub async fn events(db: &Db, limit: i64) -> Result<Vec<Event>> {
    sqlx::query_as(
        "SELECT a.event, a.resource_type, a.resource_id, \
                u.name AS actor_name, u.slug AS actor_slug, a.created_at \
         FROM actions a LEFT JOIN users u ON u.id = a.actor_id \
         WHERE a.resource_type IN ('user', 'invite', 'api_key', 'integration', 'webhook', 'setting') \
         ORDER BY a.created_at DESC, a.id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, LIST_LIMIT))
    .fetch_all(db)
    .await
    .map_err(dberr)
}

// ---------------------------------------------------------------------------
// the periodic sweep
// ---------------------------------------------------------------------------

/// Everything the Security module wants done on a timer, in one call.
///
/// Called beside [`crate::repos::sessions`]' own housekeeping from the app's
/// sweeper. Logs rather than returns, because nothing is waiting on the
/// answer and a failure here must not take a request with it — the guard
/// enforces the same policy on every request, so a sweep that does not run
/// leaves rows behind but never leaves a session usable.
pub async fn sweep(db: &Db, max_age_secs: i64, idle_secs: i64) {
    match expire_sessions(db, max_age_secs, idle_secs).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("security sweep: expired {n} staff session(s)"),
        Err(e) => tracing::warn!("security sweep: could not expire sessions: {}", e.message()),
    }
    match expire_orphan_sessions(db).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("security sweep: dropped {n} session(s) with no user"),
        Err(e) => tracing::warn!("security sweep: could not drop orphan sessions: {}", e.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_never_shown_in_full() {
        let s = Session {
            session_id: "0123456789abcdef0123456789abcdef".into(),
            ..Session {
                session_id: String::new(),
                user_id: String::new(),
                user_name: String::new(),
                user_email: String::new(),
                user_slug: String::new(),
                user_status: "active".into(),
                role: String::new(),
                created_at: String::new(),
                updated_at: None,
                verified: true,
            }
        };
        assert_eq!(s.short(), "012345");
        assert_eq!(s.short().len(), 6, "six characters, not a working cookie");
    }

    #[test]
    fn a_lockout_key_is_read_for_what_it_protects() {
        let of = |key: &str| Lockout {
            key: key.into(),
            count: None,
            last_request: None,
            first_request: None,
            lifetime: None,
        }
        .kind();
        assert_eq!(of("user_login:203.0.113.4"), "Staff sign-in");
        assert_eq!(of("user_verification:203.0.113.4"), "Two-factor code");
        assert_eq!(of("private_block:203.0.113.4"), "Site access code");
        assert_eq!(of("someone@example.com reset"), "Password reset");
        assert_eq!(of("someone@example.comlogin"), "Member sign-in");
        assert_eq!(of("/members/api/send-magic-link:203.0.113.4"), "Member magic link");
        assert_eq!(of("/authentication/password_reset:203.0.113.4"), "Password reset");
        // An unrecognised shape is not guessed at.
        assert_eq!(of("something-new"), "Other");
    }

    #[test]
    fn a_session_that_never_needed_a_code_counts_as_active_at_its_creation() {
        // The whole reason `last_active` coalesces: `updated_at` is written
        // only when the session data changes, so an ordinary session has none
        // and an idle timeout reading null as "never" would expire everybody.
        let base = Session {
            session_id: "a".into(),
            user_id: "u".into(),
            user_name: String::new(),
            user_email: String::new(),
            user_slug: String::new(),
            user_status: "active".into(),
            role: String::new(),
            created_at: "2026-11-15 10:00:00".into(),
            updated_at: None,
            verified: true,
        };
        assert_eq!(base.last_active(), "2026-11-15 10:00:00");
        let used = Session { updated_at: Some("2026-11-15 12:00:00".into()), ..base.clone() };
        assert_eq!(used.last_active(), "2026-11-15 12:00:00");
        // An empty string is what a blank column reads as, and is not a time.
        let blank = Session { updated_at: Some(String::new()), ..base };
        assert_eq!(blank.last_active(), "2026-11-15 10:00:00");
    }

    #[test]
    fn a_pre_2fa_session_is_not_listed_as_half_signed_in() {
        let row = |data: &str| {
            Session::from(SessionRow {
                session_id: "a".into(),
                user_id: "u".into(),
                name: None,
                email: None,
                slug: None,
                status: None,
                role: None,
                session_data: data.into(),
                created_at: "2026-11-15 10:00:00".into(),
                updated_at: None,
            })
            .verified
        };
        assert!(row(r#"{"passport":{"user":"u"}}"#), "no key at all reads as verified");
        assert!(row(r#"{"verified":true}"#));
        assert!(!row(r#"{"verified":false}"#));
        assert!(row("not json at all"), "an unreadable row must not read as a pending sign-in");
    }
}
