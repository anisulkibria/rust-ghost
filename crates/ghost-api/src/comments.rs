//! The comments system: the members-facing API, the serializer both APIs
//! share, and the service rules between them.
//!
//! ## Why the routes look like this
//!
//! Ghost's comments widget (`comments-ui`, loaded from the CDN by the
//! `{{comments}}` theme helper) hardcodes every path it calls against the
//! site origin — `{siteUrl}/members/api/comments/...`. So the shapes here are
//! a contract with a script this server does not ship, not a design choice:
//!
//! | widget call | route |
//! |---|---|
//! | counts for the post | `GET /members/api/comments/counts/?ids=…` |
//! | the thread | `GET /members/api/comments/post/{post_id}/` |
//! | more replies | `GET /members/api/comments/{id}/replies/` |
//! | one comment (deep link) | `GET /members/api/comments/{id}/` |
//! | post / edit / delete | `POST /`, `PUT /{id}/`, `PUT /{id}/` with `status: deleted` |
//! | like / unlike / report | `POST`/`DELETE /{id}/like/`, `POST /{id}/report/` |
//!
//! Deletion is a `PUT` with `status: "deleted"`, not a `DELETE`: Ghost
//! answers `DELETE /{id}/` with 405 "You cannot destroy comments", because a
//! comment row is never removed — the thread under it would be orphaned.
//!
//! ## What the serializer is careful about
//!
//! A comment that is hidden or deleted still appears in the thread when it
//! has visible replies, but its `html` is nulled out. The reply it was
//! addressed at is summarised in `in_reply_to_snippet`, which becomes the
//! literal string `[removed]` when that comment is no longer published —
//! otherwise moderating a comment would leave its text visible in the quote
//! lines of every reply to it.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ghost_core::comments as rules;
use ghost_db::repos::comments as repo;
use ghost_db::repos::comments::{BrowseComments, CommentMember, CommentWithRelations};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::members::Member;
use crate::AppState;

/// Ghost's `maxLimitCap` for the members API. A request asking for more is
/// clamped rather than refused.
const MAX_LIMIT: i64 = 100;
/// `limit=all` has no meaning on a public endpoint that anyone can call.
const DEFAULT_LIMIT: i64 = 15;

// ---------------------------------------------------------------------------
// per-member commenting suspension
// ---------------------------------------------------------------------------

/// A moderator's suspension of one member's ability to comment, stored as
/// JSON in `members.commenting`.
///
/// Ghost's codec fails *open*: malformed JSON is read as "not suspended",
/// because a corrupt column should not silently gag a member with no way to
/// tell why.
#[derive(Debug, Clone, Default)]
pub struct Commenting {
    pub disabled: bool,
    pub disabled_reason: Option<String>,
    /// When set, the suspension lapses at this time on its own.
    pub disabled_until: Option<String>,
}

impl Commenting {
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(v) = raw.and_then(|r| serde_json::from_str::<Value>(r).ok()) else {
            return Self::default();
        };
        if !v.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            return Self::default();
        }
        // a suspension with no reason is not a valid suspension
        let Some(reason) = v.get("disabledReason").and_then(Value::as_str) else {
            return Self::default();
        };
        let until = v.get("disabledUntil").and_then(Value::as_str).map(str::to_string);
        if let Some(u) = &until {
            if ghost_core::dates::from_api_string(u).is_none() {
                return Self::default();
            }
        }
        Self { disabled: true, disabled_reason: Some(reason.to_string()), disabled_until: until }
    }

    pub fn disabled(reason: &str, until: Option<&str>) -> Self {
        Self {
            disabled: true,
            disabled_reason: Some(reason.to_string()),
            disabled_until: until.map(str::to_string),
        }
    }

    pub fn to_storage(&self) -> String {
        json!({
            "disabled": self.disabled,
            "disabledReason": self.disabled_reason,
            "disabledUntil": self.disabled_until,
        })
        .to_string()
    }

    pub fn to_json(&self) -> Value {
        if !self.disabled {
            return Value::Null;
        }
        json!({
            "disabled": true,
            "disabled_reason": self.disabled_reason,
            "disabled_until": self.disabled_until,
        })
    }

    /// A suspension with an expiry in the past no longer applies.
    pub fn can_comment(&self) -> bool {
        if !self.disabled {
            return true;
        }
        match self.disabled_until.as_deref().and_then(ghost_core::dates::from_api_string) {
            Some(until) => until <= chrono::Utc::now(),
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// router
// ---------------------------------------------------------------------------

/// Paths are absolute rather than nested, for the same reason the rest of
/// the members API is: the site router owns a `/{*path}` catch-all that a
/// nested router's own `/` route would lose to.
///
/// Each route is declared with and without its trailing slash. The widget
/// always sends the slash; a hand-written client or a proxy that normalises
/// paths may not.
pub fn comments_router() -> Router<AppState> {
    let mut router = Router::new();
    // `counts` before `{id}`, so the literal segment wins.
    for (path, method) in [
        ("counts", get(counts)),
        ("post/{post_id}", get(browse_by_post)),
        ("{id}/replies", get(replies)),
        ("{id}/like", post(like).delete(unlike)),
        ("{id}/dislike", post(dislike).delete(undislike)),
        ("{id}/report", post(report)),
        ("{id}", get(read).put(edit).delete(destroy)),
        ("", get(browse).post(add)),
    ] {
        let base = format!("/members/api/comments/{path}");
        let trimmed = base.trim_end_matches('/').to_string();
        router = router.route(&format!("{trimmed}/"), method.clone()).route(&trimmed, method);
    }
    router
}

// ---------------------------------------------------------------------------
// responses
// ---------------------------------------------------------------------------

/// Every comments response is scoped to one visitor's session (the `liked`
/// flag, and whether hidden comments are visible), so none of it may be
/// cached by a shared proxy.
fn private_json(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "private, no-cache, no-store")], Json(body)).into_response()
}

