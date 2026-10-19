//! Audit log (actions table).
use ghost_core::{dates, ids, Result};
use sqlx::SqlitePool;

pub async fn record(
    pool: &SqlitePool,
    resource_type: &str,
    resource_id: &str,
    actor_id: &str,
    event: &str,
) -> Result<()> {
    let now = dates::now_db();
    let context = serde_json::json!({}).to_string();
    sqlx::query("INSERT INTO actions (id, resource_id, resource_type, actor_id, actor_type, event, context, created_at) VALUES (?, ?, ?, ?, 'user', ?, ?, ?)")
        .bind(&ids::new_id())
        .bind(resource_id)
        .bind(resource_type)
        .bind(actor_id)
        .bind(event)
        .bind(&context)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}
