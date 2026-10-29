//! The rest of Ghost's Admin API.
//!
//! These endpoints existed in this port only as HTML forms in the admin, or
//! not at all. A form handler is not an API: Ghost's own SDK, Zapier, a
//! deploy script and every third-party integration speak REST, and a feature
//! reachable only through a rendered page is unavailable to all of them.
//!
//! What is here is the half that needs nothing but the database and the
//! settings. The half that reuses the admin's own import, export and theme
//! handling lives beside that code and is merged into the same prefix from
//! the binary — see `ghost_admin_ui::rest_router`.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use ghost_core::{dates, ids};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn admin_router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            // --- content helpers ------------------------------------------
            .route(&format!("/slugs/{{kind}}/{{name}}{suffix}"), get(slug_for))
            .route(&format!("/slugs/{{kind}}/{{name}}/{{id}}{suffix}"), get(slug_for_existing))
            .route(&format!("/posts/export{suffix}"), get(posts_export))
            .route(&format!("/search-index/pages{suffix}"), get(search_index_pages))
            .route(&format!("/search-index/users{suffix}"), get(search_index_users))
            // --- settings -------------------------------------------------
            .route(&format!("/config{suffix}"), get(config))
            .route(&format!("/custom_theme_settings{suffix}"), get(read_theme_settings).put(edit_theme_settings))
            .route(&format!("/settings/verifications{suffix}"), axum::routing::put(verifications))
            .route(&format!("/slack/test{suffix}"), post(slack_test))
            // --- staff ----------------------------------------------------
            .route(&format!("/users/email/{{email}}{suffix}"), get(user_by_email))
            .route(&format!("/users/slug/{{slug}}{suffix}"), get(user_by_slug))
            .route(&format!("/users/{{id}}/token{suffix}"), get(read_staff_token).put(regenerate_staff_token))
            // --- members --------------------------------------------------
            .route(&format!("/labels/slug/{{slug}}{suffix}"), get(label_by_slug))
            .route(&format!("/members/stripe_connect{suffix}"), get(stripe_connect))
            .route(&format!("/members/{{id}}/sessions{suffix}"), axum::routing::delete(end_member_sessions))
            .route(&format!("/feedback/{{id}}{suffix}"), get(post_feedback))
            // --- newsletters ----------------------------------------------
            .route(&format!("/links/bulk{suffix}"), axum::routing::put(links_bulk))
            // --- recommendations ------------------------------------------
            .route(&format!("/recommendations{suffix}"), get(browse_recommendations).post(add_recommendation))
            .route(
                &format!("/recommendations/{{id}}{suffix}"),
                get(read_recommendation).put(edit_recommendation).delete(delete_recommendation),
            )
            .route(&format!("/recommendations/check{suffix}"), post(check_recommendation))
            // --- gifts ----------------------------------------------------
            .route(&format!("/gift_links/remove_all{suffix}"), axum::routing::put(remove_all_gift_links))
            .route(&format!("/gifts/flush_deliveries{suffix}"), axum::routing::put(flush_gift_deliveries))
            .route(&format!("/gifts/flush_reminders{suffix}"), axum::routing::put(flush_gift_reminders))
            // --- scheduler ------------------------------------------------
            .route(&format!("/schedules/{{resource}}/{{id}}{suffix}"), axum::routing::put(publish_scheduled));
    }
    router
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]}))).into_response()
}

fn private(body: Value) -> Response {
    ([("cache-control", "private, no-store")], Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// content helpers
// ---------------------------------------------------------------------------

/// `GET /slugs/{kind}/{name}` — what slug a title would get.
///
/// The editor asks before saving so it can show the URL a post will have.
/// Uniqueness is per resource kind, which is why the kind is in the path.
async fn slug_for(State(state): State<AppState>, headers: HeaderMap, Path((kind, name)): Path<(String, String)>) -> Response {
    slug_response(&state, &headers, &kind, &name, None).await
}

/// The same, excluding one row — so re-saving a post keeps its own slug
/// instead of being handed `my-post-2`.
async fn slug_for_existing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, name, id)): Path<(String, String, String)>,
) -> Response {
    slug_response(&state, &headers, &kind, &name, Some(&id)).await
}

