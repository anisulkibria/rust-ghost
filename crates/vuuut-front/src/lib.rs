//! vuuut-front: the public site — routing → fetch → render.
mod analytics;
// Tools → Page Builder: the request-time pass, Theme Builder parts and
// maintenance mode. `pub` for the admin canvas, which expands the same way.
pub mod builder_site;
// The public directory (Tools → Directory & Classified Listings).
mod directory;
// The online store (Tools → E-commerce & Online Stores).
mod shop;
mod images;
pub mod cards;
pub mod comment_counts;
mod llms;
mod pretty_urls;
mod private;
mod render;
pub mod robots;
mod routes;
mod sitemap;
mod rss;

use axum::routing::get;
use axum::Router;
use vuuut_api::AppState;

pub use vuuut_api::themes::url_resolver;

/// Render an authorized editor snapshot through the same theme/context as /p/.
/// Authentication belongs to the caller; this function does not register a route.
pub async fn preview_snapshot(
    state: &AppState,
    post: &vuuut_core::models::Post,
    value: serde_json::Value,
) -> vuuut_core::Result<String> {
    routes::render_preview_snapshot(state,post,value,serde_json::json!({"status":"paid","tiers":post.tier_ids}),"/p/preview/").await
}

pub fn site_router(state: AppState) -> Router {
    // Store and directory addresses must be translated before the site's
    // router selects a route, so install the middleware on an outer router.
    let inner = site_routes(state.clone());
    Router::new()
        .fallback_service(inner)
        .layer(axum::middleware::from_fn_with_state(state.clone(), shop::addresses))
        .layer(axum::middleware::from_fn_with_state(state, directory::addresses))
}

fn site_routes(state: AppState) -> Router {
    Router::new()
        .merge(vuuut_api::newsletter_events::public_router())
        .merge(vuuut_api::federation::public_router())
        // form responses; inside this router so a private site's gate covers
        // them like any page
        .merge(vuuut_api::forms::public_router())
        .merge(vuuut_api::popups::public_router())
        .merge(vuuut_api::cookies::public_router())
        .merge(vuuut_api::site_search::public_router())
        .merge(vuuut_api::ads::public_router())
        .merge(vuuut_api::social_media::share_router())
        // Tools → Directory & Classified Listings: the public directory.
        .merge(directory::router())
        // Tools → E-commerce & Online Stores: the storefront at /shop/.
        .merge(shop::router())
        .route("/assets/{*path}", get(routes::theme_asset))
        .route("/content/images/{*path}", get(images::serve))
        .route("/content/media/{*path}", get(routes::content_media))
        .route("/content/files/{*path}", get(routes::content_file))
        // the Koenig card assets are served from the binary; everything else
        // under /public/ comes off disk
        .route("/public/cards.min.css", get(cards::cards_css))
        // Tools → Page Builder: the layout stylesheet, linked with `?v=` a
        // hash of its content, so it can be cached for good.
        .route("/public/vuuut-builder.css", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/css; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_core::builder::CSS,
        )}))
        .route("/public/vuuut-builder.js", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_core::builder::RUNTIME,
        )}))
        .route("/public/cards.min.js", get(cards::cards_js))
        .route("/public/staff-toolbar.js", get(|axum::extract::State(state): axum::extract::State<AppState>| async move {
            use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
            let script = include_str!("staff_toolbar.js").replace("base+'ghost/", &format!("base+'{}/", state.admin_slug()));
            ([(CONTENT_TYPE, "text/javascript; charset=utf-8"), (CACHE_CONTROL, "no-store")], script)
        }))
        .route("/public/comment-counts.min.js", get(comment_counts::comment_counts_js))
        // The language switcher dropdown `{{ghost_head}}` injects. Versioned
        // by content hash in its URL, so it can be cached for good.
        .route("/public/language-switcher.js", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_render::language_switcher::SCRIPT,
        )}))
        // Portal and Search in the admin's design, and the script that puts
        // the stylesheets into their iframes. Versioned by content hash in
        // their URLs, so they can be cached for good.
        .route("/public/vuuut-system.js", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_render::system_skin::SCRIPT,
        )}))
        .route("/public/vuuut-portal.css", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/css; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_render::system_skin::PORTAL_CSS,
        )}))
        .route("/public/vuuut-search.css", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/css; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            vuuut_render::system_skin::SEARCH_CSS,
        )}))
        .route("/public/{*path}", get(routes::public_file))
        .route("/favicon.ico", get(routes::favicon))
        .route("/sitemap.xml", get(sitemap::sitemap_index))
        // The crawler's first request. A private site answers this in the
        // gate below before this route is reached.
        .route("/robots.txt", get(robots::robots_txt))
        // the LLM index, gated on Settings → Meta data
        .route("/llms.txt", get(llms::llms_txt))
        // the access-code screen for a private site; a no-op redirect home
        // while `is_private` is off
        .route("/private/", get(private::private_page).post(private::private_login))
        .route("/private", get(private::private_page).post(private::private_login))
        .route("/", get(routes::entrypoint))
        .route("/{*path}", get(routes::entrypoint))
        // Tools → Caching & Performance: answer anonymous page views from
        // memory. Inside the analytics counter, so a cached view still
        // counts, and inside the private gate, so a private site is never
        // served from it.
        // Tools → Page Builder: the Theme Builder's header, footer and
        // popups. Inside the page cache, so a cached page carries them.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            builder_site::site_parts,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            vuuut_api::caching::page_cache,
        ))
        // Coming soon / maintenance: outside the cache, so a cached page is
        // never served while the site is closed.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            builder_site::maintenance,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            analytics::track_views,
        ))
        // One address per page, before a view is counted — so `/a-post` and
        // `/a-post/` are not two rows in the analytics either.
        .layer(axum::middleware::from_fn(pretty_urls::canonicalize))
        // Outermost, so a private site answers before anything is rendered or
        // a view is counted.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            private::guard,
        ))
        // Outside even that, so the headers are on a private site's redirect
        // and on its access-code screen as well as on a page.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        // Outermost of all: an address the firewall refuses is refused before
        // the private gate, before a view is counted, before a page is
        // rendered and before a theme is read. The whole point of a firewall
        // is that the work does not happen.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            vuuut_api::firewall::public_guard,
        ))
        // gzip, switched from Tools → Caching & Performance without a
        // restart: the predicate reads the flag the page-cache middleware
        // refreshes from settings on every request.
        .layer(
            tower_http::compression::CompressionLayer::new().gzip(true).compress_when(
                tower_http::compression::Predicate::and(tower_http::compression::DefaultPredicate::new(),
                    |_: axum::http::StatusCode,
                     _: axum::http::Version,
                     _: &axum::http::HeaderMap,
                     _: &axum::http::Extensions| vuuut_api::caching::gzip_enabled(),
                ),
            ),
        )
        .with_state(state)
}

