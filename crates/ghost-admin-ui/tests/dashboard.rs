//! The admin home end to end: sign in, land on it, and see the right things.
//!
//! The refusals come first, as the module pattern asks — but this module's
//! refusals are a different shape from every other one's. The dashboard is
//! where `/ghost/` lands, so it must *not* be closed below Administrator:
//! refusing it would mean a Contributor signs in to a "Not allowed" page.
//! What is closed is the content. So the first two tests are the pair that
//! matters:
//!
//! * nobody without a session gets in at all;
//! * a signed-in Author gets the page, and does not get the Administrator-only
//!   sections or any link into them.
//!
//! The third guarantee worth having is the author narrowing: a writer's home
//! screen shows a writer's own drafts, never the whole site's. That is tested
//! from both sides, because a scoping bug that leaks is invisible until
//! somebody notices a draft they should not have seen.
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
        self.session_for(&self.owner).await
    }

    async fn session_for(&self, user_id: &str) -> String {
        let id = ghost_db::repos::sessions::create(&self.state.db, user_id).await.unwrap();
        format!("vuuut-admin-session={id}")
    }

    /// A staff member in `role`, returned as their user id.
    async fn staff(&self, role: &str, name: &str) -> String {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO users (id, name, slug, password, email, status, created_at) \
             VALUES (?, ?, ?, 'unused', ?, 'active', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(name)
        .bind(format!("staff-{id}"))
        .bind(format!("staff-{id}@example.test"))
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
        id
    }

    /// A post in `status`, attributed to `author`, returned as its id.
    ///
    /// `at` is both the publish time and the last edit, which is all either
    /// card reads.
    async fn post(&self, author: &str, status: &str, title: &str, at: &str) -> String {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, 'post', ?, 'public', ?, ?, ?)",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
        .bind(title)
        .bind(format!("slug-{id}"))
        .bind(status)
        .bind(at)
        .bind(at)
        .bind(at)
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

    /// A member who reported a comment on `post`, so the moderation queue has
    /// something in it.
    async fn reported_comment(&self, post: &str) {
        let member = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO members (id, uuid, transient_id, email, status, email_count, email_opened_count, created_at) \
             VALUES (?, ?, ?, ?, 'free', 0, 0, '2026-11-01 00:00:00')",
        )
        .bind(&member)
        .bind(ghost_core::ids::uuid_v4())
        // `transient_id` is NOT NULL UNIQUE: it is the handle a member's own
        // sessions are keyed by, and rotating it signs them all out.
        .bind(ghost_core::ids::uuid_v4())
        .bind(format!("reader-{member}@example.test"))
        .execute(&self.state.db)
        .await
        .unwrap();
        let comment = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO comments (id, post_id, member_id, html, status, created_at, updated_at) \
             VALUES (?, ?, ?, '<p>rude</p>', 'published', '2026-11-02 00:00:00', '2026-11-02 00:00:00')",
        )
        .bind(&comment)
        .bind(post)
        .bind(&member)
        .execute(&self.state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO comment_reports (id, comment_id, member_id, created_at) \
             VALUES (?, ?, ?, '2026-11-03 00:00:00')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&comment)
        .bind(&member)
        .execute(&self.state.db)
        .await
        .unwrap();
    }
}

async fn fixture() -> Fixture {
    // A site with everything on the setup checklist already done, so a test
    // about something else is not reading a page full of checklist rows.
    fixture_with(vec![
        ("description".into(), serde_json::json!("A site about things")),
        ("icon".into(), serde_json::json!("/content/images/icon.png")),
        ("web_analytics".into(), serde_json::json!(true)),
    ])
    .await
}

async fn fixture_with(extra_settings: Vec<(String, serde_json::Value)>) -> Fixture {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = ghost_db::repos::users::setup_owner(&db, "Olive Owner", "owner@example.test", "unused")
        .await
        .unwrap();
    let mut values: Vec<(String, serde_json::Value)> = vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Dashboard test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ];
    values.extend(extra_settings);
    let settings = Arc::new(ghost_db::SettingsCache::from_values(values));
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

async fn send(
    state: &ghost_api::AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
) -> Reply {
    let mut request =
        Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
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
    send(state, "GET", uri, Some(cookie)).await
}

#[tokio::test]
async fn nobody_without_a_session_gets_in() {
    let f = fixture().await;
    let anon = send(&f.state, "GET", "/dashboard/", None).await;
    assert!(
        anon.status.is_redirection() || anon.status == StatusCode::UNAUTHORIZED,
        "the dashboard answered {} to nobody",
        anon.status
    );
    assert!(!anon.body.contains("Pick up where you left off"), "{}", anon.body);
}

