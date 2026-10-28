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
        .route("/posts", get(browse_posts))
        .route("/posts/slug/{slug}", get(read_post_by_slug))
        .route("/posts/slug/{slug}/", get(read_post_by_slug))
        .route("/posts/{id}", get(read_post_by_id))
        .route("/posts/{id}/", get(read_post_by_id))
        .route("/pages/", get(browse_pages))
        .route("/pages", get(browse_pages))
        .route("/pages/slug/{slug}", get(read_page_by_slug))
        .route("/pages/slug/{slug}/", get(read_page_by_slug))
        .route("/pages/{id}", get(read_page_by_id))
        .route("/pages/{id}/", get(read_page_by_id))
        .route("/tags/", get(browse_tags))
        .route("/tags", get(browse_tags))
        .route("/tags/slug/{slug}", get(read_tag_by_slug))
        .route("/tags/slug/{slug}/", get(read_tag_by_slug))
        .route("/tags/{id}", get(read_tag_by_id))
        .route("/tags/{id}/", get(read_tag_by_id))
        .route("/authors/", get(browse_authors))
        .route("/authors", get(browse_authors))
        .route("/authors/slug/{slug}", get(read_author_by_slug))
        .route("/authors/slug/{slug}/", get(read_author_by_slug))
        .route("/authors/{id}", get(read_author_by_id))
        .route("/authors/{id}/", get(read_author_by_id))
        .route("/settings/", get(read_settings))
        .route("/settings", get(read_settings))
        .route("/tiers/", get(browse_tiers))
        .route("/tiers", get(browse_tiers))
        .route("/newsletters/", get(browse_newsletters))
        .route("/newsletters", get(browse_newsletters))
        // Portal reads an offer and the recommendation list through the
        // Content API, not `/members/api` — `#/portal/offers/{id}` (where an
        // offer share link lands) and `#/portal/recommendations` both 404
        // without these. Registered with and without the trailing slash
        // because Portal builds the URL by concatenation.
        .route("/offers/{id}/", get(read_offer))
        .route("/offers/{id}", get(read_offer))
        .route("/recommendations/", get(browse_recommendations))
        .route("/recommendations", get(browse_recommendations))
        // Sodo Search's index feeds. It requests these without a trailing
        // slash tolerance of its own, so register both spellings.
        .route("/search-index/posts/", get(search_index_posts))
        .route("/search-index/posts", get(search_index_posts))
        .route("/search-index/authors/", get(search_index_authors))
        .route("/search-index/authors", get(search_index_authors))
        .route("/search-index/tags/", get(search_index_tags))
        .route("/search-index/tags", get(search_index_tags))
        .layer(axum::middleware::from_fn(crate::api_headers))
        .layer(tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([axum::http::Method::GET, axum::http::Method::HEAD, axum::http::Method::OPTIONS])
            .allow_headers(tower_http::cors::Any)
            .expose_headers([axum::http::HeaderName::from_static("content-version")])
            .max_age(std::time::Duration::from_secs(86_400)))
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

/// Content API auth: a publishable `?key=<secret>` of a content-type key.
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
        Err(error_response(StatusCode::UNAUTHORIZED, "Content API key is required"))
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
    pub formats: Vec<String>,
}

