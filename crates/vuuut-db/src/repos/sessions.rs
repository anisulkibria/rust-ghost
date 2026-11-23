//! DB-backed admin sessions (express-session equivalent).
//!
//! A session carries its verification state in `session_data`, the way Ghost
//! keeps `verified` on the express session. When email 2FA is switched on, a
//! correct password only buys an *unverified* session: it names the user but
//! [`get_user`] refuses to resolve it until the emailed code is accepted, so
//! every guard in the admin — UI and API alike — keeps the login out until
//! then.
use vuuut_core::{dates, ids, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use crate::Db;

/// How long an emailed sign-in code stays usable. Matches Ghost's
/// `AUTH_CODE_VALIDITY_MS` in core/server/auth/session/session-service.js.
pub const AUTH_CODE_VALIDITY_SECS: i64 = 5 * 60;

pub async fn create(pool: &Db, user_id: &str) -> Result<String> {
    insert(pool, user_id, true).await
}

/// Create a session that still needs an emailed code before it counts as a
/// login.
pub async fn create_unverified(pool: &Db, user_id: &str) -> Result<String> {
    insert(pool, user_id, false).await
}

async fn insert(pool: &Db, user_id: &str, verified: bool) -> Result<String> {
    let session_id = ids::random_hex(16); // 32 chars
    let now = dates::now_db();
    let data = json!({ "passport": { "user": user_id }, "verified": verified }).to_string();
    sqlx::query("INSERT INTO sessions (id, session_id, user_id, session_data, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(&session_id)
        .bind(user_id)
        .bind(&data)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(session_id)
}

/// The user a *verified* session belongs to — the lookup every admin guard
/// goes through.
///
/// **Prefer [`get_user_within`].** This form applies no lifetime, which is
/// what the session table did before the Security module existed: the cookie
/// carried a seven-day `Max-Age` and the row behind it lived forever, so a
/// session id read out of the database was a login with no expiry date. It is
/// kept for the callers that are asking a different question — "whose
/// pending sign-in is this" — and for the tests.
pub async fn get_user(pool: &Db, session_id: &str) -> Result<Option<String>> {
    Ok(load(pool, session_id).await?.filter(|s| s.verified).map(|s| s.user_id))
}

/// The user a verified session belongs to, provided the session is still
/// inside the site's session policy.
///
/// `max_age_secs` is the longest a session may live from when it was created,
/// however much it is used; `idle_secs` is the longest it may sit unused, and
/// `0` means there is no idle limit. Both come from the Security module's
/// settings — see `vuuut_core::security::session_max_age_secs`.
///
/// ## Why the expired row is deleted here
///
/// The alternative is to refuse it and leave it, and let the periodic sweep
/// clear up. That works, and the sweep does run — but it makes the guard's
/// correctness depend on a background task having been scheduled, and a
/// deployment that never starts the sweeper would accumulate rows forever
/// while still (correctly) refusing them. Deleting on the request that found
/// it costs one statement on a path that was about to reject the request
/// anyway, and means the policy is enforced by the thing that reads the
/// session rather than by the thing that tidies up after it.
///
/// ## Why `updated_at` falls back to `created_at`
///
/// `updated_at` is written only when the session *data* changes, which for a
/// session that never needed an emailed code is never. Reading a null as
/// "last used at the dawn of time" would expire every ordinary session the
/// moment an idle timeout was switched on. The fallback makes the idle clock
/// start at sign-in, which is the only reading that is both safe and true.
pub async fn get_user_within(
    pool: &Db,
    session_id: &str,
    max_age_secs: i64,
    idle_secs: i64,
) -> Result<Option<String>> {
    let Some(session) = load(pool, session_id).await? else { return Ok(None) };
    if !session.verified {
        return Ok(None);
    }

    let now = chrono::Utc::now();
    let age_of = |stamp: &str| dates::from_db_string(stamp).map(|t| (now - t).num_seconds());

    // A timestamp that will not parse is treated as expired rather than as
    // fresh. It should not happen; if it does, the safe reading of "I cannot
    // tell how old this session is" is not "it is new".
    let too_old = max_age_secs > 0
        && age_of(&session.created_at).map(|age| age > max_age_secs).unwrap_or(true);
    let last_used = session
        .updated_at
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&session.created_at);
    let too_idle = idle_secs > 0 && age_of(last_used).map(|age| age > idle_secs).unwrap_or(true);

    if too_old || too_idle {
        // Ignored deliberately: the session is refused either way, and a
        // failure to tidy up must not turn into a failure to log out.
        let _ = destroy(pool, session_id).await;
        return Ok(None);
    }
    Ok(Some(session.user_id))
}

/// The user a session belongs to whether or not it has been verified yet.
/// Only the code-entry screen should use this.
pub async fn get_pending_user(pool: &Db, session_id: &str) -> Result<Option<String>> {
    Ok(load(pool, session_id).await?.map(|s| s.user_id))
}

/// Whether a session is still waiting on its emailed code.
pub async fn is_unverified(pool: &Db, session_id: &str) -> Result<bool> {
    Ok(load(pool, session_id).await?.is_some_and(|s| !s.verified))
}

struct SessionRow {
    user_id: String,
    verified: bool,
    data: Value,
    created_at: String,
    /// Written only when the session data changes — accepting an emailed
    /// code, and nothing else — so an ordinary session has none for its whole
    /// life. [`get_user_within`] explains why that matters.
    updated_at: Option<String>,
}

async fn load(pool: &Db, session_id: &str) -> Result<Option<SessionRow>> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT user_id, session_data, created_at, updated_at FROM sessions WHERE session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    Ok(row.map(|(user_id, raw, created_at, updated_at)| {
        let data: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
        // Sessions minted before 2FA existed carry no `verified` key at all.
        // They predate the requirement, so they stay signed in rather than
        // every open tab being logged out the moment the setting is added.
        let verified = data.get("verified").and_then(Value::as_bool).unwrap_or(true);
        SessionRow { user_id, verified, data, created_at, updated_at }
    }))
}

