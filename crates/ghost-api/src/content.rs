//! Content API (`/ghost/api/content/*`) — public reads with `?key=` auth.
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use ghost_db::repos;
use ghost_routing::RouterTree;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::AppState;

pub fn content_router() -> Router<AppState> {
    Router::new()
        .route("/posts/", get(browse_posts))
        .route("/posts/slug/{slug}", get(read_post_by_slug))
        .route("/posts/{id}", get(read_post_by_id))
        .route("/pages/", get(browse_pages))
        .route("/pages/slug/{slug}", get(read_page_by_slug))
        .route("/pages/{id}", get(read_page_by_id))
        .route("/tags/", get(browse_tags))
        .route("/tags/slug/{slug}", get(read_tag_by_slug))
        .route("/tags/{id}", get(read_tag_by_id))
        .route("/authors/", get(browse_authors))
        .route("/authors/slug/{slug}", get(read_author_by_slug))
        .route("/authors/{id}", get(read_author_by_id))
        .route("/settings/", get(read_settings))
        .route("/tiers/", get(browse_tiers))
        .route("/newsletters/", get(browse_newsletters))
        // Sodo Search's index feeds. It requests these without a trailing
        // slash tolerance of its own, so register both spellings.
        .route("/search-index/posts/", get(search_index_posts))
        .route("/search-index/posts", get(search_index_posts))
        .route("/search-index/authors/", get(search_index_authors))
        .route("/search-index/authors", get(search_index_authors))
        .route("/search-index/tags/", get(search_index_tags))
        .route("/search-index/tags", get(search_index_tags))
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

/// Content API auth: `?key=<secret>` of a content-type key. Anonymous
/// requests still see public content (Ghost's public context).
async fn authenticate(state: &AppState, params: &HashMap<String, String>) -> Result<(), Response> {
    if let Some(key) = params.get("key") {
        match repos::api_keys::find_by_secret(&state.db, key).await {
            Ok(Some(k)) if k.key_type == "content" => Ok(()),
            _ => Err(error_response(
                StatusCode::UNAUTHORIZED,
                "Unknown Content API key",
            )),
        }
    } else {
        // anonymous = public context
        Ok(())
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let body = json!({ "errors": [{ "message": message, "errorType": "UnauthorizedError" }] });
    (status, axum::Json(body)).into_response()
}

fn error_from(e: ghost_core::GhostError) -> Response {
    let status = StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = json!({ "errors": [e.to_api_json()] });
    (status, axum::Json(body)).into_response()
}

fn not_found(what: &str) -> Response {
    let body = json!({ "errors": [{ "message": format!("{what} could not be found"), "errorType": "NotFoundError" }] });
    (StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// query parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct BrowseParams {
    pub limit: i64,
    pub page: i64,
    pub order: Option<String>,
    pub filter: Option<String>,
    pub include: Vec<String>,
    pub fields: Vec<String>,
}

pub(crate) fn parse_browse(params: &HashMap<String, String>) -> BrowseParams {
    BrowseParams {
        limit: params.get("limit").map(|l| if l == "all" { -1 } else { l.parse().unwrap_or(15) }).unwrap_or(15),
        page: params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1),
        order: params.get("order").cloned(),
        filter: params.get("filter").cloned(),
        include: params.get("include").map(|i| i.split(',').map(str::to_string).collect()).unwrap_or_default(),
        fields: params.get("fields").map(|i| i.split(',').map(str::to_string).collect()).unwrap_or_default(),
    }
}

/// Strip Content-API-forbidden fields from a serialized post.
fn public_post(j: &mut Value) {
    if let Value::Object(m) = j {
        for k in [
            "plaintext", "mobiledoc", "lexical", "email_recipient_filter", "published_by",
            "newsletter_id", "locale", "email_subject", "show_title_and_feature_image",
        ] {
            m.remove(k);
        }
    }
}

/// Snapshot the router and wrap it as a URL resolver for the serializers here.
///
/// Delegates to `themes::url_resolver` rather than rebuilding the lookup:
/// that one resolves tags and authors through their taxonomy routes and adds
/// the install's base path, so the `url` an API client reads is the same URL a
/// theme would link to. A second local copy of this logic drifted from it
/// before — the API answered unprefixed paths while the site rendered
/// prefixed ones.
pub(crate) async fn resolve(state: &AppState) -> Arc<dyn Fn(&Value) -> String + Send + Sync> {
    let router: Arc<RouterTree> = state.router.read().await.clone();
    crate::themes::url_resolver(router)
}

// ---------------------------------------------------------------------------
// posts / pages
// ---------------------------------------------------------------------------

async fn browse_posts_impl(
    state: AppState,
    params: HashMap<String, String>,
    post_type: &'static str,
) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let q = parse_browse(&params);
    let filter = match q.filter.as_deref().map(ghost_nql::parse).transpose() {
        Ok(f) => f,
        Err(e) => return error_from(e),
    };

    let result = repos::posts::browse(
        &state.db,
        &repos::posts::BrowsePosts {
            filter,
            order: q.order.clone(),
            limit: q.limit,
            page: q.page,
            post_type: Some(post_type.to_string()),
            public: true,
            include_authors: true,
            include_tags: true,
        },
    )
    .await;

    match result {
        Ok(page) => {
            let resolver = resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, &page.posts).await.unwrap_or_default();
            let mut items = Vec::new();
            for p in &page.posts {
                let authors = rel.authors_for(&p.id);
                let tags = rel.tags_for(&p.id);
                let mut j = ghost_core::models::post_to_json(p, &authors, &tags, "");
                j["url"] = json!(resolver(&j));
                j["type"] = json!(post_type);
                public_post(&mut j);
                items.push(j);
            }
            let doc = if post_type == "page" { "pages" } else { "posts" };
            envelope(doc, items, &q, page.total, page.page, page.limit)
        }
        Err(e) => error_from(e),
    }
}

