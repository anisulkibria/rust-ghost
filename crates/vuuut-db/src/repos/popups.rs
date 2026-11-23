//! Popups, and the leads they capture.
//!
//! Every query binds its values; nothing from a request is ever spliced into
//! SQL. Both tables are sized to stay inside the row on MySQL, so nothing
//! here needs [`crate::types::Text`].

use super::dberr;
use crate::Db;
use vuuut_core::{dates, ids, Result};

const COLS: &str = "id, name, status, kind, heading, body, button_label, button_url, \
success_message, capture_email, capture_name, trigger_kind, trigger_value, frequency, \
frequency_days, audience, surface, targeting, paths, starts_at, ends_at, impressions, \
submissions, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Popup {
    pub id: String,
    pub name: String,
    /// `draft` or `live`.
    pub status: String,
    pub kind: String,
    pub heading: String,
    pub body: String,
    pub button_label: String,
    pub button_url: String,
    pub success_message: String,
    /// 0 or 1, not `bool`: sqlx's `Any` driver will not decode a MySQL
    /// boolean column into one. See `captures_email` below.
    pub capture_email: i64,
    pub capture_name: i64,
    pub trigger_kind: String,
    pub trigger_value: i64,
    pub frequency: String,
    pub frequency_days: i64,
    pub audience: String,
    /// Which kind of page — `any`, `posts`, `pages`, `home`, `archives`.
    pub surface: String,
    pub targeting: String,
    /// One path prefix per line.
    pub paths: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub impressions: i64,
    pub submissions: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Popup {
    pub fn is_live(&self) -> bool {
        self.status == "live"
    }
    pub fn captures_email(&self) -> bool {
        self.capture_email != 0
    }
    pub fn captures_name(&self) -> bool {
        self.capture_name != 0
    }
    pub fn path_rules(&self) -> Vec<String> {
        vuuut_core::popups::parse_paths(&self.paths)
    }
}

/// What a write supplies. The admin builds one of these out of a form after
/// `vuuut_core::popups` has normalised every field.
#[derive(Debug, Clone)]
pub struct PopupInput {
    pub name: String,
    pub status: String,
    pub kind: String,
    pub heading: String,
    pub body: String,
    pub button_label: String,
    pub button_url: String,
    pub success_message: String,
    pub capture_email: i64,
    pub capture_name: i64,
    pub trigger_kind: String,
    pub trigger_value: i64,
    pub frequency: String,
    pub frequency_days: i64,
    pub audience: String,
    pub surface: String,
    pub targeting: String,
    pub paths: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
}

