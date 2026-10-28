//! Site request handling: match route → fetch data → render template.
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use ghost_api::AppState;
use ghost_core::models;
use ghost_core::GhostError;
use ghost_db::repos;
use ghost_routing::tree::{RouteMatch, TaxonomyKind};
use serde_json::{json, Value};

use crate::render::{render_template, RenderCtx};
use crate::resolver_now;
use crate::rss;

fn sitemap_resource_name(path: &str) -> Option<String> {
    let name = path.strip_prefix("/sitemap-")?.strip_suffix(".xml")?;
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Some(name.to_string())
    } else {
        None
    }
}

/// A redirects.yaml answer, carrying the file's own status code: Ghost sends
/// the literal 301/302 the rule was written under
/// (dynamic-redirect-manager.js: `res.redirect(permanent ? 301 : 302)`), not
/// axum's `Redirect::permanent`/`temporary` 308/307 — the number is the point
/// of the rule, not an implementation detail.
fn redirect_response(target: String, permanent: bool) -> Response {
    let status = if permanent {
        StatusCode::MOVED_PERMANENTLY
    } else {
        StatusCode::FOUND
    };
    (status, [(header::LOCATION, target)]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_answer_with_the_status_code_their_rule_was_written_under() {
        assert_eq!(
            redirect_response("/new/".into(), true).status(),
            StatusCode::MOVED_PERMANENTLY
        );
        assert_eq!(
            redirect_response("/new/".into(), false).status(),
            StatusCode::FOUND
        );
    }
}

/// Main entrypoint for every non-asset site path.
pub async fn entrypoint(State(state): State<AppState>, uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().to_string();
    let query = uri.query().map(str::to_string);

    // 0. .md content negotiation: /my-post.md → markdown source.
    //
    // Part of what Settings → Meta data's LLM switch controls, so it is off
    // with that switch — otherwise turning the setting off would still leave
    // every post readable as plain Markdown.
    if let Some(md_path) = path.strip_suffix(".md").filter(|_| crate::llms::enabled(&state)) {
        let md_path = format!("{md_path}/");
        let router = state.router.read().await.clone();
        if let Some(ghost_routing::tree::RouteMatch::CollectionEntry { slug, .. } | ghost_routing::tree::RouteMatch::StaticPage { slug }) =
            router.match_path(&md_path)
        {
            let found = match repos::posts::find_by_slug(&state.db, &slug, "post", true).await {
                Ok(Some(p)) => Ok(Some(p)),
                _ => repos::posts::find_by_slug(&state.db, &slug, "page", true).await,
            };
            if let Ok(Some(post)) = found {
                if let Some(response)=ghost_api::machine_payments::serve(&state,&post,&uri,&headers).await{return response}
                // Markdown is a public representation, like Ghost's LLM feed:
                // browser membership must never put private bodies into it.
                let html = ghost_api::content::public_html(&post);
                let gated = post.visibility != "public";
                let status = if gated && html.trim().is_empty() { StatusCode::FORBIDDEN } else { StatusCode::OK };
                let mut source = if !gated && html == post.html.as_deref().unwrap_or("") {
                    post.markdown.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| ghost_render::markdown::html_to_plaintext(&html))
                } else { ghost_render::markdown::html_to_plaintext(&html) };
                if gated { source.push_str("\n\nThis content requires a subscription.\n"); }
                return (status, [(header::CONTENT_TYPE, "text/markdown; charset=UTF-8"),
                    (header::CACHE_CONTROL, "private, no-store"), (header::VARY, "Cookie")], source).into_response();
            }
        }
    }

    // /feed/ → /rss/ (Ghost's permanent redirect)
    if path == "/feed" || path == "/feed/" {
        // Ghost's own redirect is a literal 301 (rss-router.js), not the 308
        // axum's `Redirect::permanent` would send.
        return (
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, ghost_core::url::base("/rss/"))],
        )
            .into_response();
    }

    // 1. redirects.yaml
    {
        let redirects = state.redirects.read().await.clone();
        if let Some((target, permanent)) = redirects.match_path(&path, query.as_deref()) {
            // targets are written in site terms, so an internal one needs the
            // base path; an absolute URL to another host is left alone
            return redirect_response(ghost_core::url::base(&target), permanent);
        }
    }

    // 2. /ghost → admin (mounted separately in the main app). Only paths the
    // admin router did not claim reach this, so it is a 404 rather than a
    // redirect: bouncing to `/ghost/` from here would loop on itself.
    if path == "/ghost" || path == "/ghost/" || path.starts_with("/ghost/") {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, &path, headers).await;
    }

    // 3. per-resource sitemaps (/sitemap-posts.xml …)
    if let Some(resource) = sitemap_resource_name(&path) {
        return crate::sitemap::sitemap_resource(state, resource).await;
    }

    // 4. router tree
    let router = state.router.read().await.clone();
    let Some(route) = router.match_path(&path) else {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, &path, headers).await;
    };

    match route {
        RouteMatch::Preview { uuid, .. } => render_preview(state, uuid, &path, headers).await,
        RouteMatch::Rss { kind, name, .. } => rss::render_rss(state, kind, name, &path).await,
        RouteMatch::CollectionIndex { name, filter, order, limit, templates, data, page, .. } => {
            render_collection(state, name, filter, order, limit, templates, data, page, &path, headers).await
        }
        RouteMatch::Channel { name, filter, order, limit, templates, page, .. } => {
            render_collection(state, name, filter, order, limit, templates, None, page, &path, headers).await
        }
        RouteMatch::CollectionEntry { slug, .. } => render_entry(state, slug, &path, query.as_deref(), headers).await,
        RouteMatch::StaticPage { slug } => render_entry(state, slug, &path, query.as_deref(), headers).await,
        RouteMatch::Taxonomy { kind, slug, page, .. } => {
            render_taxonomy(state, kind, slug, page, &path, headers).await
        }
        RouteMatch::Custom { name, templates, data, content_type } => {
            render_custom(state, name, templates, data, content_type, &path, headers).await
        }
    }
}

