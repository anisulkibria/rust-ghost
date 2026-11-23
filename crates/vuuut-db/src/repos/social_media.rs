//! Social media: connected accounts and the shares made to them.
//!
//! Credentials are a JSON object in `social_accounts.credentials`; this repo
//! stores and returns them and nothing else reads them except
//! `vuuut_api::social_media`, which uses them to talk to the platforms. They
//! never go to a browser. Every column is VARCHAR or an integer on MySQL.

use super::dberr;
use crate::Db;
use vuuut_core::{dates, ids, Result};
use serde_json::{Map, Value};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    pub id: String,
    pub network: String,
    pub name: String,
    pub credentials: String,
    /// 0 or 1 — not `bool`, which sqlx `Any` cannot decode from MySQL.
    pub auto_share: i64,
    pub template: String,
    pub status: String,
    pub last_error: String,
    pub last_checked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Account {
    pub fn creds(&self) -> Map<String, Value> {
        serde_json::from_str::<Value>(&self.credentials)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }
    pub fn shares_automatically(&self) -> bool {
        self.auto_share != 0
    }
}

const ACCOUNT_COLS: &str = "id, network, name, credentials, auto_share, template, status, last_error, last_checked_at, created_at, updated_at";

pub async fn accounts(pool: &Db) -> Result<Vec<Account>> {
    sqlx::query_as::<_, Account>(&format!("SELECT {ACCOUNT_COLS} FROM social_accounts ORDER BY created_at"))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

pub async fn account(pool: &Db, id: &str) -> Result<Option<Account>> {
    sqlx::query_as::<_, Account>(&format!("SELECT {ACCOUNT_COLS} FROM social_accounts WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

pub async fn add_account(pool: &Db, network: &str, name: &str, creds: &Map<String, Value>, auto_share: bool) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO social_accounts (id, network, name, credentials, auto_share, template, status, last_error, created_at, updated_at) \
         VALUES (?,?,?,?,?,'','untested','',?,?)",
    )
    .bind(&id)
    .bind(network)
    .bind(name)
    .bind(Value::Object(creds.clone()).to_string())
    .bind(if auto_share { 1_i64 } else { 0 })
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

pub async fn update_account(pool: &Db, id: &str, name: &str, creds: &Map<String, Value>, auto_share: bool, template: &str) -> Result<()> {
    sqlx::query("UPDATE social_accounts SET name = ?, credentials = ?, auto_share = ?, template = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(Value::Object(creds.clone()).to_string())
        .bind(if auto_share { 1_i64 } else { 0 })
        .bind(template)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// The outcome of a test or a share, so the account list shows whether it
/// currently works.
pub async fn set_health(pool: &Db, id: &str, ok: bool, error: &str, name: Option<&str>) {
    let error: String = error.chars().take(500).collect();
    let _ = match name {
        Some(n) => sqlx::query("UPDATE social_accounts SET status = ?, last_error = ?, last_checked_at = ?, name = ? WHERE id = ?")
            .bind(if ok { "ok" } else { "error" })
            .bind(&error)
            .bind(dates::now_db())
            .bind(n)
            .bind(id)
            .execute(pool)
            .await,
        None => sqlx::query("UPDATE social_accounts SET status = ?, last_error = ?, last_checked_at = ? WHERE id = ?")
            .bind(if ok { "ok" } else { "error" })
            .bind(&error)
            .bind(dates::now_db())
            .bind(id)
            .execute(pool)
            .await,
    };
}

/// Disconnects the account. Its share history goes too: rows pointing at an
/// account that no longer exists could not be retried or explained.
pub async fn delete_account(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM social_shares WHERE account_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM social_accounts WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// shares
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Share {
    pub id: String,
    pub account_id: String,
    pub post_id: String,
    pub body: String,
    pub status: String,
    pub trigger_kind: String,
    pub send_at: Option<String>,
    pub sent_at: Option<String>,
    pub remote_id: String,
    pub remote_url: String,
    pub error: String,
    pub attempts: i64,
    pub created_by: String,
    pub created_at: String,
}

const SHARE_COLS: &str = "id, account_id, post_id, body, status, trigger_kind, send_at, sent_at, remote_id, remote_url, error, attempts, created_by, created_at";

/// A new share: `queued` to go now, or `scheduled` for `send_at`.
pub async fn add_share(pool: &Db, account_id: &str, post_id: &str, body: &str, trigger: &str, send_at: Option<&str>, by: &str) -> Result<String> {
    let id = ids::new_id();
    sqlx::query(&format!(
        "INSERT INTO social_shares ({SHARE_COLS}) VALUES (?,?,?,?,?,?,?,NULL,'','','',0,?,?)"
    ))
    .bind(&id)
    .bind(account_id)
    .bind(post_id)
    .bind(body)
    .bind(if send_at.is_some() { "scheduled" } else { "queued" })
    .bind(trigger)
    .bind(send_at)
    .bind(by)
    .bind(dates::now_db())
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

pub async fn share(pool: &Db, id: &str) -> Result<Option<Share>> {
    sqlx::query_as::<_, Share>(&format!("SELECT {SHARE_COLS} FROM social_shares WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// Claim a share for sending: only one sender ever gets it, because the
/// status changes from `queued`/`scheduled`/`failed` to `sending` in one
/// statement and only the caller whose update landed proceeds.
pub async fn claim(pool: &Db, id: &str) -> bool {
    let done = sqlx::query(
        "UPDATE social_shares SET status = 'sending', attempts = attempts + 1 WHERE id = ? AND status IN ('queued', 'scheduled', 'failed')",
    )
    .bind(id)
    .execute(pool)
    .await;
    matches!(done, Ok(r) if r.rows_affected() == 1)
}

pub async fn finish(pool: &Db, id: &str, result: std::result::Result<(String, String), String>) {
    let _ = match result {
        Ok((remote_id, remote_url)) => {
            sqlx::query("UPDATE social_shares SET status = 'sent', sent_at = ?, remote_id = ?, remote_url = ?, error = '' WHERE id = ?")
                .bind(dates::now_db())
                .bind(remote_id.chars().take(191).collect::<String>())
                .bind(remote_url.chars().take(500).collect::<String>())
                .bind(id)
                .execute(pool)
                .await
        }
        Err(e) => {
            sqlx::query("UPDATE social_shares SET status = 'failed', error = ? WHERE id = ?")
                .bind(e.chars().take(500).collect::<String>())
                .bind(id)
                .execute(pool)
                .await
        }
    };
}

/// Scheduled shares whose time has come.
pub async fn due(pool: &Db, now: &str) -> Vec<Share> {
    sqlx::query_as::<_, Share>(&format!(
        "SELECT {SHARE_COLS} FROM social_shares WHERE status = 'scheduled' AND send_at <= ? ORDER BY send_at LIMIT 50"
    ))
    .bind(now)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Newest first, optionally one status.
pub async fn history(pool: &Db, status: Option<&str>, limit: i64) -> Result<Vec<Share>> {
    let limit = limit.clamp(1, 1000);
    match status {
        Some(s) => sqlx::query_as::<_, Share>(&format!("SELECT {SHARE_COLS} FROM social_shares WHERE status = ? ORDER BY created_at DESC, id DESC LIMIT ?"))
            .bind(s)
            .bind(limit)
            .fetch_all(pool)
            .await,
        None => sqlx::query_as::<_, Share>(&format!("SELECT {SHARE_COLS} FROM social_shares ORDER BY created_at DESC, id DESC LIMIT ?"))
            .bind(limit)
            .fetch_all(pool)
            .await,
    }
    .map_err(dberr)
}

pub async fn of_post(pool: &Db, post_id: &str) -> Vec<Share> {
    sqlx::query_as::<_, Share>(&format!("SELECT {SHARE_COLS} FROM social_shares WHERE post_id = ? ORDER BY created_at DESC"))
        .bind(post_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Whether this account already shared this post automatically — so a post
/// unpublished and published again is not announced twice.
pub async fn already_auto_shared(pool: &Db, account_id: &str, post_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM social_shares WHERE account_id = ? AND post_id = ? AND trigger_kind = 'auto'")
        .bind(account_id)
        .bind(post_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

pub async fn cancel(pool: &Db, id: &str) -> bool {
    let done = sqlx::query("DELETE FROM social_shares WHERE id = ? AND status IN ('scheduled', 'failed', 'queued')")
        .bind(id)
        .execute(pool)
        .await;
    matches!(done, Ok(r) if r.rows_affected() == 1)
}

/// `(status, count)` across all shares.
pub async fn counts(pool: &Db) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM social_shares GROUP BY status")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// A post's public tag names, in its own order — for hashtags.
pub async fn post_tag_names(pool: &Db, post_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT tags.name FROM posts_tags JOIN tags ON tags.id = posts_tags.tag_id \
         WHERE posts_tags.post_id = ? AND tags.visibility = 'public' ORDER BY posts_tags.sort_order",
    )
    .bind(post_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Recently published posts, for the composer's picker: `(id, title, published_at)`.
pub async fn recent_posts(pool: &Db, limit: i64) -> Vec<(String, Option<String>, Option<String>)> {
    sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT id, title, published_at FROM posts WHERE status IN ('published', 'sent') AND type = 'post' \
         ORDER BY published_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}