async fn slug_response(state: &AppState, headers: &HeaderMap, kind: &str, name: &str, id: Option<&str>) -> Response {
    if let Err(r) = permissions::require(state, headers, Role::Contributor).await {
        return r;
    }
    // The table is chosen from a fixed list, never from the path.
    let (table, extra) = match kind {
        "post" => ("posts", " AND type = 'post'"),
        "page" => ("posts", " AND type = 'page'"),
        "tag" => ("tags", ""),
        "user" | "author" => ("users", ""),
        "newsletter" => ("newsletters", ""),
        _ => return error(StatusCode::UNPROCESSABLE_ENTITY, "Unknown slug type."),
    };
    let base = ghost_core::slug::safe_string(name);
    if base.is_empty() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A name is required.");
    }
    // `-2`, `-3`, … until one is free, which is how Ghost deduplicates.
    let mut candidate = base.clone();
    for attempt in 2..1000 {
        let taken: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {table} WHERE slug = ? AND (? IS NULL OR id != ?){extra})"
        ))
        .bind(&candidate)
        .bind(id)
        .bind(id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
        if !taken {
            break;
        }
        candidate = format!("{base}-{attempt}");
    }
    private(json!({"slugs": [{"slug": candidate}]}))
}

/// `GET /posts/export` — every post as CSV, which is what Ghost's own
/// "export posts" button downloads.
async fn posts_export(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, String, String, Option<String>, Option<String>, i64, i64)>(
        "SELECT p.id, p.title, p.slug, p.status, p.visibility, p.published_at, p.created_at, \
                COALESCE((SELECT COUNT(*) FROM page_views v WHERE v.path LIKE '%/' || p.slug || '/'), 0), \
                COALESCE((SELECT e.opened_count FROM emails e WHERE e.post_id = p.id), 0) \
         FROM posts p WHERE p.type = 'post' ORDER BY p.created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut csv = String::from("id,title,slug,status,visibility,published_at,created_at,views,email_opens\n");
    for (id, title, slug, status, visibility, published_at, created_at, views, opens) in rows {
        let cell = |value: &str| {
            if value.contains([',', '"', '\n']) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                value.to_string()
            }
        };
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{views},{opens}\n",
            cell(&id),
            cell(title.as_deref().unwrap_or("")),
            cell(slug.as_deref().unwrap_or("")),
            cell(&status),
            cell(&visibility),
            cell(published_at.as_deref().unwrap_or("")),
            cell(created_at.as_deref().unwrap_or("")),
        ));
    }
    (
        [
            ("content-type", "text/csv; charset=utf-8"),
            ("content-disposition", "attachment; filename=\"posts.csv\""),
            ("cache-control", "private, no-store"),
        ],
        csv,
    )
        .into_response()
}

/// The two search-index resources this port was missing. Ghost's admin
/// searches across posts, pages, tags and staff; pages and staff had no
/// endpoint, so they could not be found.
async fn search_index_pages(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Contributor).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT id, title, slug FROM posts WHERE type = 'page' AND status = 'published' ORDER BY updated_at DESC LIMIT 10000",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"pages": rows.into_iter().map(|(id, title, slug)|
        json!({"id": id, "title": title, "slug": slug})).collect::<Vec<_>>()}))
}

