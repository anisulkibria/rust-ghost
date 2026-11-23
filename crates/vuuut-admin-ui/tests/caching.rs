//! Tools → Caching & Performance end to end, through the real public router:
//! turn the page cache on in the admin, then see an anonymous page come back
//! from memory, a signed-in visitor bypass it, an edit empty it, a matching
//! ETag answered with 304, and gzip switched on without a restart. Refusals
//! first.
//!
//! One test owns the whole sequence because the cache and the content
//! generation are process-wide; separate tests running in parallel would
//! invalidate each other's pages.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

struct Fixture {
    _db: vuuut_db::TestDb,
    state: vuuut_api::AppState,
    owner: String,
}

impl Fixture {
    async fn session(&self) -> String {
        let id = vuuut_db::repos::sessions::create(&self.state.db, &self.owner).await.unwrap();
        format!("vuuut-admin-session={id}")
    }

    /// A session for somebody who is not an administrator.
    async fn author_session(&self) -> String {
        let id = vuuut_core::ids::new_id();
        sqlx::query(
            "INSERT INTO users (id, name, slug, password, email, status, created_at) \
             VALUES (?, 'An author', ?, 'unused', ?, 'active', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(format!("an-author-{id}"))
        .bind(format!("author-{id}@example.test"))
        .execute(&self.state.db)
        .await
        .unwrap();
        let role: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Author' LIMIT 1")
            .fetch_one(&self.state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
            .bind(vuuut_core::ids::new_id())
            .bind(&role)
            .bind(&id)
            .execute(&self.state.db)
            .await
            .unwrap();
        let session = vuuut_db::repos::sessions::create(&self.state.db, &id).await.unwrap();
        format!("vuuut-admin-session={session}")
    }

    /// A published post or page with a body, a visibility and tags.
    async fn post(&self, slug: &str, title: &str, body: &str, kind: &str, visibility: &str, tags: &[&str]) {
        let id = vuuut_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'published', ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(vuuut_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(format!("<p>{body}</p>"))
        .bind(body)
        .bind(kind)
        .bind(visibility)
        .execute(&self.state.db)
        .await
        .unwrap();
        for (i, tag) in tags.iter().enumerate() {
            let tag_id: Option<String> = sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?")
                .bind(tag)
                .fetch_optional(&self.state.db)
                .await
                .unwrap();
            let tag_id = match tag_id {
                Some(t) => t,
                None => {
                    let t = vuuut_core::ids::new_id();
                    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at, updated_at) VALUES (?, ?, ?, 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
                        .bind(&t)
                        .bind(tag)
                        .bind(tag)
                        .execute(&self.state.db)
                        .await
                        .unwrap();
                    t
                }
            };
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, ?)")
                .bind(vuuut_core::ids::new_id())
                .bind(&id)
                .bind(&tag_id)
                .bind(i as i64)
                .execute(&self.state.db)
                .await
                .unwrap();
        }
    }
}

async fn fixture() -> Fixture {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Cache test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}}, "paths": {}
    }))
    .unwrap();
    let routes =
        vuuut_routing::routes_yaml::parse_routes_yaml(vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML)
            .unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
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
    let state = vuuut_api::AppState {
        db,
        config: Arc::new(config),
        settings,
        theme: Arc::new(tokio::sync::RwLock::new(Arc::new(theme))),
        router: Arc::new(tokio::sync::RwLock::new(router)),
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(
            vuuut_routing::redirects::Redirects::default(),
        ))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(vuuut_db::limits::Limits::default()),
        // Empty: a fixture firewall lets everybody through.
        firewall: vuuut_db::repos::firewall::RuleCache::empty(),
    };
    Fixture { _db: test_db, state, owner: owner.id }
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

impl Reply {
    fn flash(&self) -> String {
        let Some(query) = self.location.split_once('?').map(|(_, q)| q) else { return String::new() };
        form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == "ok" || key == "error")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

async fn send(
    state: &vuuut_api::AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    form: Option<&[(&str, &str)]>,
) -> Reply {
    let mut request =
        Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let body = match form {
        Some(pairs) => {
            request = request.header("content-type", "application/x-www-form-urlencoded");
            let mut s = form_urlencoded::Serializer::new(String::new());
            for (k, v) in pairs {
                s.append_pair(k, v);
            }
            Body::from(s.finish())
        }
        None => Body::empty(),
    };
    let app = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location =
        response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &vuuut_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "GET", uri, Some(cookie), None, None).await
}

async fn post(
    state: &vuuut_api::AppState,
    uri: &str,
    cookie: &str,
    pairs: &[(&str, &str)],
) -> Reply {
    send(state, "POST", uri, Some(cookie), Some(ORIGIN), Some(pairs)).await
}




struct Site {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn site(state: &vuuut_api::AppState, uri: &str, extra: &[(&str, &str)]) -> Site {
    let mut req = Request::builder().method("GET").uri(uri).header("host", HOST).header("accept", "text/html");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let response = vuuut_front::site_router(state.clone()).oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 20_000_000).await.unwrap().to_vec();
    Site { status, headers, body }
}