fn no_content() -> Response {
    (StatusCode::NO_CONTENT, [(header::CACHE_CONTROL, "private, no-cache, no-store")]).into_response()
}

/// Ghost's error envelope. The widget surfaces `errors[0].message` verbatim
/// in a toast, so these strings are visitor-facing.
fn api_error(e: &ghost_core::GhostError) -> Response {
    private_json(
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        json!({ "errors": [{
            "message": e.message(),
            "type": e.error_type(),
            "errorType": e.error_type(),
            "context": e.context(),
        }] }),
    )
}

fn err(status: StatusCode, message: &str, error_type: &str) -> Response {
    private_json(
        status,
        json!({ "errors": [{ "message": message, "type": error_type, "errorType": error_type }] }),
    )
}

// ---------------------------------------------------------------------------
// service rules
// ---------------------------------------------------------------------------

/// Settings → Comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enabled {
    Off,
    All,
    Paid,
}

pub fn enabled(state: &AppState) -> Enabled {
    match state.settings.get_str("comments_enabled").unwrap_or_default().as_str() {
        "all" => Enabled::All,
        "paid" => Enabled::Paid,
        _ => Enabled::Off,
    }
}

/// With comments off, every endpoint except the admin moderation ones is
/// closed — Ghost answers 405, not 404, because the resource exists and the
/// method is refused by configuration.
fn check_enabled(state: &AppState) -> ghost_core::Result<()> {
    if enabled(state) == Enabled::Off {
        return Err(ghost_core::GhostError::NoPermissionError {
            message: "Comments are not enabled for this site.".into(),
            context: None,
        });
    }
    Ok(())
}

/// Whether this member clears the site-wide commenting level. Separate from
/// the post's own access check, which `check_post_access` does.
fn check_comment_access(state: &AppState, m: &Member) -> ghost_core::Result<()> {
    if enabled(state) == Enabled::Paid && m.status == "free" {
        return Err(ghost_core::GhostError::NoPermissionError {
            message: "You do not have permission to comment on this post.".into(),
            context: None,
        });
    }
    Ok(())
}

/// A member may only comment on a post they are allowed to read.
async fn check_post_access(state: &AppState, post_id: &str, m: &Member) -> ghost_core::Result<()> {
    let visibility: Option<String> = sqlx::query_scalar("SELECT visibility FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?
        .flatten();
    let Some(visibility) = visibility else {
        return Err(ghost_core::GhostError::NotFoundError {
            message: "Post not found.".into(),
            context: None,
        });
    };
    let post_tiers = ghost_db::repos::tiers::post_tier_ids(&state.db, post_id).await?;
    let member_tiers = ghost_db::repos::tiers::member_tier_ids(&state.db, &m.id).await?;
    if rules::check_post_tier_access(&visibility, Some(&m.status), &post_tiers, &member_tiers) {
        return Ok(());
    }
    Err(ghost_core::GhostError::NoPermissionError {
        message: "You do not have permission to comment on this post.".into(),
        context: None,
    })
}

/// The gate Ghost applies ahead of every *write*: a suspended member can
/// still read and still report, but cannot post, edit, delete or like.
fn check_member_commenting(m: &Member) -> ghost_core::Result<()> {
    if m.commenting.can_comment() {
        return Ok(());
    }
    Err(ghost_core::GhostError::NoPermissionError {
        message: "Your commenting ability has been disabled.".into(),
        context: m.commenting.disabled_reason.clone(),
    })
}

// ---------------------------------------------------------------------------
// serialization
// ---------------------------------------------------------------------------

/// Which API is asking. The two differ in more than field visibility: an
/// admin request sees hidden comments, member email addresses and report
/// counts, and falls back to the thread parent for the reply snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    Public,
    Admin,
}

