//! Users (staff) repository.
use ghost_core::{dates, ids, models::User, slug, GhostError, Result};
use sqlx::SqlitePool;
use std::collections::BTreeSet;

use super::{dberr, UserRow, USER_COLS};

pub async fn find_by_ids(pool: &SqlitePool, user_ids: &BTreeSet<String>) -> Result<Vec<User>> {
    let mut out = Vec::new();
    for chunk in user_ids.iter().collect::<Vec<_>>().chunks(100) {
        let mut q = format!("SELECT {USER_COLS} FROM users WHERE id IN (");
        q.push_str(&vec!["?"; chunk.len()].join(","));
        q.push_str(")");
        let mut query = sqlx::query_as::<_, UserRow>(&q);
        for id in chunk {
            query = query.bind(*id);
        }
        let rows = query.fetch_all(pool).await.map_err(dberr)?;
        out.extend(rows.into_iter().map(Into::into));
    }
    Ok(out)
}

pub async fn find_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>> {
    let user = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLS} FROM users WHERE email = ? LIMIT 1"
    ))
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    .map(Into::into);
    Ok(user)
}

pub async fn find_by_slug(pool: &SqlitePool, slug_: &str) -> Result<Option<User>> {
    let user = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLS} FROM users WHERE slug = ? LIMIT 1"
    ))
    .bind(slug_)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    .map(Into::into);
    Ok(user)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<User>> {
    let user = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLS} FROM users WHERE id = ? LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?
    .map(Into::into);
    Ok(user)
}

pub async fn list(pool: &SqlitePool, public_only: bool) -> Result<Vec<User>> {
    let rows = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLS} FROM users {} ORDER BY name",
        if public_only { "WHERE status = 'active' AND visibility = 'public'" } else { "" }
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// The Owner = the user holding the Owner role.
pub async fn find_owner(pool: &SqlitePool) -> Result<Option<User>> {
    let user_id: Option<String> = sqlx::query_scalar(
        "SELECT ru.user_id FROM roles_users ru JOIN roles r ON r.id = ru.role_id WHERE r.name = 'Owner' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    match user_id {
        Some(id) => find_by_id(pool, &id).await,
        None => Ok(None),
    }
}

/// Is setup complete? Ghost: an Owner user with active status exists.
pub async fn is_setup(pool: &SqlitePool) -> Result<bool> {
    let owner = find_owner(pool).await?;
    Ok(owner.map(|u| u.status.as_deref() == Some("active")).unwrap_or(false))
}

