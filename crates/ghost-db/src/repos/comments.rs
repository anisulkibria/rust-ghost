//! Comments repository — threads, likes, reports and their counts.
//!
//! Mirrors Ghost's `Comment` bookshelf model (`core/server/models/comment.js`)
//! including the parts that are easy to get subtly wrong:
//!
//! * **Thread structure survives moderation.** A top-level comment that was
//!   hidden or deleted is still returned if it has visible replies, because
//!   dropping it would orphan them. Its body is nulled out by the serializer
//!   instead, which is what produces the "This comment has been removed"
//!   placeholder in the UI.
//! * **Public and admin requests see different sets.** Public requests never
//!   see `hidden` or `deleted`; an admin moderating from the front end sees
//!   `hidden` but never `deleted`.
//! * **`direct_replies` is not `replies`.** A reply counts towards its
//!   parent's `direct_replies` only if it is addressed at the parent rather
//!   than at a sibling (`in_reply_to_id`), so the UI can say "3 replies" on
//!   the thread and "1 reply" on the comment inside it.
//!
//! Relations are batch-loaded per page rather than per row: a page of 20
//! comments with 5 replies each is 4 queries here, against 100+ if each
//! comment loaded its own.
use ghost_core::{dates, ids, GhostError, Result};
use ghost_nql::Expr;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use super::dberr;
use super::posts::push_clause_with_binds;

const COMMENT_COLS: &str = "id, post_id, member_id, parent_id, in_reply_to_id, status, html, edited_at, pinned_at, created_at, updated_at";

/// Date keys whose filter values arrive ISO-shaped and must be normalised to
/// the DB's `YYYY-MM-DD HH:MM:SS` before they can be compared as text.
const DATE_KEYS: &[&str] = &["created_at", "updated_at", "edited_at", "pinned_at"];

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Comment {
    pub id: String,
    pub post_id: String,
    pub member_id: Option<String>,
    pub parent_id: Option<String>,
    pub in_reply_to_id: Option<String>,
    pub status: String,
    pub html: Option<String>,
    pub edited_at: Option<String>,
    pub pinned_at: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// The counts Ghost exposes under a comment's `count` object.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    pub replies: i64,
    pub direct_replies: i64,
    pub likes: i64,
    pub reports: i64,
    /// Whether the requesting member liked this comment. Ghost models it as
    /// `count.liked` and the serializer turns it into the `liked` boolean.
    pub liked: bool,
}

/// A comment's author, as the API exposes it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommentMember {
    pub id: String,
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub email: String,
    pub expertise: Option<String>,
    pub status: String,
    pub commenting: Option<String>,
    pub enable_comment_notifications: bool,
}

/// The post fields the admin moderation screen shows alongside a comment.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommentPost {
    pub id: String,
    pub uuid: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub feature_image: Option<String>,
    pub custom_excerpt: Option<String>,
    pub plaintext: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
}

/// One comment with everything the serializer needs attached.
#[derive(Debug, Clone)]
pub struct CommentWithRelations {
    pub comment: Comment,
    pub member: Option<CommentMember>,
    pub post: Option<CommentPost>,
    pub counts: Counts,
    /// The comment this one is addressed at, for `in_reply_to_snippet`.
    pub in_reply_to: Option<Comment>,
    /// The thread parent, which admin requests use for the same snippet when
    /// `in_reply_to_id` is unset.
    pub parent: Option<Comment>,
    /// Loaded for top-level comments only — a reply never has replies.
    pub replies: Vec<CommentWithRelations>,
}

#[derive(Debug)]
pub struct CommentsPage {
    pub comments: Vec<CommentWithRelations>,
    pub total: i64,
    pub limit: i64,
    pub page: i64,
}

/// What to fetch, and from whose point of view.
#[derive(Debug, Clone, Default)]
pub struct BrowseComments {
    pub filter: Option<Expr>,
    pub order: Option<String>,
    pub limit: i64,
    pub page: i64,
    /// `Some(None)` = top-level only (Ghost's `parentId: null`), `Some(Some
    /// (id))` = the replies of `id`, `None` = no constraint, which is the
    /// admin screen's flat list.
    pub parent_id: Option<Option<String>>,
    /// Admin requests see `hidden`; nobody sees `deleted`.
    pub is_admin: bool,
    /// Skip the keep-deleted-parents-with-replies rule. Ghost sets this for
    /// its flat moderation list, where there is no thread to preserve.
    pub browse_all: bool,
    /// Whose `liked` flag to compute.
    pub member_id: Option<String>,
    /// Load each comment's replies and the post it belongs to.
    pub with_replies: bool,
    pub with_post: bool,
}

