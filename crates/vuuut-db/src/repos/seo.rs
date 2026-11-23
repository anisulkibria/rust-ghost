//! The one query behind the SEO report.
//!
//! The report judges every post and page on a handful of fields, and the
//! fields come from three places — `posts`, `posts_meta` and a count over
//! `posts_tags`. Reading them separately would be one query per entry; this
//! is one query for the lot, which is what makes the dashboard a page rather
//! than a job.
//!
//! The module owns no table of its own. **The content is the authority**: a
//! report is a view of the posts as they are at the moment it is asked for,
//! so there is nothing to keep in sync, nothing to invalidate, and no way for
//! the report to disagree with the editor. That is also why there is no
//! migration for this module.
//!
//! `plaintext` is the one wide column read here, for the word count, and it
//! is why [`ENTRY_LIMIT`] exists: the audit is bounded so that a site with
//! fifty thousand posts cannot be asked to load all of them into one admin
//! response. The newest entries are the ones a writer can still act on.
use super::dberr;
use crate::types::Text;
use crate::Db;
use vuuut_core::seo::Entry;
use vuuut_core::Result;

/// The most entries one report covers, newest first.
pub const ENTRY_LIMIT: i64 = 1000;

/// Everything the report is computed from, newest first.
///
/// Scheduled and draft entries are included: the cheapest time to fix a
/// missing description is before the post goes out. Which of them count as
/// each other's duplicates is [`vuuut_core::seo::audit_all`]'s decision, not
/// this query's.
pub async fn entries(db: &Db, limit: i64) -> Result<Vec<Entry>> {
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT p.id, p.title, p.slug, p.type AS kind, p.status, p.visibility, \
                p.published_at, p.feature_image, p.canonical_url, p.custom_excerpt, p.locale, \
                p.plaintext, \
                m.meta_title, m.meta_description, m.feature_image_alt, \
                m.og_image, m.twitter_image, \
                (SELECT COUNT(*) FROM posts_tags pt WHERE pt.post_id = p.id) AS tag_count \
         FROM posts p LEFT JOIN posts_meta m ON m.post_id = p.id \
         WHERE p.type IN ('post', 'page') \
         ORDER BY COALESCE(p.published_at, p.updated_at, p.created_at) DESC \
         LIMIT ?",
    )
    .bind(limit.clamp(1, ENTRY_LIMIT))
    .fetch_all(db)
    .await
    .map_err(dberr)?;

    Ok(rows.into_iter().map(Entry::from).collect())
}

/// How many posts and pages exist in total, so the report can say when it is
/// showing only part of them rather than quietly truncating.
pub async fn total(db: &Db) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE type IN ('post', 'page')")
        .fetch_one(db)
        .await
        .map_err(dberr)
}

/// `plaintext` is `MEDIUMTEXT` on MySQL, so it is read through
/// [`Text`] rather than `String` — the Any driver cannot decode the
/// wider text types into a plain `String`.
#[derive(sqlx::FromRow)]
struct EntryRow {
    id: String,
    title: Option<String>,
    slug: Option<String>,
    kind: String,
    status: String,
    visibility: String,
    published_at: Option<String>,
    feature_image: Option<String>,
    canonical_url: Option<String>,
    custom_excerpt: Option<String>,
    locale: Option<String>,
    plaintext: Option<Text>,
    meta_title: Option<String>,
    meta_description: Option<String>,
    feature_image_alt: Option<String>,
    og_image: Option<String>,
    twitter_image: Option<String>,
    tag_count: i64,
}

impl From<EntryRow> for Entry {
    fn from(r: EntryRow) -> Self {
        Entry {
            id: r.id,
            title: r.title.unwrap_or_default(),
            slug: r.slug.unwrap_or_default(),
            kind: r.kind,
            status: r.status,
            visibility: r.visibility,
            published_at: r.published_at,
            meta_title: r.meta_title,
            meta_description: r.meta_description,
            custom_excerpt: r.custom_excerpt,
            feature_image: r.feature_image,
            feature_image_alt: r.feature_image_alt,
            og_image: r.og_image,
            twitter_image: r.twitter_image,
            canonical_url: r.canonical_url,
            // Counted here rather than stored: `reading_time` is rounded to
            // whole minutes and so cannot distinguish 40 words from 200.
            word_count: r
                .plaintext
                .map(|t| t.0.split_whitespace().count() as i64)
                .unwrap_or(0),
            tag_count: r.tag_count,
            locale: r.locale.filter(|c| !c.is_empty()),
            duplicate_title: false,
            duplicate_description: false,
        }
    }
}
