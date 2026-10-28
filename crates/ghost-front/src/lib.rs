//! ghost-front: the public site — routing → fetch → render.
mod analytics;
mod images;
pub mod cards;
pub mod comment_counts;
mod llms;
mod private;
mod render;
mod routes;
mod sitemap;
mod rss;

use axum::routing::get;
use axum::Router;
use ghost_api::AppState;

pub use ghost_api::themes::url_resolver;

pub fn site_router(state: AppState) -> Router {
    Router::new()
        .merge(ghost_api::newsletter_events::public_router())
        .merge(ghost_api::federation::public_router())
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
        .route("/public/{*path}", get(routes::public_file))
        .route("/favicon.ico", get(routes::favicon))
        .route("/sitemap.xml", get(sitemap::sitemap_index))
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
    url_resolver(router)
}
