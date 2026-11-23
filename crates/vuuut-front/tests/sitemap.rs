//! The sitemaps as a browser and a crawler each receive them.
//!
//! The point of the arrangement under test is that those two are the *same
//! document*: one XML response, valid against the sitemaps.org schema, that a
//! browser additionally renders through `/sitemap.xsl`. So the assertions come
//! in pairs — the presentation is there, and nothing about the XML a crawler
//! parses has changed to make room for it.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use vuuut_api::AppState;
use vuuut_db::repos;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

const ORIGIN: &str = "https://example.test";

struct Fixture {
    _db: vuuut_db::TestDb,
    state: AppState,
    app: Router,
}

struct Reply {
    status: StatusCode,
    content_type: String,
    vary: String,
    body: String,
}

impl Reply {
    fn is_xml(&self) -> bool {
        self.content_type.starts_with("text/xml")
    }
    fn is_html(&self) -> bool {
        self.content_type.starts_with("text/html")
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::with_settings(&[]).await
    }

    async fn with_settings(extra: &[(&str, serde_json::Value)]) -> Self {
        let test_db = vuuut_db::pool::connect_test().await.unwrap();
        let db = test_db.pool();
        vuuut_db::seed::seed_if_empty(&db).await.unwrap();
        repos::users::setup_owner(&db, "Test owner", "owner@example.test", "unused-test-hash")
            .await
            .unwrap();
        let settings = Arc::new(vuuut_db::SettingsCache::from_values(
            [("site_url".to_string(), json!(ORIGIN)), ("title".to_string(), json!("Sitemap test"))]
                .into_iter()
                .chain(extra.iter().map(|(k, v)| (k.to_string(), v.clone()))),
        ));
        let config = serde_json::from_value(json!({
            "url": ORIGIN, "server": {},
            "database": {"connection": {"filename": ":memory:"}}, "paths": {}
        }))
        .unwrap();
        let routes = vuuut_routing::routes_yaml::parse_routes_yaml(
            vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML,
        )
        .unwrap();
        let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
        let theme_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
        let deps = vuuut_render::HelperDeps {
            pool: Arc::new(db.clone()),
            runtime: tokio::runtime::Handle::current(),
            url_resolver: vuuut_api::themes::url_resolver(router.clone()),
            asset_hashes: Arc::new(HashMap::new()),
            global_asset_hash: "test".into(),
            theme_dir: theme_dir.clone(),
            site_origin: ORIGIN.into(),
            content_api_key: String::new(),
        };
        let theme = vuuut_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
        let state = AppState {
            db,
            config: Arc::new(config),
            settings,
            theme: Arc::new(RwLock::new(Arc::new(theme))),
            router: Arc::new(RwLock::new(router)),
            redirects: Arc::new(RwLock::new(Arc::new(
                vuuut_routing::redirects::Redirects::default(),
            ))),
            setup_done: Arc::new(AtomicBool::new(true)),
            frontend_key: Arc::new(RwLock::new(None)),
            limits: Arc::new(vuuut_db::limits::Limits::default()),
        // Empty: a fixture firewall lets everybody through.
        firewall: vuuut_db::repos::firewall::RuleCache::empty(),
        };
        let app = vuuut_front::site_router(state.clone());
        Self { _db: test_db, state, app }
    }

    /// A fetch with no browser-navigation signals: curl, a crawler, a feed
    /// reader. What every sitemap consumer looks like.
    async fn get(&self, path: &str) -> Reply {
        self.fetch(path, &[]).await
    }

