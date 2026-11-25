//! The public directory: `/directory/`.
//!
//! Every screen is rendered inside the active theme as a transient page
//! (`routes::render_virtual_page`), so it carries the site's own header,
//! footer and type with no theme changes. While the directory is switched
//! off in Tools → Directory & Classified Listings, every address here is the
//! theme's 404.
//!
//! In the order a visitor meets them:
//!
//! * `GET /directory/` — browse: search, category, kind, location, pages.
//! * `GET /directory/category/{slug}/` — one category.
//! * `GET /directory/{slug}/` — one listing, with structured data (a
//!   `LocalBusiness`/`Organization` for an entry, a `Product` with an
//!   `Offer` for a classified) so search engines understand it.
//! * `GET /directory/{slug}/go/{website|email|phone}/` — counts a contact and
//!   redirects to the listing's *stored* website, mailto or tel. The target
//!   comes from the database, never from the request, so this cannot be
//!   used as an open redirect.
//! * `GET|POST /directory/submit/` — the public form, when submissions are
//!   on. Same-origin, a honeypot, a minimum fill time, a per-address budget
//!   (`brute`) and at most five listings per email a day; the listing is
//!   rebuilt by `vuuut_core::directory::ListingInput::from_form` and waits
//!   for review unless the site has turned moderation off.
//!
//! Every value from a listing is escaped where it is shown; a description
//! is plain text turned into paragraphs by `vuuut_core::directory::paragraphs`.

use crate::routes::{render_virtual_page, render_virtual_page_as};
use serde_json::{json, Value};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Form, Router,
};
use vuuut_api::AppState;
use vuuut_core::directory::{self as rules, Kind, PriceType, Settings};
use vuuut_db::repos::directory as repo;
use std::collections::HashMap;

const BASE: &str = "/directory/";
static CSS: &str = include_str!("../assets/directory.css");

fn css_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in CSS.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:x}")
    });
    &V
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/directory/", get(index))
        .route("/directory", get(|| async { Redirect::permanent(&link(BASE)) }))
        .route("/directory/assets/{v}/directory.css", get(|| async {
            ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response()
        }))
        .route("/directory/submit/", get(submit_page).post(submit))
        .route("/directory/category/{slug}/", get(category))
        .route("/directory/{slug}/", get(listing))
        .route("/directory/{slug}/go/{to}/", get(go))
}

fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

async fn not_found(state: &AppState, path: &str, headers: HeaderMap) -> Response {
    crate::render::error_page(state, StatusCode::NOT_FOUND, &public(path), headers).await
}

