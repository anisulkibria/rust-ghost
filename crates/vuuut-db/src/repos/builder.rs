//! Page Builder & Visual Editor: layouts, their revisions, and templates.
//!
//! `data` and `html` are long columns (`MEDIUMTEXT` on MySQL) and are read
//! through [`crate::types::Text`]. Every write that can change a public page
//! calls [`crate::content::touch`] so the page cache lets go of it.
//!
//! Nothing here validates a layout: callers pass JSON that has already been
//! through `vuuut_core::builder::normalise` and HTML from `render`.

use super::dberr;
use crate::types::Text;
use crate::Db;
use vuuut_core::{dates, ids, Result};
use sqlx::Row;

/// How many earlier versions of a layout are kept.
pub const KEEP_REVISIONS: i64 = 30;

#[derive(Debug, Clone)]
pub struct Layout {
    pub post_id: String,
    pub data: String,
    pub html: String,
    pub active: bool,
    pub updated_by: String,
    pub updated_at: String,
}

fn layout_from(row: &sqlx::any::AnyRow) -> Layout {
    Layout {
        post_id: row.try_get("post_id").unwrap_or_default(),
        data: row.try_get::<Text, _>("data").map(|t| t.0).unwrap_or_default(),
        html: row.try_get::<Text, _>("html").map(|t| t.0).unwrap_or_default(),
        active: row.try_get::<i64, _>("active").unwrap_or(0) != 0,
        updated_by: row.try_get("updated_by").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

pub async fn get(pool: &Db, post_id: &str) -> Result<Option<Layout>> {
    let row = sqlx::query("SELECT post_id, data, html, active, updated_by, updated_at FROM builder_layouts WHERE post_id = ?")
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row.as_ref().map(layout_from))
}

/// The HTML the site should draw for a post, when it has a layout that is
/// switched on. One indexed read per page view.
pub async fn active_html(pool: &Db, post_id: &str) -> Option<String> {
    sqlx::query("SELECT html FROM builder_layouts WHERE post_id = ? AND active = 1")
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<Text, _>("html").ok().map(|t| t.0))
}

/// Store a layout. The version it replaces goes to `builder_revisions` (when
/// it differs), and only the newest [`KEEP_REVISIONS`] are kept.
pub async fn save(pool: &Db, post_id: &str, data: &str, html: &str, active: bool, user_id: &str) -> Result<()> {
    let now = dates::now_db();
    let before = get(pool, post_id).await?;
    let mut tx = pool.begin().await.map_err(dberr)?;
    match &before {
        Some(old) => {
            if old.data != data {
                sqlx::query("INSERT INTO builder_revisions (id, post_id, data, created_by, created_at) VALUES (?,?,?,?,?)")
                    .bind(ids::new_id())
                    .bind(post_id)
                    .bind(&old.data)
                    .bind(&old.updated_by)
                    .bind(&old.updated_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr)?;
            }
            sqlx::query("UPDATE builder_layouts SET data = ?, html = ?, active = ?, updated_by = ?, updated_at = ? WHERE post_id = ?")
                .bind(data)
                .bind(html)
                .bind(active as i64)
                .bind(user_id)
                .bind(&now)
                .bind(post_id)
                .execute(&mut *tx)
                .await
                .map_err(dberr)?;
        }
        None => {
            sqlx::query("INSERT INTO builder_layouts (post_id, data, html, active, updated_by, created_at, updated_at) VALUES (?,?,?,?,?,?,?)")
                .bind(post_id)
                .bind(data)
                .bind(html)
                .bind(active as i64)
                .bind(user_id)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await
                .map_err(dberr)?;
        }
    }
    tx.commit().await.map_err(dberr)?;
    prune(pool, post_id).await;
    crate::content::touch();
    Ok(())
}

