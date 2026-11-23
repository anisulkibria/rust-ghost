//! Cookie consent: the declaration and the proof-of-consent log.
//!
//! Every query binds its values. Both tables are VARCHAR-only on MySQL, so
//! nothing here needs [`crate::types::Text`] — except the scan's read of post
//! bodies and code injection, which are `MEDIUMTEXT` and go through it.

use super::dberr;
use crate::types::Text;
use crate::Db;
use vuuut_core::{dates, ids, Result};

// ---------------------------------------------------------------------------
// the declaration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Declaration {
    pub id: String,
    pub name: String,
    pub provider: String,
    /// `necessary`, `preferences`, `analytics` or `marketing`.
    pub category: String,
    pub duration: String,
    pub purpose: String,
    /// The `vuuut_core::cookies::TRACKERS` key it was added from, or ''.
    pub tracker_key: String,
    pub created_at: String,
    pub updated_at: String,
}

const DECL_COLS: &str = "id, name, provider, category, duration, purpose, tracker_key, created_at, updated_at";

/// In the order a visitor reads them: by category, then by name.
pub async fn declarations(pool: &Db) -> Result<Vec<Declaration>> {
    sqlx::query_as::<_, Declaration>(&format!(
        "SELECT {DECL_COLS} FROM cookie_declarations ORDER BY \
         CASE category WHEN 'necessary' THEN 0 WHEN 'preferences' THEN 1 WHEN 'analytics' THEN 2 ELSE 3 END, \
         name"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// Adds one row and returns only its id — a create that succeeded must not be
/// able to report failure through a read-back (see `popups::insert`).
pub async fn declare(
    pool: &Db,
    d: &vuuut_core::cookies::Declared,
    tracker_key: &str,
) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO cookie_declarations (id, name, provider, category, duration, purpose, tracker_key, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&d.name)
    .bind(&d.provider)
    .bind(d.category.key())
    .bind(&d.duration)
    .bind(&d.purpose)
    .bind(tracker_key)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

/// Whether a cookie of this name is already declared, so a second click on
/// "Add to declaration" does not make a second row.
pub async fn is_declared(pool: &Db, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cookie_declarations WHERE name = ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

pub async fn undeclare(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM cookie_declarations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the consent log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Consent {
    pub id: String,
    pub consent_id: String,
    pub action: String,
    pub categories: String,
    pub revision: i64,
    /// 0 or 1 — not `bool`, which sqlx `Any` cannot decode from MySQL.
    pub gpc: i64,
    pub page: String,
    pub created_at: String,
}

const CONSENT_COLS: &str = "id, consent_id, action, categories, revision, gpc, page, created_at";

pub async fn record(
    pool: &Db,
    consent_id: &str,
    choice: &vuuut_core::cookies::Choice,
    revision: i64,
    gpc: bool,
    page: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cookie_consents (id, consent_id, action, categories, revision, gpc, page, created_at) \
         VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(ids::new_id())
    .bind(consent_id)
    .bind(choice.action.key())
    .bind(choice.categories())
    .bind(revision)
    .bind(if gpc { 1_i64 } else { 0_i64 })
    .bind(page)
    .bind(dates::now_db())
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Newest first. `search` is a consent-id prefix that has already been
/// through `vuuut_core::cookies::log_query` (hex and dashes only).
pub async fn log(pool: &Db, search: &str, limit: i64) -> Result<Vec<Consent>> {
    let limit = limit.clamp(1, 10_000);
    if search.is_empty() {
        sqlx::query_as::<_, Consent>(&format!(
            "SELECT {CONSENT_COLS} FROM cookie_consents ORDER BY created_at DESC, id DESC LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)
    } else {
        sqlx::query_as::<_, Consent>(&format!(
            "SELECT {CONSENT_COLS} FROM cookie_consents WHERE consent_id LIKE ? ORDER BY created_at DESC, id DESC LIMIT ?"
        ))
        .bind(format!("{search}%"))
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)
    }
}

pub async fn count(pool: &Db) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cookie_consents")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// `(action, categories, n)` since a moment — the input to
/// `vuuut_core::cookies::Summary::from_rows`, which does the arithmetic.
pub async fn tally(pool: &Db, since: &str) -> Result<Vec<(String, String, i64)>> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT action, categories, COUNT(*) FROM cookie_consents WHERE created_at >= ? \
         GROUP BY action, categories",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// Deletes every record older than `days`. Returns how many went.
pub async fn purge_older_than(pool: &Db, days: i64) -> Result<u64> {
    let cutoff = dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days.max(1))));
    let done = sqlx::query("DELETE FROM cookie_consents WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected())
}

// ---------------------------------------------------------------------------
// what the scan reads
// ---------------------------------------------------------------------------

/// A published post or page whose markup could carry a tracker.
#[derive(Debug, Clone)]
pub struct Scannable {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub markup: String,
}

#[derive(sqlx::FromRow)]
struct ScanRow {
    id: String,
    title: String,
    #[sqlx(rename = "type")]
    kind: String,
    html: Option<Text>,
    codeinjection_head: Option<Text>,
    codeinjection_foot: Option<Text>,
}

/// Published posts and pages that contain a script or an iframe, or have
/// code injection of their own. The `LIKE`s narrow the read to the rows worth
/// scanning; the matching itself is `vuuut_core::cookies::detect`.
pub async fn scannable(pool: &Db, limit: i64) -> Result<Vec<Scannable>> {
    let rows = sqlx::query_as::<_, ScanRow>(
        "SELECT id, title, type, html, codeinjection_head, codeinjection_foot FROM posts \
         WHERE status = 'published' AND ( \
           html LIKE '%<script%' OR html LIKE '%<iframe%' \
           OR (codeinjection_head IS NOT NULL AND codeinjection_head <> '') \
           OR (codeinjection_foot IS NOT NULL AND codeinjection_foot <> '') ) \
         ORDER BY published_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 5_000))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let part = |t: Option<Text>| t.map(Text::into_string).unwrap_or_default();
            Scannable {
                id: r.id,
                title: r.title,
                kind: r.kind,
                markup: format!("{}\n{}\n{}", part(r.codeinjection_head), part(r.codeinjection_foot), part(r.html)),
            }
        })
        .collect())
}
