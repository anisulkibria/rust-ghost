//! Admin API (`/ghost/api/admin/*`) — setup flow, session auth, CRUD.
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use ghost_db::repos;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::state::AppState;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        // setup + auth
        .route("/authentication/setup", get(setup_status).post(complete_setup))
        .route("/session", post(login).delete(logout))
        // posts / pages
        .route("/posts/", get(admin_browse_posts).post(admin_add_post))
        .route("/posts/{id}", get(admin_read_post).put(admin_edit_post).delete(admin_delete_post))
        .route("/pages/", get(admin_browse_pages).post(admin_add_page))
        .route("/pages/{id}", get(admin_read_post).put(admin_edit_post).delete(admin_delete_post))
        // tags
        .route("/tags/", get(admin_browse_tags).post(admin_add_tag))
        .route("/tags/{id}", get(admin_read_tag).put(admin_edit_tag).delete(admin_delete_tag))
        // users
        .route("/users/", get(admin_browse_users))
        .route("/users/{id}", get(admin_read_user).put(admin_edit_user))
        // The trailing-slash form too: the comments widget's auth frame asks
        // for `/users/me/?include=roles`, and Ghost's `prettyUrls` middleware
        // makes both spellings resolve there.
        .route("/users/{id}/", get(admin_read_user).put(admin_edit_user))
        .route("/users/password", post(change_password))
        // newsletters
        .route("/newsletters/", get(admin_browse_newsletters).post(admin_add_newsletter))
        .route("/newsletters/{id}", get(admin_read_newsletter).put(admin_edit_newsletter))
        // settings
        .route("/settings/", get(admin_browse_settings).put(admin_edit_settings))
        // integrations
        .route("/integrations/", get(admin_browse_integrations).post(admin_add_integration))
        .route("/integrations/{id}", get(admin_read_integration))
        // site info (used by tooling)
        .route("/site", get(site_info))
        .route("/site/", get(site_info))
        // Webhooks. Zapier registers and removes its own triggers through
        // these: without them a Zap cannot be set up at all, however correct
        // the delivery side is.
        .route("/webhooks", post(admin_add_webhook))
        .route("/webhooks/", post(admin_add_webhook))
        .route("/webhooks/{id}", put(admin_edit_webhook).delete(admin_delete_webhook))
        .route("/webhooks/{id}/", put(admin_edit_webhook).delete(admin_delete_webhook))
}

// ---------------------------------------------------------------------------
// error helpers
// ---------------------------------------------------------------------------

fn err(status: StatusCode, message: &str, error_type: &str) -> Response {
    (status, Json(json!({ "errors": [{ "message": message, "errorType": error_type }] }))).into_response()
}

fn from_ghost(e: ghost_core::GhostError) -> Response {
    err(
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        &e.message(),
        e.error_type(),
    )
}