async fn search_index_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Contributor).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT id, name, slug FROM users WHERE status = 'active' ORDER BY name LIMIT 10000",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"users": rows.into_iter().map(|(id, name, slug)|
        json!({"id": id, "name": name, "slug": slug})).collect::<Vec<_>>()}))
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// `GET /config` — what the admin client needs to know about this install.
///
/// Deliberately only the facts a signed-in staff member may see about the
/// server's shape. No keys, no secrets, no paths.
async fn config(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Contributor).await {
        return r;
    }
    // `labs` goes out with the shipped flags overlaid, the way Ghost's
    // `labs.getAll()` does — a client that branches on `editorReact` must see
    // it as true whatever the row says, because it is no longer a switch.
    let mut labs = match state.settings.get("labs") {
        Some(Value::Object(m)) => m,
        Some(Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => serde_json::Map::new(),
    };
    for flag in ghost_core::LABS_GA_FLAGS {
        labs.insert((*flag).to_string(), json!(true));
    }
    // Ghost derives this one rather than storing it.
    labs.insert(
        "members".into(),
        json!(state.settings.get_str("members_signup_access").as_deref() != Some("none")),
    );

    let response = json!({"config": {
        "version": ghost_core::GHOST_VERSION,
        "environment": state.config.env,
        "database": state.config.database.client,
        "mail": ghost_mail::MailConfig::from_settings(
            &state.settings,
            &state.settings.get_str("title").unwrap_or_default(),
            &state.site_url(),
        ).transport_name(),
        "useGravatar": false,
        "labs": Value::Object(labs),
        "stripeDirect": false,
        "emailAnalytics": state.settings.get_bool("email_track_opens"),
        // What the host has decided about this site: which limits are in
        // force, and where to go about them. Null on a self-hosted install,
        // which is how a client tells the two situations apart.
        "hostSettings": host_settings(&state),
        // Web analytics is served from this port's own `page_views` table
        // rather than from a column store, so the descriptor Ghost uses to
        // point its admin at Tinybird points at this server instead. `id` is
        // the site's own uuid, as Ghost's is.
        "stats": {
            "endpoint": format!("{}/ghost/api/admin/stats", state.site_url()),
            "id": state.settings.get_str("site_uuid"),
            "provider": "native",
        },
    }});
    private(response)
}

/// The host-settings slice a signed-in staff member may see.
///
/// Built field by field rather than passed through, because `hostSettings` is
/// where a hosting platform keeps things that are its business and not the
/// site's — the limits are published so the admin screens can grey out what
/// the plan does not include, and nothing else is.
fn host_settings(state: &AppState) -> Value {
    let host = &state.config.host_settings;
    if host.is_null() {
        return Value::Null;
    }
    let mut limits = serde_json::Map::new();
    for name in state.limits.names() {
        limits.insert(name.to_string(), json!({"disabled": state.limits.is_disabled(name)}));
    }
    json!({
        "billing": {
            "enabled": host.pointer("/billing/enabled").cloned().unwrap_or(json!(false)),
            "url": host.pointer("/billing/url").cloned().unwrap_or(Value::Null),
        },
        "limits": Value::Object(limits),
    })
}

/// `GET`/`PUT /custom_theme_settings` — the settings a theme declares.
///
/// Ghost keys these by the active theme, and so does this: a setting from a
/// theme that is no longer active is kept but not served, so switching away
/// and back does not lose the values.
async fn read_theme_settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let theme = crate::themes::active_theme_name(&state);
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT id, key, type, value FROM custom_theme_settings WHERE theme = ? ORDER BY key",
    )
    .bind(&theme)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"custom_theme_settings": rows.into_iter().map(|(id, key, kind, value)|
        json!({"id": id, "key": key, "type": kind, "value": value})).collect::<Vec<_>>()}))
}

async fn edit_theme_settings(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let theme = crate::themes::active_theme_name(&state);
    let incoming = body.get("custom_theme_settings").and_then(Value::as_array).cloned().unwrap_or_default();
    for setting in &incoming {
        let Some(key) = setting.get("key").and_then(Value::as_str) else { continue };
        // Only a key the active theme declares: this is the theme's own
        // vocabulary, not a general-purpose store.
        let declared: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM custom_theme_settings WHERE theme = ? AND key = ?)")
                .bind(&theme)
                .bind(key)
                .fetch_one(&state.db)
                .await
                .unwrap_or(false);
        if !declared {
            return error(StatusCode::UNPROCESSABLE_ENTITY, &format!("The active theme has no setting named {key}."));
        }
        let value = match setting.get("value") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Null) | None => None,
            Some(other) => Some(other.to_string()),
        };
        let _ = sqlx::query("UPDATE custom_theme_settings SET value = ? WHERE theme = ? AND key = ?")
            .bind(value)
            .bind(&theme)
            .bind(key)
            .execute(&state.db)
            .await;
    }
    read_theme_settings(State(state), headers).await
}

