//! Integrations: the builtin apps' keys, and custom integration CRUD.
use ghost_core::models::Integration;
use ghost_core::{dates, ids, Result};
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

// ---------------------------------------------------------------------------
// custom integrations
// ---------------------------------------------------------------------------

/// Every custom integration, newest first, with its description.
///
/// `list` above is kept for callers that only want the name/slug pair; this
/// one carries the fields the integrations screen shows.
pub async fn list_custom(pool: &SqlitePool) -> Result<Vec<Integration>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, type, name, slug, description, created_at FROM integrations \
         WHERE type = 'custom' ORDER BY created_at DESC, name",
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
            description: r.4,
            created_at: r.5,
            updated_at: None,
        })
        .collect())
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Integration>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, type, name, slug, description, created_at FROM integrations WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    Ok(row.map(|r| Integration {
        id: r.0,
        integration_type: r.1,
        name: r.2,
        slug: r.3,
        icon_image: None,
        description: r.4,
        created_at: r.5,
        updated_at: None,
    }))
}

/// Create a custom integration with both API keys.
///
/// Ghost gives every custom integration a content key *and* an admin key,
/// because which one a script needs depends on whether it only reads published
/// content or also writes. Handing out both up front saves a round trip
/// through this screen later.
pub async fn create_custom(pool: &SqlitePool, name: &str, description: Option<&str>) -> Result<Integration> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ghost_core::GhostError::ValidationError {
            message: "an integration needs a name".into(),
            context: None,
        });
    }
    let now = dates::now_db();
    let id = ids::new_id();
    let slug = unique_slug(pool, &ghost_core::slug::safe_string(name)).await?;

    sqlx::query(
        "INSERT INTO integrations (id, type, name, slug, description, created_at, updated_at) \
         VALUES (?, 'custom', ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(&slug)
    .bind(description.map(str::trim).filter(|d| !d.is_empty()))
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(super::dberr)?;

    let admin_role: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Admin Integration'")
        .fetch_optional(pool)
        .await
        .map_err(super::dberr)?;
    // Lengths match Ghost's: a 26-char content key, and an admin key that is
    // an `id:secret` pair where the secret half is 64 hex characters.
    for (key_type, secret, role) in [
        ("content", ids::random_hex(13), None),
        ("admin", ids::random_hex(32), admin_role),
    ] {
        sqlx::query(
            "INSERT INTO api_keys (id, type, secret, role_id, integration_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(key_type)
        .bind(&secret)
        .bind(&role)
        .bind(&id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    }

    Ok(Integration {
        id,
        integration_type: "custom".into(),
        name: name.to_string(),
        slug: Some(slug),
        icon_image: None,
        description: description.map(str::to_string),
        created_at: Some(now),
        updated_at: None,
    })
}

/// The slug has a UNIQUE constraint, so two integrations called "Zapier Copy"
/// would otherwise fail the second insert rather than both being created.
async fn unique_slug(pool: &SqlitePool, base: &str) -> Result<String> {
    let base = if base.is_empty() { "integration" } else { base };
    for suffix in 0..100 {
        let candidate = match suffix {
            0 => base.to_string(),
            n => format!("{base}-{n}"),
        };
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM integrations WHERE slug = ?")
            .bind(&candidate)
            .fetch_one(pool)
            .await
            .map_err(super::dberr)?;
        if taken == 0 {
            return Ok(candidate);
        }
    }
    Ok(format!("{base}-{}", ids::random_hex(4)))
}

pub async fn update_custom(pool: &SqlitePool, id: &str, name: &str, description: Option<&str>) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ghost_core::GhostError::ValidationError {
            message: "an integration needs a name".into(),
            context: None,
        });
    }
    sqlx::query("UPDATE integrations SET name = ?, description = ?, updated_at = ? WHERE id = ? AND type = 'custom'")
        .bind(name)
        .bind(description.map(str::trim).filter(|d| !d.is_empty()))
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}

/// Delete a custom integration and its keys.
///
/// Only `custom` rows can go: the internal and builtin ones carry keys the
/// server itself authenticates with, and removing those breaks the frontend's
/// own Content API access.
pub async fn delete_custom(pool: &SqlitePool, id: &str) -> Result<()> {
    let kind: Option<String> = sqlx::query_scalar("SELECT type FROM integrations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(super::dberr)?;
    match kind.as_deref() {
        Some("custom") => {}
        Some(other) => {
            return Err(ghost_core::GhostError::ValidationError {
                message: format!("a {other} integration cannot be deleted"),
                context: None,
            })
        }
        None => {
            return Err(ghost_core::GhostError::NotFoundError {
                message: "integration not found".into(),
                context: None,
            })
        }
    }
    sqlx::query("DELETE FROM api_keys WHERE integration_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    sqlx::query("DELETE FROM webhooks WHERE integration_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    sqlx::query("DELETE FROM integrations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(super::dberr)?;
    Ok(())
}

/// The admin key of a builtin integration, by slug — what Zapier's screen
/// shows for pasting into Zapier itself.
pub async fn admin_key_for_slug(pool: &SqlitePool, slug: &str) -> Result<Option<String>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT k.id, k.secret FROM api_keys k JOIN integrations i ON i.id = k.integration_id \
         WHERE i.slug = ? AND k.type = 'admin' LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(super::dberr)?;
    // Ghost presents an admin key as `id:secret`; both halves are needed to
    // sign a JWT, so the screen has to show them joined.
    Ok(row.map(|(id, secret)| format!("{id}:{secret}")))
}