/// Serialize a page of posts with relations resolved (one query per page).
///
/// `member` is the `@member` context, which decides each post's `access`
/// flag — a listing shows a lock on a post the visitor cannot read, and the
/// answer depends on who is asking.
async fn posts_page_json(
    state: &AppState,
    posts: &[ghost_core::models::Post],
    member: &Value,
) -> Vec<Value> {
    let rel = repos::posts::load_relations(&state.db, posts).await.unwrap_or_default();
    let resolver = resolver_now(state).await;
    let status = crate::render::member_status(member);
    let tiers = crate::render::member_tiers(member);
    posts
        .iter()
        .map(|p| {
            let authors = rel.authors_for(&p.id);
            let tags = rel.tags_for(&p.id);
            let mut j = models::post_to_json(p, &authors, &tags, "");
            j["url"] = json!(resolver(&j));
            ghost_render::util::resolve_nested_urls(&mut j, &*resolver);
            crate::render::apply_member_tier_access(&mut j, status, &tiers);
            j
        })
        .collect()
}

/// Serialize a single post with relations.
async fn post_json(
    state: &AppState,
    post: &ghost_core::models::Post,
    url: Option<&str>,
    member: &Value,
) -> Value {
    let rel: ghost_db::repos::PostRelations =
        repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
    let authors = rel.authors_for(&post.id);
    let tags = rel.tags_for(&post.id);
    let mut j = models::post_to_json(post, &authors, &tags, url.unwrap_or(""));
    let resolver = resolver_now(state).await;
    if url.is_none() {
        j["url"] = json!(resolver(&j));
    }
    ghost_render::util::resolve_nested_urls(&mut j, &*resolver);
    crate::render::apply_member_tier_access(&mut j, crate::render::member_status(member), &crate::render::member_tiers(member));
    j
}