/// `GET /settings/routes/yaml` — the current routing configuration, as the/// `PUT /settings/verifications` — Ghost's endpoint for confirming a
/// settings change that needed an emailed token.
async fn verifications(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    if token.is_empty() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A verification token is required.");
    }
    // The only setting this port gates behind a token is the members support
    // address, which is verified where it is changed. Anything else has
    // nothing to confirm, and saying so is better than pretending.
    error(StatusCode::NOT_FOUND, "There is no pending change for that token.")
}
/// `POST /slack/test` — send the test message the Slack settings screen has
/// a button for.
async fn slack_test(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(cfg) = crate::slack::SlackConfig::from_settings(&state.settings) else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "No Slack webhook URL is configured.");
    };
    let title = state.settings.get_str("title").unwrap_or_default();
    match crate::slack::send_test(&cfg, &title, &state.site_url()).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
    }
}

// ---------------------------------------------------------------------------
// staff
// ---------------------------------------------------------------------------

async fn user_by_email(State(state): State<AppState>, headers: HeaderMap, Path(email): Path<String>) -> Response {
    staff_lookup(&state, &headers, "email", &email).await
}

async fn user_by_slug(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    staff_lookup(&state, &headers, "slug", &slug).await
}

async fn staff_lookup(state: &AppState, headers: &HeaderMap, column: &str, value: &str) -> Response {
    if let Err(r) = permissions::require(state, headers, Role::Author).await {
        return r;
    }
    // The column is one of two literals chosen here, not by the request.
    let column = match column {
        "email" => "email",
        _ => "slug",
    };
    let found: Option<String> = sqlx::query_scalar(&format!("SELECT id FROM users WHERE {column} = ?"))
        .bind(value)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(id) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match ghost_db::repos::users::find_by_id(&state.db, &id).await {
        Ok(Some(user)) => private(json!({"users": [crate::admin::admin_user_json(&user)]})),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET`/`PUT /users/{id}/token` — a staff member's personal API token.
///
/// Only your own: a token is a credential, and one administrator reading
/// another's would be a way to act as them indefinitely.
async fn read_staff_token(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match permissions::require(&state, &headers, Role::Author).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    if user.id != id {
        return error(StatusCode::FORBIDDEN, "You can only read your own token.");
    }
    match staff_token(&state, &id).await {
        Some(token) => private(json!({"apiKey": {"id": id, "secret": token}})),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn regenerate_staff_token(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match permissions::require(&state, &headers, Role::Author).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    if user.id != id {
        return error(StatusCode::FORBIDDEN, "You can only regenerate your own token.");
    }
    let secret = ids::random_hex(32);
    let now = dates::now_db();
    // `user_id` is not unique on `api_keys` — an integration's keys live in
    // the same table — so the old one is removed rather than upserted. In a
    // transaction, because a staff member with no token between the two
    // statements would be a staff member whose integration stops working.
    let Ok(mut tx) = state.db.begin().await else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The token could not be created.");
    };
    let replaced = sqlx::query("DELETE FROM api_keys WHERE user_id = ? AND type = 'admin'")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .and(
            sqlx::query("INSERT INTO api_keys (id, type, secret, user_id, created_at, updated_at) VALUES (?, 'admin', ?, ?, ?, ?)")
                .bind(ids::new_id())
                .bind(&secret)
                .bind(&id)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await,
        );
    if replaced.is_err() || tx.commit().await.is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The token could not be created.");
    }
    match staff_token(&state, &id).await {
        Some(token) => private(json!({"apiKey": {"id": id, "secret": token}})),
        None => error(StatusCode::INTERNAL_SERVER_ERROR, "The token could not be created."),
    }
}

async fn staff_token(state: &AppState, user_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT secret FROM api_keys WHERE user_id = ? AND type = 'admin' LIMIT 1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------
// members
// ---------------------------------------------------------------------------

async fn label_by_slug(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, name, slug FROM labels WHERE slug = ?").bind(&slug).fetch_optional(&state.db).await.ok().flatten();
    match row {
        Some((id, name, slug)) => private(json!({"labels": [{"id": id, "name": name, "slug": slug}]})),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /members/stripe_connect` — whether paid membership is wired up.
///
/// Ghost's endpoint returns the Connect onboarding URL. Connect itself is
/// deferred in this port, so this reports what is actually true: whether
/// direct keys are configured, and in which mode.
async fn stripe_connect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let secret = state.settings.get_str("stripe_secret_key").unwrap_or_default();
    private(json!({"stripe_connect": {
        "connected": !secret.is_empty(),
        "mode": if secret.is_empty() {
            Value::Null
        } else if secret.starts_with("sk_live") {
            json!("live")
        } else {
            json!("test")
        },
        // Ghost's hosted Connect flow is not offered here, and saying so is
        // more use to a client than an onboarding URL that goes nowhere.
        "onboarding_url": Value::Null,
    }}))
}

/// `DELETE /members/{id}/sessions` — sign a member out everywhere.
///
/// A member's session is the `transient_id` their cookie carries, so moving
/// it invalidates every cookie at once.
async fn end_member_sessions(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let result = sqlx::query("UPDATE members SET transient_id = ?, updated_at = ? WHERE id = ?")
        .bind(ids::uuid_v4())
        .bind(dates::now_db())
        .bind(&id)
        .execute(&state.db)
        .await;
    match result {
        Ok(result) if result.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "The member could not be signed out."),
    }
}