fn envelope(doc: &str, items: Vec<Value>, q: &BrowseParams, total: i64, page: i64, limit: i64) -> Response {
    let pagination = ghost_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
    let mut m = Map::new();
    m.insert(doc.to_string(), Value::Array(items));
    m.insert("meta".into(), json!({ "pagination": pagination }));
    if !q.fields.is_empty() {
        // minimal field filtering applied on the serialized shape
        let filtered: Vec<Value> = m.get(doc)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| {
                        let mut out = Map::new();
                        if let Value::Object(o) = item {
                            for f in &q.fields {
                                if let Some(v) = o.get(f) {
                                    out.insert(f.clone(), v.clone());
                                }
                            }
                            // url always included (Ghost)
                            if let Some(v) = o.get("url") {
                                out.insert("url".into(), v.clone());
                            }
                        }
                        Value::Object(out)
                    })
                    .collect()
            })
            .unwrap_or_default();
        m.insert(doc.to_string(), Value::Array(filtered));
    }
    ok_json(Value::Object(m))
}

fn ok_json(body: Value) -> Response {
    (
        StatusCode::OK,
        [(
            header::CACHE_CONTROL,
            "public, max-age=0, s-maxage=60, stale-while-revalidate=120",
        )],
        axum::Json(body),
    )
        .into_response()
}

async fn read_post_impl(state: AppState, params: HashMap<String, String>, by: BySelector, post_type: &'static str) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let found = match by {
        BySelector::Slug(slug) => repos::posts::find_by_slug(&state.db, &slug, post_type, true).await,
        BySelector::Id(id) => repos::posts::find_by_id(&state.db, &id).await.map(|p| p.filter(|p| p.status == "published" && p.post_type == post_type)),
    };
    match found {
        Ok(Some(post)) => {
            let resolver = resolve(&state).await;
            let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
                .await
                .unwrap_or_default();
            let authors = rel.authors_for(&post.id);
            let tags = rel.tags_for(&post.id);
            let mut j = ghost_core::models::post_to_json(&post, &authors, &tags, "");
            j["url"] = json!(resolver(&j));
            j["type"] = json!(post_type);
            public_post(&mut j);
            let mut m = Map::new();
            let doc = if post_type == "page" { "pages" } else { "posts" };
            m.insert(doc.to_string(), Value::Array(vec![j]));
            ok_json(Value::Object(m))
        }
        Ok(None) => not_found(post_type),
        Err(e) => error_from(e),
    }
}

enum BySelector {
    Slug(String),
    Id(String),
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

async fn browse_posts(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    browse_posts_impl(state, params, "post").await
}

async fn browse_pages(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    browse_posts_impl(state, params, "page").await
}

async fn read_post_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_post_impl(state, params, BySelector::Slug(slug), "post").await
}

async fn read_post_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_post_impl(state, params, BySelector::Id(id), "post").await
}

async fn read_page_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_post_impl(state, params, BySelector::Slug(slug), "page").await
}

async fn read_page_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_post_impl(state, params, BySelector::Id(id), "page").await
}

// ---------------------------------------------------------------------------
// tags
// ---------------------------------------------------------------------------

