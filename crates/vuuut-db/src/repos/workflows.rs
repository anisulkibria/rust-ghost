//! Workflow & editorial management: where each post is in the pipeline,
//! the checklist ticks, the notes and the history.
//!
//! Posts are read, never written, from here — except `create_idea`, which
//! goes through `repos::posts::insert` like every other new post. MEDIUMTEXT
//! columns of `posts` (plaintext, custom_excerpt) are read through
//! `types::Text`; the workflow tables are VARCHAR-only on MySQL.

use super::dberr;
use crate::types::Text;
use crate::Db;
use vuuut_core::{dates, ids, Result};

/// One post or page as the board sees it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Entry {
    pub post_id: String,
    pub title: Option<String>,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub status: String,
    pub published_at: Option<String>,
    pub updated_at: String,
    pub stage: Option<String>,
    pub assignee_id: Option<String>,
    pub reviewer_id: Option<String>,
    pub due_on: Option<String>,
    pub priority: Option<String>,
}

const ENTRY_SELECT: &str = "SELECT posts.id AS post_id, posts.title, posts.type, posts.status, posts.published_at, \
posts.updated_at, w.stage, w.assignee_id, w.reviewer_id, w.due_on, w.priority \
FROM posts LEFT JOIN workflow_items w ON w.post_id = posts.id";

