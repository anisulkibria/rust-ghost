//! Metadata for stored files, and finding where one is used.
//!
//! The library's contents come from the filesystem, not from here — see
//! `migrations/sqlite/0012_media.sql` for why. This repo owns the one table
//! that does exist (`media_meta`, the words put beside a file) and the one
//! question about media that only the database can answer: which posts point
//! at a given URL.
//!
//! Every query binds its values. A path arriving from a request has already
//! been through [`ghost_core::media::parse_location`] before it reaches these
//! functions; the bind is what keeps it out of the SQL regardless.
use super::dberr;
use crate::Db;
use ghost_core::media::Location;
use ghost_core::{dates, Result};
use std::collections::HashMap;

/// What has been written about one file. Absent for a file nobody has
/// described, which is the ordinary case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub title: String,
    pub alt: String,
    pub caption: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Meta {
    /// Whether anything at all has been written. A row of three empty strings
    /// is indistinguishable from no row, so saving one deletes it instead.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.alt.is_empty() && self.caption.is_empty()
    }
}

/// The fields a save sets. Separate from [`Meta`] so the timestamps stay the
/// repo's business.
#[derive(Debug, Clone, Default)]
pub struct MetaInput<'a> {
    pub title: &'a str,
    pub alt: &'a str,
    pub caption: &'a str,
}

type MetaRow = (String, String, String, String, String);

const COLUMNS: &str = "title, alt, caption, created_at, updated_at";

fn to_meta((title, alt, caption, created_at, updated_at): MetaRow) -> Meta {
    Meta { title, alt, caption, created_at, updated_at }
}

/// What has been written about one file.
pub async fn find(pool: &Db, at: &Location) -> Result<Option<Meta>> {
    let row: Option<MetaRow> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM media_meta WHERE path = ?"))
        .bind(at.path())
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row.map(to_meta))
}

/// Everything that has been written about anything, keyed by stored path.
///
/// One query for the whole library rather than one per tile: the grid draws a
/// screenful of files at a time and each needs its title. The table only holds
/// rows for files somebody has actually described, so on most sites this is a
/// handful of rows — and a site that has described every one of ten thousand
/// files has ten thousand short rows, which is still one cheap read.
///
/// Rows whose file has since been deleted outside the admin are returned too
/// and simply never looked up. They are not pruned here: a GET should not
/// write, and a stale row is unreachable anyway — every upload gets a fresh
/// random tail, so nothing new lands on a departed file's path.
pub async fn all(pool: &Db) -> Result<HashMap<String, Meta>> {
    let rows: Vec<(String, String, String, String, String, String)> =
        sqlx::query_as(&format!("SELECT path, {COLUMNS} FROM media_meta"))
            .fetch_all(pool)
            .await
            .map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(path, title, alt, caption, created_at, updated_at)| {
            (path, Meta { title, alt, caption, created_at, updated_at })
        })
        .collect())
}

/// Write what has been said about a file.
///
/// An input with nothing in it deletes the row instead of storing three empty
/// strings, so "has anything been written about this file" has one answer
/// rather than two.
pub async fn save(pool: &Db, at: &Location, input: &MetaInput<'_>) -> Result<()> {
    if input.title.is_empty() && input.alt.is_empty() && input.caption.is_empty() {
        delete(pool, at).await?;
        return Ok(());
    }
    let now = dates::now_db();
    sqlx::query(ghost_core::dialect::sql(
        "INSERT INTO media_meta (path, kind, title, alt, caption, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(path) DO UPDATE SET title = excluded.title, alt = excluded.alt, \
         caption = excluded.caption, updated_at = excluded.updated_at",
    ))
    .bind(at.path())
    .bind(at.kind.as_str())
    .bind(input.title)
    .bind(input.alt)
    .bind(input.caption)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

/// Move the metadata with the file. Nothing to do when the file had none.
///
/// A row already sitting at the destination is removed first: the caller has
/// established that no *file* is there, so a row there is left over from a
/// file that was deleted, and the upsert would otherwise keep its words.
pub async fn rename(pool: &Db, from: &Location, to: &Location) -> Result<()> {
    let Some(meta) = find(pool, from).await? else { return Ok(()) };
    delete(pool, to).await?;
    save(pool, to, &MetaInput { title: &meta.title, alt: &meta.alt, caption: &meta.caption }).await?;
    delete(pool, from).await
}

/// Forget what was written about a file.
pub async fn delete(pool: &Db, at: &Location) -> Result<()> {
    sqlx::query("DELETE FROM media_meta WHERE path = ?")
        .bind(at.path())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// A post or page that points at a file.
#[derive(Debug, Clone)]
pub struct Usage {
    pub id: String,
    pub title: String,
    /// `post` or `page`, as the posts table spells it.
    pub post_type: String,
    pub status: String,
}

/// Which posts and pages reference a URL, and how many in total.
///
/// Used to warn before a delete, so removing a file does not silently empty a
/// picture out of a published post. Matched as a substring of the rendered
/// HTML and of the editor's own document, because a URL appears in both and
/// either one alone would miss cases: `html` is empty for a post never
/// rendered, and `lexical` is empty for one imported as HTML.
///
/// `limit` bounds the named rows; the count is the true total.
pub async fn usage(pool: &Db, url: &str, limit: i64) -> Result<(Vec<Usage>, i64)> {
    // Matched literally: `%` and `_` are ordinary characters in a file name
    // but wildcards in LIKE, and a file called `plan_a.pdf` must not report
    // itself as used by a post that mentions `plan-a.pdf`.
    let pattern = format!("%{}%", like_escape(url));
    // The editor stores a URL inside a JSON string, where `/` arrives escaped.
    let escaped = format!("%{}%", like_escape(&url.replace('/', "\\/")));
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, title, type, status FROM posts \
         WHERE html LIKE ? ESCAPE '~' OR lexical LIKE ? ESCAPE '~' OR lexical LIKE ? ESCAPE '~' \
            OR feature_image = ? \
         ORDER BY updated_at DESC, id DESC LIMIT ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(&escaped)
    .bind(url)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts \
         WHERE html LIKE ? ESCAPE '~' OR lexical LIKE ? ESCAPE '~' OR lexical LIKE ? ESCAPE '~' \
            OR feature_image = ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(&escaped)
    .bind(url)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    Ok((
        rows.into_iter()
            .map(|(id, title, post_type, status)| Usage {
                id,
                title: title.unwrap_or_default(),
                post_type: post_type.unwrap_or_else(|| "post".into()),
                status: status.unwrap_or_else(|| "draft".into()),
            })
            .collect(),
        total,
    ))
}

/// Defuse the wildcards in a string used as a LIKE pattern.
///
/// `~` is the escape character — chosen over the conventional `\` because
/// MySQL also reads `\` as a string escape, so a backslash would have to be
/// doubled differently on each engine. Nothing about `~` is special to either.
fn like_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '%' | '_' | '~') {
            out.push('~');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_wildcards_in_a_file_name_are_matched_literally() {
        assert_eq!(like_escape("/content/files/plan_a.pdf"), "/content/files/plan~_a.pdf");
        assert_eq!(like_escape("100%-off"), "100~%-off");
        assert_eq!(like_escape("a~b"), "a~~b");
        // a backslash is left alone: it is the escape character on neither
        // engine's LIKE once ESCAPE '~' is in force
        assert_eq!(like_escape("a\\/b"), "a\\/b");
    }

    #[test]
    fn metadata_with_nothing_in_it_counts_as_absent() {
        assert!(Meta::default().is_empty());
        assert!(!Meta { alt: "A cat".into(), ..Default::default() }.is_empty());
    }
}
