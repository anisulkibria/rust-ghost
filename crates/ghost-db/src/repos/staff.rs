//! Staff roles and invitations.
//!
//! Ghost's staff model: every user holds exactly one role, and new staff join
//! by redeeming an emailed invite rather than being created directly. The
//! Owner role is special — it is never assignable and never revocable, only
//! transferred — so it is deliberately absent from `ASSIGNABLE_ROLES`.

use ghost_core::{dates, ids, GhostError, Result};
use sqlx::SqlitePool;

use super::dberr;

/// The roles an owner or administrator can hand out, in Ghost's own order
/// (least to most privileged). `Owner` is not here on purpose, and neither
/// are the integration roles, which belong to API keys rather than people.
pub const ASSIGNABLE_ROLES: &[(&str, &str)] = &[
    ("Contributor", "Can write and edit their own drafts, but cannot publish."),
    ("Author", "Can write, edit and publish their own posts."),
    ("Editor", "Can write and publish anything, and manage other writers."),
    ("Administrator", "Full access to everything except transferring ownership."),
];

pub fn is_assignable_role(name: &str) -> bool {
    ASSIGNABLE_ROLES.iter().any(|(r, _)| *r == name)
}

/// How long an invitation stays redeemable. Ghost uses a week.
const INVITE_VALIDITY_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone)]
pub struct StaffMember {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub email: Option<String>,
    pub profile_image: Option<String>,
    pub status: Option<String>,
    pub role: String,
    pub last_seen: Option<String>,
}

/// Everyone with a staff role, owner first, then by role seniority and name —
/// the order Ghost's Staff screen uses.
pub async fn list_staff(pool: &SqlitePool) -> Result<Vec<StaffMember>> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT u.id, u.name, u.slug, u.email, u.profile_image, u.status, r.name, u.last_seen \
         FROM users u \
         LEFT JOIN roles_users ru ON ru.user_id = u.id \
         LEFT JOIN roles r ON r.id = ru.role_id \
         ORDER BY CASE r.name \
             WHEN 'Owner' THEN 0 WHEN 'Administrator' THEN 1 WHEN 'Editor' THEN 2 \
             WHEN 'Author' THEN 3 WHEN 'Contributor' THEN 4 ELSE 5 END, \
         lower(u.name)",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| StaffMember {
            id: r.0,
            name: r.1,
            slug: r.2,
            email: r.3,
            profile_image: r.4,
            status: r.5,
            role: r.6.unwrap_or_default(),
            last_seen: r.7,
        })
        .collect())
}

/// Replace a user's role. One row per user in `roles_users`, so this clears
/// before inserting rather than accumulating roles.
pub async fn set_role(pool: &SqlitePool, user_id: &str, role: &str) -> Result<()> {
    if !is_assignable_role(role) {
        return Err(GhostError::BadRequest(format!("{role} is not a role you can assign")));
    }
    let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .ok_or_else(|| GhostError::BadRequest(format!("unknown role {role}")))?;

    sqlx::query("DELETE FROM roles_users WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
        .bind(ids::new_id())
        .bind(&role_id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Invite {
    pub id: String,
    pub email: String,
    pub role: String,
    pub token: String,
    pub expires: i64,
    pub status: String,
    pub created_at: String,
}

impl Invite {
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp_millis() > self.expires
    }
}

pub async fn list_invites(pool: &SqlitePool) -> Result<Vec<Invite>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String, i64, String, String)>(
        "SELECT i.id, i.email, r.name, i.token, i.expires, i.status, i.created_at \
         FROM invites i LEFT JOIN roles r ON r.id = i.role_id ORDER BY i.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| Invite {
            id: r.0,
            email: r.1,
            role: r.2.unwrap_or_default(),
            token: r.3,
            expires: r.4,
            status: r.5,
            created_at: r.6,
        })
        .collect())
}

