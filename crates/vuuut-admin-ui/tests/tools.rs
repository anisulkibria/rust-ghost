//! Tools, end to end: the hub, the moved pages and their old URLs, and the
//! Redirects and Routes editors — what they show, what saving writes, and
//! what the testers say about an address.
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

    /// A published entry, in `locale` (None = the default) and translation
    /// `group`.
    #[allow(dead_code)]
    async fn entry(&self, kind: &str, slug: &str, title: &str, locale: Option<&str>, group: Option<&str>, status: &str) -> String {
        let id = vuuut_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, locale, translation_group, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>x</p>', 'x', ?, ?, 'public', ?, ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(vuuut_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(kind)
        .bind(status)
        .bind(locale)
        .bind(group)
        .execute(&self.state.db)
        .await
        .unwrap();
        id
    }
}

async fn fixture() -> Fixture {
    fixture_with_theme("source").await
}

async fn fixture_with_theme(theme: &str) -> Fixture {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Languages test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let content = std::env::temp_dir().join(format!("ghost-tools-test-{}", vuuut_core::ids::new_id()));
    std::fs::create_dir_all(&content).unwrap();
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": content.to_string_lossy()}
    }))
    .unwrap();
    let routes =
        vuuut_routing::routes_yaml::parse_routes_yaml(vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML)
            .unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes").join(theme);
    let deps = vuuut_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: vuuut_api::themes::url_resolver_live(router.clone(), settings.clone()),
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



