//! The Cookie consent module end to end: find a tracker, declare it, switch
//! the banner on, see the public page hold the tracker back, post a
//! decision the way the visitor script does, and read it back in the log
//! and its export.
//!
//! The refusals come first. The consent log is a record about visitors and
//! the settings decide what runs on every page, so neither is for an Author.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

struct Fixture {
    _db: ghost_db::TestDb,
    state: ghost_api::AppState,
    owner: String,
}

impl Fixture {
    async fn session(&self) -> String {
        let id = ghost_db::repos::sessions::create(&self.state.db, &self.owner).await.unwrap();
        format!("vuuut-admin-session={id}")
    }

    /// A session for somebody who is not an administrator.
    async fn author_session(&self) -> String {
        let id = ghost_core::ids::new_id();
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
            .bind(ghost_core::ids::new_id())
            .bind(&role)
            .bind(&id)
            .execute(&self.state.db)
            .await
            .unwrap();
        let session = ghost_db::repos::sessions::create(&self.state.db, &id).await.unwrap();
        format!("vuuut-admin-session={session}")
    }

    /// A published post, with optional code injection.
    async fn post_with(&self, slug: &str, title: &str, html: &str, head: &str) {
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, codeinjection_head, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'post', 'published', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(ghost_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(html)
        .bind(head)
        .execute(&self.state.db)
        .await
        .unwrap();
    }
}

async fn fixture() -> Fixture {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Cookies test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}}, "paths": {}
    }))
    .unwrap();
    let routes =
        ghost_routing::routes_yaml::parse_routes_yaml(ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML)
            .unwrap();
    let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
    let deps = ghost_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: ghost_api::themes::url_resolver(router.clone()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: "test".into(),
        theme_dir: theme_dir.clone(),
        site_origin: ORIGIN.into(),
        content_api_key: String::new(),
    };
    let theme = ghost_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
    let state = ghost_api::AppState {
        db,
        config: Arc::new(config),
        settings,
        theme: Arc::new(tokio::sync::RwLock::new(Arc::new(theme))),
        router: Arc::new(tokio::sync::RwLock::new(router)),
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(
            ghost_routing::redirects::Redirects::default(),
        ))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(ghost_db::limits::Limits::default()),
        // Empty: a fixture firewall lets everybody through.
        firewall: ghost_db::repos::firewall::RuleCache::empty(),
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
    state: &ghost_api::AppState,
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
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location =
        response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &ghost_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "GET", uri, Some(cookie), None, None).await
}

async fn post(
    state: &ghost_api::AppState,
    uri: &str,
    cookie: &str,
    pairs: &[(&str, &str)],
) -> Reply {
    send(state, "POST", uri, Some(cookie), Some(ORIGIN), Some(pairs)).await
}


const GA: &str = r#"<script async src="https://www.googletagmanager.com/gtag/js?id=G-TEST"></script>"#;

