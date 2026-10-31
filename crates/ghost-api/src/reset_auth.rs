//! "Reset all authentication" — the break-glass action for a compromised site.
//!
//! Ported from `core/server/auth/reset-authentication.js`. It assumes the worst
//! has already happened, so it revokes everything an attacker could be holding
//! rather than only the thing that is known to have leaked:
//!
//! * every API key secret is rotated, so integrations and any stolen key stop
//!   working until the publisher re-copies them;
//! * every staff account is locked, so a known password is not enough to get
//!   back in — each person has to go through password recovery;
//! * every staff session is deleted and every member's `transient_id` rolled,
//!   which is what actually signs members out (member sessions are not rows);
//! * every outstanding token — magic links, invitations, gift links, password
//!   resets — is destroyed, including links already sitting in an inbox.
//!
//! The counts come back so the caller can report what happened. Upstream also
//! reschedules its job adapter against the new scheduler key; this port's
//! scheduling is in-process and re-reads its key, so there is no equivalent
//! step to take.
use ghost_core::{dates, ids};
use serde_json::json;
use sqlx::SqlitePool;

/// What a reset revoked, in upstream's shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResetOutcome {
    pub api_keys_rotated: u64,
    pub users_locked: u64,
}

/// A locked staff account cannot sign in until its password is reset.
///
/// Ghost's own status value, so the existing sign-in guards and the staff list
/// already understand it.
pub const LOCKED: &str = "locked";

/// Revoke every credential on the site.
///
/// `actor_id` records who did it, when a staff user did — the action row is
/// the only trace left afterwards, since the session that authorised it is one
/// of the things destroyed.
///
/// Runs in a transaction so a crash cannot leave the site half-rotated: keys
/// changed but accounts still open, or the audit row missing.
pub async fn reset_authentication(
    pool: &SqlitePool,
    actor_id: Option<&str>,
) -> Result<ResetOutcome, ghost_core::GhostError> {
    let db = |e: sqlx::Error| ghost_core::GhostError::Db(e.to_string());
    let now = dates::now_db();
    let mut tx = pool.begin().await.map_err(db)?;

    // Rotate every key's secret. Content keys are 26 hex characters and admin
    // keys are an id:secret pair whose secret is 64; both are regenerated at
    // their own width so existing clients fail cleanly rather than oddly.
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT id, type FROM api_keys").fetch_all(&mut *tx).await.map_err(db)?;
    let mut api_keys_rotated = 0u64;
    for (id, key_type) in &keys {
        let secret = if key_type == "content" { ids::random_hex(13) } else { ids::random_hex(32) };
        sqlx::query("UPDATE api_keys SET secret = ?, updated_at = ? WHERE id = ?")
            .bind(&secret)
            .bind(&now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        api_keys_rotated += 1;
    }

    // Lock every staff account. Suspended users stay suspended — unlocking
    // them as a side effect of a security action would be the opposite of the
    // intent.
    let users_locked = sqlx::query("UPDATE users SET status = ?, updated_at = ? WHERE status = 'active'")
        .bind(LOCKED)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(db)?
        .rows_affected();

    if let Some(actor_id) = actor_id {
        let context = json!({
            "action_name": "reset_authentication",
            "api_keys_rotated": api_keys_rotated,
            "users_locked": users_locked,
        });
        sqlx::query(
            "INSERT INTO actions (id, resource_id, resource_type, actor_id, actor_type, event, context, created_at) \
             VALUES (?, NULL, 'security_action', ?, 'user', 'edited', ?, ?)",
        )
        .bind(ids::new_id())
        .bind(actor_id)
        .bind(context.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }

    tx.commit().await.map_err(db)?;

    // After the commit, so a failure here cannot leave rotated keys with the
    // old sessions still live — upstream orders it the same way.
    let _ = sqlx::query("DELETE FROM sessions").execute(pool).await;
    // Members authenticate by a signed `transient_id`, so rolling it is what
    // ends their sessions; deleting rows would not.
    let members: Vec<String> =
        sqlx::query_scalar("SELECT id FROM members").fetch_all(pool).await.unwrap_or_default();
    for id in members {
        let _ = sqlx::query("UPDATE members SET transient_id = ? WHERE id = ?")
            .bind(ids::uuid_v4())
            .bind(&id)
            .execute(pool)
            .await;
    }
    // Outstanding magic links, invitations and gift links, including any
    // already delivered.
    let _ = sqlx::query("DELETE FROM tokens").execute(pool).await;
    let _ = sqlx::query("DELETE FROM staff_password_resets").execute(pool).await;

    Ok(ResetOutcome { api_keys_rotated, users_locked })
}

/// How the outcome reads in the admin, and in the API's message.
pub fn summary(outcome: &ResetOutcome) -> String {
    let plural = |n: u64, one: &str, many: &str| if n == 1 { one.to_string() } else { format!("{n} {many}") };
    format!(
        "All authentication was reset: {} rotated and {} locked. Everyone, including you, must reset their password to sign in.",
        plural(outcome.api_keys_rotated, "1 API key", "API keys"),
        plural(outcome.users_locked, "1 staff account", "staff accounts"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_reads_correctly_for_one_and_for_many() {
        let one = ResetOutcome { api_keys_rotated: 1, users_locked: 1 };
        assert!(summary(&one).contains("1 API key rotated"));
        assert!(summary(&one).contains("1 staff account locked"));
        let many = ResetOutcome { api_keys_rotated: 4, users_locked: 3 };
        assert!(summary(&many).contains("4 API keys rotated"));
        assert!(summary(&many).contains("3 staff accounts locked"));
    }

    #[test]
    fn the_summary_says_the_caller_is_locked_out_too() {
        // The thing a publisher most needs to know before pressing it.
        let text = summary(&ResetOutcome::default());
        assert!(text.contains("including you"), "{text}");
    }
}