/// Ghost's comment output mapper.
pub fn comment_json(c: &CommentWithRelations, audience: Audience, site_url: &str) -> Value {
    let public = audience == Audience::Public;
    let mut o = Map::new();
    o.insert("id".into(), json!(c.comment.id));
    o.insert("parent_id".into(), json!(c.comment.parent_id));
    o.insert("in_reply_to_id".into(), json!(c.comment.in_reply_to_id));
    o.insert("in_reply_to_snippet".into(), reply_snippet(c, public));
    o.insert("status".into(), json!(c.comment.status));

    // A hidden comment's body is withheld from visitors, and a deleted one's
    // from everybody — including the admin screens, since the commenter
    // chose to retract it.
    let withhold = (public && c.comment.status != "published") || c.comment.status == "deleted";
    o.insert(
        "html".into(),
        if withhold { Value::Null } else { json!(c.comment.html) },
    );
    o.insert("created_at".into(), json!(ghost_core::dates::db_to_api(Some(&c.comment.created_at))));
    o.insert("edited_at".into(), json!(ghost_core::dates::db_to_api(c.comment.edited_at.as_deref())));

    o.insert("member".into(), match &c.member {
        Some(m) => member_json(m, public),
        None => Value::Null,
    });

    if !c.replies.is_empty() || c.comment.parent_id.is_none() {
        o.insert(
            "replies".into(),
            Value::Array(c.replies.iter().map(|r| comment_json(r, audience, site_url)).collect()),
        );
    }
    if let Some(parent) = &c.parent {
        // The parent is included as a bare comment: it is context for the
        // reply, not a thread of its own, so it carries no replies.
        o.insert("parent".into(), comment_json(&bare(parent), audience, site_url));
    }
    if let Some(post) = &c.post {
        o.insert("post".into(), post_json(post, site_url));
    }

    o.insert("liked".into(), json!(c.counts.liked));
    let mut counts = Map::new();
    counts.insert("replies".into(), json!(c.counts.replies));
    counts.insert("direct_replies".into(), json!(c.counts.direct_replies));
    counts.insert("likes".into(), json!(c.counts.likes));
    counts.insert("dislikes".into(), json!(c.counts.dislikes));
    if !public {
        counts.insert("reports".into(), json!(c.counts.reports));
    }
    o.insert("count".into(), Value::Object(counts));

    Value::Object(o)
}

fn bare(c: &repo::Comment) -> CommentWithRelations {
    CommentWithRelations {
        comment: c.clone(),
        member: None,
        post: None,
        counts: Default::default(),
        in_reply_to: None,
        parent: None,
        replies: Vec::new(),
    }
}

/// The one-line summary of the comment a reply is answering.
///
/// Public requests only ever get a snippet of a *published* comment; an
/// admin request additionally gets hidden ones, and falls back to the thread
/// parent for first-level replies, which have no `in_reply_to_id`. When the
/// referenced comment is not published the snippet is the literal
/// `[removed]` — a moderated comment must not stay readable through the
/// quote line of every reply to it.
fn reply_snippet(c: &CommentWithRelations, public: bool) -> Value {
    let referenced = c.in_reply_to.as_ref().or_else(|| {
        if !public && c.comment.parent_id.is_some() && c.comment.in_reply_to_id.is_none() {
            c.parent.as_ref()
        } else {
            None
        }
    });
    let Some(referenced) = referenced else {
        return Value::Null;
    };
    let visible = referenced.status == "published" || (!public && referenced.status == "hidden");
    if visible {
        json!(rules::snippet(referenced.html.as_deref().unwrap_or_default()))
    } else {
        json!("[removed]")
    }
}

fn member_json(m: &CommentMember, public: bool) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(m.id));
    o.insert("uuid".into(), json!(m.uuid));
    o.insert("name".into(), json!(m.name));
    if !public {
        o.insert("email".into(), json!(m.email));
    }
    o.insert("expertise".into(), json!(m.expertise));
    // Ghost serves a Gravatar here, which this port does not resolve; the
    // widget falls back to a generated initial bubble.
    o.insert("avatar_image".into(), Value::Null);
    if !public {
        let commenting = Commenting::parse(m.commenting.as_deref());
        o.insert("can_comment".into(), json!(commenting.can_comment()));
        o.insert("commenting".into(), commenting.to_json());
    }
    Value::Object(o)
}

fn post_json(p: &repo::CommentPost, site_url: &str) -> Value {
    let url = format!("{}/{}/", site_url.trim_end_matches('/'), p.slug.clone().unwrap_or_default());
    let excerpt = p.custom_excerpt.clone().or_else(|| {
        p.plaintext.as_deref().map(|t| t.chars().take(500).collect::<String>())
    });
    json!({
        "id": p.id,
        "uuid": p.uuid,
        "title": p.title,
        "url": url,
        "feature_image": p.feature_image.as_deref().map(ghost_core::url::base_asset),
        "excerpt": excerpt,
    })
}

/// The `{comments: [...], meta: {pagination}}` envelope both APIs return.
pub fn page_json(page: &repo::CommentsPage, audience: Audience, site_url: &str) -> Value {
    json!({
        "comments": page.comments.iter().map(|c| comment_json(c, audience, site_url)).collect::<Vec<_>>(),
        "meta": { "pagination": ghost_core::models::pagination_json(page.page, page.limit, page.total) },
    })
}

// ---------------------------------------------------------------------------
// request parsing
// ---------------------------------------------------------------------------