/// The Security module's response headers on every public response.
///
/// Much shorter than the admin's set, and deliberately: see
/// [`vuuut_core::security::response_headers`]. A publication's pages are
/// embedded on purpose and its own Portal runs in an iframe, so nothing
/// framing-related belongs here, and the public site's referrer policy is the
/// SEO module's setting and is already sent by the renderer.
///
/// What is left is `X-Content-Type-Options: nosniff`, which matters most on
/// exactly this router: `/content/files/`, `/content/media/` and
/// `/content/images/` serve files somebody uploaded, from the site's own
/// origin. A `.txt` that a sniffing browser decides is HTML is stored XSS on
/// the publication, and the declared type is the only type a response should
/// have.
///
/// A header a handler set itself is never overwritten. The one that matters
/// is the public file route's `Content-Security-Policy: sandbox` on an SVG,
/// which is strictly stronger than anything here; clobbering it would be a
/// downgrade dressed as hardening.
async fn security_headers(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let headers = vuuut_api::security_settings::response_headers(
        &state,
        vuuut_core::security::Surface::Public,
    );
    let mut resp = next.run(req).await;
    for (name, value) in headers {
        let Ok(name) = axum::http::HeaderName::from_bytes(name.as_bytes()) else { continue };
        let Ok(value) = value.parse::<axum::http::HeaderValue>() else { continue };
        if !resp.headers().contains_key(&name) {
            resp.headers_mut().insert(name, value);
        }
    }
    resp
}

/// Async helper: read the current tree and wrap it.
pub async fn resolver_now(state: &vuuut_api::AppState) -> vuuut_render::helpers::UrlResolver {
    let router = state.router.read().await.clone();
    vuuut_api::themes::url_resolver_live(router, state.settings.clone())
}