fn stylesheet() -> String {
    format!(r#"<link rel="stylesheet" href="{}">"#, link(&format!("/directory/assets/{}/directory.css", css_version())))
}

tokio::task_local! {
    static PATHS: rules::Paths;
}

fn public(path: &str) -> String {
    PATHS.try_with(|paths| paths.public_path(path)).unwrap_or_else(|_| path.into())
}

fn link(path: &str) -> String {
    vuuut_core::url::base(&public(path))
}

pub(crate) async fn addresses(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let paths = rules::Paths::read(|key| state.settings.get_str(key));
    PATHS.scope(paths.clone(), route_addresses(paths, req, next)).await
}

async fn route_addresses(
    paths: rules::Paths,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if paths == rules::Paths::default() {
        return next.run(req).await;
    }

    let path = req.uri().path();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let redirect = |path: &str| {
        Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(path))).into_response()
    };
    if let Some(internal) = paths.internal_path(path) {
        if !path.ends_with('/') && !path.rsplit('/').next().unwrap_or_default().contains('.') {
            return redirect(&format!("{path}/"));
        }
        if internal != path {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        }
    } else if path == BASE.trim_end_matches('/') || path.starts_with(BASE) {
        let mut target = paths.public_path(path);
        if target != path {
            if !target.ends_with('/') && !target.rsplit('/').next().unwrap_or_default().contains('.') {
                target.push('/');
            }
            return redirect(&target);
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------------------
// browse
// ---------------------------------------------------------------------------

async fn index(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    // `?category=` (or a theme search's `?what=`) picks a category here
    // rather than redirecting, so it combines with the other filters.
    let wanted = q.get("category").or_else(|| q.get("what")).map(|c| rules::slugify(c)).filter(|c| !c.is_empty());
    let cat = match wanted {
        Some(slug) => repo::category_by_slug(&state.db, &slug).await,
        None => None,
    };
    browse(state, headers, q, cat).await
}

async fn category(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}category/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    match repo::category_by_slug(&state.db, &rules::slugify(&slug)).await {
        Some(c) => browse(state, headers, q, Some(c)).await,
        None => not_found(&state, &path, headers).await,
    }
}

async fn browse(state: AppState, headers: HeaderMap, q: HashMap<String, String>, cat: Option<repo::Category>) -> Response {
    let s = settings(&state);
    let path = cat.as_ref().map(|c| format!("{BASE}category/{}/", c.slug)).unwrap_or_else(|| BASE.to_string());
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let now = vuuut_core::dates::now_db();
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let kind = match q.get("kind").map(String::as_str) {
        Some("directory") => Some("directory"),
        Some("classified") => Some("classified"),
        _ => None,
    };
    let locations = repo::locations(&state.db, &now).await;
    // A location is only one the directory already has — never free text.
    let wanted_place = q.get("location").or_else(|| q.get("where")).map(String::as_str).unwrap_or("");
    let location = locations.iter().find(|x| x.eq_ignore_ascii_case(wanted_place) || rules::slugify(x) == rules::slugify(wanted_place) && !wanted_place.is_empty()).cloned();
    let levels: Vec<i64> = q.get("price").map(|p| p.split(',').filter_map(|n| n.trim().parse().ok()).filter(|n| (1..=4).contains(n)).collect()).unwrap_or_default();
    let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).max(1);
    let (items, total) = repo::browse_public(
        &state.db,
        &repo::Browse {
            terms: &terms,
            category_id: cat.as_ref().map(|c| c.id.as_str()),
            kind,
            visible_at: Some(&now),
            location: location.as_deref(),
            price_levels: &levels,
            limit: s.per_page,
            offset: (page - 1).saturating_mul(s.per_page),
            ..Default::default()
        },
    )
    .await
    .unwrap_or_default();
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let counts: HashMap<String, i64> = repo::category_counts(&state.db, &now).await.into_iter().collect();
    let by_id: HashMap<&str, &repo::Category> = categories.iter().map(|c| (c.id.as_str(), c)).collect();

    let chips: String = std::iter::once(format!(
        r#"<a class="vd-chip{on}" href="{href}">All</a>"#,
        on = if cat.is_none() { " on" } else { "" },
        href = link(BASE),
    ))
    .chain(categories.iter().filter(|c| counts.get(&c.id).copied().unwrap_or(0) > 0).map(|c| {
        format!(
            r#"<a class="vd-chip{on}" href="{href}">{name} <span>{n}</span></a>"#,
            on = if cat.as_ref().is_some_and(|x| x.id == c.id) { " on" } else { "" },
            href = link(&format!("{BASE}category/{}/", c.slug)),
            name = esc(&c.name),
            n = counts.get(&c.id).copied().unwrap_or(0),
        )
    }))
    .collect();

    let kind_opts = [("", "Everything"), ("directory", "Directory"), ("classified", "Classifieds")]
        .iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if kind.unwrap_or("") == *v { " selected" } else { "" }))
        .collect::<String>();
    let loc_opts: String = std::iter::once(r#"<option value="">Anywhere</option>"#.to_string())
        .chain(locations.iter().map(|l| format!(r#"<option value="{v}"{s}>{v}</option>"#, v = esc(l), s = if location.as_deref() == Some(l.as_str()) { " selected" } else { "" })))
        .collect();

    let cards: String = items.iter().map(|l| card(&s, l, &by_id)).collect();
    let pages = total / s.per_page + i64::from(total % s.per_page != 0);
    let page_link = |n: i64| {
        let mut params = vec![format!("page={n}")];
        if !query.is_empty() {
            params.push(format!("q={}", vuuut_core::social_media::encode(&query)));
        }
        if let Some(k) = kind {
            params.push(format!("kind={k}"));
        }
        if let Some(l) = &location {
            params.push(format!("location={}", vuuut_core::social_media::encode(l)));
        }
        if !levels.is_empty() {
            params.push(format!("price={}", levels.iter().map(i64::to_string).collect::<Vec<_>>().join(",")));
        }
        format!("{}?{}", link(&path), params.join("&amp;"))
    };
    let pager = if pages > 1 {
        format!(
            r#"<nav class="vd-pager">{prev}<span>Page {page} of {pages}</span>{next}</nav>"#,
            prev = if page > 1 { format!(r#"<a href="{}">← Newer</a>"#, page_link(page - 1)) } else { String::new() },
            next = if page < pages { format!(r#"<a href="{}">Older →</a>"#, page_link(page + 1)) } else { String::new() },
        )
    } else {
        String::new()
    };
    let title = match &cat {
        Some(c) => format!("{} — {}", c.name, s.title),
        None => s.title.clone(),
    };

    // The theme's `directory/category.hbs` (falling back to
    // `directory/directory.hbs`) or `directory/directory.hbs`: `directory`,
    // `listings`, `categories` (each with `current`), `places` (each with
    // `current`), `category`, `query`, `kind`, `location`, `price`,
    // `heading`, `intro`, `total` and `pagination` (`page`, `pages`,
    // `prev_url`, `next_url`).
    let names: &[&str] = if cat.is_some() { &["directory/category", "directory/directory"] } else { &["directory/directory"] };
    let templates = theme_templates(&state, names).await;
    if !templates.is_empty() {
        let places = repo::places(&state.db, &now).await;
        let mut data = serde_json::Map::new();
        data.insert("directory".into(), directory_json(&s));
        data.insert("listings".into(), Value::Array(items.iter().map(|l| repo::listing_json(&s, l, &categories)).collect()));
        data.insert("categories".into(), Value::Array(categories.iter().map(|c| {
            let mut j = repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0));
            j["current"] = json!(cat.as_ref().is_some_and(|x| x.id == c.id));
            j
        }).collect()));
        data.insert("places".into(), Value::Array(places.iter().map(|(name, n, img)| {
            let mut j = repo::place_json(&s, name, *n, Some(img));
            j["current"] = json!(location.as_deref() == Some(name.as_str()));
            j
        }).collect()));
        data.insert("category".into(), cat.as_ref().map(|c| repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0))).unwrap_or(Value::Null));
        data.insert("query".into(), json!(query));
        data.insert("kind".into(), json!(kind.unwrap_or("")));
        data.insert("location".into(), json!(location.clone().unwrap_or_default()));
        data.insert("price".into(), json!(levels.iter().map(i64::to_string).collect::<Vec<_>>().join(",")));
        data.insert("heading".into(), json!(cat.as_ref().map(|c| c.name.clone()).or_else(|| location.as_ref().map(|l| format!("Places in {l}"))).unwrap_or_else(|| s.title.clone())));
        data.insert("intro".into(), json!(cat.as_ref().map(|c| c.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| s.intro.clone())));
        data.insert("total".into(), json!(total));
        data.insert("pagination".into(), json!({
            "page": page,
            "pages": pages.max(1),
            "prev_url": if page > 1 { json!(page_link(page - 1).replace("&amp;", "&")) } else { Value::Null },
            "next_url": if page < pages { json!(page_link(page + 1).replace("&amp;", "&")) } else { Value::Null },
        }));
        return render_virtual_page_as(&state, &headers, &public(&path), &title, &s.intro, String::new(), None, false, &templates, data, &["directory"]).await;
    }

    let body = format!(
        r#"{css}<div class="vd">
{intro}
<form class="vd-search" method="get" action="{action}" role="search">
<input type="search" name="q" value="{q}" placeholder="Search listings" aria-label="Search listings" maxlength="100">
<select name="kind" aria-label="Kind">{kind_opts}</select>
<select name="location" aria-label="Location">{loc_opts}</select>
<button type="submit">Search</button>
</form>
<nav class="vd-chips" aria-label="Categories">{chips}</nav>
<p class="vd-count">{count}{submit}</p>
<div class="vd-grid">{cards}</div>
{empty}
{pager}
</div>"#,
        css = stylesheet(),
        intro = cat.as_ref().map(|c| c.description.clone()).filter(|d| !d.is_empty()).or_else(|| (!s.intro.is_empty() && cat.is_none()).then(|| s.intro.clone())).map(|t| format!(r#"<p class="vd-intro">{}</p>"#, esc(&t))).unwrap_or_default(),
        action = link(&path),
        q = esc(&query),
        count = if total == 1 { "1 listing".to_string() } else { format!("{total} listings") },
        submit = if s.submissions { format!(r#" · <a href="{}">Add a listing</a>"#, link(&format!("{BASE}submit/"))) } else { String::new() },
        empty = if items.is_empty() { r#"<p class="vd-empty">Nothing matches. Try fewer words or another category.</p>"#.to_string() } else { String::new() },
    );
    render_virtual_page(&state, &headers, &public(&path), &title, &s.intro, body, None).await
}

fn card(s: &Settings, l: &repo::Listing, cats: &HashMap<&str, &repo::Category>) -> String {
    let price = rules::format_price(l.price_cents, &s.currency, PriceType::from_key(&l.price_type));
    let cat = l.category_id.as_deref().and_then(|c| cats.get(c));
    format!(
        r#"<a class="vd-card{feat}" href="{href}">{img}<div class="vd-card-body">
<div class="vd-card-meta">{badge}{cat}{loc}</div>
<h2>{title}</h2><p>{summary}</p>{price}</div></a>"#,
        feat = if l.is_featured() { " vd-featured" } else { "" },
        href = link(&format!("{BASE}{}/", l.slug)),
        img = if l.image.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&l.image)) },
        badge = if l.is_featured() { r#"<span class="vd-badge">Featured</span>"#.to_string() } else if l.kind == "classified" { r#"<span class="vd-badge vd-badge-c">Classified</span>"#.to_string() } else { String::new() },
        cat = cat.map(|c| format!(r#"<span>{}</span>"#, esc(&c.name))).unwrap_or_default(),
        loc = if l.location.is_empty() { String::new() } else { format!(r#"<span>{}</span>"#, esc(&l.location)) },
        title = esc(&l.title),
        summary = esc(if l.summary.is_empty() { &l.description } else { &l.summary }.chars().take(160).collect::<String>().as_str()),
        price = price.map(|p| format!(r#"<span class="vd-price">{}</span>"#, esc(&p))).unwrap_or_default(),
    )
}

/// Those of `names` the active theme has, in order.
async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

/// `directory`: what every directory template can use.
fn directory_json(s: &Settings) -> Value {
    json!({
        "title": s.title,
        "intro": s.intro,
        "url": link(BASE),
        "submit_url": if s.submissions { json!(link(&format!("{BASE}submit/"))) } else { Value::Null },
        "show_contact": s.show_contact,
        "currency": s.currency,
        "style_url": link(&format!("/directory/assets/{}/directory.css", css_version())),
    })
}

pub(crate) async fn builder_cards(state: &AppState, ids: &[String]) -> String {
    let s = settings(state);
    if !s.enabled {
        return String::new();
    }
    let now = vuuut_core::dates::now_db();
    let listings: HashMap<_, _> = repo::visible_by_ids(&state.db, ids, &now).await
        .into_iter().map(|listing| (listing.id.clone(), listing)).collect();
    if listings.is_empty() {
        return String::new();
    }
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let cats = categories.iter().map(|c| (c.id.as_str(), c)).collect();
    PATHS.scope(s.paths.clone(), async {
        let cards: String = ids.iter().filter_map(|id| listings.get(id))
            .map(|listing| card(&s, listing, &cats)).collect();
        format!(r#"{}<div class="vd"><div class="vb-catalogue">{cards}</div></div>"#, stylesheet())
    }).await
}

// ---------------------------------------------------------------------------
// one listing
// ---------------------------------------------------------------------------

async fn visible_listing(state: &AppState, slug: &str) -> Option<repo::Listing> {
    let l = repo::by_slug(&state.db, &rules::slugify(slug)).await.ok().flatten()?;
    rules::visible(&l.status, l.expires_at.as_deref(), &vuuut_core::dates::now_db()).then_some(l)
}

async fn listing(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(l) = visible_listing(&state, &slug).await else {
        return not_found(&state, &path, headers).await;
    };
    let agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !["bot", "crawler", "spider", "preview"].iter().any(|b| agent.contains(b)) {
        repo::add_view(&state.db, &l.id).await;
    }
    let cats = repo::categories(&state.db).await.unwrap_or_default();
    let cat = l.category_id.as_deref().and_then(|c| cats.iter().find(|x| x.id == c));
    let absolute = vuuut_core::url::absolute(&state.site_url(), &link(&path));
    let schema = structured_data(&s, &l, &absolute);
    let excerpt = if !l.seo_description.is_empty() { l.seo_description.clone() } else if l.summary.is_empty() { l.description.chars().take(160).collect() } else { l.summary.clone() };
    let page_title = if l.seo_title.is_empty() { l.title.clone() } else { l.seo_title.clone() };
    let j = repo::listing_json(&s, &l, &cats);

    // Nearby: other listings in the same town, else the same category.
    let now = vuuut_core::dates::now_db();
    let mut nearby = Vec::new();
    if !l.location.is_empty() {
        nearby = repo::browse_public(&state.db, &repo::Browse { visible_at: Some(&now), location: Some(&l.location), limit: 5, ..Default::default() }).await.map(|r| r.0).unwrap_or_default();
    }
    if nearby.len() < 2 {
        if let Some(c) = &l.category_id {
            nearby = repo::browse_public(&state.db, &repo::Browse { visible_at: Some(&now), category_id: Some(c), limit: 5, ..Default::default() }).await.map(|r| r.0).unwrap_or_default();
        }
    }
    nearby.retain(|x| x.id != l.id);
    nearby.truncate(4);

    // The theme's `directory/listing.hbs`: `directory`, `listing` (see
    // `vuuut_db::repos::directory::listing_json`, plus `schema`, the
    // JSON-LD) and `nearby`.
    let templates = theme_templates(&state, &["directory/listing"]).await;
    if !templates.is_empty() {
        let mut listing = j.clone();
        listing["schema"] = json!(schema);
        let mut data = serde_json::Map::new();
        data.insert("directory".into(), directory_json(&s));
        data.insert("listing".into(), listing);
        data.insert("nearby".into(), Value::Array(nearby.iter().map(|x| repo::listing_json(&s, x, &cats)).collect()));
        return render_virtual_page_as(&state, &headers, &public(&path), &page_title, &excerpt, String::new(), l.images().first().cloned(), false, &templates, data, &["directory"]).await;
    }

    let str_of = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut contact = Vec::new();
    if !l.website.is_empty() {
        contact.push(format!(r#"<a class="vd-btn" href="{}" rel="nofollow noopener" target="_blank">Visit website</a>"#, esc(&str_of("website_url"))));
    }
    if !l.booking_url.is_empty() {
        contact.push(format!(r#"<a class="vd-btn" href="{}" rel="nofollow noopener" target="_blank">Book</a>"#, esc(&l.booking_url)));
    }
    if s.show_contact && !l.email.is_empty() {
        contact.push(format!(r#"<a class="vd-btn vd-btn-tonal" href="{}">Email</a>"#, esc(&str_of("email_url"))));
    }
    if s.show_contact && !l.phone.is_empty() {
        contact.push(format!(r#"<a class="vd-btn vd-btn-tonal" href="{}">Call {}</a>"#, esc(&str_of("phone_url")), esc(&l.phone)));
    }
    if let Some(d) = j.get("directions_url").and_then(Value::as_str) {
        contact.push(format!(r#"<a class="vd-btn vd-btn-tonal" href="{}" rel="noopener" target="_blank">Directions</a>"#, esc(d)));
    }
    let images = l.images();
    let gallery = if images.len() > 1 {
        format!(r#"<div class="vd-gallery">{}</div>"#, images.iter().skip(1).map(|u| format!(r#"<a href="{u}" target="_blank" rel="noopener"><img src="{u}" alt="" loading="lazy"></a>"#, u = esc(u))).collect::<String>())
    } else {
        String::new()
    };
    let hours = l.hours();
    let hours_html = if hours.is_empty() {
        String::new()
    } else {
        format!(
            r#"<section class="vd-box"><h2>Opening hours</h2><dl class="vd-hours">{}</dl>{}</section>"#,
            rules::DAYS_OF_WEEK.iter().enumerate().filter(|(i, _)| hours.0[*i] != rules::Day::Unknown).map(|(i, (_, n, _))| format!("<dt>{n}</dt><dd>{}</dd>", esc(&hours.0[i].label()))).collect::<String>(),
            if l.hours_note.is_empty() { String::new() } else { format!(r#"<p class="vd-note">{}</p>"#, esc(&l.hours_note)) },
        )
    };
    let chips = |key: &str, title: &str| -> String {
        let list = j.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        if list.is_empty() {
            return String::new();
        }
        format!(r#"<section class="vd-box"><h2>{title}</h2><ul class="vd-tags">{}</ul></section>"#, list.iter().map(|v| format!("<li>{}</li>", esc(v["label"].as_str().unwrap_or("")))).collect::<String>())
    };
    let socials: String = l.social_list().iter().map(|(k, u)| format!(r#"<a href="{}" rel="nofollow noopener" target="_blank">{}</a>"#, esc(u), esc(rules::SOCIALS.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or(k)))).collect();
    let map = j.get("map_embed_url").and_then(Value::as_str).map(|u| format!(r#"<iframe class="vd-map" title="Map" loading="lazy" src="{}"></iframe>"#, esc(u))).unwrap_or_default();
    let price_line = [j.get("price_signs").and_then(Value::as_str).map(str::to_string), j.get("price").and_then(Value::as_str).map(str::to_string)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let facts = [
        ("Category", cat.map(|c| c.name.clone()).unwrap_or_default()),
        ("Address", l.full_address()),
        ("Phone", if s.show_contact { l.phone.clone() } else { String::new() }),
        ("Website", rules::host_of(&l.website)),
        ("Established", l.established.clone()),
    ]
    .iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| format!("<dt>{k}</dt><dd>{}</dd>", esc(v)))
    .collect::<String>();
    let nearby_html = if nearby.is_empty() {
        String::new()
    } else {
        let by_id: HashMap<&str, &repo::Category> = cats.iter().map(|c| (c.id.as_str(), c)).collect();
        format!(r#"<section class="vd-nearby"><h2>{}</h2><div class="vd-grid">{}</div></section>"#, if l.location.is_empty() { "More like this".to_string() } else { format!("More in {}", esc(&l.location)) }, nearby.iter().map(|x| card(&s, x, &by_id)).collect::<String>())
    };
    let body = format!(
        r#"{css}<div class="vd vd-one">
<p class="vd-crumbs"><a href="{dir}">{dir_title}</a>{cat_crumb}</p>
<div class="vd-one-head">{logo}{badge}{summary}{price}</div>
<div class="vd-contact">{contact}</div>
<div class="vd-one-grid"><div class="vd-desc">{desc}{gallery}{video}</div>
<aside><dl class="vd-facts">{facts}</dl>{map}{hours_html}{amenities}{payments}{socials}</aside></div>
{expires}{nearby_html}
<script type="application/ld+json">{schema}</script>
</div>"#,
        css = stylesheet(),
        dir = link(BASE),
        dir_title = esc(&s.title),
        cat_crumb = cat.map(|c| format!(r#" › <a href="{}">{}</a>"#, link(&format!("{BASE}category/{}/", c.slug)), esc(&c.name))).unwrap_or_default(),
        logo = if l.logo.is_empty() { String::new() } else { format!(r#"<img class="vd-logo" src="{}" alt="">"#, esc(&l.logo)) },
        badge = if l.kind == "classified" { r#"<span class="vd-badge vd-badge-c">Classified</span>"# } else { "" },
        summary = if l.summary.is_empty() { String::new() } else { format!(r#"<p class="vd-tagline">{}</p>"#, esc(&l.summary)) },
        price = if price_line.is_empty() { String::new() } else { format!(r#"<span class="vd-price vd-price-lg">{}</span>"#, esc(&price_line)) },
        desc = l.description_markup(),
        video = if l.video_url.is_empty() { String::new() } else { format!(r#"<p><a href="{}" rel="noopener" target="_blank">Watch the video →</a></p>"#, esc(&l.video_url)) },
        amenities = chips("amenities", "Amenities"),
        payments = chips("payments", "Payment"),
        socials = if socials.is_empty() { String::new() } else { format!(r#"<section class="vd-box"><h2>Follow</h2><p class="vd-socials">{socials}</p></section>"#) },
        contact = contact.join(""),
        expires = l.expires_at.as_deref().map(|e| format!(r#"<p class="vd-expires">Listed until {}</p>"#, esc(e.get(..10).unwrap_or(e)))).unwrap_or_default(),
    );
    render_virtual_page(&state, &headers, &public(&path), &page_title, &excerpt, body, images.first().cloned()).await
}

/// schema.org for one listing, as JSON — escaped for a `<script>` by turning
/// every `</` into `<\/`.
fn structured_data(s: &Settings, l: &repo::Listing, url: &str) -> String {
    let mut v = if l.kind == "classified" {
        let mut p = serde_json::json!({ "@context": "https://schema.org", "@type": "Product", "name": l.title, "url": url, "description": l.summary });
        let t = PriceType::from_key(&l.price_type);
        if matches!(t, PriceType::Fixed | PriceType::Negotiable | PriceType::Free) {
            p["offers"] = serde_json::json!({
                "@type": "Offer",
                "price": format!("{}.{:02}", l.price_cents / 100, l.price_cents % 100),
                "priceCurrency": s.currency,
                "availability": "https://schema.org/InStock",
                "url": url,
            });
        }
        p
    } else {
        let kind = if l.address.is_empty() { "Organization" } else { "LocalBusiness" };
        let mut o = serde_json::json!({ "@context": "https://schema.org", "@type": kind, "name": l.title, "url": url, "description": l.summary });
        if !l.address.is_empty() {
            o["address"] = serde_json::json!({ "@type": "PostalAddress", "streetAddress": l.address, "addressLocality": l.location });
        }
        if !l.website.is_empty() {
            o["sameAs"] = serde_json::json!([l.website]);
        }
        if s.show_contact && !l.phone.is_empty() {
            o["telephone"] = serde_json::json!(l.phone);
        }
        if !l.address.is_empty() {
            o["address"]["addressRegion"] = json!(l.region);
            o["address"]["postalCode"] = json!(l.postal_code);
            o["address"]["addressCountry"] = json!(l.country);
        }
        if let (Ok(lat), Ok(lng)) = (l.latitude.parse::<f64>(), l.longitude.parse::<f64>()) {
            o["geo"] = json!({ "@type": "GeoCoordinates", "latitude": lat, "longitude": lng });
        }
        if let Some((_, signs, _)) = rules::PRICE_LEVELS.iter().find(|(n, _, _)| *n == l.price_level) {
            o["priceRange"] = json!(signs);
        }
        let hours = l.hours();
        let spec: Vec<Value> = rules::DAYS_OF_WEEK.iter().enumerate().flat_map(|(i, (_, name, _))| match &hours.0[i] {
            rules::Day::Open(r) => r.iter().map(|(a, b)| json!({ "@type": "OpeningHoursSpecification", "dayOfWeek": name, "opens": format!("{:02}:{:02}", a / 60, a % 60), "closes": format!("{:02}:{:02}", (b / 60) % 24, b % 60) })).collect::<Vec<_>>(),
            rules::Day::AllDay => vec![json!({ "@type": "OpeningHoursSpecification", "dayOfWeek": name, "opens": "00:00", "closes": "23:59" })],
            _ => Vec::new(),
        }).collect();
        if !spec.is_empty() {
            o["openingHoursSpecification"] = json!(spec);
        }
        let mut same: Vec<String> = if l.website.is_empty() { Vec::new() } else { vec![l.website.clone()] };
        same.extend(l.social_list().into_iter().map(|(_, u)| u));
        if !same.is_empty() {
            o["sameAs"] = json!(same);
        }
        if !l.logo.is_empty() {
            o["logo"] = json!(l.logo);
        }
        o
    };
    let images = l.images();
    if !images.is_empty() {
        v["image"] = serde_json::json!(images);
    }
    v.to_string().replace("</", "<\\/")
}

async fn go(State(state): State<AppState>, headers: HeaderMap, Path((slug, to)): Path<(String, String)>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/go/{to}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(l) = visible_listing(&state, &slug).await else {
        return not_found(&state, &path, headers).await;
    };
    // The target is the listing's own stored value — never anything from
    // this request — so this cannot send a visitor somewhere arbitrary.
    let target = match to.as_str() {
        "website" if !l.website.is_empty() => l.website.clone(),
        "email" if s.show_contact && !l.email.is_empty() => format!("mailto:{}", l.email),
        "phone" if s.show_contact && !l.phone.is_empty() => format!("tel:{}", l.phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect::<String>()),
        _ => return not_found(&state, &path, headers).await,
    };
    repo::add_contact_click(&state.db, &l.id).await;
    (StatusCode::FOUND, [(header::LOCATION, target), (header::CACHE_CONTROL, "no-store".into())]).into_response()
}

// ---------------------------------------------------------------------------
// submissions
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

async fn submit_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}submit/");
    if !s.enabled || !s.submissions {
        return not_found(&state, &path, headers).await;
    }
    let member = crate::render::member_context(&state, &headers).await;
    let signed_in = !member.is_null();
    if q.get("sent").is_some() {
        let message = if s.moderate {
            "Thank you. Your listing has been sent for review — we will email you when it is live."
        } else {
            "Thank you. Your listing is live."
        };
        let body = format!(r#"{css}<div class="vd"><p class="vd-done">{message}</p><p><a href="{dir}">Back to the directory</a></p></div>"#, css = stylesheet(), dir = link(BASE));
        return render_virtual_page(&state, &headers, &public(&path), "Listing received", "", body, None).await;
    }
    if s.members_only && !signed_in {
        let body = format!(
            r##"{css}<div class="vd"><p class="vd-done">Listings can be added by members. <a href="#/portal/signin" data-portal="signin">Sign in</a> or <a href="#/portal/signup" data-portal="signup">become a member</a> to add one.</p></div>"##,
            css = stylesheet()
        );
        return render_virtual_page(&state, &headers, &public(&path), "Add a listing", "", body, None).await;
    }
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let cat_opts: String = categories.iter().map(|c| format!(r#"<option value="{}">{}</option>"#, esc(&c.id), esc(&c.name))).collect();
    let price_opts: String = PriceType::ALL.iter().filter(|p| **p != PriceType::None).map(|p| format!(r#"<option value="{}">{}</option>"#, p.key(), p.label())).collect();
    let email = member.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let name = member.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let error = q.get("error").map(|e| format!(r#"<p class="vd-error" role="alert">{}</p>"#, esc(&e.chars().take(200).collect::<String>()))).unwrap_or_default();
    let body = format!(
        r#"{css}<div class="vd">
{error}
<form class="vd-form" method="post" action="{action}">
<input type="hidden" name="t" value="{t}">
<div class="vd-hp" aria-hidden="true"><label>Leave this empty <input name="website_url" tabindex="-1" autocomplete="off"></label></div>
<fieldset><legend>What is it?</legend>
<label class="vd-radio"><input type="radio" name="kind" value="directory" checked> A business, place or service to list</label>
<label class="vd-radio"><input type="radio" name="kind" value="classified"> Something for sale, wanted, or a job</label></fieldset>
<label>Title<input name="title" required minlength="3" maxlength="{mt}"></label>
<label>Category<select name="category_id" required><option value="">Choose…</option>{cat_opts}</select></label>
<label>In one line<input name="summary" maxlength="{ms}"></label>
<label>Description<textarea name="description" rows="6" required minlength="20" maxlength="{md}"></textarea></label>
<div class="vd-row"><label>Price type<select name="price_type">{price_opts}</select></label><label>Price<input name="price" inputmode="decimal" placeholder="Only for classifieds"></label></div>
<div class="vd-row"><label>Location<input name="location" maxlength="80" placeholder="Town or area"></label><label>Address<input name="address" maxlength="120"></label></div>
<label>Website<input name="website" type="url" maxlength="500" placeholder="https://"></label>
<label>Image address<input name="image" type="url" maxlength="500" placeholder="https://"></label>
<div class="vd-row"><label>Contact email (shown on the listing)<input name="email" type="email" maxlength="191"></label><label>Phone<input name="phone" maxlength="40"></label></div>
<fieldset><legend>About you</legend><p class="vd-note">Only the site's staff see this.</p>
<div class="vd-row"><label>Your name<input name="submitter_name" value="{name}" maxlength="80"></label><label>Your email<input name="submitter_email" type="email" required value="{email}" maxlength="191"></label></div></fieldset>
<button type="submit" class="vd-btn">{cta}</button>
</form></div>"#,
        css = stylesheet(),
        action = link(&path),
        t = now_ms(),
        mt = rules::MAX_TITLE,
        ms = rules::MAX_SUMMARY,
        md = rules::MAX_DESCRIPTION,
        name = esc(name),
        email = esc(email),
        cta = if s.moderate { "Send for review" } else { "Publish listing" },
    );
    render_virtual_page(&state, &headers, &public(&path), "Add a listing", "", body, None).await
}

const PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 5, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };

async fn submit(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}submit/");
    if !s.enabled || !s.submissions {
        return not_found(&state, &path, headers).await;
    }
    let back = |msg: &str| Redirect::to(&format!("{}?error={}", link(&path), vuuut_core::social_media::encode(msg))).into_response();
    if !vuuut_api::forms::same_origin(&headers, &state.site_url()) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let member = crate::render::member_context(&state, &headers).await;
    if s.members_only && member.is_null() {
        return back("Sign in as a member to add a listing.");
    }
    let done = Redirect::to(&format!("{}?sent=1", link(&path))).into_response();
    // A bot that fills the hidden field, or a form sent faster than a person
    // can type, gets the same thank-you a person gets and nothing is stored.
    if form.get("website_url").is_some_and(|v| !v.is_empty()) {
        return done;
    }
    let started = form.get("t").and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    if now_ms() - started < rules::MIN_FILL_MS {
        return done;
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("directory_submit:{ip}"), &PER_IP).await.is_err() {
        return back("Too many listings from here. Please wait a little.");
    }
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let input = match rules::ListingInput::from_form(&form, false, |id| categories.iter().any(|c| c.id == id)) {
        Ok(i) => i,
        Err(m) => return back(&m),
    };
    if vuuut_core::currency::fixed_price_to_minor(input.price_cents, &s.currency).is_none() {
        return back("Enter a whole price for this currency.");
    }
    let day_ago = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(1)));
    if repo::submitted_since(&state.db, &input.submitter_email, &day_ago).await >= 5 {
        return back("That address has sent five listings today. Please try again tomorrow.");
    }
    let status = if s.moderate { "pending" } else { "published" };
    let expires = (status == "published" && input.kind == Kind::Classified.key())
        .then(|| vuuut_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(s.classified_days))));
    let member_id = member.get("uuid").or_else(|| member.get("id")).and_then(|v| v.as_str()).map(str::to_string);
    match repo::insert(&state.db, &input, status, expires.as_deref(), member_id.as_deref()).await {
        Ok(_) => done,
        Err(e) => {
            tracing::error!("directory: storing a submission failed: {}", e.message());
            back("That could not be saved. Please try again.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_data_cannot_close_its_script() {
        let l = repo::Listing {
            id: "x".into(), slug: "s".into(), title: "</script><script>alert(1)</script>".into(), kind: "classified".into(),
            status: "published".into(), category_id: None, summary: "".into(), description: "".into(), image: "".into(),
            website: "".into(), email: "".into(), phone: "".into(), address: "".into(), location: "".into(), price_cents: 2500,
            price_type: "fixed".into(), keywords: "".into(), featured: 0, submitter_name: "".into(), submitter_email: "".into(),
            member_id: None, reject_reason: "".into(), views: 0, contact_clicks: 0, expires_at: None, published_at: None,
            created_at: "".into(), updated_at: "".into(), ..Default::default()
        };
        let json = structured_data(&Settings::default(), &l, "https://e.x/directory/s/");
        assert!(!json.contains("</script>"));
        assert!(json.contains(r#""price":"25.00""#) && json.contains(r#""@type":"Product""#));
    }
}