/// Statuses hidden from a given audience.
fn excluded(is_admin: bool) -> &'static [&'static str] {
    if is_admin {
        &["deleted"]
    } else {
        &["hidden", "deleted"]
    }
}

/// Ghost's `applyCustomQuery`: filter by status, but keep a non-visible
/// top-level comment that still has visible replies under it.
fn status_clause(is_admin: bool, browse_all: bool) -> String {
    let list = excluded(is_admin).iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    if browse_all {
        return format!("comments.status NOT IN ({list})");
    }
    format!(
        "(comments.status NOT IN ({list}) OR EXISTS (SELECT 1 FROM comments replies \
         WHERE replies.parent_id = comments.id AND replies.status NOT IN ({list})))"
    )
}

/// Ghost's `orderAttributes` for comments, plus the two count columns the
/// sorting dropdown offers. An unrecognised column is dropped rather than
/// interpolated — this string comes straight off the query string.
fn order_clause(order: Option<&str>) -> String {
    let default = "comments.created_at DESC, comments.id DESC".to_string();
    let Some(order) = order else { return default };
    let mut parts = Vec::new();
    for part in order.split(',') {
        let mut tokens = part.split_whitespace();
        let Some(col) = tokens.next() else { continue };
        let dir = if tokens.next().unwrap_or("desc").eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" };
        let expr = match col {
            "created_at" | "updated_at" | "edited_at" | "pinned_at" | "id" | "status" => {
                format!("comments.{col}")
            }
            "count__likes" | "count.likes" => {
                "(SELECT COUNT(*) FROM comment_likes cl WHERE cl.comment_id = comments.id)".into()
            }
            "count__reports" | "count.reports" => {
                "(SELECT COUNT(*) FROM comment_reports cr WHERE cr.comment_id = comments.id)".into()
            }
            _ => continue,
        };
        parts.push(format!("{expr} {dir}"));
    }
    if parts.is_empty() {
        return default;
    }
    // A stable tiebreak matters more here than elsewhere: comments-ui pages
    // by offset, and two comments with equal like counts swapping places
    // between pages would show one twice and hide the other.
    parts.push("comments.id DESC".into());
    parts.join(", ")
}

/// Shared WHERE construction for the page query and its COUNT(*) twin.
fn push_where(qb: &mut QueryBuilder<'_, Sqlite>, q: &BrowseComments) -> Result<()> {
    qb.push(" WHERE ");
    qb.push(status_clause(q.is_admin, q.browse_all));

    match &q.parent_id {
        Some(None) => {
            qb.push(" AND comments.parent_id IS NULL");
        }
        Some(Some(id)) => {
            qb.push(" AND comments.parent_id = ");
            qb.push_bind(id.clone());
        }
        None => {}
    }

    if let Some(filter) = &q.filter {
        let mut filter = filter.clone();
        ghost_nql::normalize_date_values(&mut filter, DATE_KEYS);
        let sql = ghost_nql::compile(&filter, "comments")?;
        qb.push(" AND (");
        push_clause_with_binds(qb, &sql.clause, &sql.binds);
        qb.push(")");
    }
    Ok(())
}

