//! The Admin API's comment endpoints (`/ghost/api/admin/comments/*`) and the
//! auth frame that lets the front-end widget reach them.
//!
//! Two different clients use these:
//!
//! 1. **The comments widget, acting for a signed-in moderator.** It cannot
//!    call the Admin API directly — it runs on the site origin, the Admin
//!    API is under `/ghost`, and the session cookie is scoped to that path,
//!    so a cross-path `fetch` would carry no credentials. Ghost's answer is
//!    an invisible iframe at `/ghost/auth-frame/` which *is* same-origin
//!    with the Admin API: the widget posts messages to it, the frame makes
//!    the request with the cookie attached, and posts the JSON back. That is
//!    why [`auth_frame`] exists and why it serves a script rather than data.
//!
//! 2. **The admin moderation screen**, which browses every comment on the
//!    site rather than one post's thread.
//!
//! Because the widget calls these while *impersonating* the moderator's own
//! member account (`impersonate_member_uuid`), the responses have to compute
//! `liked` for that member — the frame's request carries a staff session, not
//! a member one, so there is nobody else to compute it for.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use ghost_db::repos::comments as repo;
use ghost_db::repos::comments::BrowseComments;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::comments::{comment_json, list_params, page_json, Audience, Commenting};
use crate::AppState;

/// Mounted under `/ghost/api/admin` alongside the rest of the Admin API.
pub fn admin_comments_router() -> Router<AppState> {
    Router::new()
        // The flat, every-post list the moderation screen uses.
        .route("/comments/", get(browse_all))
        .route("/comments", get(browse_all).post(add))
        // Static segments before `{id}`, so an id can never shadow them.
        .route("/comments/post/{post_id}", get(browse_by_post))
        .route("/comments/post/{post_id}/", get(browse_by_post))
        .route("/comments/{id}/replies", get(replies))
        .route("/comments/{id}/replies/", get(replies))
        .route("/comments/{id}/reports", get(reports))
        .route("/comments/{id}/reports/", get(reports))
        .route("/comments/{id}/likes", get(likes))
        .route("/comments/{id}/likes/", get(likes))
        .route("/comments/{id}", get(read).put(edit))
        .route("/comments/{id}/", get(read).put(edit))
        // Per-member commenting suspension, which the members screen drives.
        .route("/members/{id}/commenting/disable", post(disable_commenting))
        .route("/members/{id}/commenting/disable/", post(disable_commenting))
        .route("/members/{id}/commenting/enable", post(enable_commenting))
        .route("/members/{id}/commenting/enable/", post(enable_commenting))
        // Ghost registers this as `PUT /comments/:id` too; declaring the
        // method explicitly keeps the 405 for anything else.
        .route("/comments/{id}/status", put(edit))
}

fn err(status: StatusCode, message: &str, error_type: &str) -> Response {
    (status, Json(json!({ "errors": [{ "message": message, "errorType": error_type }] }))).into_response()
}

fn ok_json(body: Value) -> Response {
    (StatusCode::OK, [(header::CACHE_CONTROL, "private, no-cache, no-store")], Json(body)).into_response()
}

fn api_error(e: &ghost_core::GhostError) -> Response {
    err(
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        &e.message(),
        e.error_type(),
    )
}

