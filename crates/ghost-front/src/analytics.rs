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
    let referrer = referrer_source(&headers, &state.site_url());

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
        let state = state.clone();
        tokio::spawn(async move {
            // Resolved off the request's own cookies, so the audience recorded
            // is who the visitor was at the time — a later upgrade to paid
            // does not rewrite the traffic they generated as a free member.
            let audience = match ghost_api::members::current_member(&state, &headers).await {
                Some(m) if m.status != "free" => "paid",
                Some(_) => "free",
                None => "public",
            };
            let _ = sqlx::query(
                "INSERT INTO page_views (id, path, viewer, referrer, audience, created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(ghost_core::ids::new_id())
            .bind(&path)
            .bind(&viewer)
            .bind(referrer.as_deref())
            .bind(audience)
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

/// The source a visit came from: the referring host, with `www.` dropped and
/// the site's own host treated as no referrer at all.
///
/// Only the host is kept. The admin reports on sources, not on individual
/// referring URLs, and holding the full URL would mean storing whatever query
/// string the other site happened to send — which can carry their visitor's
/// search terms or session ids.
fn referrer_source(headers: &HeaderMap, site_url: &str) -> Option<String> {
    let raw = headers.get(header::REFERER)?.to_str().ok()?;
    let host = host_of(raw)?;
    // internal navigation is not a source
    if host_of(site_url).is_some_and(|own| own == host) {
        return None;
    }
    Some(host)
}

/// The lowercased host of a URL, without `www.`.
fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#']).next()?;
    // strip any userinfo and port
    let host = host.rsplit('@').next()?.split(':').next()?;
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
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
    fn referrer_keeps_the_source_host_only() {
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, "https://www.google.com/search?q=secret+terms".parse().unwrap());
        // the query string is where a referring site leaks its own visitor's
        // search terms, so only the host survives
        assert_eq!(referrer_source(&headers, "https://example.com").as_deref(), Some("google.com"));

        headers.insert(header::REFERER, "https://t.co/abc".parse().unwrap());
        assert_eq!(referrer_source(&headers, "https://example.com").as_deref(), Some("t.co"));
    }

    #[test]
    fn referrer_ignores_the_site_itself() {
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, "https://example.com/some-post/".parse().unwrap());
        // clicking through the site is navigation, not a traffic source
        assert_eq!(referrer_source(&headers, "https://example.com"), None);
        headers.insert(header::REFERER, "https://www.example.com/x/".parse().unwrap());
        assert_eq!(referrer_source(&headers, "https://example.com"), None);
        assert_eq!(referrer_source(&HeaderMap::new(), "https://example.com"), None);
    }

    #[test]
    fn host_of_handles_ports_and_userinfo() {
        assert_eq!(host_of("http://localhost:2368/x").as_deref(), Some("localhost"));
        assert_eq!(host_of("https://user:pw@Example.COM/x").as_deref(), Some("example.com"));
        assert_eq!(host_of("not a url").as_deref(), Some("not a url"));
    }

    #[test]
    fn viewer_id_mints_a_stable_shape_without_a_cookie() {
        let headers = HeaderMap::new();
        let id = viewer_id(&headers).unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
