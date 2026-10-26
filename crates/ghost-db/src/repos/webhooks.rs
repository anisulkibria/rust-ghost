//! Webhooks: the callbacks a custom integration registers against site events.
use ghost_core::{dates, ids, GhostError, Result};
use sqlx::SqlitePool;

use super::dberr;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Webhook {
    pub id: String,
    pub event: String,
    pub target_url: String,
    pub name: Option<String>,
    pub secret: Option<String>,
    pub integration_id: String,
    pub last_triggered_at: Option<String>,
    pub last_triggered_status: Option<String>,
    pub last_triggered_error: Option<String>,
    pub created_at: Option<String>,
}

const COLS: &str = "id, event, target_url, name, secret, integration_id, last_triggered_at, \
     last_triggered_status, last_triggered_error, created_at";

/// Every webhook an integration has registered.
pub async fn list_for_integration(pool: &SqlitePool, integration_id: &str) -> Result<Vec<Webhook>> {
    let rows = sqlx::query_as::<_, Webhook>(&format!(
        "SELECT {COLS} FROM webhooks WHERE integration_id = ? ORDER BY created_at"
    ))
    .bind(integration_id)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

/// Every webhook listening for one event, across all integrations — what the
/// dispatcher asks for when something happens.
pub async fn list_for_event(pool: &SqlitePool, event: &str) -> Result<Vec<Webhook>> {
    let rows = sqlx::query_as::<_, Webhook>(&format!(
        "SELECT {COLS} FROM webhooks WHERE event = ?"
    ))
    .bind(event)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

pub async fn create(
    pool: &SqlitePool,
    integration_id: &str,
    event: &str,
    target_url: &str,
    name: &str,
    secret: &str,
) -> Result<Webhook> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO webhooks (id, event, target_url, name, secret, api_version, integration_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'v5', ?, ?, ?)",
    )
    .bind(&id)
    .bind(event)
    .bind(target_url)
    .bind(name)
    .bind(secret)
    .bind(integration_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    find(pool, &id).await?.ok_or_else(|| GhostError::Internal("webhook vanished".into()))
}

pub async fn find(pool: &SqlitePool, id: &str) -> Result<Option<Webhook>> {
    let row = sqlx::query_as::<_, Webhook>(&format!("SELECT {COLS} FROM webhooks WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row)
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM webhooks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Record the outcome of a delivery, which is what the admin row reports.
///
/// `error` is bounded to the column's width rather than left to the database
/// to reject — a long upstream error must not lose the status too.
pub async fn record_delivery(pool: &SqlitePool, id: &str, status: &str, error: Option<&str>) {
    let trimmed = error.map(|e| e.chars().take(50).collect::<String>());
    let _ = sqlx::query(
        "UPDATE webhooks SET last_triggered_at = ?, last_triggered_status = ?, last_triggered_error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(dates::now_db())
    .bind(status.chars().take(50).collect::<String>())
    .bind(trimmed)
    .bind(dates::now_db())
    .bind(id)
    .execute(pool)
    .await;
}
