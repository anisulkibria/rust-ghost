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
    // The site's own switch, and then the host's: a plan without analytics
    // means nothing is collected whatever the site has asked for.
    if !state.settings.get_bool("web_analytics") || state.limits.is_disabled("limitAnalytics") {
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
    // The source and medium the referring host resolves to, so the analytics
    // page can group "Google" and "Bing" as search rather than listing hosts.
    let (source, medium) = classify_referrer(referrer.as_deref());
    // Campaign tags off the URL the visitor arrived on. These are how a
    // publisher tells one newsletter or ad from another, and they are the
    // visitor's own link, not a third party's.
    let query = req.uri().query().unwrap_or("").to_string();
    let [utm_source, utm_medium, utm_campaign, utm_term, utm_content] = utm_tags(&query);
    // Country, from whichever CDN header is in front of the site. Nothing
    // is inferred from the IP: there is no database here to do it with, and
    // guessing would be worse than leaving it unknown.
    let country = country_of(&headers).or_else(||country_from_database(&state,&peer));

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
                "INSERT INTO page_views (id, path, viewer, referrer, audience, created_at, device, browser, \
                 referrer_source, referrer_medium, utm_source, utm_medium, utm_campaign, country, utm_term, utm_content) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(ghost_core::ids::new_id())
            .bind(&path)
            .bind(&viewer)
            .bind(referrer.as_deref())
            .bind(audience)
            .bind(ghost_core::dates::now_db())
            .bind(device).bind(browser)
            .bind(source).bind(medium)
            .bind(utm_source).bind(utm_medium).bind(utm_campaign)
            .bind(country).bind(utm_term).bind(utm_content)
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

/// The referring host as a named source and a medium.
///
/// Ghost's referrer translator maps a long list of hosts onto a handful of
/// names, so that a report reads "Search" and "Social" rather than fifty
/// hostnames. This is the same idea over the hosts that actually turn up:
/// anything unrecognised keeps its host as the source and counts as a
/// referral, which is honest and still groups.
fn classify_referrer(host: Option<&str>) -> (Option<String>, Option<&'static str>) {
    const SEARCH: &[(&str, &str)] = &[
        ("google.", "Google"), ("bing.", "Bing"), ("duckduckgo.", "DuckDuckGo"), ("yahoo.", "Yahoo"),
        ("yandex.", "Yandex"), ("baidu.", "Baidu"), ("ecosia.", "Ecosia"), ("brave.com", "Brave"),
        ("startpage.com", "Startpage"), ("search.marginalia", "Marginalia"),
    ];
    const SOCIAL: &[(&str, &str)] = &[
        ("t.co", "Twitter"), ("twitter.com", "Twitter"), ("x.com", "Twitter"),
        ("facebook.com", "Facebook"), ("fb.me", "Facebook"), ("instagram.com", "Instagram"),
        ("linkedin.com", "LinkedIn"), ("lnkd.in", "LinkedIn"), ("reddit.com", "Reddit"),
        ("news.ycombinator.com", "Hacker News"), ("bsky.app", "Bluesky"), ("mastodon.", "Mastodon"),
        ("threads.net", "Threads"), ("youtube.com", "YouTube"), ("youtu.be", "YouTube"),
        ("pinterest.", "Pinterest"), ("tiktok.com", "TikTok"), ("substack.com", "Substack"),
        ("medium.com", "Medium"), ("telegram.", "Telegram"), ("whatsapp.com", "WhatsApp"),
    ];
    let Some(host) = host else { return (None, None) };
    for (needle, name) in SEARCH {
        if host.contains(needle) {
            return (Some((*name).to_string()), Some("search"));
        }
    }
    for (needle, name) in SOCIAL {
        if host == *needle || host.ends_with(&format!(".{needle}")) || host.contains(needle) {
            return (Some((*name).to_string()), Some("social"));
        }
    }
    (Some(host.to_string()), Some("referral"))
}

/// `utm_source`, `utm_medium` and `utm_campaign` from a query string.
///
/// The other two Ghost stores — term and content — are for ad platforms and
/// have nowhere to show here, so they are not collected: a column that is
/// written and never read is a liability rather than a feature.
fn utm_tags(query: &str) -> [Option<String>; 5] {
    if query.is_empty() {
        return Default::default();
    }
    let mut found: [Option<String>; 5] = Default::default();
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        let slot = match key.as_ref() {
            "utm_source" => 0,
            "utm_medium" => 1,
            "utm_campaign" => 2,
            "utm_term" => 3,
            "utm_content" => 4,
            _ => continue,
        };
        if found[slot].is_none() {
            // Bounded: a campaign name is a label, and the column is 191.
            found[slot] = Some(value.chars().take(191).collect());
        }
    }
    found
}