pub(crate) fn parse_browse(params: &HashMap<String, String>) -> BrowseParams {
    BrowseParams {
        limit: params.get("limit").map(|l| if l == "all" { -1 } else { l.parse::<i64>().ok().filter(|n| *n > 0).unwrap_or(15).min(100) }).unwrap_or(15),
        page: params.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).clamp(1, 10_000_000),
        order: params.get("order").cloned(),
        filter: params.get("filter").cloned(),
        include: params.get("include").map(|i| i.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
        formats: params.get("formats").or_else(|| params.get("fields")).map(|s| s.split(',').map(str::trim).map(str::to_string).collect()).unwrap_or_else(|| vec!["html".into()]),
        fields: params.get("fields").map(|i| i.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
    }
}

/// Strip Content-API-forbidden fields from a serialized post.
fn public_post(j: &mut Value) {
    if let Value::Object(m) = j {
        for k in [
            "mobiledoc", "lexical", "markdown", "status", "type", "email_recipient_filter", "published_by",
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
    let resolver = crate::themes::url_resolver(router);
    let site = state.site_url();
    Arc::new(move |value| ghost_core::url::absolute(&site, &resolver(value)))
}

/// Apply fields consistently to both read and browse responses.
pub(crate) fn select_fields(value: &mut Value, q: &BrowseParams) {
    if !q.fields.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.retain(|key, _| q.fields.contains(key));
        }
    }
}

pub use ghost_core::models::public_html;

fn content_post(
    post: &ghost_core::models::Post,
    authors: &[ghost_core::models::User],
    tags: &[ghost_core::models::Tag],
    q: &BrowseParams,
    site_url: &str,
    resolver: &dyn Fn(&Value) -> String,
) -> Value {
    let public_tags: Vec<_> = tags.iter().filter(|tag| tag.visibility == "public").cloned().collect();
    let mut value = ghost_core::models::post_to_json(post, authors, &public_tags, "");
    value["url"] = json!(resolver(&value));
    // Public Content keys never grant access to a members-only body.
    let access = post.visibility == "public";
    let html = public_html(post);
    let plain = ghost_render::markdown::html_to_plaintext(&html);
    value["access"] = json!(access);
    value["html"] = json!(ghost_core::url::absolutize_html(site_url, &html));
    value["plaintext"] = json!(plain);
    if !access || html != post.html.as_deref().unwrap_or_default() {
        value["excerpt"] = json!(if html.is_empty() { String::new() } else {
            post.custom_excerpt.clone().unwrap_or_else(|| ghost_core::models::excerpt_from_plaintext(Some(&plain), 50).unwrap_or_default())
        });
    }
    for relation in ["authors", "tags"] {
        if q.include.iter().any(|include| include == relation) {
            if let Some(items) = value[relation].as_array_mut() {
                for item in items {
                    item["url"] = json!(resolver(item));
                    absolute_images(item, site_url);
                }
            }
            let primary = if relation == "authors" { "primary_author" } else { "primary_tag" };
            value[primary] = value[relation].as_array().and_then(|items| items.first()).cloned().unwrap_or(Value::Null);
        } else if let Some(object) = value.as_object_mut() {
            object.remove(relation);
            object.remove(if relation == "authors" { "primary_author" } else { "primary_tag" });
        }
    }
    absolute_images(&mut value, site_url);
    public_post(&mut value);
    for format in ["html", "plaintext"] {
        if !q.formats.iter().any(|f| f == format) {
            value.as_object_mut().unwrap().remove(format);
        }
    }
    select_fields(&mut value, q);
    value
}

fn absolute_images(value: &mut Value, site: &str) {
    for key in ["feature_image", "profile_image", "cover_image", "og_image", "twitter_image"] {
        if let Some(path) = value.get(key).and_then(Value::as_str) {
            value[key] = json!(ghost_core::url::absolute(site, path));
        }
    }
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
                let j = content_post(p, &authors, &tags, &q, &state.site_url(), &*resolver);
                items.push(j);
            }
            let doc = if post_type == "page" { "pages" } else { "posts" };
            envelope(doc, items, &q, page.total, page.page, page.limit)
        }
        Err(e) => error_from(e),
    }
}