pub async fn browse(pool: &SqlitePool, q: &BrowseComments) -> Result<CommentsPage> {
    let limit = if q.limit > 0 { q.limit } else { 15 };
    let page = q.page.max(1);

    let mut qb = QueryBuilder::<Sqlite>::new("SELECT ");
    qb.push(COMMENT_COLS);
    qb.push(" FROM comments");
    push_where(&mut qb, q)?;
    qb.push(" ORDER BY ");
    qb.push(order_clause(q.order.as_deref()));
    if q.limit >= 0 {
        qb.push(" LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind((page - 1) * limit);
    }
    let rows: Vec<Comment> = qb.build_query_as::<Comment>().fetch_all(pool).await.map_err(dberr)?;

    let mut cqb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM comments");
    push_where(&mut cqb, q)?;
    let total: i64 = cqb.build_query_scalar().fetch_one(pool).await.map_err(dberr)?;

    let comments = hydrate(pool, rows, q).await?;
    Ok(CommentsPage { comments, total, limit, page })
}

/// Attach members, counts, reply threads and (optionally) posts to a page of
/// bare comment rows.
async fn hydrate(
    pool: &SqlitePool,
    rows: Vec<Comment>,
    q: &BrowseComments,
) -> Result<Vec<CommentWithRelations>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows.iter().map(|c| c.id.clone()).collect();

    let counts = load_counts(pool, &ids, q.is_admin, q.member_id.as_deref()).await?;
    let members = load_members(pool, rows.iter().filter_map(|c| c.member_id.clone()).collect()).await?;
    let referenced: Vec<String> = rows
        .iter()
        .filter_map(|c| c.in_reply_to_id.clone())
        .chain(rows.iter().filter_map(|c| c.parent_id.clone()))
        .collect();
    let referenced = load_by_ids(pool, referenced).await?;

    let posts = if q.with_post {
        load_posts(pool, rows.iter().map(|c| c.post_id.clone()).collect()).await?
    } else {
        Vec::new()
    };

    // One query for every reply of every comment on this page, rather than
    // one query per comment.
    let mut reply_rows: Vec<Comment> = Vec::new();
    if q.with_replies {
        let mut rqb = QueryBuilder::<Sqlite>::new("SELECT ");
        rqb.push(COMMENT_COLS);
        rqb.push(" FROM comments WHERE status NOT IN (");
        let mut sep = rqb.separated(", ");
        for s in excluded(q.is_admin) {
            sep.push_bind(s.to_string());
        }
        rqb.push(") AND parent_id IN (");
        let mut sep = rqb.separated(", ");
        for id in &ids {
            sep.push_bind(id.clone());
        }
        rqb.push(") ORDER BY created_at ASC, id ASC");
        reply_rows = rqb.build_query_as::<Comment>().fetch_all(pool).await.map_err(dberr)?;
    }

    let reply_ids: Vec<String> = reply_rows.iter().map(|c| c.id.clone()).collect();
    let reply_counts = load_counts(pool, &reply_ids, q.is_admin, q.member_id.as_deref()).await?;
    let reply_members =
        load_members(pool, reply_rows.iter().filter_map(|c| c.member_id.clone()).collect()).await?;
    let reply_referenced =
        load_by_ids(pool, reply_rows.iter().filter_map(|c| c.in_reply_to_id.clone()).collect()).await?;

    let find_member = |id: &Option<String>, pool: &[CommentMember]| -> Option<CommentMember> {
        id.as_ref().and_then(|id| pool.iter().find(|m| m.id == *id).cloned())
    };
    let find_ref = |id: &Option<String>, pool: &[Comment]| -> Option<Comment> {
        id.as_ref().and_then(|id| pool.iter().find(|c| c.id == *id).cloned())
    };

    let built_replies: Vec<CommentWithRelations> = reply_rows
        .into_iter()
        .map(|c| CommentWithRelations {
            member: find_member(&c.member_id, &reply_members),
            post: None,
            counts: counts_for(&reply_counts, &c.id),
            in_reply_to: find_ref(&c.in_reply_to_id, &reply_referenced),
            parent: None,
            replies: Vec::new(),
            comment: c,
        })
        .collect();

    Ok(rows
        .into_iter()
        .map(|c| {
            let replies = built_replies
                .iter()
                .filter(|r| r.comment.parent_id.as_deref() == Some(c.id.as_str()))
                .cloned()
                .collect();
            CommentWithRelations {
                member: find_member(&c.member_id, &members),
                post: posts.iter().find(|p| p.id == c.post_id).cloned(),
                counts: counts_for(&counts, &c.id),
                in_reply_to: find_ref(&c.in_reply_to_id, &referenced),
                parent: find_ref(&c.parent_id, &referenced),
                replies,
                comment: c,
            }
        })
        .collect())
}

fn counts_for(counts: &[(String, Counts)], id: &str) -> Counts {
    counts.iter().find(|(cid, _)| cid == id).map(|(_, c)| *c).unwrap_or_default()
}

