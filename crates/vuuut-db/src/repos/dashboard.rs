//! The queries behind the admin home screen.
//!
//! The dashboard owns no table. **The rest of the site is the authority**: a
//! home screen is a view of the posts, members, comments and queues as they
//! are at the moment it is asked for, so there is nothing to keep in sync,
//! nothing to invalidate, and no way for it to disagree with the screen it
//! links to. That is also why this module has no migration — the same argument
//! [`super::seo`] makes.
//!
//! Three reads serve the whole page:
//!
//! 1. [`snapshot`] — every count on the screen, as one query of scalar
//!    subqueries. A dashboard that issued a query per figure would be a job
//!    rather than a page, and this one has eleven figures on it.
//! 2. [`work_queue`] — the drafts and scheduled posts to pick up, optionally
//!    narrowed to one author.
//! 3. [`recent_published`] and [`views_for_paths`] — what went out lately and
//!    how it has been read.
//!
//! None of them reads a wide column. `posts.plaintext`, `html` and `lexical`
//! are `MEDIUMTEXT` on MySQL and cannot be decoded into a `String` through the
//! `Any` driver at all, and the home screen has no use for a post's body — so
//! the select lists here are narrow on purpose, not by accident.
//!
//! Every instant a query compares against is computed in Rust and bound.
//! SQLite would take `datetime('now', ?)`; MySQL's `INTERVAL` is syntax rather
//! than a value, so the whole project computes these in one place instead. See
//! [`vuuut_core::dialect`].
use super::dberr;
use crate::Db;
use vuuut_core::Result;

/// Every count the home screen shows, in one round trip.
///
/// Read as a struct rather than a tuple of eleven integers because nothing
/// catches a transposed pair in a tuple that wide — `(drafts, scheduled)`
/// swapped compiles, runs, and quietly lies on the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct Snapshot {
    /// Posts in `draft`.
    pub drafts: i64,
    /// Posts in `scheduled`, including any that are overdue.
    pub scheduled: i64,
    /// Scheduled posts whose publish time has already passed. Non-zero means
    /// the scheduler is not running.
    pub overdue_scheduled: i64,
    /// Posts that are live — `published` or `sent`, since an emailed post is
    /// published too.
    pub published: i64,
    /// Published pages.
    pub pages: i64,
    /// Members, all statuses.
    pub members: i64,
    /// Members on a paid plan.
    pub paid_members: i64,
    /// Members who signed up inside the window `since` names.
    pub new_members: i64,
    /// Members the site already had when that window opened.
    ///
    /// Counted directly rather than as `members - new_members`: a member who
    /// signed up inside the window and was then deleted is in neither figure,
    /// and subtracting would report the window as quieter than it was.
    pub members_before: i64,
    /// Paid tiers a visitor can actually reach: active **and** visible.
    ///
    /// The visibility test is what makes this mean "for sale". A fresh install
    /// is seeded with an active paid tier called "Default Product" at
    /// `visibility = 'none'` — it exists so there is something to configure,
    /// not because the site sells anything. Counting it would tell every new
    /// site to go and connect Stripe.
    pub paid_tiers: i64,
    /// Comments a reader reported that are still standing.
    pub reported_comments: i64,
    /// Form submissions still unread.
    pub new_responses: i64,
    /// Newsletter sends that ended in failure.
    pub failed_emails: i64,
}

/// Count everything the home screen reports.
///
/// `now` bounds "overdue" and `since` bounds "new members"; both are
/// `'YYYY-MM-DD HH:MM:SS'` UTC strings, computed by the caller.
///
/// One statement, so one failure mode: if the read fails the screen shows a
/// blank snapshot rather than a page of half-filled figures silently mixing a
/// fresh count with a missing one.
pub async fn snapshot(db: &Db, now: &str, since: &str) -> Result<Snapshot> {
    sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM posts WHERE type = 'post' AND status = 'draft') AS drafts, \
         (SELECT COUNT(*) FROM posts WHERE type = 'post' AND status = 'scheduled') AS scheduled, \
         (SELECT COUNT(*) FROM posts WHERE type = 'post' AND status = 'scheduled' \
           AND published_at IS NOT NULL AND published_at < ?) AS overdue_scheduled, \
         (SELECT COUNT(*) FROM posts WHERE type = 'post' AND status IN ('published', 'sent')) AS published, \
         (SELECT COUNT(*) FROM posts WHERE type = 'page' AND status = 'published') AS pages, \
         (SELECT COUNT(*) FROM members) AS members, \
         (SELECT COUNT(*) FROM members WHERE status = 'paid') AS paid_members, \
         (SELECT COUNT(*) FROM members WHERE created_at >= ?) AS new_members, \
         (SELECT COUNT(*) FROM members WHERE created_at < ?) AS members_before, \
         (SELECT COUNT(*) FROM products WHERE type = 'paid' AND active = 1 \
           AND visibility <> 'none') AS paid_tiers, \
         (SELECT COUNT(*) FROM comments c WHERE c.status <> 'deleted' \
           AND EXISTS (SELECT 1 FROM comment_reports r WHERE r.comment_id = c.id)) AS reported_comments, \
         (SELECT COUNT(*) FROM form_submissions WHERE status = 'new') AS new_responses, \
         (SELECT COUNT(*) FROM emails WHERE status = 'failed') AS failed_emails",
    )
    .bind(now)
    .bind(since)
    .bind(since)
    .fetch_one(db)
    .await
    .map_err(dberr)
}