async fn public(state: &ghost_api::AppState, body: &str, origin: Option<&str>, gpc: bool) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/cookies/api/consent/")
        .header("host", HOST)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if gpc {
        request = request.header("sec-gpc", "1");
    }
    let app = ghost_api::cookies::public_router().with_state(state.clone());
    let response = app.oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    for uri in ["/cookies/", "/cookies/declarations/", "/cookies/log/", "/cookies/log.csv", "/settings/cookies/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(
            anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN,
            "{uri} answered {} to nobody",
            anon.status
        );
        let author = get(&f.state, uri, &f.author_session().await).await;
        assert_ne!(author.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    for uri in ["/cookies/switch/", "/cookies/renew/", "/cookies/log/purge/", "/settings/cookies/", "/cookies/declarations/"] {
        let forged = send(&f.state, "POST", uri, Some(&cookie), Some("https://elsewhere.example"), Some(&[("on", "1")])).await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN, "{uri} accepted a cross-site write");
    }
    // And a stranger's POST gets the sign-in redirect, not an extractor 422.
    let anon = send(&f.state, "POST", "/cookies/declarations/", None, Some(ORIGIN), Some(&[])).await;
    assert_ne!(anon.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_tracker_found_with_the_banner_off_is_critical_and_can_be_declared_in_one_click() {
    let f = fixture().await;
    f.post_with("tracked", "Tracked post", "<p>x</p>", GA).await;
    let cookie = f.session().await;

    let page = get(&f.state, "/cookies/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Google Analytics"), "{}", page.body);
    assert!(page.body.contains("Loads before consent"));
    assert!(page.body.contains("Post: Tracked post"));
    assert!(page.body.contains("set cookies before anyone agrees"), "{}", page.body);
    assert!(page.body.contains("not in the cookie declaration"));

    let r = post(&f.state, "/cookies/declarations/import/", &cookie, &[("tracker", "google_analytics")]).await;
    assert!(r.status.is_redirection());
    assert!(r.flash().contains("3 cookies"), "{}", r.flash());
    // Twice does not duplicate.
    let again = post(&f.state, "/cookies/declarations/import/", &cookie, &[("tracker", "google_analytics")]).await;
    assert!(again.flash().contains("0 cookies"), "{}", again.flash());
    let list = get(&f.state, "/cookies/declarations/", &cookie).await;
    assert_eq!(list.body.matches(">_ga<").count(), 1, "{}", list.body);
    assert!(list.body.contains("vuuut_consent"), "built-in cookies are listed");
    let page = get(&f.state, "/cookies/", &cookie).await;
    assert!(!page.body.contains("not in the cookie declaration"));

    // An unknown tracker key is refused rather than trusted.
    let bad = post(&f.state, "/cookies/declarations/import/", &cookie, &[("tracker", "../../etc")]).await;
    assert!(bad.location.contains("error="));
}