/// The query parameters both APIs accept.
pub struct ListParams {
    pub filter: Option<ghost_nql::Expr>,
    pub order: Option<String>,
    pub limit: i64,
    pub page: i64,
    /// Admin-only: act as this member, so `liked` is computed for them.
    pub impersonate_member_uuid: Option<String>,
}

pub fn list_params(q: &HashMap<String, String>) -> ghost_core::Result<ListParams> {
    let filter = match q.get("filter").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(f) => Some(ghost_nql::parse(f)?),
        None => None,
    };
    let limit = q
        .get("limit")
        .and_then(|l| l.parse::<i64>().ok())
        .map(|l| l.clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);
    Ok(ListParams {
        filter,
        order: q.get("order").cloned().filter(|o| !o.trim().is_empty()),
        limit,
        page: q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).max(1),
        impersonate_member_uuid: q.get("impersonate_member_uuid").cloned(),
    })
}

/// Pull the single comment out of Ghost's `{comments: [ … ]}` request body.
fn body_comment(body: &Value) -> Option<&Value> {
    body.get("comments").and_then(|c| c.as_array()).and_then(|a| a.first())
}

// ---------------------------------------------------------------------------
// GET /members/api/comments/counts/
// ---------------------------------------------------------------------------

/// Published comment counts, keyed by post id.
///
/// The only unauthenticated route here: it is what `comment-counts.min.js`
/// calls to fill in `{{comment_count}}` on index pages, where there is no
/// session to speak of. Ghost's `commentsCountAPI` max-age defaults to 0, so
/// this is revalidated rather than cached, but it is `public` — the answer
/// does not depend on who is asking.
async fn counts(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let rows = match q.get("ids").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(ids) => {
            let ids: Vec<String> = ids
                .split(',')
                .map(|s| s.trim().to_string())
                // the ids come from a query string; anything that is not an
                // ObjectId cannot match a row, so it is dropped rather than
                // round-tripped into the response
                .filter(|s| ghost_core::ids::is_objectid(s))
                .collect();
            repo::counts_by_post(&state.db, &ids).await
        }
        None => repo::all_counts(&state.db).await,
    };
    match rows {
        Ok(rows) => {
            let body: Map<String, Value> =
                rows.into_iter().map(|(id, n)| (id, json!(n))).collect();
            (
                StatusCode::OK,
                [(header::CACHE_CONTROL, "public, max-age=0")],
                Json(Value::Object(body)),
            )
                .into_response()
        }
        Err(e) => api_error(&e),
    }
}

// ---------------------------------------------------------------------------
// GET the thread
// ---------------------------------------------------------------------------

