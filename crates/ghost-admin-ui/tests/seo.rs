//! The SEO module end to end: publish something imperfect, read the report
//! through the real router, filter it, change the settings, and see the
//! change reach the public site.
//!
//! The refusals come first, for the reason the module pattern gives: a report
//! that names every draft on the site, and a settings page that can take the
//! whole site out of the index, are not things an Author may open.
//!
//! The last test is the one worth having. Everything else checks that the
//! admin renders; `a_setting_saved_here_changes_what_the_site_serves` checks
//! that the switches are connected to anything at all, which is the failure
//! mode a settings page quietly has.
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
        format!("ghost-admin-api-session={id}")
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
        format!("ghost-admin-api-session={session}")
    }

    /// A published post with no meta description, no image and no tags —
    /// three findings, which is what most real posts look like.
    async fn bare_post(&self, slug: &str, title: &str) -> String {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>short</p>', 'just a few words here', 'post', 'published', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .execute(&self.state.db)
        .await
        .unwrap();
        id
    }

    /// A page with everything filled in, so the report has something to be
    /// quiet about.
    async fn good_page(&self, slug: &str, title: &str) {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, feature_image, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>x</p>', ?, '/content/images/hero.jpg', 'page', 'published', 'public', '2026-11-02 00:00:00', '2026-11-02 00:00:00', '2026-11-02 00:00:00')",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind("word ".repeat(400))
        .execute(&self.state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO posts_meta (id, post_id, meta_description, feature_image_alt) \
             VALUES (?, ?, ?, 'A photograph of the thing')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&id)
        .bind("A meta description of exactly the sort of length a search engine shows in full.")
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
        ("title".into(), serde_json::json!("SEO test")),
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

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let f = fixture().await;
    for uri in ["/seo/", "/seo/content/", "/settings/seo/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(
            anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN,
            "{uri} answered {} to nobody",
            anon.status
        );
        let author = get(&f.state, uri, &f.author_session().await).await;
        assert_ne!(author.status, StatusCode::OK, "{uri} opened for an Author");
    }
    // And a save needs both the session and the origin.
    let cookie = f.session().await;
    let forged = send(
        &f.state,
        "POST",
        "/settings/seo/",
        Some(&cookie),
        Some("https://elsewhere.example"),
        Some(&[("seo_structured_data", "1")]),
    )
    .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_overview_counts_the_findings_and_says_what_to_fix_first() {
    let f = fixture().await;
    f.bare_post("thin", "Thin").await;
    f.good_page("colophon", "About this site, in full").await;
    let cookie = f.session().await;

    let page = get(&f.state, "/seo/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    // How to run it again, and what "again" means — there is no stored
    // report, so the control is a link back here and the page says so.
    assert!(page.body.contains("Re-run audit"), "{}", page.body);
    assert!(page.body.contains(r#"href="/ghost/seo/">"#), "{}", page.body);
    assert!(page.body.contains("the audit runs again every time this page is opened"), "{}", page.body);
    assert!(page.body.contains("Checked at "), "{}", page.body);
    // The site's own setup, checked line by line.
    assert!(page.body.contains("Site setup"), "{}", page.body);
    assert!(page.body.contains("Home page description"));
    assert!(page.body.contains("Default social image"));
    // The content findings, counted and linked.
    assert!(page.body.contains("No meta description"), "{}", page.body);
    assert!(page.body.contains("No tags"));
    assert!(page.body.contains(r#"href="/ghost/seo/content/?issue=no_description""#), "{}", page.body);
    // The tiles are there, and the thin post drags the average below full.
    // (The count is the two above plus the two fixture entries a fresh
    // install seeds, which is exactly what a new site looks like.)
    assert!(page.body.contains(r#"<div class="label">Audited</div><div class="value">4</div>"#), "{}", page.body);
    assert!(!page.body.contains(r#"<div class="label">Average score</div><div class="value">100</div>"#));
}

#[tokio::test]
async fn the_content_screen_lists_the_worst_first_and_filters_by_finding() {
    let f = fixture().await;
    f.bare_post("thin", "Thin").await;
    f.good_page("colophon", "About this site, in full").await;
    let cookie = f.session().await;

    let all = get(&f.state, "/seo/content/", &cookie).await;
    assert_eq!(all.status, StatusCode::OK);
    let thin_at = all.body.find("Thin").expect("the thin post is listed");
    let good_at = all.body.find("About this site, in full").expect("the page is listed");
    assert!(thin_at < good_at, "the worst entry has to come first");
    // The score badge and the editor link.
    assert!(all.body.contains("ic seo-score"), "{}", all.body);
    // Re-running from here keeps the filters rather than dropping them.
    let filtered_rerun = get(&f.state, "/seo/content/?issue=no_tags&type=post", &cookie).await;
    assert!(
        // escaped on the way out, like every other interpolated value
        filtered_rerun.body.contains(r#"href="/ghost/seo/content/?issue=no_tags&amp;type=post&amp;state=all""#),
        "{}",
        filtered_rerun.body
    );
    assert!(all.body.contains("/ghost/posts/"), "{}", all.body);

    // Filtered to one finding, the complete page drops out.
    let filtered = get(&f.state, "/seo/content/?issue=no_description", &cookie).await;
    assert!(filtered.body.contains("Thin"));
    assert!(!filtered.body.contains("About this site, in full"), "{}", filtered.body);

    // Filtered by type, the post drops out.
    let pages = get(&f.state, "/seo/content/?type=page", &cookie).await;
    assert!(pages.body.contains("About this site, in full"));
    assert!(!pages.body.contains(">Thin<"), "{}", pages.body);

    // A forged filter shows everything rather than reaching the page.
    let forged = get(&f.state, "/seo/content/?issue=%22+onload%3D%22alert(1)", &cookie).await;
    assert_eq!(forged.status, StatusCode::OK);
    assert!(!forged.body.contains("onload="), "{}", forged.body);
    assert!(forged.body.contains("Thin"));
}

#[tokio::test]
async fn a_hostile_title_is_escaped_everywhere_the_report_shows_it() {
    let f = fixture().await;
    f.bare_post("x", r#"<img src=x onerror=alert(1)>"#).await;
    let cookie = f.session().await;
    for uri in ["/seo/", "/seo/content/"] {
        let page = get(&f.state, uri, &cookie).await;
        assert!(!page.body.contains("<img src=x"), "{uri} rendered it as markup");
    }
}

#[tokio::test]
async fn the_settings_page_offers_every_switch_and_saves_them() {
    let f = fixture().await;
    let cookie = f.session().await;

    let page = get(&f.state, "/settings/seo/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    for name in ghost_core::seo::BOOL_SETTINGS.iter().map(|(k, _)| k) {
        assert!(page.body.contains(&format!(r#"name="{name}""#)), "{name} is not on the page");
    }
    assert!(page.body.contains(r#"name="seo_robots_txt""#));
    assert!(page.body.contains(r#"name="indexnow_api_key""#));
    assert!(page.body.contains(r#"name="seo_referrer_policy""#));
    // The defaults are reflected: structured data on, site-title suffix off.
    assert!(page.body.contains(r#"name="seo_structured_data" value="1" checked"#), "{}", page.body);
    assert!(!page.body.contains(r#"name="seo_append_site_title" value="1" checked"#));

    let saved = post(
        &f.state,
        "/settings/seo/",
        &cookie,
        &[
            ("seo_append_site_title", "1"),
            ("seo_sitemap_images", "1"),
            ("seo_referrer_policy", "same-origin"),
            ("seo_robots_txt", "User-agent: *\r\nDisallow: /secret/  \r\n"),
            ("indexnow_api_key", "abcdef0123456789"),
        ],
    )
    .await;
    assert_eq!(saved.flash(), "SEO settings saved.");

    let back = get(&f.state, "/settings/seo/", &cookie).await;
    assert!(back.body.contains(r#"name="seo_append_site_title" value="1" checked"#));
    // A switch that was not posted is written as off rather than left alone —
    // an unchecked checkbox sends nothing at all.
    assert!(!back.body.contains(r#"name="seo_structured_data" value="1" checked"#), "{}", back.body);
    assert!(back.body.contains(r#"<option value="same-origin" selected>"#));
    // Stored cleaned: no carriage returns, no trailing spaces.
    assert_eq!(
        f.state.settings.get_str("seo_robots_txt").unwrap(),
        "User-agent: *\nDisallow: /secret/"
    );
    assert_eq!(f.state.settings.get_str("indexnow_api_key").unwrap(), "abcdef0123456789");
}

#[tokio::test]
async fn an_indexnow_key_that_could_not_work_is_refused_rather_than_stored() {
    // The key doubles as a URL path on this site. A key the submitter will
    // later decline to use would leave the field looking saved while nothing
    // was ever announced.
    let f = fixture().await;
    let cookie = f.session().await;
    for bad in ["short", "has spaces in it", "../../etc/passwd"] {
        let reply = post(&f.state, "/settings/seo/", &cookie, &[("indexnow_api_key", bad)]).await;
        assert!(reply.flash().contains("8–128"), "{bad} was accepted: {}", reply.flash());
        assert!(f.state.settings.get_str("indexnow_api_key").unwrap_or_default().is_empty());
    }
}

#[tokio::test]
async fn the_module_is_on_the_modules_hub_and_in_settings() {
    let f = fixture().await;
    let cookie = f.session().await;
    let hub = get(&f.state, "/modules/", &cookie).await;
    assert!(hub.body.contains(r#"href="/ghost/seo/""#), "the SEO card is missing from Modules");
    assert!(hub.body.contains("<h2>SEO</h2>"), "{}", hub.body);

    let settings = get(&f.state, "/settings/", &cookie).await;
    assert!(settings.body.contains(r#"/ghost/settings/seo/"#), "the SEO group is missing from Settings");
}

/// The test the settings page exists for: a switch here has to change what a
/// visitor is served, not just what the admin remembers.
#[tokio::test]
async fn a_setting_saved_here_changes_what_the_site_serves() {
    let f = fixture().await;
    let cookie = f.session().await;
    let site = || ghost_render::context::build_site(&f.state.settings, ORIGIN, &format!("{ORIGIN}/ghost"));

    // By default the renderer publishes structured data and does not append
    // the site title.
    assert!(ghost_core::seo::flag(&site(), "seo_structured_data"));
    assert!(!ghost_core::seo::flag(&site(), "seo_append_site_title"));

    post(
        &f.state,
        "/settings/seo/",
        &cookie,
        &[("seo_append_site_title", "1"), ("seo_referrer_policy", "no-referrer")],
    )
    .await;

    // Both reach the render root, which is where `{{ghost_head}}` reads them.
    let site = site();
    assert!(!ghost_core::seo::flag(&site, "seo_structured_data"));
    assert!(ghost_core::seo::flag(&site, "seo_append_site_title"));
    assert_eq!(ghost_core::seo::referrer_policy(&site), "no-referrer");
}

/// Nothing is stored for this module, so a site with no content at all must
/// render both screens rather than dividing by zero on the average.
#[tokio::test]
async fn a_site_with_no_content_still_renders_both_screens() {
    let f = fixture().await;
    let cookie = f.session().await;
    // Including the two entries a fresh install seeds, so "no content" means
    // what it says.
    sqlx::query("DELETE FROM posts").execute(&f.state.db).await.unwrap();
    let overview = get(&f.state, "/seo/", &cookie).await;
    assert_eq!(overview.status, StatusCode::OK);
    assert!(overview.body.contains("Nothing to fix"), "{}", overview.body);
    let content = get(&f.state, "/seo/content/", &cookie).await;
    assert_eq!(content.status, StatusCode::OK);
    assert!(content.body.contains("Nothing matches"), "{}", content.body);
}
