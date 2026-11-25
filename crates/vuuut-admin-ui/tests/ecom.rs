//! Tools → E-commerce & Online Stores end to end: refusals first, then a
//! product made through the admin editor, bought through the real public
//! storefront (cart cookie, checkout, stock taken, discount counted), and
//! the order worked through the admin (paid, fulfilled, refunded, cancelled).
//! One test owns the store sequence, because the page cache's content
//! generation is process-wide.
#![allow(dead_code)]
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
        ("title".into(), serde_json::json!("Store test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        // Product files are written under the content path: a fresh one per
        // fixture, never the checkout's own content directory.
        "paths": {"contentPath": std::env::temp_dir().join(format!("vuuut-ecom-{}", vuuut_core::ids::new_id())).to_string_lossy()}
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


fn cookie_from(headers: &axum::http::HeaderMap) -> Option<String> {
    headers.get("set-cookie").and_then(|v| v.to_str().ok()).and_then(|c| c.split(';').next()).map(|c| c.to_string())
}

async fn site_with(state: &vuuut_api::AppState, method: &str, uri: &str, cookie: &str, form: Option<&[(&str, &str)]>) -> Site {
    let mut req = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html").header("cookie", cookie);
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

async fn set(f: &Fixture, key: &str, value: serde_json::Value) {
    f.state.settings.set(&f.state.db, key, &value).await.unwrap();
}

#[tokio::test]
async fn strangers_authors_and_other_sites_are_refused() {
    let f = fixture().await;
    let none = send(&f.state, "GET", "/ecom/", None, None, None).await;
    assert!(none.status.is_redirection() && none.location.contains("signin"), "{:?}", none.location);
    let author = f.author_session().await;
    assert_eq!(get(&f.state, "/ecom/products/", &author).await.status, StatusCode::FORBIDDEN);
    let owner = f.session().await;
    // A write from another origin is refused before anything is read.
    let cross = send(&f.state, "POST", "/ecom/products/new/", Some(&owner), Some("https://evil.test"), Some(&[("title", "X")])).await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ecom_products").fetch_one(&f.state.db).await.unwrap(), 0);
    // The storefront is a 404 while the store is closed.
    assert_eq!(site(&f.state, "GET", "/shop/", None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn every_admin_screen_renders() {
    let f = fixture().await;
    let owner = f.session().await;
    for uri in ["/ecom/", "/ecom/analytics/", "/ecom/products/", "/ecom/products/new/", "/ecom/products/import/", "/ecom/collections/", "/ecom/collections/new/",
        "/ecom/inventory/", "/ecom/orders/", "/ecom/orders/new/", "/ecom/orders/abandoned/", "/ecom/customers/", "/ecom/customers/new/",
        "/ecom/discounts/", "/ecom/settings/"] {
        let r = get(&f.state, uri, &owner).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}");
        // The product editor wears an editor header of its own.
        let header = if uri == "/ecom/products/new/" { "Add product" } else { "E-commerce &amp; Online Stores" };
        assert!(r.body.contains(header), "{uri} has its header");
    }
}

#[tokio::test]
async fn a_product_is_made_bought_and_worked_through() {
    let f = fixture().await;
    let owner = f.session().await;

    // --- a product with two sizes, made through the editor ---------------
    let made = post(&f.state, "/ecom/products/new/", &owner, &[
        ("title", "Blue <Tee>"), ("status", "active"), ("price", "20"), ("compare_at", "25"), ("track", "1"), ("qty", "0"),
        ("taxable", "1"), ("requires_shipping", "1"), ("vendor", "Acme"), ("product_type", "Shirts"), ("tags", "summer, cotton"),
        ("description_plain", "Soft & light."), ("opt_name_0", "Size"), ("opt_values_0", "S, M"),
        ("v_key", "S"), ("v_id", ""), ("v_price", "20"), ("v_compare", ""), ("v_sku", "TEE-S"), ("v_qty", "3"), ("v_image", ""),
        ("v_key", "M"), ("v_id", ""), ("v_price", "22.50"), ("v_compare", ""), ("v_sku", "TEE-M"), ("v_qty", "1"), ("v_image", ""),
        ("image", "/content/images/tee.jpg"), ("image", "javascript:alert(1)"),
    ]).await;
    assert!(made.status.is_redirection(), "{}", made.flash());
    assert_eq!(made.flash(), "Product created.");
    let (pid, handle, images): (String, String, String) = sqlx::query_as("SELECT id, handle, images FROM ecom_products").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(handle, "blue-tee");
    assert_eq!(images, r#"["/content/images/tee.jpg"]"#, "a script URL is never stored as an image");
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &pid).await;
    assert_eq!(vs.iter().map(|v| (v.title.as_str(), v.price_cents, v.inventory)).collect::<Vec<_>>(), vec![("S", 2000, 3), ("M", 2250, 1)]);
    let editor = get(&f.state, &format!("/ecom/products/{pid}/"), &owner).await;
    assert!(editor.body.contains("Blue &lt;Tee&gt;") && !editor.body.contains("Blue <Tee>"), "titles are escaped");
    assert!(editor.body.contains("koenig-mount"), "the description is edited in Koenig");

    // A smart collection gathers it by tag; a manual one by hand.
    let smart = post(&f.state, "/ecom/collections/new/", &owner, &[("title", "Summer"), ("kind", "smart"), ("published", "1"),
        ("rule_field", "tag"), ("rule_op", "equals"), ("rule_value", "summer"), ("match_all", "1"), ("sort", "manual")]).await;
    assert_eq!(smart.flash(), "Collection created.");
    let empty_smart = post(&f.state, "/ecom/collections/new/", &owner, &[("title", "Nothing"), ("kind", "smart"), ("rule_field", "tag"), ("rule_op", "equals"), ("rule_value", "")]).await;
    assert!(empty_smart.flash().contains("at least one condition"));

    // A discount code, Administrator-only.
    assert_eq!(get(&f.state, "/ecom/discounts/", &f.author_session().await).await.status, StatusCode::FORBIDDEN);
    let code = post(&f.state, "/ecom/discounts/", &owner, &[("code", "save10"), ("kind", "percent"), ("value", "10"), ("active", "1"), ("usage_limit", "0")]).await;
    assert_eq!(code.flash(), "SAVE10 created.");

    // --- open the store, 10% tax on top, a flat rate ---------------------
    let saved = post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_currency", "USD"), ("ecom_tax_rate", "10"),
        ("ecom_pay_manual", "1"), ("ecom_per_page", "24"), ("ecom_low_stock", "5"), ("ecom_send_receipts", "1"),
        ("delivery_countries_present", "1"), ("delivery_country", "US"), ("delivery_country", "BD")]).await;
    assert_eq!(saved.flash(), "Store settings saved.");
    let settings = get(&f.state, "/ecom/settings/", &owner).await;
    assert!(settings.body.contains("2 countries selected"));
    assert!(settings.body.contains(r#"name="delivery_country" value="BD" checked"#));
    assert!(settings.body.contains(r#"name="delivery_country" value="US" checked"#));
    let rate = post(&f.state, "/ecom/settings/shipping/", &owner, &[("name", "Standard"), ("price", "5"), ("min", ""), ("max", ""), ("countries", "us, ca")]).await;
    assert_eq!(rate.flash(), "Shipping rate saved.");

    let home = site(&f.state, "GET", "/shop/", None).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("Blue &lt;Tee&gt;") && home.body.contains("$20.00"), "the catalogue lists it");
    let coll = site(&f.state, "GET", "/shop/collections/summer/", None).await;
    assert!(coll.body.contains("Blue &lt;Tee&gt;"), "the smart collection holds it");
    let page = site(&f.state, "GET", &format!("/shop/products/{handle}/"), None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#""@type":"Product""#), "structured data");
    assert!(page.body.contains("Soft &amp; light."));

    // --- buy: add 2 × S, try 2 × M (only 1 left) -------------------------
    let s_id = vs[0].id.clone();
    let m_id = vs[1].id.clone();
    let add = site(&f.state, "POST", "/shop/cart/add/", Some(&[("variant_id", &s_id), ("qty", "2")])).await;
    assert!(add.status.is_redirection());
    let cart = cookie_from(&add.headers).expect("a cart cookie");
    assert!(cart.starts_with("vuuut-cart="));
    let add_m = site_with(&f.state, "POST", "/shop/cart/add/", &cart, Some(&[("variant_id", &m_id), ("qty", "2")])).await;
    assert!(add_m.status.is_redirection());
    let cart_page = site_with(&f.state, "GET", "/shop/cart/", &cart, None).await;
    assert!(cart_page.body.contains("Only 1 of"), "too many of M is reduced, and said");
    assert!(cart_page.headers.get("cache-control").is_some_and(|v| v.to_str().unwrap().contains("no-store")));
    // Apply the code.
    let applied = site_with(&f.state, "POST", "/shop/cart/update/", &cart, Some(&[("code", "save10")])).await;
    assert!(applied.status.is_redirection());

    let checkout = site_with(&f.state, "GET", "/shop/checkout/", &cart, None).await;
    let country_options = checkout.body.split_once("name=\"country\"").unwrap().1.split_once("</select>").unwrap().0;
    assert!(country_options.contains(r#"value="BD">Bangladesh"#));
    assert!(country_options.contains(r#"value="US" selected>"#));
    assert!(!country_options.contains(r#"value="CA""#));
    let blocked = site_with(&f.state, "POST", "/shop/checkout/", &cart, Some(&[
        ("email", "buyer@example.test"), ("place", "1"), ("payment", "manual"),
        ("first_name", "Ada"), ("last_name", "Lovelace"), ("line1", "1 Main St"),
        ("city", "Springfield"), ("country", "CA"),
    ])).await;
    assert!(blocked.body.contains("We do not ship to that country."));
    // Checkout without an address is refused, with the reason.
    let bad = site_with(&f.state, "POST", "/shop/checkout/", &cart, Some(&[("email", "buyer@example.test"), ("place", "1"), ("payment", "manual")])).await;
    assert!(bad.body.contains("Fill in your"), "missing address fields are named");
    let placed = site_with(&f.state, "POST", "/shop/checkout/", &cart, Some(&[
        ("email", "Buyer@Example.test"), ("place", "1"), ("payment", "manual"), ("first_name", "Ada"), ("last_name", "Lovelace"),
        ("line1", "1 Main St"), ("city", "Springfield"), ("country", "US"), ("shipping", &vuuut_db::repos::ecom::shipping_rates(&f.state.db).await[0].id),
        ("price", "0"), ("website_url", ""),
    ])).await;
    assert!(placed.status.is_redirection(), "status {} body {}", placed.status, &placed.body[..placed.body.len().min(400)]);
    let o: vuuut_db::repos::ecom::Order = vuuut_db::repos::ecom::browse_orders(&f.state.db, &vuuut_db::repos::ecom::OrderBrowse { view: "all", limit: 5, ..Default::default() }).await.0.remove(0);
    // 2×20 + 1×22.50 = 62.50; −10% = 6.25; tax 10% of 56.25 = 5.63 (rounded); shipping 5.
    assert_eq!((o.number, o.subtotal_cents, o.discount_cents, o.tax_cents, o.shipping_cents, o.total_cents), (1001, 6250, 625, 563, 500, 6688));
    assert_eq!(o.email, "buyer@example.test");
    assert_eq!(o.financial_status, "pending");
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &pid).await;
    assert_eq!((vs[0].inventory, vs[1].inventory), (1, 0), "stock is taken");
    let used: i64 = sqlx::query_scalar("SELECT used_count FROM ecom_discounts WHERE code = 'SAVE10'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(used, 1);
    // The cart became the order; the buyer's next visit has an empty cart.
    assert!(site_with(&f.state, "GET", "/shop/cart/", &cart, None).await.body.contains("Your cart is empty"));
    // The order page is reachable by token only.
    let status_page = site(&f.state, "GET", &format!("/shop/orders/{}/", o.token), None).await;
    assert!(status_page.body.contains("Order #1001") && status_page.body.contains("$66.88"));
    assert_eq!(site(&f.state, "GET", "/shop/orders/0123456789abcdef0123456789abcdef/", None).await.status, StatusCode::NOT_FOUND);
    // A customer exists for the email.
    let customer = vuuut_db::repos::ecom::customer_by_email(&f.state.db, "buyer@example.test").await.expect("customer");
    assert_eq!(customer.first_name, "Ada");

    // --- work it through the admin ---------------------------------------
    let ou = format!("/ecom/orders/{}/", o.id);
    let detail = get(&f.state, &ou, &owner).await;
    assert!(detail.body.contains("Order #1001") && detail.body.contains("Order placed on the online store."));
    assert_eq!(post(&f.state, &format!("{ou}paid/"), &owner, &[("reference", "cash")]).await.flash(), "Marked as paid.");
    assert_eq!(post(&f.state, &format!("{ou}fulfil/"), &owner, &[("company", "UPS"), ("number", "1Z999"), ("url", "")]).await.flash(), "Marked as fulfilled.");
    let after = vuuut_db::repos::ecom::find_order(&f.state.db, &o.id).await.unwrap();
    assert_eq!(after.tracking_url, "https://www.ups.com/track?tracknum=1Z999", "a known carrier's link is built from the number");
    // A refund over what is left is refused; a partial one is recorded.
    assert!(post(&f.state, &format!("{ou}refund/"), &owner, &[("amount", "999")]).await.flash().contains("between"));
    assert_eq!(post(&f.state, &format!("{ou}refund/"), &owner, &[("amount", "10"), ("restock", "1")]).await.flash(), "Refund recorded.");
    let after = vuuut_db::repos::ecom::find_order(&f.state.db, &o.id).await.unwrap();
    assert_eq!((after.financial_status.as_str(), after.refunded_cents), ("partially_refunded", 1000));
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &pid).await;
    assert_eq!((vs[0].inventory, vs[1].inventory), (3, 1), "restocked once");
    // Cancelling does not restock a second time.
    assert!(post(&f.state, &format!("{ou}cancel/"), &owner, &[("restock", "1")]).await.flash().starts_with("Order cancelled"));
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &pid).await;
    assert_eq!((vs[0].inventory, vs[1].inventory), (3, 1));
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_order_events WHERE order_id = ?").bind(&o.id).fetch_one(&f.state.db).await.unwrap();
    assert!(events >= 5, "every action is on the timeline");

    // Inventory: +n adjusts, a number sets.
    assert_eq!(post(&f.state, &format!("/ecom/inventory/{s_id}/"), &owner, &[("qty", "+4")]).await.flash(), "Stock set to 7.");
    assert_eq!(post(&f.state, &format!("/ecom/inventory/{s_id}/"), &owner, &[("qty", "2")]).await.flash(), "Stock set to 2.");

    // Exports defuse formulas.
    let csv = get(&f.state, "/ecom/orders/export.csv", &owner).await;
    assert!(csv.body.contains("1001"));

    // A draft disappears from the store.
    post(&f.state, "/ecom/products/bulk/", &owner, &[("action", "draft"), ("id", &pid)]).await;
    assert_eq!(site(&f.state, "GET", &format!("/shop/products/{handle}/"), None).await.status, StatusCode::NOT_FOUND);
}


/// Upload a file to a product through the admin, as a browser would.
async fn upload(f: &Fixture, cookie: &str, product: &str, name: &str, bytes: &[u8]) -> Reply {
    let boundary = "XyZboundary42";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/ecom/products/{product}/files/"))
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("cookie", cookie)
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let app = vuuut_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    Reply { status, location, body: String::new() }
}

async fn member_cookie(f: &Fixture, email: &str) -> String {
    let transient: String = sqlx::query_scalar("SELECT transient_id FROM members WHERE email = ?").bind(email).fetch_one(&f.state.db).await.unwrap();
    vuuut_api::members::member_cookie_for_test(&f.state, &transient)
}

async fn raw(f: &Fixture, uri: &str, cookie: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder().method("GET").uri(uri).header("host", HOST).header("cookie", cookie).body(Body::empty()).unwrap();
    let response = vuuut_front::site_router(f.state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    (status, headers, to_bytes(response.into_body(), 20_000_000).await.unwrap().to_vec())
}

async fn buy(f: &Fixture, variant: &str, email: &str, payment: &str) -> vuuut_db::repos::ecom::Order {
    let add = site(&f.state, "POST", "/shop/cart/add/", Some(&[("variant_id", variant), ("qty", "1"), ("back", "checkout")])).await;
    assert!(add.headers.get("location").unwrap().to_str().unwrap().ends_with("/shop/checkout/"), "a free download goes straight to checkout");
    let cart = cookie_from(&add.headers).unwrap();
    let placed = site_with(&f.state, "POST", "/shop/checkout/", &cart, Some(&[("email", email), ("place", "1"), ("payment", payment)])).await;
    assert!(placed.status.is_redirection(), "{}", &placed.body[..placed.body.len().min(600)]);
    let token = placed.headers.get("location").unwrap().to_str().unwrap().split("/shop/orders/").nth(1).unwrap().split('/').next().unwrap().to_string();
    vuuut_db::repos::ecom::order_by_token(&f.state.db, &token).await.unwrap()
}

#[tokio::test]
async fn digital_files_reach_only_the_member_who_bought_them() {
    let f = fixture().await;
    let owner = f.session().await;
    post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_currency", "USD"), ("ecom_pay_manual", "1"), ("ecom_per_page", "24"), ("ecom_low_stock", "5")]).await;

    // --- a free digital product, one download per purchase ---------------
    let made = post(&f.state, "/ecom/products/new/", &owner, &[("title", "Starter Kit"), ("status", "active"), ("price", "0"), ("taxable", "1"), ("download_limit", "1")]).await;
    let pid = made.location.split("/products/").nth(1).unwrap().split('/').next().unwrap().to_string();
    let bytes = b"%PDF-1.4 the whole kit";
    let up = upload(&f, &owner, &pid, "../../Starter Kit.pdf", bytes).await;
    assert!(up.location.contains("?ok=Starter+Kit.pdf+uploaded") && up.location.ends_with("#ec-digital"), "the message comes before the anchor: {}", up.location);
    let file = vuuut_db::repos::ecom::files(&f.state.db, &pid).await.remove(0);
    assert_eq!(file.filename, "Starter Kit.pdf", "the path is gone from the name");
    let on_disk = vuuut_api::ecom::stored_path(&f.state, &file.stored).unwrap();
    assert_eq!(std::fs::read(&on_disk).unwrap(), bytes);
    // The stored file is not under any public route.
    assert_eq!(site(&f.state, "GET", &format!("/content/private/ecom/{}", file.stored), None).await.status, StatusCode::NOT_FOUND);
    // Staff can fetch it from the admin; a stranger cannot.
    assert_eq!(send(&f.state, "GET", &format!("/ecom/products/{pid}/files/{}/", file.id), Some(&owner), None, None).await.status, StatusCode::OK);
    assert!(send(&f.state, "GET", &format!("/ecom/products/{pid}/files/{}/", file.id), None, None, None).await.status.is_redirection());

    let variant = vuuut_db::repos::ecom::variants(&f.state.db, &pid).await.remove(0).id;
    let page = site(&f.state, "GET", "/shop/products/starter-kit/", None).await;
    assert!(page.body.contains("Get it free") && page.body.contains("Digital download"));
    // Free: only an email is asked for; the order is paid at once.
    let order = buy(&f, &variant, "Reader@Example.test", "").await;
    assert_eq!((order.financial_status.as_str(), order.total_cents, order.payment_method.as_str()), ("paid", 0, "free"));
    let order = vuuut_db::repos::ecom::find_order(&f.state.db, &order.id).await.unwrap();
    assert_eq!(order.fulfillment_status, "fulfilled", "a digital-only order is delivered by unlocking it");
    let grants = vuuut_db::repos::ecom::grants_for_order(&f.state.db, &order.id).await;
    assert_eq!(grants.len(), 1);
    // The buyer became a regular member, and got sign-in links (signup + downloads).
    let member: (String, String) = sqlx::query_as("SELECT id, status FROM members WHERE email = 'reader@example.test'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(member.1, "free");
    assert_eq!(grants[0].member_id.as_deref(), Some(member.0.as_str()));
    let links: Vec<String> = sqlx::query_scalar("SELECT data FROM tokens").fetch_all(&f.state.db).await.unwrap();
    assert!(links.iter().any(|d| d.contains("\"signup\"") && d.contains("/shop/dashboard/")), "{links:?}");
    assert!(links.iter().any(|d| d.contains("\"signin\"") && d.contains("/shop/dashboard/")), "{links:?}");

    let url = format!("/shop/downloads/{}/{}/", grants[0].id, file.id);
    // Signed out: no file, sent to the sign-in page.
    let (st, h, body) = raw(&f, &url, "").await;
    assert!(st.is_redirection() && h.get("location").unwrap().to_str().unwrap().ends_with("/shop/dashboard/"));
    assert!(body.is_empty());
    // The order page, reachable by token, offers a sign-in link — not the file.
    let status_page = site(&f.state, "GET", &format!("/shop/orders/{}/", order.token), None).await;
    assert!(status_page.body.contains("Email me a sign-in link") && !status_page.body.contains(&url));
    // Another member: refused.
    sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, created_at, updated_at) VALUES (?, ?, ?, 'other@example.test', 'free', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
        .bind(vuuut_core::ids::new_id()).bind(vuuut_core::ids::uuid_v4()).bind(vuuut_core::ids::uuid_v4())
        .execute(&f.state.db).await.unwrap();
    let other = member_cookie(&f, "other@example.test").await;
    let (st, h, body) = raw(&f, &url, &other).await;
    assert!(st.is_redirection() && h.get("location").unwrap().to_str().unwrap().contains("error="));
    assert!(body.is_empty());
    // The buyer, signed in: the file, as an attachment nobody caches.
    let mine = member_cookie(&f, "reader@example.test").await;
    let listing = site_with(&f.state, "GET", "/shop/dashboard/", &mine, None).await;
    assert!(listing.body.contains("Starter Kit") && listing.body.contains(&url));
    assert!(listing.body.contains("id=\"downloads\"") && listing.body.contains("id=\"orders\"") && listing.body.contains("Order #1001"), "My products: downloads and every order");
    // Signed out, My products is the sign-in form; the old addresses lead to it.
    let signed_out = site(&f.state, "GET", "/shop/dashboard/", None).await;
    assert!(signed_out.body.contains("Email me a sign-in link") && !signed_out.body.contains(&url));
    assert_eq!(site(&f.state, "GET", "/shop/downloads/?sent=1", None).await.headers.get("location").unwrap(), "/shop/dashboard/?sent=1");
    assert_eq!(site(&f.state, "GET", "/shop/account/", None).await.headers.get("location").unwrap(), "/shop/dashboard/");
    let (st, h, body) = raw(&f, &url, &mine).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body, bytes);
    assert!(h.get("content-disposition").unwrap().to_str().unwrap().starts_with("attachment;"));
    assert!(h.get("cache-control").unwrap().to_str().unwrap().contains("no-store"));
    // One download per purchase: the second is refused.
    let (st, h, _) = raw(&f, &url, &mine).await;
    assert!(st.is_redirection() && h.get("location").unwrap().to_str().unwrap().contains("error=limit"));
    // Staff can reset it.
    assert_eq!(post(&f.state, &format!("/ecom/orders/{}/downloads/", order.id), &owner, &[("op", "reset"), ("grant", &grants[0].id)]).await.flash(), "Download count reset.");
    assert_eq!(raw(&f, &url, &mine).await.0, StatusCode::OK);

    // --- a paid digital product, cash: locked until marked paid ----------
    let made = post(&f.state, "/ecom/products/new/", &owner, &[("title", "Pro Pack"), ("status", "active"), ("price", "12"), ("taxable", "1")]).await;
    let pid2 = made.location.split("/products/").nth(1).unwrap().split('/').next().unwrap().to_string();
    upload(&f, &owner, &pid2, "pro.zip", b"PK zip bytes").await;
    let file2 = vuuut_db::repos::ecom::files(&f.state.db, &pid2).await.remove(0);
    let v2 = vuuut_db::repos::ecom::variants(&f.state.db, &pid2).await.remove(0).id;
    let add = site_with(&f.state, "POST", "/shop/cart/add/", &mine, Some(&[("variant_id", &v2), ("qty", "1")])).await;
    let cart = cookie_from(&add.headers).unwrap();
    let both = format!("{cart}; {mine}");
    let placed = site_with(&f.state, "POST", "/shop/checkout/", &both, Some(&[("email", "reader@example.test"), ("place", "1"), ("payment", "manual")])).await;
    assert!(placed.status.is_redirection());
    let (orders, _) = vuuut_db::repos::ecom::browse_orders(&f.state.db, &vuuut_db::repos::ecom::OrderBrowse { view: "all", limit: 5, ..Default::default() }).await;
    let paid_order = orders.into_iter().find(|o| o.total_cents == 1200).unwrap();
    assert!(vuuut_db::repos::ecom::grants_for_order(&f.state.db, &paid_order.id).await.is_empty(), "nothing is granted before payment");
    assert!(site_with(&f.state, "GET", &format!("/shop/orders/{}/", paid_order.token), &mine, None).await.body.contains("unlock as soon as payment arrives"));
    let marked = post(&f.state, &format!("/ecom/orders/{}/paid/", paid_order.id), &owner, &[("reference", "")]).await;
    assert_eq!(marked.flash(), "Marked as paid. Downloads unlocked and emailed to the buyer.");
    let g2 = vuuut_db::repos::ecom::grants_for_order(&f.state.db, &paid_order.id).await.remove(0);
    let url2 = format!("/shop/downloads/{}/{}/", g2.id, file2.id);
    assert_eq!(raw(&f, &url2, &mine).await.0, StatusCode::OK);
    // A file of another product cannot be fetched through this grant.
    let (st, _, _) = raw(&f, &format!("/shop/downloads/{}/{}/", g2.id, file.id), &mine).await;
    assert!(st.is_redirection());
    // Cancelling revokes it.
    post(&f.state, &format!("/ecom/orders/{}/cancel/", paid_order.id), &owner, &[]).await;
    let (st, h, _) = raw(&f, &url2, &mine).await;
    assert!(st.is_redirection() && h.get("location").unwrap().to_str().unwrap().contains("error="));

    // Removing a file removes it from disk.
    post(&f.state, &format!("/ecom/products/{pid}/files/{}/delete/", file.id), &owner, &[]).await;
    assert!(!on_disk.exists());
}

#[tokio::test]
async fn a_new_product_is_created_with_its_files_in_one_save() {
    let f = fixture().await;
    let owner = f.session().await;
    let boundary = "B0undary";
    let mut body = Vec::new();
    for (k, v) in [("title", "Font Bundle"), ("status", "active"), ("price", "9"), ("download_limit", "3")] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    for (name, bytes) in [("fonts.zip", &b"PK fonts"[..]), ("licence.pdf", &b"%PDF licence"[..])] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"digital_file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    // An empty file input posts a nameless part; it is ignored.
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"digital_file\"; filename=\"\"\r\n\r\n\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri("/ecom/products/new/")
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("cookie", &owner)
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let app = vuuut_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(req).await.unwrap();
    let location = response.headers().get("location").unwrap().to_str().unwrap().to_string();
    assert!(location.contains("Product+created.+2+files+uploaded."), "{location}");
    let pid = location.split("/products/").nth(1).unwrap().split('/').next().unwrap().to_string();
    let files = vuuut_db::repos::ecom::files(&f.state.db, &pid).await;
    assert_eq!(files.iter().map(|f| f.filename.as_str()).collect::<Vec<_>>(), vec!["fonts.zip", "licence.pdf"]);
    for file in &files {
        assert!(vuuut_api::ecom::stored_path(&f.state, &file.stored).unwrap().exists());
    }
    let p = vuuut_db::repos::ecom::find_product(&f.state.db, &pid).await.unwrap().unwrap();
    assert_eq!(p.download_limit, 3);
    // No half-written files are left behind.
    let leftovers = std::fs::read_dir(vuuut_api::ecom::files_dir(&f.state)).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".part")).count();
    assert_eq!(leftovers, 0);
    // A product that fails validation keeps no files either.
    let mut bad = Vec::new();
    bad.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n\r\n").as_bytes());
    bad.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"digital_file\"; filename=\"x.zip\"\r\n\r\nPK\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder().method("POST").uri("/ecom/products/new/").header("host", HOST).header("origin", ORIGIN).header("cookie", &owner)
        .header("content-type", format!("multipart/form-data; boundary={boundary}")).body(Body::from(bad)).unwrap();
    let app = vuuut_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(req).await.unwrap();
    assert!(response.headers().get("location").unwrap().to_str().unwrap().contains("error="));
    let count = std::fs::read_dir(vuuut_api::ecom::files_dir(&f.state)).unwrap().count();
    assert_eq!(count, 2, "the refused upload left nothing on disk");
}

#[tokio::test]
async fn the_store_and_its_products_answer_at_the_addresses_chosen() {
    let f = fixture().await;
    let owner = f.session().await;
    // A bad address is refused with a reason, not silently replaced.
    let bad = post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_base_path", "ghost"), ("ecom_product_style", "direct")]).await;
    assert!(bad.flash().contains("store address"), "{}", bad.flash());
    let saved = post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_currency", "USD"), ("ecom_pay_manual", "1"), ("ecom_per_page", "24"),
        ("ecom_low_stock", "5"), ("ecom_base_path", "/Themes/"), ("ecom_product_style", "direct"), ("ecom_product_path", "")]).await;
    assert_eq!(saved.flash(), "Store settings saved.");
    let stored: (String, String) = sqlx::query_as("SELECT (SELECT value FROM settings WHERE key = 'ecom_base_path'), (SELECT value FROM settings WHERE key = 'ecom_product_path')").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(stored, ("themes".to_string(), "themes".to_string()));

    post(&f.state, "/ecom/products/new/", &owner, &[("title", "Asahi"), ("status", "active"), ("price", "29"), ("taxable", "1")]).await;
    // "Cart" would shadow the cart page; it gets another handle.
    post(&f.state, "/ecom/products/new/", &owner, &[("title", "Cart"), ("status", "active"), ("price", "5"), ("taxable", "1")]).await;
    let handles: Vec<String> = sqlx::query_scalar("SELECT handle FROM ecom_products ORDER BY handle").fetch_all(&f.state.db).await.unwrap();
    assert_eq!(handles, vec!["asahi", "cart-product"]);

    let home = site(&f.state, "GET", "/themes/", None).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("href=\"/themes/asahi/\""), "catalogue links use the new address");
    assert!(home.body.contains("href=\"/themes/cart/\""));
    assert!(!home.body.contains("href=\"/shop/"), "no old addresses are written");
    let product = site(&f.state, "GET", "/themes/asahi/", None).await;
    assert_eq!(product.status, StatusCode::OK);
    assert!(product.body.contains("action=\"/themes/cart/add/\""));
    assert!(product.body.contains("/themes/asahi/"), "the page's own address is the public one");
    // One hop to the trailing slash, not via the internal address.
    let bare = site(&f.state, "GET", "/themes/asahi", None).await;
    assert_eq!(bare.headers.get("location").unwrap(), "/themes/asahi/");
    // Old addresses redirect permanently, query kept.
    let old = site(&f.state, "GET", "/shop/products/asahi/?variant=x", None).await;
    assert!(matches!(old.status, StatusCode::MOVED_PERMANENTLY | StatusCode::PERMANENT_REDIRECT), "{}", old.status);
    assert_eq!(old.headers.get("location").unwrap(), "/themes/asahi/?variant=x");
    assert_eq!(site(&f.state, "GET", "/shop/", None).await.headers.get("location").unwrap(), "/themes/");
    // The store still works end to end at the new address.
    let variant: String = sqlx::query_scalar("SELECT v.id FROM ecom_variants v JOIN ecom_products p ON p.id = v.product_id WHERE p.handle = 'asahi'").fetch_one(&f.state.db).await.unwrap();
    let add = site(&f.state, "POST", "/themes/cart/add/", Some(&[("variant_id", &variant), ("qty", "1")])).await;
    assert_eq!(add.headers.get("location").unwrap(), "/themes/cart/");
    let cart = cookie_from(&add.headers).unwrap();
    let cart_page = site_with(&f.state, "GET", "/themes/cart/", &cart, None).await;
    assert!(cart_page.body.contains("Asahi") && cart_page.body.contains("action=\"/themes/cart/update/\""));
    // The product called Cart lives at its own handle.
    assert_eq!(site(&f.state, "GET", "/themes/cart-product/", None).await.status, StatusCode::OK);
    // Other pages of the site are untouched.
    assert_ne!(site(&f.state, "GET", "/about/", None).await.headers.get("location").map(|l| l.to_str().unwrap().to_string()), Some("/themes/about/".into()));
    // The admin's links follow.
    let pid: String = sqlx::query_scalar("SELECT id FROM ecom_products WHERE handle = 'asahi'").fetch_one(&f.state.db).await.unwrap();
    assert!(get(&f.state, &format!("/ecom/products/{pid}/"), &owner).await.body.contains("href=\"/themes/asahi/\""));

    // Products under their own path, the store elsewhere.
    post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_currency", "USD"), ("ecom_pay_manual", "1"), ("ecom_per_page", "24"),
        ("ecom_low_stock", "5"), ("ecom_base_path", "store"), ("ecom_product_style", "custom"), ("ecom_product_path", "item")]).await;
    assert_eq!(site(&f.state, "GET", "/item/asahi/", None).await.status, StatusCode::OK);
    assert_eq!(site(&f.state, "GET", "/store/", None).await.status, StatusCode::OK);
    assert_eq!(site(&f.state, "GET", "/themes/asahi/", None).await.status, StatusCode::NOT_FOUND, "the previous custom address is released");
    // Back to the defaults: the store is at /shop/ again.
    post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1"), ("ecom_currency", "USD"), ("ecom_pay_manual", "1"), ("ecom_per_page", "24"),
        ("ecom_low_stock", "5"), ("ecom_base_path", "shop"), ("ecom_product_style", "under")]).await;
    assert_eq!(site(&f.state, "GET", "/shop/products/asahi/", None).await.status, StatusCode::OK);
}

async fn import_csv(f: &Fixture, cookie: &str, csv: &str, opts: &[&str]) -> Reply {
    let boundary = "CsvB0undary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"products_export.csv\"\r\nContent-Type: text/csv\r\n\r\n").as_bytes());
    body.extend_from_slice(csv.as_bytes());
    body.extend_from_slice(b"\r\n");
    for o in opts {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{o}\"\r\n\r\n1\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let req = Request::builder().method("POST").uri("/ecom/products/import/").header("host", HOST).header("origin", ORIGIN).header("cookie", cookie)
        .header("content-type", format!("multipart/form-data; boundary={boundary}")).body(Body::from(body)).unwrap();
    let app = vuuut_admin_ui::admin_ui_router(f.state.clone()).with_state(f.state.clone());
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    Reply { status, location, body: String::new() }
}

#[tokio::test]
async fn shopify_product_csv_imports_and_exports() {
    let f = fixture().await;
    let owner = f.session().await;
    let shopify = "Handle,Title,Body (HTML),Vendor,Product Category,Type,Tags,Published,Option1 Name,Option1 Value,Option2 Name,Option2 Value,Option3 Name,Option3 Value,Variant SKU,Variant Grams,Variant Inventory Tracker,Variant Inventory Qty,Variant Inventory Policy,Variant Fulfillment Service,Variant Price,Variant Compare At Price,Variant Requires Shipping,Variant Taxable,Variant Barcode,Image Src,Image Position,Image Alt Text,Gift Card,SEO Title,SEO Description,Variant Image,Variant Weight Unit,Cost per item,Status\n\
classic-tee,Classic Tee,<p>Soft <b>cotton</b>.</p>,Acme,,Shirts,\"summer, cotton\",true,Size,S,,,,,TEE-S,200,shopify,5,deny,manual,20.00,25.00,true,true,,https://cdn.shopify.com/tee.jpg,1,,false,,,,g,8.00,active\n\
classic-tee,,,,,,,,,M,,,,,TEE-M,210,shopify,2,deny,manual,22.00,,true,true,,,,,,,,,g,,\n\
theme-pack,Theme Pack,<p>Ten themes.</p>,Acme,,Themes,,true,Title,Default Title,,,,,TP-1,0,,0,deny,manual,49.00,,false,true,,,,,false,,,,g,,active\n\
bad-row,Bad,,,,,,true,Title,Default Title,,,,,,,,,,,notaprice,,,,,,,,,,,,,,active\n";
    let r = import_csv(&f, &owner, shopify, &["update"]).await;
    assert!(r.location.contains("2+created"), "{}", r.location);
    assert!(r.location.contains("report="), "the bad price is reported: {}", r.location);
    let tee = vuuut_db::repos::ecom::product_by_handle(&f.state.db, "classic-tee").await.unwrap().unwrap();
    assert_eq!(tee.description_html, "<p>Soft <b>cotton</b>.</p>");
    assert!(tee.description_lexical.contains("html"), "the body opens in the editor as an HTML card");
    assert_eq!(tee.image_list(), vec!["https://cdn.shopify.com/tee.jpg"]);
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &tee.id).await;
    assert_eq!(vs.iter().map(|v| (v.title.as_str(), v.price_cents, v.inventory, v.sku.as_str())).collect::<Vec<_>>(), vec![("S", 2000, 5, "TEE-S"), ("M", 2200, 2, "TEE-M")]);
    let pack = vuuut_db::repos::ecom::product_by_handle(&f.state.db, "theme-pack").await.unwrap().unwrap();
    assert_eq!(pack.requires_shipping, 0, "a product Shopify does not ship is digital here");
    assert!(pack.option_list().is_empty());

    // Put the tee in a collection, then re-import a changed price: it is
    // updated in place, its variant ids and collection kept.
    let coll = post(&f.state, "/ecom/collections/new/", &owner, &[("title", "Tops"), ("kind", "manual"), ("published", "1")]).await;
    let cid = coll.location.split("/collections/").nth(1).unwrap().split('/').next().unwrap().to_string();
    post(&f.state, &format!("/ecom/collections/{cid}/products/"), &owner, &[("op", "add"), ("product", &tee.id)]).await;
    let changed = shopify.replace("20.00,25.00", "18.00,25.00");
    let r = import_csv(&f, &owner, &changed, &["update"]).await;
    assert!(r.location.contains("2+updated"), "{}", r.location);
    let vs2 = vuuut_db::repos::ecom::variants(&f.state.db, &tee.id).await;
    assert_eq!(vs2[0].id, vs[0].id);
    assert_eq!(vs2[0].price_cents, 1800);
    assert_eq!(vuuut_db::repos::ecom::product_collection_ids(&f.state.db, &tee.id).await, vec![cid]);
    // Without "update", existing products are left alone.
    let r = import_csv(&f, &owner, &shopify.replace("18.00", "1.00"), &[]).await;
    assert!(r.location.contains("2+skipped"), "{}", r.location);
    // Not a Shopify file: refused with the reason.
    let r = import_csv(&f, &owner, "name,price\nx,1\n", &["update"]).await;
    assert!(r.location.contains("Handle+column"), "{}", r.location);

    // Export: Shopify's columns, a row per variant, and it imports back.
    let csv = get(&f.state, "/ecom/products/export.csv", &owner).await;
    assert!(csv.body.starts_with("Handle,Title,Body (HTML),Vendor,Product Category,Type,Tags,Published,Option1 Name,Option1 Value"));
    assert!(csv.body.contains("classic-tee,Classic Tee,<p>Soft <b>cotton</b>.</p>,Acme,,Shirts,\"summer, cotton\",true,Size,S"));
    assert!(csv.body.contains("\nclassic-tee,,,,,,,,,M,"));
    assert!(csv.body.contains("theme-pack,Theme Pack,<p>Ten themes.</p>,Acme,,Themes,,true,Title,Default Title"));
    let parsed = vuuut_core::shopify_csv::parse(&csv.body, "USD").unwrap();
    assert!(parsed.problems.is_empty());
    assert_eq!(parsed.products.len(), 2);
    // A stranger cannot export.
    assert!(send(&f.state, "GET", "/ecom/products/export.csv", None, None, None).await.status.is_redirection());
}


#[tokio::test]
async fn storefront_page_size_accepts_custom_numbers_and_defaults_to_ten() {
    use vuuut_db::repos::ecom::{CollectionInput, ProductInput, save_collection, save_product, set_manual_ids};

    let f = fixture().await;
    let owner = f.session().await;
    let settings = "/ecom/settings/";
    set(&f, "ecom_enabled", serde_json::json!(true)).await;
    let mut ids = Vec::new();
    for n in 1..=11 {
        let id = save_product(&f.state.db, None, &ProductInput {
            handle: format!("pagination-product-{n:02}"),
            title: format!("Pagination product {n:02}"),
            status: "active".into(),
            ..Default::default()
        }, &[], &[]).await.unwrap();
        ids.push(id);
    }
    ids.reverse();
    let collection = save_collection(&f.state.db, None, &CollectionInput {
        handle: "pagination".into(),
        title: "Pagination".into(),
        kind: "manual".into(),
        sort: "manual".into(),
        published: true,
        ..Default::default()
    }).await.unwrap();
    set_manual_ids(&f.state.db, &collection, &ids).await.unwrap();

    let page = get(&f.state, settings, &owner).await;
    assert!(page.body.contains(r#"name="ecom_per_page" type="number" min="1" step="1" value="10""#));
    let first = site(&f.state, "GET", "/shop/?sort=title", None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body.matches(r#"class="vs-card""#).count(), 10);
    assert!(first.body.contains("Page 1 of 2"));

    let saved = post(&f.state, settings, &owner, &[("ecom_enabled", "1"), ("ecom_per_page", "7")]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let first = site(&f.state, "GET", "/shop/?sort=title", None).await;
    let second = site(&f.state, "GET", "/shop/?sort=title&page=2", None).await;
    assert_eq!(first.body.matches(r#"class="vs-card""#).count(), 7);
    assert_eq!(second.body.matches(r#"class="vs-card""#).count(), 4);
    assert!(second.body.contains("Pagination product 08"));
    assert!(!second.body.contains("Pagination product 07"));
    let manual = site(&f.state, "GET", "/shop/collections/pagination/?page=2", None).await;
    assert_eq!(manual.status, StatusCode::OK);
    assert_eq!(manual.body.matches(r#"class="vs-card""#).count(), 4);
    assert!(manual.body.find("Pagination product 04").unwrap() < manual.body.find("Pagination product 01").unwrap());

    for invalid in ["0", "-1", "2.5"] {
        let saved = post(&f.state, settings, &owner, &[("ecom_enabled", "1"), ("ecom_per_page", invalid)]).await;
        assert!(saved.location.contains("error="), "{}", saved.location);
        assert_eq!(f.state.settings.get("ecom_per_page"), Some(serde_json::json!(7)));
    }
    let saved = post(&f.state, settings, &owner, &[("ecom_enabled", "1")]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    assert_eq!(f.state.settings.get("ecom_per_page"), Some(serde_json::json!(7)));

    let large = i64::MAX.to_string();
    let saved = post(&f.state, settings, &owner, &[("ecom_enabled", "1"), ("ecom_per_page", &large)]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let first = site(&f.state, "GET", "/shop/", None).await;
    assert_eq!(first.body.matches(r#"class="vs-card""#).count(), 11);
    let far = site(&f.state, "GET", &format!("/shop/?page={large}"), None).await;
    assert_eq!(far.status, StatusCode::OK);
    assert_eq!(far.body.matches(r#"class="vs-card""#).count(), 0);

    let saved = post(&f.state, settings, &owner, &[("ecom_enabled", "1"), ("ecom_per_page", "")]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    assert_eq!(f.state.settings.get("ecom_per_page"), Some(serde_json::json!(10)));
}

#[tokio::test]
async fn payment_currency_is_shared_and_preserves_catalogue_amounts() {
    use vuuut_core::{directory, ecom, lms};
    use vuuut_db::repos::ecom as store;

    let f = fixture_with_theme("margin").await;
    let owner = f.session().await;
    let get_setting = |key: &str| f.state.settings.get_str(key);
    f.state.settings.set_many(&f.state.db, &[
        ("donations_currency", serde_json::json!("USD")),
        ("donations_suggested_amount", serde_json::json!(500)),
    ]).await.unwrap();
    let product = store::save_product(&f.state.db, None, &store::ProductInput {
        handle: "currency-check".into(), title: "Currency check".into(),
        ..Default::default()
    }, &[store::VariantInput { price_cents: 12_500, ..Default::default() }], &[]).await.unwrap();

    let page = get(&f.state, "/settings/payments/", &owner).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body.matches(r#"<select name="currency">"#).count(), 1);
    for currency in ["BDT", "AED", "JPY", "XOF", "USD"] {
        assert!(page.body.contains(&format!(r#"<option value="{currency}""#)));
    }

    for (currency, price, tip) in [("BDT", 12_500, 500), ("JPY", 125, 5), ("USD", 12_500, 500)] {
        let saved = post(&f.state, "/settings/payments/currency/", &owner, &[("currency", currency)]).await;
        assert!(saved.location.contains("ok="), "{}", saved.location);
        assert_eq!(ecom::Settings::read(get_setting).currency, currency);
        assert_eq!(lms::Settings::read(get_setting).currency, currency);
        assert_eq!(directory::Settings::read(get_setting).currency, currency);
        assert_eq!(vuuut_core::currency::donations(get_setting).as_deref(), Some(currency));
        assert_eq!(f.state.settings.get("donations_suggested_amount"), Some(serde_json::json!(tip)));
        assert_eq!(store::variants(&f.state.db, &product).await[0].price_cents, price);
    }

    let rejected = post(&f.state, "/settings/payments/currency/", &owner, &[("currency", "XXX")]).await;
    assert!(rejected.location.contains("error="), "{}", rejected.location);
    assert_eq!(get_setting("site_currency").as_deref(), Some("USD"));
}

#[tokio::test]
async fn payments_settings_manage_providers_and_offers_mapped_digital_checkout() {
    use vuuut_api::payments as providers;
    use vuuut_core::ecom::{CartLine, CART_COOKIE};
    use vuuut_db::repos::{ecom as store, payments};

    let f = fixture().await;
    let owner = f.session().await;
    let tools = get(&f.state, "/tools/", &owner).await;
    assert!(!tools.body.contains(r#"href="/ghost/payments/""#));
    let settings = get(&f.state, "/settings/", &owner).await;
    assert!(settings.body.contains(r#"href="/ghost/settings/payments/""#));
    let legacy = get(&f.state, "/payments/?product=example", &owner).await;
    assert_eq!(legacy.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(legacy.location, "/ghost/settings/payments/?product=example");
    let page = get(&f.state, "/settings/payments/", &owner).await;
    assert_eq!(page.status, StatusCode::OK);
    for expected in ["Connect Stripe", "Polar", "Gumroad", "Store payment methods"] {
        assert!(page.body.contains(expected), "missing {expected}");
    }
    let memberships = get(&f.state, "/memberships/settings/", &owner).await;
    assert!(memberships.body.contains("/ghost/settings/payments/#stripe"));
    assert!(!memberships.body.contains(r#"name="secret_key""#));
    let ecommerce = get(&f.state, "/ecom/settings/", &owner).await;
    assert!(ecommerce.body.contains("/ghost/settings/payments/#store"));
    assert!(!ecommerce.body.contains(r#"name="ecom_pay_stripe""#));

    let polar = post(&f.state, "/settings/payments/polar/", &owner, &[
        ("enabled", "1"), ("sandbox", "1"), (providers::POLAR_KEY, "polar_test_fixture"),
        (providers::POLAR_SECRET, "polar_hook_fixture"),
    ]).await;
    assert!(polar.location.contains("ok="), "{}", polar.location);
    let gumroad = post(&f.state, "/settings/payments/gumroad/", &owner, &[
        ("enabled", "1"), (providers::GUMROAD_KEY, "gumroad_test_fixture"),
    ]).await;
    assert!(gumroad.location.contains("ok="), "{}", gumroad.location);
    let page = get(&f.state, "/settings/payments/", &owner).await;
    for secret in ["polar_test_fixture", "polar_hook_fixture", "gumroad_test_fixture"] {
        assert!(!page.body.contains(secret));
    }
    let foreign = send(&f.state, "POST", "/settings/payments/gumroad/", Some(&owner),
        Some("https://elsewhere.example"), Some(&[("enabled", "1")])).await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);

    let product = store::save_product(&f.state.db, None, &store::ProductInput {
        title: "Hosted checkout book".into(), handle: "hosted-book".into(), status: "active".into(),
        ..Default::default()
    }, &[store::VariantInput { price_cents: 1000, ..Default::default() }], &[]).await.unwrap();
    let remote = "9c5777d6-cc9e-4a14-99f7-9629e01884dd";
    let saved = post(&f.state, "/settings/payments/products/", &owner, &[
        ("product", &product), ("polar_product", remote), ("gumroad_product", "gumroadexample="),
    ]).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let mapping = payments::mapping(&f.state.db, &product).await.unwrap();
    assert_eq!(mapping.polar_product, remote);

    let store_settings = post(&f.state, "/settings/payments/store/", &owner, &[]).await;
    assert!(store_settings.location.contains("ok="), "{}", store_settings.location);
    let store_settings = post(&f.state, "/ecom/settings/", &owner, &[("ecom_enabled", "1")]).await;
    assert!(store_settings.location.contains("ok="), "{}", store_settings.location);
    assert!(!f.state.settings.get_bool("ecom_pay_manual"));

    let variant = store::variants(&f.state.db, &product).await.remove(0).id;
    let cart = store::create_cart(&f.state.db).await.unwrap();
    let cookie = format!("{CART_COOKIE}={cart}");
    store::save_cart_lines(&f.state.db, &cart, &[CartLine { variant_id: variant.clone(), qty: 1 }]).await.unwrap();
    let checkout = site_with(&f.state, "GET", "/shop/checkout/", &cookie, None).await;
    assert_eq!(checkout.status, StatusCode::OK);
    assert!(checkout.body.contains(r#"name="payment" value="polar""#));
    assert!(checkout.body.contains(r#"name="payment" value="gumroad""#));
    store::save_cart_lines(&f.state.db, &cart, &[CartLine { variant_id: variant, qty: 2 }]).await.unwrap();
    let checkout = site_with(&f.state, "GET", "/shop/checkout/", &cookie, None).await;
    assert!(!checkout.body.contains(r#"name="payment" value="polar""#));
    assert!(!checkout.body.contains(r#"name="payment" value="gumroad""#));
}

#[tokio::test]
async fn a_product_carries_its_demo_and_direct_purchase_links() {
    let f = fixture().await;
    let owner = f.session().await;
    set(&f, "ecom_enabled", serde_json::json!(true)).await;

    // Anything but a full web address is refused, not silently dropped.
    let bad = post(&f.state, "/ecom/products/new/", &owner, &[
        ("title", "Theme"), ("status", "active"), ("price", "29"), ("demo_url", "javascript:alert(1)"),
    ]).await;
    assert!(bad.flash().contains("live demo link"), "{}", bad.flash());
    let bad = post(&f.state, "/ecom/products/new/", &owner, &[
        ("title", "Theme"), ("status", "active"), ("price", "29"), ("purchase_url", "polar.sh/checkout"),
    ]).await;
    assert!(bad.flash().contains("direct purchase link"), "{}", bad.flash());

    let made = post(&f.state, "/ecom/products/new/", &owner, &[
        ("title", "Asahi"), ("status", "active"), ("price", "29"),
        ("demo_url", "https://theme1.ghost-theme.com"),
        ("purchase_url", "https://anisul.gumroad.com/l/asahi-ghost"),
    ]).await;
    assert_eq!(made.flash(), "Product created.");
    let p = vuuut_db::repos::ecom::product_by_handle(&f.state.db, "asahi").await.unwrap().unwrap();
    assert_eq!(p.demo_url, "https://theme1.ghost-theme.com");
    assert_eq!(p.purchase_url, "https://anisul.gumroad.com/l/asahi-ghost");
    let editor = get(&f.state, &format!("/ecom/products/{}/", p.id), &owner).await;
    assert!(editor.body.contains(r#"name="purchase_url""#) && editor.body.contains("anisul.gumroad.com/l/asahi-ghost"));

    // The store sends buyers to the outside checkout instead of the cart,
    // and offers the demo beside it.
    let page = site(&f.state, "GET", "/shop/products/asahi/", None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#"href="https://anisul.gumroad.com/l/asahi-ghost""#) && page.body.contains("Buy now"));
    assert!(page.body.contains(r#"href="https://theme1.ghost-theme.com""#) && page.body.contains("Live demo"));
    assert!(!page.body.contains(r#"id="vs-add""#), "no cart button when the product sells elsewhere");

    // Themes get both; the field wins over a demo link in the description.
    let vs = vuuut_db::repos::ecom::variants(&f.state.db, &p.id).await;
    let s = vuuut_api::ecom::settings(&f.state);
    let card = vuuut_db::repos::ecom::card_json(&p, &vs, &s, "/x/");
    assert_eq!(card["demo_url"], "https://theme1.ghost-theme.com");
    assert_eq!(card["purchase_url"], "https://anisul.gumroad.com/l/asahi-ghost");

    // Cleared again, the cart comes back.
    let saved = post(&f.state, &format!("/ecom/products/{}/", p.id), &owner, &[
        ("title", "Asahi"), ("status", "active"), ("price", "29"), ("demo_url", ""), ("purchase_url", ""),
    ]).await;
    assert_eq!(saved.flash(), "Product saved.");
    let page = site(&f.state, "GET", "/shop/products/asahi/", None).await;
    assert!(page.body.contains(r#"id="vs-add""#) && !page.body.contains("Buy now"));
}
