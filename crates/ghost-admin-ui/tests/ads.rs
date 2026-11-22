//! Tools → Advertising & Ad Management end to end: build ads in the admin,
//! then see the public side choose, place, label and count them, serve
//! ads.txt, and spare paying members — and export the result. Refusals
//! first.
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

    /// A published post or page with a body, a visibility and tags.
    async fn post(&self, slug: &str, title: &str, body: &str, kind: &str, visibility: &str, tags: &[&str]) {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'published', ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
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
                    let t = ghost_core::ids::new_id();
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
                .bind(ghost_core::ids::new_id())
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
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Ads test")),
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




const LIVE_IMAGE: &[(&str, &str)] = &[
    ("name", "Spring banner"), ("status", "live"), ("kind", "image"),
    ("image", "https://cdn.example/banner.png"), ("link", "https://shop.example/spring"), ("alt", "Spring sale"),
    ("placement", "post_paragraph"), ("paragraph", "1"),
    ("audience", "everyone"), ("device", "all"), ("targeting", "everywhere"), ("weight", "1"),
    ("cpm", "2.00"), ("cpc", "0.50"),
];

async fn ad_id(f: &Fixture, name: &str) -> String {
    ghost_db::repos::ads::list(&f.state.db).await.unwrap().into_iter().find(|a| a.name == name).unwrap().id
}

async fn public(state: &ghost_api::AppState, method: &str, uri: &str, body: &str, origin: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri).header("host", HOST)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    let app = ghost_api::ads::public_router().with_state(state.clone());
    let response = app.oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_tool_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    for uri in ["/ads/", "/ads/list/", "/ads/new/", "/ads/advertisers/", "/ads/settings/", "/ads/report.csv"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        let author = get(&f.state, uri, &f.author_session().await).await;
        assert_ne!(author.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    for uri in ["/ads/new/", "/ads/settings/", "/ads/settings/switch/", "/ads/advertisers/"] {
        let forged = send(&f.state, "POST", uri, Some(&cookie), Some("https://elsewhere.example"), Some(&[("name", "x")])).await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN, "{uri}");
    }
}

#[tokio::test]
async fn an_ad_built_in_the_admin_is_placed_labelled_and_counted() {
    let f = fixture().await;
    let cookie = f.session().await;
    let r = post(&f.state, "/ads/new/", &cookie, LIVE_IMAGE).await;
    assert!(r.status.is_redirection() && !r.location.contains("error="), "{}", r.location);
    let id = ad_id(&f, "Spring banner").await;

    // Ads are off: nothing is chosen, and ads.txt is a 404.
    assert!(ghost_api::ads::plan(&f.state, "/hello/", &serde_json::Value::Null, "post", &[]).await.is_none());
    post(&f.state, "/ads/settings/", &cookie, &[
        ("ads_enabled", "1"), ("ads_hide_paid", "1"), ("ads_label", "Sponsored"), ("ads_max_in_content", "3"),
        ("ads_txt", "google.com, pub-123, DIRECT, f08c47fec0942fa0\nnot a record"),
    ]).await;

    let plan = ghost_api::ads::plan(&f.state, "/hello/", &serde_json::Value::Null, "post", &[]).await.expect("an ad");
    let mut data = serde_json::json!({"post": {"html": "<p>one</p><p>two</p>"}});
    plan.apply_to_post(&mut data);
    let body = data["post"]["html"].as_str().unwrap();
    assert!(body.starts_with("<p>one</p><aside class=\"vad vad-post_paragraph\""), "{body}");
    assert!(body.contains(r#"<span class="vad-label">Sponsored</span>"#));
    assert!(body.contains(r#"rel="sponsored noopener""#));
    let page = plan.finish("<html><head></head><body>x</body></html>".into());
    assert!(page.contains("/ads/api/assets/") && page.contains("ads.css"), "{page}");

    // Not on a page, and not for a paying member.
    assert!(ghost_api::ads::plan(&f.state, "/about/", &serde_json::Value::Null, "page", &[]).await.is_none());
    let paid = serde_json::json!({"status": "paid"});
    assert!(ghost_api::ads::plan(&f.state, "/hello/", &paid, "post", &[]).await.is_none());

    // Counting: same-origin only, forged ids ignored.
    let body = format!("i={id},nope,ffffffffffffffffffffffff&c={id}");
    assert_eq!(public(&f.state, "POST", "/ads/api/events/", &body, Some("https://elsewhere.example")).await.0, StatusCode::FORBIDDEN);
    assert_eq!(public(&f.state, "POST", "/ads/api/events/", &body, Some(ORIGIN)).await.0, StatusCode::NO_CONTENT);
    let ad = ghost_db::repos::ads::find(&f.state.db, &id).await.unwrap().unwrap();
    assert_eq!((ad.impressions, ad.clicks), (1, 1));

    // ads.txt is served as typed, and the odd line is flagged in the admin.
    let (status, txt) = public(&f.state, "GET", "/ads.txt", "", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(txt.starts_with("google.com, pub-123, DIRECT"));
    let settings = get(&f.state, "/ads/settings/", &cookie).await;
    assert!(settings.body.contains("doesn't look like a seller record"), "{}", settings.body);

    // The figures reach the overview, the ad's page and the export.
    let overview = get(&f.state, "/ads/", &cookie).await;
    assert!(overview.body.contains("Spring banner"), "{}", overview.body);
    let edit = get(&f.state, &format!("/ads/{id}/"), &cookie).await;
    assert!(edit.body.contains("100.00%"), "1 click of 1 impression: {}", edit.body);
    let csv = get(&f.state, "/ads/report.csv", &cookie).await;
    assert!(csv.body.contains("\"Spring banner\",\"House ad\",\"live\""), "{}", csv.body);
    assert!(csv.body.contains("\"0.50\""), "CPC of one click is earned: {}", csv.body);
}

#[tokio::test]
async fn a_bad_link_is_refused_and_a_code_ad_never_runs_in_the_admin() {
    let f = fixture().await;
    let cookie = f.session().await;
    let mut bad = LIVE_IMAGE.to_vec();
    bad.retain(|(k, _)| *k != "link");
    bad.push(("link", "javascript:alert(1)"));
    let r = post(&f.state, "/ads/new/", &cookie, &bad).await;
    assert!(r.location.contains("error="), "{}", r.location);
    assert!(ghost_db::repos::ads::list(&f.state.db).await.unwrap().is_empty());

    post(&f.state, "/ads/new/", &cookie, &[("name", "Network"), ("kind", "code"), ("code", "<script>window.pwned=1</script>")]).await;
    let id = ad_id(&f, "Network").await;
    let edit = get(&f.state, &format!("/ads/{id}/"), &cookie).await;
    assert!(!edit.body.contains("<script>window.pwned"), "the code ran in the admin");
    assert!(edit.body.contains("&lt;script&gt;window.pwned"));
}

#[tokio::test]
async fn advertisers_get_their_own_report_and_survive_as_house_ads() {
    let f = fixture().await;
    let cookie = f.session().await;
    let bad = post(&f.state, "/ads/advertisers/", &cookie, &[("name", "Acme"), ("website", "javascript:x")]).await;
    assert!(bad.location.contains("error="));
    let r = post(&f.state, "/ads/advertisers/", &cookie, &[("name", "Acme <Ltd>"), ("email", "ads@acme.test"), ("website", "https://acme.test")]).await;
    let adv = r.location.trim_start_matches("/ghost/ads/advertisers/").split('/').next().unwrap().to_string();
    assert!(ghost_core::ids::is_objectid(&adv), "{}", r.location);
    let mut with = LIVE_IMAGE.to_vec();
    with.push(("advertiser_id", &adv));
    post(&f.state, "/ads/new/", &cookie, &with).await;
    let page = get(&f.state, &format!("/ads/advertisers/{adv}/"), &cookie).await;
    assert!(page.body.contains("Acme &lt;Ltd&gt;") && page.body.contains("Spring banner"), "{}", page.body);
    let id = ad_id(&f, "Spring banner").await;
    ghost_db::repos::ads::count(&f.state.db, std::slice::from_ref(&id), false).await;
    let csv = get(&f.state, &format!("/ads/advertisers/{adv}/report.csv"), &cookie).await;
    assert!(csv.body.lines().nth(1).unwrap().contains("\"Spring banner\",\"1\",\"0\""), "{}", csv.body);
    post(&f.state, &format!("/ads/advertisers/{adv}/delete/"), &cookie, &[]).await;
    assert_eq!(ghost_db::repos::ads::find(&f.state.db, &id).await.unwrap().unwrap().advertiser_id, None);
}

#[tokio::test]
async fn duplicating_makes_a_draft_with_fresh_counters() {
    let f = fixture().await;
    let cookie = f.session().await;
    post(&f.state, "/ads/new/", &cookie, LIVE_IMAGE).await;
    let id = ad_id(&f, "Spring banner").await;
    ghost_db::repos::ads::count(&f.state.db, std::slice::from_ref(&id), false).await;
    post(&f.state, &format!("/ads/{id}/duplicate/"), &cookie, &[]).await;
    let copy = ghost_db::repos::ads::list(&f.state.db).await.unwrap().into_iter().find(|a| a.name == "Spring banner (copy)").unwrap();
    assert_eq!((copy.status.as_str(), copy.impressions, copy.cpc_cents), ("draft", 0, 50));
    post(&f.state, &format!("/ads/{}/delete/", copy.id), &cookie, &[]).await;
    assert!(ghost_db::repos::ads::find(&f.state.db, &copy.id).await.unwrap().is_none());
}