async fn browse_tags(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let q = parse_browse(&params);
    let filter = match q.filter.as_deref().map(ghost_nql::parse).transpose() {
        Ok(f) => f,
        Err(e) => return error_from(e),
    };
    let include_count = q.include.iter().any(|i| i == "count.posts");

    match repos::tags::browse(&state.db, filter.as_ref(), q.order.as_deref(), q.limit, q.page, true, include_count).await {
        Ok((tags, total)) => {
            let resolver = resolve(&state).await;
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
            envelope("tags", items, &q, total, q.page, if q.limit > 0 { q.limit } else { 15 })
        }
        Err(e) => error_from(e),
    }
}

async fn read_tag_by(state: AppState, by: BySelector, params: HashMap<String, String>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let found = match by {
        BySelector::Slug(slug) => repos::tags::find_by_slug(&state.db, &slug).await,
        BySelector::Id(id) => repos::tags::find_by_id(&state.db, &id).await,
    };
    match found {
        Ok(Some(t)) if t.visibility == "public" => {
            let resolver = resolve(&state).await;
            let mut j = ghost_core::models::tag_to_json(&t, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            ok_json(json!({ "tags": [j] }))
        }
        _ => not_found("tag"),
    }
}

async fn read_tag_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_tag_by(state, BySelector::Slug(slug), params).await
}

async fn read_tag_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_tag_by(state, BySelector::Id(id), params).await
}

// ---------------------------------------------------------------------------
// authors
// ---------------------------------------------------------------------------

async fn browse_authors(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let q = parse_browse(&params);
    let filter = match q.filter.as_deref().map(ghost_nql::parse).transpose() {
        Ok(f) => f,
        Err(e) => return error_from(e),
    };
    match repos::users::list(&state.db, true).await {
        Ok(users) => {
            let resolver = resolve(&state).await;
            let mut items: Vec<Value> = users
                .iter()
                .map(|u| {
                    let mut j = ghost_core::models::user_public_json(u, None);
                    let url = resolver(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            if let Some(f) = &filter {
                items.retain(|i| ghost_nql::evaluate(f, i));
            }
            let total = items.len() as i64;
            let start = ((q.page.max(1) - 1) * q.limit.max(1)) as usize;
            let items: Vec<Value> = if q.limit < 0 {
                items
            } else {
                items.into_iter().skip(start).take(q.limit as usize).collect()
            };
            envelope("authors", items, &q, total, q.page, q.limit)
        }
        Err(e) => error_from(e),
    }
}

async fn read_author_by(state: AppState, by: BySelector, params: HashMap<String, String>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let found = match by {
        BySelector::Slug(slug) => repos::users::find_by_slug(&state.db, &slug).await,
        BySelector::Id(id) => repos::users::find_by_id(&state.db, &id).await,
    };
    match found {
        Ok(Some(u)) if u.status.as_deref() == Some("active") => {
            let resolver = resolve(&state).await;
            let mut j = ghost_core::models::user_public_json(&u, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            ok_json(json!({ "authors": [j] }))
        }
        _ => not_found("author"),
    }
}

async fn read_author_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_author_by(state, BySelector::Slug(slug), params).await
}

async fn read_author_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    read_author_by(state, BySelector::Id(id), params).await
}

// ---------------------------------------------------------------------------
// search-index: the three feeds Sodo Search builds its client-side index from
// ---------------------------------------------------------------------------

/// Sodo Search downloads each feed once per page load and indexes it in the
/// browser, so these are deliberately flat, unpaginated and trimmed to the
/// exact column set Ghost's `search-index-public` controller returns — any
/// extra field is dead weight in every visitor's download.
const SEARCH_INDEX_LIMIT: i64 = 10_000;

fn pick(j: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    if let Value::Object(o) = j {
        for k in keys {
            out.insert((*k).to_string(), o.get(*k).cloned().unwrap_or(Value::Null));
        }
    }
    Value::Object(out)
}

async fn search_index_posts(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let result = repos::posts::browse(
        &state.db,
        &repos::posts::BrowsePosts {
            filter: None,
            order: Some("updated_at DESC".into()),
            limit: SEARCH_INDEX_LIMIT,
            page: 1,
            post_type: Some("post".into()),
            public: true,
            include_authors: false,
            include_tags: false,
        },
    )
    .await;
    match result {
        Ok(page) => {
            let resolver = resolve(&state).await;
            let items: Vec<Value> = page
                .posts
                .iter()
                .map(|p| {
                    let mut j = ghost_core::models::post_to_json(p, &[], &[], "");
                    j["url"] = json!(resolver(&j));
                    pick(&j, &["id", "slug", "title", "excerpt", "url", "updated_at", "visibility"])
                })
                .collect();
            ok_json(json!({ "posts": items }))
        }
        Err(e) => error_from(e),
    }
}

async fn search_index_authors(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    match repos::users::list(&state.db, true).await {
        Ok(users) => {
            let resolver = resolve(&state).await;
            let items: Vec<Value> = users
                .iter()
                .take(SEARCH_INDEX_LIMIT as usize)
                .map(|u| {
                    let mut j = ghost_core::models::user_public_json(u, None);
                    let url = resolver(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    pick(&j, &["id", "slug", "name", "url", "profile_image"])
                })
                .collect();
            ok_json(json!({ "authors": items }))
        }
        Err(e) => error_from(e),
    }
}

async fn search_index_tags(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    // `public = true` is the repo's `visibility:public` filter, matching
    // Ghost's `filter: 'visibility:public'` here — internal (`#`) tags stay
    // out of the search index.
    match repos::tags::browse(&state.db, None, Some("updated_at DESC"), SEARCH_INDEX_LIMIT, 1, true, false).await {
        Ok((tags, _total)) => {
            let resolver = resolve(&state).await;
            let items: Vec<Value> = tags
                .iter()
                .map(|t| {
                    let mut j = ghost_core::models::tag_to_json(t, None);
                    let url = resolver(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    pick(&j, &["id", "slug", "name", "url"])
                })
                .collect();
            ok_json(json!({ "tags": items }))
        }
        Err(e) => error_from(e),
    }
}

// ---------------------------------------------------------------------------
// settings / tiers / newsletters
// ---------------------------------------------------------------------------

/// Wrap the public site settings in the Content API envelope.
///
/// `settings` is a bare object, not a one-element array. The Admin API returns
/// settings as a `{key, value}` list, but the Content API returns a single
/// flat object (Ghost's output serializer skips the array wrapper when
/// `isContentAPI`), and Portal depends on that: it does
/// `[{settings: S}] = ...; site = {...S}`. Spreading an array would file every
/// setting under the key `0`, leaving `members_signup_access` and
/// `portal_plans` undefined and the signup/signin modal empty.
fn settings_envelope(site: Value) -> Value {
    json!({ "settings": site, "meta": {} })
}

async fn read_settings(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let site = ghost_render::context::build_site(&state.settings, &state.site_url(), &state.admin_url());
    ok_json(settings_envelope(site))
}

async fn browse_tiers(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<i64>, Option<i64>)>(
        "SELECT id, name, slug, description, monthly_price, yearly_price FROM products WHERE active = 1",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0,
                "name": r.1,
                "slug": r.2,
                "description": r.3,
                "active": true,
                "type": "paid",
                "monthly_price": r.4,
                "yearly_price": r.5,
                "benefits": [],
            })
        })
        .collect();
    ok_json(json!({ "tiers": items }))
}

