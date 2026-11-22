//! Advertising: advertisers, ads, and their daily counts.
//!
//! Every value is bound. Every column is VARCHAR or an integer on MySQL, so
//! nothing here needs `types::Text`; counters and flags are read as `i64`.
//!
//! Counting is deliberately approximate and never fails the caller: an
//! impression is an `UPDATE … + 1` with no transaction, because a lost
//! increment costs a decimal place and a row lock per page view would cost
//! the site its speed.

use super::dberr;
use crate::Db;
use ghost_core::ads::AdInput;
use ghost_core::{dates, ids, Result};

// ---------------------------------------------------------------------------
// advertisers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Advertiser {
    pub id: String,
    pub name: String,
    pub email: String,
    pub website: String,
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
}

const ADV_COLS: &str = "id, name, email, website, notes, created_at, updated_at";

pub async fn advertisers(pool: &Db) -> Result<Vec<Advertiser>> {
    sqlx::query_as::<_, Advertiser>(&format!("SELECT {ADV_COLS} FROM ad_advertisers ORDER BY name"))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

pub async fn advertiser(pool: &Db, id: &str) -> Result<Option<Advertiser>> {
    sqlx::query_as::<_, Advertiser>(&format!("SELECT {ADV_COLS} FROM ad_advertisers WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

pub async fn save_advertiser(pool: &Db, id: Option<&str>, name: &str, email: &str, website: &str, notes: &str) -> Result<String> {
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE ad_advertisers SET name = ?, email = ?, website = ?, notes = ?, updated_at = ? WHERE id = ?")
                .bind(name)
                .bind(email)
                .bind(website)
                .bind(notes)
                .bind(&now)
                .bind(id)
                .execute(pool)
                .await
                .map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO ad_advertisers (id, name, email, website, notes, created_at, updated_at) VALUES (?,?,?,?,?,?,?)")
                .bind(&id)
                .bind(name)
                .bind(email)
                .bind(website)
                .bind(notes)
                .bind(&now)
                .bind(&now)
                .execute(pool)
                .await
                .map_err(dberr)?;
            Ok(id)
        }
    }
}

/// Deletes the advertiser. Their ads stay, as house ads with no advertiser —
/// deleting a client should not silently take creatives off the site.
pub async fn delete_advertiser(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("UPDATE ads SET advertiser_id = NULL WHERE advertiser_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    sqlx::query("DELETE FROM ad_advertisers WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// ads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Ad {
    pub id: String,
    pub advertiser_id: Option<String>,
    pub name: String,
    pub status: String,
    pub kind: String,
    pub placement: String,
    pub paragraph: i64,
    pub slot: String,
    pub image: String,
    pub link: String,
    pub alt: String,
    pub headline: String,
    pub body: String,
    pub cta: String,
    pub code: String,
    pub audience: String,
    pub device: String,
    pub targeting: String,
    /// One rule per line.
    pub rules: String,
    pub weight: i64,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub max_impressions: i64,
    pub max_clicks: i64,
    pub cpm_cents: i64,
    pub cpc_cents: i64,
    pub impressions: i64,
    pub clicks: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Ad {
    pub fn rule_list(&self) -> Vec<String> {
        self.rules.lines().map(str::to_string).filter(|l| !l.is_empty()).collect()
    }
}

const AD_COLS: &str = "id, advertiser_id, name, status, kind, placement, paragraph, slot, image, link, alt, \
headline, body, cta, code, audience, device, targeting, rules, weight, starts_at, ends_at, max_impressions, \
max_clicks, cpm_cents, cpc_cents, impressions, clicks, created_at, updated_at";

/// Live first, then by most recently changed.
pub async fn list(pool: &Db) -> Result<Vec<Ad>> {
    sqlx::query_as::<_, Ad>(&format!(
        "SELECT {AD_COLS} FROM ads ORDER BY CASE status WHEN 'live' THEN 0 WHEN 'paused' THEN 1 ELSE 2 END, updated_at DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn live(pool: &Db) -> Result<Vec<Ad>> {
    sqlx::query_as::<_, Ad>(&format!("SELECT {AD_COLS} FROM ads WHERE status = 'live'"))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

pub async fn find(pool: &Db, id: &str) -> Result<Option<Ad>> {
    sqlx::query_as::<_, Ad>(&format!("SELECT {AD_COLS} FROM ads WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

pub async fn of_advertiser(pool: &Db, advertiser_id: &str) -> Result<Vec<Ad>> {
    sqlx::query_as::<_, Ad>(&format!("SELECT {AD_COLS} FROM ads WHERE advertiser_id = ? ORDER BY name"))
        .bind(advertiser_id)
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

/// The write columns, in bind order. Shared by insert and update so the two
/// can never disagree about how many values there are — the mistake that
/// once made every Popups save a silent no-op.
const WRITE_COLS: [&str; 25] = [
    "advertiser_id", "name", "status", "kind", "placement", "paragraph", "slot", "image", "link", "alt",
    "headline", "body", "cta", "code", "audience", "device", "targeting", "rules", "weight", "starts_at",
    "ends_at", "max_impressions", "max_clicks", "cpm_cents", "cpc_cents",
];

fn bind_input<'q>(
    q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    a: &'q AdInput,
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    q.bind(a.advertiser_id.clone())
        .bind(&a.name)
        .bind(a.status.key())
        .bind(a.kind.key())
        .bind(a.placement.key())
        .bind(a.paragraph)
        .bind(&a.slot)
        .bind(&a.image)
        .bind(&a.link)
        .bind(&a.alt)
        .bind(&a.headline)
        .bind(&a.body)
        .bind(&a.cta)
        .bind(&a.code)
        .bind(a.audience.key())
        .bind(a.device.key())
        .bind(a.targeting.key())
        .bind(a.rules.join("\n"))
        .bind(a.weight)
        .bind(a.starts_at.clone())
        .bind(a.ends_at.clone())
        .bind(a.max_impressions)
        .bind(a.max_clicks)
        .bind(a.cpm_cents)
        .bind(a.cpc_cents)
}

/// Creates the ad and returns only its id (see `popups::insert` on why a
/// create must not report failure through a read-back).
pub async fn insert(pool: &Db, a: &AdInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    let sql = format!(
        "INSERT INTO ads (id, {cols}, impressions, clicks, created_at, updated_at) VALUES (?, {marks}, 0, 0, ?, ?)",
        cols = WRITE_COLS.join(", "),
        marks = vec!["?"; WRITE_COLS.len()].join(", "),
    );
    let q = sqlx::query(&sql).bind(&id);
    bind_input(q, a).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn update(pool: &Db, id: &str, a: &AdInput) -> Result<()> {
    let sql = format!(
        "UPDATE ads SET {sets}, updated_at = ? WHERE id = ?",
        sets = WRITE_COLS.iter().map(|c| format!("{c} = ?")).collect::<Vec<_>>().join(", "),
    );
    let q = sqlx::query(&sql);
    bind_input(q, a).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE ads SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Deletes the ad and its daily counts. Export first if the numbers matter —
/// the screen says so.
pub async fn destroy(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM ad_daily WHERE ad_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM ads WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// counting
// ---------------------------------------------------------------------------

/// One more impression or click for each id, on the running total and on
/// today's row. Only ids of live ads are counted, so a forged id is a no-op.
///
/// The daily row is created with `INSERT` and a duplicate is ignored — the
/// one portable spelling of "create if missing" across both engines — then
/// incremented.
pub async fn count(pool: &Db, ids: &[String], clicks: bool) {
    let day = dates::now_db().get(..10).unwrap_or("").to_string();
    let col = if clicks { "clicks" } else { "impressions" };
    for id in ids {
        let done = sqlx::query(&format!("UPDATE ads SET {col} = {col} + 1 WHERE id = ? AND status = 'live'"))
            .bind(id)
            .execute(pool)
            .await;
        if !matches!(done, Ok(r) if r.rows_affected() == 1) {
            continue;
        }
        let _ = sqlx::query("INSERT INTO ad_daily (ad_id, day, impressions, clicks) VALUES (?, ?, 0, 0)")
            .bind(id)
            .bind(&day)
            .execute(pool)
            .await;
        let _ = sqlx::query(&format!("UPDATE ad_daily SET {col} = {col} + 1 WHERE ad_id = ? AND day = ?"))
            .bind(id)
            .bind(&day)
            .execute(pool)
            .await;
    }
}

/// `(day, impressions, clicks)` from `since` (a `YYYY-MM-DD`), optionally for
/// the given ads only. Days with nothing are absent; the caller fills them.
pub async fn daily(pool: &Db, since: &str, only: Option<&[String]>) -> Result<Vec<(String, i64, i64)>> {
    let rows = sqlx::query_as::<_, (String, String, i64, i64)>(
        "SELECT ad_id, day, impressions, clicks FROM ad_daily WHERE day >= ? ORDER BY day",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let mut out: Vec<(String, i64, i64)> = Vec::new();
    for (ad, day, i, c) in rows {
        if only.is_some_and(|ids| !ids.contains(&ad)) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.0 == day => {
                last.1 += i;
                last.2 += c;
            }
            _ => out.push((day, i, c)),
        }
    }
    Ok(out)
}

/// `(ad_id, impressions, clicks)` per ad since a day.
///
/// Added up here rather than with `SUM()`: MySQL returns `SUM` of an integer
/// column as DECIMAL, which sqlx `Any` cannot decode — the read would fail
/// on production while every SQLite test passed.
pub async fn totals_since(pool: &Db, since: &str) -> Result<Vec<(String, i64, i64)>> {
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT ad_id, impressions, clicks FROM ad_daily WHERE day >= ? ORDER BY ad_id",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let mut out: Vec<(String, i64, i64)> = Vec::new();
    for (ad, i, c) in rows {
        match out.last_mut() {
            Some(last) if last.0 == ad => {
                last.1 += i;
                last.2 += c;
            }
            _ => out.push((ad, i, c)),
        }
    }
    Ok(out)
}
