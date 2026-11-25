//! Tools → Directory & Classified Listings end to end: categories and
//! listings built in the admin, the public directory switched on and read
//! through the real public router, a visitor's submission moderated, a
//! classified expiring — and the defences on the public form. Refusals
//! first. One test owns the public sequence, because the page cache's
//! content generation is process-wide.
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
        ("title".into(), serde_json::json!("Directory test")),
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

fn old_t() -> String {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 - 10_000).to_string()
}

async fn listing_id(f: &Fixture, title: &str) -> String {
    sqlx::query_scalar("SELECT id FROM directory_listings WHERE title = ?").bind(title).fetch_one(&f.state.db).await.unwrap()
}

#[tokio::test]
async fn the_tool_is_closed_to_strangers_and_settings_to_editors() {
    let f = fixture().await;
    for uri in ["/directories/", "/directories/listings/", "/directories/categories/", "/directories/settings/", "/directories/export.csv"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        assert_ne!(get(&f.state, uri, &f.author_session().await).await.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    let forged = send(&f.state, "POST", "/directories/listings/new/", Some(&cookie), Some("https://elsewhere.example"), Some(&[("title", "x")])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_listing_goes_from_draft_to_the_public_directory_and_visitors_are_moderated() {
    let f = fixture().await;
    let cookie = f.session().await;

    // Off: the public directory does not exist.
    assert_eq!(site(&f.state, "GET", "/directory/", None).await.status, StatusCode::NOT_FOUND);

    // A category, then a listing — a draft until published.
    post(&f.state, "/directories/categories/", &cookie, &[("name", "Food & drink"), ("tint", "amber"), ("description", "Places to eat")]).await;
    let cat: String = sqlx::query_scalar("SELECT id FROM directory_categories WHERE slug = 'food-drink'").fetch_one(&f.state.db).await.unwrap();
    let r = post(&f.state, "/directories/listings/new/", &cookie, &[
        ("kind", "directory"), ("title", "Corner <Café>"), ("category_id", &cat), ("summary", "Good coffee"),
        ("description", "We roast our own beans.\n\nOpen every day."), ("website", "https://corner.example/"),
        ("email", "hi@corner.example"), ("phone", "+1 555 010 2030"), ("location", "Springfield"), ("address", "1 Main St"),
    ]).await;
    assert!(!r.location.contains("error="), "{}", r.location);
    let cafe = listing_id(&f, "Corner <Café>").await;
    let bad = post(&f.state, "/directories/listings/new/", &cookie, &[("title", "Bad link"), ("website", "javascript:alert(1)")]).await;
    assert!(bad.location.contains("error="), "{}", bad.location);

    post(&f.state, "/directories/settings/", &cookie, &[
        ("directory_enabled", "1"), ("directory_title", "Town guide"), ("directory_submissions", "1"),
        ("directory_moderate", "1"), ("directory_show_contact", "1"), ("directory_classified_days", "30"),
        ("directory_per_page", "24"), ("directory_currency", "USD"),
    ]).await;
    let index = site(&f.state, "GET", "/directory/", None).await;
    assert_eq!(index.status, StatusCode::OK);
    assert!(!index.body.contains("Corner &lt;Café&gt;"), "a draft is not public");

    post(&f.state, &format!("/directories/listings/{cafe}/status/"), &cookie, &[("action", "publish")]).await;
    let index = site(&f.state, "GET", "/directory/", None).await;
    assert!(index.body.contains("Corner &lt;Café&gt;"), "{}", index.body);
    assert!(!index.body.contains("<Café>"), "escaped");
    assert!(index.body.contains("Town guide"));
    // Rendered in the theme: the page carries the theme's own document.
    assert!(index.body.contains("</html>") && index.body.contains("directory.css"));

    let one = site(&f.state, "GET", "/directory/corner-cafe/", None).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.body);
    assert!(one.body.contains("<p>We roast our own beans.</p><p>Open every day.</p>"), "{}", one.body);
    assert!(one.body.contains(r#""@type":"LocalBusiness""#));
    let slug: String = sqlx::query_scalar("SELECT slug FROM directory_listings WHERE id = ?").bind(&cafe).fetch_one(&f.state.db).await.unwrap();
    // The contact link goes to the stored website, and is counted.
    let go = site(&f.state, "GET", &format!("/directory/{}/go/website/", urlencode(&slug)), None).await;
    assert_eq!(go.status, StatusCode::FOUND);
    assert_eq!(go.headers.get("location").unwrap(), "https://corner.example/");
    let go = site(&f.state, "GET", &format!("/directory/{}/go/elsewhere/", urlencode(&slug)), None).await;
    assert_eq!(go.status, StatusCode::NOT_FOUND);
    let clicks: i64 = sqlx::query_scalar("SELECT contact_clicks FROM directory_listings WHERE id = ?").bind(&cafe).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(clicks, 1);

    // A full listing from the editor: the description written in Koenig,
    // photos, the address in parts, the map, opening hours, price range,
    // amenities, payments and profiles — and the status chosen in the form.
    let lexical = vuuut_core::dummies::document(vec![
        vuuut_core::dummies::paragraph("Roasted on site every morning."),
        vuuut_core::dummies::list(&["Flat white", "Cardamom bun"]),
    ]);
    let r = post(&f.state, &format!("/directories/listings/{cafe}/"), &cookie, &[
        ("kind", "directory"), ("title", "Corner <Café>"), ("slug", &slug), ("category_id", &cat), ("summary", "Good coffee"),
        ("description_lexical", &lexical), ("website", "https://corner.example/"), ("email", "hi@corner.example"),
        ("phone", "+1 555 010 2030"), ("location", "Springfield"), ("address", "1 Main St"), ("region", "IL"),
        ("postal_code", "62701"), ("country", "United States"), ("latitude", "39.7817"), ("longitude", "-89.6501"),
        ("hours_mon", "open"), ("hours_mon_open", "07:00"), ("hours_mon_close", "18:00"), ("hours_sun", "closed"),
        ("price_level", "2"), ("amenity", "wifi"), ("amenity", "made-up"), ("payment", "cash"),
        ("image", "https://img.example/front.jpg"), ("image", "https://img.example/inside.jpg"),
        ("social_instagram", "https://instagram.com/corner"), ("booking_url", "https://book.example/corner"),
        ("status", "published"),
    ]).await;
    assert!(!r.location.contains("error="), "{}", r.location);
    let stored = vuuut_db::repos::directory::find(&f.state.db, &cafe).await.unwrap().unwrap();
    assert_eq!(stored.status, "published");
    assert_eq!((stored.image.as_str(), stored.gallery.as_str()), ("https://img.example/front.jpg", "https://img.example/inside.jpg"));
    assert_eq!(stored.amenities, "wifi", "only choices on the list are kept");
    assert!(stored.description.contains("Roasted on site"), "plain text kept for search and cards");
    let one = site(&f.state, "GET", &format!("/directory/{}/", urlencode(&slug)), None).await;
    for needle in ["Roasted on site every morning.", "Cardamom bun", "7 am – 6 pm", "Closed", "Free Wi-Fi", "Cash", "$$",
        "https://img.example/inside.jpg", "https://instagram.com/corner", "https://book.example/corner",
        "1 Main St, Springfield, IL 62701, United States", "openingHoursSpecification", r#""priceRange":"$$""#, "GeoCoordinates"] {
        assert!(one.body.contains(needle), "{needle} missing: {}", one.body);
    }
    assert!(site(&f.state, "GET", "/directory/?q=roasted", None).await.body.contains("Corner &lt;Café&gt;"));
    assert!(site(&f.state, "GET", "/directory/?price=2", None).await.body.contains("Corner &lt;Café&gt;"));
    assert!(!site(&f.state, "GET", "/directory/?price=4", None).await.body.contains("Corner &lt;Café&gt;"));
    assert!(site(&f.state, "GET", "/directory/?what=food-drink&where=springfield", None).await.body.contains("Corner &lt;Café&gt;"));
    // Saving without a status field leaves it live.
    post(&f.state, &format!("/directories/listings/{cafe}/"), &cookie, &[("kind", "directory"), ("title", "Corner <Café>"), ("slug", &slug), ("category_id", &cat), ("summary", "Good coffee"), ("website", "https://corner.example/")]).await;
    assert_eq!(vuuut_db::repos::directory::find(&f.state.db, &cafe).await.unwrap().unwrap().status, "published");
    // The editor opens it in Koenig with every field filled in.
    let editor = get(&f.state, &format!("/directories/listings/{cafe}/"), &cookie).await.body;
    assert!(editor.contains("koenig-mount") && editor.contains("directory-editor.js"), "{editor}");

    // Search and category pages.
    assert!(site(&f.state, "GET", "/directory/?q=coffee", None).await.body.contains("Corner &lt;Café&gt;"));
    assert!(!site(&f.state, "GET", "/directory/?q=bicycle", None).await.body.contains("Corner &lt;Café&gt;"));
    assert!(site(&f.state, "GET", "/directory/category/food-drink/", None).await.body.contains("Places to eat"));

    // A visitor's classified: the defences first.
    let form = [
        ("kind", "classified"), ("title", "Red bicycle"), ("category_id", cat.as_str()), ("price_type", "fixed"), ("price", "120"),
        ("description", "A red bicycle, barely ridden, with a basket."), ("submitter_email", "rider@example.test"),
    ];
    let mut honeypot = form.to_vec();
    honeypot.extend([("website_url", "http://spam"), ("t", "1")]);
    site(&f.state, "POST", "/directory/submit/", Some(&honeypot)).await;
    let t = old_t();
    let mut fast = form.to_vec();
    let now_t = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64).to_string();
    fast.push(("t", &now_t));
    site(&f.state, "POST", "/directory/submit/", Some(&fast)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM directory_listings WHERE title = 'Red bicycle'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(count, 0, "a bot and a too-fast form store nothing");

    let mut real = form.to_vec();
    real.push(("t", &t));
    let sent = site(&f.state, "POST", "/directory/submit/", Some(&real)).await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER, "{}", sent.body);
    assert!(sent.headers.get("location").unwrap().to_str().unwrap().contains("sent=1"));
    let bike = listing_id(&f, "Red bicycle").await;
    assert!(!site(&f.state, "GET", "/directory/", None).await.body.contains("Red bicycle"), "waits for review");
    let overview = get(&f.state, "/directories/", &cookie).await;
    assert!(overview.body.contains("Red bicycle") && overview.body.contains("Approve"), "{}", overview.body);

    post(&f.state, &format!("/directories/listings/{bike}/status/"), &cookie, &[("action", "publish")]).await;
    let index = site(&f.state, "GET", "/directory/", None).await;
    assert!(index.body.contains("Red bicycle") && index.body.contains("$120"), "{}", index.body);
    let expires: Option<String> = sqlx::query_scalar("SELECT expires_at FROM directory_listings WHERE id = ?").bind(&bike).fetch_one(&f.state.db).await.unwrap();
    assert!(expires.is_some(), "a published classified has an end date");

    // Expired, it leaves the site; renewed, it comes back.
    sqlx::query("UPDATE directory_listings SET expires_at = '2000-01-01 00:00:00' WHERE id = ?").bind(&bike).execute(&f.state.db).await.unwrap();
    vuuut_db::content::touch();
    assert!(!site(&f.state, "GET", "/directory/", None).await.body.contains("Red bicycle"));
    post(&f.state, &format!("/directories/listings/{bike}/status/"), &cookie, &[("action", "renew")]).await;
    assert!(site(&f.state, "GET", "/directory/", None).await.body.contains("Red bicycle"));

    // The export defuses and quotes.
    let csv = get(&f.state, "/directories/export.csv", &cookie).await;
    assert!(csv.body.contains("\"Red bicycle\",\"classified\",\"published\""), "{}", csv.body);
}

fn urlencode(s: &str) -> String {
    vuuut_core::social_media::encode(s)
}

#[tokio::test]
async fn directory_addresses_move_public_routes_and_keep_default_links() {
    let f = fixture().await;
    let cookie = f.session().await;
    let saved = post(&f.state, "/directories/settings/", &cookie, &[
        ("directory_enabled", "1"),
        ("directory_submissions", "1"),
        ("directory_base_path", "/Town Guide/"),
        ("directory_listing_path", "places"),
    ]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);

    post(&f.state, "/directories/categories/", &cookie, &[("name", "Food")]).await;
    let food: String = sqlx::query_scalar("SELECT id FROM directory_categories WHERE slug = 'food'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let saved = post(&f.state, "/directories/listings/new/", &cookie, &[
        ("title", "Corner Cafe"),
        ("category_id", &food),
        ("website", "https://corner.example/"),
    ]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let cafe = listing_id(&f, "Corner Cafe").await;
    post(&f.state, &format!("/directories/listings/{cafe}/status/"), &cookie, &[("action", "publish")]).await;

    let page = get(&f.state, "/directories/settings/", &cookie).await;
    assert!(page.body.contains(r#"name="directory_base_path" value="town-guide""#));
    let overview = get(&f.state, "/directories/", &cookie).await;
    assert!(overview.body.contains(r#"href="/town-guide/""#));
    let listing = get(&f.state, &format!("/directories/listings/{cafe}/"), &cookie).await;
    assert!(listing.body.contains(r#"href="/places/corner-cafe/""#));

    let index = site(&f.state, "GET", "/town-guide/?q=coffee", None).await;
    assert_eq!(index.status, StatusCode::OK);
    let index = site(&f.state, "GET", "/town-guide/", None).await;
    assert!(index.body.contains(r#"href="/places/corner-cafe/""#));
    assert!(index.body.contains(r#"href="/town-guide/category/food/""#));
    let css = index.body.split(r#"href="/town-guide/assets/"#).nth(1).unwrap().split('"').next().unwrap();
    assert_eq!(site(&f.state, "GET", &format!("/town-guide/assets/{css}"), None).await.status, StatusCode::OK);
    assert_eq!(site(&f.state, "GET", "/town-guide/category/food/", None).await.status, StatusCode::OK);

    let listing = site(&f.state, "GET", "/places/corner-cafe/", None).await;
    assert_eq!(listing.status, StatusCode::OK);
    assert!(listing.body.contains("/places/corner-cafe/go/website/"));
    let contact = site(&f.state, "GET", "/places/corner-cafe/go/website/", None).await;
    assert_eq!(contact.headers["location"], "https://corner.example/");
    let form = site(&f.state, "GET", "/town-guide/submit/", None).await;
    assert!(form.body.contains(r#"action="/town-guide/submit/""#));

    for (method, old, new) in [
        ("GET", "/directory/?q=coffee", "/town-guide/?q=coffee"),
        ("GET", "/directory/corner-cafe?ref=old", "/places/corner-cafe/?ref=old"),
        ("GET", "/directory/category/food/", "/town-guide/category/food/"),
        ("POST", "/directory/submit/", "/town-guide/submit/"),
    ] {
        let redirect = site(&f.state, method, old, None).await;
        assert_eq!(redirect.status, StatusCode::PERMANENT_REDIRECT, "{old}");
        assert_eq!(redirect.headers["location"], new);
    }

    let invalid = post(&f.state, "/directories/settings/", &cookie, &[
        ("directory_base_path", "town-guide"),
        ("directory_listing_path", "town-guide/category"),
    ]).await;
    assert!(invalid.location.contains("error="));
    let conflict = post(&f.state, "/ecom/settings/", &cookie, &[
        ("ecom_base_path", "places"),
        ("ecom_product_style", "under"),
    ]).await;
    assert!(conflict.location.contains("error="));
    assert_eq!(f.state.settings.get_str("directory_listing_path").as_deref(), Some("places"));

    for (prefix, listing_url) in [
        ("town-guide/listings", "/town-guide/listings/corner-cafe/"),
        ("", "/town-guide/corner-cafe/"),
    ] {
        let saved = post(&f.state, "/directories/settings/", &cookie, &[
            ("directory_enabled", "1"),
            ("directory_listing_path", prefix),
        ]).await;
        assert!(saved.location.contains("ok="), "{}", saved.location);
        let index = site(&f.state, "GET", "/town-guide/", None).await;
        assert!(index.body.contains(&format!(r#"href="{listing_url}""#)));
        assert_eq!(site(&f.state, "GET", listing_url, None).await.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn directory_page_size_accepts_custom_numbers_and_defaults_to_ten() {
    use vuuut_core::directory::ListingInput;
    use vuuut_db::repos::directory::{insert, save_category};

    let f = fixture().await;
    let cookie = f.session().await;
    let settings = "/directories/settings/";
    let enabled = post(&f.state, settings, &cookie, &[("directory_enabled", "1")]).await;
    assert!(enabled.location.contains("ok="), "{}", enabled.location);

    let category = save_category(&f.state.db, None, "Places", "places", "", "blue", 0).await.unwrap();
    for n in 1..=501 {
        let listing = ListingInput {
            slug: format!("test-place-{n}"),
            title: format!("Test place {n}"),
            kind: "directory".into(),
            category_id: Some(category.clone()),
            ..Default::default()
        };
        insert(&f.state.db, &listing, "published", None, None).await.unwrap();
    }

    let page = get(&f.state, settings, &cookie).await;
    assert!(page.body.contains(r#"name="directory_per_page" type="number" min="1" step="1" value="10""#));
    let first = site(&f.state, "GET", "/directory/", None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body.matches(r#"class="vd-card""#).count(), 10);
    assert!(first.body.contains("Page 1 of 51"));

    let saved = post(&f.state, settings, &cookie, &[("directory_enabled", "1"), ("directory_per_page", "7")]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let first = site(&f.state, "GET", "/directory/", None).await;
    let second = site(&f.state, "GET", "/directory/category/places/?page=2&q=test", None).await;
    assert_eq!(first.body.matches(r#"class="vd-card""#).count(), 7);
    assert_eq!(second.body.matches(r#"class="vd-card""#).count(), 7);
    assert!(second.body.contains("Page 2 of 72"));
    assert!(second.body.contains("/directory/category/places/?page=3&amp;q=test"));

    let invalid = post(&f.state, settings, &cookie, &[("directory_enabled", "1"), ("directory_per_page", "0")]).await;
    assert!(invalid.location.contains("error="), "{}", invalid.location);
    let saved = post(&f.state, settings, &cookie, &[("directory_enabled", "1")]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    assert_eq!(f.state.settings.get("directory_per_page"), Some(serde_json::json!(7)));

    let large = i64::MAX.to_string();
    let saved = post(&f.state, settings, &cookie, &[("directory_enabled", "1"), ("directory_per_page", &large)]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let all = site(&f.state, "GET", "/directory/", None).await;
    assert_eq!(all.body.matches(r#"class="vd-card""#).count(), 501);
    let far = site(&f.state, "GET", &format!("/directory/?page={large}"), None).await;
    assert_eq!(far.status, StatusCode::OK);
    assert_eq!(far.body.matches(r#"class="vd-card""#).count(), 0);

    let cleared = post(&f.state, settings, &cookie, &[("directory_enabled", "1"), ("directory_per_page", "")]).await;
    assert!(cleared.location.contains("ok="), "{}", cleared.location);
    assert_eq!(f.state.settings.get("directory_per_page"), Some(serde_json::json!(10)));
}
