//! ghost-api: AppState, auth and the Content + Admin HTTP APIs.
pub mod admin;
pub mod announce;
pub mod admin_members;
pub mod admin_rest;
pub mod apikey_auth;
pub mod admin_comments;
pub mod automations;
pub mod automation_designs;
pub mod automations_engine;
pub mod comments;
pub mod checkout_design;
pub mod comments_mail;
pub mod content;
pub mod identity;
pub mod images;
pub mod explore;
pub mod federation;
pub mod member_events;
pub mod members;
pub mod mentions;
pub mod management;
pub mod machine_payments;
pub mod newsletter_events;
pub mod newsletter_audience;
pub mod notifications;
pub mod oembed;
pub mod permissions;
pub mod publishing;
pub mod password_reset;
pub mod slack;
pub mod snippets;
pub mod stripe_tiers;
pub mod state;
pub mod stats;
pub mod storage;
pub mod themes;
pub mod webhooks;

pub use state::AppState;

/// A [`ghost_core::GhostError`] as Ghost's JSON error envelope, at the status
/// the error itself declares.
///
/// Most of this port's handlers build their own error bodies, which is fine
/// when there is one way to fail. The limit checks are different: any create
/// path can refuse for a reason that carries `errorDetails` and a `help` link,
/// and spelling that out at each call site is how the shapes drift apart.
pub fn error_json(e: &ghost_core::GhostError) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::from_u16(e.status_code())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({ "errors": [e.to_api_json()] })),
    )
        .into_response()
}

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

pub mod compat;

pub mod automation_tracking;

pub mod automated_emails;

pub mod email_analytics;

pub mod apps;
