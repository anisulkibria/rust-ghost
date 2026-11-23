//! Search and Filtering: the insights log, and the tag lookup the built-in
//! engine needs.
//!
//! The posts themselves are read through `repos::posts::browse` — the same
//! call the Content API's search-index feed makes — so both engines see the
//! same set of published posts. What is here is the part `browse` does not
//! return: each post's public tags.

use super::dberr;
use crate::Db;
use vuuut_core::{dates, ids, Result};

/// `(post_id, tag name, tag slug)` for every public tag on a published post
/// or page, in the post's own tag order. Internal (`#`) tags are not public
/// and never reach search.
pub async fn post_tags(pool: &Db) -> Result<Vec<(String, String, String)>> {
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT posts_tags.post_id, tags.name, tags.slug FROM posts_tags \
         JOIN tags ON tags.id = posts_tags.tag_id \
         JOIN posts ON posts.id = posts_tags.post_id \
         WHERE tags.visibility = 'public' AND posts.status = 'published' \
         ORDER BY posts_tags.post_id, posts_tags.sort_order",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// Every public tag, by name, for the admin's exclusion picker.
pub async fn public_tags(pool: &Db) -> Result<Vec<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT name, slug FROM tags WHERE visibility = 'public' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(dberr)
}

/// One search a visitor settled on. `query` is already in
/// `vuuut_core::site_search::log_form`.
pub async fn record(pool: &Db, query: &str, results: i64) -> Result<()> {
    sqlx::query("INSERT INTO search_queries (id, query, results, created_at) VALUES (?,?,?,?)")
        .bind(ids::new_id())
        .bind(query)
        .bind(results)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// The most searched queries since a moment: `(query, searches, last result
/// count)`. `zero_only` keeps only the ones that last found nothing.
pub async fn top(pool: &Db, since: &str, zero_only: bool, limit: i64) -> Result<Vec<(String, i64, i64)>> {
    let sql = if zero_only {
        "SELECT query, COUNT(*), MAX(results) FROM search_queries WHERE created_at >= ? \
         GROUP BY query HAVING MAX(results) = 0 ORDER BY COUNT(*) DESC, query LIMIT ?"
    } else {
        "SELECT query, COUNT(*), MAX(results) FROM search_queries WHERE created_at >= ? \
         GROUP BY query ORDER BY COUNT(*) DESC, query LIMIT ?"
    };
    sqlx::query_as::<_, (String, i64, i64)>(sql)
        .bind(since)
        .bind(limit.clamp(1, 500))
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

/// `(searches, searches that found nothing)` since a moment.
pub async fn totals(pool: &Db, since: &str) -> (i64, i64) {
    let all = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM search_queries WHERE created_at >= ?")
        .bind(since)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let zero = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM search_queries WHERE created_at >= ? AND results = 0")
        .bind(since)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    (all, zero)
}

pub async fn purge_older_than(pool: &Db, days: i64) -> Result<u64> {
    let cutoff = dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days.max(1))));
    let done = sqlx::query("DELETE FROM search_queries WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(done.rows_affected())
}

pub async fn clear(pool: &Db) -> Result<u64> {
    let done = sqlx::query("DELETE FROM search_queries").execute(pool).await.map_err(dberr)?;
    Ok(done.rows_affected())
}
