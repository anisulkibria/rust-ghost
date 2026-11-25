//! The online store: `/shop/`.
//!
//! Every screen renders inside the active theme as a transient page
//! (`routes::render_virtual_page_with`), so the store wears the site's own
//! header, footer and type with no theme changes. While the store is off in
//! Tools → E-commerce & Online Stores, every address here is the theme's 404.
//!
//! In the order a buyer meets them:
//!
//! * `GET /shop/` — the catalogue: search, sort, collections, pages.
//! * `GET /shop/collections/` and `/shop/collections/{handle}/`.
//! * `GET /shop/products/{handle}/` — gallery, option picker, stock, add to
//!   cart, and schema.org `Product` data.
//! * `POST /shop/cart/add/`, `GET /shop/cart/`, `POST /shop/cart/update/` —
//!   the cart, held server-side under a random token in the `vuuut-cart`
//!   cookie. The cookie holds nothing else.
//! * `GET|POST /shop/checkout/` — contact, address, shipping, payment.
//!   Every price is recomputed from the database on the POST; nothing the
//!   browser sends about money is believed.
//! * `GET /shop/checkout/paid/` — the return from Stripe. The session is
//!   fetched from Stripe and must name this order and be paid before the
//!   order is marked paid.
//! * `GET /shop/orders/{token}/` — the order's status page, reachable only
//!   by its unguessable token.
//! * `GET /shop/dashboard/` — "My products": a signed-in member's orders
//!   (every purchase, shipped or digital) and a Downloads section; signed
//!   out, a form that emails a sign-in link. `/shop/downloads/` and
//!   `/shop/account/` lead here. `GET /shop/downloads/{grant}/
//!   {file}/` streams one file, and only to the signed-in member whose email
//!   the grant names, while the order is paid and the grant within its limit
//!   and date. Files live outside every public route (`content/private/`).
//!
//! Writes are same-origin only; checkout has a per-address budget.

use crate::routes::{render_virtual_page_as, render_virtual_page_with};
use serde_json::{json, Value};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use vuuut_api::AppState;
use vuuut_core::ecom::{self as rules, Address, CartLine, PaymentMethod, Settings};
use vuuut_db::repos::ecom as repo;
use std::collections::HashMap;

const BASE: &str = "/shop/";
static CSS: &str = include_str!("../assets/shop.css");
static JS: &str = include_str!("../assets/shop.js");

fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}

fn assets_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash(&format!("{CSS}{JS}")));
    &V
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/shop/", get(index))
        .route("/shop", get(|| async { Redirect::permanent("/shop/") }))
        .route("/shop/assets/{v}/shop.css", get(|| async { asset("text/css; charset=utf-8", CSS) }))
        .route("/shop/assets/{v}/shop.js", get(|| async { asset("text/javascript; charset=utf-8", JS) }))
        .route("/shop/collections/", get(collections_index))
        .route("/shop/collections/{handle}/", get(collection))
        .route("/shop/products/{handle}/", get(product))
        .route("/shop/cart/", get(cart_page))
        .route("/shop/cart/add/", post(cart_add))
        .route("/shop/cart/update/", post(cart_update))
        .route("/shop/checkout/", get(checkout_page).post(checkout_submit))
        .route("/shop/checkout/paid/", get(stripe_return))
        .route("/shop/orders/{token}/", get(order_page))
        .route("/shop/orders/{token}/pay/", post(order_pay))
        .route("/shop/dashboard/", get(dashboard))
        .route("/shop/account/", get(to_dashboard))
        .route("/shop/downloads/", get(to_dashboard))
        .route("/shop/downloads/signin/", post(downloads_signin))
        .route("/shop/downloads/{grant}/{file}/", get(download_file))
        .route("/shop/orders/{token}/send-link/", post(order_send_link))
        .route("/shop/policies/{which}/", get(policy))
}

fn asset(ct: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], body).into_response()
}

fn settings(state: &AppState) -> Settings {
    vuuut_api::ecom::settings(state)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// A store link as visitors see it: the internal `/shop/…` route mapped to
/// the store's configured address, under the install's base path.
fn link(path: &str) -> String {
    vuuut_core::url::base(&public(path))
}

tokio::task_local! {
    /// The store's addresses for the request being answered, set by
    /// [`addresses`] around the whole of it.
    static PATHS: rules::Paths;
}

/// An internal `/shop/…` path as visitors see it, for this request.
fn public(path: &str) -> String {
    PATHS.try_with(|p| p.public_path(path)).unwrap_or_else(|_| path.to_string())
}

fn money(c: i64, s: &Settings) -> String {
    esc(&rules::format_money(c, &s.currency))
}

async fn not_found(state: &AppState, path: &str, headers: HeaderMap) -> Response {
    crate::render::error_page(state, StatusCode::NOT_FOUND, path, headers).await
}

fn head_assets() -> String {
    let v = assets_version();
    format!(
        r#"<link rel="stylesheet" href="{css}"><script src="{js}" defer></script>"#,
        css = link(&format!("/shop/assets/{v}/shop.css")),
        js = link(&format!("/shop/assets/{v}/shop.js")),
    )
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_string())
}

async fn current_cart(state: &AppState, headers: &HeaderMap) -> Option<repo::Cart> {
    let token = cookie(headers, rules::CART_COOKIE)?;
    repo::find_cart(&state.db, &token).await
}

fn cart_cookie(state: &AppState, token: &str) -> HeaderValue {
    let secure = if state.site_url().starts_with("https://") { "; Secure" } else { "" };
    let path = { let p = vuuut_core::url::subdir(); if p.is_empty() { "/".to_string() } else { format!("{p}/") } };
    HeaderValue::from_str(&format!("{}={token}; Path={path}; Max-Age=5184000; HttpOnly; SameSite=Lax{secure}", rules::CART_COOKIE)).unwrap_or(HeaderValue::from_static(""))
}

fn same_origin(state: &AppState, headers: &HeaderMap) -> bool {
    vuuut_api::forms::same_origin(headers, &state.site_url())
}