fn xcache(s: &Site) -> String {
    s.headers.get("x-cache").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

#[tokio::test]
async fn the_tool_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    for uri in ["/caching/", "/caching/pages/", "/caching/settings/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        assert_ne!(get(&f.state, uri, &f.author_session().await).await.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    let forged = send(&f.state, "POST", "/caching/purge/", Some(&cookie), Some("https://elsewhere.example"), Some(&[])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn anonymous_pages_are_served_from_memory_until_something_changes() {
    let f = fixture().await;
    let cookie = f.session().await;

    // Off: the response is untouched — no X-Cache, no Cache-Control added.
    let before = site(&f.state, "/", &[]).await;
    assert_eq!(before.status, StatusCode::OK);
    assert!(before.headers.get("x-cache").is_none(), "{:?}", before.headers);

    let saved = post(&f.state, "/caching/settings/", &cookie, &[
        ("cache_pages", "1"), ("cache_ttl", "3600"), ("cache_max_pages", "500"),
        ("cache_browser_ttl", "60"), ("cache_cdn_ttl", "300"), ("cache_etag", "1"), ("cache_gzip", "1"),
        ("perf_lazy_images", "1"), ("perf_speculation_level", "1"), ("perf_preconnect", "https://fonts.gstatic.com\njavascript:x"),
        ("cache_exclude", "/about/"),
    ]).await;
    assert!(!saved.location.contains("error="), "{}", saved.location);

    let first = site(&f.state, "/", &[]).await;
    assert_eq!(xcache(&first), "MISS");
    let second = site(&f.state, "/", &[("cookie", "vuuut_consent=x")]).await;
    assert_eq!(xcache(&second), "HIT", "a consent cookie is not personal");
    assert_eq!(second.body, first.body);
    let cc = second.headers.get("cache-control").unwrap().to_str().unwrap();
    assert_eq!(cc, "public, max-age=60, s-maxage=300, stale-while-revalidate=300");
    let html = String::from_utf8_lossy(&first.body);
    assert!(html.contains(r#"<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>"#));
    assert!(html.contains(r#"<script type="speculationrules">"#));
    assert!(!html.contains("javascript:x"));

    // A member is never served the stored page.
    let member = site(&f.state, "/", &[("cookie", "vuuut-members-ssr=abc")]).await;
    assert!(xcache(&member).starts_with("BYPASS (signed in)"), "{}", xcache(&member));
    // An excluded path is never stored.
    let about = site(&f.state, "/about/", &[]).await;
    if about.status == StatusCode::OK {
        assert!(xcache(&about).contains("excluded"), "{}", xcache(&about));
    }

    // ETag → 304 with no body.
    let tag = first.headers.get("etag").unwrap().to_str().unwrap().to_string();
    let not_modified = site(&f.state, "/", &[("if-none-match", &tag)]).await;
    assert_eq!(not_modified.status, StatusCode::NOT_MODIFIED);
    assert!(not_modified.body.is_empty());

    // gzip, when the client asks.
    let zipped = site(&f.state, "/", &[("accept-encoding", "gzip")]).await;
    assert_eq!(zipped.headers.get("content-encoding").and_then(|v| v.to_str().ok()), Some("gzip"));

    // The admin sees the page, and the numbers.
    let pages = get(&f.state, "/caching/pages/", &cookie).await;
    assert!(pages.body.contains(r#"<a href="/" target="_blank""#), "{}", pages.body);
    let overview = get(&f.state, "/caching/", &cookie).await;
    assert!(overview.body.contains("Page cache") && overview.body.contains("served from memory"), "{}", overview.body);

    // An edit anywhere empties it: a setting written through the cache.
    f.state.settings.set(&f.state.db, "description", &serde_json::json!("changed")).await.unwrap();
    assert_eq!(xcache(&site(&f.state, "/", &[]).await), "MISS");
    assert_eq!(xcache(&site(&f.state, "/", &[]).await), "HIT");

    // Purging one page from the admin.
    let r = post(&f.state, "/caching/pages/purge/", &cookie, &[("key", "/")]).await;
    assert!(r.flash().contains("Purged"), "{}", r.flash());
    assert_eq!(xcache(&site(&f.state, "/", &[]).await), "MISS");
    let r = post(&f.state, "/caching/pages/purge/", &cookie, &[("key", "/../../etc/passwd")]).await;
    assert!(r.location.contains("error="));
}

#[tokio::test]
async fn bad_cloudflare_details_are_refused_before_anything_is_saved() {
    let f = fixture().await;
    let cookie = f.session().await;
    let r = post(&f.state, "/caching/settings/", &cookie, &[("cf_zone", "not-a-zone"), ("cf_token", "abcdefghijklmnopqrstuvwxyz")]).await;
    assert!(r.flash().contains("32 letters"), "{}", r.flash());
    let r = post(&f.state, "/caching/settings/", &cookie, &[("cf_zone", "0123456789abcdef0123456789abcdef")]).await;
    assert!(r.flash().contains("API token"), "{}", r.flash());
    assert!(vuuut_db::repos::caching::cdn(&f.state.db).await.is_none());
}