pub async fn role_name(pool: &SqlitePool, user_id: &str) -> Result<Option<String>> {
    let name: Option<String> = sqlx::query_scalar(
        "SELECT r.name FROM roles_users ru JOIN roles r ON r.id = ru.role_id WHERE ru.user_id = ? LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    Ok(name)
}

async fn unique_slug(pool: &SqlitePool, base: &str, self_id: Option<&str>) -> Result<String> {
    let mut s = slug::safe_string(base);
    if s.len() > 185 {
        s.truncate(185);
        s = s.trim_matches('-').to_string();
    }
    if s.is_empty() {
        s = "author-1".into();
    }
    for p in ["ghost", "rss", "amp"] {
        if s == p {
            s = format!("{s}-author");
        }
    }
    let mut candidate = s.clone();
    let mut i = 1;
    loop {
        let count: i64 = match self_id {
            Some(id) => sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE slug = ? AND id != ?")
                .bind(&candidate)
                .bind(id)
                .fetch_one(pool)
                .await
                .map_err(dberr)?,
            None => sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE slug = ?")
                .bind(&candidate)
                .fetch_one(pool)
                .await
                .map_err(dberr)?,
        };
        if count == 0 {
            return Ok(candidate);
        }
        i += 1;
        candidate = format!("{s}-{i}");
    }
}

/// Complete setup: activate the placeholder Owner with real credentials.
pub async fn setup_owner(pool: &SqlitePool, name: &str, email: &str, password_hash: &str) -> Result<User> {
    let mut owner = find_owner(pool)
        .await?
        .ok_or_else(|| GhostError::Internal("placeholder owner missing".into()))?;
    let now = dates::now_db();
    let slug_ = unique_slug(pool, name, Some(&owner.id)).await?;
    sqlx::query("UPDATE users SET name = ?, email = ?, password = ?, slug = ?, status = 'active', updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(email)
        .bind(password_hash)
        .bind(&slug_)
        .bind(&now)
        .bind(&owner.id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    owner = find_by_id(pool, &owner.id)
        .await?
        .ok_or_else(|| GhostError::Internal("owner vanished".into()))?;
    Ok(owner)
}

#[derive(Debug, Default, Clone)]
pub struct UserChanges {
    pub name: Option<String>,
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub location: Option<String>,
    pub profile_image: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub twitter: Option<String>,
    pub facebook: Option<String>,
    pub threads: Option<String>,
    pub bluesky: Option<String>,
    pub mastodon: Option<String>,
    pub tiktok: Option<String>,
    pub youtube: Option<String>,
    pub instagram: Option<String>,
    pub linkedin: Option<String>,
    /// An explicitly edited slug. Ghost lets staff hand-edit the author URL;
    /// when absent (or unchanged) a rename re-slugs from the new name instead.
    pub slug: Option<String>,
    // Notification opt-outs; `None` leaves the flag as it is.
    pub comment_notifications: Option<bool>,
    pub free_member_signup_notification: Option<bool>,
    pub paid_subscription_started_notification: Option<bool>,
    pub paid_subscription_canceled_notification: Option<bool>,
    pub mention_notifications: Option<bool>,
    pub recommendation_notifications: Option<bool>,
    pub milestone_notifications: Option<bool>,
    pub donation_notifications: Option<bool>,
    pub gift_subscription_notifications: Option<bool>,
}

pub async fn update(pool: &SqlitePool, id: &str, changes: &UserChanges) -> Result<User> {
    let existing = find_by_id(pool, id).await?.ok_or(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None })?;
    let now = dates::now_db();
    let name = changes.name.clone().unwrap_or(existing.name.clone().unwrap_or_default());
    let existing_slug = existing.slug.clone().unwrap_or_default();
    let slug_ = match changes.slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if slug::safe_string(s) != existing_slug => unique_slug(pool, s, Some(id)).await?,
        _ if name != existing.name.clone().unwrap_or_default() => {
            unique_slug(pool, &name, Some(id)).await?
        }
        _ => existing_slug,
    };
    sqlx::query("UPDATE users SET name = ?, slug = ?, email = ?, password = ?, bio = ?, website = ?, location = ?, profile_image = ?, cover_image = ?, status = ?, meta_title = ?, meta_description = ?, twitter = ?, facebook = ?, threads = ?, bluesky = ?, mastodon = ?, tiktok = ?, youtube = ?, instagram = ?, linkedin = ?, comment_notifications = ?, free_member_signup_notification = ?, paid_subscription_started_notification = ?, paid_subscription_canceled_notification = ?, mention_notifications = ?, recommendation_notifications = ?, milestone_notifications = ?, donation_notifications = ?, gift_subscription_notifications = ?, updated_at = ? WHERE id = ?")
        .bind(&name)
        .bind(&slug_)
        .bind(changes.email.clone().or(existing.email.clone()))
        .bind(changes.password_hash.clone().or(existing.password.clone()))
        .bind(changes.bio.clone().or(existing.bio.clone()))
        .bind(changes.website.clone().or(existing.website.clone()))
        .bind(changes.location.clone().or(existing.location.clone()))
        .bind(changes.profile_image.clone().or(existing.profile_image.clone()))
        .bind(changes.cover_image.clone().or(existing.cover_image.clone()))
        .bind(changes.status.clone().or(existing.status.clone()))
        .bind(changes.meta_title.clone().or(existing.meta_title.clone()))
        .bind(changes.meta_description.clone().or(existing.meta_description.clone()))
        .bind(changes.twitter.clone().or(existing.twitter.clone()))
        .bind(changes.facebook.clone().or(existing.facebook.clone()))
        .bind(changes.threads.clone().or(existing.threads.clone()))
        .bind(changes.bluesky.clone().or(existing.bluesky.clone()))
        .bind(changes.mastodon.clone().or(existing.mastodon.clone()))
        .bind(changes.tiktok.clone().or(existing.tiktok.clone()))
        .bind(changes.youtube.clone().or(existing.youtube.clone()))
        .bind(changes.instagram.clone().or(existing.instagram.clone()))
        .bind(changes.linkedin.clone().or(existing.linkedin.clone()))
        .bind(changes.comment_notifications.unwrap_or(existing.comment_notifications))
        .bind(changes.free_member_signup_notification.unwrap_or(existing.free_member_signup_notification))
        .bind(changes.paid_subscription_started_notification.unwrap_or(existing.paid_subscription_started_notification))
        .bind(changes.paid_subscription_canceled_notification.unwrap_or(existing.paid_subscription_canceled_notification))
        .bind(changes.mention_notifications.unwrap_or(existing.mention_notifications))
        .bind(changes.recommendation_notifications.unwrap_or(existing.recommendation_notifications))
        .bind(changes.milestone_notifications.unwrap_or(existing.milestone_notifications))
        .bind(changes.donation_notifications.unwrap_or(existing.donation_notifications))
        .bind(changes.gift_subscription_notifications.unwrap_or(existing.gift_subscription_notifications))
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_by_id(pool, id).await?.ok_or_else(|| GhostError::Internal("user vanished".into()))
}

/// Update by slug — the staff profile routes address users that way.
pub async fn update_by_slug(pool: &SqlitePool, slug_: &str, changes: &UserChanges) -> Result<User> {
    let user = find_by_slug(pool, slug_)
        .await?
        .ok_or(ghost_core::GhostError::NotFoundError { message: "not found".into(), context: None })?;
    update(pool, &user.id, changes).await
}

/// Create an additional staff user.
pub async fn insert_staff(pool: &SqlitePool, name: &str, email: &str, password_hash: &str, role: &str) -> Result<User> {
    let id = ids::new_id();
    let now = dates::now_db();
    let slug_ = unique_slug(pool, name, None).await?;
    sqlx::query("INSERT INTO users (id, name, slug, password, email, status, visibility, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'active', 'public', ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(&slug_)
        .bind(password_hash)
        .bind(email)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(dberr)?;
    let role_id: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    if let Some(rid) = role_id {
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
            .bind(&ids::new_id())
            .bind(&rid)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }
    find_by_id(pool, &id).await?.ok_or_else(|| GhostError::Internal("user vanished".into()))
}

pub async fn update_last_seen(pool: &SqlitePool, id: &str) -> Result<()> {
    let now = dates::now_db();
    let _ = sqlx::query("UPDATE users SET last_seen = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await;
    Ok(())
}

pub async fn verify_password(pool: &SqlitePool, email: &str, password: &str) -> Result<Option<User>> {
    let Some(user) = find_by_email(pool, email).await? else { return Ok(None) };
    if user.status.as_deref() != Some("active") {
        return Ok(None);
    }
    let hash = user.password.clone().unwrap_or_default();
    let password = password.to_string();
    let ok = tokio::task::spawn_blocking(move || bcrypt::verify(&password, &hash).unwrap_or(false))
        .await
        .map_err(|e| GhostError::Internal(e.to_string()))?;
    if ok {
        Ok(Some(user))
    } else {
        Ok(None)
    }
}
