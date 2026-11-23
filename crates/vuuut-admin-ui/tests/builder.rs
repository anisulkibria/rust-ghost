//! Tools → Page Builder & Visual Editor end to end: refusals first, then a
//! layout saved through the editor's endpoint and read back through the real
//! public router — escaped, swapped in for the post body, switched off again —
//! plus revisions, templates and a settings change re-drawing every layout.
//! One test owns the public sequence, because the page cache's content
//! generation is process-wide.
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
        ("title".into(), serde_json::json!("Builder test")),
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
    body: String,
}

async fn site(state: &vuuut_api::AppState, method: &str, uri: &str, form: Option<&[(&str, &str)]>) -> Site {
    let mut req = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    let body = match form {
        Some(pairs) => {
            req = req.header("content-type", "application/x-www-form-urlencoded").header("origin", ORIGIN);
            let mut s = form_urlencoded::Serializer::new(String::new());
            for (k, v) in pairs {
                s.append_pair(k, v);
            }
            Body::from(s.finish())
        }
        None => Body::empty(),
    };
    let response = vuuut_front::site_router(state.clone()).oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Site { status, headers, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn post_id(f: &Fixture, slug: &str) -> String {
    sqlx::query_scalar("SELECT id FROM posts WHERE slug = ?").bind(slug).fetch_one(&f.state.db).await.unwrap()
}

async fn json(state: &vuuut_api::AppState, uri: &str, cookie: Option<&str>, origin: Option<&str>, body: &serde_json::Value) -> Reply {
    let mut request = Request::builder().method("POST").uri(uri).header("host", HOST).header("accept", "application/json").header("content-type", "application/json");
    if let Some(c) = cookie {
        request = request.header("cookie", c);
    }
    if let Some(o) = origin {
        request = request.header("origin", o);
    }
    let app = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

fn layout() -> serde_json::Value {
    serde_json::json!({"sections": [{"id": "s1", "s": {"bg": "colour", "bg_colour": "c2"}, "cols": [
        {"id": "c1", "s": {}, "items": [
            {"id": "w1", "t": "heading", "s": {"text": "Built <script>alert(1)</script>", "tag": "h1"}},
            {"id": "w2", "t": "button", "s": {"text": "Go", "href": "javascript:alert(1)"}},
            {"id": "w3", "t": "text", "s": {"html": "<p onclick=\"x()\">Hello <b>there</b></p>"}}
        ]},
        {"id": "c2", "s": {"w": 2}, "items": [{"id": "w4", "t": "feature", "s": {"icon": "bolt", "title": "Fast"}}]}
    ]}]})
}

#[tokio::test]
async fn strangers_and_the_wrong_people_are_refused() {
    let f = fixture().await;
    f.post("designed", "About", "Original words", "page", "public", &[]).await;
    let id = post_id(&f, "designed").await;
    let editor = format!("/posts/{id}/builder/");
    let save = format!("/posts/{id}/builder/save/");

    // no session
    let r = send(&f.state, "GET", "/builder/", None, None, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    assert!(r.location.contains("/ghost/signin/"));
    let r = send(&f.state, "GET", &editor, None, None, None).await;
    assert!(r.location.contains("/ghost/signin/"));
    let r = json(&f.state, &save, None, Some(ORIGIN), &serde_json::json!({"layout": layout()})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // an Author: not the tool, and not somebody else's page
    let author = f.author_session().await;
    assert_eq!(get(&f.state, "/builder/", &author).await.status, StatusCode::FORBIDDEN);
    assert_eq!(get(&f.state, &editor, &author).await.status, StatusCode::FORBIDDEN);
    let r = json(&f.state, &save, Some(&author), Some(ORIGIN), &serde_json::json!({"layout": layout()})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // the owner, but from another site
    let owner = f.session().await;
    let r = json(&f.state, &save, Some(&owner), Some("https://evil.test"), &serde_json::json!({"layout": layout()})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = send(&f.state, "POST", "/builder/settings/", Some(&owner), Some("https://evil.test"), Some(&[("builder_enabled", "1")])).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // a bad id never reaches a query
    let r = get(&f.state, "/posts/..%2F..%2Fetc/builder/", &owner).await;
    assert_ne!(r.status, StatusCode::OK);

    // nothing was stored
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM builder_layouts").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_layout_is_designed_saved_shown_and_switched_off() {
    let f = fixture().await;
    let owner = f.session().await;
    f.post("designed", "About", "Original words", "page", "public", &[]).await;
    let id = post_id(&f, "designed").await;

    // the tool and the editor open
    let hub = get(&f.state, "/builder/", &owner).await;
    assert_eq!(hub.status, StatusCode::OK);
    assert!(hub.body.contains("Page Builder &amp; Visual Editor") || hub.body.contains("Page Builder & Visual Editor"));
    let page = get(&f.state, &format!("/posts/{id}/"), &owner).await;
    assert!(page.body.contains(&format!("/ghost/posts/{id}/builder/")), "the post editor links to the builder");
    let ed = get(&f.state, &format!("/posts/{id}/builder/"), &owner).await;
    assert_eq!(ed.status, StatusCode::OK);
    assert!(ed.body.contains("id=\"vb-boot\""));
    assert!(ed.body.contains("builder-editor.js"));
    assert!(ed.body.contains("Original words"), "the first layout brings the page's own text in");
    assert!(!ed.body.contains("</script><script>"));

    // save through the editor's endpoint
    let r = json(&f.state, &format!("/posts/{id}/builder/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": layout(), "active": true, "showTitle": false})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    // the public page is drawn from the layout, escaped and cleaned
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert_eq!(s.status, StatusCode::OK);
    assert!(s.body.contains("vb-page"), "layout drawn");
    assert!(s.body.contains("/public/vuuut-builder.css?v="));
    assert!(s.body.contains("Built &lt;script&gt;"));
    assert!(!s.body.contains("<script>alert(1)"));
    assert!(!s.body.contains("javascript:alert"));
    assert!(!s.body.contains("onclick"));
    assert!(s.body.contains("<p>Hello <b>there</b></p>"));
    assert!(s.body.contains("--vb-cols:minmax(0,1fr) minmax(0,2fr)"));
    assert!(s.body.contains("/public/vuuut-builder.js?v="));
    assert!(!s.body.contains("<p>Original words</p>"), "the post body is replaced");
    let css = site(&f.state, "GET", "/public/vuuut-builder.css", None).await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(css.body.contains(".vb-page"));

    // the canvas: an empty root on the preview
    let uuid: String = sqlx::query_scalar("SELECT uuid FROM posts WHERE id = ?").bind(&id).fetch_one(&f.state.db).await.unwrap();
    let canvas = site(&f.state, "GET", &format!("/p/{uuid}/?vb=canvas"), None).await;
    assert!(canvas.body.contains("data-vb-canvas"));

    // a second save keeps the first as a revision
    let mut second = layout();
    second["sections"][0]["cols"][0]["items"][0]["s"]["text"] = serde_json::json!("Second version");
    let r = json(&f.state, &format!("/posts/{id}/builder/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": second, "active": true})).await;
    assert_eq!(r.status, StatusCode::OK);
    let revs = get(&f.state, &format!("/posts/{id}/builder/revisions/"), &owner).await;
    let v: serde_json::Value = serde_json::from_str(&revs.body).unwrap();
    let list = v["revisions"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    let rid = list[0]["id"].as_str().unwrap();
    let one = get(&f.state, &format!("/posts/{id}/builder/revisions/{rid}/"), &owner).await;
    assert!(one.body.contains("Built"), "{}", one.body);

    // a template, listed in the tool
    let r = json(&f.state, &format!("/posts/{id}/builder/templates/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"name": "My hero", "kind": "section", "data": layout()})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert!(get(&f.state, "/builder/templates/", &owner).await.body.contains("My hero"));

    // settings re-draw every layout
    let r = post(&f.state, "/builder/settings/", &owner, &[
        ("builder_enabled", "1"), ("builder_post_types", "all"), ("builder_who", "everyone"),
        ("builder_container_width", "1240"), ("builder_section_space", "96"), ("builder_radius", "12"),
        ("builder_heading_font", "serif"), ("builder_animations", "1"), ("palette_accent", "1"),
        ("palette_1", "#123456"), ("palette_2", "#000000"), ("palette_3", "#444444"), ("palette_4", "#eeeeee"), ("palette_5", "#ffffff"),
    ]).await;
    assert!(r.flash().contains("saved"), "{}", r.flash());
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(s.body.contains("--vb-w:1240px"), "re-rendered with the new width");
    assert!(s.body.contains("--vb-c2:#123456"));
    assert!(s.body.contains("Second version"));

    // switched off: the page's own content is back
    let r = post(&f.state, &format!("/builder/{id}/active/"), &owner, &[("active", "0")]).await;
    assert!(r.flash().contains("off"), "{}", r.flash());
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(s.body.contains("Original words"));
    assert!(!s.body.contains("vb-page"));

    // builder off site-wide: layouts kept but never drawn, editor refuses
    let r = post(&f.state, &format!("/builder/{id}/active/"), &owner, &[("active", "1")]).await;
    assert!(r.flash().contains("on"));
    let r = post(&f.state, "/builder/settings/", &owner, &[("builder_post_types", "all")]).await;
    assert!(r.flash().contains("saved"));
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(s.body.contains("Original words"), "a switched-off builder draws nothing");
    let ed = get(&f.state, &format!("/posts/{id}/builder/"), &owner).await;
    assert_eq!(ed.status, StatusCode::SEE_OTHER);
    let r = post(&f.state, "/builder/settings/", &owner, &[("builder_enabled", "1"), ("builder_post_types", "all")]).await;
    assert!(r.flash().contains("saved"));

    // removing the layout
    let r = post(&f.state, &format!("/builder/{id}/delete/"), &owner, &[]).await;
    assert!(r.flash().contains("removed"), "{}", r.flash());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM builder_layouts").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_new_page_opens_in_the_builder() {
    let f = fixture().await;
    let owner = f.session().await;
    let r = post(&f.state, "/builder/new/", &owner, &[("title", "Launch")]).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert!(r.location.contains("/builder/"), "{}", r.location);
    let (kind, status): (String, String) = sqlx::query_as("SELECT type, status FROM posts WHERE title = 'Launch'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!((kind.as_str(), status.as_str()), ("page", "draft"));
    let r = post(&f.state, "/builder/new/", &owner, &[("title", "  ")]).await;
    assert!(r.flash().contains("title"));
}

#[tokio::test]
async fn the_canvas_is_drawn_by_the_server_and_expanded() {
    let f = fixture().await;
    let owner = f.session().await;
    f.post("designed", "Designed", "Body words", "page", "public", &[]).await;
    f.post("a-story", "A story to list", "Story words", "post", "public", &[]).await;
    let id = post_id(&f, "designed").await;
    let layout = serde_json::json!({"sections": [{"cols": [{"items": [
        {"id": "w1", "t": "heading", "s": {"text": "On [[post.title]]", "if_who": "paid"}},
        {"id": "w2", "t": "posts", "s": {"count": 3}},
        {"id": "w3", "t": "inner", "cols": [{"items": []}, {"items": []}]}
    ]}]}]});
    let r = json(&f.state, &format!("/posts/{id}/builder/render/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": layout})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let html = v["html"].as_str().unwrap();
    assert!(html.contains("On Designed"), "dynamic tag filled for the editor");
    assert!(html.contains("A story to list"), "posts widget drawn");
    assert!(html.contains("vbx-ph"), "empty inner columns get drop targets");
    assert!(html.contains("draggable"));
    // a stranger cannot draw
    let r = json(&f.state, &format!("/posts/{id}/builder/render/"), None, Some(ORIGIN), &serde_json::json!({"layout": layout})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // on the site, the paid-only heading is gone for a visitor
    let r = json(&f.state, &format!("/posts/{id}/builder/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": layout, "active": true})).await;
    assert_eq!(r.status, StatusCode::OK);
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(!s.body.contains("On Designed"), "display condition hides it from visitors");
    assert!(!s.body.contains("vb-if:"), "markers never reach the visitor");
    assert!(s.body.contains("A story to list"));
}

#[tokio::test]
async fn theme_builder_parts_reach_every_page() {
    let f = fixture().await;
    let owner = f.session().await;
    f.post("designed", "Designed", "Body words", "page", "public", &[]).await;
    f.post("news-item", "News item", "News body", "post", "public", &["news"]).await;
    // a header
    let r = post(&f.state, "/builder/templates/new/", &owner, &[("kind", "header"), ("name", "Main header")]).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let hid: String = sqlx::query_scalar("SELECT id FROM builder_templates WHERE name = 'Main header'").fetch_one(&f.state.db).await.unwrap();
    let header = serde_json::json!({"sections": [{"cols": [{"items": [{"id": "h1", "t": "heading", "s": {"text": "BUILT HEADER"}}]}]}]});
    let r = json(&f.state, &format!("/builder/templates/{hid}/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": header, "active": true, "name": "Main header", "options": {"include": "all", "exclude": "/news-item/"}})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(s.body.contains("BUILT HEADER"));
    assert!(s.body.contains("vb-part-header"));
    assert!(s.body.contains(":not(.vb-part-header *)"), "the theme's header is hidden");
    let s = site(&f.state, "GET", "/news-item/", None).await;
    assert!(!s.body.contains("BUILT HEADER"), "excluded address");
    // a popup
    let r = post(&f.state, "/builder/templates/new/", &owner, &[("kind", "popup"), ("name", "Exit popup")]).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let pid: String = sqlx::query_scalar("SELECT id FROM builder_templates WHERE name = 'Exit popup'").fetch_one(&f.state.db).await.unwrap();
    let r = post(&f.state, &format!("/builder/templates/{pid}/active/"), &owner, &[("active", "1")]).await;
    assert!(r.flash().contains("on"));
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert!(s.body.contains(&format!(r#"id="popup-{pid}""#)));
    // a single-post template wraps posts tagged news
    let r = post(&f.state, "/builder/templates/new/", &owner, &[("kind", "single"), ("name", "News layout")]).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let sid: String = sqlx::query_scalar("SELECT id FROM builder_templates WHERE name = 'News layout'").fetch_one(&f.state.db).await.unwrap();
    let single = serde_json::json!({"sections": [{"cols": [{"items": [{"id": "t1", "t": "heading", "s": {"text": "Story: [[post.title]]"}}, {"id": "t2", "t": "post-content", "s": {}}]}]}]});
    let r = json(&f.state, &format!("/builder/templates/{sid}/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": single, "active": true, "options": {"post_types": "post", "tags": "news"}})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let s = site(&f.state, "GET", "/news-item/", None).await;
    assert!(s.body.contains("Story: News item"), "{}", &s.body[..s.body.len().min(400)]);
    assert!(s.body.contains("<p>News body</p>"), "the post's own content inside the template");
    // the tool lists them
    let page = get(&f.state, "/builder/theme/", &owner).await;
    assert!(page.body.contains("Main header") && page.body.contains("Exit popup") && page.body.contains("News layout"));
}

#[tokio::test]
async fn global_widgets_update_everywhere_and_maintenance_closes_the_site() {
    let f = fixture().await;
    let owner = f.session().await;
    f.post("designed", "Designed", "Body words", "page", "public", &[]).await;
    f.post("soon", "Soon", "We open soon", "page", "public", &[]).await;
    let id = post_id(&f, "designed").await;
    let r = post(&f.state, "/builder/templates/new/", &owner, &[("kind", "widget"), ("name", "Shared button")]).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let gid: String = sqlx::query_scalar("SELECT id FROM builder_templates WHERE name = 'Shared button'").fetch_one(&f.state.db).await.unwrap();
    let layout = serde_json::json!({"sections": [{"cols": [{"items": [{"id": "g", "t": "global", "s": {"ref": gid}}]}]}]});
    let r = json(&f.state, &format!("/posts/{id}/builder/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": layout, "active": true})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(site(&f.state, "GET", "/designed/", None).await.body.contains("Get started"));
    let changed = serde_json::json!({"sections": [{"cols": [{"items": [{"id": "b", "t": "button", "s": {"text": "Changed everywhere"}}]}]}]});
    let r = json(&f.state, &format!("/builder/templates/{gid}/save/"), Some(&owner), Some(ORIGIN), &serde_json::json!({"layout": changed})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(site(&f.state, "GET", "/designed/", None).await.body.contains("Changed everywhere"));
    // maintenance
    let soon = post_id(&f, "soon").await;
    let r = post(&f.state, "/builder/settings/", &owner, &[("builder_enabled", "1"), ("builder_maintenance", "maintenance"), ("builder_maintenance_page", &soon)]).await;
    assert!(r.flash().contains("saved"), "{}", r.flash());
    let s = site(&f.state, "GET", "/designed/", None).await;
    assert_eq!(s.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(s.body.contains("We open soon"));
    assert!(s.headers.get("retry-after").is_some());
    let r = post(&f.state, "/builder/settings/", &owner, &[("builder_enabled", "1"), ("builder_maintenance", "off")]).await;
    assert!(r.flash().contains("saved"));
    assert_eq!(site(&f.state, "GET", "/designed/", None).await.status, StatusCode::OK);
}

/// Not a test: serves the real admin and site on 127.0.0.1:1980 with sample
/// builder content and an owner session, for headless screenshots.
/// `cargo test -p vuuut-admin-ui --test builder serve_for_screenshots -- --ignored`
/// writes the session cookie to /tmp/vb-serve.json and serves for 150s.
#[tokio::test]
#[ignore]
async fn serve_for_screenshots() {
    let f = fixture().await;
    let owner = f.session().await;
    for (slug, title) in [("landing", "Landing"), ("one", "First story"), ("two", "Second story"), ("three", "Third story")] {
        f.post(slug, title, "Some words worth reading.", if slug == "landing" { "page" } else { "post" }, "public", &["news"]).await;
    }
    let id = post_id(&f, "landing").await;
    sqlx::query("UPDATE posts SET show_title_and_feature_image = 0 WHERE id = ?").bind(&id).execute(&f.state.db).await.unwrap();
    let uuid: String = sqlx::query_scalar("SELECT uuid FROM posts WHERE id = ?").bind(&id).fetch_one(&f.state.db).await.unwrap();
    f.post("a-draft", "A draft", "Draft words.", "post", "public", &[]).await;
    let draft = post_id(&f, "a-draft").await;
    sqlx::query("UPDATE posts SET status = 'draft', lexical = ? WHERE id = ?")
        .bind(r#"{"root":{"children":[{"children":[{"detail":0,"format":0,"mode":"normal","style":"","text":"Draft words.","type":"extended-text","version":1}],"direction":"ltr","format":"","indent":0,"type":"paragraph","version":1}],"direction":"ltr","format":"","indent":0,"type":"root","version":1}}"#)
        .bind(&draft).execute(&f.state.db).await.unwrap();
    sqlx::query("UPDATE posts SET lexical = ? WHERE id = ?")
        .bind(r#"{"root":{"children":[{"children":[{"detail":0,"format":0,"mode":"normal","style":"","text":"Some words worth reading.","type":"extended-text","version":1}],"direction":"ltr","format":"","indent":0,"type":"paragraph","version":1}],"direction":"ltr","format":"","indent":0,"type":"root","version":1}}"#)
        .bind(&id).execute(&f.state.db).await.unwrap();
    // the user's real post, when it has been exported for a reproduction
    let one = post_id(&f, "one").await;
    if let (Ok(lex), Ok(html)) = (std::fs::read_to_string("/tmp/vb-post-lexical.json"), std::fs::read_to_string("/tmp/vb-post-html.html")) {
        sqlx::query("UPDATE posts SET lexical = ?, html = ? WHERE id = ?").bind(lex.trim()).bind(html.trim()).bind(&one).execute(&f.state.db).await.unwrap();
    }
    let mut state = f.state.clone();
    let config = serde_json::from_value(serde_json::json!({"url": "http://127.0.0.1:1980", "server": {}, "database": {"connection": {"filename": ":memory:"}}, "paths": {}})).unwrap();
    state.config = Arc::new(config);
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!("http://127.0.0.1:1980")),
        ("title".into(), serde_json::json!("Builder demo")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
        ("navigation".into(), serde_json::json!([{"label": "Home", "url": "/"}, {"label": "Stories", "url": "/one/"}, {"label": "About", "url": "/landing/"}])),
    ]));
    state.settings = settings;
    let admin = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let app = vuuut_front::site_router(state.clone()).nest("/ghost", admin);
    std::fs::write("/tmp/vb-serve.json", serde_json::json!({"cookie": owner, "post": id, "uuid": uuid, "draft": draft, "real": one}).to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1980").await.unwrap();
    let server = axum::serve(listener, app);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(150), server).await;
}