/// `GET /feedback/{id}` — what members said about one post.
async fn post_feedback(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let (more, less): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(score = 1),0), COALESCE(SUM(score = 0),0) FROM members_feedback WHERE post_id = ?",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap_or((0, 0));
    private(json!({"feedback": [{"post_id": id, "more_like_this": more, "less_like_this": less}]}))
}

/// `PUT /links/bulk` — retarget many newsletter links at once, which is how
/// a wrong URL in a sent email gets fixed.
async fn links_bulk(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String,String>>, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let filter = q.get("filter").map(String::as_str).or_else(||body.pointer("/bulk/meta/filter").and_then(Value::as_str)).unwrap_or("");
    let to = body.pointer("/bulk/meta/link/to").or_else(||body.pointer("/bulk/meta/to")).or_else(||body.pointer("/bulk/action/to")).and_then(Value::as_str).unwrap_or("");
    if !reqwest::Url::parse(to).is_ok_and(|url|matches!(url.scheme(),"http"|"https")) {
        return error(StatusCode::UNPROCESSABLE_ENTITY,"An HTTP or HTTPS destination is required.");
    }
    let expression=match ghost_nql::parse(filter) { Ok(e)=>e,Err(_)=>return error(StatusCode::UNPROCESSABLE_ENTITY,"A valid link filter is required") };
    let rows=match sqlx::query_as::<_,(String,String,String,String)>("SELECT id,post_id,from_url,to_url FROM newsletter_links").fetch_all(&state.db).await { Ok(rows)=>rows,Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
    let selected:Vec<_>=rows.into_iter().filter(|(id,post,from,target)|ghost_nql::evaluate(&expression,&json!({"id":id,"post_id":post,"from":from,"to":target}))).collect();
    let mut tx=match state.db.begin().await {Ok(tx)=>tx,Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response()};
    for (id,_,_,_) in &selected {
        if sqlx::query("UPDATE newsletter_links SET to_url=?,updated_at=? WHERE id=?").bind(to).bind(dates::now_db()).bind(id).execute(&mut *tx).await.is_err(){return StatusCode::INTERNAL_SERVER_ERROR.into_response()}
    }
    if tx.commit().await.is_err(){return StatusCode::INTERNAL_SERVER_ERROR.into_response()}
    private(json!({"bulk":{"meta":{"stats":{"successful":selected.len()}}}}))

}

// ---------------------------------------------------------------------------
// recommendations
// ---------------------------------------------------------------------------

async fn recommendation_rows(state: &AppState, id: Option<&str>) -> Vec<Value> {
    sqlx::query_as::<_, (String, String, Option<String>, Option<String>, bool, bool, String)>(
        "SELECT id, url, title, description, one_click_subscribe, featured, created_at \
         FROM recommendations WHERE (? IS NULL OR id = ?) ORDER BY created_at DESC LIMIT 200",
    )
    .bind(id)
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, url, title, description, one_click, featured, created_at)| {
        json!({"id": id, "url": url, "title": title, "description": description,
               "one_click_subscribe": one_click, "featured": featured, "created_at": created_at})
    })
    .collect()
}

