//! API keys + integrations repository.
use ghost_core::{dates, ids, models::{ApiKey, Integration}, Result};
use sqlx::SqlitePool;

pub async fn find_by_secret(pool: &SqlitePool, secret: &str) -> Result<Option<ApiKey>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, type, secret, role_id, integration_id, user_id FROM api_keys WHERE secret = ? LIMIT 1",
    )
    .bind(secret)
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    Ok(row.map(|r| ApiKey {
        id: r.0,
        key_type: r.1,
        secret: r.2,
        role_id: r.3,
        integration_id: r.4,
        user_id: r.5,
        last_seen_at: None,
        last_seen_version: None,
        created_at: None,
        updated_at: None,
    }))
}

pub async fn find_by_integration(pool: &SqlitePool, integration_id: &str) -> Result<Vec<ApiKey>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, type, secret, role_id, integration_id, user_id FROM api_keys WHERE integration_id = ?",
    )
    .bind(integration_id)
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| ApiKey {
            id: r.0,
            key_type: r.1,
            secret: r.2,
            role_id: r.3,
            integration_id: r.4,
            user_id: r.5,
            last_seen_at: None,
            last_seen_version: None,
            created_at: None,
            updated_at: None,
        })
        .collect())
}

/// Create an integration with a fresh content or admin key.
pub async fn create_integration(
    pool: &SqlitePool,
    name: &str,
    key_type: &str,
) -> Result<(Integration, ApiKey)> {
    let now = dates::now_db();
    let integration_id = ids::new_id();
    let slug = ghost_core::slug::safe_string(name);
    sqlx::query("INSERT INTO integrations (id, type, name, slug, created_at, updated_at) VALUES (?, 'custom', ?, ?, ?, ?)")
        .bind(&integration_id)
        .bind(name)
        .bind(&slug)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(super::dberr)?;

    let key_id = ids::new_id();
    let secret = if key_type == "admin" { ids::random_hex(32) } else { ids::random_hex(13) };
    let role_id: Option<String> = if key_type == "admin" {
        sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Admin Integration'")
            .fetch_optional(pool)
            .await
            .map_err(super::dberr)?
    } else {
        None
    };
    sqlx::query("INSERT INTO api_keys (id, type, secret, role_id, integration_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&key_id)
        .bind(key_type)
        .bind(&secret)
        .bind(&role_id)
        .bind(&integration_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(super::dberr)?;

    let integration = Integration {
        id: integration_id.clone(),
        integration_type: "custom".into(),
        name: name.to_string(),
        slug: Some(slug),
        icon_image: None,
        description: None,
        created_at: Some(now),
        updated_at: None,
    };
    let key = ApiKey {
        id: key_id,
        key_type: key_type.to_string(),
        secret,
        role_id,
        integration_id: Some(integration_id),
        user_id: None,
        last_seen_at: None,
        last_seen_version: None,
        created_at: None,
        updated_at: None,
    };
    Ok((integration, key))
}
