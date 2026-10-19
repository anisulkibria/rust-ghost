//! Settings repository rows (cache lives in settings_cache.rs).
use ghost_core::{dates, ids, models::Setting, Result};
use sqlx::SqlitePool;

pub async fn all(pool: &SqlitePool) -> Result<Vec<Setting>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, `group`, key, value, type, flags, created_at FROM settings",
    )
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| Setting {
            id: r.0,
            group: r.1,
            key: r.2,
            value: r.3.unwrap_or_default(),
            value_type: r.4.unwrap_or_else(|| "string".into()),
            flags: r.5,
            created_at: r.6,
            updated_at: None,
        })
        .collect())
}

pub async fn upsert(pool: &SqlitePool, key: &str, value: &str, typ: &str) -> Result<()> {
    let now = dates::now_db();
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if exists > 0 {
        sqlx::query("UPDATE settings SET value = ?, type = ?, updated_at = ? WHERE key = ?")
            .bind(value)
            .bind(typ)
            .bind(&now)
            .bind(key)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    } else {
        sqlx::query("INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at) VALUES (?, 'site', ?, ?, ?, ?, ?)")
            .bind(&ids::new_id())
            .bind(key)
            .bind(value)
            .bind(typ)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    }
    Ok(())
}