    /// The headers a browser sends when someone opens the URL in a tab.
    async fn navigate(&self, path: &str) -> Reply {
        self.fetch(
            path,
            &[
                ("sec-fetch-mode", "navigate"),
                ("sec-fetch-dest", "document"),
                ("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ],
        )
        .await
    }

    async fn fetch(&self, path: &str, extra: &[(&str, &str)]) -> Reply {
        let mut request = Request::builder().uri(path);
        for (name, value) in extra {
            request = request.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        };
        let content_type = header("content-type");
        let vary = header("vary");
        let body = to_bytes(response.into_body(), 10_000_000).await.unwrap();
        Reply { status, content_type, vary, body: String::from_utf8_lossy(&body).into_owned() }
    }

    /// A published post with a feature image, for the `<image:image>` block.
    async fn illustrated_post(&self, slug: &str, title: &str, image: &str) {
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, feature_image, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>hi</p>', ?, 'post', 'published', 'public', '2026-11-10 09:00:00', '2026-11-14 09:00:00', '2026-11-10 09:00:00')",
        )
        .bind(vuuut_core::ids::new_id())
        .bind(vuuut_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(image)
        .execute(&self.state.db)
        .await
        .unwrap();
    }

    /// A published post, so the posts sitemap has something in it.
    async fn post(&self, slug: &str, title: &str) {
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>hi</p>', 'post', 'published', 'public', '2026-11-10 09:00:00', '2026-11-14 09:00:00', '2026-11-10 09:00:00')",
        )
        .bind(vuuut_core::ids::new_id())
        .bind(vuuut_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .execute(&self.state.db)
        .await
        .unwrap();
    }
}

/// The assertion the whole feature has to keep: a crawler's view is unchanged.
#[tokio::test]
async fn a_crawler_gets_the_same_xml_it_always_did() {
    let f = Fixture::new().await;
    let reply = f.get("/sitemap.xml").await;

    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.is_xml(), "served as {}", reply.content_type);
    assert!(reply.body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{}", reply.body);
    // The XSLT instruction is gone for good — it is what made Chrome paint a
    // red deprecation banner over the page.
    assert!(!reply.body.contains("<?xml-stylesheet"), "the stylesheet instruction came back");

    assert!(reply.body.contains(r#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#));
    assert!(reply.body.trim_end().ends_with("</sitemapindex>"));
    for resource in ["posts", "pages", "tags", "authors"] {
        assert!(
            reply.body.contains(&format!("<loc>{ORIGIN}/sitemap-{resource}.xml</loc>")),
            "{resource} is missing from the index"
        );
    }
    // An index is an index, not a urlset.
    assert!(!reply.body.contains("<urlset"));
    assert!(!reply.body.contains("<url>"));
    // So a cache never serves one audience's copy to the other.
    assert!(reply.vary.contains("Sec-Fetch-Dest"), "Vary is {:?}", reply.vary);
    assert!(reply.vary.contains("Accept"), "Vary is {:?}", reply.vary);
}

#[tokio::test]
async fn a_resource_sitemap_is_xml_for_a_crawler_and_a_page_for_a_person() {
    let f = Fixture::new().await;
    f.post("welcome", "Welcome to the site").await;

    let xml = f.get("/sitemap-posts.xml").await;
    assert_eq!(xml.status, StatusCode::OK);
    assert!(xml.is_xml(), "served as {}", xml.content_type);
    assert!(xml.body.contains(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">"#));
    assert!(xml.body.contains(&format!("<loc>{ORIGIN}/welcome/</loc>")), "{}", xml.body);
    assert!(xml.body.contains("<lastmod>2026-11-14T"), "{}", xml.body);
    // A title has nowhere to live in a sitemap and must not appear in one.
    assert!(!xml.body.contains("Welcome to the site"), "the title leaked into the XML");

    let page = f.navigate("/sitemap-posts.xml").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.is_html(), "served as {}", page.content_type);
    assert!(page.body.starts_with("<!doctype html>"));
    // The page is the same listing, linked and titled.
    assert!(page.body.contains(&format!(r#"href="{ORIGIN}/welcome/""#)), "the address is not a link");
    assert!(page.body.contains("Welcome to the site"), "the title is not shown");
    assert!(page.body.contains("/welcome/"), "the path is not shown");
    assert!(page.body.contains("2026-11-14"), "the date is not shown");
    assert!(page.body.contains("All sitemaps"), "no way back to the index");
    assert!(!page.body.contains("<urlset"), "the page is not XML");
}

#[tokio::test]
async fn opening_the_index_in_a_browser_gives_a_page_of_links() {
    let f = Fixture::new().await;
    f.post("welcome", "Welcome").await;
    let page = f.navigate("/sitemap.xml").await;

    assert_eq!(page.status, StatusCode::OK);
    assert!(page.is_html(), "served as {}", page.content_type);
    assert!(page.body.starts_with("<!doctype html>"));
    for resource in ["posts", "pages", "tags", "authors"] {
        assert!(
            page.body.contains(&format!(r#"href="{ORIGIN}/sitemap-{resource}.xml""#)),
            "{resource} is not a link on the index page"
        );
    }
    assert!(page.body.contains("4 sitemaps"));
    // Each card says how much is in it, which the XML has no way to express.
    assert!(page.body.contains("1 address"), "the posts count is missing: {}", page.body);
    assert!(page.vary.contains("Sec-Fetch-Dest"));
}

/// An unknown sitemap is a 404 whoever asks, so the negotiation cannot be used
/// to coax a page out of a path that has no sitemap behind it.
#[tokio::test]
async fn an_unknown_sitemap_is_not_found_in_either_representation() {
    let f = Fixture::new().await;
    assert_eq!(f.get("/sitemap-nonsense.xml").await.status, StatusCode::NOT_FOUND);
    assert_eq!(f.navigate("/sitemap-nonsense.xml").await.status, StatusCode::NOT_FOUND);
}

/// The headers real crawlers send, against the real router.
///
/// `wants_html`'s own unit tests cover the matrix; this is the end-to-end
/// check that nothing between the socket and the handler changes the answer.
#[tokio::test]
async fn the_clients_that_actually_fetch_sitemaps_all_get_xml() {
    let f = Fixture::new().await;
    let clients: [(&str, &[(&str, &str)]); 4] = [
        ("curl", &[("accept", "*/*")]),
        // Googlebot's Accept names text/html, which is why Accept alone can
        // never be what decides this.
        ("googlebot", &[("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")]),
        ("a feed reader", &[("accept", "application/xml, text/xml")]),
        // A browser, but fetching rather than navigating — a prefetch or a
        // script, neither of which is someone reading the page.
        (
            "a browser subresource fetch",
            &[("sec-fetch-mode", "cors"), ("sec-fetch-dest", "empty"), ("accept", "text/html")],
        ),
    ];
    for (who, headers) in clients {
        for path in ["/sitemap.xml", "/sitemap-posts.xml"] {
            let reply = f.fetch(path, headers).await;
            assert!(reply.is_xml(), "{who} got {} from {path}", reply.content_type);
            assert!(reply.body.starts_with("<?xml"), "{who} got a non-XML body from {path}");
        }
    }
}

// ---------------------------------------------------------------------------
// robots.txt — the other half of crawler discovery, and the file that makes
// the sitemaps above findable without a search console
// ---------------------------------------------------------------------------

/// This used to 404 on every public site, which left the sitemap index
/// undiscoverable except by guessing the URL.
#[tokio::test]
async fn a_public_site_serves_a_robots_txt_naming_its_sitemap() {
    let f = Fixture::new().await;
    let reply = f.get("/robots.txt").await;

    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.content_type.starts_with("text/plain"), "served as {}", reply.content_type);
    assert!(reply.body.contains("User-agent: *"), "{}", reply.body);
    assert!(reply.body.contains(&format!("Sitemap: {ORIGIN}/sitemap.xml")), "{}", reply.body);
    // The admin and the machinery, and nothing that is a page.
    assert!(reply.body.contains("Disallow: /ghost/"), "{}", reply.body);
    assert!(!reply.body.contains("Disallow: /\n"), "no content path may be blocked: {}", reply.body);
    // A browser that opens it gets the file, not the sitemap page treatment.
    assert!(f.navigate("/robots.txt").await.body.contains("User-agent: *"));
}

#[tokio::test]
async fn a_custom_robots_txt_replaces_the_default_entirely() {
    let f = Fixture::with_settings(&[(
        "seo_robots_txt",
        json!("User-agent: BadBot\nDisallow: /"),
    )])
    .await;
    let reply = f.get("/robots.txt").await;

    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.starts_with("User-agent: BadBot"), "{}", reply.body);
    // Nothing of the default survives — including the Sitemap line, which is
    // the site's to put back. A merged file would be a file nobody can
    // predict from the admin.
    assert!(!reply.body.contains("Sitemap:"), "{}", reply.body);
    // A directive on an unterminated last line is ignored by some crawlers.
    assert!(reply.body.ends_with('\n'), "{:?}", reply.body);
}

/// Feature images are how a site's pictures get into image search. Ghost puts
/// them in the sitemap; this did not until the SEO pass.
#[tokio::test]
async fn a_feature_image_is_listed_with_the_page_it_illustrates() {
    let f = Fixture::new().await;
    f.illustrated_post("welcome", "Welcome", "/content/images/2026/11/hero.jpg").await;

    let xml = f.get("/sitemap-posts.xml").await;
    assert!(xml.body.contains(r#"xmlns:image="http://www.google.com/schemas/sitemap-image/1.1""#), "{}", xml.body);
    assert!(
        xml.body.contains(&format!("<image:loc>{ORIGIN}/content/images/2026/11/hero.jpg</image:loc>")),
        "{}",
        xml.body
    );
    assert!(xml.body.contains("<image:caption>hero.jpg</image:caption>"), "{}", xml.body);

    // A site that would rather not list them can say so.
    let off = Fixture::with_settings(&[("seo_sitemap_images", json!(false))]).await;
    off.illustrated_post("welcome", "Welcome", "/content/images/2026/11/hero.jpg").await;
    let xml = off.get("/sitemap-posts.xml").await;
    assert!(!xml.body.contains("<image:image>"), "{}", xml.body);
    assert!(xml.body.contains("<loc>"), "the page itself is still listed: {}", xml.body);
}

/// A crawler can tell from the index alone whether a sub-sitemap is worth
/// refetching.
#[tokio::test]
async fn the_index_dates_each_sub_sitemap() {
    let f = Fixture::new().await;
    f.post("welcome", "Welcome").await;
    let reply = f.get("/sitemap.xml").await;
    // The *shape*, not today's date: this assertion used to name
    // `2026-11-14` and started failing on 2026-11-15, when the calendar
    // rolled over rather than when anything changed. What the test is for is
    // that the index carries a `lastmod` per entry at all.
    let dated = regex::Regex::new(&format!(
        r"<loc>{}/sitemap-posts\.xml</loc><lastmod>\d{{4}}-\d{{2}}-\d{{2}}T",
        regex::escape(ORIGIN),
    ))
    .unwrap();
    assert!(dated.is_match(&reply.body), "{}", reply.body);
}