/// Every draft, every scheduled entry, and what was published since `since`.
pub async fn board(pool: &Db, published_since: &str) -> Result<Vec<Entry>> {
    sqlx::query_as::<_, Entry>(&format!(
        "{ENTRY_SELECT} WHERE posts.status IN ('draft', 'scheduled') \
         OR (posts.status IN ('published', 'sent') AND posts.published_at >= ?) \
         ORDER BY posts.updated_at DESC LIMIT 1000"
    ))
    .bind(published_since)
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn entry(pool: &Db, post_id: &str) -> Result<Option<Entry>> {
    sqlx::query_as::<_, Entry>(&format!("{ENTRY_SELECT} WHERE posts.id = ?"))
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// Entries with a due date, a schedule or a publication in `[from, to]`
/// (both `YYYY-MM-DD`), for the calendar.
pub async fn dated(pool: &Db, from: &str, to: &str) -> Result<Vec<Entry>> {
    let to_end = format!("{to} 23:59:59");
    sqlx::query_as::<_, Entry>(&format!(
        "{ENTRY_SELECT} WHERE (w.due_on >= ? AND w.due_on <= ?) \
         OR (posts.status IN ('scheduled', 'published', 'sent') AND posts.published_at >= ? AND posts.published_at <= ?) \
         LIMIT 2000"
    ))
    .bind(from)
    .bind(to)
    .bind(format!("{from} 00:00:00"))
    .bind(&to_end)
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// `(post_id, author_id)` for the given posts — who may move their own work.
pub async fn authors(pool: &Db) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT pa.post_id, pa.author_id FROM posts_authors pa JOIN posts ON posts.id = pa.post_id \
         WHERE posts.status IN ('draft', 'scheduled', 'published', 'sent')",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

pub async fn authors_of(pool: &Db, post_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT author_id FROM posts_authors WHERE post_id = ? ORDER BY sort_order")
        .bind(post_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// The workflow fields of one entry. Writes the whole row: the caller read
/// it first and changes what it means to change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemInput {
    pub stage: String,
    pub assignee_id: Option<String>,
    pub reviewer_id: Option<String>,
    pub due_on: Option<String>,
    pub priority: String,
}

/// Update the row, or create it when the post has none yet. `UPDATE` first
/// and `INSERT` only when nothing changed is the portable upsert across both
/// engines.
pub async fn save_item(pool: &Db, post_id: &str, i: &ItemInput) -> Result<()> {
    let now = dates::now_db();
    let done = sqlx::query(
        "UPDATE workflow_items SET stage = ?, assignee_id = ?, reviewer_id = ?, due_on = ?, priority = ?, updated_at = ? WHERE post_id = ?",
    )
    .bind(&i.stage)
    .bind(i.assignee_id.clone())
    .bind(i.reviewer_id.clone())
    .bind(i.due_on.clone())
    .bind(&i.priority)
    .bind(&now)
    .bind(post_id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    if done.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO workflow_items (post_id, stage, assignee_id, reviewer_id, due_on, priority, updated_at) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(post_id)
        .bind(&i.stage)
        .bind(i.assignee_id.clone())
        .bind(i.reviewer_id.clone())
        .bind(i.due_on.clone())
        .bind(&i.priority)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(dberr)?;
    }
    Ok(())
}

/// A new draft post with this title, placed in `stage`. The post goes
/// through `posts::insert`, so it is a post like any other.
pub async fn create_idea(pool: &Db, title: &str, author_id: &str, item: &ItemInput) -> Result<String> {
    let new = super::posts::NewPost {
        title: title.to_string(),
        slug: None,
        lexical: None,
        markdown: None,
        html: None,
        plaintext: None,
        custom_excerpt: None,
        feature_image: None,
        featured: false,
        post_type: "post".into(),
        status: "draft".into(),
        visibility: "public".into(),
        published_at: None,
        codeinjection_head: None,
        codeinjection_foot: None,
        custom_template: None,
        canonical_url: None,
        show_title_and_feature_image: None,
        meta_title: None,
        meta_description: None,
        og_title: None,
        og_description: None,
        og_image: None,
        twitter_title: None,
        twitter_description: None,
        twitter_image: None,
        email_subject: None,
        feature_image_alt: None,
        feature_image_caption: None,
        author_ids: vec![author_id.to_string()],
        tag_ids: vec![],
        tier_ids: vec![],
        reading_time: 0,
    };
    let post = super::posts::insert(pool, &new, author_id).await?;
    save_item(pool, &post.id, item).await?;
    Ok(post.id)
}

// ---------------------------------------------------------------------------
// checklist, notes, history
// ---------------------------------------------------------------------------

pub async fn checks(pool: &Db, post_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT check_key FROM workflow_checks WHERE post_id = ?")
        .bind(post_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// `(post_id, ticks)` across the board, for each card's progress.
pub async fn check_counts(pool: &Db) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT post_id, COUNT(*) FROM workflow_checks GROUP BY post_id")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn set_check(pool: &Db, post_id: &str, key: &str, done: bool, actor: &str) -> Result<()> {
    sqlx::query("DELETE FROM workflow_checks WHERE post_id = ? AND check_key = ?")
        .bind(post_id)
        .bind(key)
        .execute(pool)
        .await
        .map_err(dberr)?;
    if done {
        sqlx::query("INSERT INTO workflow_checks (post_id, check_key, done_by, done_at) VALUES (?,?,?,?)")
            .bind(post_id)
            .bind(key)
            .bind(actor)
            .bind(dates::now_db())
            .execute(pool)
            .await
            .map_err(dberr)?;
    }
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Note {
    pub id: String,
    pub post_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: String,
}

pub async fn notes(pool: &Db, post_id: &str) -> Vec<Note> {
    sqlx::query_as::<_, Note>("SELECT id, post_id, author_id, body, created_at FROM workflow_notes WHERE post_id = ? ORDER BY created_at, id")
        .bind(post_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn note_counts(pool: &Db) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT post_id, COUNT(*) FROM workflow_notes GROUP BY post_id")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn add_note(pool: &Db, post_id: &str, author_id: &str, body: &str) -> Result<String> {
    let id = ids::new_id();
    sqlx::query("INSERT INTO workflow_notes (id, post_id, author_id, body, created_at) VALUES (?,?,?,?,?)")
        .bind(&id)
        .bind(post_id)
        .bind(author_id)
        .bind(body)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(id)
}

/// Deletes a note, only if `author_id` wrote it (or `any` is set, for
/// editors). Returns whether one went.
pub async fn delete_note(pool: &Db, note_id: &str, author_id: &str, any: bool) -> bool {
    let done = if any {
        sqlx::query("DELETE FROM workflow_notes WHERE id = ?").bind(note_id).execute(pool).await
    } else {
        sqlx::query("DELETE FROM workflow_notes WHERE id = ? AND author_id = ?").bind(note_id).bind(author_id).execute(pool).await
    };
    matches!(done, Ok(r) if r.rows_affected() == 1)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub id: String,
    pub post_id: String,
    pub actor_id: String,
    pub kind: String,
    pub detail: String,
    pub created_at: String,
}

/// Never fails the caller: history is a record of what happened, and a lost
/// line must not undo the change it describes.
pub async fn record(pool: &Db, post_id: &str, actor_id: &str, kind: &str, detail: &str) {
    let detail: String = detail.chars().take(300).collect();
    let _ = sqlx::query("INSERT INTO workflow_events (id, post_id, actor_id, kind, detail, created_at) VALUES (?,?,?,?,?,?)")
        .bind(ids::new_id())
        .bind(post_id)
        .bind(actor_id)
        .bind(kind)
        .bind(detail)
        .bind(dates::now_db())
        .execute(pool)
        .await;
}

pub async fn events(pool: &Db, post_id: &str, limit: i64) -> Vec<Event> {
    sqlx::query_as::<_, Event>(
        "SELECT id, post_id, actor_id, kind, detail, created_at FROM workflow_events WHERE post_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(post_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// The latest events across the site, for the board's activity strip.
pub async fn recent_events(pool: &Db, limit: i64) -> Vec<Event> {
    sqlx::query_as::<_, Event>(
        "SELECT id, post_id, actor_id, kind, detail, created_at FROM workflow_events ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// facts for the automatic checks
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct FactRow {
    title: Option<String>,
    plaintext: Option<Text>,
    custom_excerpt: Option<Text>,
    feature_image: Option<Text>,
}

pub async fn facts(pool: &Db, post_id: &str) -> vuuut_core::workflows::Facts {
    let row = sqlx::query_as::<_, FactRow>("SELECT title, plaintext, custom_excerpt, feature_image FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let meta: Option<Option<String>> = sqlx::query_scalar("SELECT meta_description FROM posts_meta WHERE post_id = ?")
        .bind(post_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags WHERE post_id = ?")
        .bind(post_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let Some(r) = row else { return Default::default() };
    let some = |t: &Option<Text>| t.as_ref().is_some_and(|t| !t.0.trim().is_empty());
    vuuut_core::workflows::Facts {
        has_title: r.title.as_deref().is_some_and(|t| !t.trim().is_empty() && t != "(Untitled)"),
        words: r.plaintext.as_ref().map(|t| t.0.split_whitespace().count() as i64).unwrap_or(0),
        has_feature_image: some(&r.feature_image),
        has_excerpt: some(&r.custom_excerpt),
        has_meta_description: meta.flatten().is_some_and(|m| !m.trim().is_empty()),
        tags,
    }
}

/// Moves every entry in a stage that no longer exists to `to`, so removing
/// a stage never strands work in a column nobody can see.
pub async fn rehome(pool: &Db, gone: &[String], to: &str) -> Result<u64> {
    let mut n = 0;
    for key in gone {
        let done = sqlx::query("UPDATE workflow_items SET stage = ? WHERE stage = ?")
            .bind(to)
            .bind(key)
            .execute(pool)
            .await
            .map_err(dberr)?;
        n += done.rows_affected();
    }
    Ok(n)
}