/// All five counts for a batch of comments, in one query.
///
/// `direct_replies` is split into two subqueries rather than one with an
/// `OR`, the way Ghost does it: an `OR` across `parent_id` and
/// `in_reply_to_id` cannot use either index.
async fn load_counts(
    pool: &SqlitePool,
    ids: &[String],
    is_admin: bool,
    member_id: Option<&str>,
) -> Result<Vec<(String, Counts)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let list = excluded(is_admin).iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    let mut qb = QueryBuilder::<Sqlite>::new(format!(
        "SELECT c.id, \
         (SELECT COUNT(*) FROM comments r WHERE r.parent_id = c.id AND r.status NOT IN ({list})) AS replies, \
         (SELECT COUNT(*) FROM comments r1 WHERE r1.parent_id = c.id AND r1.in_reply_to_id IS NULL AND r1.status NOT IN ({list})) \
         + (SELECT COUNT(*) FROM comments r2 WHERE r2.in_reply_to_id = c.id AND r2.status NOT IN ({list})) AS direct_replies, \
         (SELECT COUNT(*) FROM comment_likes cl WHERE cl.comment_id = c.id) AS likes, \
         (SELECT COUNT(*) FROM comment_reports cr WHERE cr.comment_id = c.id) AS reports, \
         (SELECT COUNT(*) FROM comment_likes cl2 WHERE cl2.comment_id = c.id AND cl2.member_id = "
    ));
    // A signed-out visitor gets a literal 0, matching Ghost's `count.liked`
    // fallback — not a NULL that would have to be interpreted downstream.
    match member_id {
        Some(id) => qb.push_bind(id.to_string()),
        None => qb.push_bind(String::new()),
    };
    qb.push(") AS liked FROM comments c WHERE c.id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(id.clone());
    }
    qb.push(")");

    let rows: Vec<(String, i64, i64, i64, i64, i64)> =
        qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;
    Ok(rows
        .into_iter()
        .map(|(id, replies, direct_replies, likes, reports, liked)| {
            (id, Counts { replies, direct_replies, likes, reports, liked: liked > 0 })
        })
        .collect())
}

async fn load_members(pool: &SqlitePool, mut ids: Vec<String>) -> Result<Vec<CommentMember>> {
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT id, uuid, name, email, expertise, status, commenting, enable_comment_notifications \
         FROM members WHERE id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id.clone());
    }
    qb.push(")");
    qb.build_query_as().fetch_all(pool).await.map_err(dberr)
}

async fn load_posts(pool: &SqlitePool, mut ids: Vec<String>) -> Result<Vec<CommentPost>> {
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT id, uuid, title, slug, feature_image, custom_excerpt, plaintext, status, visibility \
         FROM posts WHERE id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id.clone());
    }
    qb.push(")");
    qb.build_query_as().fetch_all(pool).await.map_err(dberr)
}

async fn load_by_ids(pool: &SqlitePool, mut ids: Vec<String>) -> Result<Vec<Comment>> {
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT ");
    qb.push(COMMENT_COLS);
    qb.push(" FROM comments WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &ids {
        sep.push_bind(id.clone());
    }
    qb.push(")");
    qb.build_query_as().fetch_all(pool).await.map_err(dberr)
}

// ---------------------------------------------------------------------------
// single reads
// ---------------------------------------------------------------------------

