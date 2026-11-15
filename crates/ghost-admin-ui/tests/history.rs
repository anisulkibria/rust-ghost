//! Post history and Page history end to end: save a post twice, find the
//! earlier version in the editor's panel and on the screen, read it, restore
//! it, and confirm the restore is itself undoable.
//!
//! The refusals come first. A history names drafts and scheduled entries and
//! can rewrite a published post's text, so who may reach it is the part worth
//! pinning hardest — in particular that restoring a snapshot cannot be used
//! to write code injection or an author list somebody was never allowed to
//! type.
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
    async fn session_for(&self, user_id: &str) -> String {
        let id = ghost_db::repos::sessions::create(&self.state.db, user_id).await.unwrap();
        format!("ghost-admin-api-session={id}")
    }

    async fn session(&self) -> String {
        self.session_for(&self.owner).await
    }

    /// A staff member with the named role, and a session for them.
    async fn staff(&self, role: &str) -> (String, String) {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO users (id, name, slug, password, email, status, created_at) \
             VALUES (?, ?, ?, 'unused', ?, 'active', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(format!("Ada the {role}"))
        .bind(format!("a-{}-{id}", role.to_lowercase()))
        .bind(format!("{}-{id}@example.test", role.to_lowercase()))
        .execute(&self.state.db)
        .await
        .unwrap();
        let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ? LIMIT 1")
            .bind(role)
            .fetch_one(&self.state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
            .bind(ghost_core::ids::new_id())
            .bind(&role_id)
            .bind(&id)
            .execute(&self.state.db)
            .await
            .unwrap();
        let cookie = self.session_for(&id).await;
        (id, cookie)
    }

    /// A published post owned by `author`, with a body and a title.
    async fn post(&self, kind: &str, slug: &str, title: &str, html: &str, author: &str) -> String {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, lexical, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, ?, 'words', '{}', ?, 'published', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(html)
        .bind(kind)
        .execute(&self.state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, 0)")
            .bind(ghost_core::ids::new_id())
            .bind(&id)
            .bind(author)
            .execute(&self.state.db)
            .await
            .unwrap();
        id
    }

    /// Edit through the repo, which is what writes the revision — the same
    /// call the editor's save makes.
    ///
    /// `explicit_save` because this stands in for somebody pressing Save. An
    /// autosave would fold into the version already banked unless ten minutes
    /// had passed (Ghost's rule, in `ghost_core::revisions`), and the title
    /// has to actually change for a version to be earned at all — the title
    /// is one of the four fields that rule watches.
    async fn edit(&self, id: &str, title: &str, html: &str, editor: Option<&str>) {
        ghost_db::repos::posts::update(
            &self.state.db,
            id,
            &ghost_db::repos::posts::PostChanges {
                title: Some(title.into()),
                html: Some(html.into()),
                editor_id: editor.map(str::to_string),
                explicit_save: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    async fn title_of(&self, id: &str) -> String {
        sqlx::query_scalar::<_, Option<String>>("SELECT title FROM posts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.state.db)
            .await
            .unwrap()
            .unwrap_or_default()
    }

    async fn revision_ids(&self, id: &str) -> Vec<String> {
        ghost_db::repos::publishing::revisions(&self.state.db, id, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
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
        ("title".into(), serde_json::json!("History test")),
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
) -> Reply {
    let mut request =
        Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let location =
        response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &ghost_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "GET", uri, Some(cookie), None).await
}

async fn post(state: &ghost_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "POST", uri, Some(cookie), Some(ORIGIN)).await
}

#[tokio::test]
async fn the_history_is_closed_to_anyone_who_may_not_edit_the_entry() {
    let f = fixture().await;
    let id = f.post("post", "a-post", "A post", "<p>first</p>", &f.owner).await;
    f.edit(&id, "A post, revised", "<p>second</p>", Some(&f.owner)).await;
    let revision = f.revision_ids(&id).await[0].clone();

    // Nobody at all.
    let anon = send(&f.state, "GET", &format!("/posts/{id}/history/"), None, None).await;
    assert!(anon.status.is_redirection() || anon.status == StatusCode::UNAUTHORIZED, "{}", anon.status);

    // A Contributor who does not own the post.
    let (_, contributor) = f.staff("Contributor").await;
    let refused = get(&f.state, &format!("/posts/{id}/history/"), &contributor).await;
    assert_ne!(refused.status, StatusCode::OK, "a Contributor opened someone else's history");

    // And a restore needs the origin as well as the session.
    let forged = send(
        &f.state,
        "POST",
        &format!("/posts/{id}/history/{revision}/restore/"),
        Some(&f.session().await),
        Some("https://elsewhere.example"),
    )
    .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert_eq!(f.title_of(&id).await, "A post, revised", "nothing was restored");
}

#[tokio::test]
async fn an_id_that_is_not_an_object_id_never_reaches_a_query() {
    // Both the post id and the revision id arrive from the URL and go into a
    // query and an href.
    let f = fixture().await;
    let cookie = f.session().await;
    for uri in [
        "/posts/..%2f..%2fsettings/history/",
        "/posts/not-an-id/history/",
        "/posts/6553f7a1b2c3d4e5f6a7b8c9/history/not-an-id/restore/",
    ] {
        let reply = send(&f.state, if uri.ends_with("restore/") { "POST" } else { "GET" }, uri, Some(&cookie), Some(ORIGIN)).await;
        assert_ne!(reply.status, StatusCode::OK, "{uri}");
    }
}

#[tokio::test]
async fn the_editor_offers_the_history_once_there_is_one() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("post", "a-post", "A post", "<p>first</p>", &f.owner).await;

    // A post nobody has edited yet: the panel is there and says so, with
    // nothing to open.
    let fresh = get(&f.state, &format!("/posts/{id}/"), &cookie).await;
    assert_eq!(fresh.status, StatusCode::OK);
    assert!(fresh.body.contains("Post history"), "the panel is missing");
    assert!(fresh.body.contains("No earlier versions yet"), "{}", fresh.body);

    // After two edits it lists them and links to the screen.
    f.edit(&id, "A post, revised", "<p>second</p>", Some(&f.owner)).await;
    f.edit(&id, "A post, revised twice", "<p>third</p>", Some(&f.owner)).await;
    let edited = get(&f.state, &format!("/posts/{id}/"), &cookie).await;
    assert!(edited.body.contains("See all 2 versions"), "{}", edited.body);
    assert!(edited.body.contains(&format!("/ghost/posts/{id}/history/")), "{}", edited.body);

    // A brand-new post has no panel at all — there is no id to have a
    // history under yet.
    let new = get(&f.state, "/posts/new/", &cookie).await;
    assert!(!new.body.contains("Post history"), "a new post should not offer a history");
}

#[tokio::test]
async fn a_page_gets_a_page_history_pointing_at_the_pages_editor() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("page", "colophon", "About", "<p>first</p>", &f.owner).await;
    f.edit(&id, "About, revised", "<p>second</p>", Some(&f.owner)).await;

    let editor = get(&f.state, &format!("/pages/{id}/"), &cookie).await;
    assert!(editor.body.contains("Page history"), "{}", editor.body);
    assert!(editor.body.contains(&format!("/ghost/pages/{id}/history/")), "{}", editor.body);

    let screen = get(&f.state, &format!("/pages/{id}/history/"), &cookie).await;
    assert_eq!(screen.status, StatusCode::OK);
    assert!(screen.body.contains("<h1>Page history</h1>"), "{}", screen.body);
    // Every link back goes to the pages editor, never the posts one.
    assert!(screen.body.contains(&format!(r#"href="/ghost/pages/{id}/""#)), "{}", screen.body);
    assert!(!screen.body.contains(&format!("/ghost/posts/{id}/")), "{}", screen.body);
}

#[tokio::test]
async fn the_screen_shows_what_a_version_said_and_who_saved_it() {
    let f = fixture().await;
    let cookie = f.session().await;
    let (editor_id, _) = f.staff("Editor").await;
    let id = f.post("post", "a-post", "The first title", "<p>The first body</p>", &f.owner).await;
    f.edit(&id, "The second title", "<p>The second body</p>", Some(&editor_id)).await;

    let screen = get(&f.state, &format!("/posts/{id}/history/"), &cookie).await;
    assert_eq!(screen.status, StatusCode::OK);
    // The snapshot is of the state being replaced, so the newest revision
    // holds the *first* title and body.
    assert!(screen.body.contains("The first title"), "{}", screen.body);
    assert!(screen.body.contains("The first body"), "{}", screen.body);
    // Who saved it, which is what `author_id` was added for.
    assert!(screen.body.contains("Ada the Editor"), "the revision's author is missing: {}", screen.body);
    // And what restoring will and will not touch, said on the page.
    assert!(screen.body.contains("leaves the status, the publish date and the URL"), "{}", screen.body);
}

#[tokio::test]
async fn a_forged_revision_parameter_shows_the_newest_instead_of_reaching_the_query() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("post", "a-post", "The first title", "<p>first</p>", &f.owner).await;
    f.edit(&id, "The second title", "<p>second</p>", Some(&f.owner)).await;

    let reply = get(&f.state, &format!("/posts/{id}/history/?revision=%22+onload%3D%22alert(1)"), &cookie).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.body.contains("onload="), "{}", reply.body);
    assert!(reply.body.contains("The first title"), "it falls back to the newest version");
}

#[tokio::test]
async fn restoring_returns_the_text_and_is_itself_undoable() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("post", "a-post", "The first title", "<p>The first body</p>", &f.owner).await;
    f.edit(&id, "The second title", "<p>The second body</p>", Some(&f.owner)).await;
    assert_eq!(f.title_of(&id).await, "The second title");

    let revision = f.revision_ids(&id).await[0].clone();
    let restored = post(&f.state, &format!("/posts/{id}/history/{revision}/restore/"), &cookie).await;
    assert!(restored.flash().contains("has been restored"), "{}", restored.flash());
    // Post-redirect-get, back to the editor.
    assert!(restored.location.starts_with(&format!("/ghost/posts/{id}/?")), "{}", restored.location);
    assert_eq!(f.title_of(&id).await, "The first title");

    // The restore itself was a save, so the version it replaced is now a
    // version of its own — which is what makes the banner's promise true.
    let after = f.revision_ids(&id).await;
    assert_eq!(after.len(), 2, "the replaced version was not kept");
    let undo = post(&f.state, &format!("/posts/{id}/history/{}/restore/", after[0]), &cookie).await;
    assert!(undo.flash().contains("has been restored"));
    assert_eq!(f.title_of(&id).await, "The second title", "the restore was not undoable");

    // And it is recorded beside the ordinary edits in Settings → History.
    let events: Vec<String> =
        sqlx::query_scalar("SELECT event FROM actions WHERE resource_id = ? ORDER BY created_at")
            .bind(&id)
            .fetch_all(&f.state.db)
            .await
            .unwrap();
    assert!(events.iter().any(|e| e == "restored"), "{events:?}");
}

#[tokio::test]
async fn restoring_a_version_that_rewrites_code_injection_needs_an_editor() {
    // A snapshot carries code injection and an author list. Restoring one is
    // a way to write both, and an Author may type neither — so the snapshot
    // must not become the loophole.
    let f = fixture().await;
    let (author_id, author_cookie) = f.staff("Author").await;
    let id = f.post("post", "a-post", "A post", "<p>first</p>", &author_id).await;

    // An administrator adds code injection, writes a little more so the
    // injected state is banked as a version, then takes the injection away.
    // Code injection is not one of the four fields that earn a version on
    // their own, which is why the title change in between is needed.
    f.edit(&id, "A post, edited", "<p>second</p>", Some(&f.owner)).await;
    ghost_db::repos::posts::update(
        &f.state.db,
        &id,
        &ghost_db::repos::posts::PostChanges {
            codeinjection_head: Some("<script>injected()</script>".into()),
            editor_id: Some(f.owner.clone()),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    f.edit(&id, "A post, edited twice", "<p>third</p>", Some(&f.owner)).await;
    ghost_db::repos::posts::update(
        &f.state.db,
        &id,
        &ghost_db::repos::posts::PostChanges {
            clearing: vec!["codeinjection_head"],
            editor_id: Some(f.owner.clone()),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // The newest version is the one banked while the injection was in place.
    let injected = f.revision_ids(&id).await[0].clone();
    assert!(
        ghost_db::repos::publishing::revision_snapshot(&f.state.db, &id, &injected)
            .await
            .unwrap()
            .is_some_and(|s| s.post.codeinjection_head.is_some()),
        "the fixture did not bank a version carrying the injection"
    );

    // The author owns the post, so the history opens — but the restore does
    // not go through.
    assert_eq!(get(&f.state, &format!("/posts/{id}/history/"), &author_cookie).await.status, StatusCode::OK);
    let refused = post(&f.state, &format!("/posts/{id}/history/{injected}/restore/"), &author_cookie).await;
    assert!(refused.flash().contains("needs an Editor"), "{}", refused.flash());
    let head: Option<String> = sqlx::query_scalar("SELECT codeinjection_head FROM posts WHERE id = ?")
        .bind(&id)
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    assert!(head.is_none_or(|h| h.is_empty()), "the injection was written anyway");

    // An administrator restoring the same version is allowed to.
    let allowed = post(&f.state, &format!("/posts/{id}/history/{injected}/restore/"), &f.session().await).await;
    assert!(allowed.flash().contains("has been restored"), "{}", allowed.flash());
}

#[tokio::test]
async fn restoring_leaves_the_status_the_date_and_the_url_alone() {
    // The whole point of the repo's restore rule: recovering the text of a
    // published post must not unpublish it or move its URL out from under
    // whatever is already linking to it.
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("post", "the-slug", "A post", "<p>first</p>", &f.owner).await;
    f.edit(&id, "A post, revised", "<p>second</p>", Some(&f.owner)).await;
    sqlx::query("UPDATE posts SET slug = 'the-new-slug' WHERE id = ?")
        .bind(&id)
        .execute(&f.state.db)
        .await
        .unwrap();

    let revision = f.revision_ids(&id).await[0].clone();
    post(&f.state, &format!("/posts/{id}/history/{revision}/restore/"), &cookie).await;

    let (status, slug, published): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, slug, published_at FROM posts WHERE id = ?")
            .bind(&id)
            .fetch_one(&f.state.db)
            .await
            .unwrap();
    assert_eq!(status, "published");
    assert_eq!(slug.as_deref(), Some("the-new-slug"), "the URL moved");
    assert!(published.is_some(), "the publish date was cleared");
    assert_eq!(f.title_of(&id).await, "A post", "the text did not come back");
}

#[tokio::test]
async fn a_version_that_no_longer_exists_says_so_rather_than_failing() {
    let f = fixture().await;
    let cookie = f.session().await;
    let id = f.post("post", "a-post", "A post", "<p>first</p>", &f.owner).await;
    f.edit(&id, "A post, revised", "<p>second</p>", Some(&f.owner)).await;

    let gone = ghost_core::ids::new_id();
    let reply = post(&f.state, &format!("/posts/{id}/history/{gone}/restore/"), &cookie).await;
    assert!(!reply.flash().is_empty(), "it should redirect with a message");
    assert!(reply.flash().contains("could not be restored") || reply.flash().contains("no longer exists"), "{}", reply.flash());
    assert_eq!(f.title_of(&id).await, "A post, revised", "nothing changed");
}
