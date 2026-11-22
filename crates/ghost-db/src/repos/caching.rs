//! Caching & Performance: the CDN connection. The page cache itself lives in
//! memory (`ghost_api::caching`); only what must survive a restart is here.

use super::dberr;
use crate::Db;
use ghost_core::{dates, Result};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Cdn {
    pub provider: String,
    pub zone: String,
    pub token: String,
    pub last_purge: Option<String>,
    pub last_error: String,
}

impl Cdn {
    pub fn connected(&self) -> bool {
        !self.zone.is_empty() && !self.token.is_empty()
    }
}

pub async fn cdn(pool: &Db) -> Option<Cdn> {
    sqlx::query_as::<_, Cdn>("SELECT provider, zone, token, last_purge, last_error FROM cdn_config WHERE id = 'default'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Save the connection. A blank token keeps the stored one — the form never
/// shows it back.
pub async fn save_cdn(pool: &Db, zone: &str, token: Option<&str>) -> Result<()> {
    let now = dates::now_db();
    let existing = cdn(pool).await;
    let token = token.map(str::to_string).or_else(|| existing.as_ref().map(|c| c.token.clone())).unwrap_or_default();
    if existing.is_some() {
        sqlx::query("UPDATE cdn_config SET zone = ?, token = ?, updated_at = ? WHERE id = 'default'")
            .bind(zone)
            .bind(&token)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(dberr)?;
    } else {
        sqlx::query("INSERT INTO cdn_config (id, provider, zone, token, last_error, updated_at) VALUES ('default', 'cloudflare', ?, ?, '', ?)")
            .bind(zone)
            .bind(&token)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(dberr)?;
    }
    Ok(())
}

pub async fn forget_cdn(pool: &Db) -> Result<()> {
    sqlx::query("DELETE FROM cdn_config WHERE id = 'default'").execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn record_purge(pool: &Db, error: &str) {
    let _ = sqlx::query("UPDATE cdn_config SET last_purge = ?, last_error = ? WHERE id = 'default'")
        .bind(dates::now_db())
        .bind(error.chars().take(300).collect::<String>())
        .execute(pool)
        .await;
}
