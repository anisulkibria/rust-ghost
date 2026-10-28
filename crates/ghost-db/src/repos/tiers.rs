//! Membership tier visibility shared by the portal settings and tier editor.
use ghost_core::Result;
use sqlx::SqlitePool;

/// Only current grants count. Expired gifts/complimentary grants must not
/// unlock a tier even if the cached member status has not been swept yet.
pub async fn member_tier_ids(pool: &SqlitePool, member_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT DISTINCT product_id FROM members_products WHERE member_id = ? AND (expiry_at IS NULL OR datetime(expiry_at) > datetime('now'))")
        .bind(member_id).fetch_all(pool).await.map_err(super::dberr)
}

pub async fn post_tier_ids(pool: &SqlitePool, post_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT product_id FROM posts_products WHERE post_id = ? ORDER BY sort_order")
        .bind(post_id).fetch_all(pool).await.map_err(super::dberr)
}

pub async fn validate_post_tiers(pool: &SqlitePool, visibility: &str, ids: &[String]) -> Result<()> {
    let invalid = |message: &str| ghost_core::GhostError::ValidationError { message: message.into(), context: Some("tiers".into()) };
    if !matches!(visibility, "public" | "members" | "paid" | "tiers") { return Err(invalid("Invalid post visibility.")); }
    for id in ids {
        let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE id = ? AND type = 'paid')")
            .bind(id).fetch_one(pool).await.map_err(super::dberr)?;
        if !found { return Err(invalid("A selected paid tier does not exist.")); }
    }
    Ok(())
}

pub async fn set_free_signup_visibility(pool: &SqlitePool, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE products SET visibility = ?, active = CASE WHEN ? THEN 1 ELSE active END, updated_at = ? WHERE type = 'free'")
        .bind(if enabled { "public" } else { "none" }).bind(enabled)
        .bind(ghost_core::dates::now_db()).execute(pool).await.map_err(super::dberr)?;
    Ok(())
}