async fn browse_by_post(
    State(state): State<AppState>,
    Path(post_id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    browse_thread(state, Some(post_id), q, headers).await
}

async fn browse(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let post_id = q.get("post_id").cloned();
    browse_thread(state, post_id, q, headers).await
}

async fn browse_thread(
    state: AppState,
    post_id: Option<String>,
    q: HashMap<String, String>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let params = match list_params(&q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    let member = crate::members::current_member(&state, &headers).await;

    let filter = match post_id {
        Some(id) => ghost_nql::and(Some(post_id_filter(&id)), params.filter),
        None => params.filter,
    };

    let q = BrowseComments {
        filter,
        order: params.order,
        limit: params.limit,
        page: params.page,
        parent_id: Some(None),
        is_admin: false,
        browse_all: false,
        member_id: member.as_ref().map(|m| m.id.clone()),
        with_replies: true,
        with_post: false,
    };
    match repo::browse(&state.db, &q).await {
        Ok(page) => private_json(StatusCode::OK, page_json(&page, Audience::Public, &state.site_url())),
        Err(e) => api_error(&e),
    }
}

async fn replies(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let params = match list_params(&q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    let member = crate::members::current_member(&state, &headers).await;

    let q = BrowseComments {
        filter: params.filter,
        // Replies always read oldest-first: they are a conversation, and the
        // widget pages through them with `id:>'…'`.
        order: Some(params.order.unwrap_or_else(|| "created_at asc".into())),
        limit: params.limit,
        page: params.page,
        parent_id: Some(Some(id)),
        is_admin: false,
        browse_all: false,
        member_id: member.as_ref().map(|m| m.id.clone()),
        with_replies: false,
        with_post: false,
    };
    match repo::browse(&state.db, &q).await {
        Ok(page) => private_json(StatusCode::OK, page_json(&page, Audience::Public, &state.site_url())),
        Err(e) => api_error(&e),
    }
}

/// One comment by id — how a `#ghost-comments-<id>` deep link out of a
/// notification email resolves the comment it should scroll to.
async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let member = crate::members::current_member(&state, &headers).await;
    let found = repo::find_with_relations(
        &state.db,
        &id,
        false,
        member.as_ref().map(|m| m.id.as_str()),
        true,
    )
    .await;
    match found {
        // A hidden comment is not "present but empty" to a visitor — it is
        // not there at all, which is what lets the widget show its "this
        // comment is no longer available" notice.
        Ok(Some(c)) if c.comment.status != "hidden" => private_json(
            StatusCode::OK,
            json!({ "comments": [comment_json(&c, Audience::Public, &state.site_url())] }),
        ),
        Ok(_) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// `post_id:<id>` as an expression, so it can be ANDed with whatever filter
/// the caller sent rather than concatenated into their filter string.
pub fn post_id_filter(post_id: &str) -> ghost_nql::Expr {
    ghost_nql::Expr::Cond(ghost_nql::Cond {
        key: "post_id".into(),
        cmp: ghost_nql::Cmp::Eq,
        value: ghost_nql::NqlValue::Str(post_id.into()),
    })
}

// ---------------------------------------------------------------------------
// POST a comment or a reply
// ---------------------------------------------------------------------------

async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let Some(member) = crate::members::current_member(&state, &headers).await else {
        return err(StatusCode::UNAUTHORIZED, "Unable to find member", "UnauthorizedError");
    };
    for check in [check_member_commenting(&member), check_comment_access(&state, &member)] {
        if let Err(e) = check {
            return api_error(&e);
        }
    }
    let Some(data) = body_comment(&body) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Comment is required", "ValidationError");
    };
    let html = match rules::prepare_html(data.get("html").and_then(Value::as_str).unwrap_or_default()) {
        Ok(h) => h,
        Err(e) => return api_error(&e),
    };
    let parent_id = data.get("parent_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let in_reply_to_id = data.get("in_reply_to_id").and_then(Value::as_str).filter(|s| !s.is_empty());

    let created = match parent_id {
        Some(parent_id) => reply_to_comment(&state, &member, parent_id, in_reply_to_id, &html).await,
        None => {
            let post_id = data.get("post_id").and_then(Value::as_str).unwrap_or_default();
            if post_id.is_empty() {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "post_id is required", "ValidationError");
            }
            comment_on_post(&state, &member, post_id, &html).await
        }
    };
    let comment = match created {
        Ok(c) => c,
        Err(e) => return api_error(&e),
    };

    // Notifications go out after the comment is safely stored, and a failure
    // to send one must not fail the request — the comment is published either
    // way, and the commenter cannot act on an SMTP error.
    notify_new_comment(&state, &comment).await;
    let _ = repo::touch_member_commented(&state.db, &member.id).await;

    match repo::find_with_relations(&state.db, &comment.id, false, Some(&member.id), true).await {
        Ok(Some(c)) => with_invalidation(
            &comment,
            private_json(
                StatusCode::CREATED,
                json!({ "comments": [comment_json(&c, Audience::Public, &state.site_url())] }),
            ),
        ),
        Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// The status a brand-new comment lands with: `published`, unless the site
/// holds new comments for review (`comments_moderation: review`), in which
/// case `hidden` — the state the public API already withholds from visitors
/// and the Comments screen's Hidden filter queues up for a moderator.
fn new_comment_status(state: &AppState) -> &'static str {
    if state.settings.get_str("comments_moderation").as_deref() == Some("review") {
        "hidden"
    } else {
        "published"
    }
}

/// Ghost's `commentOnPost`.
pub async fn comment_on_post(
    state: &AppState,
    member: &Member,
    post_id: &str,
    html: &str,
) -> ghost_core::Result<repo::Comment> {
    check_post_access(state, post_id, member).await?;
    repo::insert(
        &state.db,
        repo::NewComment {
            post_id,
            member_id: &member.id,
            parent_id: None,
            in_reply_to_id: None,
            html,
            status: new_comment_status(state),
            created_at: None,
        },
    )
    .await
}

/// Ghost's `replyToComment`.
///
/// Threads are two levels deep by design, so replying to a reply is refused.
/// `in_reply_to_id` is how a reply addresses a *sibling* within the same
/// thread: it is kept only when it points at a published comment under the
/// same parent, because the API exposes a snippet of it and a dangling or
/// cross-thread reference would leak text from somewhere else.
pub async fn reply_to_comment(
    state: &AppState,
    member: &Member,
    parent_id: &str,
    in_reply_to_id: Option<&str>,
    html: &str,
) -> ghost_core::Result<repo::Comment> {
    let Some(parent) = repo::find_raw(&state.db, parent_id).await? else {
        return Err(ghost_core::GhostError::BadRequest("Comment could not be found".into()));
    };
    if parent.parent_id.is_some() {
        return Err(ghost_core::GhostError::BadRequest("Can not reply to a reply".into()));
    }
    check_post_access(state, &parent.post_id, member).await?;

    let mut addressed = None;
    if let Some(id) = in_reply_to_id {
        if let Some(c) = repo::find_raw(&state.db, id).await? {
            if c.status == "published" && c.parent_id.as_deref() == Some(parent.id.as_str()) {
                addressed = Some(c.id);
            }
        }
    }

    repo::insert(
        &state.db,
        repo::NewComment {
            post_id: &parent.post_id,
            member_id: &member.id,
            parent_id: Some(&parent.id),
            in_reply_to_id: addressed.as_deref(),
            html,
            status: new_comment_status(state),
            created_at: None,
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// PUT: edit, or retract
// ---------------------------------------------------------------------------

async fn edit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let Some(member) = crate::members::current_member(&state, &headers).await else {
        return err(StatusCode::UNAUTHORIZED, "Unable to find member", "UnauthorizedError");
    };
    if let Err(e) = check_member_commenting(&member) {
        return api_error(&e);
    }
    let Some(data) = body_comment(&body) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Comment is required", "ValidationError");
    };
    let Ok(Some(existing)) = repo::find_raw(&state.db, &id).await else {
        return err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError");
    };
    // Only the author may change their own comment. Moderators act through
    // the Admin API, which has its own staff session.
    if existing.member_id.as_deref() != Some(member.id.as_str()) {
        return err(
            StatusCode::FORBIDDEN,
            "You do not have permission to edit comments",
            "NoPermissionError",
        );
    }

    let wants_delete = data.get("status").and_then(Value::as_str) == Some("deleted");
    let updated = if wants_delete {
        repo::set_status(&state.db, &id, "deleted").await
    } else {
        match data.get("html").and_then(Value::as_str).filter(|h| !h.trim().is_empty()) {
            Some(html) => match rules::prepare_html(html) {
                // An edit with no body is a no-op in Ghost rather than an
                // error: the widget sends the whole comment back, and a
                // metadata-only change must not wipe the text.
                Ok(html) => repo::update_html(&state.db, &id, &html).await,
                Err(e) => return api_error(&e),
            },
            None => Ok(Some(existing)),
        }
    };

    let updated = match updated {
        Ok(Some(c)) => c,
        Ok(None) => return err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => return api_error(&e),
    };
    match repo::find_with_relations(&state.db, &id, false, Some(&member.id), true).await {
        Ok(Some(c)) => with_invalidation(
            &updated,
            private_json(
                StatusCode::OK,
                json!({ "comments": [comment_json(&c, Audience::Public, &state.site_url())] }),
            ),
        ),
        Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// Ghost refuses this outright: a comment row is never deleted, because the
/// replies beneath it would lose their thread. Retraction is a `PUT` with
/// `status: "deleted"`.
async fn destroy() -> Response {
    err(
        StatusCode::METHOD_NOT_ALLOWED,
        "You cannot destroy comments.",
        "MethodNotAllowedError",
    )
}

/// Ghost sets `X-Cache-Invalidate` on every comment write so a cache in
/// front of the site drops the thread it belongs to.
fn with_invalidation(c: &repo::Comment, mut resp: Response) -> Response {
    let mut paths = vec![format!("/api/members/comments/post/{}/", c.post_id)];
    if let Some(parent) = &c.parent_id {
        paths.push(format!("/api/members/comments/{parent}/replies/"));
    }
    if let Ok(v) = paths.join(", ").parse() {
        resp.headers_mut().insert("x-cache-invalidate", v);
    }
    resp
}

// ---------------------------------------------------------------------------
// like / unlike / report
// ---------------------------------------------------------------------------

async fn like(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    react(&state, &id, &headers, repo::Reaction::Like).await
}

async fn unlike(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    unreact(&state, &id, &headers, repo::Reaction::Like).await
}

async fn dislike(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    react(&state, &id, &headers, repo::Reaction::Dislike).await
}

async fn undislike(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    unreact(&state, &id, &headers, repo::Reaction::Dislike).await
}

/// Both directions of a reaction, since nothing about them differs but the
/// sign and the wording of the "already done that" refusal.
async fn react(state: &AppState, id: &str, headers: &HeaderMap, reaction: repo::Reaction) -> Response {
    if let Err(e) = check_enabled(state) {
        return api_error(&e);
    }
    let Some(member) = crate::members::current_member(state, headers).await else {
        return err(StatusCode::UNAUTHORIZED, "Unable to find member", "UnauthorizedError");
    };
    for check in [check_member_commenting(&member), check_comment_access(state, &member)] {
        if let Err(e) = check {
            return api_error(&e);
        }
    }
    match repo::has_reacted(&state.db, id, &member.id, reaction).await {
        Ok(true) => {
            let already = match reaction {
                repo::Reaction::Like => "This comment was liked already",
                repo::Reaction::Dislike => "This comment was disliked already",
            };
            return err(StatusCode::BAD_REQUEST, already, "BadRequestError");
        }
        Err(e) => return api_error(&e),
        Ok(false) => {}
    }
    // Reacting the other way moves the reaction rather than adding a second:
    // a member has one opinion per comment.
    if let Err(e) = repo::react(&state.db, id, &member.id, reaction).await {
        return api_error(&e);
    }
    invalidated_no_content(state, id).await
}

async fn unreact(state: &AppState, id: &str, headers: &HeaderMap, reaction: repo::Reaction) -> Response {
    if let Err(e) = check_enabled(state) {
        return api_error(&e);
    }
    let Some(member) = crate::members::current_member(state, headers).await else {
        return err(StatusCode::UNAUTHORIZED, "Unable to find member", "UnauthorizedError");
    };
    if let Err(e) = check_member_commenting(&member) {
        return api_error(&e);
    }
    let missing = match reaction {
        repo::Reaction::Like => "Unable to find like",
        repo::Reaction::Dislike => "Unable to find dislike",
    };
    match repo::unreact(&state.db, id, &member.id, reaction).await {
        Ok(true) => invalidated_no_content(state, id).await,
        Ok(false) => err(StatusCode::NOT_FOUND, missing, "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// Reporting stays open to a member whose commenting was suspended: it is a
/// moderation signal, not a contribution.
async fn report(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(e) = check_enabled(&state) {
        return api_error(&e);
    }
    let Some(member) = crate::members::current_member(&state, &headers).await else {
        return err(StatusCode::UNAUTHORIZED, "Unable to find member", "UnauthorizedError");
    };
    let Ok(Some(comment)) = repo::find_raw(&state.db, &id).await else {
        return err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError");
    };
    match repo::report(&state.db, &id, &member.id).await {
        // Already reported by this member: Ghost ignores it silently rather
        // than telling the reporter they are repeating themselves.
        Ok(false) => no_content(),
        Ok(true) => {
            notify_report(&state, &comment, &member).await;
            no_content()
        }
        Err(e) => api_error(&e),
    }
}

async fn invalidated_no_content(state: &AppState, comment_id: &str) -> Response {
    match repo::find_raw(&state.db, comment_id).await {
        Ok(Some(c)) => with_invalidation(&c, no_content()),
        _ => no_content(),
    }
}

// ---------------------------------------------------------------------------
// notifications
// ---------------------------------------------------------------------------

/// Ghost's `sendNewCommentNotifications`: the post's authors, then whoever is
/// being replied to — the thread parent and, separately, the sibling the
/// reply is addressed at.
async fn notify_new_comment(state: &AppState, comment: &repo::Comment) {
    crate::comments_mail::notify_post_authors(state, comment).await;
    if comment.parent_id.is_some() {
        crate::comments_mail::notify_reply_target(state, comment, false).await;
    }
    if comment.in_reply_to_id.is_some() {
        crate::comments_mail::notify_reply_target(state, comment, true).await;
    }
}

async fn notify_report(state: &AppState, comment: &repo::Comment, reporter: &Member) {
    crate::comments_mail::notify_report(state, comment, reporter).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commenting_parses_ghosts_storage_shape() {
        let off = Commenting::parse(Some(r#"{"disabled":false,"disabledReason":null,"disabledUntil":null}"#));
        assert!(!off.disabled && off.can_comment());

        let on = Commenting::parse(Some(r#"{"disabled":true,"disabledReason":"spam","disabledUntil":null}"#));
        assert!(on.disabled && !on.can_comment());
        assert_eq!(on.disabled_reason.as_deref(), Some("spam"));

        // an expiry in the past means the suspension has lapsed
        let lapsed = Commenting::parse(Some(
            r#"{"disabled":true,"disabledReason":"spam","disabledUntil":"2020-01-01T00:00:00.000Z"}"#,
        ));
        assert!(lapsed.disabled && lapsed.can_comment());

        let future = Commenting::parse(Some(
            r#"{"disabled":true,"disabledReason":"spam","disabledUntil":"2999-01-01T00:00:00.000Z"}"#,
        ));
        assert!(!future.can_comment());
    }

    #[test]
    fn commenting_fails_open_on_junk() {
        for junk in [None, Some(""), Some("not json"), Some(r#"{"disabled":true}"#)] {
            assert!(Commenting::parse(junk).can_comment(), "{junk:?}");
        }
    }

    #[test]
    fn commenting_round_trips_through_storage() {
        let c = Commenting::disabled("spam", Some("2999-01-01T00:00:00.000Z"));
        let back = Commenting::parse(Some(&c.to_storage()));
        assert_eq!(back.disabled_reason, c.disabled_reason);
        assert_eq!(back.disabled_until, c.disabled_until);
    }

    fn comment(status: &str, html: &str) -> repo::Comment {
        repo::Comment {
            id: "c1".into(),
            post_id: "p1".into(),
            member_id: Some("m1".into()),
            parent_id: None,
            in_reply_to_id: None,
            status: status.into(),
            html: Some(html.into()),
            edited_at: None,
            pinned_at: None,
            created_at: "2026-01-01 00:00:00".into(),
            updated_at: None,
        }
    }

    fn with(c: repo::Comment) -> CommentWithRelations {
        CommentWithRelations {
            comment: c,
            member: None,
            post: None,
            counts: Default::default(),
            in_reply_to: None,
            parent: None,
            replies: Vec::new(),
        }
    }

    #[test]
    fn withholds_bodies_the_way_ghost_does() {
        let site = "https://x.test";
        let published = with(comment("published", "<p>hi</p>"));
        assert_eq!(comment_json(&published, Audience::Public, site)["html"], json!("<p>hi</p>"));

        let hidden = with(comment("hidden", "<p>hi</p>"));
        assert_eq!(comment_json(&hidden, Audience::Public, site)["html"], Value::Null);
        // a moderator needs to read what they are moderating
        assert_eq!(comment_json(&hidden, Audience::Admin, site)["html"], json!("<p>hi</p>"));

        let deleted = with(comment("deleted", "<p>hi</p>"));
        assert_eq!(comment_json(&deleted, Audience::Public, site)["html"], Value::Null);
        assert_eq!(comment_json(&deleted, Audience::Admin, site)["html"], Value::Null);
    }

    #[test]
    fn public_responses_withhold_email_and_report_counts() {
        let member = CommentMember {
            id: "m1".into(),
            uuid: Some("u".into()),
            name: Some("Ada".into()),
            email: "ada@x.test".into(),
            expertise: None,
            status: "free".into(),
            commenting: None,
            enable_comment_notifications: true,
        };
        let mut c = with(comment("published", "<p>hi</p>"));
        c.member = Some(member);

        let public = comment_json(&c, Audience::Public, "https://x.test");
        assert!(public["member"].get("email").is_none());
        assert!(public["count"].get("reports").is_none());

        let admin = comment_json(&c, Audience::Admin, "https://x.test");
        assert_eq!(admin["member"]["email"], json!("ada@x.test"));
        assert_eq!(admin["count"]["reports"], json!(0));
        assert_eq!(admin["member"]["can_comment"], json!(true));
    }

    #[test]
    fn reply_snippets_hide_moderated_text() {
        let mut reply = with(repo::Comment {
            parent_id: Some("c0".into()),
            in_reply_to_id: Some("c9".into()),
            ..comment("published", "<p>reply</p>")
        });
        reply.in_reply_to = Some(comment("published", "<p>Original text</p>"));
        assert_eq!(
            comment_json(&reply, Audience::Public, "https://x.test")["in_reply_to_snippet"],
            json!("Original text")
        );

        reply.in_reply_to = Some(comment("hidden", "<p>Original text</p>"));
        assert_eq!(
            comment_json(&reply, Audience::Public, "https://x.test")["in_reply_to_snippet"],
            json!("[removed]")
        );
        // a moderator sees the hidden text, because they are judging it
        assert_eq!(
            comment_json(&reply, Audience::Admin, "https://x.test")["in_reply_to_snippet"],
            json!("Original text")
        );
    }

    #[test]
    fn admin_falls_back_to_the_thread_parent_for_a_snippet() {
        let mut reply = with(repo::Comment {
            parent_id: Some("c0".into()),
            in_reply_to_id: None,
            ..comment("published", "<p>reply</p>")
        });
        reply.parent = Some(comment("published", "<p>Thread opener</p>"));
        // a first-level reply has no `in_reply_to_id`, so the public API has
        // nothing to summarise
        assert_eq!(
            comment_json(&reply, Audience::Public, "https://x.test")["in_reply_to_snippet"],
            Value::Null
        );
        assert_eq!(
            comment_json(&reply, Audience::Admin, "https://x.test")["in_reply_to_snippet"],
            json!("Thread opener")
        );
    }

    #[test]
    fn limits_are_clamped_not_refused() {
        let p = |v: &str| {
            list_params(&HashMap::from([("limit".to_string(), v.to_string())])).unwrap().limit
        };
        assert_eq!(p("20"), 20);
        assert_eq!(p("5000"), MAX_LIMIT);
        assert_eq!(p("0"), 1);
        assert_eq!(p("all"), DEFAULT_LIMIT);
        assert_eq!(list_params(&HashMap::new()).unwrap().limit, DEFAULT_LIMIT);
    }

    /// Parsing a filter accepts any key; it is compiling one against the
    /// `comments` resource that rejects a key with no column behind it. Both
    /// steps run before a query is issued, so an unknown key is a 422 either
    /// way — this pins down which layer refuses it, since only the compile
    /// step knows what a comment has.
    #[test]
    fn an_unknown_filter_key_is_rejected_at_compile_time() {
        let parse = |f: &str| {
            list_params(&HashMap::from([("filter".to_string(), f.to_string())]))
                .unwrap()
                .filter
                .unwrap()
        };
        assert!(ghost_nql::compile(&parse("secret:1"), "comments").is_err());
        assert!(ghost_nql::compile(&parse("post_id:abc"), "comments").is_ok());
        assert!(ghost_nql::compile(&parse("created_at:<=2026-01-01T00:00:00.000Z"), "comments").is_ok());
        assert!(ghost_nql::compile(&parse("count.reports:>0"), "comments").is_ok());
    }

    /// A date filter value arrives ISO-shaped but the column is text in the
    /// DB's own format, so the literal has to be rewritten or the comparison
    /// silently matches the wrong rows.
    #[test]
    fn date_filter_values_are_normalised_to_db_format() {
        let mut expr = ghost_nql::parse("created_at:<=2026-01-02T03:04:05.000Z").unwrap();
        ghost_nql::normalize_date_values(&mut expr, &["created_at"]);
        let sql = ghost_nql::compile(&expr, "comments").unwrap();
        assert_eq!(sql.binds, vec![json!("2026-01-02 03:04:05")]);
    }
}