pub async fn find_invite_by_token(pool: &SqlitePool, token: &str) -> Result<Option<Invite>> {
    let row = sqlx::query_as::<_, (String, String, Option<String>, String, i64, String, String)>(
        "SELECT i.id, i.email, r.name, i.token, i.expires, i.status, i.created_at \
         FROM invites i LEFT JOIN roles r ON r.id = i.role_id WHERE i.token = ? LIMIT 1",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    Ok(row.map(|r| Invite {
        id: r.0,
        email: r.1,
        role: r.2.unwrap_or_default(),
        token: r.3,
        expires: r.4,
        status: r.5,
        created_at: r.6,
    }))
}

/// Create (or refresh) an invitation. Re-inviting an address replaces the
/// outstanding invite rather than erroring, which is also how "resend" works
/// — the old token stops being valid, so a forwarded email cannot be redeemed
/// after the invite was reissued.
pub async fn create_invite(pool: &SqlitePool, email: &str, role: &str) -> Result<Invite> {
    if !is_assignable_role(role) {
        return Err(GhostError::BadRequest(format!("{role} is not a role you can assign")));
    }
    let email = email.trim().to_lowercase();
    let taken: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = ?")
        .bind(&email)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    if taken.is_some() {
        return Err(GhostError::BadRequest("Someone with that email is already on the team.".into()));
    }
    let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?
        .ok_or_else(|| GhostError::BadRequest(format!("unknown role {role}")))?;

    sqlx::query("DELETE FROM invites WHERE lower(email) = ?")
        .bind(&email)
        .execute(pool)
        .await
        .map_err(dberr)?;

    let token = ids::random_hex(32);
    let expires = chrono::Utc::now().timestamp_millis() + INVITE_VALIDITY_SECS * 1000;
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO invites (id, role_id, token, email, status, expires, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'pending', ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&role_id)
    .bind(&token)
    .bind(&email)
    .bind(expires)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;

    find_invite_by_token(pool, &token)
        .await?
        .ok_or_else(|| GhostError::Internal("invite vanished".into()))
}

pub async fn mark_invite_sent(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE invites SET status = 'sent', updated_at = ? WHERE id = ?")
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

pub async fn revoke_invite(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM invites WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

pub async fn delete_invite_by_email(pool: &SqlitePool, email: &str) -> Result<()> {
    sqlx::query("DELETE FROM invites WHERE lower(email) = lower(?)")
        .bind(email)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Count of people who can still administer the site. Used to refuse the last
/// removal: a site with no owner or admin cannot be managed again.
pub async fn admin_count(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users u \
         JOIN roles_users ru ON ru.user_id = u.id \
         JOIN roles r ON r.id = ru.role_id \
         WHERE r.name IN ('Owner', 'Administrator') AND u.status != 'inactive'",
    )
    .fetch_one(pool)
    .await
    .map_err(dberr)
}

/// Suspend or reactivate. Ghost keeps the row and flips `status`, so the
/// person's posts keep their author rather than being orphaned by a delete.
pub async fn set_user_status(pool: &SqlitePool, user_id: &str, status: &str) -> Result<()> {
    if !matches!(status, "active" | "inactive") {
        return Err(GhostError::BadRequest("unknown status".into()));
    }
    sqlx::query("UPDATE users SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(dates::now_db())
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Remove a staff account. Posts they authored are kept — `posts_authors`
/// rows go, and the post's denormalised `author_id` is left for the caller to
/// reassign if it points here.
pub async fn delete_staff(pool: &SqlitePool, user_id: &str) -> Result<()> {
    for sql in [
        "DELETE FROM roles_users WHERE user_id = ?",
        "DELETE FROM posts_authors WHERE author_id = ?",
        "DELETE FROM sessions WHERE user_id = ?",
        "DELETE FROM permissions_users WHERE user_id = ?",
        "DELETE FROM users WHERE id = ?",
    ] {
        sqlx::query(sql).bind(user_id).execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_is_not_assignable() {
        // Ghost transfers ownership through a separate flow; it must never be
        // handed out from the role picker
        assert!(!is_assignable_role("Owner"));
        // nor are the integration roles, which belong to API keys
        assert!(!is_assignable_role("Admin Integration"));
        assert!(!is_assignable_role("Scheduler Integration"));
    }

    #[test]
    fn the_four_staff_roles_are_assignable_in_ghosts_order() {
        let names: Vec<&str> = ASSIGNABLE_ROLES.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["Contributor", "Author", "Editor", "Administrator"]);
        assert!(names.iter().all(|n| is_assignable_role(n)));
    }

    #[test]
    fn invite_expiry_is_a_week_out() {
        let now = chrono::Utc::now().timestamp_millis();
        let invite = Invite {
            id: "1".into(),
            email: "a@b.c".into(),
            role: "Author".into(),
            token: "t".into(),
            expires: now + INVITE_VALIDITY_SECS * 1000,
            status: "pending".into(),
            created_at: String::new(),
        };
        assert!(!invite.is_expired());

        let stale = Invite { expires: now - 1000, ..invite };
        assert!(stale.is_expired());
    }
}