/// Only Administrators and the Owner may read or moderate comments.
///
/// Ghost grants the `comment` permission set to `Administrator`, `Admin
/// Integration` and `Super Editor` — and to nothing below. This port's role
/// model folds `Super Editor` into `Editor`, so honouring the fixture
/// literally would hand moderation to every Editor; requiring Administrator
/// is the conservative reading, and the one that cannot over-grant.
async fn require_moderator(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ghost_core::models::User, Response> {
    let Some(user) = crate::admin::current_user(state, headers).await else {
        return Err(err(StatusCode::UNAUTHORIZED, "Authentication required", "NoPermissionError"));
    };
    let role = ghost_db::repos::users::role_name(&state.db, &user.id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if matches!(role.as_str(), "Owner" | "Administrator") {
        return Ok(user);
    }
    Err(err(
        StatusCode::FORBIDDEN,
        "You do not have permission to moderate comments",
        "NoPermissionError",
    ))
}

/// Resolve `impersonate_member_uuid` to a member id.
///
/// The widget sends it so that a moderator browsing through the auth frame
/// still sees their own likes highlighted. An unknown uuid is ignored rather
/// than refused — the worst case is an unhighlighted heart.
async fn impersonated(state: &AppState, uuid: Option<&str>) -> Option<String> {
    let uuid = uuid?;
    sqlx::query_scalar("SELECT id FROM members WHERE uuid = ?")
        .bind(uuid)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// browsing
// ---------------------------------------------------------------------------

/// One post's thread, as the widget's moderator view asks for it.
async fn browse_by_post(
    State(state): State<AppState>,
    Path(post_id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let params = match list_params(&q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    let member_id = impersonated(&state, params.impersonate_member_uuid.as_deref()).await;
    let filter = ghost_nql::and(Some(crate::comments::post_id_filter(&post_id)), params.filter);

    let browse = BrowseComments {
        filter,
        order: params.order,
        limit: params.limit,
        page: params.page,
        parent_id: Some(None),
        is_admin: true,
        browse_all: false,
        member_id,
        with_replies: true,
        with_post: false,
    };
    match repo::browse(&state.db, &browse).await {
        Ok(page) => ok_json(page_json(&page, Audience::Admin, &state.site_url())),
        Err(e) => api_error(&e),
    }
}

/// Every comment on the site, flat.
///
/// `include_nested=false` restricts it to thread openers; the default
/// includes replies as their own rows, which is what a moderation queue
/// wants — a reply needs moderating whether or not its parent does.
async fn browse_all(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let params = match list_params(&q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    let include_nested = !matches!(q.get("include_nested").map(String::as_str), Some("false"));

    let browse = BrowseComments {
        filter: params.filter,
        order: Some(params.order.unwrap_or_else(|| "created_at desc".into())),
        limit: params.limit,
        page: params.page,
        parent_id: if include_nested { None } else { Some(None) },
        is_admin: true,
        browse_all: true,
        member_id: None,
        // A flat list shows each comment as its own row, so nesting the
        // replies under their parents as well would list them twice.
        with_replies: false,
        with_post: true,
    };
    match repo::browse(&state.db, &browse).await {
        Ok(page) => ok_json(page_json(&page, Audience::Admin, &state.site_url())),
        Err(e) => api_error(&e),
    }
}

async fn replies(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let params = match list_params(&q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    let member_id = impersonated(&state, params.impersonate_member_uuid.as_deref()).await;

    let browse = BrowseComments {
        filter: params.filter,
        // Ghost pins this: replies are always loaded oldest-first.
        order: Some("created_at asc".into()),
        limit: params.limit,
        page: params.page,
        parent_id: Some(Some(id)),
        is_admin: true,
        browse_all: false,
        member_id,
        with_replies: false,
        with_post: false,
    };
    match repo::browse(&state.db, &browse).await {
        Ok(page) => ok_json(page_json(&page, Audience::Admin, &state.site_url())),
        Err(e) => api_error(&e),
    }
}

async fn read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let member_id = impersonated(&state, q.get("impersonate_member_uuid").map(String::as_str)).await;
    match repo::find_with_relations(&state.db, &id, true, member_id.as_deref(), true).await {
        Ok(Some(c)) => ok_json(json!({
            "comments": [comment_json(&c, Audience::Admin, &state.site_url())]
        })),
        Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

async fn reports(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    related_members(state, headers, "comment_reports", "reports", &id, &q).await
}

async fn likes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    related_members(state, headers, "comment_likes", "likes", &id, &q).await
}

/// Who liked or reported a comment — the detail behind the counts, which is
/// what turns "3 reports" into a decision a moderator can make.
async fn related_members(
    state: AppState,
    headers: HeaderMap,
    table: &str,
    doc: &str,
    comment_id: &str,
    q: &HashMap<String, String>,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let params = match list_params(q) {
        Ok(p) => p,
        Err(e) => return api_error(&e),
    };
    match repo::members_for(&state.db, table, comment_id, params.limit, params.page).await {
        Ok((rows, total)) => {
            let items: Vec<Value> = rows
                .iter()
                .map(|(m, at)| {
                    json!({
                        "id": m.id,
                        "uuid": m.uuid,
                        "name": m.name,
                        "email": m.email,
                        "expertise": m.expertise,
                        "status": m.status,
                        "created_at": ghost_core::dates::db_to_api(Some(at)),
                    })
                })
                .collect();
            ok_json(json!({
                doc: items,
                "meta": { "pagination": ghost_core::models::pagination_json(params.page, params.limit, total) },
            }))
        }
        Err(e) => api_error(&e),
    }
}

// ---------------------------------------------------------------------------
// moderation
// ---------------------------------------------------------------------------

/// Hide or restore a comment. The only field a moderator may change.
///
/// Editing the *body* is deliberately not possible from here, in Ghost or
/// here: a comment is attributed to its author, and silently rewriting what
/// they said would make that attribution a lie. Hiding is reversible and
/// visible.
async fn edit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let status = body
        .get("comments")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(status, "published" | "hidden" | "deleted") {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "status must be one of published, hidden or deleted",
            "ValidationError",
        );
    }
    match repo::set_status(&state.db, &id, status).await {
        Ok(Some(_)) => {
            // Re-read with relations so the response is the same shape as a
            // browse row, which is what the widget swaps into its state.
            match repo::find_with_relations(&state.db, &id, true, None, true).await {
                Ok(Some(c)) => ok_json(json!({
                    "comments": [comment_json(&c, Audience::Admin, &state.site_url())]
                })),
                Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
                Err(e) => api_error(&e),
            }
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

/// Create a comment as a given member, without sending notifications.
///
/// Ghost exposes this for importing an existing comment history: it takes a
/// `member_id` rather than using a session, accepts a backdated `created_at`,
/// and sets the internal context so nobody is emailed about a comment from
/// two years ago.
async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let Some(data) = body.get("comments").and_then(|c| c.as_array()).and_then(|a| a.first()) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Comment is required", "ValidationError");
    };
    let member_id = data.get("member_id").and_then(Value::as_str).unwrap_or_default();
    if member_id.is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "member_id is required", "ValidationError");
    }
    let html = match ghost_core::comments::prepare_html(
        data.get("html").and_then(Value::as_str).unwrap_or_default(),
    ) {
        Ok(h) => h,
        Err(e) => return api_error(&e),
    };
    let parent_id = data.get("parent_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let post_id = data.get("post_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    if parent_id.is_none() && post_id.is_none() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Either post_id (for top-level comments) or parent_id (for replies) must be provided",
            "ValidationError",
        );
    }

    // A `created_at` in the future would sort an imported comment ahead of
    // everything real, so it is dropped rather than trusted.
    let created_at = data
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(ghost_core::dates::from_api_string)
        .filter(|d| *d <= chrono::Utc::now())
        .map(|d| ghost_core::dates::to_db_string(&d));

    // A reply inherits its parent's post, which is also the check that the
    // parent exists and is a thread opener.
    let (post_id, parent_id, in_reply_to_id) = match parent_id {
        Some(parent_id) => match repo::find_raw(&state.db, parent_id).await {
            Ok(Some(parent)) if parent.parent_id.is_none() => {
                let addressed = data
                    .get("in_reply_to_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                (parent.post_id.clone(), Some(parent.id), addressed)
            }
            Ok(Some(_)) => {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "Can not reply to a reply", "ValidationError")
            }
            Ok(None) => {
                return err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError")
            }
            Err(e) => return api_error(&e),
        },
        None => (post_id.unwrap_or_default().to_string(), None, None),
    };

    let created = repo::insert(
        &state.db,
        repo::NewComment {
            post_id: &post_id,
            member_id,
            parent_id: parent_id.as_deref(),
            in_reply_to_id: in_reply_to_id.as_deref(),
            html: &html,
            // an admin/import write is pre-moderated by definition: it goes
            // out visible whatever the site's review setting is
            status: "published",
            created_at,
        },
    )
    .await;
    let created = match created {
        Ok(c) => c,
        Err(e) => return api_error(&e),
    };
    match repo::find_with_relations(&state.db, &created.id, true, None, true).await {
        Ok(Some(c)) => (
            StatusCode::CREATED,
            Json(json!({ "comments": [comment_json(&c, Audience::Admin, &state.site_url())] })),
        )
            .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "Comment could not be found", "NotFoundError"),
        Err(e) => api_error(&e),
    }
}

// ---------------------------------------------------------------------------
// per-member commenting suspension
// ---------------------------------------------------------------------------

/// Suspend a member's ability to comment.
///
/// A reason is mandatory — it is shown to the member in place of the comment
/// form, so a suspension with no reason would leave them unable to comment
/// and unable to find out why. `hide_comments` additionally hides everything
/// they have already posted, which is the usual response to a spam run.
async fn disable_commenting(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or_default().trim();
    if reason.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "A reason is required when disabling commenting for a member",
            "ValidationError",
        );
    }
    if reason.chars().count() > 2000 {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Reason must be 2000 characters or less",
            "ValidationError",
        );
    }
    let expires_at = body.get("expires_at").and_then(Value::as_str).filter(|s| !s.is_empty());
    let hide_comments = body
        .get("hide_comments")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let commenting = Commenting::disabled(reason, expires_at);
    if let Err(e) = save_commenting(&state, &id, &commenting).await {
        return api_error(&e);
    }
    if hide_comments {
        if let Err(e) = repo::hide_all_by_member(&state.db, &id).await {
            return api_error(&e);
        }
    }
    ok_json(json!({ "members": [{ "id": id, "commenting": commenting.to_json(), "can_comment": false }] }))
}

