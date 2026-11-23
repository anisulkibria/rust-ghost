//! Directory & classified listings: categories and listings.
//!
//! Every value is bound — the public search included, whose words go into
//! `LIKE ?` placeholders, never into the SQL. VARCHAR-only on MySQL; flags
//! and counters are read as `i64`.

use super::dberr;
use crate::Db;
use vuuut_core::directory::ListingInput;
use vuuut_core::{dates, ids, Result};

// ---------------------------------------------------------------------------
// categories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub tint: String,
    pub sort_order: i64,
}

const CAT_COLS: &str = "id, name, slug, description, tint, sort_order";

pub async fn categories(pool: &Db) -> Result<Vec<Category>> {
    sqlx::query_as::<_, Category>(&format!("SELECT {CAT_COLS} FROM directory_categories ORDER BY sort_order, name"))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

pub async fn category_by_slug(pool: &Db, slug: &str) -> Option<Category> {
    sqlx::query_as::<_, Category>(&format!("SELECT {CAT_COLS} FROM directory_categories WHERE slug = ?"))
        .bind(slug)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub async fn save_category(pool: &Db, id: Option<&str>, name: &str, slug: &str, description: &str, tint: &str, sort: i64) -> Result<String> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE directory_categories SET name = ?, slug = ?, description = ?, tint = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(name).bind(slug).bind(description).bind(tint).bind(sort).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO directory_categories (id, name, slug, description, tint, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?)")
                .bind(&id).bind(name).bind(slug).bind(description).bind(tint).bind(sort).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

/// Deletes the category; its listings stay, uncategorised.
pub async fn delete_category(pool: &Db, id: &str) -> Result<()> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    sqlx::query("UPDATE directory_listings SET category_id = NULL WHERE category_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM directory_categories WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn category_slug_taken(pool: &Db, slug: &str, except: Option<&str>) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directory_categories WHERE slug = ? AND id <> ?")
        .bind(slug)
        .bind(except.unwrap_or(""))
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

/// `(category_id, visible listings)`.
pub async fn category_counts(pool: &Db, now: &str) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT category_id, COUNT(*) FROM directory_listings WHERE status = 'published' AND category_id IS NOT NULL \
         AND (expires_at IS NULL OR expires_at > ?) GROUP BY category_id",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// listings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Listing {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub category_id: Option<String>,
    pub summary: String,
    pub description: String,
    pub image: String,
    pub website: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub location: String,
    pub price_cents: i64,
    pub price_type: String,
    pub keywords: String,
    pub featured: i64,
    pub submitter_name: String,
    pub submitter_email: String,
    pub member_id: Option<String>,
    pub reject_reason: String,
    pub views: i64,
    pub contact_clicks: i64,
    pub expires_at: Option<String>,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Listing {
    pub fn is_featured(&self) -> bool {
        self.featured != 0
    }
}

const COLS: &str = "id, slug, title, kind, status, category_id, summary, description, image, website, email, phone, address, \
location, price_cents, price_type, keywords, featured, submitter_name, submitter_email, member_id, reject_reason, views, \
contact_clicks, expires_at, published_at, created_at, updated_at";

pub async fn find(pool: &Db, id: &str) -> Result<Option<Listing>> {
    sqlx::query_as::<_, Listing>(&format!("SELECT {COLS} FROM directory_listings WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// Resolve a selection in one query and enforce the public listing rules.
pub async fn visible_by_ids(pool: &Db, ids: &[String], now: &str) -> Vec<Listing> {
    if ids.is_empty() {
        return Vec::new();
    }
    let marks = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT {COLS} FROM directory_listings WHERE status = 'published' AND (expires_at IS NULL OR expires_at > ?) AND id IN ({marks})");
    let mut query = sqlx::query_as::<_, Listing>(&sql).bind(now);
    for id in ids {
        query = query.bind(id);
    }
    query.fetch_all(pool).await.unwrap_or_default()
}

pub async fn by_slug(pool: &Db, slug: &str) -> Result<Option<Listing>> {
    sqlx::query_as::<_, Listing>(&format!("SELECT {COLS} FROM directory_listings WHERE slug = ?"))
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// A slug no other listing has: `bike`, then `bike-2`, `bike-3`…
pub async fn unique_slug(pool: &Db, wanted: &str, except: Option<&str>) -> String {
    // The directory's own addresses are never a listing's.
    let base = match wanted {
        "" => "listing".to_string(),
        w if vuuut_core::directory::RESERVED_SLUGS.contains(&w) => format!("{wanted}-listing"),
        w => w.to_string(),
    };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_listings WHERE slug = ? AND id <> ?")
            .bind(&candidate)
            .bind(except.unwrap_or(""))
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

/// Create a listing in `status`. `expires_at` is set by the caller from the
/// site's classified lifetime. Returns only the id (see `popups::insert`).
pub async fn insert(pool: &Db, i: &ListingInput, status: &str, expires_at: Option<&str>, member_id: Option<&str>) -> Result<String> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    let id = ids::new_id();
    let now = dates::now_db();
    let slug = unique_slug(pool, &i.slug, None).await;
    let published = (status == "published").then(|| now.clone());
    sqlx::query(&format!(
        "INSERT INTO directory_listings ({COLS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'',0,0,?,?,?,?)"
    ))
    .bind(&id).bind(&slug).bind(&i.title).bind(&i.kind).bind(status).bind(i.category_id.clone())
    .bind(&i.summary).bind(&i.description).bind(&i.image).bind(&i.website).bind(&i.email).bind(&i.phone)
    .bind(&i.address).bind(&i.location).bind(i.price_cents).bind(&i.price_type).bind(&i.keywords)
    .bind(if i.featured { 1_i64 } else { 0 }).bind(&i.submitter_name).bind(&i.submitter_email).bind(member_id)
    .bind(expires_at).bind(published).bind(&now).bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

/// Staff edits: everything the form owns. Status, counters, the submitter
/// and dates are changed elsewhere.
pub async fn update(pool: &Db, id: &str, i: &ListingInput) -> Result<()> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    let slug = unique_slug(pool, &i.slug, Some(id)).await;
    sqlx::query(
        "UPDATE directory_listings SET slug = ?, title = ?, kind = ?, category_id = ?, summary = ?, description = ?, image = ?, \
         website = ?, email = ?, phone = ?, address = ?, location = ?, price_cents = ?, price_type = ?, keywords = ?, featured = ?, \
         updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.title).bind(&i.kind).bind(i.category_id.clone()).bind(&i.summary).bind(&i.description)
    .bind(&i.image).bind(&i.website).bind(&i.email).bind(&i.phone).bind(&i.address).bind(&i.location)
    .bind(i.price_cents).bind(&i.price_type).bind(&i.keywords).bind(if i.featured { 1_i64 } else { 0 })
    .bind(dates::now_db()).bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Publish, reject, unpublish. Publishing stamps `published_at` the first
/// time and (re)starts a classified's clock from now.
pub async fn set_status(pool: &Db, id: &str, status: &str, reason: &str, expires_at: Option<&str>) -> Result<()> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    let now = dates::now_db();
    sqlx::query(
        "UPDATE directory_listings SET status = ?, reject_reason = ?, expires_at = ?, \
         published_at = CASE WHEN ? = 'published' THEN COALESCE(published_at, ?) ELSE published_at END, updated_at = ? WHERE id = ?",
    )
    .bind(status).bind(reason).bind(expires_at).bind(status).bind(&now).bind(&now).bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

pub async fn delete(pool: &Db, id: &str) -> Result<()> {
    // Directory pages are served from the page cache; a change must empty it.
    crate::content::touch();
    sqlx::query("DELETE FROM directory_listings WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// A search over listings. Words are AND-ed, each matching title, summary,
/// description, keywords or location.
#[derive(Debug, Clone, Default)]
pub struct Browse<'a> {
    pub terms: &'a [String],
    pub category_id: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    /// Only what the public may see at this moment.
    pub visible_at: Option<&'a str>,
    pub location: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

fn where_clause(b: &Browse) -> (String, Vec<String>) {
    let mut sql = String::from(" WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    for t in b.terms {
        sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ? OR LOWER(description) LIKE ? OR LOWER(keywords) LIKE ? OR LOWER(location) LIKE ?)");
        for _ in 0..5 {
            binds.push(format!("%{}%", t.replace('%', "").replace('_', "")));
        }
    }
    if let Some(c) = b.category_id {
        sql.push_str(" AND category_id = ?");
        binds.push(c.to_string());
    }
    if let Some(k) = b.kind {
        sql.push_str(" AND kind = ?");
        binds.push(k.to_string());
    }
    if let Some(s) = b.status {
        sql.push_str(" AND status = ?");
        binds.push(s.to_string());
    }
    if let Some(now) = b.visible_at {
        sql.push_str(" AND status = 'published' AND (expires_at IS NULL OR expires_at > ?)");
        binds.push(now.to_string());
    }
    if let Some(l) = b.location {
        sql.push_str(" AND LOWER(location) = ?");
        binds.push(l.to_lowercase());
    }
    (sql, binds)
}

/// One page of matches, featured first then newest, and the total.
pub async fn browse(pool: &Db, b: &Browse<'_>) -> Result<(Vec<Listing>, i64)> {
    browse_with_limit(pool, b, b.limit.clamp(1, 500)).await
}

/// The public directory uses the page size set by its administrator.
pub async fn browse_public(pool: &Db, b: &Browse<'_>) -> Result<(Vec<Listing>, i64)> {
    browse_with_limit(pool, b, b.limit.max(1)).await
}

async fn browse_with_limit(pool: &Db, b: &Browse<'_>, limit: i64) -> Result<(Vec<Listing>, i64)> {
    let (clause, binds) = where_clause(b);
    let count_sql = format!("SELECT COUNT(*) FROM directory_listings{clause}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql);
    for v in &binds {
        count = count.bind(v.clone());
    }
    let total = count.fetch_one(pool).await.map_err(dberr)?;
    let sql = format!(
        "SELECT {COLS} FROM directory_listings{clause} ORDER BY featured DESC, COALESCE(published_at, created_at) DESC, id DESC LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query_as::<_, Listing>(&sql);
    for v in &binds {
        q = q.bind(v.clone());
    }
    let rows = q.bind(limit).bind(b.offset.max(0)).fetch_all(pool).await.map_err(dberr)?;
    Ok((rows, total))
}

/// Locations in use on visible listings, for the public filter.
pub async fn locations(pool: &Db, now: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT location FROM directory_listings WHERE status = 'published' AND location <> '' \
         AND (expires_at IS NULL OR expires_at > ?) ORDER BY location LIMIT 100",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// `(status, count)`.
pub async fn counts(pool: &Db) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM directory_listings GROUP BY status")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Published classifieds whose time is up or nearly so: `(expired, expiring
/// within a week)`.
pub async fn expiry_counts(pool: &Db, now: &str, week: &str) -> (i64, i64) {
    let one = |sql: &'static str, a: String, b: Option<String>| async move {
        let mut q = sqlx::query_scalar::<_, i64>(sql).bind(a);
        if let Some(b) = b {
            q = q.bind(b);
        }
        q.fetch_one(pool).await.unwrap_or(0)
    };
    let expired = one("SELECT COUNT(*) FROM directory_listings WHERE status = 'published' AND expires_at IS NOT NULL AND expires_at <= ?", now.to_string(), None).await;
    let soon = one("SELECT COUNT(*) FROM directory_listings WHERE status = 'published' AND expires_at > ? AND expires_at <= ?", now.to_string(), Some(week.to_string())).await;
    (expired, soon)
}

/// Approximate counters; a lost increment is a rounding error.
pub async fn add_view(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE directory_listings SET views = views + 1 WHERE id = ?").bind(id).execute(pool).await;
}

pub async fn add_contact_click(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE directory_listings SET contact_clicks = contact_clicks + 1 WHERE id = ?").bind(id).execute(pool).await;
}

/// How many listings this address has sent since a moment — the submission
/// form's per-person allowance.
pub async fn submitted_since(pool: &Db, email: &str, since: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directory_listings WHERE submitter_email = ? AND created_at >= ?")
        .bind(email)
        .bind(since)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}