/// Codes are short enough to brute-force if they ever leaked, so only their
/// digest is stored — the readable code lives in the email and nowhere else.
fn digest(code: &str) -> String {
    hex::encode(Sha256::digest(code.trim().as_bytes()))
}

/// Attach a freshly issued sign-in code to a pending session.
pub async fn set_auth_code(pool: &Db, session_id: &str, code: &str) -> Result<()> {
    let Some(session) = load(pool, session_id).await? else {
        return Ok(());
    };
    let mut data = session.data;
    data["auth_code"] = json!(digest(code));
    data["auth_code_at"] = json!(dates::now_db());
    data["auth_code_tries"] = json!(0);
    save(pool, session_id, &data).await
}

/// Outcome of checking a code typed into the sign-in screen.
#[derive(PartialEq, Debug)]
pub enum CodeCheck {
    Verified,
    Incorrect,
    /// No code issued, past its five minutes, or too many wrong guesses — the
    /// caller should offer a fresh one rather than let the user keep trying.
    Expired,
}

/// Check a code against the pending session and, when it matches, promote the
/// session to verified.
pub async fn verify_auth_code(pool: &Db, session_id: &str, code: &str) -> Result<CodeCheck> {
    let Some(session) = load(pool, session_id).await? else {
        return Ok(CodeCheck::Expired);
    };
    let mut data = session.data;

    let issued = data.get("auth_code_at").and_then(Value::as_str).and_then(dates::from_db_string);
    let expected = data.get("auth_code").and_then(Value::as_str).map(str::to_string);
    let tries = data.get("auth_code_tries").and_then(Value::as_i64).unwrap_or(0);

    let (Some(expected), Some(issued)) = (expected, issued) else {
        return Ok(CodeCheck::Expired);
    };
    let age = (chrono::Utc::now() - issued).num_seconds();
    if age > AUTH_CODE_VALIDITY_SECS || tries >= 5 {
        return Ok(CodeCheck::Expired);
    }

    // Compared as digests, so the check never depends on the plaintext code
    // still being around.
    if digest(code) != expected {
        data["auth_code_tries"] = json!(tries + 1);
        save(pool, session_id, &data).await?;
        return Ok(CodeCheck::Incorrect);
    }

    data["verified"] = json!(true);
    // the code is spent — a replay has to start a new sign-in
    data["auth_code"] = Value::Null;
    data["auth_code_at"] = Value::Null;
    data["auth_code_tries"] = json!(0);
    save(pool, session_id, &data).await?;
    Ok(CodeCheck::Verified)
}

async fn save(pool: &Db, session_id: &str, data: &Value) -> Result<()> {
    sqlx::query("UPDATE sessions SET session_data = ?, updated_at = ? WHERE session_id = ?")
        .bind(data.to_string())
        .bind(dates::now_db())
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}

pub async fn destroy(pool: &Db, session_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE session_id = ?")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}

/// Kill every session a user holds — what Ghost does the moment a password
/// changes, so a cookie cloned before the change stops working. `keep` (the
/// current session id) survives, for the person who just changed their own
/// password and is handed a replacement.
pub async fn destroy_for_user(pool: &Db, user_id: &str, keep: Option<&str>) -> Result<()> {
    match keep {
        Some(sid) => {
            sqlx::query("DELETE FROM sessions WHERE user_id = ? AND session_id != ?")
                .bind(user_id)
                .bind(sid)
                .execute(pool)
                .await
                .map_err(super::dberr)?;
        }
        None => {
            sqlx::query("DELETE FROM sessions WHERE user_id = ?")
                .bind(user_id)
                .execute(pool)
                .await
                .map_err(super::dberr)?;
        }
    }
    Ok(())
}