async fn enable_commenting(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_moderator(&state, &headers).await {
        return r;
    }
    let commenting = Commenting::default();
    if let Err(e) = save_commenting(&state, &id, &commenting).await {
        return api_error(&e);
    }
    // Comments hidden when they were suspended stay hidden: un-hiding them
    // wholesale would also restore anything hidden for its own reasons.
    ok_json(json!({ "members": [{ "id": id, "commenting": Value::Null, "can_comment": true }] }))
}

async fn save_commenting(
    state: &AppState,
    member_id: &str,
    commenting: &Commenting,
) -> ghost_core::Result<()> {
    let res = sqlx::query("UPDATE members SET commenting = ?, updated_at = ? WHERE id = ?")
        .bind(commenting.to_storage())
        .bind(ghost_core::dates::now_db())
        .bind(member_id)
        .execute(&state.db)
        .await
        .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(ghost_core::GhostError::NotFoundError {
            message: "Unable to find member".into(),
            context: None,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// the auth frame
// ---------------------------------------------------------------------------

/// Ghost's `admin-auth.min.js`, with the site origin substituted in.
///
/// Copied from `core/frontend/public/admin-auth/admin-auth.min.js` so the
/// message protocol matches the widget's end of it exactly: an `{uid,
/// action, …}` message in, an `{uid, error, result}` message out, and both
/// sides refusing any message whose origin is not the one they expect.
///
/// The `id()` guard is load-bearing: `action.id` arrives from another window,
/// and it is interpolated into the request path, so it is checked against
/// the ObjectId shape before use.
const ADMIN_AUTH_JS: &str = r#""use strict";const adminUrl=window.location.href.replace("auth-frame/","")+"api/admin",siteOrigin="{{SITE_ORIGIN}}";function id(e){if(typeof e!="string"||!/^[a-f0-9]{24}$/i.test(e))throw new Error("Invalid identifier");return e}function qs(e){const t=new URLSearchParams(e).toString();return t?"?"+t:""}function setCommentStatus(e,t){const s=id(e);return fetch(`${adminUrl}/comments/${s}/`,{method:"PUT",body:JSON.stringify({comments:[{id:s,status:t}]}),headers:{"Content-Type":"application/json"}})}const actions={browseComments:e=>fetch(`${adminUrl}/comments/post/${id(e.postId)}/${qs(e.params)}`),getReplies:e=>fetch(`${adminUrl}/comments/${id(e.commentId)}/replies/${qs(e.params)}`),readComment:e=>fetch(`${adminUrl}/comments/${id(e.commentId)}/${qs(e.params)}`),getUser:()=>fetch(`${adminUrl}/users/me/?include=roles`),hideComment:e=>setCommentStatus(e.id,"hidden"),showComment:e=>setCommentStatus(e.id,"published")};window.addEventListener("message",async function(e){if(e.origin!==siteOrigin){console.warn("Ignored message to admin auth iframe because of mismatch in origin","expected",siteOrigin,"got",e.origin,"with data",e.data);return}let t;try{t=JSON.parse(e.data)}catch(n){console.error("Admin auth iframe failed to parse message from site origin:",e.data,n);return}function s(n,o){e.source.postMessage(JSON.stringify({uid:t.uid,error:n?n.message:null,result:o}),siteOrigin)}const i=actions[t.action];if(i)try{const o=await(await i(t)).json();s(null,o)}catch(n){s(n,null)}});"#;

/// `GET /ghost/auth-frame/` — the HTML shell that loads the script above.
///
/// With no admin session this answers 204 rather than the page, exactly as
/// Ghost does: every visitor's browser loads this iframe, and serving the
/// script to all of them would have each one make a doomed `/users/me/`
/// request on every page view.
pub async fn auth_frame(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let has_session = headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .map(|c| c.contains("ghost-admin-api-session"))
        .unwrap_or(false);
    if !has_session {
        return (StatusCode::NO_CONTENT, [(header::CACHE_CONTROL, "public, max-age=0")]).into_response();
    }
    let _ = &state;
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-cache, no-store"),
        ],
        // Ghost's `core/frontend/src/admin-auth/index.html`, which loads the
        // script from a sibling path.
        "<!DOCTYPE html>\n<html>\n    <head>\n        <script src=\"admin-auth.min.js\"></script>\n    </head>\n</html>\n",
    )
        .into_response()
}

/// `GET /ghost/auth-frame/admin-auth.min.js`.
pub async fn auth_frame_script(State(state): State<AppState>) -> Response {
    // The frame only accepts messages from, and replies to, this one origin.
    // Ghost substitutes it at build time; here it comes from the configured
    // site URL, so moving the site does not leave a stale origin baked in.
    let body = ADMIN_AUTH_JS.replace("{{SITE_ORIGIN}}", &site_origin(&state.site_url()));
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-cache, no-store"),
        ],
        body,
    )
        .into_response()
}

