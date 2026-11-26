//! Podcast: shows, episodes, hosts and guests, sponsors, downloads, listener
//! requests and reviews.
//!
//! Every value is bound. Long columns are MEDIUMTEXT on MySQL and are read
//! through [`crate::types::Text`] (`#[sqlx(try_from)]`), so they arrive as
//! strings on both engines. Writes that change a public page call
//! `content::touch()` so the page cache empties. A download is one row per
//! listener, episode and day, which is the whole of the de-duplication.

use super::dberr;
use crate::types::Text;
use crate::Db;
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_core::podcast::{self as rules, EpisodeInput, PersonInput, ReviewInput, Settings, ShowInput, SponsorInput};
use vuuut_core::{dates, ids, Result};

fn touch() {
    crate::content::touch();
}

fn link(s: &Settings, internal: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(internal))
}

async fn unique_slug(pool: &Db, table: &str, scope: Option<(&str, &str)>, wanted: &str, fallback: &str, reserved: &[&str], except: &str) -> String {
    let base = match wanted {
        "" => vuuut_core::directory::slugify(fallback),
        w if reserved.contains(&w) => format!("{w}-{}", vuuut_core::directory::slugify(fallback)),
        w => w.to_string(),
    };
    let base = if base.is_empty() { "untitled".to_string() } else { base };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = match scope {
            Some((col, val)) => sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE slug = ? AND id <> ? AND {col} = ?")).bind(&candidate).bind(except).bind(val).fetch_one(pool).await.unwrap_or(0),
            None => sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE slug = ? AND id <> ?")).bind(&candidate).bind(except).fetch_one(pool).await.unwrap_or(0),
        };
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

/// `(secret, salt)`: the private-feed signing secret and the listener-hash
/// salt. Made on first use if the migration's row is missing.
pub async fn config(pool: &Db) -> (String, String) {
    if let Ok(Some(row)) = sqlx::query_as::<_, (String, String)>("SELECT secret, salt FROM podcast_config WHERE id = 1").fetch_optional(pool).await {
        return row;
    }
    let secret = format!("{}{}", ids::new_id(), ids::new_id());
    let salt = ids::new_id();
    let _ = sqlx::query(crate::dialect::sql("INSERT OR IGNORE INTO podcast_config (id, secret, salt) VALUES (1, ?, ?)")).bind(&secret).bind(&salt).execute(pool).await;
    sqlx::query_as::<_, (String, String)>("SELECT secret, salt FROM podcast_config WHERE id = 1").fetch_one(pool).await.unwrap_or((secret, salt))
}

/// A member's status (`free`, `paid`, `comped`…) by uuid — all a private
/// feed needs to know about who holds it.
pub async fn member_status(pool: &Db, uuid: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM members WHERE uuid = ? LIMIT 1").bind(uuid).fetch_optional(pool).await.ok().flatten()
}

/// A new secret: every member's private feed address changes.
pub async fn rotate_secret(pool: &Db) -> Result<()> {
    let secret = format!("{}{}", ids::new_id(), ids::new_id());
    config(pool).await;
    sqlx::query("UPDATE podcast_config SET secret = ? WHERE id = 1").bind(secret).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// shows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Show {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub subtitle: String,
    #[sqlx(try_from = "Text")]
    pub description: String,
    pub artwork: String,
    pub banner: String,
    pub author: String,
    pub owner_name: String,
    pub owner_email: String,
    pub category: String,
    pub category2: String,
    pub language: String,
    pub explicit: i64,
    pub show_type: String,
    pub copyright: String,
    #[sqlx(try_from = "Text")]
    pub links: String,
    pub funding_url: String,
    pub funding_label: String,
    pub guid: String,
    pub locked: i64,
    pub complete: i64,
    pub new_feed_url: String,
    pub tint: String,
    pub status: String,
    pub sort_order: i64,
    pub seo_title: String,
    pub seo_description: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Show {
    pub fn links(&self) -> Vec<(String, String)> {
        rules::links_from_json(&self.links)
    }
    pub fn public(&self) -> bool {
        self.status != "draft"
    }
}

const SHOW_COLS: &str = "id, slug, title, subtitle, COALESCE(description, '') AS description, artwork, banner, author, owner_name, owner_email, category, category2, \
language, explicit, show_type, copyright, COALESCE(links, '') AS links, funding_url, funding_label, guid, locked, complete, new_feed_url, tint, status, sort_order, \
seo_title, seo_description, created_at, updated_at";

/// `listed_only`: the shows on the public hub (active). Hidden shows still
/// answer at their own address and feed.
pub async fn shows(pool: &Db, listed_only: bool) -> Vec<Show> {
    let filter = if listed_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Show>(&format!("SELECT {SHOW_COLS} FROM podcast_shows{filter} ORDER BY sort_order, title")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_show(pool: &Db, id: &str) -> Option<Show> {
    sqlx::query_as::<_, Show>(&format!("SELECT {SHOW_COLS} FROM podcast_shows WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn show_by_slug(pool: &Db, slug: &str) -> Option<Show> {
    sqlx::query_as::<_, Show>(&format!("SELECT {SHOW_COLS} FROM podcast_shows WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_show(pool: &Db, id: Option<&str>, i: &ShowInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO podcast_shows (id, slug, title, guid, created_at, updated_at) VALUES (?,?,?,?,?,?)")
                .bind(&id).bind(ids::new_id()).bind(&i.title).bind(ids::uuid_v4()).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "podcast_shows", None, &i.slug, &i.title, rules::RESERVED_SLUGS, &id).await;
    sqlx::query(
        "UPDATE podcast_shows SET slug = ?, title = ?, subtitle = ?, description = ?, artwork = ?, banner = ?, author = ?, owner_name = ?, owner_email = ?, \
         category = ?, category2 = ?, language = ?, explicit = ?, show_type = ?, copyright = ?, links = ?, funding_url = ?, funding_label = ?, locked = ?, \
         complete = ?, new_feed_url = ?, tint = ?, status = ?, sort_order = ?, seo_title = ?, seo_description = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.title).bind(&i.subtitle).bind(&i.description).bind(&i.artwork).bind(&i.banner).bind(&i.author).bind(&i.owner_name)
    .bind(&i.owner_email).bind(&i.category).bind(&i.category2).bind(&i.language).bind(i64::from(i.explicit)).bind(&i.show_type).bind(&i.copyright)
    .bind(rules::links_json(&i.links)).bind(&i.funding_url).bind(&i.funding_label).bind(i64::from(i.locked)).bind(i64::from(i.complete))
    .bind(&i.new_feed_url).bind(&i.tint).bind(&i.status).bind(i.sort_order).bind(&i.seo_title).bind(&i.seo_description).bind(&now).bind(&id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    set_credits(pool, "show", &id, &i.hosts).await?;
    Ok(id)
}

/// The show, its episodes and everything hanging off them. Requests keep
/// their text but lose the show.
pub async fn delete_show(pool: &Db, id: &str) -> Result<()> {
    touch();
    let eps: Vec<String> = sqlx::query_scalar("SELECT id FROM podcast_episodes WHERE show_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default();
    for e in &eps {
        delete_episode(pool, e).await?;
    }
    sqlx::query("DELETE FROM podcast_credits WHERE owner_kind = 'show' AND owner_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_reviews WHERE show_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_downloads WHERE show_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("UPDATE podcast_requests SET show_id = NULL WHERE show_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_shows WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// episodes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Episode {
    pub id: String,
    pub show_id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    #[sqlx(try_from = "Text")]
    pub notes: String,
    #[sqlx(try_from = "Text")]
    pub notes_lexical: String,
    #[sqlx(try_from = "Text")]
    pub notes_html: String,
    pub audio_url: String,
    pub audio_type: String,
    pub audio_bytes: i64,
    pub duration: i64,
    pub video_url: String,
    pub artwork: String,
    pub season: i64,
    pub number: i64,
    pub episode_type: String,
    pub explicit: i64,
    pub access: String,
    pub status: String,
    pub published_at: Option<String>,
    #[sqlx(try_from = "Text")]
    pub transcript: String,
    #[sqlx(try_from = "Text")]
    pub chapters: String,
    pub guid: String,
    pub featured: i64,
    pub downloads: i64,
    pub seo_title: String,
    pub seo_description: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Episode {
    pub fn chapters(&self) -> Vec<rules::Chapter> {
        rules::chapters_from_json(&self.chapters)
    }
    pub fn access(&self) -> rules::Access {
        rules::Access::from_key(&self.access)
    }
    /// Published and its time has come.
    pub fn is_live(&self) -> bool {
        self.status == "published" && self.published_at.as_deref().is_some_and(|p| p <= dates::now_db().as_str())
    }
    pub fn is_scheduled(&self) -> bool {
        self.status == "published" && !self.is_live()
    }
    pub fn label(&self) -> String {
        rules::episode_label(self.season, self.number, &self.episode_type)
    }
    pub fn notes_markup(&self) -> String {
        if self.notes_html.trim().is_empty() { vuuut_core::directory::paragraphs(&self.notes) } else { self.notes_html.clone() }
    }
}

const EPISODE_COLS: &str = "id, show_id, slug, title, summary, COALESCE(notes, '') AS notes, COALESCE(notes_lexical, '') AS notes_lexical, \
COALESCE(notes_html, '') AS notes_html, audio_url, audio_type, audio_bytes, duration, video_url, artwork, season, number, episode_type, explicit, access, \
status, published_at, COALESCE(transcript, '') AS transcript, COALESCE(chapters, '') AS chapters, guid, featured, downloads, seo_title, seo_description, \
created_at, updated_at";

#[derive(Debug, Clone, Default)]
pub struct EpisodeQuery<'a> {
    pub show_id: Option<&'a str>,
    /// `published` | `draft` | `scheduled` | None (all)
    pub status: Option<&'a str>,
    /// Only what a visitor may see listed: published, due, in a public show.
    pub live: bool,
    pub season: Option<i64>,
    pub search: Option<&'a str>,
    /// `newest` (default) | `oldest` | `popular` | `number`
    pub order: &'a str,
    pub limit: i64,
    pub offset: i64,
}

pub async fn episodes(pool: &Db, q: &EpisodeQuery<'_>) -> (Vec<Episode>, i64) {
    let mut sql = String::from(" FROM podcast_episodes WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    let now = dates::now_db();
    if let Some(s) = q.show_id {
        sql.push_str(" AND show_id = ?");
        binds.push(s.to_string());
    }
    if q.live {
        sql.push_str(" AND status = 'published' AND published_at IS NOT NULL AND published_at <= ? AND show_id IN (SELECT id FROM podcast_shows WHERE status <> 'draft')");
        binds.push(now.clone());
    }
    match q.status {
        Some("draft") => sql.push_str(" AND status = 'draft'"),
        Some("published") => {
            sql.push_str(" AND status = 'published' AND published_at <= ?");
            binds.push(now.clone());
        }
        Some("scheduled") => {
            sql.push_str(" AND status = 'published' AND published_at > ?");
            binds.push(now.clone());
        }
        _ => {}
    }
    if let Some(season) = q.season {
        sql.push_str(" AND season = ?");
        binds.push(season.to_string());
    }
    let words: Vec<String> = q.search.unwrap_or("").split_whitespace().take(6).map(|w| format!("%{}%", w.to_lowercase())).collect();
    for _ in &words {
        sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(summary) LIKE ? OR LOWER(COALESCE(notes, '')) LIKE ?)");
    }
    let order = match q.order {
        "oldest" => "COALESCE(published_at, created_at) ASC",
        "popular" => "downloads DESC, published_at DESC",
        "number" => "season ASC, number ASC, published_at ASC",
        _ => "CASE WHEN status = 'draft' THEN 0 ELSE 1 END, COALESCE(published_at, created_at) DESC",
    };
    let count_sql = format!("SELECT COUNT(*){sql}");
    let list_sql = format!("SELECT {EPISODE_COLS}{sql} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut c = sqlx::query_scalar::<_, i64>(&count_sql);
    let mut l = sqlx::query_as::<_, Episode>(&list_sql);
    for b in &binds {
        c = c.bind(b.clone());
        l = l.bind(b.clone());
    }
    for w in &words {
        for _ in 0..3 {
            c = c.bind(w.clone());
            l = l.bind(w.clone());
        }
    }
    let total = c.fetch_one(pool).await.unwrap_or(0);
    let rows = l.bind(q.limit.max(1)).bind(q.offset.max(0)).fetch_all(pool).await.unwrap_or_else(|e| {
        tracing::error!("podcast: listing episodes failed: {e}");
        Vec::new()
    });
    (rows, total)
}

pub async fn find_episode(pool: &Db, id: &str) -> Option<Episode> {
    sqlx::query_as::<_, Episode>(&format!("SELECT {EPISODE_COLS} FROM podcast_episodes WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn episode_by_slug(pool: &Db, show_id: &str, slug: &str) -> Option<Episode> {
    sqlx::query_as::<_, Episode>(&format!("SELECT {EPISODE_COLS} FROM podcast_episodes WHERE show_id = ? AND slug = ?")).bind(show_id).bind(slug).fetch_optional(pool).await.ok().flatten()
}

/// Seasons that have a live episode, in order.
pub async fn seasons(pool: &Db, show_id: &str) -> Vec<i64> {
    sqlx::query_scalar::<_, i64>("SELECT DISTINCT season FROM podcast_episodes WHERE show_id = ? AND status = 'published' AND published_at <= ? AND season > 0 ORDER BY season")
        .bind(show_id).bind(dates::now_db()).fetch_all(pool).await.unwrap_or_default()
}

/// The episode before and after this one, by publish date.
pub async fn adjacent(pool: &Db, e: &Episode) -> (Option<Episode>, Option<Episode>) {
    let at = e.published_at.clone().unwrap_or_default();
    let now = dates::now_db();
    let older = sqlx::query_as::<_, Episode>(&format!("SELECT {EPISODE_COLS} FROM podcast_episodes WHERE show_id = ? AND status = 'published' AND published_at < ? AND id <> ? ORDER BY published_at DESC LIMIT 1"))
        .bind(&e.show_id).bind(&at).bind(&e.id).fetch_optional(pool).await.ok().flatten();
    let newer = sqlx::query_as::<_, Episode>(&format!("SELECT {EPISODE_COLS} FROM podcast_episodes WHERE show_id = ? AND status = 'published' AND published_at > ? AND published_at <= ? AND id <> ? ORDER BY published_at ASC LIMIT 1"))
        .bind(&e.show_id).bind(&at).bind(&now).bind(&e.id).fetch_optional(pool).await.ok().flatten();
    (older, newer)
}

/// The next episode number for a show (highest + 1).
pub async fn next_number(pool: &Db, show_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(number), 0) FROM podcast_episodes WHERE show_id = ?").bind(show_id).fetch_one(pool).await.unwrap_or(0) + 1
}

pub async fn save_episode(pool: &Db, id: Option<&str>, i: &EpisodeInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let existing = match id {
        Some(id) => find_episode(pool, id).await,
        None => None,
    };
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO podcast_episodes (id, show_id, slug, title, guid, created_at, updated_at) VALUES (?,?,?,?,?,?,?)")
                .bind(&id).bind(&i.show_id).bind(ids::new_id()).bind(&i.title).bind(ids::uuid_v4()).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "podcast_episodes", Some(("show_id", &i.show_id)), &i.slug, &i.title, rules::RESERVED_EPISODE_SLUGS, &id).await;
    // A publish date: the one given, else the one it had, else now.
    let published_at: Option<String> = if !i.published_at.is_empty() {
        Some(i.published_at.clone())
    } else if i.status == "published" {
        Some(existing.as_ref().and_then(|e| e.published_at.clone()).unwrap_or_else(|| now.clone()))
    } else {
        existing.as_ref().and_then(|e| e.published_at.clone())
    };
    let chapters = if i.chapters.is_empty() { String::new() } else { serde_json::to_string(&i.chapters).unwrap_or_default() };
    sqlx::query(
        "UPDATE podcast_episodes SET show_id = ?, slug = ?, title = ?, summary = ?, notes = ?, notes_lexical = ?, notes_html = ?, audio_url = ?, audio_type = ?, \
         audio_bytes = ?, duration = ?, video_url = ?, artwork = ?, season = ?, number = ?, episode_type = ?, explicit = ?, access = ?, status = ?, published_at = ?, \
         transcript = ?, chapters = ?, featured = ?, seo_title = ?, seo_description = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&i.show_id).bind(&slug).bind(&i.title).bind(&i.summary).bind(&i.notes).bind(&i.notes_lexical).bind(&i.notes_html).bind(&i.audio_url)
    .bind(rules::audio_type(&i.audio_url)).bind(i.audio_bytes).bind(i.duration).bind(&i.video_url).bind(&i.artwork).bind(i.season).bind(i.number)
    .bind(&i.episode_type).bind(i64::from(i.explicit)).bind(&i.access).bind(&i.status).bind(published_at).bind(&i.transcript).bind(chapters)
    .bind(i64::from(i.featured)).bind(&i.seo_title).bind(&i.seo_description).bind(&now).bind(&id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    set_credits(pool, "episode", &id, &i.people).await?;
    sqlx::query("DELETE FROM podcast_episode_sponsors WHERE episode_id = ?").bind(&id).execute(pool).await.map_err(dberr)?;
    for s in &i.sponsors {
        sqlx::query("INSERT INTO podcast_episode_sponsors (episode_id, sponsor_id) VALUES (?, ?)").bind(&id).bind(s).execute(pool).await.map_err(dberr)?;
    }
    Ok(id)
}

/// Publish or unpublish without the editor (the list's quick action).
pub async fn set_episode_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    touch();
    let now = dates::now_db();
    if status == "published" {
        sqlx::query("UPDATE podcast_episodes SET status = 'published', published_at = COALESCE(published_at, ?), updated_at = ? WHERE id = ? AND audio_url <> ''")
            .bind(&now).bind(&now).bind(id).execute(pool).await.map_err(dberr)?;
    } else {
        sqlx::query("UPDATE podcast_episodes SET status = 'draft', updated_at = ? WHERE id = ?").bind(&now).bind(id).execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

pub async fn delete_episode(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM podcast_credits WHERE owner_kind = 'episode' AND owner_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_episode_sponsors WHERE episode_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_downloads WHERE episode_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_episodes WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// One episode from another host's feed. Its guid is kept so apps that had
/// it do not download it again; an episode already imported is skipped.
pub async fn import_episode(pool: &Db, show_id: &str, e: &rules::ImportedEpisode, notes_html: &str, notes_plain: &str) -> Result<bool> {
    let seen: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_episodes WHERE show_id = ? AND (guid = ? OR audio_url = ?)").bind(show_id).bind(&e.guid).bind(&e.audio_url).fetch_one(pool).await.unwrap_or(0);
    if seen > 0 {
        return Ok(false);
    }
    touch();
    let id = ids::new_id();
    let now = dates::now_db();
    let slug = unique_slug(pool, "podcast_episodes", Some(("show_id", show_id)), "", &e.title, rules::RESERVED_EPISODE_SLUGS, &id).await;
    let published = if e.published_at.is_empty() { now.clone() } else { e.published_at.clone() };
    sqlx::query(
        "INSERT INTO podcast_episodes (id, show_id, slug, title, summary, notes, notes_html, audio_url, audio_type, audio_bytes, duration, artwork, season, number, \
         episode_type, explicit, access, status, published_at, guid, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'public','published',?,?,?,?)",
    )
    .bind(&id).bind(show_id).bind(&slug).bind(if e.title.is_empty() { "Untitled episode" } else { e.title.as_str() }).bind(&e.summary).bind(notes_plain).bind(notes_html)
    .bind(&e.audio_url).bind(rules::audio_type(&e.audio_url)).bind(e.audio_bytes).bind(e.duration).bind(&e.artwork).bind(e.season).bind(e.number)
    .bind(&e.episode_type).bind(i64::from(e.explicit)).bind(&published).bind(&e.guid).bind(&now).bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(true)
}

/// `(show id, live episodes, drafts, scheduled)` for every show.
pub async fn episode_counts(pool: &Db) -> HashMap<String, (i64, i64, i64)> {
    let now = dates::now_db();
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT show_id, SUM(CASE WHEN status = 'published' AND published_at <= ? THEN 1 ELSE 0 END), SUM(CASE WHEN status = 'draft' THEN 1 ELSE 0 END), \
         SUM(CASE WHEN status = 'published' AND published_at > ? THEN 1 ELSE 0 END) FROM podcast_episodes GROUP BY show_id",
    )
    .bind(&now)
    .bind(&now)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|(s, a, b, c)| (s, (a, b, c))).collect()
}

// ---------------------------------------------------------------------------
// people and credits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Person {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub title: String,
    #[sqlx(try_from = "Text")]
    pub bio: String,
    pub photo: String,
    pub website: String,
    #[sqlx(try_from = "Text")]
    pub links: String,
}

impl Person {
    pub fn initials(&self) -> String {
        self.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
    }
    pub fn links(&self) -> Vec<String> {
        self.links.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
    }
}

const PERSON_COLS: &str = "id, slug, name, title, COALESCE(bio, '') AS bio, photo, website, COALESCE(links, '') AS links";

pub async fn people(pool: &Db) -> Vec<Person> {
    sqlx::query_as::<_, Person>(&format!("SELECT {PERSON_COLS} FROM podcast_people ORDER BY name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_person(pool: &Db, id: &str) -> Option<Person> {
    sqlx::query_as::<_, Person>(&format!("SELECT {PERSON_COLS} FROM podcast_people WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn person_by_slug(pool: &Db, slug: &str) -> Option<Person> {
    sqlx::query_as::<_, Person>(&format!("SELECT {PERSON_COLS} FROM podcast_people WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_person(pool: &Db, id: Option<&str>, i: &PersonInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO podcast_people (id, slug, name, created_at, updated_at) VALUES (?,?,?,?,?)").bind(&id).bind(ids::new_id()).bind(&i.name).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "podcast_people", None, &i.slug, &i.name, &[], &id).await;
    sqlx::query("UPDATE podcast_people SET slug = ?, name = ?, title = ?, bio = ?, photo = ?, website = ?, links = ?, updated_at = ? WHERE id = ?")
        .bind(&slug).bind(&i.name).bind(&i.title).bind(&i.bio).bind(&i.photo).bind(&i.website).bind(i.links.join("\n")).bind(&now).bind(&id)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn delete_person(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM podcast_credits WHERE person_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_people WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_credits(pool: &Db, kind: &str, owner: &str, people: &[(String, String)]) -> Result<()> {
    sqlx::query("DELETE FROM podcast_credits WHERE owner_kind = ? AND owner_id = ?").bind(kind).bind(owner).execute(pool).await.map_err(dberr)?;
    for (n, (pid, role)) in people.iter().enumerate() {
        sqlx::query("INSERT INTO podcast_credits (owner_kind, owner_id, person_id, role, sort_order) VALUES (?,?,?,?,?)")
            .bind(kind).bind(owner).bind(pid).bind(role).bind(n as i64).execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

/// `(person, role)` on a show or an episode, in their order.
pub async fn credits(pool: &Db, kind: &str, owner: &str) -> Vec<(Person, String)> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT person_id, role FROM podcast_credits WHERE owner_kind = ? AND owner_id = ? ORDER BY sort_order")
        .bind(kind).bind(owner).fetch_all(pool).await.unwrap_or_default();
    let mut out = Vec::new();
    for (pid, role) in rows {
        if let Some(p) = find_person(pool, &pid).await {
            out.push((p, role));
        }
    }
    out
}

/// Every credit at once: `owner id → [(person id, role)]`, for lists.
pub async fn all_credits(pool: &Db, kind: &str) -> HashMap<String, Vec<(String, String)>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT owner_id, person_id, role FROM podcast_credits WHERE owner_kind = ? ORDER BY sort_order")
        .bind(kind).fetch_all(pool).await.unwrap_or_default();
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (o, p, r) in rows {
        out.entry(o).or_default().push((p, r));
    }
    out
}

/// The live episodes a person is credited on, newest first.
pub async fn appearances(pool: &Db, person_id: &str) -> Vec<(Episode, String)> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT owner_id, role FROM podcast_credits WHERE owner_kind = 'episode' AND person_id = ?")
        .bind(person_id).fetch_all(pool).await.unwrap_or_default();
    let mut out = Vec::new();
    for (eid, role) in rows {
        if let Some(e) = find_episode(pool, &eid).await.filter(Episode::is_live) {
            out.push((e, role));
        }
    }
    out.sort_by(|a, b| b.0.published_at.cmp(&a.0.published_at));
    out
}

// ---------------------------------------------------------------------------
// sponsors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Sponsor {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub url: String,
    pub logo: String,
    pub promo_code: String,
    pub active: i64,
}

const SPONSOR_COLS: &str = "id, name, tagline, url, logo, promo_code, active";

pub async fn sponsors(pool: &Db) -> Vec<Sponsor> {
    sqlx::query_as::<_, Sponsor>(&format!("SELECT {SPONSOR_COLS} FROM podcast_sponsors ORDER BY active DESC, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_sponsor(pool: &Db, id: &str) -> Option<Sponsor> {
    sqlx::query_as::<_, Sponsor>(&format!("SELECT {SPONSOR_COLS} FROM podcast_sponsors WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_sponsor(pool: &Db, id: Option<&str>, i: &SponsorInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE podcast_sponsors SET name = ?, tagline = ?, url = ?, logo = ?, promo_code = ?, active = ?, updated_at = ? WHERE id = ?")
                .bind(&i.name).bind(&i.tagline).bind(&i.url).bind(&i.logo).bind(&i.promo_code).bind(i64::from(i.active)).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO podcast_sponsors (id, name, tagline, url, logo, promo_code, active, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&i.name).bind(&i.tagline).bind(&i.url).bind(&i.logo).bind(&i.promo_code).bind(i64::from(i.active)).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_sponsor(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM podcast_episode_sponsors WHERE sponsor_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM podcast_sponsors WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn episode_sponsor_ids(pool: &Db, episode_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT sponsor_id FROM podcast_episode_sponsors WHERE episode_id = ?").bind(episode_id).fetch_all(pool).await.unwrap_or_default()
}

pub async fn episode_sponsors(pool: &Db, episode_id: &str) -> Vec<Sponsor> {
    sqlx::query_as::<_, Sponsor>(&format!("SELECT {SPONSOR_COLS} FROM podcast_sponsors WHERE active = 1 AND id IN (SELECT sponsor_id FROM podcast_episode_sponsors WHERE episode_id = ?) ORDER BY name"))
        .bind(episode_id).fetch_all(pool).await.unwrap_or_default()
}

/// `sponsor id → episodes it is read on`.
pub async fn sponsor_reads(pool: &Db) -> HashMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT sponsor_id, COUNT(*) FROM podcast_episode_sponsors GROUP BY sponsor_id").fetch_all(pool).await.unwrap_or_default();
    rows.into_iter().collect()
}

// ---------------------------------------------------------------------------
// downloads
// ---------------------------------------------------------------------------

pub struct Download<'a> {
    pub episode_id: &'a str,
    pub show_id: &'a str,
    pub day: &'a str,
    pub listener: &'a str,
    pub app: &'a str,
    pub country: &'a str,
    /// `feed` (an app), `web` (the site's player) or `private`.
    pub source: &'a str,
}

/// Counts a listen unless this listener already has one for the episode
/// today. Returns whether it counted.
pub async fn record_download(pool: &Db, d: &Download<'_>) -> bool {
    let inserted = sqlx::query(crate::dialect::sql(
        "INSERT OR IGNORE INTO podcast_downloads (id, episode_id, show_id, day, listener, app, country, source, created_at) VALUES (?,?,?,?,?,?,?,?,?)",
    ))
    .bind(ids::new_id()).bind(d.episode_id).bind(d.show_id).bind(d.day).bind(d.listener).bind(d.app).bind(d.country).bind(d.source).bind(dates::now_db())
    .execute(pool)
    .await
    .map(|r| r.rows_affected() > 0)
    .unwrap_or(false);
    if inserted {
        let _ = sqlx::query("UPDATE podcast_episodes SET downloads = downloads + 1 WHERE id = ?").bind(d.episode_id).execute(pool).await;
    }
    inserted
}

/// `(day, downloads)` since `since` (YYYY-MM-DD), optionally for one show.
pub async fn downloads_per_day(pool: &Db, show_id: Option<&str>, since: &str) -> Vec<(String, i64)> {
    match show_id {
        Some(s) => sqlx::query_as("SELECT day, COUNT(*) FROM podcast_downloads WHERE day >= ? AND show_id = ? GROUP BY day ORDER BY day").bind(since).bind(s).fetch_all(pool).await,
        None => sqlx::query_as("SELECT day, COUNT(*) FROM podcast_downloads WHERE day >= ? GROUP BY day ORDER BY day").bind(since).fetch_all(pool).await,
    }
    .unwrap_or_default()
}

/// `(value, downloads)` grouped by `app`, `country` or `source`, biggest first.
pub async fn downloads_by(pool: &Db, column: &str, show_id: Option<&str>, since: &str, limit: i64) -> Vec<(String, i64)> {
    let col = match column {
        "country" => "country",
        "source" => "source",
        _ => "app",
    };
    let (filter, bind) = match show_id {
        Some(s) => (" AND show_id = ?", Some(s)),
        None => ("", None),
    };
    let sql = format!("SELECT {col}, COUNT(*) AS n FROM podcast_downloads WHERE day >= ?{filter} GROUP BY {col} ORDER BY n DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, (String, i64)>(&sql).bind(since);
    if let Some(b) = bind {
        q = q.bind(b);
    }
    q.bind(limit).fetch_all(pool).await.unwrap_or_default()
}

/// `(episode id, downloads)` since `since`, biggest first.
pub async fn top_episodes(pool: &Db, show_id: Option<&str>, since: &str, limit: i64) -> Vec<(String, i64)> {
    match show_id {
        Some(s) => sqlx::query_as("SELECT episode_id, COUNT(*) AS n FROM podcast_downloads WHERE day >= ? AND show_id = ? GROUP BY episode_id ORDER BY n DESC LIMIT ?").bind(since).bind(s).bind(limit).fetch_all(pool).await,
        None => sqlx::query_as("SELECT episode_id, COUNT(*) AS n FROM podcast_downloads WHERE day >= ? GROUP BY episode_id ORDER BY n DESC LIMIT ?").bind(since).bind(limit).fetch_all(pool).await,
    }
    .unwrap_or_default()
}

pub async fn downloads_since(pool: &Db, show_id: Option<&str>, since: &str) -> i64 {
    match show_id {
        Some(s) => sqlx::query_scalar("SELECT COUNT(*) FROM podcast_downloads WHERE day >= ? AND show_id = ?").bind(since).bind(s).fetch_one(pool).await,
        None => sqlx::query_scalar("SELECT COUNT(*) FROM podcast_downloads WHERE day >= ?").bind(since).fetch_one(pool).await,
    }
    .unwrap_or(0)
}

/// Downloads of each episode in its first `days` days — the industry's way
/// of comparing episodes fairly.
pub async fn first_days(pool: &Db, episode: &Episode, days: i64) -> i64 {
    let Some(start) = episode.published_at.as_deref().and_then(dates::from_db_string) else { return 0 };
    let from = start.format("%Y-%m-%d").to_string();
    let to = (start + chrono::Duration::days(days)).format("%Y-%m-%d").to_string();
    sqlx::query_scalar("SELECT COUNT(*) FROM podcast_downloads WHERE episode_id = ? AND day >= ? AND day < ?").bind(&episode.id).bind(from).bind(to).fetch_one(pool).await.unwrap_or(0)
}

// ---------------------------------------------------------------------------
// requests and reviews
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Request {
    pub id: String,
    pub show_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub email: String,
    pub topic: String,
    #[sqlx(try_from = "Text")]
    pub message: String,
    pub link: String,
    pub status: String,
    #[sqlx(try_from = "Text")]
    pub note: String,
    pub created_at: String,
}

const REQUEST_COLS: &str = "id, show_id, kind, name, email, topic, COALESCE(message, '') AS message, link, status, COALESCE(note, '') AS note, created_at";

pub async fn insert_request(pool: &Db, show_id: Option<&str>, i: &rules::RequestInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO podcast_requests (id, show_id, kind, name, email, topic, message, link, status, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,'new',?,?)")
        .bind(&id).bind(show_id).bind(&i.kind).bind(&i.name).bind(&i.email).bind(&i.topic).bind(&i.message).bind(&i.link).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn requests(pool: &Db, status: Option<&str>, kind: Option<&str>, limit: i64) -> Vec<Request> {
    let mut sql = format!("SELECT {REQUEST_COLS} FROM podcast_requests WHERE 1 = 1");
    if status.is_some() {
        sql.push_str(" AND status = ?");
    }
    if kind.is_some() {
        sql.push_str(" AND kind = ?");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, Request>(&sql);
    if let Some(s) = status {
        q = q.bind(s);
    }
    if let Some(k) = kind {
        q = q.bind(k);
    }
    q.bind(limit).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_request(pool: &Db, id: &str) -> Option<Request> {
    sqlx::query_as::<_, Request>(&format!("SELECT {REQUEST_COLS} FROM podcast_requests WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn update_request(pool: &Db, id: &str, status: &str, note: &str) -> Result<()> {
    sqlx::query("UPDATE podcast_requests SET status = ?, note = ?, updated_at = ? WHERE id = ?").bind(status).bind(note).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_request(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM podcast_requests WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn requests_from_since(pool: &Db, email: &str, since: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM podcast_requests WHERE email = ? AND created_at >= ?").bind(email).bind(since).fetch_one(pool).await.unwrap_or(0)
}

pub async fn new_requests(pool: &Db) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM podcast_requests WHERE status = 'new'").fetch_one(pool).await.unwrap_or(0)
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Review {
    pub id: String,
    pub show_id: String,
    pub author: String,
    pub rating: i64,
    pub title: String,
    #[sqlx(try_from = "Text")]
    pub body: String,
    pub source: String,
    pub featured: i64,
    pub created_at: String,
}

const REVIEW_COLS: &str = "id, show_id, author, rating, title, COALESCE(body, '') AS body, source, featured, created_at";

pub async fn reviews(pool: &Db, show_id: Option<&str>) -> Vec<Review> {
    match show_id {
        Some(s) => sqlx::query_as::<_, Review>(&format!("SELECT {REVIEW_COLS} FROM podcast_reviews WHERE show_id = ? ORDER BY featured DESC, created_at DESC")).bind(s).fetch_all(pool).await,
        None => sqlx::query_as::<_, Review>(&format!("SELECT {REVIEW_COLS} FROM podcast_reviews ORDER BY created_at DESC")).fetch_all(pool).await,
    }
    .unwrap_or_default()
}

pub async fn save_review(pool: &Db, id: Option<&str>, show_id: &str, i: &ReviewInput) -> Result<String> {
    touch();
    match id {
        Some(id) => {
            sqlx::query("UPDATE podcast_reviews SET show_id = ?, author = ?, rating = ?, title = ?, body = ?, source = ?, featured = ? WHERE id = ?")
                .bind(show_id).bind(&i.author).bind(i.rating).bind(&i.title).bind(&i.body).bind(&i.source).bind(i64::from(i.featured)).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO podcast_reviews (id, show_id, author, rating, title, body, source, featured, created_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(show_id).bind(&i.author).bind(i.rating).bind(&i.title).bind(&i.body).bind(&i.source).bind(i64::from(i.featured)).bind(dates::now_db())
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_review(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM podcast_reviews WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// what themes are handed
// ---------------------------------------------------------------------------

pub fn show_url(s: &Settings, show: &Show) -> String {
    link(s, &format!("/podcast/{}/", show.slug))
}

pub fn feed_url(s: &Settings, show: &Show) -> String {
    link(s, &format!("/podcast/{}/feed.xml", show.slug))
}

pub fn episode_url(s: &Settings, show: &Show, e: &Episode) -> String {
    link(s, &format!("/podcast/{}/{}/", show.slug, e.slug))
}

/// Where apps and the site's player fetch the audio: through the counter.
pub fn play_url(s: &Settings, e: &Episode) -> String {
    link(s, &format!("/podcast/play/{}.{}", e.id, rules::audio_extension(&e.audio_url)))
}

pub fn person_url(s: &Settings, p: &Person) -> String {
    link(s, &format!("/podcast/people/{}/", p.slug))
}

pub fn person_json(s: &Settings, p: &Person, role: Option<&str>) -> Value {
    json!({
        "id": p.id, "slug": p.slug, "name": p.name, "title": p.title, "bio": p.bio,
        "photo": if p.photo.is_empty() { Value::Null } else { json!(p.photo) },
        "initials": p.initials(), "website": if p.website.is_empty() { Value::Null } else { json!(p.website) },
        "links": p.links(), "url": person_url(s, p),
        "role": role.map(|r| json!(r)).unwrap_or(Value::Null),
        "role_label": role.map(|r| json!(rules::Role::from_key(r).label())).unwrap_or(Value::Null),
    })
}

pub fn sponsor_json(sp: &Sponsor) -> Value {
    json!({ "name": sp.name, "tagline": sp.tagline, "url": if sp.url.is_empty() { Value::Null } else { json!(sp.url) },
        "logo": if sp.logo.is_empty() { Value::Null } else { json!(sp.logo) }, "promo_code": if sp.promo_code.is_empty() { Value::Null } else { json!(sp.promo_code) } })
}

pub fn show_json(s: &Settings, show: &Show, hosts: &[(Person, String)], live_count: i64) -> Value {
    json!({
        "id": show.id, "slug": show.slug, "title": show.title, "name": show.title, "subtitle": show.subtitle,
        "description": show.description, "description_html": vuuut_core::directory::paragraphs(&show.description),
        "artwork": if show.artwork.is_empty() { Value::Null } else { json!(show.artwork) },
        "feature_image": if show.artwork.is_empty() { Value::Null } else { json!(show.artwork) },
        "banner": if show.banner.is_empty() { Value::Null } else { json!(show.banner) },
        "author": show.author, "category": show.category, "category_label": show.category.replace(" > ", " › "),
        "language": show.language, "explicit": show.explicit != 0, "serial": show.show_type == "serial",
        "copyright": show.copyright, "tint": show.tint,
        "url": show_url(s, show), "feed_url": feed_url(s, show),
        "links": show.links().iter().map(|(k, u)| json!({ "key": k, "label": rules::platform_label(k), "url": u })).collect::<Vec<_>>(),
        "funding": if show.funding_url.is_empty() { Value::Null } else { json!({ "url": show.funding_url, "label": if show.funding_label.is_empty() { "Support the show" } else { show.funding_label.as_str() } }) },
        "hosts": hosts.iter().map(|(p, r)| person_json(s, p, Some(r))).collect::<Vec<_>>(),
        "episode_count": live_count,
        "complete": show.complete != 0,
    })
}

/// One episode as a theme sees it — `{{#get "episodes"}}` and `episode` in
/// the `podcast/*.hbs` templates. `can_listen` is decided by the caller for
/// the visitor in hand; when false, no audio address is handed out.
pub fn episode_json(s: &Settings, show: &Show, e: &Episode, people: &[(Person, String)], sponsors: &[Sponsor], can_listen: bool) -> Value {
    let date = e.published_at.as_deref().and_then(dates::from_db_string);
    json!({
        "id": e.id, "slug": e.slug, "title": e.title, "summary": e.summary, "excerpt": e.summary,
        "url": episode_url(s, show, e),
        "show": { "title": show.title, "slug": show.slug, "url": show_url(s, show), "artwork": if show.artwork.is_empty() { Value::Null } else { json!(show.artwork) } },
        "artwork": if !e.artwork.is_empty() { json!(e.artwork) } else if !show.artwork.is_empty() { json!(show.artwork) } else { Value::Null },
        "feature_image": if !e.artwork.is_empty() { json!(e.artwork) } else if !show.artwork.is_empty() { json!(show.artwork) } else { Value::Null },
        "season": e.season, "number": e.number, "label": e.label(), "type": e.episode_type,
        "trailer": e.episode_type == "trailer", "bonus": e.episode_type == "bonus",
        "explicit": e.explicit != 0, "featured": e.featured != 0,
        "access": e.access, "members_only": e.access != "public", "can_listen": can_listen,
        "audio_url": if can_listen && !e.audio_url.is_empty() { json!(play_url(s, e)) } else { Value::Null },
        "audio_type": e.audio_type,
        "duration": e.duration, "duration_label": rules::duration_label(e.duration), "clock": rules::clock(e.duration),
        "size_label": rules::size_label(e.audio_bytes),
        "published_at": date.map(|d| json!(d.to_rfc3339())).unwrap_or(Value::Null),
        "date_label": date.map(|d| json!(d.format("%-d %b %Y").to_string())).unwrap_or(Value::Null),
        "notes": if can_listen { json!(e.notes_markup()) } else { Value::Null },
        "video_embed": if can_listen { vuuut_core::lms::video_embed(&e.video_url).map(|(u, frame)| json!({ "url": u, "iframe": frame })).unwrap_or(Value::Null) } else { Value::Null },
        "chapters": if can_listen { json!(e.chapters().iter().map(|c| json!({ "start": c.start, "time": rules::clock(c.start), "title": c.title, "url": if c.url.is_empty() { Value::Null } else { json!(c.url) } })).collect::<Vec<_>>()) } else { json!([]) },
        "has_transcript": !e.transcript.trim().is_empty(),
        "transcript_url": if can_listen && !e.transcript.trim().is_empty() { json!(link(s, &format!("/podcast/{}/{}/transcript.txt", show.slug, e.slug))) } else { Value::Null },
        "people": people.iter().map(|(p, r)| person_json(s, p, Some(r))).collect::<Vec<_>>(),
        "guests": people.iter().filter(|(_, r)| r == "guest").map(|(p, r)| person_json(s, p, Some(r))).collect::<Vec<_>>(),
        "sponsors": sponsors.iter().map(sponsor_json).collect::<Vec<_>>(),
        "downloads": e.downloads,
    })
}

pub fn review_json(r: &Review) -> Value {
    json!({ "author": r.author, "rating": r.rating, "stars": "★".repeat(r.rating.clamp(0, 5) as usize), "title": r.title, "body": r.body,
        "source": rules::REVIEW_SOURCES.iter().find(|(k, _)| *k == r.source).map(|(_, l)| *l).unwrap_or("Elsewhere"), "featured": r.featured != 0 })
}