async fn prune(pool: &Db, post_id: &str) {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM builder_revisions WHERE post_id = ? ORDER BY created_at DESC, id DESC")
        .bind(post_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    for id in ids.into_iter().skip(KEEP_REVISIONS as usize) {
        let _ = sqlx::query("DELETE FROM builder_revisions WHERE id = ?").bind(id).execute(pool).await;
    }
}

pub async fn set_active(pool: &Db, post_id: &str, active: bool) -> Result<bool> {
    let r = sqlx::query("UPDATE builder_layouts SET active = ? WHERE post_id = ?")
        .bind(active as i64)
        .bind(post_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    crate::content::touch();
    Ok(r.rows_affected() > 0)
}

/// Remove a post's layout and its history; the post goes back to its own
/// content.
pub async fn delete(pool: &Db, post_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM builder_revisions WHERE post_id = ?").bind(post_id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM builder_layouts WHERE post_id = ?").bind(post_id).execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

/// One row of the overview: a layout and the post it belongs to.
#[derive(Debug, Clone)]
pub struct Listed {
    pub post_id: String,
    pub title: String,
    pub slug: String,
    pub post_type: String,
    pub status: String,
    pub uuid: String,
    pub active: bool,
    pub data: String,
    pub updated_at: String,
}

pub async fn list(pool: &Db) -> Result<Vec<Listed>> {
    let rows = sqlx::query(
        "SELECT b.post_id, b.data, b.active, b.updated_at, p.title, p.slug, p.type, p.status, p.uuid \
         FROM builder_layouts b JOIN posts p ON p.id = b.post_id ORDER BY b.updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows
        .iter()
        .map(|r| Listed {
            post_id: r.try_get("post_id").unwrap_or_default(),
            title: r.try_get::<Option<String>, _>("title").ok().flatten().unwrap_or_default(),
            slug: r.try_get::<Option<String>, _>("slug").ok().flatten().unwrap_or_default(),
            post_type: r.try_get("type").unwrap_or_default(),
            status: r.try_get("status").unwrap_or_default(),
            uuid: r.try_get::<Option<String>, _>("uuid").ok().flatten().unwrap_or_default(),
            active: r.try_get::<i64, _>("active").unwrap_or(0) != 0,
            data: r.try_get::<Text, _>("data").map(|t| t.0).unwrap_or_default(),
            updated_at: r.try_get("updated_at").unwrap_or_default(),
        })
        .collect())
}

/// Every layout, for re-rendering after the global settings change.
pub async fn all_data(pool: &Db) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query("SELECT post_id, data FROM builder_layouts").fetch_all(pool).await.map_err(dberr)?;
    Ok(rows
        .iter()
        .map(|r| (r.try_get("post_id").unwrap_or_default(), r.try_get::<Text, _>("data").map(|t| t.0).unwrap_or_default()))
        .collect())
}

pub async fn set_html(pool: &Db, post_id: &str, html: &str) -> Result<()> {
    sqlx::query("UPDATE builder_layouts SET html = ? WHERE post_id = ?").bind(html).bind(post_id).execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

// ---------------------------------------------------------------------------
// revisions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Revision {
    pub id: String,
    pub created_by: String,
    pub created_at: String,
}

pub async fn revisions(pool: &Db, post_id: &str) -> Result<Vec<Revision>> {
    let rows = sqlx::query("SELECT id, created_by, created_at FROM builder_revisions WHERE post_id = ? ORDER BY created_at DESC, id DESC")
        .bind(post_id)
        .fetch_all(pool)
        .await
        .map_err(dberr)?;
    Ok(rows
        .iter()
        .map(|r| Revision {
            id: r.try_get("id").unwrap_or_default(),
            created_by: r.try_get("created_by").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

/// One revision's layout — only if it belongs to this post.
pub async fn revision(pool: &Db, post_id: &str, id: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT data FROM builder_revisions WHERE id = ? AND post_id = ?")
        .bind(id)
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row.and_then(|r| r.try_get::<Text, _>("data").ok().map(|t| t.0)))
}

// ---------------------------------------------------------------------------
// templates: saved sections and pages, global widgets, Theme Builder parts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub data: String,
    pub options: String,
    pub active: bool,
    pub html: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

const T_COLS: &str = "id, name, kind, data, options, active, html, created_by, created_at, updated_at";

fn template_from(r: &sqlx::any::AnyRow) -> Template {
    Template {
        id: r.try_get("id").unwrap_or_default(),
        name: r.try_get("name").unwrap_or_default(),
        kind: r.try_get("kind").unwrap_or_default(),
        data: r.try_get::<Text, _>("data").map(|t| t.0).unwrap_or_default(),
        options: r.try_get("options").unwrap_or_else(|_| "{}".into()),
        active: r.try_get::<i64, _>("active").unwrap_or(0) != 0,
        html: r.try_get::<Option<Text>, _>("html").ok().flatten().map(|t| t.0).unwrap_or_default(),
        created_by: r.try_get("created_by").unwrap_or_default(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }
}

pub async fn templates(pool: &Db) -> Result<Vec<Template>> {
    let rows = sqlx::query(&format!("SELECT {T_COLS} FROM builder_templates ORDER BY kind, name")).fetch_all(pool).await.map_err(dberr)?;
    Ok(rows.iter().map(template_from).collect())
}

pub async fn template(pool: &Db, id: &str) -> Result<Option<Template>> {
    let row = sqlx::query(&format!("SELECT {T_COLS} FROM builder_templates WHERE id = ?")).bind(id).fetch_optional(pool).await.map_err(dberr)?;
    Ok(row.as_ref().map(template_from))
}

/// Every template's layout by id, for the Template, Global widget and
/// Off-canvas widgets when a layout is drawn.
pub async fn templates_map(pool: &Db) -> std::collections::HashMap<String, serde_json::Value> {
    let rows = sqlx::query("SELECT id, data FROM builder_templates").fetch_all(pool).await.unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let data = r.try_get::<Text, _>("data").ok()?.0;
            Some((id, serde_json::from_str(&data).ok()?))
        })
        .collect()
}

/// The switched-on Theme Builder parts of one kind: `(id, html, options)`.
pub async fn active_parts(pool: &Db, kind: &str) -> Vec<(String, String, String)> {
    sqlx::query("SELECT id, html, options FROM builder_templates WHERE kind = ? AND active = 1 ORDER BY updated_at DESC")
        .bind(kind)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| {
            (
                r.try_get("id").unwrap_or_default(),
                r.try_get::<Option<Text>, _>("html").ok().flatten().map(|t| t.0).unwrap_or_default(),
                r.try_get("options").unwrap_or_else(|_| "{}".into()),
            )
        })
        .collect()
}

pub async fn create_template(pool: &Db, name: &str, kind: &str, data: &str, user_id: &str) -> Result<String> {
    create_template_full(pool, name, kind, data, "{}", "", false, user_id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_template_full(pool: &Db, name: &str, kind: &str, data: &str, options: &str, html: &str, active: bool, user_id: &str) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO builder_templates (id, name, kind, data, options, active, html, created_by, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?)")
        .bind(&id)
        .bind(name)
        .bind(kind)
        .bind(data)
        .bind(options)
        .bind(active as i64)
        .bind(html)
        .bind(user_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(dberr)?;
    crate::content::touch();
    Ok(id)
}

pub async fn update_template(pool: &Db, id: &str, name: &str, data: &str, options: &str, html: &str, active: bool) -> Result<bool> {
    let r = sqlx::query("UPDATE builder_templates SET name = ?, data = ?, options = ?, html = ?, active = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(data)
        .bind(options)
        .bind(html)
        .bind(active as i64)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    crate::content::touch();
    Ok(r.rows_affected() > 0)
}

pub async fn set_template_html(pool: &Db, id: &str, html: &str) -> Result<()> {
    sqlx::query("UPDATE builder_templates SET html = ? WHERE id = ?").bind(html).bind(id).execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

pub async fn set_template_active(pool: &Db, id: &str, active: bool) -> Result<bool> {
    let r = sqlx::query("UPDATE builder_templates SET active = ? WHERE id = ?").bind(active as i64).bind(id).execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(r.rows_affected() > 0)
}

pub async fn rename_template(pool: &Db, id: &str, name: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE builder_templates SET name = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(r.rows_affected() > 0)
}

pub async fn delete_template(pool: &Db, id: &str) -> Result<bool> {
    let r = sqlx::query("DELETE FROM builder_templates WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(r.rows_affected() > 0)
}
