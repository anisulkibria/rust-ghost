//! Page-view tracking — this port's take on Ghost's `web_analytics`.
//!
//! A middleware around the public site records one row per successful HTML
//! response, tagged with an anonymous viewer cookie (the basis for the
//! visitor count on the admin Analytics page). Collection is gated on the
//! `web_analytics` setting; the insert is fire-and-forget so it never adds
//! latency to a page.
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use ghost_api::AppState;

/// The anonymous viewer cookie Ghost-adjacent analytics counts uniques by.
const VIEWER_COOKIE: &str = "ghost_vid";

pub async fn track_views(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // not tracking? don't even issue a cookie
    if !state.settings.get_bool("web_analytics") {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let method_get = req.method() == axum::http::Method::GET;
    let headers = req.headers().clone();
    let viewer = viewer_id(&headers);

    let mut res = next.run(req).await;
    if !method_get || res.status() != StatusCode::OK {
        return res;
    }
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/html"))
        .unwrap_or(false);
    if !is_html {
        return res;
    }
    if let Some(viewer) = viewer {
        let viewer_for_cookie = viewer.clone();
        // fire-and-forget: never make the page wait on the insert
        let db = state.db.clone();
        tokio::spawn(async move {
            let _ = sqlx::query("INSERT INTO page_views (id, path, viewer, created_at) VALUES (?, ?, ?, ?)")
                .bind(ghost_core::ids::new_id())
                .bind(&path)
                .bind(&viewer)
                .bind(ghost_core::dates::now_db())
                .execute(&db)
                .await;
        });
        res.headers_mut().append(
            header::SET_COOKIE,
            format!("{VIEWER_COOKIE}={viewer_for_cookie}; Path={}/; HttpOnly; SameSite=Lax; Max-Age={}", ghost_core::url::subdir(), 60 * 60 * 24 * 365)
                .parse()
                .unwrap(),
        );
    }
    res
}

/// The viewer's existing cookie id, or a fresh one to set on the response.
fn viewer_id(headers: &HeaderMap) -> Option<String> {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(existing) = cookie.split(';').find_map(|c| c.trim().strip_prefix(&format!("{VIEWER_COOKIE}="))) {
            return Some(existing.to_string());
        }
    }
    Some(ghost_core::ids::random_hex(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_id_reuses_the_cookie_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; ghost_vid=abc123; more=2".parse().unwrap(),
        );
        assert_eq!(viewer_id(&headers).as_deref(), Some("abc123"));
    }

    #[test]
    fn viewer_id_mints_a_stable_shape_without_a_cookie() {
        let headers = HeaderMap::new();
        let id = viewer_id(&headers).unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