async fn browse_newsletters(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let rows = sqlx::query_as::<_, (String, Option<String>, String, Option<String>)>(
        "SELECT id, uuid, name, slug FROM newsletters WHERE status = 'active'",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0,
                "uuid": r.1,
                "name": r.2,
                "slug": r.3,
                "status": "active",
                "visibility": "members",
            })
        })
        .collect();
    ok_json(json!({ "newsletters": items }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_settings_are_a_flat_object_not_an_array() {
        let env = settings_envelope(json!({ "members_signup_access": "all", "portal_plans": ["free"] }));
        let settings = &env["settings"];
        assert!(settings.is_object(), "Portal spreads this value into its site object");
        // read the way Portal reads it, rather than just asserting the type
        assert_eq!(settings["members_signup_access"], json!("all"));
        assert_eq!(settings["portal_plans"], json!(["free"]));
        assert!(env["meta"].is_object());
    }

    #[test]
    fn search_index_picks_exactly_ghosts_columns() {
        // Sodo Search ships these to every visitor, so an extra field is
        // payload for everyone; a missing one breaks the result card.
        let post = json!({
            "id": "1", "slug": "s", "title": "t", "excerpt": "e", "url": "/s/",
            "updated_at": "2026-01-01", "visibility": "public",
            "html": "<p>huge</p>", "plaintext": "huge",
        });
        let picked = pick(&post, &["id", "slug", "title", "excerpt", "url", "updated_at", "visibility"]);
        let obj = picked.as_object().unwrap();
        assert_eq!(obj.len(), 7);
        assert!(!obj.contains_key("html"), "post bodies must not reach the search index");
        assert_eq!(obj["title"], json!("t"));
    }

    #[test]
    fn pick_keeps_missing_keys_as_null() {
        // an author with no profile_image must still carry the key, since
        // Sodo Search reads it positionally when rendering avatars
        let picked = pick(&json!({ "id": "1" }), &["id", "profile_image"]);
        assert_eq!(picked["profile_image"], Value::Null);
    }
}
