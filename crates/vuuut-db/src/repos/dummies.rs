//! Tools → Dummy content: the record of what was generated, and removal.
//!
//! Generated content is ordinary content — it is inserted through the same
//! repos as anything a person writes. What makes it a dummy is a row here:
//! `dummy_items` names each generated thing and the fingerprint it had when
//! it was made (a content hash or a menu link's label).
//!
//! Removal follows one rule: **delete only what is still exactly as
//! generated**. An item is kept, and stops being tracked, when
//!
//! * it has been edited since (its fingerprint no longer matches),
//! * the site's own content now depends on it — a tag or author used by a
//!   post that is not a dummy, a product somebody ordered, a member with a
//!   billing account,
//!
//! because at that point it is somebody's work, not demonstration material.
//! Something already deleted by hand is simply forgotten.

use crate::repos;
use crate::{Db, SettingsCache};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use vuuut_core::dummies::Kind;
use vuuut_core::{dates, ids, GhostError, Result};

fn dberr(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}

/// The table a kind's rows live in. Navigation lives in `settings`.
fn table(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Tags => Some("tags"),
        Kind::Users => Some("users"),
        Kind::Posts | Kind::Pages => Some("posts"),
        Kind::Members => Some("members"),
        Kind::Products => Some("ecom_products"),
        Kind::ListingCategories => Some("directory_categories"),
        Kind::Listings => Some("directory_listings"),
        Kind::Courses => Some("lms_courses"),
        Kind::CourseCategories => Some("lms_categories"),
        Kind::Services => Some("booking_services"),
        Kind::ServiceCategories => Some("booking_categories"),
        Kind::BookingStaff => Some("booking_staff"),
        Kind::Navigation => None,
    }
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

