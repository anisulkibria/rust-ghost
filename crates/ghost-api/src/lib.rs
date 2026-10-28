//! ghost-api: AppState, auth and the Content + Admin HTTP APIs.
pub mod admin;
pub mod admin_members;
pub mod apikey_auth;
pub mod admin_comments;
pub mod automations;
pub mod automation_designs;
pub mod automations_engine;
pub mod comments;
pub mod checkout_design;
pub mod comments_mail;
pub mod content;
pub mod images;
pub mod explore;
pub mod federation;
pub mod members;
pub mod management;
pub mod machine_payments;
pub mod newsletter_events;
pub mod newsletter_audience;
pub mod oembed;
pub mod permissions;
pub mod publishing;
pub mod password_reset;
pub mod slack;
pub mod snippets;
pub mod stripe_tiers;
pub mod state;
pub mod storage;
pub mod themes;
pub mod webhooks;

pub use state::AppState;

/// Version and invalidation headers used by SDKs and reverse proxies.
pub async fn api_headers(request: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let invalidates = matches!(*request.method(), axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::DELETE)
        && request.uri().path().split('/').any(|part| matches!(part, "posts" | "pages" | "tags" | "settings"));
    let mut response = next.run(request).await;
    response.headers_mut().insert("content-version", axum::http::HeaderValue::from_str(&format!("v{}", ghost_core::GHOST_VERSION_SAFE)).expect("valid API version"));
    if invalidates && response.status().is_success() {
        response.headers_mut().insert("x-cache-invalidate", axum::http::HeaderValue::from_static("/*"));
    }
    response
}