async fn render_collection(
    state: AppState,
    name: String,
    filter: Option<ghost_nql::Expr>,
    order: Option<String>,
    limit: Option<i64>,
    templates: Vec<String>,
    data: Option<serde_yaml::Value>,
    page: i64,
    path: &str,
    headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let limit_final = limit.unwrap_or(theme.posts_per_page());

    let q = repos::posts::BrowsePosts {
        filter,
        order,
        limit: limit_final,
        page,
        post_type: Some("post".into()),
        public: true,
        include_authors: true,
        include_tags: true,
    };
    let result = match repos::posts::browse(&state.db, &q).await {
        Ok(r) => r,
        Err(e) => return error_response(e),
    };

    if page > 1 {
        let pages = (result.total + result.limit.max(1) - 1) / result.limit.max(1);
        if page > pages.max(1) {
            return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
        }
    }

    let member = crate::render::member_context(&state, &headers).await;
    let posts_json = posts_page_json(&state, &result.posts, &member).await;

    // data: entries (e.g. data: page.blog)
    let mut extra = serde_json::Map::new();
    if let Some(yaml) = &data {
        if let Some(s) = yaml.as_str() {
            if let Some(data_json) = resolve_data_entry(&state, s, &member).await {
                extra.insert(data_entry_key(s), data_json);
            }
        }
    }

    let pagination = models::pagination_json(result.page, result.limit.max(1), result.total);
    let mut context: Vec<String> = Vec::new();
    if page > 1 {
        context.push("paged".into());
    }
    if path == "/" {
        context.push("home".into());
    }
    context.push(name.clone());

    let mut root = serde_json::Map::new();
    root.insert("posts".into(), Value::Array(posts_json));
    root.insert("pagination".into(), pagination);
    root.extend(extra);

    let is_front = path == "/" && page == 1;
    let candidates = {
        let router = state.router.read().await.clone();
        router.collection_templates(&name, &templates, is_front)
    };

    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

fn data_entry_key(short: &str) -> String {
    short.split('.').next().unwrap_or("data").to_string()
}

/// Resolve a `data:` route entry (`data: page.blog`).
///
/// `member` is threaded through so a `data:`-loaded post is gated for the
/// same visitor as the page embedding it.
async fn resolve_data_entry(state: &AppState, short: &str, member: &Value) -> Option<Value> {
    let (resource, slug) = short.split_once('.')?;
    let resolver = resolver_now(state).await;
    match resource {
        "page" | "post" => {
            let post_type = if resource == "page" { "page" } else { "post" };
            let p = repos::posts::find_by_slug(&state.db, slug, post_type, true).await.ok()??;
            Some(post_json(state, &p, None, member).await)
        }
        "tag" => {
            let t = repos::tags::find_by_slug(&state.db, slug).await.ok()??;
            let mut j = models::tag_to_json(&t, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            Some(j)
        }
        "author" => {
            let u = repos::users::find_by_slug(&state.db, slug).await.ok()??;
            let mut j = models::user_public_json(&u, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            Some(j)
        }
        _ => None,
    }
}

async fn render_entry(state: AppState, slug: String, path: &str, query: Option<&str>, headers: HeaderMap) -> Response {
    // posts first, then pages (Ghost's router fallthrough)
    let (post, post_type) = match repos::posts::find_by_slug(&state.db, &slug, "post", true).await {
        Ok(Some(p)) => (p, "post"),
        _ => match repos::posts::find_by_slug(&state.db, &slug, "page", true).await {
            Ok(Some(p)) => (p, "page"),
            _ => {
                // Last chance before a 404: an offer code. Ghost hands these
                // out as plain `site.com/<code>` links and sends the visitor
                // to the homepage with Portal opened on the offer. A post or
                // page of the same slug wins, which is why this is checked
                // only once both have missed.
                if let Ok(Some(offer)) =
                    repos::offers::find_redeemable_by_code(&state.db, &slug).await
                {
                    return Redirect::to(&ghost_core::url::base(&format!(
                        "/#/portal/offers/{}",
                        offer.id
                    )))
                    .into_response();
                }
                return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
            }
        },
    };

    let member = crate::render::member_context(&state, &headers).await;
    let gift_token = form_urlencoded::parse(query.unwrap_or("").as_bytes()).find(|(key,_)|key=="gift").map(|(_,value)|value.into_owned());
    let gifted = if let Some(token) = &gift_token { ghost_api::publishing::valid_gift(&state,&post.id,token).await } else { false };
    let post_member = if gifted { json!({"status":"paid","tiers":post.tier_ids}) } else { member.clone() };
    let post_json = post_json(&state, &post, None, &post_member).await;
    let mut context = vec![post_type.to_string()];
    if post_type == "page" {
        context.push("page".into());
    }

    let mut root = serde_json::Map::new();
    if post_type == "page" {
        root.insert("page".into(), post_json.clone());
        root.insert("post".into(), post_json);
    } else {
        root.insert("post".into(), post_json);
    }

    let theme = state.theme.read().await.clone();
    let mut candidates = vec![format!("post-{slug}")];
    if let Some(ct) = &post.custom_template {
        candidates.push(ct.clone());
    }
    if post_type == "page" {
        candidates.push(format!("page-{slug}"));
        candidates.push("page".into());
    }
    candidates.push("post".into());
    candidates.push("index".into());

    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => {
            let mut response = html_response(html);
            if gift_token.is_some() {
                response.headers_mut().insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response.headers_mut().insert("referrer-policy", "no-referrer".parse().unwrap());
            }
            response
        },
        Err(e) => error_response(e),
    }
}

async fn render_taxonomy(
    state: AppState,
    kind: TaxonomyKind,
    slug: String,
    page: i64,
    path: &str,
    headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let resolver = resolver_now(&state).await;

    let (taxonomy_json, name, filter, resource_label): (Value, String, Option<ghost_nql::Expr>, &'static str) =
        match kind {
            TaxonomyKind::Tag => {
                let t = match repos::tags::find_by_slug(&state.db, &slug).await {
                    Ok(Some(t)) if t.visibility == "public" => t,
                    _ => {
                        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
                    }
                };
                let mut j = models::tag_to_json(&t, None);
                let url = resolver(&j);
                if let Value::Object(m) = &mut j {
                    m.insert("url".into(), json!(url));
                }
                (j, "tag".to_string(), ghost_nql::parse(&format!("tags:'{slug}'")).ok(), "tag")
            }
            TaxonomyKind::Author => {
                let u = match repos::users::find_by_slug(&state.db, &slug).await {
                    Ok(Some(u)) if u.status.as_deref() == Some("active") => u,
                    _ => {
                        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
                    }
                };
                let mut j = models::user_public_json(&u, None);
                let url = resolver(&j);
                if let Value::Object(m) = &mut j {
                    m.insert("url".into(), json!(url));
                }
                (j, "author".to_string(), ghost_nql::parse(&format!("authors:'{slug}'")).ok(), "author")
            }
        };

    let q = repos::posts::BrowsePosts {
        filter,
        order: None,
        limit: theme.posts_per_page(),
        page,
        post_type: Some("post".into()),
        public: true,
        include_authors: true,
        include_tags: true,
    };
    let result = match repos::posts::browse(&state.db, &q).await {
        Ok(r) => r,
        Err(e) => return error_response(e),
    };

    let member = crate::render::member_context(&state, &headers).await;
    let posts_json = posts_page_json(&state, &result.posts, &member).await;

    let pagination = models::pagination_json(result.page, result.limit.max(1), result.total);
    let mut context: Vec<String> = Vec::new();
    if page > 1 {
        context.push("paged".into());
    }
    context.push(name.clone());

    let mut root = serde_json::Map::new();
    root.insert(resource_label.into(), taxonomy_json);
    root.insert("posts".into(), Value::Array(posts_json));
    root.insert("pagination".into(), pagination);

    let candidates = vec![format!("{name}-{slug}"), name.clone(), "index".into()];
    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

async fn render_custom(
    state: AppState,
    name: String,
    templates: Vec<String>,
    data: Option<serde_yaml::Value>,
    content_type: Option<String>,
    path: &str,
    _headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let member = crate::render::member_context(&state, &_headers).await;
    let mut root = serde_json::Map::new();
    if let Some(yaml) = &data {
        if let Some(s) = yaml.as_str() {
            if let Some(d) = resolve_data_entry(&state, s, &member).await {
                root.insert(data_entry_key(s), d);
            }
        }
    }
    let mut candidates = templates.clone();
    candidates.push(name.clone());
    let rctx = RenderCtx {
        context: vec![name],
        relative_url: path.to_string(),
        member,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => {
            let mut resp = html_response(html);
            if let Some(ct) = content_type {
                if let Ok(v) = ct.parse() {
                    resp.headers_mut().insert(header::CONTENT_TYPE, v);
                }
            }
            resp
        }
        Err(e) => error_response(e),
    }
}

/// Rendered pages depend on the member session: the same URL yields "Sign in"
/// for a visitor and "Account" for a member. `Vary: Cookie` is what stops a
/// shared cache in front of this server from handing one visitor's variant to
/// another.
fn html_response(mut html: String) -> Response {
    let script=format!("<script src=\"{}\" defer></script>",ghost_core::url::base("/public/staff-toolbar.js"));
    if let Some(index)=html.rfind("</body>"){html.insert_str(index,&script);}else{html.push_str(&script);}
    (
        [(header::VARY, header::COOKIE.as_str())],
        Html(html),
    )
        .into_response()
}

async fn render_preview(state: AppState, uuid: String, path: &str, headers: HeaderMap) -> Response {
    // P1: previews are open (admin gating arrives with the admin API in P3)
    let Some(post) = repos::posts::find_by_uuid(&state.db, &uuid).await.unwrap_or(None) else {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
    };
    // passed in rather than resolved, so it needs the base path putting on
    // here the way the resolver would have
    let preview_url = ghost_core::url::base(&format!("/p/{uuid}/"));
    let member = crate::render::member_context(&state, &headers).await;
    let preview_member=json!({"status":"paid","tiers":post.tier_ids});
    let post_json = post_json(&state, &post, Some(&preview_url), &preview_member).await;

    let mut context = vec!["preview".to_string()];
    let label = if post.post_type == "page" { "page" } else { "post" };
    context.push(label.to_string());

    let mut root = serde_json::Map::new();
    if post.post_type == "page" {
        root.insert("page".into(), post_json.clone());
        root.insert("post".into(), post_json);
    } else {
        root.insert("post".into(), post_json);
    }

    let theme = state.theme.read().await.clone();
    let candidates = if post.post_type == "page" {
        vec!["page".to_string(), "post".into()]
    } else {
        vec!["post".to_string()]
    };
    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

fn error_response(e: GhostError) -> Response {
    tracing::error!("render error: {e}");
    (
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Html(format!("<h1>Something went wrong</h1><p>{}</p>", e.message())),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// static assets
// ---------------------------------------------------------------------------

fn serve_file(base: &std::path::Path, rel: &str, cache: Option<&str>) -> Response {
    let file_path = base.join(rel);
    if !file_path.starts_with(base) || !file_path.is_file() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match std::fs::read(&file_path) {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref());
            if let Some(cache) = cache {
                builder = builder.header(header::CACHE_CONTROL, cache);
            }
            builder.body(axum::body::Body::from(bytes)).unwrap()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub async fn theme_asset(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let theme = state.theme.read().await.clone();
    serve_file(&theme.dir.join("assets"), &path, Some("public, max-age=31536000, immutable"))
}

/// /content/images/... — accepts Ghost's `/size/WxH/` notation (serves the
/// original image; resizing is a later-phase enhancement).
pub(crate) fn original_image(state: &AppState, path: &str) -> Response {
    serve_file(&state.config.images_dir(), path, None)
}

pub async fn content_media(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.media_dir(), &path, None)
}

pub async fn content_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.files_dir(), &path, None)
}


pub async fn public_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.content_path().join("public"), &path, None)
}

pub async fn favicon(State(state): State<AppState>) -> Response {
    let icon = state.settings.get_str("icon").unwrap_or_default();
    if !icon.is_empty() {
        if icon.starts_with("http") {
            // Ghost redirects the favicon route with 302 (serve-favicon.js).
            return redirect_response(icon, false);
        }
        // `icon` is stored as a site path (`/content/images/x.png`), and
        // `content_path()` is already the `content/` directory — so the
        // leading `content/` segment has to come off or the lookup lands on
        // `content/content/images/…`. A value that arrived with the base path
        // on it is reduced to the stored form first.
        let cleaned = ghost_core::url::strip(&icon).trim_start_matches('/').to_string();
        let cleaned = cleaned.strip_prefix("content/").unwrap_or(&cleaned);
        let base = state.config.content_path();
        let file_path = base.join(cleaned);
        if file_path.is_file() {
            if let Ok(bytes) = std::fs::read(&file_path) {
                let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(axum::body::Body::from(bytes))
                    .unwrap();
            }
        }
    }
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="16" fill="#15171A"/><text x="16" y="22" font-family="Arial" font-size="18" fill="white" text-anchor="middle">G</text></svg>"##;
    ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
}