async fn browse_recommendations(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    private(json!({"recommendations": recommendation_rows(&state, None).await}))
}

async fn read_recommendation(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let rows = recommendation_rows(&state, Some(&id)).await;
    if rows.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    private(json!({"recommendations": rows}))
}

async fn add_recommendation(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let incoming = body.pointer("/recommendations/0").cloned().unwrap_or(Value::Null);
    let Some(url) = incoming.get("url").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty()) else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A URL is required.");
    };
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That is not a valid URL.");
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A recommendation must be an HTTP or HTTPS URL.");
    }
    let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recommendations WHERE url = ?)")
        .bind(url)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
    if taken {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That site is already recommended.");
    }
    let id = ids::new_id();
    let now = dates::now_db();
    let result = sqlx::query(
        "INSERT INTO recommendations (id, url, title, description, one_click_subscribe, featured, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(url)
    .bind(incoming.get("title").and_then(Value::as_str).unwrap_or(parsed.host_str().unwrap_or(url)))
    .bind(incoming.get("description").and_then(Value::as_str).unwrap_or(""))
    .bind(incoming.get("one_click_subscribe").and_then(Value::as_bool).unwrap_or(false))
    .bind(incoming.get("featured").and_then(Value::as_bool).unwrap_or(false))
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    if result.is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The recommendation could not be saved.");
    }
    // Tell them, the same way the admin screen does.
    crate::mentions::announce_recommendation(&state, url.to_string());
    (StatusCode::CREATED, Json(json!({"recommendations": recommendation_rows(&state, Some(&id)).await}))).into_response()
}

async fn edit_recommendation(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let incoming = body.pointer("/recommendations/0").cloned().unwrap_or(Value::Null);
    let result = sqlx::query(
        "UPDATE recommendations SET \
           title = COALESCE(?, title), description = COALESCE(?, description), \
           one_click_subscribe = COALESCE(?, one_click_subscribe), featured = COALESCE(?, featured), \
           updated_at = ? WHERE id = ?",
    )
    .bind(incoming.get("title").and_then(Value::as_str))
    .bind(incoming.get("description").and_then(Value::as_str))
    .bind(incoming.get("one_click_subscribe").and_then(Value::as_bool))
    .bind(incoming.get("featured").and_then(Value::as_bool))
    .bind(dates::now_db())
    .bind(&id)
    .execute(&state.db)
    .await;
    match result {
        Ok(result) if result.rows_affected() > 0 => {
            private(json!({"recommendations": recommendation_rows(&state, Some(&id)).await}))
        }
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "The recommendation could not be saved."),
    }
}

async fn delete_recommendation(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match sqlx::query("DELETE FROM recommendations WHERE id = ?").bind(&id).execute(&state.db).await {
        Ok(result) if result.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "The recommendation could not be removed."),
    }
}

