//! Integrations read helpers.
use ghost_core::models::Integration;
use ghost_core::{Result};
use sqlx::SqlitePool;

pub async fn list(pool: &SqlitePool) -> Result<Vec<Integration>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, type, name, slug FROM integrations WHERE type = 'custom' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| Integration {
            id: r.0,
            integration_type: r.1,
            name: r.2,
            slug: r.3,
            icon_image: None,
            description: None,
            created_at: None,
            updated_at: None,
        })
        .collect())
}

pub async fn find_by_slug(pool: &SqlitePool, slug: &str) -> Result<Option<Integration>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, type, name, slug FROM integrations WHERE slug = ? LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    Ok(row.map(|r| Integration {
        id: r.0,
        integration_type: r.1,
        name: r.2,
        slug: r.3,
        icon_image: None,
        description: None,
        created_at: None,
        updated_at: None,
    }))
}

/// The content API key secret used for the internal frontend integration
/// (handy first-run credential shown in boot logs / admin UI).
pub async fn internal_content_key(pool: &SqlitePool) -> Result<Option<String>> {
    let secret: Option<String> = sqlx::query_scalar(
        "SELECT k.secret FROM api_keys k JOIN integrations i ON i.id = k.integration_id WHERE i.slug = 'ghost-core-content' AND k.type = 'content' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    Ok(secret)
}