#[tokio::test]
async fn the_hub_lists_every_tool_with_its_state() {
    let f = fixture().await;
    let cookie = f.session().await;
    let page = get(&f.state, "/tools/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    for href in ["/ghost/customizations/", "/ghost/navigations/", "/ghost/code-injection/", "/ghost/announcement/",
                 "/ghost/redirects/", "/ghost/routes/", "/ghost/integrations/", "/ghost/machine-payments/", "/ghost/growth/",
                 "/ghost/migrations/", "/ghost/backups/", "/ghost/staffs/", "/ghost/history/"] {
        assert!(page.body.contains(&format!(r#"class="tool-card" href="{href}""#)), "{href} missing");
    }
    assert!(page.body.contains("No rules yet"));
    assert!(page.body.contains(r#"href="/ghost/tools/""#), "rail links to Tools");
}

#[tokio::test]
async fn the_moved_pages_open_at_their_new_urls_and_the_old_ones_redirect() {
    let f = fixture().await;
    let cookie = f.session().await;
    for (old, new) in [
        ("/design/", "/ghost/customizations/"),
        ("/navigation/", "/ghost/navigations/"),
        ("/settings/code-injection/", "/ghost/code-injection/"),
        ("/settings/site/", "/ghost/announcement/"),
        ("/settings/advanced/", "/ghost/tools/"),
        ("/settings/staff/", "/ghost/staffs/"),
        ("/settings/integrations/", "/ghost/integrations/"),
        ("/settings/history/", "/ghost/history/"),
        ("/settings/migration/", "/ghost/migrations/"),
        ("/settings/machine-payments/", "/ghost/machine-payments/"),
        ("/settings/integrations/slack/", "/ghost/integrations/slack/"),
    ] {
        let r = get(&f.state, old, &cookie).await;
        assert!(r.status.is_redirection(), "{old} → {}", r.status);
        assert_eq!(r.location, new);
        let page = get(&f.state, new.trim_start_matches("/ghost"), &cookie).await;
        assert_eq!(page.status, StatusCode::OK, "{new}");
        assert!(
            page.body.contains(r#"title="Back to tools""#) || page.body.contains(r#"title="Back to integrations""#) || new == "/ghost/tools/",
            "{new} goes back up the Tools tree"
        );
        assert!(page.body.contains(r#"class="nav-item active" href="/ghost/tools/""#), "{new} lights Tools in the rail");
    }
    let custom = get(&f.state, "/customizations/", &cookie).await;
    assert!(custom.body.contains("/edit/\">"), "Customizations links to the theme's code");
    assert!(custom.body.contains("Edit theme code"));
    let settings = get(&f.state, "/settings/", &cookie).await;
    assert!(!settings.body.contains("Code injection") && !settings.body.contains("Announcement"));
}

#[tokio::test]
async fn broadsheet_homepage_preview_uses_unsaved_order_without_polluting_the_public_cache() {
    use vuuut_db::repos::tags::{insert, NewTag};

    let f = fixture_with_theme("broadsheet").await;
    let cookie = f.session().await;
    for (name, posts) in [("Climate Desk", 2), ("Arts Desk", 1), ("Empty Desk", 0)] {
        let tag = insert(&f.state.db, &NewTag { name: name.into(), ..Default::default() }).await.unwrap();
        for n in 0..posts {
            let slug = format!("{}-{n}", tag.slug.as_deref().unwrap());
            let entry = f.entry("post", &slug, &slug, None, None, "published").await;
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
                .bind(vuuut_core::ids::new_id()).bind(entry).bind(&tag.id)
                .execute(&f.state.db).await.unwrap();
        }
    }
    f.state.settings.set(&f.state.db, vuuut_core::caching::SETTING_PAGE_CACHE, &serde_json::json!(true)).await.unwrap();
    let public = || async {
        let response = vuuut_front::site_router(f.state.clone())
            .oneshot(Request::new(Body::empty())).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        String::from_utf8(to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    };
    let climate = "More in Climate Desk";
    let arts = "More in Arts Desk";
    let homepage = public().await;
    assert!(homepage.find(climate).unwrap() < homepage.find(arts).unwrap());
    assert!(!homepage.contains("More in Empty Desk"));

    let custom = get(&f.state, "/customizations/", &cookie).await;
    assert!(custom.body.contains("Homepage section tags"));
    assert!(custom.body.contains("Live site preview"));

    let response = vuuut_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone())
        .oneshot(Request::post("/customizations/preview/")
            .header("host", HOST).header("origin", ORIGIN).header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"homepage_section_tags":" arts-desk, missing, climate-desk, arts-desk, empty-desk "}"#)).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let preview = result["html"].as_str().unwrap();
    assert!(preview.contains(r#"class="np-home""#));
    assert!(preview.contains("/tag/arts-desk/"));
    assert!(preview.find(arts).unwrap() < preview.find(climate).unwrap());
    assert_eq!(preview.matches(arts).count(), 1);
    assert!(!preview.contains("More in Empty Desk"));
    assert_eq!(public().await, homepage);

    let saved = post(&f.state, "/customizations/", &cookie, &[("homepage_section_tags", "arts-desk")]).await;
    assert!(saved.flash().contains("Design settings saved"));
    let homepage = public().await;
    assert!(homepage.contains(arts));
    assert!(!homepage.contains(climate));
}

#[tokio::test]
async fn redirects_are_edited_saved_and_tested_on_screen() {
    let f = fixture().await;
    let cookie = f.session().await;
    let saved = post(&f.state, "/redirects/", &cookie, &[
        ("status", "301"), ("from", "/old/"), ("to", "/new/"),
        ("status", "302"), ("from", "/post/(\\d+)/"), ("to", "/posts/$1/"),
        ("status", "301"), ("from", ""), ("to", ""),
    ]).await;
    assert!(saved.status.is_redirection(), "{}", saved.body);
    assert!(saved.flash().contains("2 rules"), "{}", saved.flash());
    let on_disk = std::fs::read_to_string(f.state.config.content_path().join("redirects.yaml")).unwrap();
    assert!(on_disk.contains("/old/") && on_disk.contains("302"));

    let page = get(&f.state, "/redirects/?test=/post/12", &cookie).await;
    assert!(page.body.contains("Rule 2 catches"), "tester names the rule");
    assert!(page.body.contains("/posts/12/"));
    assert!(page.body.contains("rx-hit"), "the matching row is highlighted");
    assert!(page.body.contains(r#"value="/old/""#), "rules are on screen");
    assert!(page.body.contains("Pattern") && page.body.contains("Exact"));

    let miss = get(&f.state, "/redirects/?test=/bold/", &cookie).await;
    assert!(miss.body.contains("No rule catches"), "a plain path matches only itself");

    // a broken pattern is refused, and what was typed comes back
    let bad = post(&f.state, "/redirects/", &cookie, &[("status", "301"), ("from", "/a/(b/"), ("to", "/c/")]).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(bad.body.contains("Nothing was saved") && bad.body.contains(r#"value="/a/(b/""#));
    assert_eq!(f.state.redirects.read().await.rules().len(), 2, "live rules untouched");
}

#[tokio::test]
async fn routes_are_edited_saved_and_tested_on_screen() {
    let f = fixture().await;
    let cookie = f.session().await;
    let page = get(&f.state, "/routes/?test=/hello/", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#"name="c_permalink" value="/{slug}/""#), "default collection on screen");
    assert!(page.body.contains("<b>Post</b>"), "{}", page.body.split("test-result").nth(1).unwrap_or(""));

    let saved = post(&f.state, "/routes/", &cookie, &[
        ("r_start", "1"), ("r_path", "/team/"), ("r_templates", "page"), ("r_filter", ""),
        ("c_start", "1"), ("c_path", "/blog/"), ("c_permalink", "/blog/{slug}/"), ("c_filter", ""),
        ("c_templates", "index"), ("c_rss", "on"),
        ("tag_on", "1"), ("tag", "/topic/{slug}/"), ("author", "/author/{slug}/"),
    ]).await;
    assert!(saved.status.is_redirection(), "{}", saved.body);
    let on_disk = std::fs::read_to_string(f.state.config.content_path().join("routes.yaml")).unwrap();
    assert!(on_disk.contains("/blog/{slug}/") && on_disk.contains("/topic/{slug}/"), "{on_disk}");
    assert!(!on_disk.contains("author"), "switched-off taxonomy is gone");

    let page = get(&f.state, "/routes/?test=/blog/hello/", &cookie).await;
    assert!(page.body.contains("collection <b>blog</b>"), "tester follows the new routes");
    let tag = get(&f.state, "/routes/?test=/topic/news/", &cookie).await;
    assert!(tag.body.contains("Tag archive"));
    let custom = get(&f.state, "/routes/?test=/team/", &cookie).await;
    assert!(custom.body.contains("Template page"));

    let bad = post(&f.state, "/routes/", &cookie, &[("c_start", "1"), ("c_path", "/"), ("c_permalink", "/post/")]).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(bad.body.contains("needs {slug}"));
}

#[tokio::test]
async fn migrations_is_import_only_and_backups_holds_the_exports() {
    let f = fixture().await;
    let cookie = f.session().await;
    let m = get(&f.state, "/migrations/", &cookie).await;
    assert_eq!(m.status, StatusCode::OK);
    assert!(m.body.contains(r#"action="/ghost/migrations/import/""#));
    assert!(!m.body.contains("Download archive"), "no export on Migrations");
    let b = get(&f.state, "/backups/", &cookie).await;
    assert_eq!(b.status, StatusCode::OK);
    assert!(b.body.contains(r#"action="/ghost/backups/export/""#) && b.body.contains("Download archive"));
    assert!(!b.body.contains("Universal import"), "no import on Backups");
    assert!(b.body.contains("No backups yet"));
}

async fn tag_names(f: &Fixture) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM tags ORDER BY name").fetch_all(&f.state.db).await.unwrap()
}

#[tokio::test]
async fn a_backup_is_taken_restored_and_capped_at_ten() {
    let f = fixture().await;
    let cookie = f.session().await;

    // schedule
    let s = post(&f.state, "/backups/schedule/", &cookie, &[("schedule", "weekly")]).await;
    assert!(s.flash().contains("every week"), "{}", s.flash());
    assert_eq!(f.state.settings.get_str("backup_schedule").as_deref(), Some("weekly"));

    // a file under content/ rides along
    let images = f.state.config.content_path().join("images");
    std::fs::create_dir_all(&images).unwrap();
    std::fs::write(images.join("a.png"), b"one").unwrap();

    let before = tag_names(&f).await;
    let taken = post(&f.state, "/backups/create/", &cookie, &[]).await;
    assert!(taken.flash().starts_with("Backup taken"), "{}", taken.flash());
    let list = vuuut_api::backups::list(&f.state).await;
    assert_eq!(list.len(), 1);
    let name = list[0].name.clone();
    assert!(list[0].rows > 0 && list[0].files == 1);

    // change the site: a new tag, a changed image, a new image, schedule off
    sqlx::query("INSERT INTO tags (id, name, slug, created_at, updated_at) VALUES ('tx', 'Added later', 'added-later', '2026-01-01 00:00:00', '2026-01-01 00:00:00')")
        .execute(&f.state.db).await.unwrap();
    std::fs::write(images.join("a.png"), b"two").unwrap();
    std::fs::write(images.join("b.png"), b"new").unwrap();
    post(&f.state, "/backups/schedule/", &cookie, &[("schedule", "daily")]).await;

    let page = get(&f.state, "/backups/", &cookie).await;
    assert!(page.body.contains(&name) && page.body.contains("Manual"));

    let restored = post(&f.state, "/backups/restore/", &cookie, &[("name", &name)]).await;
    assert!(restored.flash().starts_with("Restored"), "{}", restored.flash());
    assert_eq!(tag_names(&f).await, before, "the database is back");
    assert_eq!(std::fs::read(images.join("a.png")).unwrap(), b"one", "files are back");
    assert!(!images.join("b.png").exists(), "files added since are gone");
    assert_eq!(f.state.settings.get_str("backup_schedule").as_deref(), Some("daily"), "the schedule survives a restore");
    // still signed in, and the site before the restore was kept
    assert_eq!(get(&f.state, "/backups/", &cookie).await.status, StatusCode::OK);
    let list = vuuut_api::backups::list(&f.state).await;
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].kind, vuuut_api::backups::Kind::PreRestore);

    // undo the restore from that safety copy
    let undo = post(&f.state, "/backups/restore/", &cookie, &[("name", &list[0].name)]).await;
    assert!(undo.flash().starts_with("Restored"), "{}", undo.flash());
    assert!(tag_names(&f).await.contains(&"Added later".to_string()));

    // never more than ten, oldest first out
    for _ in 0..10 {
        vuuut_api::backups::create(&f.state, vuuut_api::backups::Kind::Manual).await.unwrap();
    }
    let list = vuuut_api::backups::list(&f.state).await;
    assert_eq!(list.len(), vuuut_api::backups::MAX_BACKUPS);
    assert!(!list.iter().any(|b| b.name == name), "the oldest went first: {name} still in {:?}", list.iter().map(|b| &b.name).collect::<Vec<_>>());

    // delete, and a name that is not a backup is refused
    let del = post(&f.state, "/backups/delete/", &cookie, &[("name", &list[0].name)]).await;
    assert_eq!(del.flash(), "Backup deleted.");
    let bad = post(&f.state, "/backups/restore/", &cookie, &[("name", "../../data/ghost.db")]).await;
    assert!(bad.flash().starts_with("Nothing was restored"));
}


#[tokio::test]
async fn tool_settings_open_from_their_tools_and_old_links_redirect() {
    let f = fixture().await;
    let cookie = f.session().await;
    let settings = get(&f.state, "/settings/", &cookie).await;
    assert_eq!(settings.status, StatusCode::OK);
    for group in ["general", "emails", "features", "danger"] {
        assert!(settings.body.contains(&format!(r#"href="/ghost/settings/{group}/""#)));
    }

    for (group, tool) in [
        ("analytics", "analytics"),
        ("cookies", "cookies"),
        ("forms", "forms"),
        ("images", "media"),
        ("languages", "languages"),
        ("memberships", "memberships"),
        ("newsletters", "newsletters"),
        ("popups", "popups"),
        ("security", "security"),
        ("seo", "seo"),
    ] {
        let old = format!("/settings/{group}/");
        let path = format!("/{tool}/settings/");
        let href = format!("/ghost{path}");
        assert!(!settings.body.contains(&format!(r#"href="/ghost{old}""#)));
        assert!(!settings.body.contains(&format!(r#"href="{href}""#)));

        let tool_page = get(&f.state, &format!("/{tool}/"), &cookie).await;
        assert_eq!(tool_page.status, StatusCode::OK, "{tool}");
        assert!(tool_page.body.contains(&format!(r#"href="{href}""#)), "{tool}");

        let page = get(&f.state, &path, &cookie).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert!(page.body.contains(&format!(r#"action="{href}""#)), "{path}");
        assert!(page.body.contains(&format!(r#"href="/ghost/{tool}/""#)), "{path}");

        let bookmark = get(&f.state, &format!("{old}?ok=Saved%20settings"), &cookie).await;
        assert_eq!(bookmark.status, StatusCode::PERMANENT_REDIRECT, "{old}");
        assert_eq!(bookmark.location, format!("{href}?ok=Saved%20settings"));
    }

    let legacy_form = post(&f.state, "/settings/newsletters/new/", &cookie, &[("name", "Test")]).await;
    assert_eq!(legacy_form.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(legacy_form.location, "/ghost/newsletters/settings/new/");

    let saved = post(&f.state, "/analytics/settings/", &cookie, &[("web_analytics", "1")]).await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER);
    assert!(saved.location.starts_with("/ghost/analytics/settings/?ok="));
    assert!(f.state.settings.get_bool("web_analytics"));

    let found = get(&f.state, "/omnisearch/?q=outbound+link+tagging", &cookie).await;
    assert!(found.body.contains(r#"href="/ghost/analytics/settings/""#));
}