fn ok_json(body: Value) -> Response {
    (StatusCode::OK, [(header::CACHE_CONTROL, "private")], Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// newsletters
// ---------------------------------------------------------------------------

/// Ghost's newsletter API shape. Every column is exposed, including the
/// design settings — the Admin API is how the editor and any external tool
/// read them, and a missing field reads as "unset" rather than as an error.
fn newsletter_json(n: &ghost_db::repos::newsletters::Newsletter) -> Value {
    // Built by insertion rather than one `json!` literal: at 44 keys the
    // macro's recursive expansion hits rustc's recursion limit.
    let mut o = Map::new();
    let mut put = |k: &str, v: Value| {
        o.insert(k.to_string(), v);
    };
    put("id", json!(n.id));
    put("uuid", json!(n.uuid));
    put("name", json!(n.name));
    put("description", json!(n.description));
    put("slug", json!(n.slug));
    put("sender_name", json!(n.sender_name));
    put("sender_email", json!(n.sender_email));
    put("sender_reply_to", json!(n.sender_reply_to));
    put("status", json!(n.status));
    put("visibility", json!(n.visibility));
    put("subscribe_on_signup", json!(n.subscribe_on_signup));
    put("sort_order", json!(n.sort_order));
    put("header_image", json!(n.header_image));
    put("show_header_icon", json!(n.show_header_icon));
    put("show_header_title", json!(n.show_header_title));
    put("show_header_name", json!(n.show_header_name));
    put("show_post_title_section", json!(n.show_post_title_section));
    put("show_excerpt", json!(n.show_excerpt));
    put("show_feature_image", json!(n.show_feature_image));
    put("title_font_category", json!(n.title_font_category));
    put("title_alignment", json!(n.title_alignment));
    put("title_font_weight", json!(n.title_font_weight));
    put("body_font_category", json!(n.body_font_category));
    put("background_color", json!(n.background_color));
    put("header_background_color", json!(n.header_background_color));
    put("post_title_color", json!(n.post_title_color));
    put("section_title_color", json!(n.section_title_color));
    put("divider_color", json!(n.divider_color));
    put("link_color", json!(n.link_color));
    put("link_style", json!(n.link_style));
    put("button_color", json!(n.button_color));
    put("button_style", json!(n.button_style));
    put("button_corners", json!(n.button_corners));
    put("image_corners", json!(n.image_corners));
    put("footer_content", json!(n.footer_content));
    put("show_badge", json!(n.show_badge));
    put("show_comment_cta", json!(n.show_comment_cta));
    put("show_subscription_details", json!(n.show_subscription_details));
    put("show_latest_posts", json!(n.show_latest_posts));
    put("show_share_button", json!(n.show_share_button));
    put("feedback_enabled", json!(n.feedback_enabled));
    put("created_at", json!(ghost_core::dates::db_to_api(n.created_at.as_deref())));
    put("updated_at", json!(ghost_core::dates::db_to_api(n.updated_at.as_deref())));
    drop(put);
    Value::Object(o)
}

async fn admin_browse_newsletters(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::newsletters::list(&state.db).await {
        Ok(list) => ok_json(json!({
            "newsletters": list.iter().map(newsletter_json).collect::<Vec<_>>(),
        })),
        Err(e) => from_ghost(e),
    }
}

async fn admin_read_newsletter(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::newsletters::find_by_id(&state.db, &id).await {
        Ok(Some(n)) => ok_json(json!({ "newsletters": [newsletter_json(&n)] })),
        Ok(None) => err(StatusCode::NOT_FOUND, "Newsletter not found", "NotFoundError"),
        Err(e) => from_ghost(e),
    }
}

async fn admin_add_newsletter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let name = body
        .get("newsletters")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|n| n.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match repos::newsletters::create(&state.db, name).await {
        Ok(n) => (
            StatusCode::CREATED,
            Json(json!({ "newsletters": [newsletter_json(&n)] })),
        )
            .into_response(),
        Err(e) => from_ghost(e),
    }
}

async fn admin_edit_newsletter(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let Some(patch) = body
        .get("newsletters")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
    else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "newsletters[0] payload required",
            "ValidationError",
        );
    };
    let mut n = match repos::newsletters::find_by_id(&state.db, &id).await {
        Ok(Some(n)) => n,
        Ok(None) => return err(StatusCode::NOT_FOUND, "Newsletter not found", "NotFoundError"),
        Err(e) => return from_ghost(e),
    };

    // A PATCH-shaped PUT: only fields the body mentions are touched, so a
    // client that knows nothing about the design settings cannot reset them.
    let text = |key: &str| patch.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let flag = |key: &str| patch.get(key).and_then(|v| v.as_bool());
    // A JSON `null` on a nullable colour clears it; an absent key leaves it.
    let nullable = |key: &str| -> Option<Option<String>> {
        patch.get(key).map(|v| match v {
            Value::Null => None,
            other => other.as_str().map(str::to_string),
        })
    };

    if let Some(v) = text("name") {
        if v.trim().is_empty() {
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "A newsletter needs a name.",
                "ValidationError",
            );
        }
        n.name = v;
    }
    if let Some(v) = nullable("description") {
        n.description = v;
    }
    if let Some(v) = nullable("sender_name") {
        n.sender_name = v;
    }
    if let Some(v) = nullable("sender_email") {
        n.sender_email = v;
    }
    if let Some(v) = text("sender_reply_to") {
        n.sender_reply_to = v;
    }
    if let Some(v) = text("status") {
        n.status = v;
    }
    if let Some(v) = text("visibility") {
        n.visibility = v;
    }
    if let Some(v) = flag("subscribe_on_signup") {
        n.subscribe_on_signup = v;
    }
    if let Some(v) = patch.get("sort_order").and_then(|v| v.as_i64()) {
        n.sort_order = v;
    }
    if let Some(v) = nullable("header_image") {
        n.header_image = v;
    }
    if let Some(v) = flag("show_header_icon") {
        n.show_header_icon = v;
    }
    if let Some(v) = flag("show_header_title") {
        n.show_header_title = v;
    }
    if let Some(v) = flag("show_header_name") {
        n.show_header_name = v;
    }
    if let Some(v) = flag("show_post_title_section") {
        n.show_post_title_section = v;
    }
    if let Some(v) = flag("show_excerpt") {
        n.show_excerpt = v;
    }
    if let Some(v) = flag("show_feature_image") {
        n.show_feature_image = v;
    }
    if let Some(v) = text("title_font_category") {
        n.title_font_category = v;
    }
    if let Some(v) = text("title_alignment") {
        n.title_alignment = v;
    }
    if let Some(v) = text("title_font_weight") {
        n.title_font_weight = v;
    }
    if let Some(v) = text("body_font_category") {
        n.body_font_category = v;
    }
    if let Some(v) = text("background_color") {
        n.background_color = v;
    }
    if let Some(v) = text("header_background_color") {
        n.header_background_color = v;
    }
    if let Some(v) = nullable("post_title_color") {
        n.post_title_color = v;
    }
    if let Some(v) = nullable("section_title_color") {
        n.section_title_color = v;
    }
    if let Some(v) = nullable("divider_color") {
        n.divider_color = v;
    }
    if let Some(v) = nullable("link_color") {
        n.link_color = v;
    }
    if let Some(v) = text("link_style") {
        n.link_style = v;
    }
    if let Some(v) = nullable("button_color") {
        n.button_color = v;
    }
    if let Some(v) = text("button_style") {
        n.button_style = v;
    }
    if let Some(v) = text("button_corners") {
        n.button_corners = v;
    }
    if let Some(v) = text("image_corners") {
        n.image_corners = v;
    }
    if let Some(v) = nullable("footer_content") {
        n.footer_content = v;
    }
    if let Some(v) = flag("show_badge") {
        n.show_badge = v;
    }
    if let Some(v) = flag("show_comment_cta") {
        n.show_comment_cta = v;
    }
    if let Some(v) = flag("show_subscription_details") {
        n.show_subscription_details = v;
    }
    if let Some(v) = flag("show_latest_posts") {
        n.show_latest_posts = v;
    }
    if let Some(v) = flag("show_share_button") {
        n.show_share_button = v;
    }
    if let Some(v) = flag("feedback_enabled") {
        n.feedback_enabled = v;
    }

    match repos::newsletters::update(&state.db, &n).await {
        Ok(()) => match repos::newsletters::find_by_id(&state.db, &id).await {
            Ok(Some(fresh)) => ok_json(json!({ "newsletters": [newsletter_json(&fresh)] })),
            _ => ok_json(json!({ "newsletters": [newsletter_json(&n)] })),
        },
        Err(e) => from_ghost(e),
    }
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

const SESSION_COOKIE: &str = "ghost-admin-api-session";

/// The session id carried on a request, verified or not.
fn session_id_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix(&format!("{SESSION_COOKIE}=")))
        .map(str::to_string)
}

/// Resolve the acting user, from an Admin API key or the session cookie.
/// None = unauthenticated.
///
/// The cookie path goes through [`repos::sessions::get_user`], which only
/// resolves a verified session — so a login still waiting on its 2FA code
/// authenticates nothing.
///
/// The key path is what every integration uses: Zapier, the official
/// `@tryghost/admin-api` SDK, a headless frontend publishing posts. It is
/// checked first because a request carrying `Authorization: Ghost …` is
/// stating which identity it wants, and silently preferring a cookie that
/// happened to ride along would be the wrong one.
pub(crate) async fn current_user(state: &AppState, headers: &HeaderMap) -> Option<ghost_core::models::User> {
    if let Some(user) = user_from_api_key(state, headers).await {
        return Some(user);
    }
    let sid = session_id_from(headers)?;
    let user_id = repos::sessions::get_user(&state.db, &sid).await.ok()??;
    repos::users::find_by_id(&state.db, &user_id).await.ok()?
}