/// The bare row, with no relations — for the ownership and status checks the
/// write paths make before doing anything.
pub async fn find_raw(pool: &SqlitePool, id: &str) -> Result<Option<Comment>> {
    sqlx::query_as::<_, Comment>(&format!("SELECT {COMMENT_COLS} FROM comments WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)
}

/// One comment with the relations a single-comment response carries.
///
/// Status filtering is deliberately *not* applied: the caller decides, since
/// `read` must 404 a hidden comment for a visitor while the write paths need
/// to find their own hidden comment in order to edit it.
pub async fn find_with_relations(
    pool: &SqlitePool,
    id: &str,
    is_admin: bool,
    member_id: Option<&str>,
    with_replies: bool,
) -> Result<Option<CommentWithRelations>> {
    let Some(row) = find_raw(pool, id).await? else {
        return Ok(None);
    };
    let q = BrowseComments {
        is_admin,
        member_id: member_id.map(str::to_string),
        with_replies: with_replies && row.parent_id.is_none(),
        ..Default::default()
    };
    Ok(hydrate(pool, vec![row], &q).await?.into_iter().next())
}

// ---------------------------------------------------------------------------
// writes
// ---------------------------------------------------------------------------

pub struct NewComment<'a> {
    pub post_id: &'a str,
    pub member_id: &'a str,
    pub parent_id: Option<&'a str>,
    pub in_reply_to_id: Option<&'a str>,
    /// Already sanitised by `ghost_core::comments::prepare_html`.
    pub html: &'a str,
    /// Set only by the admin API's importer, which backdates comments.
    pub created_at: Option<String>,
}

pub async fn insert(pool: &SqlitePool, new: NewComment<'_>) -> Result<Comment> {
    let id = ids::new_id();
    let now = dates::now_db();
    let created_at = new.created_at.unwrap_or_else(|| now.clone());
    sqlx::query(
        "INSERT INTO comments (id, post_id, member_id, parent_id, in_reply_to_id, status, html, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'published', ?, ?, ?)",
    )
    .bind(&id)
    .bind(new.post_id)
    .bind(new.member_id)
    .bind(new.parent_id)
    .bind(new.in_reply_to_id)
    .bind(new.html)
    .bind(&created_at)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;

    find_raw(pool, &id)
        .await?
        .ok_or_else(|| GhostError::Internal("comment vanished after insert".into()))
}

/// Replace a comment's body and stamp `edited_at`, which is what makes the
/// UI show an "edited" marker.
pub async fn update_html(pool: &SqlitePool, id: &str, html: &str) -> Result<Option<Comment>> {
    let now = dates::now_db();
    sqlx::query("UPDATE comments SET html = ?, edited_at = ?, updated_at = ? WHERE id = ?")
        .bind(html)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_raw(pool, id).await
}

/// Moderation and deletion are both a status change — Ghost never removes a
/// comment row, so replies under a deleted comment keep their thread.
pub async fn set_status(pool: &SqlitePool, id: &str, status: &str) -> Result<Option<Comment>> {
    sqlx::query("UPDATE comments SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    find_raw(pool, id).await
}

/// Hide every published comment by one member, for the "hide existing
/// comments" option when their commenting is disabled.
pub async fn hide_all_by_member(pool: &SqlitePool, member_id: &str) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE comments SET status = 'hidden', updated_at = ? WHERE member_id = ? AND status = 'published'",
    )
    .bind(dates::now_db())
    .bind(member_id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(res.rows_affected())
}

// ---------------------------------------------------------------------------
// likes and reports
// ---------------------------------------------------------------------------

pub async fn has_liked(pool: &SqlitePool, comment_id: &str, member_id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM comment_likes WHERE comment_id = ? AND member_id = ?",
    )
    .bind(comment_id)
    .bind(member_id)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    Ok(n > 0)
}

