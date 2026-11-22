//! Tools → Workflow & Editorial Management end to end: put ideas on the
//! board, move them, assign them, note and tick them — and see the approval
//! gate refuse an unapproved publish in the editor's own save path, then let
//! it through once an editor approves. Refusals first, then the writer's
//! narrower view.
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
        ("title".into(), serde_json::json!("Workflow test")),
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




async fn draft(f: &Fixture, title: &str, author: &str) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at) VALUES (?, ?, ?, ?, '<p>x</p>', 'x', 'post', 'draft', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
        .bind(&id).bind(ghost_core::ids::uuid_v4()).bind(title).bind(format!("s-{id}"))
        .execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, 0)")
        .bind(ghost_core::ids::new_id()).bind(&id).bind(author)
        .execute(&f.state.db).await.unwrap();
    id
}

/// The newest staff member made by `author_session`.
async fn newest_author(f: &Fixture) -> String {
    sqlx::query_scalar("SELECT id FROM users WHERE email LIKE 'author-%' ORDER BY id DESC LIMIT 1")
        .fetch_one(&f.state.db).await.unwrap()
}

async fn json_move(f: &Fixture, cookie: &str, id: &str, to: &str) -> (StatusCode, String) {
    let request = Request::builder().method("POST").uri(format!("/workflows/items/{id}/move/"))
        .header("host", HOST).header("cookie", cookie).header("origin", ORIGIN)
        .header("accept", "application/json").header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("to={to}"))).unwrap();
    let app = ghost_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_tool_is_closed_to_strangers_and_to_roles_nobody_granted_it() {
    let f = fixture().await;
    for uri in ["/workflows/", "/workflows/calendar/", "/workflows/settings/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        let author = get(&f.state, uri, &f.author_session().await).await;
        assert_ne!(author.status, StatusCode::OK, "{uri} opened for an Author nobody granted it to");
    }
    let cookie = f.session().await;
    let forged = send(&f.state, "POST", "/workflows/ideas/", Some(&cookie), Some("https://elsewhere.example"), Some(&[("title", "x")])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_idea_is_created_moved_assigned_noted_and_ticked() {
    let f = fixture().await;
    let cookie = f.session().await;
    let me = f.owner.clone();
    let r = post(&f.state, "/workflows/ideas/", &cookie, &[("title", "A <b>bold</b> idea")]).await;
    assert!(r.flash().contains("added"), "{}", r.location);
    let board = get(&f.state, "/workflows/", &cookie).await;
    assert!(board.body.contains("A &lt;b&gt;bold&lt;/b&gt; idea"), "{}", board.body);
    let id: String = sqlx::query_scalar("SELECT id FROM posts WHERE title = 'A <b>bold</b> idea'").fetch_one(&f.state.db).await.unwrap();
    let item = ghost_db::repos::workflows::entry(&f.state.db, &id).await.unwrap().unwrap();
    assert_eq!(item.stage.as_deref(), Some("idea"));

    // The arrow form, then the board script's JSON.
    let moved = post(&f.state, &format!("/workflows/items/{id}/move/"), &cookie, &[("to", "writing")]).await;
    assert!(moved.flash().contains("Writing"), "{}", moved.location);
    let (status, body) = json_move(&f, &cookie, &id, "review").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = json_move(&f, &cookie, &id, "published").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a status is not a stage");

    // Assign, date, prioritise; forged people are dropped.
    post(&f.state, &format!("/workflows/items/{id}/"), &cookie, &[
        ("stage", "review"), ("assignee_id", &me), ("reviewer_id", "ffffffffffffffffffffffff"),
        ("due_on", "2020-01-01"), ("priority", "urgent"),
    ]).await;
    let item = ghost_db::repos::workflows::entry(&f.state.db, &id).await.unwrap().unwrap();
    assert_eq!((item.assignee_id.as_deref(), item.reviewer_id.as_deref(), item.due_on.as_deref(), item.priority.as_deref()),
        (Some(me.as_str()), None, Some("2020-01-01"), Some("urgent")));
    let overdue = get(&f.state, "/workflows/?view=overdue", &cookie).await;
    assert!(overdue.body.contains("wf-due-over"), "{}", overdue.body);

    // A note (escaped), a tick, and the history that records them.
    post(&f.state, &format!("/workflows/items/{id}/notes/"), &cookie, &[("body", "<script>x</script> needs a source")]).await;
    let bad_tick = post(&f.state, &format!("/workflows/items/{id}/check/"), &cookie, &[("key", "not-on-the-list"), ("done", "1")]).await;
    assert!(bad_tick.location.contains("error="));
    post(&f.state, &format!("/workflows/items/{id}/check/"), &cookie, &[("key", "facts-checked"), ("done", "1")]).await;
    let page = get(&f.state, &format!("/workflows/items/{id}/"), &cookie).await;
    assert!(page.body.contains("&lt;script&gt;x&lt;/script&gt; needs a source"));
    assert!(!page.body.contains("<script>x</script>"));
    assert!(page.body.contains("Ideas → Writing") && page.body.contains("Writing → In review"), "{}", page.body);
    assert!(page.body.contains("ticked “Facts checked”"));
    assert!(page.body.contains("made it urgent priority"));

    // The calendar shows the due date.
    let cal = get(&f.state, "/workflows/calendar/?m=2020-01", &cookie).await;
    assert!(cal.body.contains("January 2020") && cal.body.contains("A &lt;b&gt;bold&lt;/b&gt; idea"), "{}", cal.body);
}

#[tokio::test]
async fn the_approval_gate_holds_in_the_editor_until_an_editor_approves() {
    let f = fixture().await;
    let cookie = f.session().await;
    let me = f.owner.clone();
    let id = draft(&f, "Gated", &me).await;
    post(&f.state, "/workflows/settings/", &cookie, &[
        ("count", "4"),
        ("key_0", "idea"), ("label_0", "Ideas"), ("color_0", "slate"), ("order_0", "1"),
        ("key_1", "writing"), ("label_1", "Writing"), ("color_1", "blue"), ("order_1", "2"),
        ("key_2", "review"), ("label_2", "In review"), ("color_2", "amber"), ("order_2", "3"),
        ("key_3", "ready"), ("label_3", "Approved"), ("color_3", "green"), ("order_3", "4"),
        ("workflow_require_approval", "1"), ("workflow_approval_stage", "ready"),
        // Administrators bypass by default; switched off so the Owner in this
        // fixture is held by the gate too.
        ("workflow_checklist", "Facts checked"),
    ]).await;
    let refused = post(&f.state, &format!("/posts/{id}/"), &cookie, &[("title", "Gated"), ("status", "published"), ("markdown", "x")]).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.body);
    assert!(refused.body.contains("needs approval"), "{}", refused.body);
    let status: String = sqlx::query_scalar("SELECT status FROM posts WHERE id = ?").bind(&id).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(status, "draft");

    let (s, _) = json_move(&f, &cookie, &id, "ready").await;
    assert_eq!(s, StatusCode::OK);
    let allowed = post(&f.state, &format!("/posts/{id}/"), &cookie, &[("title", "Gated"), ("status", "published"), ("markdown", "x")]).await;
    assert_ne!(allowed.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", allowed.body);
    let status: String = sqlx::query_scalar("SELECT status FROM posts WHERE id = ?").bind(&id).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(status, "published");
    // Published, it leaves the stages for the Published column.
    let board = get(&f.state, "/workflows/", &cookie).await;
    let published = board.body.split(r#"data-stage="published""#).nth(1).unwrap();
    assert!(published.contains("Gated"));
}

#[tokio::test]
async fn a_granted_writer_sees_only_their_work_and_cannot_approve_it() {
    let f = fixture().await;
    let owner = f.session().await;
    let me = f.owner.clone();
    // Grant Authors the tool, as an Owner would on the Permissions screen.
    f.state.settings.set(&f.state.db, "permissions", &serde_json::json!({
        "author": {"tools": {"*": "none", "workflows": "edit"}, "posts": "own", "pages": "own", "tags": "none"}
    })).await.unwrap();
    let writer = f.author_session().await;
    let writer_id = newest_author(&f).await;
    let theirs = draft(&f, "Writer's draft", &writer_id).await;
    let others = draft(&f, "Someone else's", &me).await;

    let board = get(&f.state, "/workflows/", &writer).await;
    assert_eq!(board.status, StatusCode::OK, "{}", board.body);
    assert!(board.body.contains("Writer&#39;s draft") || board.body.contains("Writer's draft"), "{}", board.body);
    assert!(!board.body.contains("Someone else"), "a writer saw another's draft");
    assert_ne!(get(&f.state, &format!("/workflows/items/{others}/"), &writer).await.status, StatusCode::OK);
    assert_ne!(get(&f.state, "/workflows/settings/", &writer).await.status, StatusCode::OK);

    let (s, _) = json_move(&f, &writer, &theirs, "review").await;
    assert_eq!(s, StatusCode::OK, "submitting is the writer's");
    let (s, body) = json_move(&f, &writer, &theirs, "ready").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("editors"), "{body}");
    let (s, _) = json_move(&f, &writer, &others, "review").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "moving another's work");
    let assigned = post(&f.state, &format!("/workflows/items/{theirs}/"), &writer, &[("priority", "urgent")]).await;
    assert!(assigned.location.contains("error="), "a writer set a priority");
}

#[tokio::test]
async fn removing_a_stage_rehomes_its_work() {
    let f = fixture().await;
    let cookie = f.session().await;
    let me = f.owner.clone();
    let id = draft(&f, "In review now", &me).await;
    json_move(&f, &cookie, &id, "review").await;
    let r = post(&f.state, "/workflows/settings/", &cookie, &[
        ("count", "4"),
        ("key_0", "idea"), ("label_0", "Ideas"), ("color_0", "slate"), ("order_0", "1"),
        ("key_1", "writing"), ("label_1", "Drafting"), ("color_1", "blue"), ("order_1", "2"),
        ("key_2", "review"), ("label_2", "In review"), ("color_2", "amber"), ("order_2", "3"), ("remove_2", "1"),
        ("key_3", "ready"), ("label_3", "Approved"), ("color_3", "green"), ("order_3", "4"),
        ("new_label", "Legal check"),
    ]).await;
    assert!(r.flash().contains("1 entry moved to Drafting"), "{}", r.flash());
    let s = ghost_core::workflows::Settings::read(|k| f.state.settings.get_str(k));
    let labels: Vec<&str> = s.stages.iter().map(|st| st.label.as_str()).collect();
    assert_eq!(labels, ["Ideas", "Drafting", "Approved", "Legal check"]);
    let item = ghost_db::repos::workflows::entry(&f.state.db, &id).await.unwrap().unwrap();
    assert_eq!(item.stage.as_deref(), Some("writing"));
}
