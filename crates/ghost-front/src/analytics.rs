//! Page-view tracking — this port's take on Ghost's `web_analytics`.
//!
//! A middleware around the public site records one row per successful HTML
//! response, tagged with a daily salted fingerprint (the basis for the
//! visitor count on the admin Analytics page). No analytics cookie is issued. Collection is gated on the
//! `web_analytics` setting; the insert is fire-and-forget so it never adds
//! latency to a page.
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use ghost_api::AppState;
use sha2::{Digest,Sha256};



pub async fn track_views(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // not tracking? don't even issue a cookie
    if !state.settings.get_bool("web_analytics") {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let method_get = req.method() == axum::http::Method::GET;
    let headers = req.headers().clone();
    if headers.get("dnt").is_some_and(|v|v=="1") || headers.get("sec-gpc").is_some_and(|v|v=="1") { return next.run(req).await; }
    let agent=headers.get(header::USER_AGENT).and_then(|v|v.to_str().ok()).unwrap_or("").to_string();
    if ["bot","crawler","spider","preview"].iter().any(|bot|agent.to_lowercase().contains(bot)) { return next.run(req).await; }
    let peer=req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|p|p.0.ip().to_string()).unwrap_or_default();
    let viewer = viewer_id(&peer,&agent,&state.settings.get_str("theme_session_secret").unwrap_or_default(),&chrono::Utc::now().format("%Y-%m-%d").to_string());
    let lower=agent.to_lowercase();
    let device=if lower.contains("ipad")||lower.contains("tablet"){ "tablet" }else if lower.contains("mobile") {"mobile"}else{"desktop"};
    let browser=if lower.contains("firefox"){ "Firefox" }else if lower.contains("edg/"){ "Edge" }else if lower.contains("chrome"){ "Chrome" }else if lower.contains("safari"){ "Safari" }else{"Other"};
    let referrer = referrer_source(&headers, &state.site_url());

    let res = next.run(req).await;
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
    {
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
                "INSERT INTO page_views (id, path, viewer, referrer, audience, created_at, device, browser) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(ghost_core::ids::new_id())
            .bind(&path)
            .bind(&viewer)
            .bind(referrer.as_deref())
            .bind(audience)
            .bind(ghost_core::dates::now_db())
            .bind(device).bind(browser)
            .execute(&db)
            .await;
        });

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

/// Daily salted fingerprint. Neither the IP nor a persistent browser id is stored.
fn viewer_id(peer:&str,agent:&str,secret:&str,day:&str)->String {
    hex::encode(Sha256::digest(format!("{secret}\0{day}\0{peer}\0{agent}").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_fingerprints_are_stable_and_rotate_without_cookies() {
        let same=viewer_id("192.0.2.1","Browser","secret","2026-10-28");
        assert_eq!(same,viewer_id("192.0.2.1","Browser","secret","2026-10-28"));
        assert_ne!(same,viewer_id("192.0.2.1","Browser","secret","2026-10-29"));
        assert_ne!(same,viewer_id("192.0.2.2","Browser","secret","2026-10-28"));
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

}