/// The staff user an Admin API key acts as.
///
/// Ghost gives an integration key its own "Admin Integration" role rather
/// than a user. This port's Admin API gates on a user rather than a role, so
/// a key that names no user acts as the owner — which is the authority Ghost
/// grants an admin integration anyway. A key is only ever created from the
/// Integrations screen, which already requires an administrator.
async fn user_from_api_key(state: &AppState, headers: &HeaderMap) -> Option<ghost_core::models::User> {
    let header = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let token = crate::apikey_auth::token_from_header(header)?;
    let verified = crate::apikey_auth::verify(state, token, "admin").await?;

    let user = match &verified.user_id {
        Some(uid) => repos::users::find_by_id(&state.db, uid).await.ok().flatten(),
        None => None,
    };
    let user = match user {
        Some(u) => Some(u),
        None => repos::users::find_owner(&state.db).await.ok().flatten(),
    }?;

    // Ghost records this so the Integrations screen can say when a key was
    // last used, which is how you tell a live integration from a stale one.
    let _ = sqlx::query("UPDATE api_keys SET last_seen_at = ?, last_seen_version = ? WHERE id = ?")
        .bind(ghost_core::dates::now_db())
        .bind(ghost_core::GHOST_VERSION_SAFE)
        .bind(&verified.key_id)
        .execute(&state.db)
        .await;

    Some(user)
}

async fn require_user(state: &AppState, headers: &HeaderMap) -> Result<ghost_core::models::User, Response> {
    current_user(state, headers).await.ok_or_else(|| {
        err(StatusCode::UNAUTHORIZED, "Authentication required", "NoPermissionError")
    })
}

fn session_cookie_header(sid: &str, max_age_secs: i64) -> String {
    format!("{SESSION_COOKIE}={sid}; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}", ghost_core::url::subdir())
}

fn clear_session_cookie_header() -> String {
    format!("{SESSION_COOKIE}=; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age=0", ghost_core::url::subdir())
}

// ---------------------------------------------------------------------------
// setup flow (unauthenticated while setup is incomplete — like Ghost)
// ---------------------------------------------------------------------------

async fn setup_status(State(state): State<AppState>) -> Response {
    let done = repos::users::is_setup(&state.db).await.unwrap_or(false);
    ok_json(json!({ "setup": [{ "status": done, "markseen": null }] }))
}

async fn complete_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // lock: only while setup is incomplete
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return err(StatusCode::FORBIDDEN, "Setup has already been completed", "NoPermissionError");
    }
    let _ = headers;

    let Some(setup) = body.get("setup").and_then(|s| s.as_array()).and_then(|a| a.first()) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "setup[0] payload required", "ValidationError");
    };
    let name = setup.get("name").and_then(|v| v.as_str()).unwrap_or("Ghost");
    let email = setup.get("email").and_then(|v| v.as_str()).unwrap_or("ghost@example.com");
    let password = setup.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let blog_title = setup.get("blogTitle").and_then(|v| v.as_str()).unwrap_or("Ghost");
    let description = setup.get("description").and_then(|v| v.as_str()).unwrap_or("");

    if password.len() < 10 {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Password must be at least 10 characters", "ValidationError");
    }
    let password = password.to_string();

    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(&password, 12)).await {
        Ok(Ok(h)) => h,
        _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "hash failed", "InternalServerError"),
    };

    let owner = match repos::users::setup_owner(&state.db, name, email, &hash).await {
        Ok(o) => o,
        Err(e) => return from_ghost(e),
    };

    // title / description settings
    let seeded_title = state.settings.get_str("title").unwrap_or_default();
    let _ = state.settings.set(&state.db, "title", &json!(blog_title)).await;
    if !description.is_empty() {
        let _ = state.settings.set(&state.db, "description", &json!(description)).await;
    }
    // The seeded newsletter was named after the placeholder title; this is
    // the one moment it is renamed to match the real one. From here on the
    // name is the owner's, and a later title change leaves it alone.
    let _ = repos::newsletters::name_after_site(&state.db, &seeded_title, blog_title).await;
    // fix fixture post placeholders
    let _ = sqlx::query("UPDATE posts SET title = ? WHERE slug = 'coming-soon'")
        .bind(blog_title)
        .execute(&state.db)
        .await;
    state.refresh_setup_flag().await;
    let _ = repos::actions::record(&state.db, "user", &owner.id, &owner.id, "setup").await;

    (
        StatusCode::CREATED,
        Json(json!({ "users": [ghost_core::models::user_public_json(&owner, None)] })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// session login / logout
// ---------------------------------------------------------------------------

async fn login(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    // origin check (CSRF defense like Ghost's session-service)
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
        if !origin.contains(host) {
            return err(StatusCode::FORBIDDEN, "Invalid origin", "NoPermissionError");
        }
    }

    let email = body.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    let user = match repos::users::verify_password(&state.db, email, password).await {
        Ok(Some(u)) => u,
        _ => return err(StatusCode::UNAUTHORIZED, "Invalid email or password", "UnauthorizedError"),
    };

    // When email 2FA is required the password alone cannot mint a session.
    // A caller that already holds an unverified session sends the code back
    // as `token`, which is the shape Ghost's own admin client uses.
    // A first-ever sign-in skips the code, the way Ghost's own session
    // endpoint does — the password came from an emailed link already.
    if state.settings.get_bool("require_email_mfa") && user.last_seen.is_some() {
        let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let pending = session_id_from(&headers);
        match (pending, token) {
            (Some(sid), token) if !token.is_empty() => {
                use ghost_db::repos::sessions::CodeCheck;
                match repos::sessions::verify_auth_code(&state.db, &sid, token).await {
                    Ok(CodeCheck::Verified) => {
                        let _ = repos::users::update_last_seen(&state.db, &user.id).await;
                        let mut resp = (StatusCode::CREATED, Json(json!({ "users": [admin_user_json(&user)] }))).into_response();
                        resp.headers_mut().insert(
                            header::SET_COOKIE,
                            session_cookie_header(&sid, 60 * 60 * 24 * 7).parse().unwrap(),
                        );
                        return resp;
                    }
                    _ => return err(StatusCode::UNAUTHORIZED, "That verification code is not valid", "UnauthorizedError"),
                }
            }
            _ => {
                // No code yet: hand back an unverified session so the caller
                // has something to carry the code against. The code itself is
                // emailed by the admin UI's sign-in screen, which is where a
                // human completes this flow.
                let Ok(sid) = repos::sessions::create_unverified(&state.db, &user.id).await else {
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "Could not start a session", "InternalServerError");
                };
                let mut resp = err(StatusCode::FORBIDDEN, "A verification code is required to sign in", "Needs2FAError");
                resp.headers_mut().insert(
                    header::SET_COOKIE,
                    session_cookie_header(&sid, 60 * 60 * 24 * 7).parse().unwrap(),
                );
                return resp;
            }
        }
    }

    let sid = match repos::sessions::create(&state.db, &user.id).await {
        Ok(s) => s,
        Err(e) => return from_ghost(e),
    };
    let _ = repos::users::update_last_seen(&state.db, &user.id).await;

    let mut resp = (
        StatusCode::CREATED,
        Json(json!({ "users": [admin_user_json(&user)] })),
    )
        .into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie_header(&sid, 60 * 60 * 24 * 7).parse().unwrap(),
    );
    resp
}

