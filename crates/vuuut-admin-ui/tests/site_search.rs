//! Tools → Search and Filtering end to end: configure the search in the
//! admin, then search the public endpoint the way the overlay does and see
//! the settings take effect — and see Sodo's index feed follow the same
//! rules. Refusals first.
//!
//! The members-only test is the one that matters most: no setting may make
//! a paid post's body findable.
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
        ("title".into(), serde_json::json!("Search test")),
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



async fn public(state: &vuuut_api::AppState, uri: &str) -> serde_json::Value {
    let request = Request::builder().method("GET").uri(uri).header("host", HOST).body(Body::empty()).unwrap();
    let app = vuuut_api::site_search::public_router().with_state(state.clone());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let bytes = to_bytes(response.into_body(), 5_000_000).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn titles(v: &serde_json::Value) -> Vec<String> {
    v["results"].as_array().unwrap().iter().map(|r| r["title"].as_str().unwrap().to_string()).collect()
}

const VUUUT: &[(&str, &str)] = &[
    ("search_engine", "vuuut"),
    ("search_include_members", "1"),
    ("search_show_authors", "1"),
    ("search_show_tags", "1"),
    ("search_shortcut", "1"),
    ("search_log", "1"),
];

#[tokio::test]
async fn the_tool_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    for uri in ["/search/", "/search/try/", "/search/insights/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        let author = get(&f.state, uri, &f.author_session().await).await;
        assert_ne!(author.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    for uri in ["/search/", "/search/insights/clear/"] {
        let forged = send(&f.state, "POST", uri, Some(&cookie), Some("https://elsewhere.example"), Some(&[("search_engine", "off")])).await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN, "{uri}");
    }
    // The staff palette moved out of the tool's way and still answers.
    let palette = get(&f.state, "/omnisearch/?q=posts", &cookie).await;
    assert_eq!(palette.status, StatusCode::OK);
}

#[tokio::test]
async fn nothing_is_answered_until_the_built_in_engine_is_chosen() {
    let f = fixture().await;
    f.post("rust", "Rust macros", "all about macros", "post", "public", &[]).await;
    let v = public(&f.state, "/search/api/?q=rust").await;
    assert!(v["results"].as_array().unwrap().is_empty());
    let page = "<html><head></head><body></body></html>".to_string();
    assert_eq!(vuuut_api::site_search::inject(&f.state, page.clone()), page, "Sodo stays the default");

    let cookie = f.session().await;
    let r = post(&f.state, "/search/", &cookie, VUUUT).await;
    assert!(r.status.is_redirection() && !r.location.contains("error="), "{}", r.location);
    let v = public(&f.state, "/search/api/?q=rust").await;
    assert_eq!(titles(&v), ["Rust macros"]);
    let injected = vuuut_api::site_search::inject(&f.state, page);
    assert!(injected.contains("/search/api/assets/") && injected.contains("search.css"), "{injected}");
}

#[tokio::test]
async fn full_text_synonyms_and_filters_change_what_a_reader_finds() {
    let f = fixture().await;
    // The word sits past the generated excerpt (the opening few hundred
    // characters), so only a search of the whole body can reach it.
    let long = format!("{} a kubernetes primer for beginners", "filler words to pad ".repeat(60));
    f.post("k8s", "Running clusters", &long, "post", "public", &["devops"]).await;
    f.post("js", "Learning JavaScript", "closures and promises", "post", "public", &["code"]).await;
    f.post("js-about", "About JavaScript here", "who we are", "page", "public", &[]).await;
    let cookie = f.session().await;
    post(&f.state, "/search/", &cookie, VUUUT).await;

    // Body text is not searched until asked.
    assert!(titles(&public(&f.state, "/search/api/?q=kubernetes").await).is_empty());
    let mut with_text = VUUUT.to_vec();
    with_text.extend([("search_full_text", "1"), ("search_synonyms", "js, javascript"), ("search_include_pages", "1")]);
    post(&f.state, "/search/", &cookie, &with_text).await;
    let v = public(&f.state, "/search/api/?q=kubernetes").await;
    assert_eq!(titles(&v), ["Running clusters"]);
    // The passage quotes the body and marks the word.
    let r = &v["results"][0];
    assert!(r["excerpt"].as_str().unwrap().contains("kubernetes"));
    assert_eq!(r["marks"][0][1].as_i64().unwrap() - r["marks"][0][0].as_i64().unwrap(), 10);

    // A synonym finds both, and the type filter narrows to the page.
    let both = titles(&public(&f.state, "/search/api/?q=js").await);
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(titles(&public(&f.state, "/search/api/?q=js&type=page").await), ["About JavaScript here"]);
    // The tag filter, and its facet.
    let v = public(&f.state, "/search/api/?q=javascript&tag=code").await;
    assert_eq!(titles(&v), ["Learning JavaScript"]);
    assert!(v["facets"].as_array().unwrap().iter().any(|t| t["slug"] == "code"));

    // Excluding a tag removes its posts from both engines.
    let mut excluding = with_text.clone();
    excluding.push(("exclude_tag", "devops"));
    post(&f.state, "/search/", &cookie, &excluding).await;
    assert!(titles(&public(&f.state, "/search/api/?q=kubernetes").await).is_empty());
    let settings = get(&f.state, "/search/", &cookie).await;
    assert!(settings.body.contains(r#"value="devops" checked"#), "the exclusion is shown back");
}

#[tokio::test]
async fn a_members_only_body_cannot_be_found_whatever_the_settings() {
    let f = fixture().await;
    f.post("paid", "Premium report", "the secret quarterly numbers", "post", "paid", &[]).await;
    let cookie = f.session().await;
    let mut all_on = VUUUT.to_vec();
    all_on.extend([("search_full_text", "1"), ("search_weight_body", "10")]);
    post(&f.state, "/search/", &cookie, &all_on).await;
    assert!(titles(&public(&f.state, "/search/api/?q=quarterly").await).is_empty());
    // Its title is still findable, and it says it is for members.
    let v = public(&f.state, "/search/api/?q=premium").await;
    assert_eq!(titles(&v), ["Premium report"]);
    assert_eq!(v["results"][0]["members"], true);
    assert!(!v.to_string().contains("quarterly"));

    // Switched off, members-only posts are not listed at all.
    let mut hidden = VUUUT.to_vec();
    hidden.retain(|(k, _)| *k != "search_include_members");
    post(&f.state, "/search/", &cookie, &hidden).await;
    assert!(titles(&public(&f.state, "/search/api/?q=premium").await).is_empty());
}

#[tokio::test]
async fn a_settled_search_is_counted_and_a_miss_shows_in_insights() {
    let f = fixture().await;
    f.post("rust", "Rust macros", "", "post", "public", &[]).await;
    let cookie = f.session().await;
    post(&f.state, "/search/", &cookie, VUUUT).await;
    public(&f.state, "/search/api/?q=Rust&log=1").await;
    public(&f.state, "/search/api/?q=rust+&log=1").await;
    public(&f.state, "/search/api/?q=%3Cscript%3Eelixir&log=1").await;
    public(&f.state, "/search/api/?q=unlogged").await;

    let page = get(&f.state, "/search/insights/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#"<span class="ss-count">2</span>"#), "{}", page.body);
    assert!(page.body.contains("script elixir"), "stored in normal form: {}", page.body);
    assert!(!page.body.contains("unlogged"));
    assert!(!page.body.contains("<script>elixir"));
    // A miss links straight to the tester.
    assert!(page.body.contains("/ghost/search/try/?q=script+elixir"), "{}", page.body);

    let tried = get(&f.state, "/search/try/?q=rust", &cookie).await;
    assert!(tried.body.contains("Rust macros") && tried.body.contains("title: word"), "{}", tried.body);

    post(&f.state, "/search/insights/clear/", &cookie, &[]).await;
    let page = get(&f.state, "/search/insights/", &cookie).await;
    assert!(page.body.contains("No searches counted yet."));
}

#[tokio::test]
async fn sodo_indexes_by_the_same_rules() {
    let f = fixture().await;
    f.post("open", "Open post", "body words here", "post", "public", &[]).await;
    f.post("hidden", "Hidden post", "", "post", "public", &["internal-notes"]).await;
    f.post("contact", "Contact page", "", "page", "public", &[]).await;
    let key: String = sqlx::query_scalar("SELECT secret FROM api_keys WHERE type = 'content' LIMIT 1")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let cookie = f.session().await;
    post(&f.state, "/search/", &cookie, &[
        ("search_engine", "sodo"),
        ("search_include_pages", "1"),
        ("search_include_members", "1"),
        ("search_full_text", "1"),
        ("exclude_tag", "internal-notes"),
    ])
    .await;
    let request = Request::builder()
        .uri(format!("/search-index/posts/?key={key}"))
        .header("host", HOST)
        .body(Body::empty())
        .unwrap();
    let app = vuuut_api::content::content_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(request).await.unwrap();
    let bytes = to_bytes(response.into_body(), 5_000_000).await.unwrap();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("Open post") && body.contains("Contact page"), "{body}");
    assert!(!body.contains("Hidden post"), "{body}");
    assert!(body.contains("body words here"), "full text rides in the excerpt: {body}");
}


#[tokio::test]
async fn the_migrated_search_defaults_load_a_working_homepage_engine() {
    let mut f = fixture().await;
    f.state.settings = Arc::new(vuuut_db::SettingsCache::load(&f.state.db).await.unwrap());
    assert_eq!(f.state.settings.get_str("search_engine").as_deref(), Some("sodo"));
    assert_eq!(f.state.settings.get("search_show_authors"), Some(serde_json::json!(true)));
    assert_eq!(f.state.settings.get("search_weight_title"), Some(serde_json::json!(8)));
    assert_eq!(f.state.settings.get_str("search_exclude_tags").as_deref(), Some(""));

    let app = vuuut_front::site_router(f.state.clone());
    let response = app.oneshot(Request::builder().uri("/").header("host", HOST).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 5_000_000).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("data-ghost-search"));
    assert!(html.contains("/umd/sodo-search.min.js"));
}