/// `POST /recommendations/check` — read a candidate site's own description,
/// so the admin can fill the form in before saving.
async fn check_recommendation(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let url = body.pointer("/recommendations/0/url").and_then(Value::as_str).unwrap_or("");
    // The same guard the webmention receiver uses: this fetches a URL the
    // caller chose.
    let Some(parsed) = crate::mentions::fetchable(url) else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A public HTTP or HTTPS URL is required.");
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .unwrap_or_default();
    let Ok(response) = client.get(parsed.clone()).send().await else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That site could not be reached.");
    };
    if !response.status().is_success() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That site could not be reached.");
    }
    let html = response.text().await.unwrap_or_default();
    let metadata = crate::mentions::metadata_of(&html);
    // Whether they already recommend this site back, which is the thing
    // worth knowing before recommending them.
    let one_click: bool = html.contains("/members/api/") || html.contains("data-portal");
    private(json!({"recommendations": [{
        "url": parsed.to_string(),
        "title": metadata.site_title.or(metadata.title),
        "excerpt": metadata.excerpt,
        "featured_image": metadata.image,
        "favicon": metadata.favicon,
        "one_click_subscribe": one_click,
    }]}))
}

// ---------------------------------------------------------------------------
// gifts and scheduling
// ---------------------------------------------------------------------------

/// `PUT /gift_links/remove_all` — invalidate every article gift link.
///
/// The publisher's "reset all links" button: anybody holding an old link
/// loses access, which is the point of it.
async fn remove_all_gift_links(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match sqlx::query("DELETE FROM article_gift_links").execute(&state.db).await {
        Ok(result) => private(json!({"gift_links": {"removed": result.rows_affected()}})),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "The gift links could not be removed."),
    }
}

/// `PUT /gifts/flush_deliveries` — send the gift emails that are due now
/// rather than waiting for the next scheduler pass.
async fn flush_gift_deliveries(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let before:i64=match sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_gift_deliveries WHERE sent_at IS NOT NULL").fetch_one(&state.db).await{Ok(n)=>n,Err(_)=>return error(StatusCode::INTERNAL_SERVER_ERROR,"Could not read gift deliveries")};
    crate::members::deliver_due_gifts(&state).await;
    let after:i64=match sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_gift_deliveries WHERE sent_at IS NOT NULL").fetch_one(&state.db).await{Ok(n)=>n,Err(_)=>return error(StatusCode::INTERNAL_SERVER_ERROR,"Could not read gift deliveries")};
    private(json!({"gifts":{"flushed":after-before}}))
}

/// `PUT /gifts/flush_reminders` — the same, for the "your gift is about to
/// run out" reminders.
async fn flush_gift_reminders(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match crate::members::send_due_gift_reminders(&state).await {
        Ok(count)=>private(json!({"gifts":{"sent":count}})),
        Err(e)=>crate::error_json(&e),
    }
}

/// `PUT /schedules/{resource}/{id}` — publish something that was scheduled.
///
/// Ghost's scheduler is a separate process that calls back in at the due
/// time. This port schedules in-process, so the endpoint exists for the
/// external caller Ghost's clients expect — and it refuses anything that is
/// not actually due, so it cannot be used to publish early.
async fn publish_scheduled(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((resource, id)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    if !matches!(resource.as_str(), "posts" | "pages") {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Only posts and pages are scheduled.");
    }
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, published_at FROM posts WHERE id = ?").bind(&id).fetch_optional(&state.db).await.ok().flatten();
    let Some((status, published_at)) = row else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if status != "scheduled" {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That post is not scheduled.");
    }
    // Ghost sends the time it believed the post was due; a mismatch means
    // the schedule moved and this call is stale.
    if let Some(expected) = q.get("published_at") {
        if published_at.as_deref().map(|at| at.replace('T', " ").replace('Z', "")).as_deref()
            != Some(expected.replace('T', " ").replace('Z', "").as_str())
        {
            return error(StatusCode::UNPROCESSABLE_ENTITY, "That schedule has changed.");
        }
    }
    let due = published_at.as_deref().is_some_and(|at| at <= dates::now_db().as_str());
    if !due {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "That post is not due yet.");
    }
    // The in-process scheduler owns the publish itself, including the email
    // and the webhooks; this only brings its due time forward to now.
    let _ = sqlx::query("UPDATE posts SET published_at = ?, updated_at = ? WHERE id = ?")
        .bind(dates::now_db())
        .bind(dates::now_db())
        .bind(&id)
        .execute(&state.db)
        .await;
    private(json!({"schedules": [{"id": id, "status": "due"}]}))
}
