//! ghost-front: the public site — routing → fetch → render.
mod analytics;
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
use ghost_api::AppState;

pub use ghost_api::themes::url_resolver;

/// Render an authorized editor snapshot through the same theme/context as /p/.
/// Authentication belongs to the caller; this function does not register a route.
pub async fn preview_snapshot(
    state: &AppState,
    post: &ghost_core::models::Post,
    value: serde_json::Value,
) -> ghost_core::Result<String> {
    routes::render_preview_snapshot(state,post,value,serde_json::json!({"status":"paid","tiers":post.tier_ids}),"/p/preview/").await
}

pub fn site_router(state: AppState) -> Router {
    Router::new()
        .merge(ghost_api::newsletter_events::public_router())
        .merge(ghost_api::federation::public_router())
        // form responses; inside this router so a private site's gate covers
        // them like any page
        .merge(ghost_api::forms::public_router())
        .route("/assets/{*path}", get(routes::theme_asset))
        .route("/content/images/{*path}", get(images::serve))
        .route("/content/media/{*path}", get(routes::content_media))
        .route("/content/files/{*path}", get(routes::content_file))
        // the Koenig card assets are served from the binary; everything else
        // under /public/ comes off disk
        .route("/public/cards.min.css", get(cards::cards_css))
        .route("/public/cards.min.js", get(cards::cards_js))
        .route("/public/staff-toolbar.js", get(||async {([(axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("staff_toolbar.js"))}))
        .route("/public/comment-counts.min.js", get(comment_counts::comment_counts_js))
        // The language switcher dropdown `{{ghost_head}}` injects. Versioned
        // by content hash in its URL, so it can be cached for good.
        .route("/public/language-switcher.js", get(||async {(
            [
                (axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL,"public, max-age=31536000, immutable"),
            ],
            ghost_render::language_switcher::SCRIPT,
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
        .with_state(state)
}

/// Async helper: read the current tree and wrap it.
pub async fn resolver_now(state: &ghost_api::AppState) -> ghost_render::helpers::UrlResolver {
    let router = state.router.read().await.clone();
    ghost_api::themes::url_resolver_live(router, state.settings.clone())
}