#[tokio::test]
async fn settings_saved_here_hold_trackers_back_on_the_public_page() {
    let f = fixture().await;
    let cookie = f.session().await;
    let page = format!("<html><head>{GA}</head><body><p>Hi</p></body></html>");

    // Off: the page goes out exactly as rendered.
    assert_eq!(ghost_api::cookies::inject(&f.state, page.clone()).await, page);

    let saved = post(
        &f.state,
        "/settings/cookies/",
        &cookie,
        &[
            ("cookies_enabled", "1"),
            ("cookies_mode", "opt_in"),
            ("cookies_layout", "bar"),
            ("cookies_title", "Cookies? \"Yes\" <b>please</b>"),
            ("cookies_policy_url", "/privacy/"),
            ("cookies_expiry_months", "6"),
            ("cookies_log", "1"),
            ("cookies_log_days", "180"),
        ],
    )
    .await;
    assert!(saved.status.is_redirection(), "{}", saved.status);
    assert!(!saved.location.contains("error="), "{}", saved.location);

    let out = ghost_api::cookies::inject(&f.state, page.clone()).await;
    assert!(out.contains(r#"<script type="text/plain" data-cookie-category="analytics" async src="https://www.googletagmanager.com/gtag/js?id=G-TEST">"#), "{out}");
    assert!(out.contains("/cookies/api/assets/"));
    assert!(out.contains("cookies.css"));
    // The configuration rides in an attribute, escaped.
    assert!(out.contains("&quot;layout&quot;:&quot;bar&quot;"), "{out}");
    assert!(!out.contains("<b>please</b>"));
    assert!(out.contains("&quot;days&quot;:182"));

    // A policy link that could run is refused, and nothing else changes.
    let bad = post(&f.state, "/settings/cookies/", &cookie, &[("cookies_enabled", "1"), ("cookies_policy_url", "javascript:alert(1)")]).await;
    assert!(bad.location.contains("error="), "{}", bad.location);
    let settings = get(&f.state, "/settings/cookies/", &cookie).await;
    assert!(settings.body.contains(r#"value="/privacy/""#), "{}", settings.body);

    // Inform-only does not hold anything back.
    post(&f.state, "/settings/cookies/", &cookie, &[("cookies_enabled", "1"), ("cookies_mode", "notice")]).await;
    let notice = ghost_api::cookies::inject(&f.state, page.clone()).await;
    assert!(notice.contains(GA));
}

#[tokio::test]
async fn a_decision_posted_like_the_visitor_script_reaches_the_log_and_the_export() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let body = format!("action=custom&categories=analytics%2Cmarketing&id={id}&page=%2Fabout%2C%2F");

    // Banner off: nothing is recorded.
    let (status, _) = public(&f.state, &body, Some(ORIGIN), false).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    post(&f.state, "/cookies/switch/", &cookie, &[("on", "1")]).await;

    // Cross-site, forged action, missing id: refused.
    assert_eq!(public(&f.state, &body, Some("https://elsewhere.example"), false).await.0, StatusCode::FORBIDDEN);
    assert_eq!(public(&f.state, &format!("action=sell&id={id}"), Some(ORIGIN), false).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(public(&f.state, "action=accept_all&id=nope", Some(ORIGIN), false).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    // With GPC on the request, marketing is refused whatever was sent.
    let (status, _) = public(&f.state, &body, Some(ORIGIN), true).await;
    assert_eq!(status, StatusCode::OK);

    let log = get(&f.state, "/cookies/log/?q=0F8FAD5B", &cookie).await;
    assert!(log.body.contains(id), "{}", log.body);
    assert!(log.body.contains("GPC signal"));
    assert!(log.body.contains("Chose some"));
    let csv = get(&f.state, "/cookies/log.csv", &cookie).await;
    assert!(csv.body.contains(&format!("\"{id}\",\"custom\",\"necessary,analytics,marketing\"")), "{}", csv.body);
    assert!(csv.body.contains("\"/about,/\""));

    // A hostile search is reduced before it is echoed.
    let hostile = get(&f.state, "/cookies/log/?q=%22%3E%3Cscript%3E", &cookie).await;
    assert!(!hostile.body.contains("<script>alert"));
    assert!(!hostile.body.contains("\"><script>"));

    // The figures pick it up.
    let overview = get(&f.state, "/cookies/", &cookie).await;
    assert!(overview.body.contains(r#"<span class="ck-tile-value">1</span>"#), "{}", overview.body);
}

#[tokio::test]
async fn asking_again_moves_the_revision_and_the_switch_turns_the_banner_off() {
    let f = fixture().await;
    let cookie = f.session().await;
    let r = post(&f.state, "/cookies/renew/", &cookie, &[]).await;
    assert!(r.flash().contains("revision 2"), "{}", r.flash());
    assert_eq!(ghost_api::cookies::settings(&f.state).revision, 2);
    post(&f.state, "/cookies/switch/", &cookie, &[("on", "1")]).await;
    assert!(ghost_api::cookies::settings(&f.state).enabled);
    post(&f.state, "/cookies/switch/", &cookie, &[("on", "0")]).await;
    assert!(!ghost_api::cookies::settings(&f.state).enabled);
}

#[tokio::test]
async fn a_hand_declared_cookie_is_validated_escaped_and_removable() {
    let f = fixture().await;
    let cookie = f.session().await;
    let bad = post(&f.state, "/cookies/declarations/", &cookie, &[("name", "a;b"), ("category", "analytics")]).await;
    assert!(bad.location.contains("error="));
    let dup = post(&f.state, "/cookies/declarations/", &cookie, &[("name", "vuuut_consent"), ("category", "necessary")]).await;
    assert!(dup.flash().contains("already declared"), "{}", dup.flash());
    post(&f.state, "/cookies/declarations/", &cookie, &[("name", "pref"), ("category", "preferences"), ("purpose", "<img src=x onerror=alert(1)>")]).await;
    let list = get(&f.state, "/cookies/declarations/", &cookie).await;
    assert!(list.body.contains(">pref<"));
    assert!(!list.body.contains("<img src=x"));
    let id = ghost_db::repos::cookies::declarations(&f.state.db).await.unwrap()[0].id.clone();
    post(&f.state, &format!("/cookies/declarations/{id}/delete/"), &cookie, &[]).await;
    assert!(ghost_db::repos::cookies::declarations(&f.state.db).await.unwrap().is_empty());
    // A forged id is refused without touching anything.
    let forged = post(&f.state, "/cookies/declarations/..%2F..%2F/delete/", &cookie, &[]).await;
    assert!(forged.status.is_redirection() || forged.status == StatusCode::NOT_FOUND);
}
