//! Portfolio: projects and case studies, disciplines, services, clients,
//! testimonials, the team, the résumé (entries and skills) and enquiries.
//!
//! Every value is bound. Long columns are MEDIUMTEXT on MySQL and are read
//! through [`crate::types::Text`]. Writes that change a public page call
//! `content::touch()`. A project's password is stored hashed
//! (`vuuut_core::portfolio::hash_password`) and never leaves this module in
//! any JSON.

use super::dberr;
use crate::types::Text;
use crate::Db;
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_core::portfolio::{self as rules, ClientInput, EntryInput, MemberInput, ProjectInput, ServiceInput, Settings, SkillInput, TestimonialInput};
use vuuut_core::{dates, ids, Result};

fn touch() {
    crate::content::touch();
}

pub fn link(s: &Settings, internal: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(internal))
}

fn opt(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

async fn unique_slug(pool: &Db, table: &str, wanted: &str, fallback: &str, reserved: &[&str], except: &str) -> String {
    let base = match wanted {
        "" => vuuut_core::directory::slugify(fallback),
        w if reserved.contains(&w) => format!("{w}-{}", vuuut_core::directory::slugify(fallback)),
        w => w.to_string(),
    };
    let base = if base.is_empty() { "untitled".to_string() } else { base };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE slug = ? AND id <> ?")).bind(&candidate).bind(except).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

pub async fn secret(pool: &Db) -> String {
    if let Ok(Some(s)) = sqlx::query_scalar::<_, String>("SELECT secret FROM portfolio_config WHERE id = 1").fetch_optional(pool).await {
        return s;
    }
    let s = format!("{}{}", ids::new_id(), ids::new_id());
    let _ = sqlx::query(crate::dialect::sql("INSERT OR IGNORE INTO portfolio_config (id, secret) VALUES (1, ?)")).bind(&s).execute(pool).await;
    sqlx::query_scalar::<_, String>("SELECT secret FROM portfolio_config WHERE id = 1").fetch_one(pool).await.unwrap_or(s)
}

// ---------------------------------------------------------------------------
// categories (disciplines)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub tint: String,
    pub sort_order: i64,
}

pub async fn categories(pool: &Db) -> Vec<Category> {
    sqlx::query_as::<_, Category>("SELECT id, name, slug, description, tint, sort_order FROM portfolio_categories ORDER BY sort_order, name").fetch_all(pool).await.unwrap_or_default()
}

pub async fn category_by_slug(pool: &Db, slug: &str) -> Option<Category> {
    sqlx::query_as::<_, Category>("SELECT id, name, slug, description, tint, sort_order FROM portfolio_categories WHERE slug = ?").bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_category(pool: &Db, id: Option<&str>, name: &str, slug: &str, description: &str, tint: &str, sort: i64) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_categories (id, name, slug, created_at, updated_at) VALUES (?,?,?,?,?)").bind(&id).bind(name).bind(ids::new_id()).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "portfolio_categories", slug, name, &[], &id).await;
    sqlx::query("UPDATE portfolio_categories SET name = ?, slug = ?, description = ?, tint = ?, sort_order = ?, updated_at = ? WHERE id = ?")
        .bind(name).bind(&slug).bind(description).bind(tint).bind(sort).bind(&now).bind(&id).execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn delete_category(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_project_categories WHERE category_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_categories WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `category id → published projects in it`.
pub async fn category_counts(pool: &Db) -> HashMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT pc.category_id, COUNT(*) FROM portfolio_project_categories pc JOIN portfolio_projects p ON p.id = pc.project_id \
         WHERE p.status IN ('published', 'protected') GROUP BY pc.category_id",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().collect()
}

// ---------------------------------------------------------------------------
// projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub subtitle: String,
    pub summary: String,
    #[sqlx(try_from = "Text")]
    pub body: String,
    #[sqlx(try_from = "Text")]
    pub body_lexical: String,
    #[sqlx(try_from = "Text")]
    pub body_html: String,
    pub cover: String,
    #[sqlx(try_from = "Text")]
    pub gallery: String,
    pub video_url: String,
    pub client_id: Option<String>,
    pub client_name: String,
    pub year: i64,
    pub duration: String,
    pub role: String,
    pub industry: String,
    pub location: String,
    pub tools: String,
    pub live_url: String,
    pub repo_url: String,
    #[sqlx(try_from = "Text")]
    pub metrics: String,
    #[sqlx(try_from = "Text")]
    pub credits: String,
    pub testimonial_id: Option<String>,
    pub accent: String,
    pub layout: String,
    pub featured: i64,
    pub status: String,
    pub password_hash: String,
    pub sort_order: i64,
    pub views: i64,
    pub published_at: Option<String>,
    pub seo_title: String,
    pub seo_description: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Project {
    pub fn gallery(&self) -> Vec<String> {
        self.gallery.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
    pub fn images(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.cover.is_empty() {
            v.push(self.cover.clone());
        }
        v.extend(self.gallery());
        v
    }
    pub fn tools(&self) -> Vec<String> {
        rules::list(&self.tools, 30, 40)
    }
    pub fn metrics(&self) -> Vec<rules::Metric> {
        rules::metrics_from_json(&self.metrics)
    }
    pub fn credits(&self) -> Vec<rules::Credit> {
        rules::credits_from_json(&self.credits)
    }
    pub fn status(&self) -> rules::ProjectStatus {
        rules::ProjectStatus::from_key(&self.status)
    }
    pub fn protected(&self) -> bool {
        self.status == "protected" && !self.password_hash.is_empty()
    }
    pub fn body_markup(&self) -> String {
        if self.body_html.trim().is_empty() { vuuut_core::directory::paragraphs(&self.body) } else { self.body_html.clone() }
    }
}

const PROJECT_COLS: &str = "id, slug, title, subtitle, summary, COALESCE(body, '') AS body, COALESCE(body_lexical, '') AS body_lexical, \
COALESCE(body_html, '') AS body_html, cover, COALESCE(gallery, '') AS gallery, video_url, client_id, client_name, year, duration, role, industry, location, \
tools, live_url, repo_url, COALESCE(metrics, '') AS metrics, COALESCE(credits, '') AS credits, testimonial_id, accent, layout, featured, status, password_hash, \
sort_order, views, published_at, seo_title, seo_description, created_at, updated_at";

#[derive(Debug, Clone, Default)]
pub struct ProjectQuery<'a> {
    /// Only what the public grid lists (published and protected).
    pub listed: bool,
    pub status: Option<&'a str>,
    pub category_id: Option<&'a str>,
    pub service_id: Option<&'a str>,
    pub client_id: Option<&'a str>,
    pub featured: bool,
    pub search: Option<&'a str>,
    /// `manual` (default: featured, then sort order, then newest) | `newest` | `popular` | `title`
    pub order: &'a str,
    pub limit: i64,
    pub offset: i64,
}

pub async fn projects(pool: &Db, q: &ProjectQuery<'_>) -> (Vec<Project>, i64) {
    let mut sql = String::from(" FROM portfolio_projects WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    if q.listed {
        sql.push_str(" AND status IN ('published', 'protected')");
    }
    if let Some(s) = q.status {
        sql.push_str(" AND status = ?");
        binds.push(s.into());
    }
    if let Some(c) = q.category_id {
        sql.push_str(" AND id IN (SELECT project_id FROM portfolio_project_categories WHERE category_id = ?)");
        binds.push(c.into());
    }
    if let Some(c) = q.service_id {
        sql.push_str(" AND id IN (SELECT project_id FROM portfolio_project_services WHERE service_id = ?)");
        binds.push(c.into());
    }
    if let Some(c) = q.client_id {
        sql.push_str(" AND client_id = ?");
        binds.push(c.into());
    }
    if q.featured {
        sql.push_str(" AND featured = 1");
    }
    let words: Vec<String> = q.search.unwrap_or("").split_whitespace().take(6).map(|w| format!("%{}%", w.to_lowercase())).collect();
    for _ in &words {
        sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ? OR LOWER(client_name) LIKE ? OR LOWER(tools) LIKE ?)");
    }
    let order = match q.order {
        "newest" => "COALESCE(published_at, created_at) DESC",
        "popular" => "views DESC",
        "title" => "title ASC",
        "year" => "year DESC, sort_order ASC",
        _ => "featured DESC, sort_order ASC, COALESCE(published_at, created_at) DESC",
    };
    let count_sql = format!("SELECT COUNT(*){sql}");
    let mut c = sqlx::query_scalar::<_, i64>(&count_sql);
    let list_sql = format!("SELECT {PROJECT_COLS}{sql} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut l = sqlx::query_as::<_, Project>(&list_sql);
    for b in &binds {
        c = c.bind(b.clone());
        l = l.bind(b.clone());
    }
    for w in &words {
        for _ in 0..4 {
            c = c.bind(w.clone());
            l = l.bind(w.clone());
        }
    }
    let total = c.fetch_one(pool).await.unwrap_or(0);
    let rows = l.bind(q.limit.max(1)).bind(q.offset.max(0)).fetch_all(pool).await.unwrap_or_else(|e| {
        tracing::error!("portfolio: listing projects failed: {e}");
        Vec::new()
    });
    (rows, total)
}

pub async fn find_project(pool: &Db, id: &str) -> Option<Project> {
    sqlx::query_as::<_, Project>(&format!("SELECT {PROJECT_COLS} FROM portfolio_projects WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn project_by_slug(pool: &Db, slug: &str) -> Option<Project> {
    sqlx::query_as::<_, Project>(&format!("SELECT {PROJECT_COLS} FROM portfolio_projects WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn project_category_ids(pool: &Db, id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT category_id FROM portfolio_project_categories WHERE project_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default()
}

pub async fn project_service_ids(pool: &Db, id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT service_id FROM portfolio_project_services WHERE project_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default()
}

/// Every project's categories at once: `project id → [category id]`.
pub async fn all_project_categories(pool: &Db) -> HashMap<String, Vec<String>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT project_id, category_id FROM portfolio_project_categories").fetch_all(pool).await.unwrap_or_default();
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (p, c) in rows {
        out.entry(p).or_default().push(c);
    }
    out
}

pub async fn save_project(pool: &Db, id: Option<&str>, i: &ProjectInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let existing = match id {
        Some(id) => find_project(pool, id).await,
        None => None,
    };
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_projects (id, slug, title, created_at, updated_at) VALUES (?,?,?,?,?)").bind(&id).bind(ids::new_id()).bind(&i.title).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "portfolio_projects", &i.slug, &i.title, rules::RESERVED_SLUGS, &id).await;
    let published_at = existing.as_ref().and_then(|p| p.published_at.clone()).or_else(|| (i.status != "draft").then(|| now.clone()));
    let hash = if !i.password.is_empty() {
        rules::hash_password(&i.password, &ids::new_id())
    } else {
        existing.as_ref().map(|p| p.password_hash.clone()).unwrap_or_default()
    };
    let metrics = if i.metrics.is_empty() { String::new() } else { serde_json::to_string(&i.metrics).unwrap_or_default() };
    let credits = if i.credits.is_empty() { String::new() } else { serde_json::to_string(&i.credits).unwrap_or_default() };
    sqlx::query(
        "UPDATE portfolio_projects SET slug = ?, title = ?, subtitle = ?, summary = ?, body = ?, body_lexical = ?, body_html = ?, cover = ?, gallery = ?, \
         video_url = ?, client_id = ?, client_name = ?, year = ?, duration = ?, role = ?, industry = ?, location = ?, tools = ?, live_url = ?, repo_url = ?, \
         metrics = ?, credits = ?, testimonial_id = ?, accent = ?, layout = ?, featured = ?, status = ?, password_hash = ?, sort_order = ?, published_at = ?, \
         seo_title = ?, seo_description = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.title).bind(&i.subtitle).bind(&i.summary).bind(&i.body).bind(&i.body_lexical).bind(&i.body_html).bind(&i.cover)
    .bind(i.gallery.join("\n")).bind(&i.video_url).bind(i.client_id.clone()).bind(&i.client_name).bind(i.year).bind(&i.duration).bind(&i.role)
    .bind(&i.industry).bind(&i.location).bind(i.tools.join(", ")).bind(&i.live_url).bind(&i.repo_url).bind(metrics).bind(credits)
    .bind(i.testimonial_id.clone()).bind(&i.accent).bind(&i.layout).bind(i64::from(i.featured)).bind(&i.status).bind(hash).bind(i.sort_order)
    .bind(published_at).bind(&i.seo_title).bind(&i.seo_description).bind(&now).bind(&id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_project_categories WHERE project_id = ?").bind(&id).execute(pool).await.map_err(dberr)?;
    for c in &i.category_ids {
        sqlx::query("INSERT INTO portfolio_project_categories (project_id, category_id) VALUES (?, ?)").bind(&id).bind(c).execute(pool).await.map_err(dberr)?;
    }
    sqlx::query("DELETE FROM portfolio_project_services WHERE project_id = ?").bind(&id).execute(pool).await.map_err(dberr)?;
    for c in &i.service_ids {
        sqlx::query("INSERT INTO portfolio_project_services (project_id, service_id) VALUES (?, ?)").bind(&id).bind(c).execute(pool).await.map_err(dberr)?;
    }
    Ok(id)
}

pub async fn clear_password(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE portfolio_projects SET password_hash = '', status = CASE WHEN status = 'protected' THEN 'published' ELSE status END WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_project(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_project_categories WHERE project_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_project_services WHERE project_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("UPDATE portfolio_testimonials SET project_id = NULL WHERE project_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_projects WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn duplicate_project(pool: &Db, id: &str) -> Result<Option<String>> {
    let Some(p) = find_project(pool, id).await else { return Ok(None) };
    let input = ProjectInput {
        title: format!("{} (copy)", p.title),
        slug: String::new(),
        subtitle: p.subtitle.clone(), summary: p.summary.clone(), body: p.body.clone(), body_lexical: p.body_lexical.clone(), body_html: p.body_html.clone(),
        cover: p.cover.clone(), gallery: p.gallery(), video_url: p.video_url.clone(), client_id: p.client_id.clone(), client_name: p.client_name.clone(),
        year: p.year, duration: p.duration.clone(), role: p.role.clone(), industry: p.industry.clone(), location: p.location.clone(), tools: p.tools(),
        live_url: p.live_url.clone(), repo_url: p.repo_url.clone(), metrics: p.metrics(), credits: p.credits(), testimonial_id: None,
        category_ids: project_category_ids(pool, id).await, service_ids: project_service_ids(pool, id).await, accent: p.accent.clone(),
        layout: p.layout.clone(), featured: false, status: "draft".into(), password: String::new(), sort_order: p.sort_order,
        seo_title: String::new(), seo_description: String::new(),
    };
    Ok(Some(save_project(pool, None, &input).await?))
}

pub async fn count_view(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE portfolio_projects SET views = views + 1 WHERE id = ?").bind(id).execute(pool).await;
}

/// The listed projects either side of this one in the grid's order.
pub async fn neighbours(pool: &Db, id: &str) -> (Option<Project>, Option<Project>) {
    let (all, _) = projects(pool, &ProjectQuery { listed: true, limit: 1000, ..Default::default() }).await;
    let Some(i) = all.iter().position(|p| p.id == id) else { return (None, None) };
    let prev = if i > 0 { all.get(i - 1).cloned() } else { all.last().filter(|p| p.id != id).cloned() };
    let next = all.get(i + 1).cloned().or_else(|| all.first().filter(|p| p.id != id).cloned());
    (prev, next)
}

pub async fn status_counts(pool: &Db) -> HashMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT status, COUNT(*) FROM portfolio_projects GROUP BY status").fetch_all(pool).await.unwrap_or_default();
    rows.into_iter().collect()
}

// ---------------------------------------------------------------------------
// services
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Service {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub summary: String,
    #[sqlx(try_from = "Text")]
    pub description: String,
    pub image: String,
    pub icon: String,
    pub price_from: i64,
    pub price_note: String,
    #[sqlx(try_from = "Text")]
    pub deliverables: String,
    pub status: String,
    pub featured: i64,
    pub sort_order: i64,
}

impl Service {
    pub fn deliverables(&self) -> Vec<String> {
        self.deliverables.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
}

const SERVICE_COLS: &str = "id, slug, name, summary, COALESCE(description, '') AS description, image, icon, price_from, price_note, COALESCE(deliverables, '') AS deliverables, status, featured, sort_order";

pub async fn services(pool: &Db, shown_only: bool) -> Vec<Service> {
    let f = if shown_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM portfolio_services{f} ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_service(pool: &Db, id: &str) -> Option<Service> {
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM portfolio_services WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn service_by_slug(pool: &Db, slug: &str) -> Option<Service> {
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM portfolio_services WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_service(pool: &Db, id: Option<&str>, i: &ServiceInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_services (id, slug, name, created_at, updated_at) VALUES (?,?,?,?,?)").bind(&id).bind(ids::new_id()).bind(&i.name).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "portfolio_services", &i.slug, &i.name, &[], &id).await;
    sqlx::query("UPDATE portfolio_services SET slug = ?, name = ?, summary = ?, description = ?, image = ?, icon = ?, price_from = ?, price_note = ?, deliverables = ?, status = ?, featured = ?, sort_order = ?, updated_at = ? WHERE id = ?")
        .bind(&slug).bind(&i.name).bind(&i.summary).bind(&i.description).bind(&i.image).bind(&i.icon).bind(i.price_from).bind(&i.price_note)
        .bind(i.deliverables.join("\n")).bind(&i.status).bind(i64::from(i.featured)).bind(i.sort_order).bind(&now).bind(&id)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn delete_service(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_project_services WHERE service_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_services WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// clients, testimonials, team
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub logo: String,
    pub url: String,
    pub industry: String,
    pub featured: i64,
    pub sort_order: i64,
}

pub async fn clients(pool: &Db) -> Vec<Client> {
    sqlx::query_as::<_, Client>("SELECT id, name, logo, url, industry, featured, sort_order FROM portfolio_clients ORDER BY sort_order, name").fetch_all(pool).await.unwrap_or_default()
}

pub async fn save_client(pool: &Db, id: Option<&str>, i: &ClientInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE portfolio_clients SET name = ?, logo = ?, url = ?, industry = ?, featured = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(&i.name).bind(&i.logo).bind(&i.url).bind(&i.industry).bind(i64::from(i.featured)).bind(i.sort_order).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_clients (id, name, logo, url, industry, featured, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&i.name).bind(&i.logo).bind(&i.url).bind(&i.industry).bind(i64::from(i.featured)).bind(i.sort_order).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_client(pool: &Db, id: &str) -> Result<()> {
    touch();
    // The project keeps the name it was credited to.
    sqlx::query("UPDATE portfolio_projects SET client_name = (SELECT name FROM portfolio_clients WHERE id = ?), client_id = NULL WHERE client_id = ?").bind(id).bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_clients WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Testimonial {
    pub id: String,
    #[sqlx(try_from = "Text")]
    pub quote: String,
    pub author: String,
    pub role: String,
    pub company: String,
    pub photo: String,
    pub rating: i64,
    pub project_id: Option<String>,
    pub featured: i64,
    pub sort_order: i64,
}

const TESTIMONIAL_COLS: &str = "id, COALESCE(quote, '') AS quote, author, role, company, photo, rating, project_id, featured, sort_order";

pub async fn testimonials(pool: &Db) -> Vec<Testimonial> {
    sqlx::query_as::<_, Testimonial>(&format!("SELECT {TESTIMONIAL_COLS} FROM portfolio_testimonials ORDER BY featured DESC, sort_order, created_at DESC")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_testimonial(pool: &Db, id: &str) -> Option<Testimonial> {
    sqlx::query_as::<_, Testimonial>(&format!("SELECT {TESTIMONIAL_COLS} FROM portfolio_testimonials WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_testimonial(pool: &Db, id: Option<&str>, i: &TestimonialInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE portfolio_testimonials SET quote = ?, author = ?, role = ?, company = ?, photo = ?, rating = ?, project_id = ?, featured = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(&i.quote).bind(&i.author).bind(&i.role).bind(&i.company).bind(&i.photo).bind(i.rating).bind(i.project_id.clone()).bind(i64::from(i.featured)).bind(i.sort_order).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_testimonials (id, quote, author, role, company, photo, rating, project_id, featured, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&i.quote).bind(&i.author).bind(&i.role).bind(&i.company).bind(&i.photo).bind(i.rating).bind(i.project_id.clone()).bind(i64::from(i.featured)).bind(i.sort_order).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_testimonial(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE portfolio_projects SET testimonial_id = NULL WHERE testimonial_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM portfolio_testimonials WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Member {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub role: String,
    #[sqlx(try_from = "Text")]
    pub bio: String,
    pub photo: String,
    pub email: String,
    #[sqlx(try_from = "Text")]
    pub links: String,
    pub status: String,
    pub sort_order: i64,
}

impl Member {
    pub fn initials(&self) -> String {
        self.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
    }
    pub fn links(&self) -> Vec<String> {
        self.links.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
}

const MEMBER_COLS: &str = "id, slug, name, role, COALESCE(bio, '') AS bio, photo, email, COALESCE(links, '') AS links, status, sort_order";

pub async fn team(pool: &Db, active_only: bool) -> Vec<Member> {
    let f = if active_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Member>(&format!("SELECT {MEMBER_COLS} FROM portfolio_team{f} ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_member(pool: &Db, id: &str) -> Option<Member> {
    sqlx::query_as::<_, Member>(&format!("SELECT {MEMBER_COLS} FROM portfolio_team WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn member_by_slug(pool: &Db, slug: &str) -> Option<Member> {
    sqlx::query_as::<_, Member>(&format!("SELECT {MEMBER_COLS} FROM portfolio_team WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_member(pool: &Db, id: Option<&str>, i: &MemberInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_team (id, slug, name, created_at, updated_at) VALUES (?,?,?,?,?)").bind(&id).bind(ids::new_id()).bind(&i.name).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "portfolio_team", &i.slug, &i.name, &[], &id).await;
    sqlx::query("UPDATE portfolio_team SET slug = ?, name = ?, role = ?, bio = ?, photo = ?, email = ?, links = ?, status = ?, sort_order = ?, updated_at = ? WHERE id = ?")
        .bind(&slug).bind(&i.name).bind(&i.role).bind(&i.bio).bind(&i.photo).bind(&i.email).bind(i.links.join("\n")).bind(&i.status).bind(i.sort_order).bind(&now).bind(&id)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn delete_member(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_team WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// résumé
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub organisation: String,
    pub location: String,
    pub url: String,
    pub date_from: String,
    pub date_to: String,
    pub ongoing: i64,
    #[sqlx(try_from = "Text")]
    pub description: String,
    pub sort_order: i64,
}

impl Entry {
    pub fn span(&self) -> String {
        rules::span_label(&self.date_from, &self.date_to, self.ongoing != 0)
    }
}

const ENTRY_COLS: &str = "id, kind, title, organisation, location, url, date_from, date_to, ongoing, COALESCE(description, '') AS description, sort_order";

/// Newest first within each kind: ongoing, then by end, then by start.
pub async fn entries(pool: &Db) -> Vec<Entry> {
    sqlx::query_as::<_, Entry>(&format!("SELECT {ENTRY_COLS} FROM portfolio_entries ORDER BY kind, sort_order, ongoing DESC, date_to DESC, date_from DESC")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_entry(pool: &Db, id: &str) -> Option<Entry> {
    sqlx::query_as::<_, Entry>(&format!("SELECT {ENTRY_COLS} FROM portfolio_entries WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_entry(pool: &Db, id: Option<&str>, i: &EntryInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE portfolio_entries SET kind = ?, title = ?, organisation = ?, location = ?, url = ?, date_from = ?, date_to = ?, ongoing = ?, description = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(&i.kind).bind(&i.title).bind(&i.organisation).bind(&i.location).bind(&i.url).bind(&i.start).bind(&i.end).bind(i64::from(i.current)).bind(&i.description).bind(i.sort_order).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_entries (id, kind, title, organisation, location, url, date_from, date_to, ongoing, description, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&i.kind).bind(&i.title).bind(&i.organisation).bind(&i.location).bind(&i.url).bind(&i.start).bind(&i.end).bind(i64::from(i.current)).bind(&i.description).bind(i.sort_order).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_entry(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_entries WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub grp: String,
    pub level: i64,
    pub sort_order: i64,
}

pub async fn skills(pool: &Db) -> Vec<Skill> {
    sqlx::query_as::<_, Skill>("SELECT id, name, grp, level, sort_order FROM portfolio_skills ORDER BY grp, sort_order, name").fetch_all(pool).await.unwrap_or_default()
}

pub async fn save_skill(pool: &Db, id: Option<&str>, i: &SkillInput) -> Result<String> {
    touch();
    match id {
        Some(id) => {
            sqlx::query("UPDATE portfolio_skills SET name = ?, grp = ?, level = ?, sort_order = ? WHERE id = ?").bind(&i.name).bind(&i.group).bind(i.level).bind(i.sort_order).bind(id).execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO portfolio_skills (id, name, grp, level, sort_order, created_at) VALUES (?,?,?,?,?,?)").bind(&id).bind(&i.name).bind(&i.group).bind(i.level).bind(i.sort_order).bind(dates::now_db()).execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_skill(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM portfolio_skills WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// enquiries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Enquiry {
    pub id: String,
    pub name: String,
    pub email: String,
    pub company: String,
    pub phone: String,
    pub budget: String,
    pub timeline: String,
    pub services: String,
    pub project: String,
    #[sqlx(try_from = "Text")]
    pub message: String,
    pub status: String,
    #[sqlx(try_from = "Text")]
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
}

const ENQUIRY_COLS: &str = "id, name, email, company, phone, budget, timeline, services, project, COALESCE(message, '') AS message, status, COALESCE(note, '') AS note, created_at, updated_at";

pub async fn insert_enquiry(pool: &Db, i: &rules::EnquiryInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO portfolio_enquiries (id, name, email, company, phone, budget, timeline, services, project, message, status, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,'new',?,?)")
        .bind(&id).bind(&i.name).bind(&i.email).bind(&i.company).bind(&i.phone).bind(&i.budget).bind(&i.timeline).bind(i.services.join(", ")).bind(&i.project).bind(&i.message).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn enquiries(pool: &Db, status: Option<&str>, limit: i64) -> Vec<Enquiry> {
    match status {
        Some(s) => sqlx::query_as::<_, Enquiry>(&format!("SELECT {ENQUIRY_COLS} FROM portfolio_enquiries WHERE status = ? ORDER BY created_at DESC LIMIT ?")).bind(s).bind(limit).fetch_all(pool).await,
        None => sqlx::query_as::<_, Enquiry>(&format!("SELECT {ENQUIRY_COLS} FROM portfolio_enquiries WHERE status <> 'spam' ORDER BY created_at DESC LIMIT ?")).bind(limit).fetch_all(pool).await,
    }
    .unwrap_or_default()
}

pub async fn find_enquiry(pool: &Db, id: &str) -> Option<Enquiry> {
    sqlx::query_as::<_, Enquiry>(&format!("SELECT {ENQUIRY_COLS} FROM portfolio_enquiries WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn update_enquiry(pool: &Db, id: &str, status: &str, note: &str) -> Result<()> {
    sqlx::query("UPDATE portfolio_enquiries SET status = ?, note = ?, updated_at = ? WHERE id = ?").bind(status).bind(note).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_enquiry(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM portfolio_enquiries WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn enquiry_counts(pool: &Db) -> HashMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT status, COUNT(*) FROM portfolio_enquiries GROUP BY status").fetch_all(pool).await.unwrap_or_default();
    rows.into_iter().collect()
}

pub async fn enquiries_from_since(pool: &Db, email: &str, since: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_enquiries WHERE email = ? AND created_at >= ?").bind(email).bind(since).fetch_one(pool).await.unwrap_or(0)
}

/// `(day, enquiries)` since `since`.
pub async fn enquiries_per_day(pool: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as("SELECT SUBSTR(created_at, 1, 10) AS d, COUNT(*) FROM portfolio_enquiries WHERE created_at >= ? AND status <> 'spam' GROUP BY SUBSTR(created_at, 1, 10) ORDER BY d")
        .bind(since).fetch_all(pool).await.unwrap_or_default()
}

// ---------------------------------------------------------------------------
// what themes are handed
// ---------------------------------------------------------------------------

/// Everything a project's JSON needs to name its links.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    pub categories: Vec<Category>,
    pub services: Vec<Service>,
    pub clients: Vec<Client>,
    pub testimonials: Vec<Testimonial>,
    pub project_categories: HashMap<String, Vec<String>>,
}

impl Lookup {
    pub async fn load(pool: &Db) -> Lookup {
        Lookup {
            categories: categories(pool).await,
            services: services(pool, false).await,
            clients: clients(pool).await,
            testimonials: testimonials(pool).await,
            project_categories: all_project_categories(pool).await,
        }
    }
    pub fn client_name(&self, p: &Project) -> String {
        p.client_id.as_deref().and_then(|id| self.clients.iter().find(|c| c.id == id)).map(|c| c.name.clone()).unwrap_or_else(|| p.client_name.clone())
    }
}

pub fn project_url(s: &Settings, p: &Project) -> String {
    link(s, &format!("/portfolio/{}/", p.slug))
}

pub fn category_url(s: &Settings, c: &Category) -> String {
    link(s, &format!("/portfolio/category/{}/", c.slug))
}

pub fn service_url(s: &Settings, v: &Service) -> String {
    link(s, &format!("/portfolio/services/{}/", v.slug))
}

pub fn price_label(s: &Settings, v: &Service) -> String {
    if v.price_from <= 0 {
        return String::new();
    }
    let p = vuuut_core::directory::format_price(v.price_from, &s.currency, vuuut_core::directory::PriceType::Fixed).unwrap_or_default();
    format!("From {p}")
}

fn byline(role: &str, company: &str) -> String {
    [role, company].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(", ")
}

pub fn testimonial_json(t: &Testimonial) -> Value {
    json!({ "id": t.id, "quote": t.quote, "author": t.author, "role": t.role, "company": t.company, "photo": opt(&t.photo),
        "initials": t.author.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase(),
        "rating": t.rating, "stars": "★".repeat(t.rating.clamp(0, 5) as usize), "featured": t.featured != 0,
        "byline": byline(&t.role, &t.company) })
}

pub fn category_json(s: &Settings, c: &Category, count: i64) -> Value {
    json!({ "id": c.id, "name": c.name, "slug": c.slug, "description": c.description, "tint": c.tint, "count": count, "url": category_url(s, c) })
}

pub fn client_json(c: &Client) -> Value {
    json!({ "id": c.id, "name": c.name, "logo": opt(&c.logo), "url": opt(&c.url), "industry": c.industry, "featured": c.featured != 0 })
}

pub fn service_json(s: &Settings, v: &Service) -> Value {
    json!({
        "id": v.id, "slug": v.slug, "name": v.name, "title": v.name, "summary": v.summary, "description": vuuut_core::directory::paragraphs(&v.description),
        "image": opt(&v.image), "feature_image": opt(&v.image), "icon": v.icon, "url": service_url(s, v),
        "price_from": v.price_from, "price_label": price_label(s, v), "price_note": v.price_note,
        "deliverables": v.deliverables(), "featured": v.featured != 0,
    })
}

pub fn member_json(s: &Settings, m: &Member) -> Value {
    json!({ "id": m.id, "slug": m.slug, "name": m.name, "role": m.role, "bio": m.bio, "photo": opt(&m.photo), "initials": m.initials(),
        "links": m.links(), "url": link(s, &format!("/portfolio/team/{}/", m.slug)) })
}

pub fn entry_json(e: &Entry) -> Value {
    json!({ "id": e.id, "kind": e.kind, "kind_label": rules::EntryKind::from_key(&e.kind).label(), "title": e.title, "organisation": e.organisation,
        "location": e.location, "url": opt(&e.url), "start": e.date_from, "end": e.date_to, "current": e.ongoing != 0, "span": e.span(),
        "description": e.description, "description_html": vuuut_core::directory::paragraphs(&e.description) })
}

pub fn skill_json(k: &Skill) -> Value {
    json!({ "name": k.name, "group": k.grp, "level": k.level })
}

/// One project as a theme sees it — `{{#get "projects"}}` and `project` in
/// the `portfolio/*.hbs` templates. `open` is false for a protected project
/// the visitor has not unlocked: its body, gallery, video and links stay
/// out of the JSON entirely, not just out of the template.
pub fn project_json(s: &Settings, p: &Project, x: &Lookup, open: bool) -> Value {
    let cats: Vec<&Category> = x.project_categories.get(&p.id).map(|ids| x.categories.iter().filter(|c| ids.contains(&c.id)).collect()).unwrap_or_default();
    let client = p.client_id.as_deref().and_then(|id| x.clients.iter().find(|c| c.id == id));
    let testimonial = p.testimonial_id.as_deref().and_then(|id| x.testimonials.iter().find(|t| t.id == id));
    let date = p.published_at.as_deref().and_then(dates::from_db_string);
    json!({
        "id": p.id, "slug": p.slug, "title": p.title, "subtitle": p.subtitle, "summary": p.summary, "excerpt": p.summary,
        "url": project_url(s, p),
        "cover": opt(&p.cover), "feature_image": opt(&p.cover),
        "client": x.client_name(p), "client_logo": client.map(|c| opt(&c.logo)).unwrap_or(Value::Null), "client_url": client.map(|c| opt(&c.url)).unwrap_or(Value::Null),
        "year": if p.year > 0 { json!(p.year) } else { Value::Null }, "duration": p.duration, "role": p.role, "industry": p.industry, "location": p.location,
        "tools": p.tools(), "accent": opt(&p.accent), "layout": p.layout, "featured": p.featured != 0,
        "protected": p.protected(), "locked": p.protected() && !open,
        "categories": cats.iter().map(|c| json!({ "name": c.name, "slug": c.slug, "url": category_url(s, c), "tint": c.tint })).collect::<Vec<_>>(),
        "category_slugs": cats.iter().map(|c| c.slug.clone()).collect::<Vec<_>>().join(" "),
        "primary_category": cats.first().map(|c| json!(c.name)).unwrap_or(Value::Null),
        "body": if open { json!(p.body_markup()) } else { Value::Null },
        "gallery": if open { json!(p.gallery()) } else { json!([]) },
        "images": if open { json!(p.images()) } else { json!(p.images().into_iter().take(1).collect::<Vec<_>>()) },
        "video_embed": if open { vuuut_core::lms::video_embed(&p.video_url).map(|(u, frame)| json!({ "url": u, "iframe": frame })).unwrap_or(Value::Null) } else { Value::Null },
        "live_url": if open { opt(&p.live_url) } else { Value::Null },
        "repo_url": if open { opt(&p.repo_url) } else { Value::Null },
        "metrics": if open { json!(p.metrics()) } else { json!([]) },
        "credits": if open { json!(p.credits()) } else { json!([]) },
        "testimonial": if open { testimonial.map(testimonial_json).unwrap_or(Value::Null) } else { Value::Null },
        "views": p.views,
        "published_at": date.map(|d| json!(d.to_rfc3339())).unwrap_or(Value::Null),
    })
}