fn admin_user_json(u: &ghost_core::models::User) -> Value {
    let mut j = ghost_core::models::user_public_json(u, None);
    if let Value::Object(m) = &mut j {
        m.insert("email".into(), json!(u.email));
        m.insert("status".into(), json!(u.status));
        m.insert("last_seen".into(), json!(ghost_core::dates::db_to_api(u.last_seen.as_deref())));
        m.insert("created_at".into(), json!(ghost_core::dates::db_to_api(u.created_at.as_deref())));
    }
    j
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(sid) = cookie.split(';').find_map(|c| c.trim().strip_prefix(&format!("{SESSION_COOKIE}="))) {
            let _ = repos::sessions::destroy(&state.db, sid).await;
        }
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, clear_session_cookie_header())],
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// posts / pages CRUD (markdown pipeline in)
// ---------------------------------------------------------------------------

fn markdown_pipeline(markdown: &str) -> (String, String, i64) {
    ghost_render::markdown::render_markdown(markdown)
}

async fn admin_browse_posts_impl(state: AppState, params: HashMap<String, String>, post_type: &str, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let q = crate::content::parse_browse(&params);
    let filter = match q.filter.as_deref().map(ghost_nql::parse).transpose() {
        Ok(f) => f,
        Err(e) => return from_ghost(e),
    };
    // admin can also filter by ?status=
    let filter = match params.get("status") {
        Some(s) if !s.is_empty() => Some(ghost_nql::Expr::And(
            vec![
                filter.unwrap_or(ghost_nql::Expr::Cond(ghost_nql::Cond { key: "status".into(), cmp: ghost_nql::Cmp::Eq, value: ghost_nql::NqlValue::Str(s.clone()) })),
            ],
        )),
        _ => filter,
    };
    let result = repos::posts::browse(
        &state.db,
        &repos::posts::BrowsePosts {
            filter,
            order: q.order.clone(),
            limit: q.limit,
            page: q.page,
            post_type: Some(post_type.to_string()),
            public: false,
            include_authors: true,
            include_tags: true,
        },
    )
    .await;
    match result {
        Ok(page) => {
            let resolver = crate::content::resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, &page.posts).await.unwrap_or_default();
            let items: Vec<Value> = page
                .posts
                .iter()
                .map(|p| {
                    let authors = rel.authors_for(&p.id);
                    let tags = rel.tags_for(&p.id);
                    let mut j = ghost_core::models::post_to_json(p, &authors, &tags, "");
                    j["url"] = json!(resolver(&j));
                    j["type"] = json!(post_type);
                    j["status"] = json!(p.status);
                    j
                })
                .collect();
            let doc = if post_type == "page" { "pages" } else { "posts" };
            let pagination = ghost_core::models::pagination_json(page.page, page.limit.max(1), page.total);
            ok_json(json!({ doc: items, "meta": { "pagination": pagination } }))
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_browse_posts(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    admin_browse_posts_impl(state, params, "post", headers).await
}

async fn admin_browse_pages(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    admin_browse_posts_impl(state, params, "page", headers).await
}

fn post_input_to_new(body: &Value, post_type: &str) -> repos::posts::NewPost {
    let posts = body.get(if post_type == "page" { "pages" } else { "posts" })
        .and_then(|p| p.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| body.clone());

    let str_field = |key: &str| posts.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let markdown = str_field("markdown");
    let lexical = str_field("lexical").filter(|l| !l.trim().is_empty());
    let html_in = str_field("html");
    // Markdown is this server's own editor format and html is what a client
    // sends ready-made; lexical is Ghost's, and is rendered the same way the
    // admin editor's documents are.
    let (html, plaintext, reading_time) = if let Some(md) = &markdown {
        markdown_pipeline(md)
    } else {
        // Ready-made HTML wins over lexical, because a client that sends
        // both has already rendered the cards with Ghost's own renderer.
        let html = match (&html_in, &lexical) {
            (Some(html), _) => html.clone(),
            (None, Some(lexical)) => ghost_render::lexical::to_html(lexical),
            (None, None) => String::new(),
        };
        let plain = ghost_render::markdown::html_to_plaintext(&html);
        let rt = ghost_render::markdown::reading_time(&plain);
        (html, plain, rt)
    };

    repos::posts::NewPost {
        title: posts.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string(),
        slug: str_field("slug"),
        lexical,
        markdown,
        html: Some(html),
        plaintext: Some(plaintext),
        custom_excerpt: str_field("custom_excerpt"),
        feature_image: str_field("feature_image"),
        featured: posts.get("featured").and_then(|v| v.as_bool()).unwrap_or(false),
        post_type: post_type.to_string(),
        status: posts.get("status").and_then(|v| v.as_str()).unwrap_or("draft").to_string(),
        visibility: posts.get("visibility").and_then(|v| v.as_str()).unwrap_or("public").to_string(),
        published_at: None,
        codeinjection_head: str_field("codeinjection_head"),
        codeinjection_foot: str_field("codeinjection_foot"),
        custom_template: str_field("custom_template"),
        canonical_url: str_field("canonical_url"),
        show_title_and_feature_image: posts.get("show_title_and_feature_image").and_then(|v| v.as_bool()),
        meta_title: str_field("meta_title"),
        meta_description: str_field("meta_description"),
        og_title: str_field("og_title"),
        og_description: str_field("og_description"),
        og_image: str_field("og_image"),
        twitter_title: str_field("twitter_title"),
        twitter_description: str_field("twitter_description"),
        twitter_image: str_field("twitter_image"),
        email_subject: str_field("email_subject"),
        feature_image_alt: str_field("feature_image_alt"),
        feature_image_caption: str_field("feature_image_caption"),
        author_ids: posts.get("authors").and_then(|a| a.as_array()).map(|a| {
            a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()
        }).unwrap_or_default(),
        tag_ids: posts.get("tags").and_then(|a| a.as_array()).map(|a| {
            a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()
        }).unwrap_or_default(),
        reading_time,
    }
}

async fn admin_add_post_impl(state: AppState, user: ghost_core::models::User, body: Value, post_type: &str) -> Response {
    let mut new = post_input_to_new(&body, post_type);
    // tags may arrive as {name} (create-or-find, like Ghost) or {id}
    if let Some(tags) = body.get(if post_type == "page" { "pages" } else { "posts" })
        .and_then(|p| p.as_array()).and_then(|a| a.first())
        .and_then(|p| p.get("tags")).and_then(|t| t.as_array()).cloned()
    {
        let mut ids = Vec::new();
        for t in tags {
            if let Some(id) = t.get("id").and_then(|i| i.as_str()) {
                ids.push(id.to_string());
            } else if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                let slug = ghost_core::slug::safe_string(name);
                let existing = repos::tags::find_by_slug(&state.db, &slug).await.ok().flatten();
                let tag = match existing {
                    Some(t) => t,
                    None => match repos::tags::insert(&state.db, &repos::tags::NewTag {
                        name: name.to_string(),
                        ..Default::default()
                    }).await {
                        Ok(t) => t,
                        Err(e) => return from_ghost(e),
                    },
                };
                ids.push(tag.id);
            }
        }
        new.tag_ids = ids;
    }
    match repos::posts::insert(&state.db, &new, &user.id).await {
        Ok(post) => {
            let resolver = crate::content::resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
                .await
                .unwrap_or_default();
            let j = ghost_core::models::post_to_json(
                &post,
                &rel.authors_for(&post.id),
                &rel.tags_for(&post.id),
                "",
            );
            let mut j = j;
            j["url"] = json!(resolver(&j));
            j["type"] = json!(post_type);
            let _ = repos::actions::record(&state.db, post_type, &post.id, &user.id, "added").await;
            let doc = if post_type == "page" { "pages" } else { "posts" };
            (StatusCode::CREATED, Json(json!({ doc: [j] }))).into_response()
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_add_post(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    admin_add_post_impl(state, user, body, "post").await
}

async fn admin_add_page(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    admin_add_post_impl(state, user, body, "page").await
}

async fn admin_read_post(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let _user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match repos::posts::find_by_id(&state.db, &id).await {
        Ok(Some(post)) => {
            let resolver = crate::content::resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
                .await
                .unwrap_or_default();
            let mut j = ghost_core::models::post_to_json(&post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
            j["url"] = json!(resolver(&j));
            j["type"] = json!(post.post_type);
            let mut m = Map::new();
            m.insert(if post.post_type == "page" { "pages" } else { "posts" }.to_string(), Value::Array(vec![j]));
            ok_json(Value::Object(m))
        }
        _ => err(StatusCode::NOT_FOUND, "Post not found", "NotFoundError"),
    }
}

async fn admin_edit_post(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let post_type = repos::posts::find_by_id(&state.db, &id)
        .await
        .ok()
        .flatten()
        .map(|p| p.post_type)
        .unwrap_or_else(|| "post".into());

    let posts = body.get(if post_type == "page" { "pages" } else { "posts" })
        .and_then(|p| p.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| body.clone());

    let str_field = |key: &str| posts.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let markdown = str_field("markdown");
    let lexical = str_field("lexical").filter(|l| !l.trim().is_empty());
    // Only recompute the rendered columns when the request actually carries
    // content; a PUT that just flips `featured` leaves the body alone.
    let (html, plaintext, reading_time) = if let Some(md) = &markdown {
        let (h, p, rt) = markdown_pipeline(md);
        (Some(h), Some(p), Some(rt))
    } else {
        let rendered = match (str_field("html"), &lexical) {
            (Some(html), _) => Some(html),
            (None, Some(lexical)) => Some(ghost_render::lexical::to_html(lexical)),
            (None, None) => None,
        };
        match rendered {
            Some(html) => {
                let plain = ghost_render::markdown::html_to_plaintext(&html);
                let rt = ghost_render::markdown::reading_time(&plain);
                (Some(html), Some(plain), Some(rt))
            }
            None => (None, None, None),
        }
    };

    let changes = repos::posts::PostChanges {
        title: str_field("title"),
        slug: str_field("slug"),
        lexical,
        markdown,
        html,
        plaintext,
        custom_excerpt: str_field("custom_excerpt"),
        feature_image: str_field("feature_image"),
        featured: posts.get("featured").and_then(|v| v.as_bool()),
        status: str_field("status"),
        visibility: str_field("visibility"),
        published_at: None,
        codeinjection_head: str_field("codeinjection_head"),
        codeinjection_foot: str_field("codeinjection_foot"),
        custom_template: str_field("custom_template"),
        canonical_url: str_field("canonical_url"),
        show_title_and_feature_image: posts.get("show_title_and_feature_image").and_then(|v| v.as_bool()),
        meta_title: str_field("meta_title"),
        meta_description: str_field("meta_description"),
        og_title: str_field("og_title"),
        og_description: str_field("og_description"),
        og_image: str_field("og_image"),
        twitter_title: str_field("twitter_title"),
        twitter_description: str_field("twitter_description"),
        twitter_image: str_field("twitter_image"),
        email_subject: str_field("email_subject"),
        feature_image_alt: str_field("feature_image_alt"),
        feature_image_caption: str_field("feature_image_caption"),
        reading_time,
        tag_ids: posts.get("tags").and_then(|a| a.as_array()).map(|a| {
            a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()
        }),
        author_ids: posts.get("authors").and_then(|a| a.as_array()).map(|a| {
            a.iter().filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect()
        }),
        // A PATCH-shaped API: absent means unchanged, so nothing is cleared.
        clearing: vec![],
    };

    match repos::posts::update(&state.db, &id, &changes).await {
        Ok(post) => {
            let resolver = crate::content::resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
                .await
                .unwrap_or_default();
            let mut j = ghost_core::models::post_to_json(&post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
            j["url"] = json!(resolver(&j));
            let _ = repos::actions::record(&state.db, &post_type, &post.id, &user.id, "edited").await;
            let doc = if post.post_type == "page" { "pages" } else { "posts" };
            ok_json(json!({ doc: [j] }))
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_delete_post(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match repos::posts::destroy(&state.db, &id).await {
        Ok(()) => {
            let _ = repos::actions::record(&state.db, "post", &id, &user.id, "deleted").await;
            (StatusCode::NO_CONTENT, Json(json!({ "posts": [] }))).into_response()
        }
        Err(e) => from_ghost(e),
    }
}

// ---------------------------------------------------------------------------
// tags CRUD
// ---------------------------------------------------------------------------

async fn admin_browse_tags(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let q = crate::content::parse_browse(&params);
    let include_count = q.include.iter().any(|i| i == "count.posts");
    match repos::tags::browse(&state.db, None, q.order.as_deref(), q.limit, q.page, false, include_count).await {
        Ok((tags, total)) => {
            let resolver = crate::content::resolve(&state).await;
            let items: Vec<Value> = tags
                .iter()
                .map(|t| {
                    let mut j = ghost_core::models::tag_to_json(t, None);
                    let url = resolver(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            let pagination = ghost_core::models::pagination_json(q.page, if q.limit > 0 { q.limit } else { 15 }, total);
            ok_json(json!({ "tags": items, "meta": { "pagination": pagination } }))
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_read_tag(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let found = if ghost_core::ids::is_objectid(&id) {
        repos::tags::find_by_id(&state.db, &id).await
    } else {
        repos::tags::find_by_slug(&state.db, &id).await
    };
    match found {
        Ok(Some(t)) => {
            let resolver = crate::content::resolve(&state).await;
            let mut j = ghost_core::models::tag_to_json(&t, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            ok_json(json!({ "tags": [j] }))
        }
        _ => err(StatusCode::NOT_FOUND, "Tag not found", "NotFoundError"),
    }
}

async fn admin_add_tag(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let tag = body.get("tags").and_then(|t| t.as_array()).and_then(|a| a.first()).cloned().unwrap_or(body);
    let new = repos::tags::NewTag {
        name: tag.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string(),
        slug: tag.get("slug").and_then(|v| v.as_str()).map(str::to_string),
        description: tag.get("description").and_then(|v| v.as_str()).map(str::to_string),
        feature_image: tag.get("feature_image").and_then(|v| v.as_str()).map(str::to_string),
        visibility: tag.get("visibility").and_then(|v| v.as_str()).unwrap_or("public").to_string(),
        accent_color: tag.get("accent_color").and_then(|v| v.as_str()).map(str::to_string),
        meta_title: tag.get("meta_title").and_then(|v| v.as_str()).map(str::to_string),
        meta_description: tag.get("meta_description").and_then(|v| v.as_str()).map(str::to_string),
    };
    match repos::tags::insert(&state.db, &new).await {
        Ok(t) => {
            let _ = repos::actions::record(&state.db, "tag", &t.id, &user.id, "added").await;
            (StatusCode::CREATED, Json(json!({ "tags": [ghost_core::models::tag_to_json(&t, None)] }))).into_response()
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_edit_tag(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let tag = body.get("tags").and_then(|t| t.as_array()).and_then(|a| a.first()).cloned().unwrap_or(body);
    // Absent key means "leave it alone"; a present key (even empty) is a
    // change, which is what lets a client clear a field.
    let f = |key: &str| tag.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let changes = repos::tags::TagChanges {
        name: f("name"),
        slug: f("slug"),
        description: f("description"),
        feature_image: f("feature_image"),
        visibility: f("visibility"),
        accent_color: f("accent_color"),
        meta_title: f("meta_title"),
        meta_description: f("meta_description"),
        canonical_url: f("canonical_url"),
        og_image: f("og_image"),
        og_title: f("og_title"),
        og_description: f("og_description"),
        twitter_image: f("twitter_image"),
        twitter_title: f("twitter_title"),
        twitter_description: f("twitter_description"),
        codeinjection_head: f("codeinjection_head"),
        codeinjection_foot: f("codeinjection_foot"),
    };
    match repos::tags::update(&state.db, &id, &changes).await {
        Ok(t) => ok_json(json!({ "tags": [ghost_core::models::tag_to_json(&t, None)] })),
        Err(e) => from_ghost(e),
    }
}

async fn admin_delete_tag(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::tags::destroy(&state.db, &id).await {
        Ok(()) => (StatusCode::NO_CONTENT, Json(json!({ "tags": [] }))).into_response(),
        Err(e) => from_ghost(e),
    }
}

// ---------------------------------------------------------------------------
// users
// ---------------------------------------------------------------------------

async fn admin_browse_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::users::list(&state.db, false).await {
        Ok(users) => {
            let items: Vec<Value> = users
                .iter()
                .map(|u| {
                    let mut j = admin_user_json(u);
                    if let Value::Object(m) = &mut j {
                        m.insert("roles".into(), json!([]));
                    }
                    j
                })
                .collect();
            ok_json(json!({ "users": items }))
        }
        Err(e) => from_ghost(e),
    }
}

/// Read one staff user.
///
/// `id` may be the literal `me`, which Ghost resolves to the session's own
/// user — the comments widget's auth frame calls `/users/me/?include=roles`
/// to find out whether the person browsing the site is a moderator, and that
/// is the only way to ask without already knowing the answer.
async fn admin_read_user(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let actor = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let found = if id == "me" {
        Some(actor)
    } else {
        repos::users::find_by_id(&state.db, &id).await.ok().flatten()
    };
    let Some(u) = found else {
        return err(StatusCode::NOT_FOUND, "User not found", "NotFoundError");
    };

    let mut json = admin_user_json(&u);
    let wants_roles = params
        .get("include")
        .map(|i| i.split(',').any(|p| p.trim() == "roles"))
        .unwrap_or(false);
    if wants_roles {
        let name = repos::users::role_name(&state.db, &u.id).await.ok().flatten();
        if let Value::Object(m) = &mut json {
            m.insert(
                "roles".into(),
                match name {
                    Some(name) => json!([{ "name": name }]),
                    None => json!([]),
                },
            );
        }
    }
    ok_json(json!({ "users": [json] }))
}

async fn admin_edit_user(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let user = body.get("users").and_then(|u| u.as_array()).and_then(|a| a.first()).cloned().unwrap_or(body);
    let changes = repos::users::UserChanges {
        name: user.get("name").and_then(|v| v.as_str()).map(str::to_string),
        email: user.get("email").and_then(|v| v.as_str()).map(str::to_string),
        password_hash: None,
        bio: user.get("bio").and_then(|v| v.as_str()).map(str::to_string),
        website: user.get("website").and_then(|v| v.as_str()).map(str::to_string),
        location: user.get("location").and_then(|v| v.as_str()).map(str::to_string),
        profile_image: user.get("profile_image").and_then(|v| v.as_str()).map(str::to_string),
        cover_image: user.get("cover_image").and_then(|v| v.as_str()).map(str::to_string),
        status: user.get("status").and_then(|v| v.as_str()).map(str::to_string),
        meta_title: user.get("meta_title").and_then(|v| v.as_str()).map(str::to_string),
        meta_description: user.get("meta_description").and_then(|v| v.as_str()).map(str::to_string),
        twitter: user.get("twitter").and_then(|v| v.as_str()).map(str::to_string),
        facebook: user.get("facebook").and_then(|v| v.as_str()).map(str::to_string),
        threads: user.get("threads").and_then(|v| v.as_str()).map(str::to_string),
        bluesky: user.get("bluesky").and_then(|v| v.as_str()).map(str::to_string),
        mastodon: user.get("mastodon").and_then(|v| v.as_str()).map(str::to_string),
        tiktok: user.get("tiktok").and_then(|v| v.as_str()).map(str::to_string),
        youtube: user.get("youtube").and_then(|v| v.as_str()).map(str::to_string),
        instagram: user.get("instagram").and_then(|v| v.as_str()).map(str::to_string),
        linkedin: user.get("linkedin").and_then(|v| v.as_str()).map(str::to_string),
        slug: user.get("slug").and_then(|v| v.as_str()).map(str::to_string),
        comment_notifications: user.get("comment_notifications").and_then(|v| v.as_bool()),
        free_member_signup_notification: user.get("free_member_signup_notification").and_then(|v| v.as_bool()),
        paid_subscription_started_notification: user.get("paid_subscription_started_notification").and_then(|v| v.as_bool()),
        paid_subscription_canceled_notification: user.get("paid_subscription_canceled_notification").and_then(|v| v.as_bool()),
        mention_notifications: user.get("mention_notifications").and_then(|v| v.as_bool()),
        recommendation_notifications: user.get("recommendation_notifications").and_then(|v| v.as_bool()),
        milestone_notifications: user.get("milestone_notifications").and_then(|v| v.as_bool()),
        donation_notifications: user.get("donation_notifications").and_then(|v| v.as_bool()),
        gift_subscription_notifications: user.get("gift_subscription_notifications").and_then(|v| v.as_bool()),
    };
    match repos::users::update(&state.db, &id, &changes).await {
        Ok(u) => ok_json(json!({ "users": [admin_user_json(&u)] })),
        Err(e) => from_ghost(e),
    }
}

async fn change_password(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let pw = body.get("password").and_then(|p| p.get("newPassword")).or_else(|| body.get("newPassword")).and_then(|v| v.as_str()).unwrap_or("");
    if pw.len() < 10 {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Password must be at least 10 characters", "ValidationError");
    }
    let pw = pw.to_string();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(&pw, 12)).await {
        Ok(Ok(h)) => h,
        _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "hash failed", "InternalServerError"),
    };
    match repos::users::update(&state.db, &user.id, &repos::users::UserChanges { password_hash: Some(hash), ..Default::default() }).await {
        Ok(_) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => from_ghost(e),
    }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// Settings holding a credential, which must never be read back out.
/// Matched by suffix as well as by name so a future `*_api_key` or
/// `*_secret` is covered without having to remember to list it.
pub fn is_secret_setting(key: &str) -> bool {
    key.ends_with("_pass")
        || key.ends_with("_password")
        || key.ends_with("_api_key")
        || key.ends_with("_secret")
        || key.ends_with("_private_key")
}

async fn admin_browse_settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::settings::all(&state.db).await {
        Ok(all) => {
            let items: Vec<Value> = all
                .iter()
                .map(|s| {
                    // Credentials are write-only over the API too: an admin
                    // token is for administering the site, not for reading
                    // back the SMTP password or Mailgun key it was given.
                    let value = if is_secret_setting(&s.key) {
                        match s.typed_value() {
                            Value::String(v) if !v.is_empty() => json!("••••••••"),
                            other => other,
                        }
                    } else {
                        s.typed_value()
                    };
                    json!({
                        "id": s.id,
                        "group": s.group,
                        "key": s.key,
                        "value": value,
                        "type": s.value_type,
                        "flags": s.flags,
                    })
                })
                .collect();
            ok_json(json!({ "settings": items }))
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_edit_settings(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let Some(list) = body.get("settings").and_then(|s| s.as_array()).cloned() else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "settings[] required", "ValidationError");
    };
    let mut updated = Vec::new();
    for item in list {
        let Some(key) = item.get("key").and_then(|k| k.as_str()) else { continue };
        let Some(value) = item.get("value") else { continue };
        if state.settings.set(&state.db, key, value).await.is_ok() {
            updated.push(json!({ "key": { "key": key, "value": value } }));
        }
    }
    ok_json(json!({ "settings": updated }))
}

// ---------------------------------------------------------------------------
// integrations
// ---------------------------------------------------------------------------

async fn admin_browse_integrations(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    match repos::integrations::list(&state.db).await {
        Ok(ints) => {
            let mut items = Vec::new();
            for i in ints {
                let mut j = json!({
                    "id": i.id, "name": i.name, "slug": i.slug,
                    "type": i.integration_type, "description": i.description,
                });
                if let Ok(keys) = repos::api_keys::find_by_integration(&state.db, &i.id).await {
                    j["api_keys"] = Value::Array(
                        keys.iter()
                            .map(|k| {
                                json!({ "id": k.id, "type": k.key_type, "secret": k.secret })
                            })
                            .collect(),
                    );
                }
                items.push(j);
            }
            ok_json(json!({ "integrations": items }))
        }
        Err(e) => from_ghost(e),
    }
}

async fn admin_read_integration(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, type, name, slug FROM integrations WHERE id = ? OR slug = ? LIMIT 1",
    )
    .bind(&id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    match rows {
        Some((id, typ, name, slug)) => {
            let mut j = json!({ "id": id, "name": name, "slug": slug, "type": typ });
            if let Ok(keys) = repos::api_keys::find_by_integration(&state.db, &id).await {
                j["api_keys"] = Value::Array(
                    keys.iter().map(|k| json!({ "id": k.id, "type": k.key_type, "secret": k.secret })).collect(),
                );
            }
            ok_json(json!({ "integrations": [j] }))
        }
        None => err(StatusCode::NOT_FOUND, "Integration not found", "NotFoundError"),
    }
}

async fn admin_add_integration(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let user = match require_user(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let name = body.get("integrations").and_then(|i| i.as_array()).and_then(|a| a.first())
        .and_then(|i| i.get("name")).and_then(|n| n.as_str())
        .or_else(|| body.get("name").and_then(|n| n.as_str()))
        .unwrap_or("Unnamed integration");
    let key_type = body.get("integrations").and_then(|i| i.as_array()).and_then(|a| a.first())
        .and_then(|i| i.get("api_keys")).and_then(|k| k.as_array()).and_then(|a| a.first())
        .and_then(|k| k.get("type")).and_then(|t| t.as_str())
        .or_else(|| body.get("type").and_then(|t| t.as_str()))
        .unwrap_or("content");
    match repos::api_keys::create_integration(&state.db, name, key_type).await {
        Ok((integration, key)) => {
            let _ = repos::actions::record(&state.db, "integration", &integration.id, &user.id, "added").await;
            (
                StatusCode::CREATED,
                Json(json!({
                    "integrations": [{
                        "id": integration.id,
                        "name": integration.name,
                        "slug": integration.slug,
                        "type": integration.integration_type,
                        "api_keys": [{ "id": key.id, "type": key.key_type, "secret": key.secret }],
                    }]
                })),
            )
                .into_response()
        }
        Err(e) => from_ghost(e),
    }
}

// ---------------------------------------------------------------------------
// site
// ---------------------------------------------------------------------------

async fn site_info(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    let _ = params;
    // The Ghost version, not this crate's. SDKs and integrations branch on
    // it to decide which API shape to speak; reporting `1.0.0` told every
    // one of them this was a pre-Ghost-5 site.
    let version = ghost_core::GHOST_VERSION_SAFE;
    ok_json(json!({
        "site": {
            "title": state.settings.get_str("title").unwrap_or_else(|| "Ghost".into()),
            "description": state.settings.get_str("description").unwrap_or_default(),
            "url": state.site_url(),
            "version": version,
        }
    }))
}

// ---------------------------------------------------------------------------
// webhooks — how an integration registers its own triggers
// ---------------------------------------------------------------------------

/// Ghost's webhook response shape, which its SDKs and Zapier parse.
fn webhook_json(w: &ghost_db::repos::webhooks::Webhook) -> Value {
    json!({
        "id": w.id,
        "event": w.event,
        "target_url": w.target_url,
        "name": w.name,
        "secret": w.secret,
        // The port writes v5 rows; reported as a constant rather than
        // selected, since nothing varies it.
        "api_version": "v5",
        "integration_id": w.integration_id,
        "last_triggered_at": w.last_triggered_at,
        "last_triggered_status": w.last_triggered_status,
        "last_triggered_error": w.last_triggered_error,
        "created_at": w.created_at,
        "updated_at": w.created_at,
    })
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct WebhookInput {
    event: String,
    target_url: String,
    name: Option<String>,
    secret: Option<String>,
    integration_id: Option<String>,
}

/// Pull the one resource out of Ghost's `{webhooks: [{...}]}` envelope.
fn first_of<'a>(body: &'a Value, key: &str) -> Option<&'a Value> {
    body.get(key).and_then(|v| v.as_array()).and_then(|a| a.first())
}

/// `POST /webhooks/` — register a listener.
///
/// The integration is taken from the authenticating key when the body does
/// not name one, which is exactly how Zapier calls this: it knows its own
/// token, not the integration id behind it.
async fn admin_add_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let Some(input) = first_of(&body, "webhooks")
        .and_then(|v| serde_json::from_value::<WebhookInput>(v.clone()).ok())
    else {
        return err(StatusCode::BAD_REQUEST, "No webhook was given", "BadRequestError");
    };

    if !crate::webhooks::is_known_event(&input.event) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Unknown webhook event: {}", input.event),
            "ValidationError",
        );
    }
    if !input.target_url.starts_with("http://") && !input.target_url.starts_with("https://") {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "target_url must be an http or https URL",
            "ValidationError",
        );
    }

    // The integration the webhook belongs to: the one the body named, else
    // the one behind the authenticating key — which is how Zapier calls
    // this, knowing its token but not the integration id behind it.
    let integration_id = match input.integration_id.clone() {
        Some(id) => id,
        None => match integration_from_api_key(&state, &headers).await {
            Some(id) => id,
            None => match default_integration_id(&state).await {
                Some(id) => id,
                None => {
                    return err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "No integration to attach this webhook to",
                        "ValidationError",
                    )
                }
            },
        },
    };

    match repos::webhooks::create(
        &state.db,
        &integration_id,
        &input.event,
        &input.target_url,
        input.name.as_deref().unwrap_or(""),
        input.secret.as_deref().unwrap_or(""),
    )
    .await
    {
        Ok(w) => (StatusCode::CREATED, Json(json!({ "webhooks": [webhook_json(&w)] }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.message(), "InternalServerError"),
    }
}

/// `PUT /webhooks/{id}` — repoint or rename an existing listener.
async fn admin_edit_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    let Ok(Some(existing)) = repos::webhooks::find(&state.db, &id).await else {
        return err(StatusCode::NOT_FOUND, "Webhook not found", "NotFoundError");
    };
    let input = first_of(&body, "webhooks").cloned().unwrap_or(Value::Null);
    let take = |key: &str, fallback: &str| -> String {
        input.get(key).and_then(|v| v.as_str()).unwrap_or(fallback).to_string()
    };

    let event = take("event", &existing.event);
    if !crate::webhooks::is_known_event(&event) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Unknown webhook event: {event}"),
            "ValidationError",
        );
    }
    let target_url = take("target_url", &existing.target_url);
    let name = take("name", existing.name.as_deref().unwrap_or(""));

    if sqlx::query(
        "UPDATE webhooks SET event = ?, target_url = ?, name = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&event)
    .bind(&target_url)
    .bind(&name)
    .bind(ghost_core::dates::now_db())
    .bind(&id)
    .execute(&state.db)
    .await
    .is_err()
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "Could not save", "InternalServerError");
    }
    match repos::webhooks::find(&state.db, &id).await {
        Ok(Some(w)) => Json(json!({ "webhooks": [webhook_json(&w)] })).into_response(),
        _ => err(StatusCode::NOT_FOUND, "Webhook not found", "NotFoundError"),
    }
}

/// `DELETE /webhooks/{id}` — how an integration cleans up after itself when
/// a trigger is switched off at the other end.
async fn admin_delete_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_user(&state, &headers).await {
        return r;
    }
    if repos::webhooks::find(&state.db, &id).await.ok().flatten().is_none() {
        return err(StatusCode::NOT_FOUND, "Webhook not found", "NotFoundError");
    }
    match repos::webhooks::delete(&state.db, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.message(), "InternalServerError"),
    }
}

/// The integration behind the authenticating Admin API key, if a key was
/// used. Re-verifies the token rather than threading it down from
/// authentication, which keeps this a plain function of the request.
async fn integration_from_api_key(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let header = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = crate::apikey_auth::token_from_header(header)?;
    crate::apikey_auth::verify(state, token, "admin").await?.integration_id
}

/// The integration a webhook lands on when nothing names one: the first
/// custom integration, else any. A webhook row must belong to something —
/// the column is a foreign key.
async fn default_integration_id(state: &AppState) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM integrations ORDER BY type = 'custom' DESC, created_at LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_recognised_as_secret() {
        for key in [
            "mail_smtp_pass",
            "mailgun_api_key",
            "members_email_auth_secret",
            "theme_session_secret",
            "members_private_key",
        ] {
            assert!(is_secret_setting(key), "{key} must never be readable back");
        }
    }

    #[test]
    fn ordinary_settings_are_not_masked() {
        // masking these would break the admin UI, which reads them to
        // populate its forms
        for key in ["title", "mail_smtp_host", "mail_smtp_user", "mailgun_domain", "mail_transport"] {
            assert!(!is_secret_setting(key), "{key} is not a credential");
        }
    }
}