pub async fn list(pool: &Db) -> Result<Vec<Popup>> {
    sqlx::query_as::<_, Popup>(&format!(
        "SELECT {COLS} FROM popups ORDER BY CASE status WHEN 'live' THEN 0 ELSE 1 END, updated_at DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn find(pool: &Db, id: &str) -> Result<Option<Popup>> {
    sqlx::query_as::<_, Popup>(&format!("SELECT {COLS} FROM popups WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// Every popup that is switched on. The caller narrows by path, audience and
/// schedule — those are `vuuut_core::popups`' decisions, and keeping them out
/// of SQL is what lets them be unit-tested without a database.
pub async fn live(pool: &Db) -> Result<Vec<Popup>> {
    sqlx::query_as::<_, Popup>(&format!(
        "SELECT {COLS} FROM popups WHERE status = 'live' ORDER BY updated_at DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// What kind of page a path is: `post`, `page`, `tag`, `author`, `home` or
/// `other`.
///
/// Asked only when a live popup actually filters by kind — see
/// `Surface::needs_lookup` — so a site that targets by path alone never runs
/// this.
///
/// It reads the slug rather than the router: resolving a path properly means
/// building a `RouterTree` from routes.yaml, which is not cached anywhere and
/// would be rebuilt per page view. One indexed lookup on the last segment is
/// right for every default permalink and for `/yyyy/mm/slug/` alike. A custom
/// route that renames a collection is the case it cannot see, and that page
/// reads as `other` — which matches only "anywhere on the site", the safe
/// way to be wrong.
pub async fn kind_of_path(pool: &Db, path: &str) -> &'static str {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return "home";
    }
    if trimmed == "tag" || trimmed.starts_with("tag/") {
        return "tag";
    }
    if trimmed == "author" || trimmed.starts_with("author/") {
        return "author";
    }
    // `/slug/page/2/` is still the thing it pages through.
    let slug = trimmed
        .split('/')
        .filter(|seg| !seg.is_empty())
        .filter(|seg| !seg.chars().all(|c| c.is_ascii_digit()))
        .filter(|seg| *seg != "page")
        .next_back()
        .unwrap_or("");
    if slug.is_empty() {
        return "other";
    }
    match sqlx::query_scalar::<_, String>(
        "SELECT type FROM posts WHERE slug = ? AND status = 'published' LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(kind)) if kind == "page" => "page",
        Ok(Some(_)) => "post",
        _ => "other",
    }
}

/// Whether anything at all is live, for deciding whether a page needs the
/// visitor script. One cheap count on an indexed column, on every page view.
pub async fn any_live(pool: &Db) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM popups WHERE status = 'live'")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

/// Creates the row and hands back its id.
///
/// It returned the whole `Popup` by reading it back, until a read-back that
/// failed to decode made the handler report "could not be created" for a row
/// that had just been created — and the operator, reasonably, pressed the
/// button again. A create that has succeeded must not be able to report
/// failure, so the only thing it reports is the id it generated.
pub async fn insert(pool: &Db, input: &PopupInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO popups (id, name, status, kind, heading, body, button_label, button_url, \
         success_message, capture_email, capture_name, trigger_kind, trigger_value, frequency, \
         frequency_days, audience, surface, targeting, paths, starts_at, ends_at, impressions, \
         submissions, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,0,?,?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&input.status)
    .bind(&input.kind)
    .bind(&input.heading)
    .bind(&input.body)
    .bind(&input.button_label)
    .bind(&input.button_url)
    .bind(&input.success_message)
    .bind(input.capture_email)
    .bind(input.capture_name)
    .bind(&input.trigger_kind)
    .bind(input.trigger_value)
    .bind(&input.frequency)
    .bind(input.frequency_days)
    .bind(&input.audience)
    .bind(&input.surface)
    .bind(&input.targeting)
    .bind(&input.paths)
    .bind(&input.starts_at)
    .bind(&input.ends_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(id)
}

/// Everything except the counters, which are nobody's business but the
/// public endpoint's.
pub async fn update(pool: &Db, id: &str, input: &PopupInput) -> Result<()> {
    let now = dates::now_db();
    sqlx::query(
        "UPDATE popups SET name = ?, status = ?, kind = ?, heading = ?, body = ?, \
         button_label = ?, button_url = ?, success_message = ?, capture_email = ?, \
         capture_name = ?, trigger_kind = ?, trigger_value = ?, frequency = ?, \
         frequency_days = ?, audience = ?, surface = ?, targeting = ?, paths = ?, starts_at = ?, \
         ends_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.status)
    .bind(&input.kind)
    .bind(&input.heading)
    .bind(&input.body)
    .bind(&input.button_label)
    .bind(&input.button_url)
    .bind(&input.success_message)
    .bind(input.capture_email)
    .bind(input.capture_name)
    .bind(&input.trigger_kind)
    .bind(input.trigger_value)
    .bind(&input.frequency)
    .bind(input.frequency_days)
    .bind(&input.audience)
    .bind(&input.surface)
    .bind(&input.targeting)
    .bind(&input.paths)
    .bind(&input.starts_at)
    .bind(&input.ends_at)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

pub async fn set_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE popups SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Deletes the popup and the leads it captured.
///
/// The leads go with it deliberately: they are the record of people who
/// answered *this* popup, and keeping them attached to an id nothing points
/// at is how a database grows rows nobody can explain. Export before
/// deleting — the admin screen says so.
pub async fn destroy(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM popup_leads WHERE popup_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    sqlx::query("DELETE FROM popups WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// One more person saw it.
///
/// Deliberately approximate: incremented in place with no transaction and no
/// error path, because a lost increment costs a decimal place on a rate and
/// nothing else reads it. Making this exact would mean a row per page view.
pub async fn record_impression(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE popups SET impressions = impressions + 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await;
}

/// Whether this address already came through this popup.
///
/// What stops one person refreshing being counted as ten conversions, and
/// what keeps a second submission from making a second lead row.
pub async fn lead_exists(pool: &Db, popup_id: &str, email: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM popup_leads WHERE popup_id = ? AND email = ?")
        .bind(popup_id)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

/// Records a lead and counts the conversion in one go.
pub async fn record_lead(
    pool: &Db,
    popup_id: &str,
    email: &str,
    name: &str,
    source_url: &str,
    member_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO popup_leads (id, popup_id, email, name, source_url, member_id, created_at) \
         VALUES (?,?,?,?,?,?,?)",
    )
    .bind(ids::new_id())
    .bind(popup_id)
    .bind(email)
    .bind(name)
    .bind(source_url)
    .bind(member_id)
    .bind(dates::now_db())
    .execute(pool)
    .await
    .map_err(dberr)?;
    sqlx::query("UPDATE popups SET submissions = submissions + 1 WHERE id = ?")
        .bind(popup_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Lead {
    pub id: String,
    pub popup_id: String,
    pub email: String,
    pub name: String,
    pub source_url: String,
    pub member_id: Option<String>,
    pub created_at: String,
}

pub async fn leads(pool: &Db, popup_id: &str, limit: i64) -> Result<Vec<Lead>> {
    sqlx::query_as::<_, Lead>(
        "SELECT id, popup_id, email, name, source_url, member_id, created_at \
         FROM popup_leads WHERE popup_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(popup_id)
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// Every lead across every popup, newest first — the module's own list.
pub async fn all_leads(pool: &Db, limit: i64) -> Result<Vec<Lead>> {
    sqlx::query_as::<_, Lead>(
        "SELECT id, popup_id, email, name, source_url, member_id, created_at \
         FROM popup_leads ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
    .map_err(dberr)
}