/// The visitor's country, as reported by whatever is in front of the site.
///
/// Every one of these is a two-letter code set by a reverse proxy that does
/// its own geolocation. Taken in order of how specific the header is to a
/// provider, and only accepted when it looks like a country code — these
/// arrive from the network, so a header holding something else must not end
/// up in the column.
fn country_of(headers: &HeaderMap) -> Option<String> {
    const HEADERS: &[&str] = &[
        "cf-ipcountry",            // Cloudflare
        "x-vercel-ip-country",     // Vercel
        "fastly-client-country",   // Fastly
        "x-geo-country",
        "x-country-code",
    ];
    for name in HEADERS {
        let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) else { continue };
        let code = value.trim().to_ascii_uppercase();
        // Cloudflare sends XX for an address it cannot place, and T1 for Tor.
        if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) && code != "XX" {
            return Some(code);
        }
    }
    None
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
    fn referring_hosts_group_into_sources_and_mediums() {
        // A report that says "Search" is useful; one that lists nine Google
        // country domains is not.
        for host in ["google.com", "google.co.uk", "news.google.com", "bing.com"] {
            assert_eq!(classify_referrer(Some(host)).1, Some("search"), "{host}");
        }
        assert_eq!(classify_referrer(Some("google.co.uk")).0.as_deref(), Some("Google"));
        for host in ["t.co", "x.com", "twitter.com"] {
            assert_eq!(classify_referrer(Some(host)), (Some("Twitter".into()), Some("social")), "{host}");
        }
        assert_eq!(classify_referrer(Some("news.ycombinator.com")).0.as_deref(), Some("Hacker News"));
        // An unknown host keeps its own name and still groups as a referral,
        // rather than being dropped for not being on a list.
        assert_eq!(classify_referrer(Some("someblog.example")), (Some("someblog.example".into()), Some("referral")));
        assert_eq!(classify_referrer(None), (None, None));
    }

    #[test]
    fn campaign_tags_are_read_from_the_arrival_url() {
        let tags = utm_tags("utm_source=newsletter&utm_medium=email&utm_campaign=launch&other=x");
        assert_eq!(
            tags,
            [Some("newsletter".into()), Some("email".into()), Some("launch".into()), None, None]
        );
        // Empty values are not tags, and a URL without any leaves nulls.
        let none: [Option<String>; 5] = Default::default();
        assert_eq!(utm_tags("utm_source=&q=hello"), none);
        assert_eq!(utm_tags(""), none);
        // Bounded to the column's width.
        let long = format!("utm_campaign={}", "x".repeat(500));
        assert_eq!(utm_tags(&long)[2].as_deref().map(str::len), Some(191));
    }

    #[test]
    fn a_country_is_taken_only_from_a_proxy_header_that_looks_like_one() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-ipcountry", "gb".parse().unwrap());
        assert_eq!(country_of(&headers).as_deref(), Some("GB"));
        // Cloudflare's "could not place this address".
        headers.insert("cf-ipcountry", "XX".parse().unwrap());
        assert_eq!(country_of(&headers), None);
        // These arrive from the network, so anything that is not a country
        // code is refused rather than stored.
        headers.insert("cf-ipcountry", "'; DROP TABLE page_views; --".parse().unwrap());
        assert_eq!(country_of(&headers), None);
        assert_eq!(country_of(&HeaderMap::new()), None);
    }

    #[test]
    fn host_of_handles_ports_and_userinfo() {
        assert_eq!(host_of("http://localhost:2368/x").as_deref(), Some("localhost"));
        assert_eq!(host_of("https://user:pw@Example.COM/x").as_deref(), Some("example.com"));
        assert_eq!(host_of("not a url").as_deref(), Some("not a url"));
    }

}

/// Optional host-provided MMDB, loaded once. IP addresses never leave the server.
fn country_from_database(state:&AppState,peer:&str)->Option<String>{
    type Readers=std::collections::HashMap<String,std::sync::Arc<maxminddb::Reader<Vec<u8>>>>;
    static READERS:std::sync::OnceLock<std::sync::Mutex<Readers>>=std::sync::OnceLock::new();
    let path=state.config.host_settings.pointer("/analytics/geoipDatabase").and_then(serde_json::Value::as_str)?;
    let ip=peer.parse().ok()?;
    let cache=READERS.get_or_init(Default::default);
    let mut readers=cache.lock().ok()?;
    if !readers.contains_key(path){
        let reader=maxminddb::Reader::open_readfile(path).map_err(|e|tracing::warn!("Could not open geolocation database: {e}")).ok()?;
        readers.insert(path.into(),std::sync::Arc::new(reader));
    }
    readers.get(path)?.lookup(ip).ok()?.decode_path::<String>(&maxminddb::path!["country","iso_code"]).ok().flatten()
}