fn envelope(doc: &str, items: Vec<Value>, q: &BrowseParams, total: i64, page: i64, limit: i64) -> Response {
    let pagination = ghost_core::models::pagination_json(page, if q.limit < 0 { total.max(1) } else if limit > 0 { limit } else { 15 }, total);
    let mut m = Map::new();
    m.insert(doc.to_string(), Value::Array(items));
    m.insert("meta".into(), json!({ "pagination": pagination }));
    if let Some(items) = m.get_mut(doc).and_then(Value::as_array_mut) {
        for item in items { select_fields(item, q); }
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
            let q = parse_browse(&params);
            let j = content_post(&post, &authors, &tags, &q, &state.site_url(), &*resolver);
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
                    absolute_images(&mut j, &state.site_url());
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
            absolute_images(&mut j, &state.site_url());
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            let q = parse_browse(&params);
            if q.include.iter().any(|include| include == "count.posts") {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags pt JOIN posts p ON p.id = pt.post_id WHERE pt.tag_id = ? AND p.status = 'published' AND p.type = 'post'")
                    .bind(&t.id).fetch_one(&state.db).await.unwrap_or(0);
                j["count"] = json!({"posts": count});
            }
            select_fields(&mut j, &q);
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

async fn author_post_counts(state: &AppState) -> Result<HashMap<String, i64>, ghost_core::GhostError> {
    sqlx::query_as::<_, (String, i64)>("SELECT pa.author_id, COUNT(DISTINCT p.id) FROM posts_authors pa JOIN posts p ON p.id = pa.post_id WHERE p.status = 'published' AND p.type = 'post' GROUP BY pa.author_id")
        .fetch_all(&state.db).await.map(|rows| rows.into_iter().collect())
        .map_err(|error| ghost_core::GhostError::Db(error.to_string()))
}

fn sort_items(items: &mut [Value], order: Option<&str>) {
    let fields: Vec<_> = order.unwrap_or("name asc").split(',').map(|part| {
        let mut words = part.split_whitespace();
        (words.next().unwrap_or("name"), words.next().is_some_and(|direction| direction.eq_ignore_ascii_case("desc")))
    }).collect();
    let at = |item: &Value, path: &str| {
        path.split('.').try_fold(item, |value, key| value.get(key)).cloned().unwrap_or(Value::Null)
    };
    items.sort_by(|left, right| {
        for (field, descending) in &fields {
            let (left, right) = (at(left, field), at(right, field));
            let comparison = match (left.as_i64(), right.as_i64()) {
                (Some(left), Some(right)) => left.cmp(&right),
                _ => left.as_str().unwrap_or("").cmp(right.as_str().unwrap_or("")),
            };
            if !comparison.is_eq() { return if *descending { comparison.reverse() } else { comparison }; }
        }
        std::cmp::Ordering::Equal
    });
}

async fn browse_authors(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let q = parse_browse(&params);
    let filter = match q.filter.as_deref().map(ghost_nql::parse).transpose() {
        Ok(f) => f,
        Err(e) => return error_from(e),
    };
    let counts = match author_post_counts(&state).await { Ok(counts) => counts, Err(error) => return error_from(error) };
    match repos::users::list(&state.db, true).await {
        Ok(users) => {
            let resolver = resolve(&state).await;
            let mut items: Vec<Value> = users
                .iter()
                .filter(|u| counts.get(&u.id).copied().unwrap_or(0) > 0)
                .map(|u| {
                    let mut j = ghost_core::models::user_public_json(u, None);
                    absolute_images(&mut j, &state.site_url());
                    if q.include.iter().any(|include| include == "count.posts") { j["count"] = json!({"posts": counts.get(&u.id).unwrap_or(&0)}); }
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
            sort_items(&mut items, q.order.as_deref());
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
        Ok(Some(u)) if u.status.as_deref() == Some("active") && u.visibility.as_deref() == Some("public") => {
            let counts = match author_post_counts(&state).await { Ok(counts) => counts, Err(error) => return error_from(error) };
            let count = counts.get(&u.id).copied().unwrap_or(0);
            if count == 0 { return not_found("author"); }
            let q = parse_browse(&params);
            let resolver = resolve(&state).await;
            let mut j = ghost_core::models::user_public_json(&u, None);
            absolute_images(&mut j, &state.site_url());
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            if q.include.iter().any(|include| include == "count.posts") { j["count"] = json!({"posts": count}); }
            select_fields(&mut j, &q);
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
                    absolute_images(&mut j, &state.site_url());
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
                    absolute_images(&mut j, &state.site_url());
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

/// Every active tier, in Ghost's `tiers` content shape.
///
/// Portal is the demanding reader here. It turns `currency` plus
/// `monthly_price`/`yearly_price` into the price objects its signup and
/// account-plan screens render, derives which plans to offer from
/// `visibility`, and needs the **free** tier in the list as well — without it
/// `portal_plans: ["free"]` is filtered away and the signup modal offers
/// nothing. A free tier carries no price, which is why those three fields are
/// dropped for it rather than sent as null.
async fn browse_tiers(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    type Row = (
        String,         // id
        String,         // name
        String,         // slug
        Option<String>, // description
        String,         // type
        String,         // visibility
        Option<String>, // currency
        Option<i64>,    // monthly_price
        Option<i64>,    // yearly_price
        i64,            // trial_days
        Option<String>, // welcome_page_url
        Option<String>, // created_at
        Option<String>, // updated_at
    );
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, name, slug, description, type, visibility, currency, monthly_price, \
         yearly_price, trial_days, welcome_page_url, created_at, updated_at \
         FROM products WHERE active = 1 ORDER BY type DESC, created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut items: Vec<Value> = Vec::with_capacity(rows.len());
    for r in rows {
        let benefits: Vec<String> = sqlx::query_scalar(
            "SELECT b.name FROM products_benefits pb JOIN benefits b ON b.id = pb.benefit_id \
             WHERE pb.product_id = ? ORDER BY pb.sort_order",
        )
        .bind(&r.0)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        let mut tier = json!({
            "id": r.0,
            "name": r.1,
            "slug": r.2,
            "description": r.3,
            "active": true,
            "type": r.4,
            "visibility": r.5,
            "trial_days": r.9,
            "welcome_page_url": r.10,
            "created_at": r.11,
            "updated_at": r.12,
            "benefits": benefits,
        });
        if r.4 != "free" {
            if let Value::Object(m) = &mut tier {
                m.insert("currency".into(), json!(r.6));
                m.insert("monthly_price".into(), json!(r.7));
                m.insert("yearly_price".into(), json!(r.8));
            }
        }
        items.push(tier);
    }
    ok_json(json!({ "tiers": items }))
}

/// Active newsletters, in Ghost's `newsletters` content shape.
///
/// `subscribe_on_signup` and `visibility` are the two that matter beyond
/// display: the first decides which boxes Portal pre-ticks on the signup and
/// account screens, the second whether a newsletter is offered to a given
/// visitor at all. Both are editable in Settings → Membership, so hard-coding
/// them silently discarded the publisher's choice. `sort_order` is what Portal
/// sorts the list by.
async fn browse_newsletters(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    type Row = (
        String,         // id
        Option<String>, // uuid
        String,         // name
        Option<String>, // description
        String,         // slug
        Option<String>, // sender_name
        Option<String>, // sender_reply_to
        String,         // visibility
        bool,           // subscribe_on_signup
        i64,            // sort_order
    );
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, uuid, name, description, slug, sender_name, sender_reply_to, visibility, \
         subscribe_on_signup, sort_order FROM newsletters WHERE status = 'active' \
         ORDER BY sort_order, created_at",
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
                "description": r.3,
                "slug": r.4,
                "sender_name": r.5,
                "sender_reply_to": r.6,
                "status": "active",
                "visibility": r.7,
                "subscribe_on_signup": r.8,
                "sort_order": r.9,
            })
        })
        .collect();
    ok_json(json!({ "newsletters": items }))
}

/// One offer, as Portal reads it on `#/portal/offers/{id}`.
///
/// This is where an offer share link ends up: the frontend turns
/// `site.com/<code>` into that hash, and Portal then asks the Content API for
/// the discount to show. An archived offer must not resolve, or a withdrawn
/// code would keep advertising its price.
async fn read_offer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let offer = repos::offers::find_by_id(&state.db, &id).await.ok().flatten();
    let Some(offer) = offer.filter(|o| o.active) else {
        return not_found("Offer");
    };
    ok_json(json!({ "offers": [crate::members::offer_public_json(&offer)] }))
}

/// The recommended publications Portal lists on `#/portal/recommendations`.
///
/// Ghost serves favicon and featured image alongside each row; this port does
/// not store either, and Portal falls back to a blank badge when they are
/// absent, so they go out as null rather than being omitted.
async fn browse_recommendations(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = authenticate(&state, &params).await {
        return resp;
    }
    let p = parse_browse(&params);
    type Row = (String, String, String, Option<String>, bool, bool, Option<String>, Option<String>);
    let mut sql = String::from(
        "SELECT id, url, title, description, one_click_subscribe, featured, created_at, updated_at \
         FROM recommendations ORDER BY featured DESC, created_at DESC",
    );
    if p.limit > 0 {
        sql.push_str(&format!(" LIMIT {} OFFSET {}", p.limit, (p.page.max(1) - 1) * p.limit));
    }
    let rows = sqlx::query_as::<_, Row>(&sql).fetch_all(&state.db).await.unwrap_or_default();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0,
                "url": r.1,
                "title": r.2,
                "description": r.3,
                "one_click_subscribe": r.4,
                "featured": r.5,
                "favicon": Value::Null,
                "featured_image": Value::Null,
                "excerpt": Value::Null,
                "created_at": r.6,
                "updated_at": r.7,
            })
        })
        .collect();
    ok_json(json!({ "recommendations": items, "meta": {} }))
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