pub async fn like(pool: &SqlitePool, comment_id: &str, member_id: &str) -> Result<()> {
    sqlx::query("INSERT INTO comment_likes (id, comment_id, member_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(comment_id)
        .bind(member_id)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

/// Returns false when there was no like to remove, which the API turns into
/// Ghost's "Unable to find like" 404.
pub async fn unlike(pool: &SqlitePool, comment_id: &str, member_id: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM comment_likes WHERE comment_id = ? AND member_id = ?")
        .bind(comment_id)
        .bind(member_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(res.rows_affected() > 0)
}

/// Record a report. Returns false if this member had already reported this
/// comment, so the caller can skip sending the owner a second email.
pub async fn report(pool: &SqlitePool, comment_id: &str, member_id: &str) -> Result<bool> {
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM comment_reports WHERE comment_id = ? AND member_id = ?",
    )
    .bind(comment_id)
    .bind(member_id)
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    if existing > 0 {
        return Ok(false);
    }
    sqlx::query("INSERT INTO comment_reports (id, comment_id, member_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(comment_id)
        .bind(member_id)
        .bind(dates::now_db())
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(true)
}

/// A member row joined to the `created_at` of the like or report that
/// brought them back — the moderation screens show both.
type MemberWithTimestamp = (
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    bool,
    String,
);

/// The members who liked or reported a comment, for the admin screens.
pub async fn members_for(
    pool: &SqlitePool,
    table: &str,
    comment_id: &str,
    limit: i64,
    page: i64,
) -> Result<(Vec<(CommentMember, String)>, i64)> {
    // `table` is chosen by the route, never by the request.
    let table = match table {
        "comment_likes" => "comment_likes",
        "comment_reports" => "comment_reports",
        other => return Err(GhostError::BadRequest(format!("unknown relation {other}"))),
    };
    let limit = if limit > 0 { limit } else { 15 };
    let page = page.max(1);
    let rows: Vec<MemberWithTimestamp> =
        sqlx::query_as(&format!(
            "SELECT m.id, m.uuid, m.name, m.email, m.expertise, m.status, m.commenting, \
             m.enable_comment_notifications, x.created_at \
             FROM {table} x JOIN members m ON m.id = x.member_id \
             WHERE x.comment_id = ? ORDER BY x.created_at DESC LIMIT ? OFFSET ?"
        ))
        .bind(comment_id)
        .bind(limit)
        .bind((page - 1) * limit)
        .fetch_all(pool)
        .await
        .map_err(dberr)?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE comment_id = ?"))
        .bind(comment_id)
        .fetch_one(pool)
        .await
        .map_err(dberr)?;
    Ok((
        rows.into_iter()
            .map(|(id, uuid, name, email, expertise, status, commenting, notify, at)| {
                (
                    CommentMember {
                        id,
                        uuid,
                        name,
                        email,
                        expertise,
                        status,
                        commenting,
                        enable_comment_notifications: notify,
                    },
                    at,
                )
            })
            .collect(),
        total,
    ))
}

// ---------------------------------------------------------------------------
// counts, for the comment-count script and the theme helper
// ---------------------------------------------------------------------------

/// Published comment counts for the given posts. Posts with none are
/// reported as 0 rather than omitted, which is what the counts script needs
/// to render its "empty" text.
pub async fn counts_by_post(pool: &SqlitePool, post_ids: &[String]) -> Result<Vec<(String, i64)>> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT post_id, COUNT(*) FROM comments WHERE status = 'published' AND post_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in post_ids {
        sep.push_bind(id.clone());
    }
    qb.push(") GROUP BY post_id");
    let rows: Vec<(String, i64)> = qb.build_query_as().fetch_all(pool).await.map_err(dberr)?;
    Ok(post_ids
        .iter()
        .map(|id| (id.clone(), rows.iter().find(|(p, _)| p == id).map(|(_, n)| *n).unwrap_or(0)))
        .collect())
}

pub async fn all_counts(pool: &SqlitePool) -> Result<Vec<(String, i64)>> {
    sqlx::query_as("SELECT post_id, COUNT(*) FROM comments WHERE status = 'published' GROUP BY post_id")
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

/// Totals for the admin moderation screen's header.
pub async fn moderation_totals(pool: &SqlitePool) -> Result<(i64, i64, i64)> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM comments WHERE status = 'published'), \
         (SELECT COUNT(*) FROM comments WHERE status = 'hidden'), \
         (SELECT COUNT(*) FROM comments c WHERE c.status <> 'deleted' \
          AND EXISTS (SELECT 1 FROM comment_reports r WHERE r.comment_id = c.id))",
    )
    .fetch_one(pool)
    .await
    .map_err(dberr)?;
    Ok(row)
}

/// Bump `last_seen_at` / `last_commented_at` the way Ghost's
/// `LastSeenAtUpdater` does on a `MemberCommentEvent`.
pub async fn touch_member_commented(pool: &SqlitePool, member_id: &str) -> Result<()> {
    let now = dates::now_db();
    sqlx::query("UPDATE members SET last_seen_at = ?, last_commented_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&now)
        .bind(member_id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_requests_hide_more_than_admin_ones() {
        assert!(status_clause(false, true).contains("'hidden'"));
        assert!(!status_clause(true, true).contains("'hidden'"));
        assert!(status_clause(true, true).contains("'deleted'"));
    }

    #[test]
    fn thread_structure_is_preserved_unless_browsing_flat() {
        assert!(status_clause(false, false).contains("replies.parent_id = comments.id"));
        assert!(!status_clause(false, true).contains("replies.parent_id"));
    }

    #[test]
    fn order_rejects_unknown_columns() {
        assert_eq!(order_clause(None), "comments.created_at DESC, comments.id DESC");
        assert_eq!(order_clause(Some("created_at asc")), "comments.created_at ASC, comments.id DESC");
        // the sort the comments UI asks for by default
        let best = order_clause(Some("count__likes desc, created_at desc"));
        assert!(best.starts_with("(SELECT COUNT(*) FROM comment_likes"));
        assert!(best.contains("comments.created_at DESC"));
        // an injection attempt contributes nothing and falls back to default
        assert_eq!(
            order_clause(Some("id; DROP TABLE comments")),
            "comments.created_at DESC, comments.id DESC"
        );
    }
}