pub async fn create_batch(pool: &Db, label: &str, summary: &str, created_by: &str) -> Result<String> {
    let id = ids::new_id();
    sqlx::query("INSERT INTO dummy_batches (id, label, summary, created_by, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(label)
        .bind(summary)
        .bind(created_by)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(id)
}

/// Record a generated row, fingerprinted as it is now. Call it after every
/// write the generator makes to the row, or the first removal will think it
/// was edited.
pub async fn record(pool: &Db, batch_id: &str, kind: Kind, item_id: &str) -> Result<()> {
    let fingerprint = current_fingerprint(pool, kind, item_id).await?
        .ok_or_else(|| GhostError::Internal("Generated item is missing".into()))?;
    record_with(pool, batch_id, kind, item_id, &fingerprint).await
}

/// Record with a fingerprint the caller already knows — a menu link's label.
pub async fn record_with(pool: &Db, batch_id: &str, kind: Kind, item_id: &str, fingerprint: &str) -> Result<()> {
    sqlx::query("INSERT INTO dummy_items (id, batch_id, kind, item_id, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(batch_id)
        .bind(kind.key())
        .bind(item_id)
        .bind(fingerprint)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Older batches stored the timestamp alone. Keep that format readable.
async fn legacy_fingerprint(pool: &Db, kind: Kind, id: &str) -> Result<Option<String>> {
    let Some(table) = table(kind) else { return Ok(None) };
    sqlx::query_scalar::<_, Option<String>>(&format!("SELECT updated_at FROM {table} WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|row| row.map(Option::unwrap_or_default))
        .map_err(dberr)
}

/// Timestamps have second precision. Compare persisted content as well, so
/// an edit made during the same second still takes ownership of the item.
async fn current_fingerprint(pool: &Db, kind: Kind, id: &str) -> Result<Option<String>> {
    let value = match kind {
        Kind::Tags => repos::tags::find_by_id(pool, id).await?.map(|tag| json!(tag)),
        Kind::Members => repos::members::find(pool, id).await?.map(|member| json!(member)),
        Kind::Users => {
            let Some(user) = repos::users::find_by_id(pool, id).await? else { return Ok(None) };
            Some(json!([user, repos::users::role_name(pool, id).await?]))
        }
        Kind::Posts | Kind::Pages => {
            let Some(post) = repos::posts::find_by_id(pool, id).await? else { return Ok(None) };
            let authors = sqlx::query_scalar::<_, String>("SELECT author_id FROM posts_authors WHERE post_id = ? ORDER BY sort_order, id")
                .bind(id).fetch_all(pool).await.map_err(dberr)?;
            let tags = sqlx::query_scalar::<_, String>("SELECT tag_id FROM posts_tags WHERE post_id = ? ORDER BY sort_order, id")
                .bind(id).fetch_all(pool).await.map_err(dberr)?;
            Some(json!([post, authors, tags]))
        }
        Kind::Products => {
            let Some(product) = repos::ecom::find_product(pool, id).await? else { return Ok(None) };
            Some(json!([product, repos::ecom::read_variants(pool, id).await?]))
        }
        Kind::ListingCategories => {
            sqlx::query_as::<_, (String, String, String, String, String, i64, String)>("SELECT name, slug, description, tint, image, sort_order, updated_at FROM directory_categories WHERE id = ?")
                .bind(id).fetch_optional(pool).await.map_err(dberr)?.map(|row| json!([row.0, row.1, row.2, row.3, row.4, row.5, row.6]))
        }
        Kind::Listings => {
            let Some(l) = repos::directory::find(pool, id).await? else { return Ok(None) };
            Some(json!([l.updated_at, l.status, l.title, l.description, l.description_lexical, l.featured, l.hours, l.image, l.gallery, l.category_id, l.website, l.phone]))
        }
        // Every save through the editors moves `updated_at`; the address and
        // name catch an edit made within the same second.
        Kind::Courses | Kind::CourseCategories | Kind::Services | Kind::ServiceCategories | Kind::BookingStaff => {
            let (table, name) = match kind {
                Kind::Courses => ("lms_courses", "title"),
                Kind::CourseCategories => ("lms_categories", "name"),
                Kind::Services => ("booking_services", "name"),
                Kind::ServiceCategories => ("booking_categories", "name"),
                _ => ("booking_staff", "name"),
            };
            sqlx::query_as::<_, (String, String, String)>(&format!("SELECT updated_at, slug, {name} FROM {table} WHERE id = ?"))
                .bind(id).fetch_optional(pool).await.map_err(dberr)?.map(|row| json!([row.0, row.1, row.2]))
        }
        Kind::Navigation => None,
    };
    Ok(value.map(|value| format!("v1:{:x}", Sha256::digest(value.to_string()))))
}

// ---------------------------------------------------------------------------
// Writes the other repos have no function for
// ---------------------------------------------------------------------------

/// A bio and location for a generated author.
pub async fn set_user_profile(pool: &Db, id: &str, bio: &str, location: &str) -> Result<()> {
    sqlx::query("UPDATE users SET bio = ?, location = ? WHERE id = ?")
        .bind(bio)
        .bind(location)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// A free member who is never emailed. `email_disabled` is what keeps every
/// newsletter and automation away from the address, on top of it being at a
/// domain that accepts no mail. Returns `None` when the address is taken.
pub async fn insert_member(pool: &Db, name: &str, email: &str, created_at: &str) -> Result<Option<String>> {
    let id = ids::new_id();
    let res = sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, name, note, status, email_disabled, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'Dummy content', 'free', 1, ?, ?)",
    )
    .bind(&id)
    .bind(ids::uuid_v4())
    .bind(ids::uuid_v4())
    .bind(email)
    .bind(name)
    .bind(created_at)
    .bind(created_at)
    .execute(pool)
    .await;
    match res {
        Ok(_) => Ok(Some(id)),
        // A unique-address clash is the one expected failure; it is skipped.
        Err(e) if e.to_string().to_ascii_lowercase().contains("unique") || e.to_string().contains("Duplicate") => Ok(None),
        Err(e) => Err(dberr(e)),
    }
}

/// Up to `limit` of the site's public tags, for posts generated without tags
/// of their own.
pub async fn public_tag_ids(pool: &Db, limit: i64) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT id FROM tags WHERE visibility = 'public' ORDER BY created_at LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn public_tag_links(pool: &Db, limit: i64) -> Result<Vec<(String, String)>> {
    sqlx::query_as("SELECT name, slug FROM tags WHERE visibility = 'public' AND name IS NOT NULL AND slug IS NOT NULL ORDER BY created_at, id LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

pub struct Batch {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub created_by: Option<String>,
    pub created_at: String,
    /// How many of each kind this batch still tracks.
    pub remaining: Vec<(Kind, i64)>,
}

impl Batch {
    pub fn total(&self) -> i64 {
        self.remaining.iter().map(|(_, n)| n).sum()
    }
}

/// Every batch that still tracks something, newest first, with the author's
/// name in place of their id.
pub async fn batches(pool: &Db) -> Vec<Batch> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, String)>(
        "SELECT b.id, b.label, b.summary, u.name, b.created_at FROM dummy_batches b \
         LEFT JOIN users u ON u.id = b.created_by ORDER BY b.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let counts = sqlx::query_as::<_, (String, String, i64)>("SELECT batch_id, kind, COUNT(*) FROM dummy_items GROUP BY batch_id, kind")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .map(|(id, label, summary, created_by, created_at)| {
            let mut remaining: Vec<(Kind, i64)> = counts
                .iter()
                .filter(|(b, _, _)| *b == id)
                .filter_map(|(_, k, n)| Kind::from_key(k).map(|k| (k, *n)))
                .collect();
            remaining.sort_by_key(|(k, _)| *k);
            Batch { id, label, summary, created_by, created_at, remaining }
        })
        .filter(|b| b.total() > 0)
        .collect()
}

/// How many of each kind are tracked across every batch.
pub async fn totals(pool: &Db) -> Vec<(Kind, i64)> {
    let mut out: Vec<(Kind, i64)> = sqlx::query_as::<_, (String, i64)>("SELECT kind, COUNT(*) FROM dummy_items GROUP BY kind")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, n)| Kind::from_key(&k).map(|k| (k, n)))
        .collect();
    out.sort_by_key(|(k, _)| *k);
    out
}

pub async fn batch_exists(pool: &Db, id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dummy_batches WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

// ---------------------------------------------------------------------------
// Removal
// ---------------------------------------------------------------------------

/// What a removal did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Removal {
    pub deleted: i64,
    /// Edited or depended on, so left in place as ordinary content.
    pub kept: i64,
    /// Already deleted by hand.
    pub gone: i64,
}

struct Item {
    id: String,
    kind: Kind,
    item_id: String,
    fingerprint: String,
}

/// The order removal goes in: what depends on others first, so a tag is
/// judged after the dummy posts that used it are gone.
const REMOVAL_ORDER: [Kind; vuuut_core::dummies::KINDS] = [
    Kind::Posts,
    Kind::Pages,
    // A course before its store product and its demo students.
    Kind::Courses,
    Kind::Products,
    Kind::Listings,
    Kind::ListingCategories,
    Kind::CourseCategories,
    // Services (and their sample bookings) before the team and categories.
    Kind::Services,
    Kind::BookingStaff,
    Kind::ServiceCategories,
    Kind::Members,
    Kind::Navigation,
    Kind::Tags,
    Kind::Users,
];

/// Remove one batch's dummies, or every batch's when `batch` is `None`.
pub async fn remove(pool: &Db, settings: &SettingsCache, batch: Option<&str>) -> Result<Removal> {
    let rows = match batch {
        Some(b) => sqlx::query_as::<_, (String, String, String, String)>("SELECT id, kind, item_id, fingerprint FROM dummy_items WHERE batch_id = ?")
            .bind(b)
            .fetch_all(pool)
            .await,
        None => sqlx::query_as::<_, (String, String, String, String)>("SELECT id, kind, item_id, fingerprint FROM dummy_items")
            .fetch_all(pool)
            .await,
    }
    .map_err(dberr)?;
    let items: Vec<Item> = rows
        .into_iter()
        .filter_map(|(id, kind, item_id, fingerprint)| Kind::from_key(&kind).map(|kind| Item { id, kind, item_id, fingerprint }))
        .collect();

    let mut out = Removal::default();
    for kind in REMOVAL_ORDER {
        for item in items.iter().filter(|i| i.kind == kind) {
            let outcome = if kind == Kind::Navigation {
                remove_menu_link(pool, settings, item).await?
            } else {
                remove_row(pool, item).await?
            };
            match outcome {
                Outcome::Deleted => out.deleted += 1,
                Outcome::Kept => out.kept += 1,
                Outcome::Gone => out.gone += 1,
            }
            // Deleted, kept or gone, it is no longer a dummy to track.
            sqlx::query("DELETE FROM dummy_items WHERE id = ?").bind(&item.id).execute(pool).await.map_err(dberr)?;
        }
    }
    sqlx::query("DELETE FROM dummy_batches WHERE id NOT IN (SELECT DISTINCT batch_id FROM dummy_items)")
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(out)
}

enum Outcome {
    Deleted,
    Kept,
    Gone,
}

async fn remove_row(pool: &Db, item: &Item) -> Result<Outcome> {
    let current = if item.fingerprint.starts_with("v1:") {
        current_fingerprint(pool, item.kind, &item.item_id).await?
    } else {
        legacy_fingerprint(pool, item.kind, &item.item_id).await?
    };
    let Some(now) = current else {
        return Ok(Outcome::Gone);
    };
    if now != item.fingerprint || in_use(pool, item.kind, &item.item_id).await? {
        return Ok(Outcome::Kept);
    }
    let id = item.item_id.as_str();
    match item.kind {
        Kind::Posts | Kind::Pages => repos::posts::destroy(pool, id).await?,
        Kind::Tags => repos::tags::destroy(pool, id).await?,
        Kind::Users => repos::staff::delete_staff(pool, id).await?,
        Kind::Products => {
            repos::ecom::delete_product(pool, id).await?;
        }
        Kind::Listings => repos::directory::delete(pool, id).await?,
        Kind::ListingCategories => repos::directory::delete_category(pool, id).await?,
        Kind::Courses => repos::lms::delete_course(pool, id).await?,
        Kind::CourseCategories => repos::lms::delete_category(pool, id).await?,
        Kind::Services => {
            // Only sample bookings remain here: a real customer's keeps the
            // service (see `in_use`).
            sqlx::query("DELETE FROM booking_bookings WHERE service_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
            repos::booking::delete_service(pool, id).await?;
        }
        Kind::BookingStaff => repos::booking::delete_staff(pool, id).await?,
        Kind::ServiceCategories => repos::booking::delete_category(pool, id).await?,
        Kind::Members => {
            for sql in [
                "DELETE FROM members_newsletters WHERE member_id = ?",
                "DELETE FROM members_labels WHERE member_id = ?",
                "DELETE FROM members WHERE id = ?",
            ] {
                sqlx::query(sql).bind(id).execute(pool).await.map_err(dberr)?;
            }
        }
        Kind::Navigation => {}
    }
    Ok(Outcome::Deleted)
}

/// Any post still using a tag or author survives this removal, even when
/// it belongs to another dummy batch.
async fn in_use(pool: &Db, kind: Kind, id: &str) -> Result<bool> {
    let queries: &[&str] = match kind {
        Kind::Posts | Kind::Pages => &[
            "SELECT COUNT(*) FROM comments WHERE post_id = ?",
            "SELECT COUNT(*) FROM emails WHERE post_id = ?",
        ],
        Kind::Tags => &["SELECT COUNT(*) FROM posts_tags WHERE tag_id = ?"],
        Kind::Users => &["SELECT COUNT(*) FROM posts_authors WHERE author_id = ?"],
        Kind::Products => &[
            "SELECT COUNT(*) FROM ecom_order_items WHERE product_id = ?",
            "SELECT COUNT(*) FROM ecom_files WHERE product_id = ?",
            "SELECT COUNT(*) FROM ecom_collection_products WHERE product_id = ?",
        ],
        Kind::Members => &[
            "SELECT COUNT(*) FROM members_stripe_customers WHERE member_id = ?",
            "SELECT COUNT(*) FROM ecom_customers WHERE member_id = ?",
            "SELECT COUNT(*) FROM members_labels WHERE member_id = ?",
            "SELECT COUNT(*) FROM lms_enrollments WHERE member_id = ?",
            "SELECT COUNT(*) FROM booking_bookings WHERE member_id = ?",
        ],
        // Enrolled by somebody who is not a demo student.
        Kind::Courses => &["SELECT COUNT(*) FROM lms_enrollments WHERE course_id = ? AND member_id NOT IN (SELECT item_id FROM dummy_items WHERE kind = 'members')"],
        Kind::CourseCategories => &["SELECT COUNT(*) FROM lms_courses WHERE category_id = ?"],
        // Booked by a real customer: sample customers are all @example.com.
        Kind::Services => &["SELECT COUNT(*) FROM booking_bookings WHERE service_id = ? AND email NOT LIKE '%@example.com'"],
        Kind::BookingStaff => &[
            "SELECT COUNT(*) FROM booking_service_staff WHERE staff_id = ?",
            "SELECT COUNT(*) FROM booking_bookings WHERE staff_id = ? AND email NOT LIKE '%@example.com'",
        ],
        Kind::ServiceCategories => &["SELECT COUNT(*) FROM booking_services WHERE category_id = ?"],
        // A category somebody's own listing uses stays.
        Kind::ListingCategories => &["SELECT COUNT(*) FROM directory_listings WHERE category_id = ?"],
        Kind::Listings => &[],
        Kind::Navigation => &[],
    };
    for sql in queries {
        let count = sqlx::query_scalar::<_, i64>(sql).bind(id).fetch_one(pool).await.map_err(dberr)?;
        if count > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A menu link: `<setting key>|<url>`, fingerprinted by its label. Removed
/// only while a link with that exact label and address is still in the menu.
async fn remove_menu_link(pool: &Db, settings: &SettingsCache, item: &Item) -> Result<Outcome> {
    let Some((key, url)) = item.item_id.split_once('|') else {
        return Ok(Outcome::Gone);
    };
    if !matches!(key, "navigation" | "secondary_navigation") {
        return Ok(Outcome::Gone);
    }
    let Some(Value::Array(links)) = settings.get(key) else {
        return Ok(Outcome::Gone);
    };
    let same_url = |l: &Value| l.get("url").and_then(Value::as_str) == Some(url);
    let exact = |l: &Value| same_url(l) && l.get("label").and_then(Value::as_str) == Some(item.fingerprint.as_str());
    if !links.iter().any(same_url) {
        return Ok(Outcome::Gone);
    }
    if links.iter().filter(|link| exact(link)).count() != 1 {
        return Ok(Outcome::Kept);
    }
    let kept: Vec<Value> = links.into_iter().filter(|l| !exact(l)).collect();
    settings.set(pool, key, &Value::Array(kept)).await?;
    Ok(Outcome::Deleted)
}