/// Monthly recurring revenue in minor units, from the live subscriptions.
///
/// Its own query rather than a twelfth column on [`Snapshot`], for a reason
/// worth stating: `subscriptions.mrr` is a `REAL`, and `SUM` over it can come
/// back as either an integer or a float depending on the engine and on whether
/// any rows matched. Folded into the snapshot, one undecodable value would
/// zero all eleven counts beside it. Alone, a failure costs only this figure.
///
/// Forward-looking, like Ghost's own: a cancelled subscription stops counting
/// the moment it lapses.
pub async fn mrr_minor(db: &Db) -> i64 {
    let total: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(mrr), 0.0) FROM subscriptions WHERE status IN ('active', 'trialing')",
    )
    .fetch_one(db)
    .await
    .unwrap_or(0.0);
    total.round() as i64
}

/// One post or page, as the home screen lists it.
///
/// Narrow on purpose — see the module docs on `MEDIUMTEXT`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Entry {
    pub id: String,
    pub title: Option<String>,
    pub slug: String,
    /// `post` or `page`.
    pub kind: String,
    /// `draft`, `scheduled`, `published` or `sent`.
    pub status: String,
    pub updated_at: Option<String>,
    pub published_at: Option<String>,
}

impl Entry {
    /// Where this entry is served on the site, as `page_views.path` records it.
    ///
    /// Pages and posts share the one top-level namespace in this project's
    /// routing, which is why the path does not depend on `kind`.
    pub fn path(&self) -> String {
        format!("/{}/", self.slug)
    }
}

/// The unfinished work: drafts and scheduled posts, most recently touched
/// first.
///
/// `author_id` narrows the list to entries somebody is an author of, and is
/// how an Author or Contributor sees their own work rather than everybody's —
/// the same constraint `posts::browse_admin` applies to the posts list, for
/// the same reason. `None` is every entry, for an Editor and above.
///
/// Scheduled entries come before drafts regardless of when they were touched:
/// one has a deadline and the other does not. Within each group the order is
/// the useful one — the soonest to go out, then the most recently edited.
pub async fn work_queue(db: &Db, author_id: Option<&str>, limit: i64) -> Result<Vec<Entry>> {
    let mine = match author_id {
        Some(_) => " AND EXISTS (SELECT 1 FROM posts_authors pa WHERE pa.post_id = p.id AND pa.author_id = ?)",
        None => "",
    };
    let sql = format!(
        "SELECT p.id, p.title, p.slug, p.type AS kind, p.status, p.updated_at, p.published_at \
         FROM posts p \
         WHERE p.status IN ('draft', 'scheduled') AND p.type IN ('post', 'page'){mine} \
         ORDER BY CASE p.status WHEN 'scheduled' THEN 0 ELSE 1 END, \
                  CASE p.status WHEN 'scheduled' THEN p.published_at ELSE NULL END ASC, \
                  COALESCE(p.updated_at, p.created_at) DESC \
         LIMIT ?"
    );
    let mut query = sqlx::query_as(&sql);
    if let Some(id) = author_id {
        query = query.bind(id);
    }
    query.bind(limit.clamp(1, 50)).fetch_all(db).await.map_err(dberr)
}

/// What went out most recently, newest first.
///
/// `author_id` narrows it the same way [`work_queue`] does, so a writer's home
/// screen reports on their own work.
pub async fn recent_published(db: &Db, author_id: Option<&str>, limit: i64) -> Result<Vec<Entry>> {
    let mine = match author_id {
        Some(_) => " AND EXISTS (SELECT 1 FROM posts_authors pa WHERE pa.post_id = p.id AND pa.author_id = ?)",
        None => "",
    };
    let sql = format!(
        "SELECT p.id, p.title, p.slug, p.type AS kind, p.status, p.updated_at, p.published_at \
         FROM posts p \
         WHERE p.status IN ('published', 'sent') AND p.type = 'post'{mine} \
         ORDER BY p.published_at DESC LIMIT ?"
    );
    let mut query = sqlx::query_as(&sql);
    if let Some(id) = author_id {
        query = query.bind(id);
    }
    query.bind(limit.clamp(1, 50)).fetch_all(db).await.map_err(dberr)
}

/// Views and unique visitors per path, for a known set of paths.
///
/// `path IN (?, …)` with one bind per path rather than a join on a built-up
/// string: `'/' || slug || '/'` is SQLite concatenation and means *logical or*
/// on MySQL, where it would silently match nothing. The paths are computed in
/// Rust by [`Entry::path`] and bound as data.
///
/// Returns `(path, visitors, views)`. A path nobody visited is absent rather
/// than zero — the caller knows which paths it asked about.
pub async fn views_for_paths(db: &Db, paths: &[String], since: &str) -> Vec<(String, i64, i64)> {
    if paths.is_empty() {
        return Vec::new();
    }
    let placeholders = std::iter::repeat_n("?", paths.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT path, COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE created_at >= ? AND path IN ({placeholders}) GROUP BY path"
    );
    let mut query = sqlx::query_as(&sql).bind(since);
    for path in paths {
        query = query.bind(path);
    }
    // A reporting figure, never a correctness one: a failed read costs the
    // view counts beside the titles, and the titles themselves still render.
    query.fetch_all(db).await.unwrap_or_default()
}

/// Unique visitors per day since `since`, as `(day, visitors)`, ascending.
///
/// Days with no visits are absent; the caller fills the gaps, because only it
/// knows how wide its chart is. Distinct viewers rather than raw hits, so this
/// and the "unique visitors" figure beside it measure the same thing.
pub async fn daily_visitors(db: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(DISTINCT viewer) FROM page_views \
         WHERE created_at >= ? GROUP BY day ORDER BY day",
    )
    .bind(since)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}