/// The page opens for every signed-in role — that is the whole point of it
/// being where `/ghost/` lands — but a Contributor is shown none of the
/// Administrator-only sections and no link into one.
#[tokio::test]
async fn a_contributor_gets_the_page_but_none_of_the_administrators_sections() {
    let f = fixture().await;
    let contributor = f.staff("Contributor", "Cora Contributor").await;
    let post = f.post(&f.owner.clone(), "published", "Public post", "2026-11-01 00:00:00").await;
    f.reported_comment(&post).await;

    let reply = get(&f.state, "/dashboard/", &f.session_for(&contributor).await).await;
    assert_eq!(reply.status, StatusCode::OK);
    // The page itself, and the actions anybody may take.
    assert!(reply.body.contains("Pick up where you left off"), "{}", reply.body);
    assert!(reply.body.contains("/ghost/posts/new/"), "{}", reply.body);

    // None of the Administrator-only sections, and nothing linking into them.
    // `/ghost/settings/` is deliberately not on this list: the rail's account
    // block links a signed-in person to their *own* profile at
    // `/ghost/staffs/<slug>/`, which every role may open. The three
    // settings screens the checklist would have linked to are covered by
    // "Finish setting up" not being here.
    for forbidden in [
        "Needs you",
        "reported comment",
        "Your publication",
        "Finish setting up",
        // The module reports are Administrator-only, like every screen they
        // link to — a writer has no business with the posture score or with
        // what is stopping a payment.
        "Your modules report",
        "/ghost/security/",
        "/ghost/memberships/",
        "/ghost/newsletters/",
        "/ghost/media/",
        "/ghost/comments/",
        "/ghost/settings/general/",
        "/ghost/settings/analytics/",
        "MRR",
    ] {
        assert!(
            !reply.body.contains(forbidden),
            "a Contributor was shown {forbidden:?}:\n{}",
            reply.body
        );
    }
}

/// The administrator sees the queue the contributor did not.
#[tokio::test]
async fn an_administrator_is_told_what_is_waiting() {
    let f = fixture().await;
    let post = f.post(&f.owner.clone(), "published", "Public post", "2026-11-01 00:00:00").await;
    f.reported_comment(&post).await;

    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("Needs you"), "{}", reply.body);
    assert!(reply.body.contains("1 reported comment"), "{}", reply.body);
    assert!(reply.body.contains("filter=reported"), "{}", reply.body);
    assert!(reply.body.contains("Your publication"), "{}", reply.body);
}

/// A quiet site shows no "Needs you" card at all. The negative is the one
/// worth testing: a card of four zeroes is what teaches people to skip it.
#[tokio::test]
async fn a_site_with_nothing_waiting_shows_no_queue_card() {
    let f = fixture().await;
    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.body.contains("Needs you"), "{}", reply.body);
}

/// An Author's home screen lists an Author's own work and nobody else's.
#[tokio::test]
async fn a_writer_sees_their_own_drafts_and_not_everybody_elses() {
    let f = fixture().await;
    let author = f.staff("Author", "Ada Author").await;
    f.post(&author, "draft", "Mine in progress", "2026-11-10 00:00:00").await;
    f.post(&f.owner.clone(), "draft", "The owners secret", "2026-11-11 00:00:00").await;

    let theirs = get(&f.state, "/dashboard/", &f.session_for(&author).await).await;
    assert_eq!(theirs.status, StatusCode::OK);
    assert!(theirs.body.contains("Mine in progress"), "{}", theirs.body);
    assert!(
        !theirs.body.contains("The owners secret"),
        "an Author was shown somebody else's draft:\n{}",
        theirs.body
    );

    // The owner, who may edit anything, sees both.
    let owners = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(owners.body.contains("Mine in progress"), "{}", owners.body);
    assert!(owners.body.contains("The owners secret"), "{}", owners.body);
}