/// A page of the store inside the theme. The store draws its own heading,
/// so the theme's title and feature image are off; `image` still becomes
/// the page's social image.
async fn page(state: &AppState, headers: &HeaderMap, path: &str, title: &str, excerpt: &str, body: String, image: Option<String>) -> Response {
    let s = settings(state);
    let count: i64 = current_cart(state, headers).await.map(|c| c.lines().iter().map(|l| l.qty).sum()).unwrap_or(0);
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let html = format!(
        r#"{assets}<div class="vs">
<nav class="vs-bar" aria-label="Store">
<a class="vs-brand" href="{home}">{store}</a>
<a href="{colls}">Collections</a>
<a href="{dls}">My products</a>
<form class="vs-find" method="get" action="{home}" role="search"><input type="search" name="q" placeholder="Search products" aria-label="Search products" maxlength="100"></form>
<a class="vs-cart-link" href="{cart}" aria-label="Cart, {count} items"><svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor" aria-hidden="true"><path d="M7 18c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zM1 2v2h2l3.6 7.59-1.35 2.45c-.16.28-.25.61-.25.96 0 1.1.9 2 2 2h12v-2H7.42c-.14 0-.25-.11-.25-.25l.03-.12.9-1.63h7.45c.75 0 1.41-.41 1.75-1.03l3.58-6.49A1 1 0 0 0 20 4H5.21l-.94-2H1zm16 16c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"/></svg><span class="vs-cart-word">Cart</span>{badge}</a>
</nav>
{body}
</div>"#,
        assets = head_assets(),
        home = link(BASE),
        store = esc(s.name(&site_title)),
        colls = link("/shop/collections/"),
        dls = link("/shop/dashboard/"),
        cart = link("/shop/cart/"),
        count = count,
        badge = if count > 0 { format!(r#"<span class="vs-badge">{count}</span>"#) } else { String::new() },
    );
    // The page's own URL (canonical, sharing) is its public address.
    let public_url = public(path);
    let res = render_virtual_page_with(state, headers, &public_url, title, excerpt, html, image, false).await;
    keep_private(res, count, path)
}

/// A page that shows a cart is somebody's; it must not be shared.
fn keep_private(mut res: Response, count: i64, path: &str) -> Response {
    if count > 0 || path.starts_with("/shop/cart") || path.starts_with("/shop/checkout") || path.starts_with("/shop/orders") || path.starts_with("/shop/account") || path.starts_with("/shop/downloads") || path.starts_with("/shop/dashboard") {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    }
    res
}

// ---------------------------------------------------------------------------
// theme templates
// ---------------------------------------------------------------------------
//
// A theme can draw the store's catalogue, collections and product pages
// itself with `shop/shop.hbs`, `shop/collection.hbs`, `shop/collections.hbs`
// and `shop/product.hbs`. When it has the template, the store hands it data
// instead of HTML; otherwise the store's own markup renders as before. Cart,
// checkout, orders and the dashboard stay the store's, since they carry
// payment and download state.
//
// Every store template gets `shop` (name, links, cart count); the rest is
// per page and documented on each builder below.

/// Those of `names` the active theme has, in order.
async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

/// `shop` for a page outside the store: the same addresses, no cart count
/// (that needs the visitor's cart, and these pages are cached for
/// everyone). Nothing while the store is off.
pub(crate) fn site_shop(state: &AppState) -> Option<Value> {
    let s = settings(state);
    s.enabled.then(|| shop_json(state, &s, 0))
}

/// `shop`: what every store template can use.
fn shop_json(state: &AppState, s: &Settings, count: i64) -> Value {
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let v = assets_version();
    let mut policies = Vec::new();
    if !s.policy_shipping.is_empty() {
        policies.push(json!({ "title": "Shipping policy", "url": link("/shop/policies/shipping/") }));
    }
    if !s.policy_refund.is_empty() {
        policies.push(json!({ "title": "Refund policy", "url": link("/shop/policies/refund/") }));
    }
    json!({
        "name": s.name(&site_title),
        "intro": s.intro,
        "currency": s.currency,
        "url": link(BASE),
        "collections_url": link("/shop/collections/"),
        "cart_url": link("/shop/cart/"),
        "checkout_url": link("/shop/checkout/"),
        "dashboard_url": link("/shop/dashboard/"),
        "add_url": link("/shop/cart/add/"),
        "cart_count": count,
        "style_url": link(&format!("/shop/assets/{v}/shop.css")),
        "script_url": link(&format!("/shop/assets/{v}/shop.js")),
        "policies": policies,
        "tax_note": if s.tax_rate_bp > 0 { if s.tax_included { "Tax included." } else { "Tax calculated at checkout." } } else { "" },
    })
}

/// A product card's data at its public address.
fn product_card(p: &repo::Product, vs: &[repo::Variant], s: &Settings) -> Value {
    repo::card_json(p, vs, s, &link(&format!("/shop/products/{}/", p.handle)))
}

fn collection_json(c: &repo::Collection) -> Value {
    json!({
        "id": c.id,
        "title": c.title,
        "handle": c.handle,
        "slug": c.handle,
        "description": c.description,
        "feature_image": if c.image.is_empty() { Value::Null } else { json!(c.image) },
        "url": link(&format!("/shop/collections/{}/", c.handle)),
    })
}

/// Render one of the theme's store templates.
#[allow(clippy::too_many_arguments)]
async fn theme_page(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    title: &str,
    excerpt: &str,
    image: Option<String>,
    templates: Vec<String>,
    mut data: serde_json::Map<String, Value>,
    kind: &str,
) -> Response {
    let s = settings(state);
    let count: i64 = current_cart(state, headers).await.map(|c| c.lines().iter().map(|l| l.qty).sum()).unwrap_or(0);
    data.insert("shop".into(), shop_json(state, &s, count));
    let public_url = public(path);
    let res = render_virtual_page_as(state, headers, &public_url, title, excerpt, String::new(), image, false, &templates, data, &["shop", kind]).await;
    keep_private(res, count, path)
}

// ---------------------------------------------------------------------------
// prices
// ---------------------------------------------------------------------------

/// `(lowest, highest, compare-at of the lowest)` across variants.
fn price_range(vs: &[repo::Variant]) -> (i64, i64, i64) {
    let min = vs.iter().min_by_key(|v| v.price_cents);
    let max = vs.iter().map(|v| v.price_cents).max().unwrap_or(0);
    (min.map(|v| v.price_cents).unwrap_or(0), max, min.map(|v| v.compare_at_cents).unwrap_or(0))
}

fn price_html(vs: &[repo::Variant], s: &Settings) -> String {
    let (min, max, compare) = price_range(vs);
    let from = if max > min { "From " } else { "" };
    let was = if compare > min { format!(r#" <s class="vs-was">{}</s>"#, money(compare, s)) } else { String::new() };
    format!(r#"<span class="vs-price">{from}{}</span>{was}"#, money(min, s))
}

fn sold_out(vs: &[repo::Variant]) -> bool {
    !vs.is_empty() && vs.iter().all(|v| !v.available(1))
}

fn card(p: &repo::Product, vs: &[repo::Variant], s: &Settings) -> String {
    let (min, _, compare) = price_range(vs);
    let badge = if sold_out(vs) {
        r#"<span class="vs-tag vs-tag-out">Sold out</span>"#
    } else if compare > min {
        r#"<span class="vs-tag">Sale</span>"#
    } else {
        ""
    };
    let images = p.image_list();
    format!(
        r#"<a class="vs-card" href="{href}"><span class="vs-card-img">{img}{alt}{badge}</span>
<span class="vs-card-body"><span class="vs-card-vendor">{vendor}</span><span class="vs-card-title">{title}</span>{price}</span></a>"#,
        href = link(&format!("/shop/products/{}/", p.handle)),
        img = images.first().map(|i| format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(i))).unwrap_or_else(|| r#"<span class="vs-noimg"></span>"#.to_string()),
        alt = images.get(1).map(|i| format!(r#"<img class="vs-alt" src="{}" alt="" loading="lazy">"#, esc(i))).unwrap_or_default(),
        vendor = esc(&p.vendor),
        title = esc(&p.title),
        price = price_html(vs, s),
    )
}

pub(crate) async fn builder_cards(state: &AppState, ids: &[String]) -> String {
    let s = settings(state);
    if !s.enabled {
        return String::new();
    }
    let products: HashMap<_, _> = repo::products_by_ids(&state.db, ids).await.into_iter()
        .filter(repo::Product::is_active).map(|product| (product.id.clone(), product)).collect();
    if products.is_empty() {
        return String::new();
    }
    let product_ids: Vec<_> = products.keys().cloned().collect();
    let variants = repo::variants_for(&state.db, &product_ids).await;
    PATHS.scope(rules::Paths::from_settings(&s), async {
        let cards: String = ids.iter().filter_map(|id| {
            let product = products.get(id)?;
            Some(card(product, variants.get(id).map(Vec::as_slice).unwrap_or_default(), &s))
        }).collect();
        let css = link(&format!("/shop/assets/{}/shop.css", assets_version()));
        format!(r#"<link rel="stylesheet" href="{css}"><div class="vs"><div class="vb-catalogue">{cards}</div></div>"#)
    }).await
}

// ---------------------------------------------------------------------------
// catalogue
// ---------------------------------------------------------------------------

const PUBLIC_SORTS: &[(&str, &str)] = &[("featured", "Featured"), ("best", "Best selling"), ("title", "A–Z"), ("price_asc", "Price, low to high"), ("price_desc", "Price, high to low"), ("newest", "Newest")];

async fn index(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    listing(state, headers, q, None).await
}

async fn collection(State(state): State<AppState>, headers: HeaderMap, Path(handle): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("/shop/collections/{handle}/");
    if !settings(&state).enabled {
        return not_found(&state, &path, headers).await;
    }
    match repo::collection_by_handle(&state.db, &rules::handleize(&handle)).await.filter(|c| c.published != 0) {
        Some(c) => listing(state, headers, q, Some(c)).await,
        None => not_found(&state, &path, headers).await,
    }
}

async fn listing(state: AppState, headers: HeaderMap, q: HashMap<String, String>, coll: Option<repo::Collection>) -> Response {
    let s = settings(&state);
    let path = coll.as_ref().map(|c| format!("/shop/collections/{}/", c.handle)).unwrap_or_else(|| BASE.to_string());
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let default_sort = coll.as_ref().map(|c| c.sort.as_str()).unwrap_or("featured");
    let sort = q.get("sort").map(String::as_str).filter(|k| PUBLIC_SORTS.iter().any(|(x, _)| x == k)).unwrap_or(default_sort).to_string();
    let page_no: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).max(1);
    let offset = (page_no - 1).saturating_mul(s.per_page);
    let ids = match &coll {
        Some(c) => Some(repo::collection_product_ids(&state.db, c, true).await),
        None => None,
    };
    let (products, total) = if coll.as_ref().is_some_and(|c| !c.is_smart()) && (sort == "manual" || sort == "featured") && terms.is_empty() {
        // The collection's own hand-made order.
        let all = ids.as_deref().unwrap_or_default();
        let total = all.len() as i64;
        let start = offset.min(total) as usize;
        let end = offset.saturating_add(s.per_page).min(total) as usize;
        let mut products = Vec::new();
        for batch in all[start..end].chunks(500) {
            let mut by_id: HashMap<_, _> = repo::products_by_ids(&state.db, batch).await
                .into_iter().map(|product| (product.id.clone(), product)).collect();
            products.extend(batch.iter().filter_map(|id| by_id.remove(id)));
        }
        (products, total)
    } else {
        let sql_sort = match sort.as_str() {
            "featured" | "manual" => "newest",
            other => other,
        };
        repo::browse_storefront(
            &state.db,
            &repo::ProductBrowse { terms: &terms, status: Some("active"), ids: ids.as_deref(), sort: sql_sort, limit: s.per_page, offset, ..Default::default() },
        )
        .await
        .unwrap_or_default()
    };
    // Featured products first on the default catalogue order.
    let mut products = products;
    if sort == "featured" && coll.is_none() {
        products.sort_by_key(|p| if p.featured != 0 { 0 } else { 1 });
    }
    let pids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let vs = repo::variants_for(&state.db, &pids).await;
    let colls: Vec<repo::Collection> = repo::collections(&state.db).await.into_iter().filter(|c| c.published != 0).collect();
    let pages = total / s.per_page + i64::from(total % s.per_page != 0);
    let page_link = |n: i64| {
        let mut p = vec![format!("page={n}")];
        if !query.is_empty() {
            p.push(format!("q={}", vuuut_core::social_media::encode(&query)));
        }
        if sort != default_sort {
            p.push(format!("sort={sort}"));
        }
        format!("{}?{}", link(&path), p.join("&amp;"))
    };
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let (title, intro, hero) = match &coll {
        Some(c) => (c.title.clone(), c.description.clone(), c.image.clone()),
        None => (if query.is_empty() { s.name(&site_title).to_string() } else { format!("Search: {query}") }, if query.is_empty() { s.intro.clone() } else { String::new() }, String::new()),
    };
    let excerpt = coll.as_ref().map(|c| if c.seo_description.is_empty() { c.description.clone() } else { c.seo_description.clone() }).unwrap_or_else(|| s.intro.clone());
    let page_title = coll.as_ref().map(|c| if c.seo_title.is_empty() { c.title.clone() } else { c.seo_title.clone() }).unwrap_or_else(|| title.clone());

    // The theme's `shop/collection.hbs` (falling back to `shop/shop.hbs`)
    // or `shop/shop.hbs`: `products`, `collection` (on a collection),
    // `collections` (each with `current`), `query`, `sort`, `sorts`,
    // `heading`, `intro`, `total` and `pagination`.
    let names: &[&str] = if coll.is_some() { &["shop/collection", "shop/shop"] } else { &["shop/shop"] };
    let templates = theme_templates(&state, names).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("products".into(), Value::Array(products.iter().map(|p| product_card(p, vs.get(&p.id).map(Vec::as_slice).unwrap_or(&[]), &s)).collect()));
        data.insert("collection".into(), coll.as_ref().map(collection_json).unwrap_or(Value::Null));
        data.insert("collections".into(), Value::Array(colls.iter().map(|c| {
            let mut j = collection_json(c);
            j["current"] = json!(coll.as_ref().is_some_and(|x| x.id == c.id));
            j
        }).collect()));
        data.insert("query".into(), json!(query));
        data.insert("sort".into(), json!(sort));
        data.insert("sorts".into(), Value::Array(PUBLIC_SORTS.iter().map(|(k, l)| json!({ "value": k, "label": l, "selected": *k == sort })).collect()));
        data.insert("heading".into(), json!(title));
        data.insert("intro".into(), json!(intro));
        data.insert("total".into(), json!(total));
        data.insert("pagination".into(), json!({
            "page": page_no,
            "pages": pages.max(1),
            "total": total,
            "limit": s.per_page,
            "prev": if page_no > 1 { json!(page_no - 1) } else { Value::Null },
            "next": if page_no < pages { json!(page_no + 1) } else { Value::Null },
            "prev_url": if page_no > 1 { json!(page_link(page_no - 1).replace("&amp;", "&")) } else { Value::Null },
            "next_url": if page_no < pages { json!(page_link(page_no + 1).replace("&amp;", "&")) } else { Value::Null },
        }));
        let image = coll.as_ref().and_then(|c| (!c.image.is_empty()).then(|| c.image.clone()));
        let kind = if coll.is_some() { "shop-collection" } else { "shop-home" };
        return theme_page(&state, &headers, &path, &page_title, &excerpt, image, templates, data, kind).await;
    }

    let cards: String = products.iter().map(|p| card(p, vs.get(&p.id).map(Vec::as_slice).unwrap_or(&[]), &s)).collect();
    let chips: String = std::iter::once(format!(r#"<a class="vs-chip{on}" href="{h}">All products</a>"#, on = if coll.is_none() { " on" } else { "" }, h = link(BASE)))
        .chain(colls.iter().take(12).map(|c| {
            format!(
                r#"<a class="vs-chip{on}" href="{h}">{t}</a>"#,
                on = if coll.as_ref().is_some_and(|x| x.id == c.id) { " on" } else { "" },
                h = link(&format!("/shop/collections/{}/", c.handle)),
                t = esc(&c.title)
            )
        }))
        .collect();
    let sort_opts: String = PUBLIC_SORTS.iter().map(|(k, l)| format!(r#"<option value="{k}"{s}>{l}</option>"#, s = if *k == sort { " selected" } else { "" })).collect();
    let pager = if pages > 1 {
        format!(
            r#"<nav class="vs-pager">{prev}<span>Page {page_no} of {pages}</span>{next}</nav>"#,
            prev = if page_no > 1 { format!(r#"<a href="{}">← Previous</a>"#, page_link(page_no - 1)) } else { String::new() },
            next = if page_no < pages { format!(r#"<a href="{}">Next →</a>"#, page_link(page_no + 1)) } else { String::new() },
        )
    } else {
        String::new()
    };
    let body = format!(
        r#"<header class="vs-hero{hero_cls}">{hero}<div><h1>{title}</h1>{intro}</div></header>
<nav class="vs-chips" aria-label="Collections">{chips}</nav>
<form class="vs-tools" method="get" action="{action}">{qkeep}<span class="vs-count">{count}</span>
<label>Sort <select name="sort" onchange="this.form.submit()">{sort_opts}</select></label><noscript><button type="submit">Sort</button></noscript></form>
<div class="vs-grid">{cards}</div>{empty}{pager}"#,
        hero_cls = if hero.is_empty() { "" } else { " vs-hero-img" },
        hero = if hero.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="">"#, esc(&hero)) },
        title = esc(&title),
        intro = if intro.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&intro)) },
        action = link(&path),
        qkeep = if query.is_empty() { String::new() } else { format!(r#"<input type="hidden" name="q" value="{}">"#, esc(&query)) },
        count = if total == 1 { "1 product".to_string() } else { format!("{total} products") },
        empty = if products.is_empty() { r#"<p class="vs-empty">No products here yet.</p>"#.to_string() } else { String::new() },
    );
    page(&state, &headers, &path, &page_title, &excerpt, body, coll.and_then(|c| (!c.image.is_empty()).then_some(c.image))).await
}

async fn collections_index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = "/shop/collections/";
    if !s.enabled {
        return not_found(&state, path, headers).await;
    }
    let colls: Vec<repo::Collection> = repo::collections(&state.db).await.into_iter().filter(|c| c.published != 0).collect();
    // The theme's `shop/collections.hbs`: `collections`, each with `image`
    // (its own, else its first product's) and `product_count`.
    let templates = theme_templates(&state, &["shop/collections"]).await;
    let mut listed = Vec::new();
    let mut cards = String::new();
    for c in &colls {
        let ids = repo::collection_product_ids(&state.db, c, true).await;
        let image = if c.image.is_empty() {
            repo::products_by_ids(&state.db, &ids.iter().take(1).cloned().collect::<Vec<_>>()).await.first().and_then(|p| p.first_image()).unwrap_or_default()
        } else {
            c.image.clone()
        };
        let mut j = collection_json(c);
        j["feature_image"] = if image.is_empty() { Value::Null } else { json!(image) };
        j["product_count"] = json!(ids.len());
        listed.push(j);
        cards.push_str(&format!(
            r#"<a class="vs-coll" href="{h}">{img}<span><b>{t}</b><small>{n}</small></span></a>"#,
            h = link(&format!("/shop/collections/{}/", c.handle)),
            img = if image.is_empty() { r#"<span class="vs-noimg"></span>"#.to_string() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&image)) },
            t = esc(&c.title),
            n = if ids.len() == 1 { "1 product".to_string() } else { format!("{} products", ids.len()) },
        ));
    }
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("collections".into(), Value::Array(listed));
        return theme_page(&state, &headers, path, "Collections", "", None, templates, data, "shop-collections").await;
    }
    let body = format!(
        r#"<header class="vs-hero"><div><h1>Collections</h1></div></header><div class="vs-colls">{cards}</div>{empty}"#,
        empty = if colls.is_empty() { r#"<p class="vs-empty">No collections yet.</p>"# } else { "" },
    );
    page(&state, &headers, path, "Collections", "", body, None).await
}

// ---------------------------------------------------------------------------
// one product
// ---------------------------------------------------------------------------

async fn product(State(state): State<AppState>, headers: HeaderMap, Path(handle): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("/shop/products/{handle}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(p) = repo::product_by_handle(&state.db, &rules::handleize(&handle)).await.ok().flatten().filter(|p| p.is_active()) else {
        return not_found(&state, &path, headers).await;
    };
    let vs = repo::variants(&state.db, &p.id).await;
    if vs.is_empty() {
        return not_found(&state, &path, headers).await;
    }
    let options = p.option_list();
    let images = p.image_list();
    let wanted = q.get("variant").and_then(|v| vs.iter().find(|x| &x.id == v));
    let first = wanted.or_else(|| vs.iter().find(|v| v.available(1))).unwrap_or(&vs[0]);

    let gallery = if images.is_empty() {
        r#"<div class="vs-gallery"><div class="vs-main"><span class="vs-noimg"></span></div></div>"#.to_string()
    } else {
        format!(
            r#"<div class="vs-gallery"><div class="vs-main"><img id="vs-main-img" src="{main}" alt="{alt}"></div>{thumbs}</div>"#,
            main = esc(if first.image.is_empty() { &images[0] } else { &first.image }),
            alt = esc(&p.title),
            thumbs = if images.len() > 1 {
                format!(
                    r#"<div class="vs-thumbs">{}</div>"#,
                    images.iter().enumerate().map(|(i, u)| format!(r#"<button type="button" class="vs-thumb{on}" data-src="{u}" aria-label="Image {n}"><img src="{u}" alt="" loading="lazy"></button>"#, on = if i == 0 { " on" } else { "" }, u = esc(u), n = i + 1)).collect::<String>()
                )
            } else {
                String::new()
            },
        )
    };

    let variant_json = serde_json::to_string(
        &vs.iter()
            .map(|v| {
                serde_json::json!({
                    "id": v.id,
                    "options": v.values(),
                    "price": rules::format_money(v.price_cents, &s.currency),
                    "compare": if v.compare_at_cents > v.price_cents { rules::format_money(v.compare_at_cents, &s.currency) } else { String::new() },
                    "available": v.available(1),
                    "image": v.image,
                    "low": v.tracked() && v.inventory > 0 && v.inventory <= s.low_stock && !v.backorder(),
                    "stock": if v.tracked() { v.inventory } else { -1 },
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
    .replace("</", "<\\/");

    let pickers: String = options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let chosen = first.values().get(i).cloned().unwrap_or_default();
            format!(
                r#"<fieldset class="vs-opt" data-index="{i}"><legend>{name}: <b data-chosen>{chosen_e}</b></legend>{vals}</fieldset>"#,
                name = esc(&o.name),
                chosen_e = esc(&chosen),
                vals = o
                    .values
                    .iter()
                    .map(|v| format!(r#"<label class="vs-pill"><input type="radio" name="opt{i}" value="{ve}"{c}><span>{ve}</span></label>"#, ve = esc(v), c = if *v == chosen { " checked" } else { "" }))
                    .collect::<String>()
            )
        })
        .collect();
    let select = format!(
        r#"<label class="vs-variant-select"{hide}>Option <select name="variant_id" id="vs-variant">{opts}</select></label>"#,
        hide = if vs.len() == 1 { " hidden" } else { "" },
        opts = vs
            .iter()
            .map(|v| format!(
                r#"<option value="{id}"{sel}{dis}>{t} — {p}{so}</option>"#,
                id = esc(&v.id),
                sel = if v.id == first.id { " selected" } else { "" },
                dis = if v.available(1) { "" } else { " disabled" },
                t = esc(&v.title),
                p = money(v.price_cents, &s),
                so = if v.available(1) { "" } else { " (sold out)" },
            ))
            .collect::<String>()
    );
    let added = q.get("added").is_some();
    let files = repo::files(&state.db, &p.id).await;
    let free = vs.iter().all(|v| v.price_cents == 0);
    let digital = if files.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="vs-digital"><b>Digital download</b><ul>{}</ul><p>{}</p></div>"#,
            files.iter().map(|f| format!("<li>{} <span>{}</span></li>", esc(&f.name), esc(&rules::format_bytes(f.size_bytes.max(0) as u64)))).collect::<String>(),
            if free {
                "Free — enter your email at checkout and the files are yours. They stay in your account under Downloads."
            } else {
                "After payment the files are added to your account under Downloads, and we email you a link that signs you in."
            }
        )
    };
    let collections: Vec<repo::Collection> = {
        let mut out = Vec::new();
        for c in repo::collections(&state.db).await.into_iter().filter(|c| c.published != 0) {
            if repo::collection_product_ids(&state.db, &c, true).await.contains(&p.id) {
                out.push(c);
            }
        }
        out
    };
    // Related: others in the first collection, else newest.
    let related_ids: Vec<String> = match collections.first() {
        Some(c) => repo::collection_product_ids(&state.db, c, true).await.into_iter().filter(|i| i != &p.id).take(4).collect(),
        None => repo::browse_products(&state.db, &repo::ProductBrowse { status: Some("active"), sort: "newest", limit: 5, ..Default::default() }).await.map(|(r, _)| r).unwrap_or_default().into_iter().map(|x| x.id).filter(|i| i != &p.id).take(4).collect(),
    };
    let related = repo::products_by_ids(&state.db, &related_ids).await;
    let rvs = repo::variants_for(&state.db, &related_ids).await;
    let related_html = if related.is_empty() {
        String::new()
    } else {
        format!(r#"<section class="vs-related"><h2>You may also like</h2><div class="vs-grid">{}</div></section>"#, related.iter().map(|r| card(r, rvs.get(&r.id).map(Vec::as_slice).unwrap_or(&[]), &s)).collect::<String>())
    };
    let absolute = vuuut_core::url::absolute(&state.site_url(), &link(&path));
    let excerpt = if !p.seo_description.is_empty() { p.seo_description.clone() } else if !p.excerpt.is_empty() { p.excerpt.clone() } else { vuuut_render::markdown::html_to_plaintext(&p.description_html).chars().take(160).collect() };
    let page_title = if p.seo_title.is_empty() { p.title.clone() } else { p.seo_title.clone() };

    // The theme's `shop/product.hbs`: `product` (the card data plus
    // `description` HTML, `options`, `variants`, `variant` — the one
    // selected —, `variants_json` for the store script's `data-variants`,
    // `files`, `button_label`, `get_free`, `added`, `schema`), `collection`
    // (its first) and `related`.
    let templates = theme_templates(&state, &["shop/product"]).await;
    if !templates.is_empty() {
        let chosen = first.values();
        let variant = |v: &repo::Variant| json!({
            "id": v.id,
            "title": v.title,
            "options": v.values(),
            "price": rules::format_money(v.price_cents, &s.currency),
            "price_cents": v.price_cents,
            "compare_at": if v.compare_at_cents > v.price_cents { json!(rules::format_money(v.compare_at_cents, &s.currency)) } else { Value::Null },
            "available": v.available(1),
            "selected": v.id == first.id,
            "feature_image": if v.image.is_empty() { Value::Null } else { json!(v.image) },
            "stock": if v.tracked() { json!(v.inventory) } else { Value::Null },
            "low_stock": v.tracked() && v.inventory > 0 && v.inventory <= s.low_stock && !v.backorder(),
            "stock_line": stock_line(v, &s),
        });
        let mut product = product_card(&p, &vs, &s);
        product["description"] = json!(p.description_html);
        product["options"] = Value::Array(options.iter().enumerate().map(|(i, o)| json!({
            "name": o.name,
            "index": i,
            "input": format!("opt{i}"),
            "chosen": chosen.get(i).cloned().unwrap_or_default(),
            "values": o.values.iter().map(|v| json!({ "value": v, "input": format!("opt{i}"), "selected": chosen.get(i) == Some(v) })).collect::<Vec<_>>(),
        })).collect());
        product["variants"] = Value::Array(vs.iter().map(variant).collect());
        product["variant"] = variant(first);
        product["has_variants"] = json!(vs.len() > 1);
        product["variants_json"] = json!(variant_json);
        product["files"] = Value::Array(files.iter().map(|f| json!({ "name": f.name, "size": rules::format_bytes(f.size_bytes.max(0) as u64) })).collect());
        product["digital"] = json!(!files.is_empty());
        product["get_free"] = json!(free && !files.is_empty());
        product["available"] = json!(first.available(1));
        product["button_label"] = json!(if !first.available(1) { "Sold out" } else if free && !files.is_empty() { "Get it free" } else { "Add to cart" });
        product["max_quantity"] = json!(rules::MAX_LINE_QTY);
        product["added"] = json!(added);
        product["schema"] = json!(product_schema(&p, &vs, &images, &s, &absolute, &state.site_url()));
        let mut data = serde_json::Map::new();
        data.insert("product".into(), product);
        data.insert("collection".into(), collections.first().map(collection_json).unwrap_or(Value::Null));
        data.insert("related".into(), Value::Array(related.iter().map(|r| product_card(r, rvs.get(&r.id).map(Vec::as_slice).unwrap_or(&[]), &s)).collect()));
        return theme_page(&state, &headers, &path, &page_title, &excerpt, images.first().cloned(), templates, data, "shop-product").await;
    }

    let body = format!(
        r#"<nav class="vs-crumbs"><a href="{home}">Shop</a>{crumb} › <span>{title}</span></nav>
<div class="vs-product">{gallery}
<div class="vs-info">
{vendor}<h1>{title}</h1>
<div class="vs-price-row" id="vs-price-row">{price}</div>
{tax_note}
<form class="vs-buy" method="post" action="{add}" data-variants="{vjson}">
{pickers}{select}
<p class="vs-stock" id="vs-stock">{stock}</p>
<div class="vs-buy-row">{buy}{demo}</div>
{free_back}{added}
</form>
{digital}
<div class="vs-desc">{desc}</div>
{policies}
</div></div>
{related}
<script type="application/ld+json">{schema}</script>"#,
        home = link(BASE),
        crumb = collections.first().map(|c| format!(r#" › <a href="{}">{}</a>"#, link(&format!("/shop/collections/{}/", c.handle)), esc(&c.title))).unwrap_or_default(),
        title = esc(&p.title),
        vendor = if p.vendor.is_empty() { String::new() } else { format!(r#"<p class="vs-vendor">{}</p>"#, esc(&p.vendor)) },
        price = {
            let was = if first.compare_at_cents > first.price_cents { format!(r#" <s class="vs-was">{}</s> <span class="vs-tag">Sale</span>"#, money(first.compare_at_cents, &s)) } else { String::new() };
            format!(r#"<span class="vs-price vs-price-lg">{}</span>{was}"#, money(first.price_cents, &s))
        },
        tax_note = if s.tax_rate_bp > 0 { format!(r#"<p class="vs-note">{}</p>"#, if s.tax_included { "Tax included." } else { "Tax calculated at checkout." }) } else { String::new() },
        add = link("/shop/cart/add/"),
        vjson = esc(&variant_json),
        stock = stock_line(first, &s),
        // A direct purchase link elsewhere (Polar, Gumroad) takes the place
        // of the cart.
        buy = if p.purchase_url.is_empty() {
            format!(
                r#"<label class="vs-qty"><span>Quantity</span><input type="number" name="qty" value="1" min="1" max="{maxq}" inputmode="numeric"></label>
<button class="vs-btn vs-btn-big" id="vs-add" type="submit"{disabled}>{add_label}</button>"#,
                maxq = rules::MAX_LINE_QTY,
                disabled = if first.available(1) { "" } else { " disabled" },
                add_label = if !first.available(1) { "Sold out" } else if free && !files.is_empty() { "Get it free" } else { "Add to cart" },
            )
        } else {
            format!(r#"<a class="vs-btn vs-btn-big vs-buy-now" href="{}" rel="noopener">Buy now ↗</a>"#, esc(&p.purchase_url))
        },
        demo = if p.demo_url.is_empty() { String::new() } else { format!(r#"<a class="vs-btn vs-btn-big vs-btn-tonal vs-demo" href="{}" target="_blank" rel="noopener">Live demo ↗</a>"#, esc(&p.demo_url)) },
        free_back = if free && !files.is_empty() { r#"<input type="hidden" name="back" value="checkout" data-keep>"# } else { "" },
        digital = digital,
        added = if added { format!(r#"<p class="vs-added" role="status">Added to your cart. <a href="{}">View cart</a> · <a href="{}">Check out</a></p>"#, link("/shop/cart/"), link("/shop/checkout/")) } else { String::new() },
        desc = p.description_html,
        policies = policy_links(&s),
        related = related_html,
        schema = product_schema(&p, &vs, &images, &s, &absolute, &state.site_url()),
    );
    page(&state, &headers, &path, &page_title, &excerpt, body, images.first().cloned()).await
}

fn stock_line(v: &repo::Variant, s: &Settings) -> String {
    if !v.available(1) {
        "Sold out".into()
    } else if v.tracked() && !v.backorder() && v.inventory <= s.low_stock {
        format!("Only {} left", v.inventory)
    } else if v.tracked() && v.inventory <= 0 {
        "Available to pre-order".into()
    } else {
        "In stock".into()
    }
}

fn policy_links(s: &Settings) -> String {
    let mut l = Vec::new();
    if !s.policy_shipping.is_empty() {
        l.push(format!(r#"<a href="{}">Shipping policy</a>"#, link("/shop/policies/shipping/")));
    }
    if !s.policy_refund.is_empty() {
        l.push(format!(r#"<a href="{}">Refund policy</a>"#, link("/shop/policies/refund/")));
    }
    if l.is_empty() { String::new() } else { format!(r#"<p class="vs-policies">{}</p>"#, l.join(" · ")) }
}

fn product_schema(p: &repo::Product, vs: &[repo::Variant], images: &[String], s: &Settings, url: &str, site: &str) -> String {
    let abs = |u: &str| if u.starts_with('/') { vuuut_core::url::absolute(site, u) } else { u.to_string() };
    let offers: Vec<serde_json::Value> = vs
        .iter()
        .map(|v| {
            serde_json::json!({
                "@type": "Offer",
                "price": rules::decimal(v.price_cents, &s.currency),
                "priceCurrency": s.currency,
                "availability": if v.available(1) { "https://schema.org/InStock" } else { "https://schema.org/OutOfStock" },
                "sku": v.sku,
                "url": format!("{url}?variant={}", v.id),
            })
        })
        .collect();
    let mut v = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Product",
        "name": p.title,
        "url": url,
        "image": images.iter().map(|i| abs(i)).collect::<Vec<_>>(),
        "description": vuuut_render::markdown::html_to_plaintext(&p.description_html).chars().take(500).collect::<String>(),
        "offers": offers,
    });
    if !p.vendor.is_empty() {
        v["brand"] = serde_json::json!({ "@type": "Brand", "name": p.vendor });
    }
    if let Some(sku) = vs.first().map(|v| v.sku.clone()).filter(|s| !s.is_empty()) {
        v["sku"] = serde_json::json!(sku);
    }
    v.to_string().replace("</", "<\\/")
}

// ---------------------------------------------------------------------------
// cart
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Line {
    variant: repo::Variant,
    product: repo::Product,
    qty: i64,
}

/// What a cart costs right now, from the database.
struct Priced {
    lines: Vec<Line>,
    /// Lines that were removed or reduced, said to the buyer.
    notes: Vec<String>,
    subtotal: i64,
    taxable: i64,
    needs_shipping: bool,
    discount: Option<repo::Discount>,
    discount_error: Option<String>,
}

async fn price_cart(state: &AppState, s: &Settings, lines: &[CartLine], code: &str, email: &str) -> Priced {
    let ids: Vec<String> = lines.iter().map(|l| l.variant_id.clone()).collect();
    let vs = repo::variants_by_ids(&state.db, &ids).await;
    let pids: Vec<String> = vs.iter().map(|v| v.product_id.clone()).collect();
    let ps = repo::products_by_ids(&state.db, &pids).await;
    let mut out = Priced { lines: Vec::new(), notes: Vec::new(), subtotal: 0, taxable: 0, needs_shipping: false, discount: None, discount_error: None };
    for l in lines {
        let Some(v) = vs.iter().find(|v| v.id == l.variant_id) else {
            out.notes.push("An item in your cart is no longer available and was removed.".into());
            continue;
        };
        let Some(p) = ps.iter().find(|p| p.id == v.product_id && p.is_active()) else {
            out.notes.push("An item in your cart is no longer available and was removed.".into());
            continue;
        };
        let mut qty = l.qty;
        if !v.available(qty) {
            if v.available(1) {
                qty = v.inventory.max(1);
                out.notes.push(format!("Only {} of “{}” left — your cart was updated.", qty, p.title));
            } else {
                out.notes.push(format!("“{}” sold out and was removed.", p.title));
                continue;
            }
        }
        out.subtotal += v.price_cents * qty;
        if p.taxable != 0 {
            out.taxable += v.price_cents * qty;
        }
        if p.requires_shipping != 0 {
            out.needs_shipping = true;
        }
        out.lines.push(Line { variant: v.clone(), product: p.clone(), qty });
    }
    let code = rules::normalise_code(code);
    if !code.is_empty() {
        match repo::discount_by_code(&state.db, &code).await {
            None => out.discount_error = Some("That discount code does not exist.".into()),
            Some(d) => {
                let used_by_buyer = d.once_per_customer != 0 && !email.is_empty() && repo::code_used_by(&state.db, &d.code, email).await;
                let view = rules::DiscountView {
                    kind: rules::DiscountKind::from_key(&d.kind),
                    value: d.value,
                    min_subtotal: d.min_subtotal_cents,
                    usage_limit: d.usage_limit,
                    used: d.used_count,
                    active: d.active != 0,
                    starts_at: d.starts_at.as_deref(),
                    ends_at: d.ends_at.as_deref(),
                    used_by_buyer,
                };
                match rules::discount_applies(&view, out.subtotal, &vuuut_core::dates::now_db(), &s.currency) {
                    Ok(()) => out.discount = Some(d),
                    Err(e) => out.discount_error = Some(e),
                }
            }
        }
    }
    out
}

impl Priced {
    fn totals(&self, s: &Settings, shipping: i64) -> rules::Totals {
        let (item_off, ship_off) = self.discount.as_ref().map(|d| rules::discount_amount(rules::DiscountKind::from_key(&d.kind), d.value, self.subtotal, shipping)).unwrap_or((0, 0));
        rules::totals(self.subtotal, self.taxable, item_off, shipping, ship_off, s.tax_rate_bp, s.tax_included, s.tax_shipping)
    }
    fn as_cart(&self) -> Vec<CartLine> {
        self.lines.iter().map(|l| CartLine { variant_id: l.variant.id.clone(), qty: l.qty }).collect()
    }
}

#[derive(serde::Deserialize)]
struct AddForm {
    #[serde(default)]
    variant_id: String,
    #[serde(default)]
    qty: Option<String>,
    #[serde(default)]
    back: Option<String>,
}

async fn cart_add(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<AddForm>) -> Response {
    let s = settings(&state);
    if !s.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This request did not come from the store.").into_response();
    }
    let qty = f.qty.and_then(|q| q.trim().parse::<i64>().ok()).unwrap_or(1).clamp(1, rules::MAX_LINE_QTY);
    let Some(v) = (vuuut_core::ids::is_objectid(&f.variant_id)).then_some(()).and(repo::find_variant(&state.db, &f.variant_id).await) else {
        return Redirect::to(&link("/shop/cart/")).into_response();
    };
    let Some(p) = repo::find_product(&state.db, &v.product_id).await.ok().flatten().filter(|p| p.is_active()) else {
        return Redirect::to(&link("/shop/cart/")).into_response();
    };
    let (token, fresh) = match current_cart(&state, &headers).await {
        Some(c) => (c.id, false),
        None => match repo::create_cart(&state.db).await {
            Ok(t) => (t, true),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    };
    let mut lines = repo::find_cart(&state.db, &token).await.map(|c| c.lines()).unwrap_or_default();
    lines.push(CartLine { variant_id: v.id.clone(), qty });
    let lines = rules::normalise_cart(lines);
    let _ = repo::save_cart_lines(&state.db, &token, &lines).await;
    let to = if f.back.as_deref() == Some("checkout") {
        link("/shop/checkout/")
    } else if f.back.as_deref() == Some("product") { link(&format!("/shop/products/{}/?variant={}&added=1", p.handle, v.id)) } else { link("/shop/cart/") };
    let mut res = Redirect::to(&to).into_response();
    if fresh {
        res.headers_mut().insert(header::SET_COOKIE, cart_cookie(&state, &token));
    }
    res
}

async fn cart_update(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<Vec<(String, String)>>) -> Response {
    if !settings(&state).enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This request did not come from the store.").into_response();
    }
    let Some(cart) = current_cart(&state, &headers).await else {
        return Redirect::to(&link("/shop/cart/")).into_response();
    };
    let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());
    let mut lines = cart.lines();
    if let Some(rm) = get("remove") {
        lines.retain(|l| l.variant_id != rm);
    }
    for l in lines.iter_mut() {
        if let Some(q) = get(&format!("qty_{}", l.variant_id)).and_then(|q| q.trim().parse::<i64>().ok()) {
            l.qty = q.clamp(0, rules::MAX_LINE_QTY);
        }
    }
    // The − / + buttons: `id:-1` or `id:1`, applied after the typed quantity.
    if let Some((vid, delta)) = get("step").as_deref().and_then(|v| v.split_once(':')) {
        let delta: i64 = delta.parse().unwrap_or(0).clamp(-1, 1);
        if let Some(l) = lines.iter_mut().find(|l| l.variant_id == vid) {
            l.qty = (l.qty + delta).clamp(0, rules::MAX_LINE_QTY);
        }
    }
    let lines = rules::normalise_cart(lines);
    let _ = repo::save_cart_lines(&state.db, &cart.id, &lines).await;
    let mut to = link("/shop/cart/");
    if get("remove_code").is_some() {
        let _ = repo::set_cart_discount(&state.db, &cart.id, "").await;
    } else if let Some(code) = get("code").map(|c| rules::normalise_code(&c)).filter(|c| !c.is_empty()) {
        let _ = repo::set_cart_discount(&state.db, &cart.id, &code).await;
    }
    if get("checkout").is_some() {
        to = link("/shop/checkout/");
    } else if let Some(back) = get("back").filter(|b| b == "checkout") {
        to = link(&format!("/shop/{back}/"));
    }
    Redirect::to(&to).into_response()
}

fn line_row(l: &Line, s: &Settings, editable: bool) -> String {
    let img = if l.variant.image.is_empty() { l.product.first_image().unwrap_or_default() } else { l.variant.image.clone() };
    format!(
        r#"<div class="vs-line"><a class="vs-line-img" href="{href}">{img}</a>
<div class="vs-line-body"><a href="{href}"><b>{title}</b></a>{variant}<span class="vs-line-unit">{unit}</span></div>
{qty}<span class="vs-line-total">{total}</span></div>"#,
        href = link(&format!("/shop/products/{}/?variant={}", l.product.handle, l.variant.id)),
        img = if img.is_empty() { r#"<span class="vs-noimg"></span>"#.to_string() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&img)) },
        title = esc(&l.product.title),
        variant = if l.variant.title == rules::DEFAULT_VARIANT { String::new() } else { format!(r#"<span class="vs-line-variant">{}</span>"#, esc(&l.variant.title)) },
        unit = money(l.variant.price_cents, s),
        qty = if editable {
            format!(
                r#"<span class="vs-line-qty"><span class="vs-stepper"><button type="submit" name="step" value="{id}:-1" aria-label="One fewer">−</button><input type="number" name="qty_{id}" value="{q}" min="0" max="{max}" aria-label="Quantity" data-autosubmit><button type="submit" name="step" value="{id}:1" aria-label="One more">+</button></span><button class="vs-remove" type="submit" name="remove" value="{id}">Remove</button></span>"#,
                id = esc(&l.variant.id),
                q = l.qty,
                max = rules::MAX_LINE_QTY
            )
        } else {
            format!(r#"<span class="vs-line-qty">× {}</span>"#, l.qty)
        },
        total = money(l.variant.price_cents * l.qty, s),
    )
}

async fn cart_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = "/shop/cart/";
    if !s.enabled {
        return not_found(&state, path, headers).await;
    }
    let cart = current_cart(&state, &headers).await;
    let lines = cart.as_ref().map(|c| c.lines()).unwrap_or_default();
    let priced = price_cart(&state, &s, &lines, cart.as_ref().map(|c| c.discount_code.as_str()).unwrap_or(""), cart.as_ref().map(|c| c.email.as_str()).unwrap_or("")).await;
    if let Some(c) = &cart {
        if priced.as_cart() != lines {
            let _ = repo::save_cart_lines(&state.db, &c.id, &priced.as_cart()).await;
        }
    }
    let body = if priced.lines.is_empty() {
        format!(
            r#"<header class="vs-hero"><div><h1>Your cart</h1></div></header>{notes}<div class="vs-empty-cart"><p>Your cart is empty.</p><a class="vs-btn" href="{home}">Continue shopping</a></div>"#,
            notes = notes_html(&priced.notes),
            home = link(BASE)
        )
    } else {
        let t = priced.totals(&s, 0);
        let code = cart.as_ref().map(|c| c.discount_code.clone()).unwrap_or_default();
        format!(
            r#"<header class="vs-hero"><div><h1>Your cart</h1></div></header>{notes}
<form class="vs-cart" method="post" action="{update}">
<div class="vs-lines">{rows}</div>
<aside class="vs-summary">
<div class="vs-code">{code_box}</div>
<dl><dt>Subtotal</dt><dd>{sub}</dd>{disc}</dl>
<p class="vs-note">{ship_note}</p>
<button class="vs-btn vs-btn-big vs-wide" type="submit" name="checkout" value="1">Check out</button>
<noscript><button class="vs-link vs-update" type="submit">Update cart</button></noscript>
<a class="vs-link" href="{home}">Continue shopping</a>
</aside></form>"#,
            notes = notes_html(&priced.notes),
            update = link("/shop/cart/update/"),
            rows = priced.lines.iter().map(|l| line_row(l, &s, true)).collect::<String>(),
            code_box = discount_box(&priced, &code),
            sub = money(t.subtotal, &s),
            disc = if t.discount > 0 { format!(r#"<dt>Discount</dt><dd>−{}</dd>"#, money(t.discount, &s)) } else { String::new() },
            ship_note = if s.tax_rate_bp > 0 && !s.tax_included { "Shipping and taxes calculated at checkout." } else { "Shipping calculated at checkout." },
            home = link(BASE),
        )
    };
    page(&state, &headers, path, "Your cart", "", body, None).await
}

fn notes_html(notes: &[String]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    format!(r#"<div class="vs-notice" role="status">{}</div>"#, notes.iter().map(|n| format!("<p>{}</p>", esc(n))).collect::<String>())
}

fn discount_box(priced: &Priced, code: &str) -> String {
    match (&priced.discount, &priced.discount_error) {
        (Some(d), _) => format!(r#"<p class="vs-code-on"><b>{}</b> applied <button class="vs-link" type="submit" name="remove_code" value="1">Remove</button></p>"#, esc(&d.code)),
        (None, err) => format!(
            r#"<label class="vs-code-in"><input name="code" value="{c}" placeholder="Discount code" maxlength="40" aria-label="Discount code"><button class="vs-btn vs-btn-tonal" type="submit">Apply</button></label>{e}"#,
            c = esc(code),
            e = err.as_ref().map(|e| format!(r#"<p class="vs-error">{}</p>"#, esc(e))).unwrap_or_default()
        ),
    }
}

// ---------------------------------------------------------------------------
// checkout
// ---------------------------------------------------------------------------

/// Shipping choices for this cart: the store's rates that fit, or a free
/// standard rate when the store has defined none.
async fn shipping_options(state: &AppState, country: &str, subtotal: i64) -> Vec<(String, String, i64)> {
    let rates = repo::shipping_rates(&state.db).await;
    if rates.is_empty() {
        return vec![("standard".into(), "Standard shipping".into(), 0)];
    }
    rates.into_iter().filter(|r| r.applies(country, subtotal)).map(|r| (r.id, r.name, r.price_cents)).collect()
}

async fn checkout_methods(
    state: &AppState, settings: &Settings, priced: &Priced, totals: rules::Totals,
) -> (Vec<PaymentMethod>, vuuut_db::repos::payments::ProductMapping) {
    use vuuut_api::payments;

    let mut mapping = vuuut_db::repos::payments::ProductMapping::default();
    if totals.total == 0 {
        return (Vec::new(), mapping);
    }
    let mut methods = settings.payment_methods(vuuut_api::ecom::stripe(state).is_some());
    let [line] = priced.lines.as_slice() else { return (methods, mapping) };
    if priced.needs_shipping || line.qty != 1 || !line.product.option_list().is_empty()
        || totals.tax != 0 || totals.discount != 0
    {
        return (methods, mapping);
    }
    let polar = payments::enabled(state, PaymentMethod::Polar);
    let gumroad = payments::enabled(state, PaymentMethod::Gumroad);
    if polar || gumroad {
        mapping = vuuut_db::repos::payments::mapping(&state.db, &line.product.id).await.unwrap_or_default();
        if polar && !mapping.polar_product.is_empty() {
            methods.push(PaymentMethod::Polar);
        }
        if gumroad && !mapping.gumroad_product.is_empty() {
            methods.push(PaymentMethod::Gumroad);
        }
    }
    (methods, mapping)
}

async fn checkout_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    checkout_view(&state, &headers, &q, &HashMap::new(), &[]).await
}

async fn checkout_view(state: &AppState, headers: &HeaderMap, q: &HashMap<String, String>, form: &HashMap<String, String>, errors: &[String]) -> Response {
    let s = settings(state);
    let path = "/shop/checkout/";
    if !s.enabled {
        return not_found(state, path, headers.clone()).await;
    }
    let Some(cart) = current_cart(state, headers).await else {
        return Redirect::to(&link("/shop/cart/")).into_response();
    };
    let member = vuuut_api::members::current_member(state, headers).await;
    let customer = match (&member, form.get("email")) {
        (Some(m), None) => repo::customer_by_email(&state.db, &m.email).await,
        _ => None,
    };
    let saved = customer.as_ref().map(|c| c.address()).unwrap_or_default();
    let val = |k: &str| -> String {
        if let Some(v) = form.get(k) {
            return v.clone();
        }
        match k {
            "email" => member.as_ref().map(|m| m.email.clone()).filter(|e| !e.is_empty()).unwrap_or_else(|| cart.email.clone()),
            "first_name" => saved.first_name.clone(),
            "last_name" => saved.last_name.clone(),
            "company" => saved.company.clone(),
            "line1" => saved.line1.clone(),
            "line2" => saved.line2.clone(),
            "city" => saved.city.clone(),
            "region" => saved.region.clone(),
            "postal" => saved.postal.clone(),
            "country" => if saved.country.is_empty() || !s.ships_to(&saved.country) { s.countries.first().cloned().unwrap_or_default() } else { saved.country.clone() },
            "phone" => saved.phone.clone(),
            _ => String::new(),
        }
    };
    let priced = price_cart(state, &s, &cart.lines(), &cart.discount_code, &val("email")).await;
    if priced.lines.is_empty() {
        return Redirect::to(&link("/shop/cart/")).into_response();
    }
    let country = val("country");
    let options = shipping_options(state, &country, priced.subtotal).await;
    // Unless the buyer chose, offer the cheapest rate that fits.
    let chosen_rate = form.get("shipping").cloned().filter(|r| options.iter().any(|o| &o.0 == r)).or_else(|| options.iter().min_by_key(|o| o.2).map(|o| o.0.clone()));
    let ship_cost = if priced.needs_shipping { options.iter().find(|o| Some(&o.0) == chosen_rate.as_ref()).map(|o| o.2).unwrap_or(0) } else { 0 };
    let t = priced.totals(&s, ship_cost);
    let free_order = t.total == 0;
    let (methods, _) = checkout_methods(state, &s, &priced, t).await;
    let chosen_pay = form.get("payment").and_then(|p| PaymentMethod::from_key(p)).filter(|p| methods.contains(p)).or(methods.first().copied());

    let field = |name: &str, label: &str, kind: &str, req: bool, auto: &str| {
        format!(
            r#"<label class="vs-field"><span>{label}{star}</span><input type="{kind}" name="{name}" value="{v}" autocomplete="{auto}"{req} maxlength="120"></label>"#,
            v = esc(&val(name)),
            star = if req { "" } else { " <i>(optional)</i>" },
            req = if req { " required" } else { "" },
        )
    };
    let country_opts: String = {
        let mut list: Vec<(&str, &str)> = if s.countries.is_empty() {
            rules::COUNTRIES.to_vec()
        } else {
            s.countries.iter().map(|code| (code.as_str(), rules::country_name(code))).collect()
        };
        list.sort_by(|a, b| a.1.cmp(b.1));
        std::iter::once(r#"<option value="">Choose…</option>"#.to_string())
            .chain(list.iter().map(|(c, n)| format!(r#"<option value="{c}"{sel}>{n}</option>"#, sel = if *c == country { " selected" } else { "" }, n = esc(n))))
            .collect()
    };
    let shipping_html = if priced.needs_shipping {
        let opts: String = if options.is_empty() {
            r#"<p class="vs-error">We do not ship to that country yet.</p>"#.to_string()
        } else {
            options
                .iter()
                .map(|(id, name, price)| {
                    format!(
                        r#"<label class="vs-radio"><input type="radio" name="shipping" value="{id}"{c} data-reprice><span>{name}</span><b>{p}</b></label>"#,
                        id = esc(id),
                        c = if Some(id) == chosen_rate.as_ref() { " checked" } else { "" },
                        name = esc(name),
                        p = if *price == 0 { "Free".to_string() } else { money(*price, &s) },
                    )
                })
                .collect()
        };
        format!(
            r#"<section class="vs-step"><h2>Delivery</h2>
<label class="vs-field"><span>Country</span><select name="country" required data-reprice>{country_opts}</select></label>
<div class="vs-two">{fname}{lname}</div>{company}{line1}{line2}
<div class="vs-three">{city}{region}{postal}</div>{phone}</section>
<section class="vs-step"><h2>Shipping method</h2>{opts}</section>"#,
            fname = field("first_name", "First name", "text", false, "given-name"),
            lname = field("last_name", "Last name", "text", true, "family-name"),
            company = field("company", "Company", "text", false, "organization"),
            line1 = field("line1", "Address", "text", true, "address-line1"),
            line2 = field("line2", "Apartment, suite, etc.", "text", false, "address-line2"),
            city = field("city", "City", "text", true, "address-level2"),
            region = field("region", "State / region", "text", false, "address-level1"),
            postal = field("postal", "Postal code", "text", false, "postal-code"),
            phone = field("phone", "Phone", "tel", s.require_phone, "tel"),
        )
    } else {
        String::new()
    };
    let pay_html = if free_order {
        r#"<p class="vs-note">Nothing to pay. Your order is confirmed as soon as you place it.</p>"#.to_string()
    } else if methods.is_empty() {
        r#"<p class="vs-error">This store is not taking payments right now.</p>"#.to_string()
    } else {
        methods
            .iter()
            .map(|m| {
                let detail = match m {
                    PaymentMethod::Stripe => "You will be taken to Stripe's secure page to pay by card.".to_string(),
                    PaymentMethod::Polar | PaymentMethod::Gumroad => format!("Complete your payment on {}. The provider may collect applicable tax.", m.label()),
                    PaymentMethod::Bank => "Pay by bank transfer — the details are shown after you order.".to_string(),
                    PaymentMethod::Manual => "Pay when your order arrives.".to_string(),
                };
                format!(
                    r#"<label class="vs-radio vs-radio-tall"><input type="radio" name="payment" value="{k}"{c}><span>{l}<small>{d}</small></span></label>"#,
                    k = m.key(),
                    c = if Some(*m) == chosen_pay { " checked" } else { "" },
                    l = esc(m.label()),
                    d = esc(&detail)
                )
            })
            .collect()
    };
    let err_html = if errors.is_empty() { String::new() } else { format!(r#"<div class="vs-notice vs-notice-err" role="alert">{}</div>"#, errors.iter().map(|e| format!("<p>{}</p>", esc(e))).collect::<String>()) };
    let summary = format!(
        r#"<aside class="vs-summary vs-summary-sticky"><h2>Order summary</h2><div class="vs-lines vs-lines-small">{rows}</div>
<form method="post" action="{update}" class="vs-code"><input type="hidden" name="back" value="checkout">{code_box}</form>
<dl><dt>Subtotal</dt><dd>{sub}</dd>{disc}{ship}{tax}<dt class="vs-total">Total</dt><dd class="vs-total">{total} <small>{cur}</small></dd></dl></aside>"#,
        rows = priced.lines.iter().map(|l| line_row(l, &s, false)).collect::<String>(),
        update = link("/shop/cart/update/"),
        code_box = discount_box(&priced, &cart.discount_code),
        sub = money(t.subtotal, &s),
        disc = if t.discount > 0 { format!(r#"<dt>Discount{}</dt><dd>−{}</dd>"#, priced.discount.as_ref().map(|d| format!(" <span class=\"vs-pillcode\">{}</span>", esc(&d.code))).unwrap_or_default(), money(t.discount, &s)) } else { String::new() },
        ship = if priced.needs_shipping { format!(r#"<dt>Shipping</dt><dd>{}</dd>"#, if t.shipping == 0 { "Free".to_string() } else { money(t.shipping, &s) }) } else { String::new() },
        tax = if s.tax_rate_bp > 0 { format!(r#"<dt>{}</dt><dd>{}</dd>"#, if s.tax_included { "Includes tax" } else { "Tax" }, money(t.tax, &s)) } else { String::new() },
        total = money(t.total, &s),
        cur = esc(&s.currency),
    );
    let body = format!(
        r#"<header class="vs-hero"><div><h1>Checkout</h1></div></header>{notes}{err_html}
<div class="vs-checkout">
<form class="vs-checkout-form" method="post" action="{action}" id="vs-checkout">
<section class="vs-step"><h2>Contact</h2>{signed}{digital_note}
<label class="vs-field"><span>Email</span><input type="email" name="email" value="{email}" required autocomplete="email" maxlength="191"></label>
<label class="vs-check"><input type="checkbox" name="marketing" value="1"{mk}> Email me with news and offers</label></section>
{shipping_html}
<section class="vs-step"><h2>Payment</h2>{pay_html}</section>
<section class="vs-step"><label class="vs-field"><span>Order note <i>(optional)</i></span><textarea name="note" rows="2" maxlength="1000">{note}</textarea></label></section>
<input type="text" name="website_url" value="" tabindex="-1" autocomplete="off" class="vs-hp" aria-hidden="true">
<button class="vs-btn vs-btn-big vs-wide" type="submit" name="place" value="1"{dis}>{place}</button>
{reprice}
<p class="vs-secure"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zM9 6c0-1.66 1.34-3 3-3s3 1.34 3 3v2H9V6z"/></svg> Your details are only used for this order.</p>
{policies}
</form>
{summary}</div>"#,
        notes = notes_html(&priced.notes),
        action = link(path),
        signed = match &member {
            Some(m) => format!(r#"<p class="vs-note">Signed in as {}. <a href="{}">My products</a></p>"#, esc(&m.email), link("/shop/dashboard/")),
            None => String::new(),
        },
        email = esc(&val("email")),
        digital_note = if repo::digital_ids(&state.db, &priced.lines.iter().map(|l| l.product.id.clone()).collect::<Vec<_>>()).await.is_empty() {
            String::new()
        } else {
            r#"<p class="vs-note">Your downloads are kept in a member account for this email. We create it if you do not have one, and email you a link that signs you in.</p>"#.to_string()
        },
        mk = if form.get("marketing").is_some() { " checked" } else { "" },
        note = esc(&val("note")),
        dis = if (methods.is_empty() && !free_order) || (priced.needs_shipping && options.is_empty()) { " disabled" } else { "" },
        place = if free_order { "Get my download".to_string() } else if matches!(chosen_pay, Some(PaymentMethod::Stripe | PaymentMethod::Polar | PaymentMethod::Gumroad)) { format!("Pay {}", money(t.total, &s)) } else { format!("Place order · {}", money(t.total, &s)) },
        policies = policy_links(&s),
        reprice = if priced.needs_shipping { r#"<noscript><button class="vs-link" type="submit" name="reprice" value="1" formnovalidate>Update shipping and totals</button></noscript>"# } else { "" },
    );
    let _ = q;
    page(state, headers, path, "Checkout", "", body, None).await
}

async fn checkout_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    Form(f): Form<Vec<(String, String)>>,
) -> Response {
    let s = settings(&state);
    if !s.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This request did not come from the store.").into_response();
    }
    let form: HashMap<String, String> = f.into_iter().collect();
    let get = |k: &str| form.get(k).cloned().unwrap_or_default();
    let Some(cart) = current_cart(&state, &headers).await else {
        return Redirect::to(&link("/shop/cart/")).into_response();
    };
    let email = get("email").trim().to_lowercase();
    if rules::is_email(&email) && email != cart.email {
        let _ = repo::set_cart_email(&state.db, &cart.id, &email).await;
    }
    // "Update totals" just redraws with what was typed.
    if form.contains_key("reprice") || !form.contains_key("place") {
        return checkout_view(&state, &headers, &HashMap::new(), &form, &[]).await;
    }
    if !get("website_url").is_empty() {
        return Redirect::to(&link("/shop/cart/")).into_response();
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("shop_checkout:{ip}"), &CHECKOUT_PER_IP).await.is_err() {
        return checkout_view(&state, &headers, &HashMap::new(), &form, &["Too many attempts from your connection. Please wait a little and try again.".into()]).await;
    }

    let mut errors: Vec<String> = Vec::new();
    if !rules::is_email(&email) {
        errors.push("Enter a valid email address.".into());
    }
    let priced = price_cart(&state, &s, &cart.lines(), &cart.discount_code, &email).await;
    if priced.lines.is_empty() {
        return Redirect::to(&link("/shop/cart/")).into_response();
    }
    if !priced.notes.is_empty() {
        let _ = repo::save_cart_lines(&state.db, &cart.id, &priced.as_cart()).await;
        errors.extend(priced.notes.iter().cloned());
    }
    let address = Address::from_form(|k| form.get(k).cloned());
    let mut ship_name = String::new();
    let mut ship_cost = 0;
    if priced.needs_shipping {
        let missing = address.missing(s.require_phone);
        if !missing.is_empty() {
            errors.push(format!("Fill in your {}.", missing.join(", ")));
        }
        if !address.country.is_empty() && !s.ships_to(&address.country) {
            errors.push("We do not ship to that country.".into());
        }
        let options = shipping_options(&state, &address.country, priced.subtotal).await;
        match options.iter().find(|o| o.0 == get("shipping")).or(if options.len() == 1 { options.first() } else { None }) {
            Some((_, name, price)) => {
                ship_name = name.clone();
                ship_cost = *price;
            }
            None => errors.push("Choose a shipping method.".into()),
        }
    }
    let totals = priced.totals(&s, ship_cost);
    let free_order = totals.total == 0;
    let (methods, mapping) = checkout_methods(&state, &s, &priced, totals).await;
    let choice = get("payment");
    let method = if choice.is_empty() && methods.len() == 1 {
        methods.first().copied()
    } else {
        PaymentMethod::from_key(&choice).filter(|method| methods.contains(method))
    };
    if method.is_none() && !free_order {
        errors.push("Choose how to pay.".into());
        return checkout_view(&state, &headers, &HashMap::new(), &form, &errors).await;
    }
    if priced.discount_error.is_some() && !cart.discount_code.is_empty() {
        // A code that stopped applying is dropped, and said, not charged wrongly.
        let _ = repo::set_cart_discount(&state.db, &cart.id, "").await;
        errors.push(priced.discount_error.clone().unwrap_or_default());
    }
    if !errors.is_empty() {
        return checkout_view(&state, &headers, &HashMap::new(), &form, &errors).await;
    }
    let provider_product = match method {
        Some(PaymentMethod::Polar) => Some(mapping.polar_product),
        Some(PaymentMethod::Gumroad) => Some(mapping.gumroad_product),
        _ => None,
    };
    let new = repo::NewOrder {
        email: email.clone(),
        phone: address.phone.clone(),
        first_name: address.first_name.clone(),
        last_name: address.last_name.clone(),
        accepts_marketing: form.contains_key("marketing"),
        currency: s.currency.clone(),
        lines: priced
            .lines
            .iter()
            .map(|l| repo::NewLine {
                product_id: l.product.id.clone(),
                variant_id: l.variant.id.clone(),
                title: l.product.title.clone(),
                variant_title: l.variant.title.clone(),
                sku: l.variant.sku.clone(),
                image: if l.variant.image.is_empty() { l.product.first_image().unwrap_or_default() } else { l.variant.image.clone() },
                unit_cents: l.variant.price_cents,
                quantity: l.qty,
                requires_shipping: l.product.requires_shipping != 0,
                track: l.variant.tracked(),
                backorder: l.variant.backorder(),
            })
            .collect(),
        totals,
        tax_included: s.tax_included,
        discount_code: priced.discount.as_ref().map(|d| d.code.clone()).unwrap_or_default(),
        shipping_method: ship_name,
        address: if priced.needs_shipping { address } else { Address::default() },
        payment_method: if free_order { "free".into() } else { method.map(|m| m.key()).unwrap_or("manual").into() },
        provider_product,
        financial_status: if totals.total == 0 { "paid".into() } else { "pending".into() },
        note: rules::clean_multiline(&get("note"), 1000),
        source: "web".into(),
        cart_id: Some(cart.id.clone()),
        actor: email.clone(),
    };
    let order = match repo::place_order(&state.db, &new).await {
        Ok(o) => o,
        Err(e) => return checkout_view(&state, &headers, &HashMap::new(), &form, &[e.to_string()]).await,
    };
    if matches!(method, Some(PaymentMethod::Polar | PaymentMethod::Gumroad)) {
        return continue_hosted_payment(&state, &order).await;
    }
    if method == Some(PaymentMethod::Stripe) && order.total_cents > 0 {
        if let Some(cfg) = vuuut_api::ecom::stripe(&state) {
            let site = state.site_url();
            let back = |p: &str| vuuut_core::url::absolute(&site, &link(p));
            let success = format!("{}?order={}&session_id={{CHECKOUT_SESSION_ID}}", back("/shop/checkout/paid/"), order.token);
            let cancel = back(&format!("/shop/orders/{}/", order.token));
            let site_title = state.settings.get_str("title").unwrap_or_default();
            let label = format!("{} — order #{}", s.name(&site_title), order.number);
            match vuuut_api::ecom::start_card_payment(&cfg, &order, &label, &success, &cancel).await {
                Ok((session, url)) => {
                    let _ = repo::set_payment_ref(&state.db, &order.id, &session).await;
                    repo::add_event(&state.db, &order.id, "payment", "Sent to Stripe to pay by card.", "").await;
                    return Redirect::to(&url).into_response();
                }
                Err(e) => {
                    repo::add_event(&state.db, &order.id, "payment", &format!("Stripe could not start a payment: {e}"), "").await;
                }
            }
        }
    }
    vuuut_api::ecom::receipt(&state, &order).await;
    vuuut_api::ecom::notify_staff(&state, &order).await;
    if order.is_paid() {
        // Free: the files unlock now.
        vuuut_api::ecom::on_paid(&state, &order).await;
    }
    Redirect::to(&link(&format!("/shop/orders/{}/?placed=1", order.token))).into_response()
}

const CHECKOUT_PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 20, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };

async fn stripe_return(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let token = q.get("order").cloned().unwrap_or_default();
    let session = q.get("session_id").cloned().unwrap_or_default();
    if !rules::is_order_token(&token) {
        return Redirect::to(&link("/shop/")).into_response();
    }
    let Some(order) = repo::order_by_token(&state.db, &token).await else {
        return Redirect::to(&link("/shop/")).into_response();
    };
    let to = link(&format!("/shop/orders/{}/?placed=1", order.token));
    if order.is_paid() {
        return Redirect::to(&to).into_response();
    }
    let Some(cfg) = vuuut_api::ecom::stripe(&state) else { return Redirect::to(&to).into_response() };
    // Believe Stripe, not the address bar: the session must be the one this
    // order started, name this order, and be paid in full.
    if session != order.payment_ref {
        return Redirect::to(&to).into_response();
    }
    if let Some(reference) = vuuut_api::ecom::card_payment_settled(&cfg, &order, &session).await {
        if repo::mark_paid(&state.db, &order.id, &reference).await.is_ok() {
            repo::add_event(&state.db, &order.id, "paid", &format!("Paid {} by card through Stripe.", rules::format_money(order.total_cents, &order.currency)), "").await;
            if let Some(o) = repo::find_order(&state.db, &order.id).await {
                vuuut_api::ecom::receipt(&state, &o).await;
                vuuut_api::ecom::notify_staff(&state, &o).await;
                vuuut_api::ecom::on_paid(&state, &o).await;
            }
        }
    }
    Redirect::to(&to).into_response()
}

// ---------------------------------------------------------------------------
// order status
// ---------------------------------------------------------------------------

async fn continue_hosted_payment(state: &AppState, order: &repo::Order) -> Response {
    match vuuut_api::payments::hosted_checkout(state, order).await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(error) => {
            tracing::warn!("could not open hosted checkout for order {}: {error}", order.id);
            Redirect::to(&link(&format!("/shop/orders/{}/?payment_error=1", order.token))).into_response()
        }
    }
}

async fn order_pay(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>) -> Response {
    if !same_origin(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !rules::is_order_token(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(order) = repo::order_by_token(&state.db, &token).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if order.financial_status != "pending" || order.is_cancelled() {
        return Redirect::to(&link(&format!("/shop/orders/{token}/"))).into_response();
    }
    continue_hosted_payment(&state, &order).await
}

async fn order_page(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("/shop/orders/{token}/");
    if !rules::is_order_token(&token) {
        return not_found(&state, &path, headers).await;
    }
    let Some(o) = repo::order_by_token(&state.db, &token).await else {
        return not_found(&state, &path, headers).await;
    };
    let items = repo::order_items(&state.db, &o.id).await;
    let m = |c: i64| esc(&rules::format_money(c, &o.currency));
    let step = |on: bool, label: &str, when: Option<&str>| {
        format!(r#"<li class="{c}"><b>{label}</b>{w}</li>"#, c = if on { "on" } else { "" }, w = when.map(|w| format!("<small>{}</small>", esc(w.get(..10).unwrap_or(w)))).unwrap_or_default())
    };
    let status = if o.is_cancelled() {
        r#"<p class="vs-notice vs-notice-err">This order was cancelled.</p>"#.to_string()
    } else {
        format!(
            r#"<ol class="vs-progress">{}{}{}</ol>"#,
            step(true, "Order placed", Some(&o.created_at)),
            step(o.is_paid(), "Paid", o.paid_at.as_deref()),
            step(o.fulfillment_status == "fulfilled", "Shipped", o.fulfilled_at.as_deref()),
        )
    };
    let pay_box = if o.is_paid() || o.is_cancelled() {
        String::new()
    } else {
        match o.payment_method.as_str() {
            "bank" => format!(r#"<div class="vs-notice"><p><b>Please pay by bank transfer</b></p><p>{}</p><p>Use <b>#{}</b> as the payment reference. We will ship once it arrives.</p></div>"#, esc(&s.bank_details).replace('\n', "<br>"), o.number),
            "manual" => r#"<div class="vs-notice"><p>You will pay when your order is delivered.</p></div>"#.to_string(),
            "stripe" => r#"<div class="vs-notice"><p>We have not received your card payment yet. If you just paid, this page will update in a moment.</p></div>"#.to_string(),
            "polar" | "gumroad" if o.financial_status == "pending" => {
                let provider = rules::payment_label(&o.payment_method);
                let href = link(&format!("/shop/orders/{token}/pay/"));
                let message = if q.contains_key("payment_error") {
                    "The payment provider could not open checkout. Please retry or contact the store."
                } else {
                    "Waiting for the payment provider to confirm payment. If you have paid, check back shortly."
                };
                format!(r#"<div class="vs-notice"><p>{message}</p><form method="post" action="{href}"><button class="vs-btn" type="submit">Continue to {provider}</button></form></div>"#)
            }
            _ => String::new(),
        }
    };
    let tracking = if o.fulfillment_status == "fulfilled" && (!o.tracking_number.is_empty() || !o.tracking_url.is_empty()) {
        format!(
            r#"<div class="vs-notice"><p><b>On its way</b> — {c} {n}</p>{l}</div>"#,
            c = esc(&o.tracking_company),
            n = esc(&o.tracking_number),
            l = if o.tracking_url.is_empty() { String::new() } else { format!(r#"<p><a class="vs-btn" href="{}" rel="noopener nofollow" target="_blank">Track package</a></p>"#, esc(&o.tracking_url)) }
        )
    } else {
        String::new()
    };
    let rows: String = items
        .iter()
        .map(|i| {
            format!(
                r#"<div class="vs-line"><span class="vs-line-img">{img}</span><div class="vs-line-body"><b>{t}</b>{v}<span class="vs-line-unit">{u}</span></div><span class="vs-line-qty">× {q}</span><span class="vs-line-total">{tot}</span></div>"#,
                img = if i.image.is_empty() { r#"<span class="vs-noimg"></span>"#.to_string() } else { format!(r#"<img src="{}" alt="">"#, esc(&i.image)) },
                t = esc(&i.title),
                v = if i.variant_title.is_empty() || i.variant_title == rules::DEFAULT_VARIANT { String::new() } else { format!(r#"<span class="vs-line-variant">{}</span>"#, esc(&i.variant_title)) },
                u = m(i.unit_cents),
                q = i.quantity,
                tot = m(i.total_cents),
            )
        })
        .collect();
    let addr = o.address();
    let downloads = order_downloads(&state, &headers, &o, q.get("sent").is_some()).await;
    let body = format!(
        r#"<header class="vs-hero"><div><p class="vs-vendor">Order #{n}</p><h1>{headline}</h1><p>{sub}</p></div></header>
{status}{pay_box}{tracking}{downloads}
<div class="vs-order">
<div class="vs-lines">{rows}</div>
<aside class="vs-summary"><dl><dt>Subtotal</dt><dd>{subt}</dd>{disc}<dt>Shipping</dt><dd>{ship}</dd>{tax}{refund}<dt class="vs-total">Total</dt><dd class="vs-total">{total}</dd></dl>
{addr_html}<p class="vs-note">Payment: {pay}</p></aside></div>
<p><a class="vs-btn vs-btn-tonal" href="{home}">Continue shopping</a></p>"#,
        n = o.number,
        headline = if q.contains_key("placed") { "Thank you!" } else { "Your order" },
        sub = esc(&format!("A confirmation goes to {}.", o.email)),
        subt = m(o.subtotal_cents),
        disc = if o.discount_cents > 0 { format!("<dt>Discount {}</dt><dd>−{}</dd>", esc(&o.discount_code), m(o.discount_cents)) } else { String::new() },
        ship = if o.shipping_cents == 0 { "Free".to_string() } else { m(o.shipping_cents) },
        tax = if o.tax_cents > 0 { format!("<dt>{}</dt><dd>{}</dd>", if o.tax_included != 0 { "Includes tax" } else { "Tax" }, m(o.tax_cents)) } else { String::new() },
        refund = if o.refunded_cents > 0 { format!("<dt>Refunded</dt><dd>−{}</dd>", m(o.refunded_cents)) } else { String::new() },
        total = m(o.total_cents),
        addr_html = if addr.is_empty() { String::new() } else { format!(r#"<h3>Shipping to</h3><p class="vs-addr">{}</p>"#, addr.lines().iter().map(|l| esc(l)).collect::<Vec<_>>().join("<br>")) },
        pay = esc(rules::payment_label(&o.payment_method)),
        home = link(BASE),
    );
    let mut res = page(&state, &headers, &path, &format!("Order #{}", o.number), "", body, None).await;
    res.headers_mut().insert(header::HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex"));
    res
}

async fn policy(State(state): State<AppState>, headers: HeaderMap, Path(which): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("/shop/policies/{which}/");
    let (title, text) = match which.as_str() {
        "refund" => ("Refund policy", s.policy_refund.clone()),
        "shipping" => ("Shipping policy", s.policy_shipping.clone()),
        _ => ("", String::new()),
    };
    if !s.enabled || text.is_empty() {
        return not_found(&state, &path, headers).await;
    }
    let body = format!(r#"<header class="vs-hero"><div><h1>{}</h1></div></header><div class="vs-desc">{}</div>"#, esc(title), rules::paragraphs(&text));
    page(&state, &headers, &path, title, "", body, None).await
}

// ---------------------------------------------------------------------------
// digital downloads
// ---------------------------------------------------------------------------

/// Sign-in links one address can be sent per hour, and per connection.
const LINK_PER_EMAIL: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 4, min_wait: 600_000, max_wait: 3_600_000, lifetime: 3600 };
const LINK_PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 10, min_wait: 600_000, max_wait: 3_600_000, lifetime: 3600 };

/// One grant's files as rows: a Download button each, or why not.
fn grant_rows(g: &repo::Grant, files: &[repo::EcomFile], unlocked: bool, now: &str) -> String {
    let allowed = if unlocked { rules::download_allowed(g.download_count, g.download_limit, g.expires_at.as_deref(), g.revoked != 0, now) } else { Err("Waiting for payment.") };
    let meta = {
        let mut m = Vec::new();
        if g.download_limit > 0 {
            m.push(format!("{} of {} downloads left", (g.download_limit - g.download_count).max(0), g.download_limit));
        }
        if let Some(e) = &g.expires_at {
            m.push(format!("available until {}", e.get(..10).unwrap_or(e)));
        }
        m.join(" · ")
    };
    let rows: String = files
        .iter()
        .map(|f| {
            let action = match allowed {
                Ok(()) => format!(r#"<a class="vs-btn" href="{}" rel="nofollow">Download</a>"#, link(&format!("/shop/downloads/{}/{}/", g.id, f.id))),
                Err(why) => format!(r#"<span class="vs-note">{}</span>"#, esc(why)),
            };
            format!(r#"<div class="vs-file"><span class="vs-file-ic" aria-hidden="true">{ext}</span><div><b>{n}</b><small>{fname} · {size}</small></div>{action}</div>"#,
                ext = esc(&f.filename.rsplit('.').next().filter(|e| e.len() <= 4 && *e != f.filename).unwrap_or("file").to_uppercase()),
                n = esc(&f.name),
                fname = esc(&f.filename),
                size = esc(&rules::format_bytes(f.size_bytes.max(0) as u64)))
        })
        .collect();
    format!(
        r#"<section class="vs-dl"><header><h3>{t}</h3>{meta}</header>{rows}</section>"#,
        t = esc(&g.title),
        meta = if meta.is_empty() { String::new() } else { format!(r#"<p class="vs-note">{}</p>"#, esc(&meta)) },
        rows = if files.is_empty() { r#"<p class="vs-note">The files for this product are being updated. Check back soon.</p>"#.to_string() } else { rows },
    )
}

/// The Downloads block on an order's status page. The page is reachable by
/// its token alone, so it never links a file to anyone but the signed-in
/// owner; everyone else is offered a sign-in link sent to the order's email.
async fn order_downloads(state: &AppState, headers: &HeaderMap, o: &repo::Order, sent: bool) -> String {
    let grants = repo::grants_for_order(&state.db, &o.id).await;
    let items = repo::order_items(&state.db, &o.id).await;
    let pids: Vec<String> = items.iter().filter_map(|i| i.product_id.clone()).collect();
    let digital = !repo::digital_ids(&state.db, &pids).await.is_empty() || !grants.is_empty();
    if !digital || o.is_cancelled() {
        return String::new();
    }
    if !rules::order_unlocks_downloads(&o.status, &o.financial_status) {
        return r#"<div class="vs-notice"><p><b>Your downloads</b></p><p>Your files unlock as soon as payment arrives. We will email you a link.</p></div>"#.to_string();
    }
    let member = vuuut_api::members::current_member(state, headers).await;
    if member.as_ref().is_some_and(|m| m.email.eq_ignore_ascii_case(&o.email)) {
        let files = repo::files_for(&state.db, &grants.iter().map(|g| g.product_id.clone()).collect::<Vec<_>>()).await;
        let now = vuuut_core::dates::now_db();
        return format!(
            r#"<div class="vs-downloads"><h2>Your downloads</h2>{}<p class="vs-note">Always available in <a href="{}">My products</a>.</p></div>"#,
            grants.iter().map(|g| grant_rows(g, files.get(&g.product_id).map(Vec::as_slice).unwrap_or(&[]), true, &now)).collect::<String>(),
            link("/shop/dashboard/#downloads"),
        );
    }
    format!(
        r#"<div class="vs-notice vs-dl-locked"><p><b>Your downloads are ready</b></p>
<p>They are kept in the member account for <b>{email}</b>. We emailed a button that signs you in and opens them.</p>{sent}
<form method="post" action="{action}"><button class="vs-btn vs-btn-tonal" type="submit">Email me a sign-in link</button></form></div>"#,
        email = esc(&o.email),
        sent = if sent { r#"<p><b>Sent.</b> Check your inbox.</p>"# } else { "" },
        action = link(&format!("/shop/orders/{}/send-link/", o.token)),
    )
}

async fn order_send_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    Path(token): Path<String>,
) -> Response {
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This request did not come from the store.").into_response();
    }
    let Some(o) = rules::is_order_token(&token).then_some(()).and(repo::order_by_token(&state.db, &token).await) else {
        return Redirect::to(&link("/shop/")).into_response();
    };
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    let ok = vuuut_api::brute::check(&state.db, &format!("shop_dl_email:{}", o.email), &LINK_PER_EMAIL).await.is_ok()
        && vuuut_api::brute::check(&state.db, &format!("shop_dl_ip:{ip}"), &LINK_PER_IP).await.is_ok();
    if ok && rules::order_unlocks_downloads(&o.status, &o.financial_status) {
        vuuut_api::ecom::send_download_email(&state, &o).await;
    }
    Redirect::to(&link(&format!("/shop/orders/{}/?sent=1", o.token))).into_response()
}

/// `/shop/downloads/` and `/shop/account/` were the first homes of what is
/// now My products; links to them (in emails already sent) land there.
async fn to_dashboard(Query(q): Query<HashMap<String, String>>) -> Response {
    let mut target = link("/shop/dashboard/");
    let kept: Vec<String> = q
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "sent" | "error" | "action" | "success"))
        .map(|(k, v)| format!("{k}={}", vuuut_core::social_media::encode(v)))
        .collect();
    if !kept.is_empty() {
        target = format!("{target}?{}", kept.join("&"));
    }
    Redirect::to(&target).into_response()
}

fn dashboard_error(code: &str) -> &'static str {
    match code {
        "limit" => "You have used all the downloads for that purchase.",
        "expired" => "That download has expired.",
        "revoked" => "That download is no longer available.",
        "unpaid" => "That order is not paid yet.",
        _ => "That download is not available.",
    }
}

/// My products. Signed out: a sign-in form, centred. Signed in: everything
/// this member has bought — a summary, a Downloads section with the files
/// (each link served only to them, see [`download_file`]) and every order,
/// shipped or digital, with its state and a link to its page.
async fn dashboard(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = "/shop/dashboard/";
    if !s.enabled {
        return not_found(&state, path, headers).await;
    }
    let error = q.get("error").map(|e| format!(r#"<div class="vs-notice vs-notice-err"><p>{}</p></div>"#, esc(dashboard_error(e)))).unwrap_or_default();
    let body = match vuuut_api::members::current_member(&state, &headers).await {
        None => format!(
            r##"<div class="vs-signin-wrap"><div class="vs-signin">
<span class="vs-signin-ic" aria-hidden="true"><svg viewBox="0 0 24 24" width="26" height="26" fill="currentColor"><path d="M18 6h-2c0-2.21-1.79-4-4-4S8 3.79 8 6H6c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm-6-2c1.1 0 2 .9 2 2h-4c0-1.1.9-2 2-2zm6 16H6V8h2v2c0 .55.45 1 1 1s1-.45 1-1V8h4v2c0 .55.45 1 1 1s1-.45 1-1V8h2v12z"/></svg></span>
<h1>My products</h1>
<p class="vs-signin-lead">Your orders and downloads live in your member account. Enter the email you bought with and we will send you a sign-in link.</p>
{sent}{error}
<form method="post" action="{action}"><label class="vs-field"><span>Email</span><input type="email" name="email" required maxlength="191" autocomplete="email" placeholder="you@example.com"></label>
<input type="text" name="website_url" value="" tabindex="-1" autocomplete="off" class="vs-hp" aria-hidden="true">
<button class="vs-btn vs-btn-big vs-wide" type="submit">Email me a sign-in link</button></form>
<p class="vs-note">Already a member? <a href="#/portal/signin" data-portal="signin">Sign in</a></p></div></div>"##,
            sent = if q.get("sent").is_some() {
                r#"<div class="vs-notice"><p><b>Check your inbox.</b> If that email has purchases or an account here, a sign-in link is on its way.</p></div>"#
            } else {
                ""
            },
            action = link("/shop/downloads/signin/"),
        ),
        Some(m) => {
            let (orders, _) = repo::browse_orders(&state.db, &repo::OrderBrowse { email: Some(&m.email), view: "all", limit: 200, ..Default::default() }).await;
            let grants = repo::grants_for_email(&state.db, &m.email).await;
            let files = repo::files_for(&state.db, &grants.iter().map(|g| g.product_id.clone()).collect::<Vec<_>>()).await;
            let by_id: HashMap<&str, &repo::Order> = orders.iter().map(|o| (o.id.as_str(), o)).collect();
            let now = vuuut_core::dates::now_db();
            let live: Vec<&repo::Order> = orders.iter().filter(|o| !o.is_cancelled()).collect();
            let spent: i64 = live.iter().filter(|o| o.is_paid()).map(|o| o.net()).sum();
            let currency = live.first().map(|o| o.currency.clone()).unwrap_or_else(|| s.currency.clone());
            let file_count: usize = grants
                .iter()
                .filter(|g| by_id.get(g.order_id.as_str()).is_some_and(|o| !o.is_cancelled()))
                .map(|g| files.get(&g.product_id).map(Vec::len).unwrap_or(0))
                .sum();

            let downloads: String = grants
                .iter()
                .filter_map(|g| by_id.get(g.order_id.as_str()).map(|o| (g, *o)))
                .filter(|(_, o)| !o.is_cancelled())
                .map(|(g, o)| {
                    format!(
                        r#"{}<p class="vs-note vs-dl-order">From <a href="{}">order #{}</a> · {}</p>"#,
                        grant_rows(g, files.get(&g.product_id).map(Vec::as_slice).unwrap_or(&[]), rules::order_unlocks_downloads(&o.status, &o.financial_status), &now),
                        link(&format!("/shop/orders/{}/", o.token)),
                        o.number,
                        esc(o.created_at.get(..10).unwrap_or(&o.created_at)),
                    )
                })
                .collect();

            let mut order_cards = String::new();
            for o in &orders {
                let items = repo::order_items(&state.db, &o.id).await;
                let m = |c: i64| esc(&rules::format_money(c, &o.currency));
                let state_chip = if o.is_cancelled() {
                    r#"<span class="vs-chip-s vs-chip-off">Cancelled</span>"#.to_string()
                } else {
                    format!(
                        r#"<span class="vs-chip-s{p}">{pl}</span><span class="vs-chip-s{f}">{fl}</span>"#,
                        p = if o.is_paid() { " vs-chip-ok" } else { " vs-chip-wait" },
                        pl = rules::financial_label(&o.financial_status),
                        f = if o.fulfillment_status == "fulfilled" { " vs-chip-ok" } else { "" },
                        fl = if items.iter().all(|i| i.requires_shipping == 0) {
                            if rules::order_unlocks_downloads(&o.status, &o.financial_status) { "Delivered" } else { "Awaiting payment" }
                        } else if o.fulfillment_status == "fulfilled" {
                            "Shipped"
                        } else {
                            "Not shipped yet"
                        },
                    )
                };
                let lines: String = items
                    .iter()
                    .map(|i| {
                        format!(
                            r#"<li><span class="vs-mini-img">{img}</span><span class="vs-mini-t"><b>{t}</b>{v}</span><span class="vs-mini-q">× {q}</span><span class="vs-mini-p">{p}</span></li>"#,
                            img = if i.image.is_empty() { r#"<span class="vs-noimg"></span>"#.to_string() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&i.image)) },
                            t = esc(&i.title),
                            v = if i.variant_title.is_empty() || i.variant_title == rules::DEFAULT_VARIANT { String::new() } else { format!("<small>{}</small>", esc(&i.variant_title)) },
                            q = i.quantity,
                            p = m(i.total_cents),
                        )
                    })
                    .collect();
                let track = if !o.tracking_url.is_empty() {
                    format!(r#"<a href="{}" rel="noopener nofollow" target="_blank">Track package</a>"#, esc(&o.tracking_url))
                } else {
                    String::new()
                };
                order_cards.push_str(&format!(
                    r#"<article class="vs-ocard{cancel}"><header><div><b>Order #{n}</b><span>{d}</span></div><div class="vs-ocard-chips">{state_chip}</div></header>
<ul class="vs-mini">{lines}</ul>
<footer><span>Total <b>{total}</b>{refund}</span><span class="vs-ocard-links">{track}<a href="{href}">View order</a></span></footer></article>"#,
                    cancel = if o.is_cancelled() { " vs-ocard-off" } else { "" },
                    n = o.number,
                    d = esc(o.created_at.get(..10).unwrap_or(&o.created_at)),
                    total = m(o.total_cents),
                    refund = if o.refunded_cents > 0 { format!(" · refunded {}", m(o.refunded_cents)) } else { String::new() },
                    href = link(&format!("/shop/orders/{}/", o.token)),
                ));
            }
            let first_name = m.name.as_deref().and_then(|n| n.split_whitespace().next()).map(str::to_string);
            format!(
                r##"<header class="vs-hero vs-dash-head"><div><h1>My products</h1><p>{hello} Signed in as {email} · <a href="#/portal/account" data-portal="account">Account</a></p></div></header>
{error}
<div class="vs-stats"><div><b>{orders}</b><span>{orders_l}</span></div><div><b>{spent}</b><span>Spent</span></div><div><b>{files}</b><span>{files_l}</span></div></div>
<section class="vs-dash-sec" id="downloads"><h2>Downloads</h2>{downloads}</section>
<section class="vs-dash-sec" id="orders"><h2>Orders</h2>{orders_html}</section>"##,
                hello = first_name.map(|n| format!("Hi {},", esc(&n))).unwrap_or_default(),
                email = esc(&m.email),
                orders = live.len(),
                orders_l = if live.len() == 1 { "Order" } else { "Orders" },
                spent = esc(&rules::format_money(spent, &currency)),
                files = file_count,
                files_l = if file_count == 1 { "File to download" } else { "Files to download" },
                downloads = if downloads.is_empty() {
                    r#"<p class="vs-empty-line">No downloadable products yet. Digital products you buy appear here, ready to download.</p>"#.to_string()
                } else {
                    format!(r#"<div class="vs-downloads">{downloads}</div>"#)
                },
                orders_html = if order_cards.is_empty() {
                    format!(r#"<div class="vs-empty-cart"><p>No orders yet.</p><a class="vs-btn" href="{}">Browse the store</a></div>"#, link(BASE))
                } else {
                    format!(r#"<div class="vs-ocards">{order_cards}</div>"#)
                },
            )
        }
    };
    let mut res = page(&state, &headers, path, "My products", "", body, None).await;
    res.headers_mut().insert(header::HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex"));
    res
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct SigninForm {
    email: String,
    website_url: String,
}

/// Email a sign-in link that opens Downloads. The answer is the same
/// whether or not the address has anything here, so it cannot be used to
/// learn who bought what; mail goes only to an address with purchases or
/// a member account.
async fn downloads_signin(
    State(state): State<AppState>,
    headers: HeaderMap,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    Form(f): Form<SigninForm>,
) -> Response {
    let done = Redirect::to(&link("/shop/dashboard/?sent=1")).into_response();
    if !settings(&state).enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This request did not come from the store.").into_response();
    }
    let email = f.email.trim().to_lowercase();
    if !f.website_url.is_empty() || !rules::is_email(&email) {
        return done;
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("shop_dl_email:{email}"), &LINK_PER_EMAIL).await.is_err()
        || vuuut_api::brute::check(&state.db, &format!("shop_dl_ip:{ip}"), &LINK_PER_IP).await.is_err()
    {
        return done;
    }
    let has_grants = !repo::grants_for_email(&state.db, &email).await.is_empty();
    let is_member: bool = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM members WHERE lower(email) = ?").bind(&email).fetch_one(&state.db).await.unwrap_or(0) > 0;
    if has_grants || is_member {
        if let Some(link) = vuuut_api::members::magic_link_for(&state, &email, "signin", &public("/shop/dashboard/")).await {
            let (bg, to) = (state.clone(), email.clone());
            tokio::spawn(async move {
                if let Err(e) = vuuut_api::members::send_signin_email(&bg, &to, &link).await {
                    tracing::warn!("shop: downloads sign-in email to {to} failed: {}", e.message());
                }
            });
        }
    }
    done
}

/// Stream one file to its owner.
///
/// Every condition is checked on every request: a signed-in member, whose
/// email is the grant's, on a paid and uncancelled order, with the grant
/// unrevoked, unexpired and under its limit, for a file of the granted
/// product. The path comes from the file's row, never from the request.
async fn download_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    Path((grant_id, file_id)): Path<(String, String)>,
) -> Response {
    let fail = |code: &str| Redirect::to(&link(&format!("/shop/dashboard/?error={code}#downloads"))).into_response();
    if !vuuut_core::ids::is_objectid(&grant_id) || !vuuut_core::ids::is_objectid(&file_id) {
        return fail("missing");
    }
    let Some(member) = vuuut_api::members::current_member(&state, &headers).await else {
        // Not signed in: to the page that offers a sign-in link.
        return Redirect::to(&link("/shop/dashboard/")).into_response();
    };
    let Some(g) = repo::find_grant(&state.db, &grant_id).await.filter(|g| g.email.eq_ignore_ascii_case(&member.email)) else {
        return fail("missing");
    };
    let Some(o) = repo::find_order(&state.db, &g.order_id).await else { return fail("missing") };
    if !rules::order_unlocks_downloads(&o.status, &o.financial_status) {
        return fail("unpaid");
    }
    if let Err(why) = rules::download_allowed(g.download_count, g.download_limit, g.expires_at.as_deref(), g.revoked != 0, &vuuut_core::dates::now_db()) {
        return fail(if why.contains("expired") { "expired" } else if why.contains("used all") { "limit" } else { "revoked" });
    }
    let Some(f) = repo::find_file(&state.db, &file_id).await.filter(|f| f.product_id == g.product_id) else {
        return fail("missing");
    };
    let Some(path) = vuuut_api::ecom::stored_path(&state, &f.stored) else { return fail("missing") };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(e) => {
            tracing::error!("shop: download {} is missing on disk: {e}", f.id);
            return fail("missing");
        }
    };
    let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !repo::record_download(&state.db, &g, &f.id, &member.id, &ip, ua).await {
        return fail("limit");
    }
    use tokio::io::AsyncReadExt;
    let stream = futures::stream::unfold(file, |mut file| async move {
        let mut buf = vec![0u8; 64 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(buf)), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    });
    let mut res = axum::body::Body::from_stream(stream).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&rules::content_disposition(&f.filename)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    h.insert(header::HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff"));
    h.insert(header::HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex"));
    res
}

// ---------------------------------------------------------------------------
// the store's public addresses
// ---------------------------------------------------------------------------

/// Outermost on the public site: translate the store's configured public
/// addresses to its internal `/shop/…` routes before anything routes, and
/// send old `/shop/…` addresses to the new ones with a 301, so links that
/// were shared before a change keep working. A no-op with the defaults.
pub(crate) async fn addresses(State(state): State<AppState>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let s = settings(&state);
    let paths = rules::Paths::from_settings(&s);
    PATHS.scope(paths.clone(), route(paths, req, next)).await
}

async fn route(paths: rules::Paths, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if paths.is_default() {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let read_only = matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
    if let Some(internal) = paths.internal_path(&path) {
        // `/themes/asahi` → `/themes/asahi/` in one hop, rather than via the
        // internal address.
        let last = path.rsplit('/').next().unwrap_or("");
        if read_only && !path.ends_with('/') && !last.contains('.') {
            return Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(&format!("{path}/")))).into_response();
        }
        if internal != path {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        }
        return next.run(req).await;
    }
    if read_only && (path == "/shop" || path.starts_with("/shop/")) {
        let target = paths.public_path(&path);
        if target != path {
            return Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(&target))).into_response();
        }
    }
    next.run(req).await
}
