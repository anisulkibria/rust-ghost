//! Generation and removal through the admin routes on an isolated database.
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
        let id = vuuut_db::repos::sessions::create(&self.state.db, &self.owner)
            .await
            .unwrap();
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
        let session = vuuut_db::repos::sessions::create(&self.state.db, &id)
            .await
            .unwrap();
        format!("vuuut-admin-session={session}")
    }
}

async fn fixture() -> Fixture {
    fixture_with_theme("atrium").await
}

async fn fixture_with_theme(theme: &str) -> Fixture {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused")
        .await
        .unwrap();
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Dummy content test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}}, "paths": {}
    }))
    .unwrap();
    let routes = vuuut_routing::routes_yaml::parse_routes_yaml(
        vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML,
    )
    .unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/themes").join(theme);
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
    Fixture {
        _db: test_db,
        state,
        owner: owner.id,
    }
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

impl Reply {
    fn flash(&self) -> String {
        let Some(query) = self.location.split_once('?').map(|(_, q)| q) else {
            return String::new();
        };
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
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST)
        .header("accept", "text/html");
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
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply {
        status,
        location,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

const BASE: &str = "/dummies/";

async fn submit(f: &Fixture, cookie: &str, action: &str, fields: &[(&str, &str)]) -> Reply {
    let response = send(
        &f.state,
        "POST",
        &format!("{BASE}{action}/"),
        Some(cookie),
        Some(ORIGIN),
        Some(fields),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    response
}

async fn batch_id(f: &Fixture, kind: &str) -> String {
    sqlx::query_scalar("SELECT batch_id FROM dummy_items WHERE kind = ? LIMIT 1")
        .bind(kind)
        .fetch_one(&f.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn podcast_and_portfolio_samples_load_from_dummy_content() {
    let f = fixture().await;
    let cookie = f.session().await;
    let page = send(&f.state, "GET", BASE, Some(&cookie), None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    for sample in ["podcast", "portfolio-personal", "portfolio-studio"] {
        assert!(page.body.contains(&format!(r#"name="sample" value="{sample}""#)));
    }

    let forbidden = send(&f.state, "POST", "/dummies/sample/", Some(&cookie), Some("https://other.test"), Some(&[("sample", "podcast")])).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    let invalid = submit(&f, &cookie, "sample", &[("sample", "unknown")]).await;
    assert!(invalid.flash().contains("Choose a podcast or portfolio sample"));

    let podcast = &[("sample", "podcast")];
    let loaded = submit(&f, &cookie, "sample", podcast).await;
    assert!(loaded.location.starts_with("/ghost/dummies/?ok="), "{}", loaded.location);
    assert!(loaded.flash().contains("11 episodes"));
    assert!(f.state.settings.get_bool(vuuut_core::podcast::SETTING_ENABLED));
    let shows = vuuut_db::repos::podcast::shows(&f.state.db, false).await;
    assert_eq!(shows.len(), 1);
    assert_eq!(shows[0].title, "Open Workshop");
    assert!(submit(&f, &cookie, "sample", podcast).await.flash().contains("already"));

    for (f, sample, name) in [(f, "portfolio-personal", "Lena Hart"), (fixture().await, "portfolio-studio", "Harbour Lane")] {
        let cookie = f.session().await;
        let fields = &[("sample", sample)];
        let loaded = submit(&f, &cookie, "sample", fields).await;
        assert!(loaded.location.starts_with("/ghost/dummies/?ok="), "{}", loaded.location);
        assert!(loaded.flash().contains("6 projects"));
        assert!(f.state.settings.get_bool(vuuut_core::portfolio::SETTING_ENABLED));
        assert_eq!(f.state.settings.get_str(vuuut_core::portfolio::SETTING_NAME).as_deref(), Some(name));
        assert!(submit(&f, &cookie, "sample", fields).await.flash().contains("already"));
    }
}

#[tokio::test]
async fn handbook_demo_renders_reuses_existing_content_and_can_be_removed() {
    use serde_json::json;
    use vuuut_core::dummies::Kind;
    use vuuut_db::repos::{dummies, posts, tags};

    let f = fixture_with_theme("handbook").await;
    let db = &f.state.db;
    let cookie = f.session().await;
    let page = send(&f.state, "GET", BASE, Some(&cookie), None, None).await;
    assert!(page.body.contains(r#"name="demo" value="documentation""#));

    let nav = json!([{"label":"Home","url":"/"}]);
    f.state.settings.set(db, "navigation", &nav).await.unwrap();
    f.state.settings.set(db, "custom_theme_settings", &json!({"handbook":{"hero_heading":"Welcome"}})).await.unwrap();
    let publishing = tags::insert(db, &tags::NewTag { name: "Publishing".into(), ..Default::default() }).await.unwrap();

    let demo = &[("demo", "documentation")];
    let generated = submit(&f, &cookie, "demo", demo).await;
    assert_eq!(generated.flash(), "Generated 5 tags, 15 posts, 1 page and 2 menu links.");
    let batches = dummies::batches(db).await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].label, "demo:documentation");
    assert!(batches[0].remaining.contains(&(Kind::Posts, 15)));
    let custom = f.state.settings.get("custom_theme_settings").unwrap();
    assert_eq!(custom["handbook"]["hero_heading"], "Welcome");
    assert_eq!(custom["handbook"]["sections"], "getting-started,publishing,customization,reference,support,troubleshooting");

    let public = vuuut_front::site_router(f.state.clone())
        .oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(public.status(), StatusCode::OK);
    let body = to_bytes(public.into_body(), 1_000_000).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    assert!(html.contains("Browse by section"));
    assert!(html.contains(r#"data-section="getting-started""#));
    assert!(html.find("Create your publication").unwrap() < html.find("Plan your handbook").unwrap());
    assert!(html.contains("/about-handbook/"));

    assert!(submit(&f, &cookie, "demo", demo).await.flash().contains("Nothing new"));
    assert_eq!(dummies::batches(db).await.len(), 1);
    sqlx::query("UPDATE posts SET title = 'My own publishing guide' WHERE slug = 'write-a-step-by-step-guide'")
        .execute(db).await.unwrap();
    let removed = submit(&f, &cookie, "remove", &[("batch", &batches[0].id)]).await;
    assert!(removed.flash().contains("Kept 1"), "{}", removed.flash());
    assert!(dummies::batches(db).await.is_empty());
    assert!(posts::find_by_slug(db, "create-your-publication", "post", true).await.unwrap().is_none());
    assert!(posts::find_by_slug(db, "write-a-step-by-step-guide", "post", true).await.unwrap().is_some());
    assert_eq!(tags::find_by_slug(db, "publishing").await.unwrap().unwrap().id, publishing.id);
    assert_eq!(f.state.settings.get("navigation"), Some(nav));
}

#[tokio::test]
async fn full_demo_generates_every_kind_and_removes_only_its_own_content() {
    use vuuut_core::dummies::Kind;
    let f = fixture().await;
    let db = &f.state.db;
    let cookie = f.session().await;
    let get = |cookie| send(&f.state, "GET", BASE, Some(cookie), None, None);
    let before = get(&cookie).await;
    assert_eq!(before.status, StatusCode::OK);
    assert!(before.body.contains("Nothing generated yet"));

    let author = f.author_session().await;
    assert_eq!(get(&author).await.status, StatusCode::FORBIDDEN);
    let wrong_origin = send(
        &f.state,
        "POST",
        "/dummies/full/",
        Some(&cookie),
        Some("https://other.test"),
        Some(&[]),
    )
    .await;
    assert_eq!(wrong_origin.status, StatusCode::FORBIDDEN);

    let result = submit(&f, &cookie, "full", &[("images", "1")]).await;
    assert!(
        result.flash().starts_with("Generated "),
        "{}",
        result.flash()
    );
    let counts = vuuut_db::repos::dummies::totals(db).await;
    for kind in Kind::ALL {
        let actual = counts.iter().find(|(k, _)| *k == kind);
        if kind == Kind::Tags {
            // The shuffled selection reuses News if it picks the seeded tag.
            let new_tags = actual.unwrap().1;
            assert!((i64::from(kind.full()) - 1..=i64::from(kind.full())).contains(&new_tags));
            continue;
        }
        let expected = match kind {
            Kind::Pages => 4,      // The installation already has an About page.
            Kind::Navigation => 6, // The five demo pages across both menus, and Places.
            _ => i64::from(kind.full()),
        };
        assert_eq!(actual.map_or(0, |(_, count)| *count), expected, "{kind:?}");
    }
    let has_images: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts WHERE feature_image LIKE 'https://images.unsplash.com/%'",
    )
    .fetch_one(db)
    .await
    .unwrap();
    assert!(has_images > 0);
    let disabled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE email_disabled = 1")
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(disabled, 40);
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_listings WHERE status = 'published' AND hours <> '' AND gallery <> ''")
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(live, 24, "every demo place is live with hours and photos");
    assert!(get(&cookie).await.body.contains("Remove all dummy content"));

    let removal = submit(&f, &cookie, "remove", &[("batch", "all")]).await;
    assert!(
        removal.flash().starts_with("Removed "),
        "{}",
        removal.flash()
    );
    assert!(vuuut_db::repos::dummies::totals(db).await.is_empty());
    let places: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_listings").fetch_one(db).await.unwrap();
    assert_eq!(places, 0, "the demo's directory goes with it");
    let original_pages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts WHERE slug = 'about' OR slug = 'coming-soon'",
    )
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(original_pages, 2);
    assert!(vuuut_db::repos::tags::find_by_slug(db, "news").await.unwrap().is_some());
}

#[tokio::test]
async fn draft_pages_never_get_public_links_and_navigation_can_be_generated_by_itself() {
    let f = fixture().await;
    let cookie = f.session().await;
    let drafts = submit(
        &f,
        &cookie,
        "custom",
        &[
            ("count_pages", "2"),
            ("count_products", "1"),
            ("count_navigation", "1"),
            ("status", "draft"),
        ],
    )
    .await;
    assert!(
        drafts.flash().starts_with("Generated "),
        "{}",
        drafts.flash()
    );
    let status: String = sqlx::query_scalar("SELECT status FROM posts WHERE slug = 'contact'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    assert_eq!(status, "draft");
    let product_status: String = sqlx::query_scalar("SELECT status FROM ecom_products")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    assert_eq!(product_status, "draft");
    let menus = f.state.settings.get("navigation").unwrap().to_string();
    assert!(!menus.contains("/contact/"), "{menus}");

    let menu = submit(&f, &cookie, "custom", &[("count_navigation", "1")]).await;
    assert!(menu.flash().starts_with("Generated "), "{}", menu.flash());
    let menus = f.state.settings.get("navigation").unwrap().to_string();
    assert!(
        menus.contains("/about/") || menus.contains("/tag/"),
        "{menus}"
    );
}

#[tokio::test]
async fn removal_keeps_tags_in_use_by_other_batches_and_edits_with_the_same_timestamp() {
    let f = fixture().await;
    let cookie = f.session().await;
    let db = &f.state.db;
    assert!(submit(&f, &cookie, "custom", &[("count_tags", "1")])
        .await
        .flash()
        .starts_with("Generated "));
    let tag_batch = batch_id(&f, "tags").await;
    assert!(submit(&f, &cookie, "custom", &[("count_posts", "4")])
        .await
        .flash()
        .starts_with("Generated "));
    let posts_batch = batch_id(&f, "posts").await;
    let tagged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags WHERE tag_id IN (SELECT item_id FROM dummy_items WHERE kind = 'tags')")
        .fetch_one(db).await.unwrap();
    assert!(tagged > 0);

    let result = submit(&f, &cookie, "remove", &[("batch", &tag_batch)]).await;
    assert!(result.flash().contains("Kept 1"), "{}", result.flash());
    let still_tagged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags WHERE tag_id NOT IN (SELECT id FROM tags WHERE slug = 'news')")
        .fetch_one(db).await.unwrap();
    assert_eq!(still_tagged, tagged);

    let edited: String =
        sqlx::query_scalar("SELECT item_id FROM dummy_items WHERE batch_id = ? LIMIT 1")
            .bind(&posts_batch)
            .fetch_one(db)
            .await
            .unwrap();
    sqlx::query("UPDATE posts SET title = 'Keep this edit' WHERE id = ?")
        .bind(&edited)
        .execute(db)
        .await
        .unwrap();
    let result = submit(&f, &cookie, "remove", &[("batch", &posts_batch)]).await;
    assert!(result.flash().contains("Kept 1"), "{}", result.flash());
    let title: String = sqlx::query_scalar("SELECT title FROM posts WHERE id = ?")
        .bind(&edited)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(title, "Keep this edit");
}