/// Scheme and host only. `postMessage` compares origins, not URLs, so a site
/// installed under a subdirectory must still get the bare origin here — and
/// a trailing slash would never match.
fn site_origin(site_url: &str) -> String {
    let (scheme, rest) = site_url.split_once("://").unwrap_or(("https", site_url));
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_drops_the_path_and_any_trailing_slash() {
        assert_eq!(site_origin("https://site.test"), "https://site.test");
        assert_eq!(site_origin("https://site.test/"), "https://site.test");
        assert_eq!(site_origin("https://site.test/blog"), "https://site.test");
        assert_eq!(site_origin("http://localhost:2368/blog/"), "http://localhost:2368");
        // a bare host with no scheme is assumed to be https rather than
        // producing an origin the browser will never match
        assert_eq!(site_origin("site.test"), "https://site.test");
    }

    #[test]
    fn the_auth_script_carries_the_whole_protocol() {
        let js = ADMIN_AUTH_JS.replace("{{SITE_ORIGIN}}", "https://site.test");
        // the origin is pinned on both legs: an inbound message from
        // anywhere else is ignored, and the reply is addressed rather than
        // broadcast with `*`
        assert!(js.contains(r#"siteOrigin="https://site.test""#));
        assert!(js.contains(r#"e.origin!==siteOrigin"#));
        assert!(js.contains(r#"postMessage(JSON.stringify({uid:t.uid,error:n?n.message:null,result:o}),siteOrigin)"#));
        assert!(!js.contains(r#"),"*")"#), "the reply must be addressed, not broadcast");
        // every action the widget can invoke
        for action in ["browseComments", "getReplies", "readComment", "getUser", "hideComment", "showComment"] {
            assert!(js.contains(action), "missing action {action}");
        }
        // and the id guard that stands between a cross-window message and
        // the request path
        assert!(js.contains(r#"/^[a-f0-9]{24}$/i.test(e)"#));
        assert!(!js.contains("{{SITE_ORIGIN}}"));
    }
}
