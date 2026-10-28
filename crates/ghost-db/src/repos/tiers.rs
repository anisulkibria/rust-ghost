//! Membership tier visibility shared by the portal settings and tier editor.
use ghost_core::Result;
use sqlx::SqlitePool;

pub async fn set_free_signup_visibility(pool: &SqlitePool, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE products SET visibility = ?, active = CASE WHEN ? THEN 1 ELSE active END, updated_at = ? WHERE type = 'free'")
        .bind(if enabled { "public" } else { "none" }).bind(enabled)
        .bind(ghost_core::dates::now_db()).execute(pool).await.map_err(super::dberr)?;
    Ok(())
}
