//! Navigation on a multilingual site, end to end: switching a language on
//! generates its menus pointing at its translations, the Navigation page
//! edits both menus for every language, and the language switcher a menu
//! carries reaches the public site as one link per language.
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

    /// A published entry, in `locale` (None = the default) and translation
    /// `group`.
    async fn entry(&self, kind: &str, slug: &str, title: &str, locale: Option<&str>, group: Option<&str>, status: &str) -> String {
        let id = ghost_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, locale, translation_group, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, '<p>x</p>', 'x', ?, ?, 'public', ?, ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(ghost_core::ids::uuid_v4())
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
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Languages test")),
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
        url_resolver: ghost_api::themes::url_resolver_live(router.clone(), settings.clone()),
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


const GROUP: &str = "6553f7a1b2c3d4e5f6a7b800";

/// An English About page with a published Bengali translation, a menu that
/// links to it, and Bengali switched on through the real settings screen.
async fn bilingual() -> (Fixture, String) {
    let f = fixture().await;
    let cookie = f.session().await;
    f.entry("page", "who-we-are", "About", None, Some(GROUP), "published").await;
    f.entry("page", "amader-somporke", "আমাদের সম্পর্কে", Some("bn"), Some(GROUP), "published").await;
    f.state
        .settings
        .set(
            &f.state.db,
            "navigation",
            &serde_json::json!([
                {"label": "Home", "url": "/"},
                {"label": "About", "url": "/who-we-are/"},
                {"label": "News", "url": "/tag/news/"},
                {"label": "Docs", "url": "https://docs.example/"},
            ]),
        )
        .await
        .unwrap();
    f.state
        .settings
        .set(&f.state.db, "secondary_navigation", &serde_json::json!([{"label": "Sign up", "url": "#/portal/"}]))
        .await
        .unwrap();
    // The flash may report the theme reload, which this fixture cannot do;
    // what matters is what the save stored.
    send(&f.state, "POST", "/settings/languages/", Some(&cookie), Some(ORIGIN), Some(&[("locales", "bn")])).await;
    assert_eq!(f.state.settings.get("locales_enabled"), Some(serde_json::json!(["bn"])));
    (f, cookie)
}

#[tokio::test]
async fn switching_a_language_on_generates_its_menus_from_the_default_ones() {
    let (f, _) = bilingual().await;
    let primary = f.state.settings.get("navigation_locales").unwrap();
    assert_eq!(
        primary["bn"],
        serde_json::json!([
            {"label": "Home", "url": "/bn/"},
            // The page's translation, under the translation's own title.
            {"label": "আমাদের সম্পর্কে", "url": "/bn/amader-somporke/"},
            {"label": "News", "url": "/bn/tag/news/"},
            {"label": "Docs", "url": "https://docs.example/"},
        ]),
        "{primary}"
    );
    let secondary = f.state.settings.get("secondary_navigation_locales").unwrap();
    assert_eq!(secondary["bn"], serde_json::json!([{"label": "Sign up", "url": "#/portal/"}]));

    // Switching it off again takes its menus away.
    let cookie = f.session().await;
    send(&f.state, "POST", "/settings/languages/", Some(&cookie), Some(ORIGIN), Some(&[])).await;
    assert_eq!(f.state.settings.get("navigation_locales"), Some(serde_json::json!({})));
}

#[tokio::test]
async fn an_untranslated_or_draft_entry_keeps_its_original_link() {
    let f = fixture().await;
    let cookie = f.session().await;
    f.entry("page", "contact", "Contact", None, None, "published").await;
    f.entry("page", "team", "Team", None, Some(GROUP), "published").await;
    // Its translation is still a draft, so linking it would be a 404.
    f.entry("page", "dol", "দল", Some("bn"), Some(GROUP), "draft").await;
    f.state
        .settings
        .set(&f.state.db, "navigation", &serde_json::json!([{"label": "Contact", "url": "/contact/"}, {"label": "Team", "url": "/team/"}]))
        .await
        .unwrap();
    send(&f.state, "POST", "/settings/languages/", Some(&cookie), Some(ORIGIN), Some(&[("locales", "bn")])).await;
    let primary = f.state.settings.get("navigation_locales").unwrap();
    assert_eq!(
        primary["bn"],
        serde_json::json!([{"label": "Contact", "url": "/contact/"}, {"label": "Team", "url": "/team/"}])
    );
}

#[tokio::test]
async fn the_navigation_page_edits_both_menus_of_every_language() {
    let (f, cookie) = bilingual().await;
    let page = send(&f.state, "GET", "/navigations/", Some(&cookie), None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    for kind in ["primary", "secondary", "primary.bn", "secondary.bn"] {
        assert!(page.body.contains(&format!(r#"value="add-{kind}""#)), "no {kind} menu");
    }
    assert!(page.body.contains("আমাদের সম্পর্কে"), "the generated Bengali menu is not shown");

    // Add the switcher to the end of the Bengali primary menu, then save.
    let added = send(
        &f.state,
        "POST",
        "/navigations/",
        Some(&cookie),
        Some(ORIGIN),
        Some(&[
            ("primary_label", "Home"), ("primary_url", "/"),
            ("primary.bn_label", "হোম"), ("primary.bn_url", "/bn/"),
            ("action", "switcher-primary.bn"),
        ]),
    )
    .await;
    assert!(added.body.contains("Language switcher"), "the switcher row was not added");
    let saved = send(
        &f.state,
        "POST",
        "/navigations/",
        Some(&cookie),
        Some(ORIGIN),
        Some(&[
            ("primary_label", "Home"), ("primary_url", "/"),
            ("primary_label", "Language"), ("primary_url", "#language-switcher"),
            ("primary.bn_label", "হোম"), ("primary.bn_url", "/bn/"),
            ("primary.bn_label", "Language"), ("primary.bn_url", "#language-switcher"),
            ("secondary.bn_label", "যোগ দিন"), ("secondary.bn_url", "#/portal/"),
            ("action", "save"),
        ]),
    )
    .await;
    assert!(saved.status.is_redirection(), "{}", saved.status);
    assert_eq!(
        f.state.settings.get("navigation_locales").unwrap()["bn"],
        serde_json::json!([{"label": "হোম", "url": "/bn/"}, {"label": "Language", "url": "#language-switcher"}])
    );
    assert_eq!(
        f.state.settings.get("secondary_navigation_locales").unwrap()["bn"],
        serde_json::json!([{"label": "যোগ দিন", "url": "#/portal/"}])
    );
    assert_eq!(f.state.settings.get("navigation").unwrap()[1]["url"], "#language-switcher");
}

async fn public(state: &ghost_api::AppState, path: &str) -> (StatusCode, String) {
    let app = ghost_front::site_router(state.clone());
    let request = Request::builder().uri(path).header("host", HOST).body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The language data `{{ghost_head}}` injected for the dropdown, parsed.
fn switcher_data(html: &str) -> serde_json::Value {
    let start = html.find(r#"<script id="vuuut-language-data" type="application/json">"#).expect("no switcher data injected");
    let body = &html[start..];
    let body = &body[body.find('>').unwrap() + 1..];
    serde_json::from_str(&body[..body.find("</script>").unwrap()]).unwrap()
}

#[tokio::test]
async fn the_switcher_reaches_the_site_as_an_injected_dropdown() {
    let (f, _) = bilingual().await;
    let switcher = serde_json::json!([{"label": "Home", "url": "/"}, {"label": "Language", "url": "#language-switcher"}]);
    f.state.settings.set(&f.state.db, "navigation", &switcher).await.unwrap();
    f.state
        .settings
        .set(&f.state.db, "navigation_locales", &serde_json::json!({"bn": [{"label": "হোম", "url": "/bn/"}, {"label": "Language", "url": "#language-switcher"}]}))
        .await
        .unwrap();

    // The theme draws one ordinary menu item, named for the language being
    // read; with scripts off it goes to the other language.
    let (status, en) = public(&f.state, "/who-we-are/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(en.contains(r#"<li class="nav-language"><a href="/bn/amader-somporke/">English</a></li>"#), "{en}");
    assert!(!en.contains("#language-switcher"), "the marker leaked onto the page");
    // ...and the head carries the dropdown, injected like Portal.
    assert!(en.contains("/public/language-switcher.js?v="), "no dropdown script");
    let data = switcher_data(&en);
    assert_eq!(data["current"], "en");
    assert_eq!(data["trigger"], serde_json::json!({"label": "English", "url": "/bn/amader-somporke/"}));
    assert_eq!(data["languages"][1], serde_json::json!({"locale": "bn", "label": "বাংলা", "url": "/bn/amader-somporke/", "active": false}));

    // The Bengali page: its own menu, and a dropdown leading back.
    f.state.settings.set(&f.state.db, "locale_hreflang", &serde_json::json!(true)).await.unwrap();
    let (status, bn) = public(&f.state, "/bn/amader-somporke/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(bn.contains(">হোম<"), "the Bengali menu was not used");
    // `x-default` is the default language's URL on every member's page.
    assert!(
        bn.contains(r#"<link rel="alternate" hreflang="x-default" href="https://example.test/who-we-are/" />"#),
        "x-default points at the wrong language: {}", bn.lines().filter(|l| l.contains("hreflang")).collect::<Vec<_>>().join("\n")
    );
    assert!(bn.contains(r#"<a href="/who-we-are/">বাংলা</a>"#), "{bn}");
    assert_eq!(switcher_data(&bn)["languages"][0]["url"], "/who-we-are/");

    // An untranslated entry: Bengali leads to the Bengali home page, and no
    // `hreflang` alternate claims a Bengali copy exists.
    f.entry("post", "only-english", "Only English", None, None, "published").await;
    let (_, only) = public(&f.state, "/only-english/").await;
    assert_eq!(switcher_data(&only)["languages"][1]["url"], "/bn/");
    assert!(!only.contains(r#"rel="alternate" hreflang="bn""#), "an alternate for a page that does not exist");

    // The script itself is served.
    let (status, js) = public(&f.state, "/public/language-switcher.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("vuuut-language-data"));
}

#[tokio::test]
async fn a_menu_without_the_switcher_injects_nothing() {
    let (f, _) = bilingual().await;
    let (_, html) = public(&f.state, "/who-we-are/").await;
    assert!(!html.contains("vuuut-language-data"), "a script nobody asked for");
    assert!(!html.contains("language-switcher.js"));
}

#[tokio::test]
async fn the_editor_views_an_entry_at_its_language_prefixed_url() {
    let (f, cookie) = bilingual().await;
    let id: String = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'amader-somporke'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let editor = send(&f.state, "GET", &format!("/pages/{id}/"), Some(&cookie), None, None).await;
    assert_eq!(editor.status, StatusCode::OK);
    assert!(
        editor.body.contains(r#"id="editor-view" href="https://example.test/bn/amader-somporke/""#),
        "View page does not carry the language prefix"
    );
    // The slug preview shows the same URL.
    assert!(editor.body.contains("https://example.test/bn/<b>amader-somporke</b>/"), "slug preview");
    // And a default-language entry is untouched.
    let en: String = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'who-we-are'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let editor = send(&f.state, "GET", &format!("/pages/{en}/"), Some(&cookie), None, None).await;
    assert!(editor.body.contains(r#"id="editor-view" href="https://example.test/who-we-are/""#));
}

/// The bug this guards: changing the publication language dropped every
/// other language and their menus. It is changed in Settings → Languages,
/// as the default publication language.
#[tokio::test]
async fn changing_the_publication_language_keeps_the_others_and_their_menus() {
    let (f, cookie) = bilingual().await;
    let english_menu = f.state.settings.get("navigation").unwrap();
    let bengali_menu = f.state.settings.get("navigation_locales").unwrap()["bn"].clone();

    let general = |locale: &'static str| {
        let state = f.state.clone();
        let cookie = cookie.clone();
        async move {
            // As the Language settings form posts it with scripts off: the
            // new default's box is still an ordinary ticked checkbox.
            send(&state, "POST", "/settings/languages/", Some(&cookie), Some(ORIGIN), Some(&[("locale", locale), ("locales", locale)])).await
        }
    };
    general("bn").await;

    assert_eq!(f.state.settings.get_str("locale").as_deref(), Some("bn"));
    assert_eq!(f.state.settings.get("locales_enabled"), Some(serde_json::json!(["en"])), "English was dropped");
    assert_eq!(f.state.settings.get("navigation").unwrap(), bengali_menu, "Bengali's menu is now the site-wide one");
    assert_eq!(f.state.settings.get("navigation_locales").unwrap()["en"], english_menu, "English lost its menu");

    // Opening the Navigation page must not tidy anything away.
    let page = send(&f.state, "GET", "/navigations/", Some(&cookie), None, None).await;
    assert!(page.body.contains(r#"value="add-primary.en""#), "no English menu on the page");
    assert_eq!(f.state.settings.get("navigation_locales").unwrap()["en"], english_menu);

    // The English content is still English, now under its prefix, and the
    // Bengali content is unprefixed.
    let (status, _) = public(&f.state, "/en/who-we-are/").await;
    assert_eq!(status, StatusCode::OK, "the English page moved nowhere");
    let (status, _) = public(&f.state, "/amader-somporke/").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = public(&f.state, "/who-we-are/").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an English page on a Bengali URL");

    // And back again restores the original arrangement.
    general("en").await;
    assert_eq!(f.state.settings.get("locales_enabled"), Some(serde_json::json!(["bn"])));
    assert_eq!(f.state.settings.get("navigation").unwrap(), english_menu);
    assert_eq!(f.state.settings.get("navigation_locales").unwrap()["bn"], bengali_menu);
    assert!(!f.state.settings.get("navigation_locales").unwrap().as_object().unwrap().contains_key("en"));
    let (status, _) = public(&f.state, "/who-we-are/").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_link_search_says_when_there_are_more_matches_than_it_shows() {
    let (f, cookie) = bilingual().await;
    let id: String = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'amader-somporke'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    // French pages: this Bengali page's set already has its English one, so
    // English pages are rightly not offered at all.
    for i in 0..25 {
        f.entry("page", &format!("english-page-{i}"), &format!("English page {i}"), Some("fr"), None, "published").await;
    }
    let reply = send(&f.state, "GET", &format!("/pages/{id}/translations/search/?q=English"), Some(&cookie), None, None).await;
    let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(json["entries"].as_array().unwrap().len(), 20, "{json}");
    assert_eq!(json["more"], true);
    let narrow = send(&f.state, "GET", &format!("/pages/{id}/translations/search/?q=English+page+7"), Some(&cookie), None, None).await;
    let json: serde_json::Value = serde_json::from_str(&narrow.body).unwrap();
    assert_eq!(json["more"], false, "{json}");
    // An English page is not offered: the set already has one.
    let english = send(&f.state, "GET", &format!("/pages/{id}/translations/search/?q=who"), Some(&cookie), None, None).await;
    let json: serde_json::Value = serde_json::from_str(&english.body).unwrap();
    assert_eq!(json["entries"], serde_json::json!([]), "{json}");
}

/// The home page lists the default language's posts — including every
/// `{{#get "posts"}}` block the theme draws itself, which is where Source
/// puts its featured row and its latest-posts grid — and each language's
/// home page lists its own. Change the default language and all of it
/// follows, with no other step.
#[tokio::test]
async fn each_home_page_lists_only_its_own_language_and_follows_a_change_of_default() {
    let (f, cookie) = bilingual().await;
    // Featured too, so Source's `{{#get "posts" filter="featured:true"}}`
    // row is exercised as well as the main list.
    f.entry("post", "english-story", "English story", None, None, "published").await;
    f.entry("post", "bangla-golpo", "Bangla golpo", Some("bn"), None, "published").await;
    sqlx::query("UPDATE posts SET featured = 1 WHERE slug IN ('english-story', 'bangla-golpo')")
        .execute(&f.state.db)
        .await
        .unwrap();

    let (status, home) = public(&f.state, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(home.contains("English story"), "the English home page lost its post");
    assert!(!home.contains("Bangla golpo"), "a Bengali post on the English home page");

    let (status, bn) = public(&f.state, "/bn/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(bn.contains("Bangla golpo"));
    assert!(!bn.contains("English story"), "an English post on the Bengali home page");
    // Linked under its prefix, wherever the theme drew it from.
    assert!(bn.contains(r#"href="/bn/bangla-golpo/""#), "the Bengali post is linked without its prefix");
    assert!(!bn.contains(r#"href="/bangla-golpo/""#));

    // Make Bengali the default. Nothing else is touched.
    send(&f.state, "POST", "/settings/languages/", Some(&cookie), Some(ORIGIN), Some(&[("locale", "bn"), ("locales", "bn")])).await;

    let (_, home) = public(&f.state, "/").await;
    assert!(home.contains("Bangla golpo"), "the new default's posts are not on the home page");
    assert!(!home.contains("English story"));
    assert!(home.contains(r#"href="/bangla-golpo/""#), "the new default is still prefixed");
    let (status, en) = public(&f.state, "/en/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(en.contains("English story"));
    assert!(!en.contains("Bangla golpo"));
    assert!(en.contains(r#"href="/en/english-story/""#), "the old default is not under its prefix");
    let (status, _) = public(&f.state, "/en/english-story/").await;
    assert_eq!(status, StatusCode::OK);
}

/// A tag with no language of its own belongs to every language, and is
/// linked in the language of the entry that carries it: a Bengali post's
/// `news` tag goes to `/bn/tag/news/`, where the Bengali posts with that
/// tag are — on the post, on its card in any list, and on the archive.
#[tokio::test]
async fn an_entrys_tags_link_to_its_own_languages_archive() {
    let (f, _) = bilingual().await;
    let en = f.entry("post", "english-story", "English story", None, None, "published").await;
    let bn = f.entry("post", "bangla-golpo", "Bangla golpo", Some("bn"), None, "published").await;
    let tag = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at) VALUES (?, 'Shared news', 'shared-news', 'public', '2026-11-01 00:00:00')")
        .bind(&tag)
        .execute(&f.state.db)
        .await
        .unwrap();
    for post in [&en, &bn] {
        sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
            .bind(ghost_core::ids::new_id())
            .bind(post)
            .bind(&tag)
            .execute(&f.state.db)
            .await
            .unwrap();
    }

    // On the post itself.
    let (_, post) = public(&f.state, "/bn/bangla-golpo/").await;
    assert!(post.contains(r#"href="/bn/tag/shared-news/""#), "the Bengali post's tag is not in Bengali");
    assert!(!post.contains(r#"href="/tag/shared-news/""#));
    // In the data a theme's post card reads (Source shows only the name;
    // a theme that links it uses this `url`), through the live resolver.
    let resolver = ghost_api::themes::url_resolver_live(f.state.router.read().await.clone(), f.state.settings.clone());
    let post_row = ghost_db::repos::posts::find_by_id(&f.state.db, &bn).await.unwrap().unwrap();
    let rel = ghost_db::repos::posts::load_relations(&f.state.db, std::slice::from_ref(&post_row)).await.unwrap();
    let mut card = ghost_core::models::post_to_json(&post_row, &rel.authors_for(&bn), &rel.tags_for(&bn), "");
    ghost_render::util::resolve_nested_urls(&mut card, &*resolver);
    assert_eq!(card["tags"][0]["url"], "/bn/tag/shared-news/");
    assert_eq!(card["primary_tag"]["url"], "/bn/tag/shared-news/");
    // The English post's tag stays unprefixed.
    let (_, english) = public(&f.state, "/english-story/").await;
    assert!(english.contains(r#"href="/tag/shared-news/""#));
    assert!(!english.contains(r#"href="/bn/tag/shared-news/""#));
    // The Bengali archive exists and lists the Bengali post only.
    let (status, archive) = public(&f.state, "/bn/tag/shared-news/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(archive.contains("Bangla golpo"));
    assert!(!archive.contains("English story"));
}

/// Every language's front page is a home page: the same `home` context and
/// the theme's `home.hbs`, so it looks exactly like the default's. Paged
/// archives stay index pages, and the canonical stays the page's own URL.
#[tokio::test]
async fn every_languages_home_page_has_the_home_design() {
    let (f, _) = bilingual().await;
    let (_, en) = public(&f.state, "/").await;
    let (status, bn) = public(&f.state, "/bn/").await;
    assert_eq!(status, StatusCode::OK);

    let body_class = |html: &str| -> String {
        let at = html.find("<body class=\"").expect("no body class");
        let rest = &html[at + 13..];
        rest[..rest.find('"').unwrap()].to_string()
    };
    assert!(body_class(&en).contains("home-template"));
    assert_eq!(body_class(&bn), body_class(&en), "the Bengali home page is not a home page");

    // Source's home.hbs, not its index.hbs: the header component is only on
    // the home page.
    let has_home_header = |html: &str| html.contains("gh-header");
    assert_eq!(has_home_header(&bn), has_home_header(&en), "different templates");
    assert!(has_home_header(&bn));

    // Still its own page to a crawler.
    assert!(bn.contains(r#"<link rel="canonical" href="https://example.test/bn/" />"#));
}

/// A language's home page takes its title, description and share cards
/// from the published page slugged with its code — a page that could never
/// be opened anyway, since `/bn/` is the Bengali home. The default's home
/// keeps the site's own details; nothing else changes.
#[tokio::test]
async fn a_page_slugged_with_a_language_code_supplies_that_home_pages_details() {
    let (f, cookie) = bilingual().await;
    f.state.settings.set(&f.state.db, "description", &serde_json::json!("The English tagline")).await.unwrap();
    let (_, before) = public(&f.state, "/bn/").await;
    assert!(before.contains("The English tagline"), "the site's details are the fallback");

    let page = f.entry("page", "bn", "বাংলা সাইট", None, None, "published").await;
    sqlx::query("UPDATE posts SET custom_excerpt = 'বাংলা ট্যাগলাইন' WHERE id = ?").bind(&page).execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO posts_meta (id, post_id, og_title) VALUES (?, ?, 'বাংলা শেয়ার')")
        .bind(ghost_core::ids::new_id())
        .bind(&page)
        .execute(&f.state.db)
        .await
        .unwrap();

    let (status, bn) = public(&f.state, "/bn/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(bn.contains("<title>বাংলা সাইট</title>"), "the page's title is not the home page's");
    assert!(bn.contains(r#"<meta name="description" content="বাংলা ট্যাগলাইন" />"#), "meta description");
    assert!(bn.contains(r#"<meta property="og:title" content="বাংলা শেয়ার" />"#), "og title");
    // Falls back along the chain where the page leaves a card field blank.
    assert!(bn.contains(r#"<meta name="twitter:title" content="বাংলা সাইট" />"#), "twitter title");
    assert!(!bn.contains("The English tagline"), "the English tagline is still on the Bengali home page");

    // The English home page is untouched.
    let (_, en) = public(&f.state, "/").await;
    assert!(en.contains("The English tagline"));
    assert!(!en.contains("বাংলা ট্যাগলাইন"));

    // The report lists it as the home page's details, not as unreachable.
    let report = send(&f.state, "GET", "/languages/", Some(&cookie), None, None).await;
    assert!(report.body.contains("Language home pages"));
    assert!(report.body.contains(&format!(r#"<a href="/ghost/pages/{page}/">বাংলা সাইট</a>"#)), "{}", report.body);
    assert!(!report.body.contains("A language prefix is shadowing these"), "flagged as unreachable");
}

/// The default publication language lives in Settings → Languages, and
/// Settings → General neither shows nor changes it any more.
#[tokio::test]
async fn the_default_publication_language_is_set_in_language_settings() {
    let (f, cookie) = bilingual().await;
    let languages = send(&f.state, "GET", "/settings/languages/", Some(&cookie), None, None).await;
    assert!(languages.body.contains("Default publication language"), "not on Language settings");
    assert!(languages.body.contains(r#"<select name="locale" data-lg-default>"#));
    assert!(languages.body.contains(r#"<option value="en" selected>"#), "the current default is not selected");

    let general = send(&f.state, "GET", "/settings/general/", Some(&cookie), None, None).await;
    assert!(!general.body.contains("Publication language"), "still on General settings");
    assert!(!general.body.contains(r#"name="locale""#));

    // A stale General form posting a locale changes nothing.
    send(&f.state, "POST", "/settings/general/", Some(&cookie), Some(ORIGIN), Some(&[("title", "Languages test"), ("locale", "bn")])).await;
    // (This fixture never stored `locale`; unset reads as English.)
    assert_ne!(f.state.settings.get_str("locale").as_deref(), Some("bn"), "General changed the language");
    assert_eq!(f.state.settings.get("locales_enabled"), Some(serde_json::json!(["bn"])));
}

/// The tag screen links the tag's archive in every language that uses it.
#[tokio::test]
async fn the_tag_screen_links_each_languages_archive() {
    let (f, cookie) = bilingual().await;
    let en = f.entry("post", "english-story", "English story", None, None, "published").await;
    let bn = f.entry("post", "bangla-golpo", "Bangla golpo", Some("bn"), None, "published").await;
    let bn2 = f.entry("post", "aro-golpo", "Aro golpo", Some("bn"), None, "published").await;
    let tag = |slug: &'static str| {
        let state = f.state.clone();
        async move {
            let id = ghost_core::ids::new_id();
            sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at) VALUES (?, ?, ?, 'public', '2026-11-01 00:00:00')")
                .bind(&id)
                .bind(slug)
                .bind(slug)
                .execute(&state.db)
                .await
                .unwrap();
            id
        }
    };
    let attach = |post: String, tag: String| {
        let state = f.state.clone();
        async move {
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
                .bind(ghost_core::ids::new_id())
                .bind(post)
                .bind(tag)
                .execute(&state.db)
                .await
                .unwrap();
        }
    };
    let shared = tag("shared-news").await;
    let bengali_only = tag("only-bangla").await;
    for p in [&en, &bn, &bn2] {
        attach(p.clone(), shared.clone()).await;
    }
    attach(bn.clone(), bengali_only.clone()).await;

    let page = send(&f.state, "GET", &format!("/tags/{shared}/"), Some(&cookie), None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#"<a href="/tag/shared-news/""#), "no English archive");
    assert!(page.body.contains(r#"<a href="/bn/tag/shared-news/""#), "no Bengali archive");
    assert!(page.body.contains("1 post<"), "English count");
    assert!(page.body.contains("2 posts<"), "Bengali count");

    // Used only by Bengali posts: only the Bengali archive is linked.
    let page = send(&f.state, "GET", &format!("/tags/{bengali_only}/"), Some(&cookie), None, None).await;
    assert!(page.body.contains(r#"<a href="/bn/tag/only-bangla/""#));
    assert!(!page.body.contains(r#"<a href="/tag/only-bangla/""#), "an English archive with nothing on it");
}

/// Everything outside the theme that names a page's language or links to an
/// entry: the head, the feed, the Markdown and llms.txt, the SEO report, and
/// the path newsletters and analytics use.
#[tokio::test]
async fn the_rest_of_the_site_knows_which_language_an_entry_is_in() {
    let (f, cookie) = bilingual().await;
    f.state.settings.set(&f.state.db, "llms_enabled", &serde_json::json!(true)).await.unwrap();
    // The same title in both languages — a brand name — is not a duplicate.
    let en = f.entry("post", "english-story", "Vuuut launch", None, None, "published").await;
    let bn = f.entry("post", "bangla-golpo", "Vuuut launch", Some("bn"), None, "published").await;

    // <head>: og:locale, the alternates, inLanguage, and this language's feed.
    let (_, page) = public(&f.state, "/bn/bangla-golpo/").await;
    assert!(page.contains(r#"<meta property="og:locale" content="bn" />"#), "og:locale");
    assert!(page.contains(r#"<meta property="og:locale:alternate" content="en" />"#), "og:locale:alternate");
    assert!(page.contains(r#""inLanguage": "bn""#), "inLanguage");
    assert!(page.contains(r#"type="application/rss+xml" title="Languages test" href="https://example.test/bn/rss/""#), "the Bengali page's feed");

    // The Bengali feed carries only Bengali posts, and says so.
    let (status, rss) = public(&f.state, "/bn/rss/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(rss.contains("<language>bn</language>"));
    assert!(rss.contains("https://example.test/bn/bangla-golpo/"));
    assert!(!rss.contains("english-story"), "an English post in the Bengali feed");
    let (_, rss_en) = public(&f.state, "/rss/").await;
    assert!(!rss_en.contains("bangla-golpo"), "a Bengali post in the English feed");

    // Markdown at the post's own URL, and not at the neutral one.
    let (status, _) = public(&f.state, "/bn/bangla-golpo.md").await;
    assert_eq!(status, StatusCode::OK, "the Bengali Markdown");
    let (status, _) = public(&f.state, "/bangla-golpo.md").await;
    assert_ne!(status, StatusCode::OK, "a Bengali post's Markdown on an English URL");
    let (_, llms) = public(&f.state, "/llms.txt").await;
    assert!(llms.contains("https://example.test/bn/bangla-golpo.md"), "{llms}");

    // The SEO report: no duplicate across languages, and the real path.
    let report = send(&f.state, "GET", "/seo/content/", Some(&cookie), None, None).await;
    // The filter menu always names the check; a finding carries its message.
    let _ = rules_label("duplicate_title");
    assert!(!report.body.contains("so the two compete for the same result"), "a translation flagged as a duplicate");
    assert!(report.body.contains("/bn/bangla-golpo/"), "the report shows the path without its prefix");

    // The path newsletters, Slack and analytics use.
    for (id, want) in [(&bn, "/bn/bangla-golpo/"), (&en, "/english-story/")] {
        let post = ghost_db::repos::posts::find_by_id(&f.state.db, id).await.unwrap().unwrap();
        assert_eq!(ghost_api::locales::entry_path(&f.state, &post).await, want);
    }
}

fn rules_label(code: &str) -> &'static str {
    ghost_core::seo::check_label(code)
}

/// A person opening a feed in a browser gets a page; a feed reader gets the
/// XML, unchanged.
#[tokio::test]
async fn a_feed_opened_in_a_browser_is_a_page_and_stays_xml_for_readers() {
    let (f, _) = bilingual().await;
    f.entry("post", "bangla-golpo", "Bangla golpo", Some("bn"), None, "published").await;

    let app = ghost_front::site_router(f.state.clone());
    let request = Request::builder()
        .uri("/bn/rss/")
        .header("host", HOST)
        .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("sec-fetch-mode", "navigate")
        .header("sec-fetch-dest", "document")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.headers()["content-type"], "text/html; charset=utf-8");
    assert!(response.headers()["vary"].to_str().unwrap().contains("Sec-Fetch-Mode"));
    let page = String::from_utf8_lossy(&to_bytes(response.into_body(), 5_000_000).await.unwrap()).into_owned();
    assert!(page.contains(r#"<html lang="bn">"#));
    assert!(page.contains(r#"value="https://example.test/bn/rss/""#), "the address to copy");
    assert!(page.contains("Bangla golpo"), "the posts");
    assert!(page.contains(r#"href="https://example.test/rss/""#), "the other language's feed");

    // A feed reader sends no Sec-Fetch pair: XML, as before.
    let (status, xml) = public(&f.state, "/bn/rss/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(xml.starts_with("<?xml"), "{}", &xml[..xml.len().min(80)]);
}
