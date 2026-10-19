//! DB-backed admin sessions (express-session equivalent).
use ghost_core::{dates, ids, Result};
use sqlx::SqlitePool;

pub async fn create(pool: &SqlitePool, user_id: &str) -> Result<String> {
    let session_id = ids::random_hex(16); // 32 chars
    let now = dates::now_db();
    let data = serde_json::json!({ "passport": { "user": user_id } }).to_string();
    sqlx::query("INSERT INTO sessions (id, session_id, user_id, session_data, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&ids::new_id())
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

pub async fn get_user(pool: &SqlitePool, session_id: &str) -> Result<Option<String>> {
    let user_id: Option<String> =
        sqlx::query_scalar("SELECT user_id FROM sessions WHERE session_id = ?")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .map_err(super::dberr)?;
    Ok(user_id)
}

pub async fn destroy(pool: &SqlitePool, session_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE session_id = ?")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}