/// Scheduled work leads the queue, because it has a deadline and a draft does
/// not — even when the draft was touched more recently.
#[tokio::test]
async fn scheduled_work_comes_before_drafts_however_recently_they_were_edited() {
    let f = fixture().await;
    let owner = f.owner.clone();
    f.post(&owner, "scheduled", "Goes out Friday", "2026-12-01 09:00:00").await;
    f.post(&owner, "draft", "Edited a moment ago", "2026-11-15 23:59:00").await;

    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    let scheduled_at = reply.body.find("Goes out Friday").expect("scheduled entry missing");
    let draft_at = reply.body.find("Edited a moment ago").expect("draft missing");
    assert!(scheduled_at < draft_at, "{}", reply.body);
    // And it says when, in words, rather than only chipping it "Scheduled".
    assert!(reply.body.contains("Goes out 1 Dec 2026 at 09:00"), "{}", reply.body);
}

/// A site with nothing in progress offers to start something. The empty state
/// is the most useful thing on a new site's home screen.
#[tokio::test]
async fn an_empty_queue_offers_a_way_to_start() {
    let f = fixture().await;
    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(reply.body.contains("All caught up"), "{}", reply.body);
    assert!(reply.body.contains("Start a post"), "{}", reply.body);
}

/// The checklist appears while the site is unfinished and disappears when it
/// is not — both halves, because a checklist that never goes away is furniture.
#[tokio::test]
async fn the_setup_checklist_appears_only_while_something_is_undone() {
    let bare = fixture_with(vec![]).await;
    // The seed ships a published welcome post, so a genuinely empty site has
    // to be made by clearing it — and that is the state "Publish your first
    // post" is for.
    sqlx::query("DELETE FROM posts").execute(&bare.state.db).await.unwrap();

    let reply = get(&bare.state, "/dashboard/", &bare.session().await).await;
    assert!(reply.body.contains("Finish setting up"), "{}", reply.body);
    assert!(reply.body.contains("Publish your first post"), "{}", reply.body);
    assert!(reply.body.contains("Say what this site is about"), "{}", reply.body);
    assert!(reply.body.contains("Turn on web analytics"), "{}", reply.body);
    // Nothing is *for sale* on a fresh site, so Stripe is not asked for — even
    // though the seed does create an active paid tier. It sits at
    // `visibility = 'none'`, which is the difference between a tier existing
    // and a tier being on offer. Getting this wrong would nag every new site
    // about a payment processor it has no use for.
    assert!(!reply.body.contains("Connect Stripe"), "{}", reply.body);

    let done = fixture().await;
    done.post(&done.owner.clone(), "published", "Something", "2026-11-01 00:00:00").await;
    let reply = get(&done.state, "/dashboard/", &done.session().await).await;
    assert!(!reply.body.contains("Finish setting up"), "{}", reply.body);

    // Put the paid tier on offer and Stripe becomes the one thing left to do.
    sqlx::query("UPDATE products SET visibility = 'public' WHERE type = 'paid'")
        .execute(&done.state.db)
        .await
        .unwrap();
    let reply = get(&done.state, "/dashboard/", &done.session().await).await;
    // ...and the Memberships module answers it, not this checklist. That
    // module measures the whole funnel — no Stripe, no plan in the portal —
    // so a one-line "Connect Stripe" above its own row would be the same
    // request made twice, worse, two sections apart.
    assert!(
        reply.body.contains("somebody joining or paying"),
        "Memberships did not report the shop it cannot run:\n{}",
        reply.body
    );
    assert!(reply.body.contains(r#"href="/ghost/memberships/""#), "{}", reply.body);
    assert!(
        !reply.body.contains("Connect Stripe"),
        "the checklist repeated what Memberships already said:\n{}",
        reply.body
    );
}

/// The modules that audit themselves are reported on the home screen as a
/// count and a link, and never as a copy of their findings.
///
/// A seeded site genuinely cannot take a payment and genuinely cannot send an
/// email — the Memberships and Newsletters modules each pin that about
/// themselves — so this is the state a fresh install is actually in, and the
/// home screen saying so is the point of the section.
#[tokio::test]
async fn the_home_screen_reports_what_the_modules_found() {
    let f = fixture().await;
    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("Your modules report"), "{}", reply.body);

    // Newsletters: a seeded site has no working bulk transport, so a post
    // emailed today reaches nobody — and that outranks the shop below it.
    assert!(reply.body.contains("the newsletter from reaching anybody"), "{}", reply.body);
    assert!(reply.body.contains(r#"href="/ghost/newsletters/""#), "{}", reply.body);
    let sending_at = reply.body.find("the newsletter from reaching").unwrap();
    let modules_at = reply.body.find("Your modules report").unwrap();
    assert!(modules_at < sending_at, "{}", reply.body);

    // And the words belong to the modules: the home screen counts, it does not
    // reproduce a finding.
    assert!(!reply.body.contains("gap-no-transport"), "{}", reply.body);
}

/// `/ghost/` is the dashboard, which is the change this module exists to make.
/// Signed out it still goes to sign-in.
#[tokio::test]
async fn the_admin_root_lands_on_the_dashboard() {
    let f = fixture().await;
    let signed_in = get(&f.state, "/", &f.session().await).await;
    assert!(signed_in.status.is_redirection(), "{}", signed_in.status);
    assert_eq!(signed_in.location, "/ghost/dashboard/");

    let anon = send(&f.state, "GET", "/", None).await;
    assert_eq!(anon.location, "/ghost/signin/");
}

/// The rail's brand link leads home, and it is the *only* thing that does.
///
/// There was a Dashboard nav item under the brand until 2026-11-16, one line
/// below a brand mark pointing at the same URL. Two controls for one
/// destination is not redundancy worth keeping, so the row went and the brand
/// is now the way home.
#[tokio::test]
async fn the_rail_leads_home_once() {
    let f = fixture().await;
    let reply = get(&f.state, "/posts/", &f.session().await).await;
    assert!(
        reply.body.contains(r#"<a class="brand" href="/ghost/dashboard/""#),
        "the brand link does not lead home:\n{}",
        reply.body
    );
    assert!(
        !reply.body.contains(r#"class="nav-item" href="/ghost/dashboard/""#),
        "the Dashboard nav item is back beside the brand:\n{}",
        reply.body
    );
    assert_eq!(
        reply.body.matches("/ghost/dashboard/").count(),
        1,
        "something other than the brand links home:\n{}",
        reply.body
    );
}

/// A published post is reported on with the reader count it actually earned,
/// counted by path rather than by a slug pasted into SQL.
#[tokio::test]
async fn recently_published_carries_its_reader_count() {
    let f = fixture().await;
    let id = f.post(&f.owner.clone(), "published", "A popular post", "2026-11-01 00:00:00").await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM posts WHERE id = ?")
        .bind(&id)
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for viewer in ["one", "two", "two"] {
        sqlx::query(
            "INSERT INTO page_views (id, path, viewer, created_at, audience) VALUES (?, ?, ?, ?, 'public')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(format!("/{slug}/"))
        .bind(viewer)
        .bind(&now)
        .execute(&f.state.db)
        .await
        .unwrap();
    }

    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(reply.body.contains("Recently published"), "{}", reply.body);
    assert!(reply.body.contains("A popular post"), "{}", reply.body);
    // Two distinct viewers, three hits: readers, not page loads.
    assert!(reply.body.contains("2 readers"), "{}", reply.body);
    assert!(reply.body.contains(&format!("/ghost/posts/{id}/analytics/")), "{}", reply.body);
}

/// A title carrying markup survives the whole round trip escaped. The unit
/// tests assert this on the row builders; this asserts it on the real page,
/// because an escape can be undone by anything between.
#[tokio::test]
async fn a_hostile_title_is_escaped_on_the_rendered_page() {
    let f = fixture().await;
    f.post(&f.owner.clone(), "draft", "<script>alert(1)</script>", "2026-11-10 00:00:00").await;
    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(!reply.body.contains("<script>alert(1)"), "{}", reply.body);
    assert!(reply.body.contains("&lt;script&gt;alert(1)"), "{}", reply.body);
}

/// A scheduled post whose time has passed is the most serious thing this
/// screen reports, and it is reported as a fault rather than as a chore.
#[tokio::test]
async fn an_overdue_scheduled_post_is_reported_as_a_fault() {
    let f = fixture().await;
    f.post(&f.owner.clone(), "scheduled", "Should have gone out", "2020-01-01 09:00:00").await;

    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(reply.body.contains("Needs you"), "{}", reply.body);
    assert!(reply.body.contains("1 scheduled post"), "{}", reply.body);
    assert!(reply.body.contains("scheduler may have stopped"), "{}", reply.body);
    assert!(reply.body.contains("chip error"), "{}", reply.body);
}

/// The greeting addresses the person who is signed in, by first name only.
#[tokio::test]
async fn the_page_greets_whoever_is_signed_in() {
    let f = fixture().await;
    let reply = get(&f.state, "/dashboard/", &f.session().await).await;
    assert!(reply.body.contains(", Olive</h1>"), "{}", reply.body);
    assert!(!reply.body.contains("Olive Owner</h1>"), "{}", reply.body);
    assert!(reply.body.contains("Dashboard test"), "the site name is missing:\n{}", reply.body);
}
