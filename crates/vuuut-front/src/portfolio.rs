//! The public portfolio pages: `/portfolio/` (or wherever Settings moves
//! them).
//!
//! Rendered inside the active theme like the store, the directory and
//! booking. A theme can take over any screen — `portfolio/index.hbs`,
//! `portfolio/project.hbs`, `portfolio/category.hbs`, `portfolio/services.hbs`,
//! `portfolio/service.hbs`, `portfolio/about.hbs`, `portfolio/resume.hbs`,
//! `portfolio/member.hbs`, `portfolio/contact.hbs` — and is handed the same
//! data plus `forms.contact` / `forms.unlock`, ready-made forms that carry
//! the checks. While the tool is off every address here is the theme's 404.
//!
//! * `GET /portfolio/` — who (name, headline, availability), featured work,
//!   every listed project with discipline filters, services, clients and
//!   testimonials.
//! * `GET /portfolio/category/{slug}/` — one discipline.
//! * `GET /portfolio/{slug}/` — a case study. A protected project shows its
//!   cover and title and a password form; nothing else of it is in the page
//!   (or the theme's data) until `POST /portfolio/{slug}/unlock/` succeeds,
//!   which sets a signed, HttpOnly cookie for that project alone.
//! * `GET /portfolio/services/`, `/portfolio/services/{slug}/`.
//! * `GET /portfolio/about/` — the person or the studio: about, team or
//!   experience, skills, clients, awards and press.
//! * `GET /portfolio/resume/` — a printable CV.
//! * `GET /portfolio/team/{slug}/` — one person on the team.
//! * `GET|POST /portfolio/contact/` — "hire me" / a project brief:
//!   same-origin, a honeypot, a fill time, a per-address budget and three a
//!   day per email.

use crate::routes::render_virtual_page_as;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_api::AppState;
use vuuut_core::portfolio::{self as rules, Settings};
use vuuut_db::repos::portfolio as repo;

const BASE: &str = "/portfolio/";
static CSS: &str = include_str!("../assets/portfolio.css");
static JS: &str = include_str!("../assets/portfolio.js");

fn version(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}
fn css_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| version(CSS));
    &V
}
fn js_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| version(JS));
    &V
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/portfolio/", get(index))
        .route("/portfolio", get(|| async { Redirect::permanent(&link(BASE)) }))
        .route("/portfolio/assets/{v}/portfolio.css", get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response() }))
        .route("/portfolio/assets/{v}/portfolio.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], JS).into_response() }))
        .route("/portfolio/category/{slug}/", get(category_page))
        .route("/portfolio/services/", get(services_page))
        .route("/portfolio/services/{slug}/", get(service_page))
        .route("/portfolio/about/", get(about_page))
        .route("/portfolio/resume/", get(resume_page))
        .route("/portfolio/team/{slug}/", get(member_page))
        .route("/portfolio/contact/", get(contact_page).post(contact_submit))
        .route("/portfolio/{slug}/", get(project_page))
        .route("/portfolio/{slug}/unlock/", post(unlock))
}

fn settings(state: &AppState) -> Settings {
    vuuut_api::portfolio::settings(state)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

tokio::task_local! {
    static PATHS: rules::Paths;
}

fn public(path: &str) -> String {
    PATHS.try_with(|p| p.public_path(path)).unwrap_or_else(|_| path.into())
}

fn link(path: &str) -> String {
    vuuut_core::url::base(&public(path))
}

/// Moves the portfolio to the address chosen in Settings.
pub(crate) async fn addresses(State(state): State<AppState>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let paths = rules::Paths::read(|k| state.settings.get_str(k));
    if paths != rules::Paths::default() {
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        if let Some(internal) = paths.internal_path(&path) {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        } else if (path == "/portfolio" || path.starts_with(BASE)) && req.method() == axum::http::Method::GET {
            return Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(&paths.public_path(&path)))).into_response();
        }
    }
    PATHS.scope(paths, next.run(req)).await
}

async fn not_found(state: &AppState, path: &str, headers: HeaderMap) -> Response {
    crate::render::error_page(state, StatusCode::NOT_FOUND, &public(path), headers).await
}

fn style_url() -> String {
    link(&format!("/portfolio/assets/{}/portfolio.css", css_version()))
}

fn script_url() -> String {
    link(&format!("/portfolio/assets/{}/portfolio.js", js_version()))
}

fn assets() -> String {
    format!(r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script>"#, style_url(), script_url())
}

async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

fn name_of(state: &AppState, s: &Settings) -> String {
    if s.name.is_empty() { state.settings.get_str("title").unwrap_or_default() } else { s.name.clone() }
}

fn portfolio_json(state: &AppState, s: &Settings) -> Value {
    json!({
        "title": s.title, "intro": s.intro, "name": name_of(state, s), "headline": s.headline, "about": s.about,
        "about_html": vuuut_core::directory::paragraphs(&s.about), "portrait": if s.portrait.is_empty() { Value::Null } else { json!(s.portrait) },
        "location": s.location, "mode": s.mode.key(), "studio": s.studio(), "personal": !s.studio(),
        "availability": s.availability.key(), "available": s.availability == rules::Availability::Available,
        "availability_label": if s.availability == rules::Availability::Hidden { Value::Null } else { json!(s.availability.label()) },
        "availability_note": s.availability_note,
        "email": if s.email.is_empty() { Value::Null } else { json!(s.email) }, "phone": if s.phone.is_empty() { Value::Null } else { json!(s.phone) },
        "resume_url": if s.resume_url.is_empty() { Value::Null } else { json!(s.resume_url) }, "founded": if s.founded > 0 { json!(s.founded) } else { Value::Null },
        "socials": s.socials.iter().map(|(k, u)| json!({ "key": k, "label": rules::social_label(k), "url": u })).collect::<Vec<_>>(),
        "url": link(BASE), "about_url": link("/portfolio/about/"), "resume_page_url": link("/portfolio/resume/"), "services_url": link("/portfolio/services/"),
        "contact_url": if s.enquiries { json!(link("/portfolio/contact/")) } else { Value::Null },
        "style_url": style_url(), "script_url": script_url(),
    })
}

/// `portfolio` on every page while the tool is on (the profile and the
/// addresses), so a theme's homepage and header can use them.
pub(crate) fn site_portfolio(state: &AppState) -> Option<Value> {
    let s = settings(state);
    s.enabled.then(|| portfolio_json(state, &s))
}

/// Which protected projects this visitor has opened.
async fn unlocked_ids(state: &AppState, headers: &HeaderMap) -> Vec<String> {
    let cookie = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| c.trim().strip_prefix(&format!("{}=", rules::UNLOCK_COOKIE)).map(str::to_string));
    let Some(cookie) = cookie else { return Vec::new() };
    let secret = repo::secret(&state.db).await;
    let mut hashes: HashMap<String, String> = HashMap::new();
    for part in cookie.split(',').take(40) {
        if let Some((id, _)) = part.split_once('.') {
            if vuuut_core::ids::is_objectid(id) {
                if let Some(p) = repo::find_project(&state.db, id).await {
                    hashes.insert(id.to_string(), p.password_hash);
                }
            }
        }
    }
    rules::unlocked(&secret, &cookie, |id| hashes.get(id).cloned())
}

fn availability_badge(s: &Settings) -> String {
    if s.availability == rules::Availability::Hidden {
        return String::new();
    }
    format!(
        r#"<span class="vf-avail vf-avail-{k}"><i></i>{l}{n}</span>"#,
        k = s.availability.key(),
        l = esc(s.availability.label()),
        n = if s.availability_note.is_empty() { String::new() } else { format!(" · {}", esc(&s.availability_note)) },
    )
}

fn socials(s: &Settings) -> String {
    if s.socials.is_empty() {
        return String::new();
    }
    format!(r#"<div class="vf-socials">{}</div>"#, s.socials.iter().map(|(k, u)| format!(r#"<a href="{}" target="_blank" rel="me noopener">{}</a>"#, esc(u), esc(rules::social_label(k)))).collect::<String>())
}

fn card(s: &Settings, p: &repo::Project, x: &repo::Lookup, open: bool) -> String {
    let j = repo::project_json(s, p, x, open);
    let cats: Vec<String> = j["categories"].as_array().map(|a| a.iter().filter_map(|c| c["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    format!(
        r#"<a class="vf-card" href="{u}" data-cats="{slugs}"{accent}><span class="vf-card-art">{img}{lock}</span>
<span class="vf-card-body"><b>{t}</b><span>{meta}</span></span></a>"#,
        u = esc(&repo::project_url(s, p)),
        slugs = esc(j["category_slugs"].as_str().unwrap_or("")),
        accent = if p.accent.is_empty() { String::new() } else { format!(r#" style="--vf-card-accent:{}""#, esc(&p.accent)) },
        img = if p.cover.is_empty() { format!(r#"<span class="vf-card-blank">{}</span>"#, esc(&p.title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())) } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&p.cover)) },
        lock = if p.protected() && !open { r#"<span class="vf-lock" title="Password protected">Private</span>"# } else { "" },
        t = esc(&p.title),
        meta = esc(&[x.client_name(p), cats.join(", "), if p.year > 0 { p.year.to_string() } else { String::new() }].iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(" · ")),
    )
}

fn testimonials_html(list: &[repo::Testimonial]) -> String {
    if list.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="vf-quotes">{}</div>"#,
        list.iter().map(|t| format!(
            r#"<figure class="vf-quote">{stars}<blockquote>{q}</blockquote><figcaption>{img}<span><b>{a}</b><small>{by}</small></span></figcaption></figure>"#,
            stars = if t.rating > 0 { format!(r#"<span class="vf-stars" aria-label="{} out of 5">{}</span>"#, t.rating, "★".repeat(t.rating.clamp(0, 5) as usize)) } else { String::new() },
            q = esc(&t.quote),
            img = if t.photo.is_empty() { format!(r#"<i class="vf-face">{}</i>"#, esc(&t.author.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>())) } else { format!(r#"<img class="vf-face" src="{}" alt="" loading="lazy">"#, esc(&t.photo)) },
            a = esc(&t.author),
            by = esc(&[t.role.as_str(), t.company.as_str()].iter().filter(|v| !v.is_empty()).cloned().collect::<Vec<_>>().join(", ")),
        )).collect::<String>()
    )
}

fn clients_html(list: &[repo::Client]) -> String {
    let shown: Vec<&repo::Client> = list.iter().filter(|c| c.featured != 0).collect();
    if shown.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="vf-logos">{}</div>"#,
        shown.iter().map(|c| {
            let inner = if c.logo.is_empty() { format!("<span>{}</span>", esc(&c.name)) } else { format!(r#"<img src="{}" alt="{}" loading="lazy">"#, esc(&c.logo), esc(&c.name)) };
            if c.url.is_empty() { format!(r#"<span class="vf-logo" title="{}">{inner}</span>"#, esc(&c.name)) } else { format!(r#"<a class="vf-logo" href="{}" target="_blank" rel="noopener" title="{}">{inner}</a>"#, esc(&c.url), esc(&c.name)) }
        }).collect::<String>()
    )
}

fn service_cards(s: &Settings, list: &[repo::Service]) -> String {
    list.iter()
        .map(|v| {
            format!(
                r#"<a class="vf-service" href="{u}"><span class="vf-service-icon vf-i-{icon}" aria-hidden="true"></span><b>{n}</b><span>{sum}</span>{price}</a>"#,
                u = esc(&repo::service_url(s, v)),
                icon = esc(&v.icon),
                n = esc(&v.name),
                sum = esc(&v.summary),
                price = { let p = repo::price_label(s, v); if p.is_empty() { String::new() } else { format!(r#"<em>{}{}</em>"#, esc(&p), if v.price_note.is_empty() { String::new() } else { format!(" {}", esc(&v.price_note)) }) } },
            )
        })
        .collect()
}

fn cta(state: &AppState, s: &Settings) -> String {
    if !s.enquiries && s.email.is_empty() {
        return String::new();
    }
    let href = if s.enquiries { link("/portfolio/contact/") } else { format!("mailto:{}", s.email) };
    format!(
        r#"<section class="vf-cta"><div><h2>{h}</h2><p>{p}</p></div><a class="vf-btn" href="{href}">{b}</a></section>"#,
        h = if s.studio() { "Have a project in mind?" } else { "Let's work together" },
        p = esc(&match s.availability {
            rules::Availability::Booked => format!("{} is fully booked right now — but get in touch for later in the year.", name_of(state, s)),
            _ if s.studio() => "Tell us what you're making. We reply to every brief within two working days.".to_string(),
            _ => "Tell me about your project and I'll get back to you within a couple of days.".to_string(),
        }),
        href = esc(&href),
        b = if s.studio() { "Start a project" } else { "Get in touch" },
    )
}

// ---------------------------------------------------------------------------
// the portfolio home and a discipline
// ---------------------------------------------------------------------------

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    if !s.enabled {
        return not_found(&state, BASE, headers).await;
    }
    render_grid(&state, headers, None).await
}

async fn category_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}category/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(c) = repo::category_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await else { return not_found(&state, &path, headers).await };
    render_grid(&state, headers, Some(c)).await
}

async fn render_grid(state: &AppState, headers: HeaderMap, category: Option<repo::Category>) -> Response {
    let s = settings(state);
    let x = repo::Lookup::load(&state.db).await;
    let opened = unlocked_ids(state, &headers).await;
    let (projects, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, category_id: category.as_ref().map(|c| c.id.as_str()), limit: 500, ..Default::default() }).await;
    let counts = repo::category_counts(&state.db).await;
    let cats: Vec<&repo::Category> = x.categories.iter().filter(|c| counts.get(&c.id).copied().unwrap_or(0) > 0).collect();
    let services = repo::services(&state.db, true).await;
    let path = match &category { Some(c) => format!("{BASE}category/{}/", c.slug), None => BASE.to_string() };
    let name = name_of(state, &s);
    let title = match &category { Some(c) => c.name.clone(), None => s.title.clone() };

    let templates = theme_templates(state, if category.is_some() { &["portfolio/category", "portfolio/index"] } else { &["portfolio/index"] }).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(state, &s));
        data.insert("projects".into(), json!(projects.iter().map(|p| repo::project_json(&s, p, &x, opened.contains(&p.id))).collect::<Vec<_>>()));
        data.insert("featured".into(), json!(projects.iter().filter(|p| p.featured != 0).map(|p| repo::project_json(&s, p, &x, opened.contains(&p.id))).collect::<Vec<_>>()));
        data.insert("categories".into(), json!(cats.iter().map(|c| { let mut j = repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0)); j["current"] = json!(category.as_ref().is_some_and(|k| k.id == c.id)); j }).collect::<Vec<_>>()));
        data.insert("category".into(), category.as_ref().map(|c| repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0))).unwrap_or(Value::Null));
        data.insert("services".into(), json!(services.iter().map(|v| repo::service_json(&s, v)).collect::<Vec<_>>()));
        data.insert("clients".into(), json!(x.clients.iter().filter(|c| c.featured != 0).map(repo::client_json).collect::<Vec<_>>()));
        data.insert("testimonials".into(), json!(x.testimonials.iter().filter(|t| t.featured != 0).map(repo::testimonial_json).collect::<Vec<_>>()));
        return render_virtual_page_as(state, &headers, &public(&path), &title, &s.intro, String::new(), None, false, &templates, data, &["portfolio"]).await;
    }

    let chips = if cats.len() > 1 {
        let mut c = format!(r#"<a class="vf-chip{on}" href="{u}" data-filter="">All</a>"#, on = if category.is_none() { " on" } else { "" }, u = esc(&link(BASE)));
        for k in &cats {
            c.push_str(&format!(r#"<a class="vf-chip{on}" href="{u}" data-filter="{slug}">{n}<small>{count}</small></a>"#,
                on = if category.as_ref().is_some_and(|x| x.id == k.id) { " on" } else { "" },
                u = esc(&repo::category_url(&s, k)), slug = esc(&k.slug), n = esc(&k.name), count = counts.get(&k.id).copied().unwrap_or(0)));
        }
        format!(r#"<nav class="vf-chips" aria-label="Filter by discipline">{c}</nav>"#)
    } else {
        String::new()
    };
    let grid: String = projects.iter().map(|p| card(&s, p, &x, opened.contains(&p.id))).collect();
    let hero = if let Some(c) = &category {
        format!(r#"<header class="vf-hero vf-hero-small"><nav class="vf-crumbs"><a href="{}">{}</a></nav><h1>{}</h1>{}</header>"#,
            esc(&link(BASE)), esc(&s.title), esc(&c.name), if c.description.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&c.description)) })
    } else {
        format!(
            r#"<header class="vf-hero">{portrait}<div class="vf-hero-text">{avail}<h1>{headline}</h1>{intro}<div class="vf-hero-actions">{contact}<a class="vf-btn vf-btn-ghost" href="{about}">{about_label}</a></div>{socials}</div></header>"#,
            portrait = if s.portrait.is_empty() { String::new() } else { format!(r#"<img class="vf-portrait" src="{}" alt="{}">"#, esc(&s.portrait), esc(&name)) },
            avail = availability_badge(&s),
            headline = esc(if s.headline.is_empty() { &name } else { &s.headline }),
            intro = if s.intro.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&s.intro)) },
            contact = if s.enquiries { format!(r#"<a class="vf-btn" href="{}">{}</a>"#, esc(&link("/portfolio/contact/")), if s.studio() { "Start a project" } else { "Hire me" }) } else if !s.email.is_empty() { format!(r#"<a class="vf-btn" href="mailto:{}">Email</a>"#, esc(&s.email)) } else { String::new() },
            about = esc(&link("/portfolio/about/")),
            about_label = if s.studio() { "About the studio" } else { "About me" },
            socials = socials(&s),
        )
    };
    let extras = if category.is_none() {
        let featured_t: Vec<repo::Testimonial> = x.testimonials.iter().filter(|t| t.featured != 0).take(3).cloned().collect();
        format!(
            "{services}{clients}{quotes}",
            services = if services.is_empty() { String::new() } else { format!(r#"<section class="vf-section"><div class="vf-section-head"><h2>{}</h2><a href="{}">All services →</a></div><div class="vf-services">{}</div></section>"#, if s.studio() { "What we do" } else { "What I do" }, esc(&link("/portfolio/services/")), service_cards(&s, &services.iter().take(6).cloned().collect::<Vec<_>>())) },
            clients = { let c = clients_html(&x.clients); if c.is_empty() { c } else { format!(r#"<section class="vf-section"><h2 class="vf-center">Trusted by</h2>{c}</section>"#) } },
            quotes = { let q = testimonials_html(&featured_t); if q.is_empty() { q } else { format!(r#"<section class="vf-section"><h2>Kind words</h2>{q}</section>"#) } },
        )
    } else {
        String::new()
    };
    let body = format!(
        r#"{assets}<div class="vf">{hero}<section class="vf-section vf-work" id="work"><div class="vf-section-head"><h2>{work}</h2></div>{chips}<div class="vf-grid" data-vf-grid>{grid}</div></section>{extras}{cta}</div>"#,
        assets = assets(),
        work = if category.is_some() { format!("{} project{}", projects.len(), if projects.len() == 1 { "" } else { "s" }) } else { "Selected work".into() },
        grid = if grid.is_empty() { r#"<p class="vf-empty">Projects are on their way.</p>"#.to_string() } else { grid },
        cta = cta(state, &s),
    );
    render_virtual_page_as(state, &headers, &public(&path), &if category.is_some() { title } else { format!("{name} — {}", s.title) }, &s.intro, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

// ---------------------------------------------------------------------------
// a project
// ---------------------------------------------------------------------------

fn project_ld(state: &AppState, s: &Settings, p: &repo::Project, x: &repo::Lookup) -> String {
    let mut v = json!({ "@context": "https://schema.org", "@type": "CreativeWork", "name": p.title, "description": p.summary,
        "url": vuuut_core::url::absolute(&state.site_url(), &repo::project_url(s, p)),
        "creator": { "@type": if s.studio() { "Organization" } else { "Person" }, "name": name_of(state, s) } });
    if !p.cover.is_empty() {
        v["image"] = json!(vuuut_core::url::absolute(&state.site_url(), &p.cover));
    }
    if p.year > 0 {
        v["dateCreated"] = json!(p.year.to_string());
    }
    let client = x.client_name(p);
    if !client.is_empty() {
        v["sponsor"] = json!({ "@type": "Organization", "name": client });
    }
    format!(r#"<script type="application/ld+json">{}</script>"#, v.to_string().replace("</", "<\\/"))
}

async fn project_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(p) = repo::project_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await.filter(|p| p.status().reachable()) else { return not_found(&state, &path, headers).await };
    let x = repo::Lookup::load(&state.db).await;
    let open = !p.protected() || unlocked_ids(&state, &headers).await.contains(&p.id);
    if open {
        repo::count_view(&state.db, &p.id).await;
    }
    let (prev, next) = repo::neighbours(&state.db, &p.id).await;
    let url = repo::project_url(&s, &p);
    let title = if p.seo_title.is_empty() { p.title.clone() } else { p.seo_title.clone() };
    let excerpt = if p.seo_description.is_empty() { p.summary.clone() } else { p.seo_description.clone() };
    let unlock_form = format!(
        r#"<form class="vf-unlock" method="post" action="{a}"><label>Password<input type="password" name="password" required autocomplete="current-password"></label><button class="vf-btn" type="submit">Open</button></form>{err}"#,
        a = esc(&link(&format!("/portfolio/{}/unlock/", p.slug))),
        err = if q.contains_key("wrong") { r#"<p class="vf-note vf-note-err">That password didn't work.</p>"# } else { "" },
    );
    let related: Vec<repo::Project> = {
        let mine = x.project_categories.get(&p.id).cloned().unwrap_or_default();
        let (all, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, limit: 200, ..Default::default() }).await;
        all.into_iter().filter(|o| o.id != p.id && x.project_categories.get(&o.id).is_some_and(|c| c.iter().any(|k| mine.contains(k)))).take(3).collect()
    };

    let templates = theme_templates(&state, &["portfolio/project"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("project".into(), repo::project_json(&s, &p, &x, open));
        data.insert("prev".into(), prev.as_ref().map(|o| repo::project_json(&s, o, &x, false)).unwrap_or(Value::Null));
        data.insert("next".into(), next.as_ref().map(|o| repo::project_json(&s, o, &x, false)).unwrap_or(Value::Null));
        data.insert("related".into(), json!(related.iter().map(|o| repo::project_json(&s, o, &x, false)).collect::<Vec<_>>()));
        data.insert("forms".into(), json!({ "unlock": if open { Value::Null } else { json!(unlock_form) } }));
        return render_virtual_page_as(&state, &headers, &public(&url), &title, &excerpt, String::new(), (!p.cover.is_empty()).then(|| p.cover.clone()), false, &templates, data, &["portfolio"]).await;
    }

    let j = repo::project_json(&s, &p, &x, open);
    let cats: Vec<String> = j["categories"].as_array().map(|a| a.iter().map(|c| format!(r#"<a href="{}">{}</a>"#, esc(c["url"].as_str().unwrap_or("")), esc(c["name"].as_str().unwrap_or("")))).collect()).unwrap_or_default();
    let services: Vec<String> = {
        let ids = repo::project_service_ids(&state.db, &p.id).await;
        x.services.iter().filter(|v| ids.contains(&v.id) && v.status == "active").map(|v| format!(r#"<a href="{}">{}</a>"#, esc(&repo::service_url(&s, v)), esc(&v.name))).collect()
    };
    let client = x.client_name(&p);
    let mut facts: Vec<(&str, String)> = Vec::new();
    if !client.is_empty() {
        facts.push(("Client", esc(&client)));
    }
    if p.year > 0 {
        facts.push(("Year", p.year.to_string()));
    }
    if !p.role.is_empty() {
        facts.push((if s.studio() { "Our role" } else { "My role" }, esc(&p.role)));
    }
    if !p.duration.is_empty() {
        facts.push(("Timeline", esc(&p.duration)));
    }
    if !p.industry.is_empty() {
        facts.push(("Industry", esc(&p.industry)));
    }
    if !p.location.is_empty() {
        facts.push(("Location", esc(&p.location)));
    }
    if !cats.is_empty() {
        facts.push(("Discipline", cats.join(", ")));
    }
    if !services.is_empty() {
        facts.push(("Services", services.join(", ")));
    }
    if open && !p.tools().is_empty() {
        facts.push(("Tools", esc(&p.tools().join(", "))));
    }
    let links = if open {
        let mut l = String::new();
        if !p.live_url.is_empty() {
            l.push_str(&format!(r#"<a class="vf-btn" href="{}" target="_blank" rel="noopener">View live ↗</a>"#, esc(&p.live_url)));
        }
        if !p.repo_url.is_empty() {
            l.push_str(&format!(r#"<a class="vf-btn vf-btn-ghost" href="{}" target="_blank" rel="noopener">Source code ↗</a>"#, esc(&p.repo_url)));
        }
        l
    } else {
        String::new()
    };
    let gallery = if open && !p.gallery().is_empty() {
        format!(r#"<section class="vf-gallery vf-gallery-{}" data-vf-gallery>{}</section>"#, esc(&p.layout), p.gallery().iter().map(|g| format!(r#"<a href="{0}" data-vf-zoom><img src="{0}" alt="" loading="lazy"></a>"#, esc(g))).collect::<String>())
    } else {
        String::new()
    };
    let metrics = if open && !p.metrics().is_empty() {
        format!(r#"<section class="vf-metrics">{}</section>"#, p.metrics().iter().map(|m| format!("<div><b>{}</b><span>{}</span></div>", esc(&m.value), esc(&m.label))).collect::<String>())
    } else {
        String::new()
    };
    let video = if open { vuuut_core::lms::video_embed(&p.video_url).map(|(u, frame)| if frame {
        format!(r#"<div class="vf-video"><iframe src="{}" title="Video" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen loading="lazy"></iframe></div>"#, esc(&u))
    } else {
        format!(r#"<div class="vf-video"><video src="{}" controls preload="metadata" playsinline></video></div>"#, esc(&u))
    }).unwrap_or_default() } else { String::new() };
    let credits = if open && !p.credits().is_empty() {
        format!(r#"<section class="vf-section vf-credits"><h2>Credits</h2><dl>{}</dl></section>"#, p.credits().iter().map(|c| format!(
            "<div><dt>{}</dt><dd>{}</dd></div>", esc(&c.role),
            if c.url.is_empty() { esc(&c.name) } else { format!(r#"<a href="{}" target="_blank" rel="noopener">{}</a>"#, esc(&c.url), esc(&c.name)) })).collect::<String>())
    } else {
        String::new()
    };
    let testimonial = if open {
        p.testimonial_id.as_deref().and_then(|id| x.testimonials.iter().find(|t| t.id == id)).map(|t| format!(r#"<section class="vf-section">{}</section>"#, testimonials_html(std::slice::from_ref(t)))).unwrap_or_default()
    } else {
        String::new()
    };
    let body_html = if open {
        let b = p.body_markup();
        if b.trim().is_empty() { String::new() } else { format!(r#"<div class="vf-body gh-content">{b}</div>"#) }
    } else {
        format!(r#"<section class="vf-locked"><h2>This case study is private</h2><p>It's shared under a confidentiality agreement. Enter the password you were given to open it.</p>{unlock_form}</section>"#)
    };
    let nav = |o: &Option<repo::Project>, label: &str, cls: &str| o.as_ref().map(|o| format!(
        r#"<a class="vf-next {cls}" href="{}">{}<span><small>{label}</small><b>{}</b></span></a>"#,
        esc(&repo::project_url(&s, o)), if o.cover.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&o.cover)) }, esc(&o.title))).unwrap_or_else(|| "<span></span>".into());
    let body = format!(
        r#"{assets}{ld}<article class="vf vf-project vf-layout-{layout}"{accent}><nav class="vf-crumbs"><a href="{home}">{home_t}</a></nav>
<header class="vf-project-head"><h1>{title}</h1>{sub}</header>
{cover}{video}
<div class="vf-project-main">{facts}<div class="vf-project-text">{summary}{body}{links}</div></div>
{metrics}{gallery}{testimonial}{credits}
<nav class="vf-nextprev">{prev}{next}</nav>
{related}{cta}</article>"#,
        assets = assets(),
        ld = project_ld(&state, &s, &p, &x),
        layout = esc(&p.layout),
        accent = if p.accent.is_empty() { String::new() } else { format!(r#" style="--vf-accent:{}""#, esc(&p.accent)) },
        home = esc(&link(BASE)),
        home_t = esc(&s.title),
        title = esc(&p.title),
        sub = if p.subtitle.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&p.subtitle)) },
        cover = if p.cover.is_empty() { String::new() } else { format!(r#"<figure class="vf-cover"><img src="{}" alt=""></figure>"#, esc(&p.cover)) },
        facts = if facts.is_empty() { String::new() } else { format!(r#"<dl class="vf-facts">{}</dl>"#, facts.iter().map(|(k, v)| format!("<div><dt>{k}</dt><dd>{v}</dd></div>")).collect::<String>()) },
        summary = if p.summary.is_empty() { String::new() } else { format!(r#"<p class="vf-summary">{}</p>"#, esc(&p.summary)) },
        body = body_html,
        links = if links.is_empty() { String::new() } else { format!(r#"<div class="vf-actions">{links}</div>"#) },
        prev = nav(&prev, "Previous", "vf-prev"),
        next = nav(&next, "Next project", "vf-nextone"),
        related = if related.is_empty() { String::new() } else { format!(r#"<section class="vf-section"><h2>More like this</h2><div class="vf-grid vf-grid-3">{}</div></section>"#, related.iter().map(|o| card(&s, o, &x, false)).collect::<String>()) },
        cta = cta(&state, &s),
    );
    let mut resp = render_virtual_page_as(&state, &headers, &public(&url), &title, &excerpt, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await;
    if p.status == "unlisted" || p.protected() {
        resp.headers_mut().insert(header::HeaderName::from_static("x-robots-tag"), header::HeaderValue::from_static("noindex"));
    }
    resp
}

const UNLOCK_BUDGET: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 8, min_wait: 30_000, max_wait: 3_600_000, lifetime: 3600 };

async fn unlock(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    if !vuuut_api::forms::same_origin(&headers, &state.site_url()) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let Some(p) = repo::project_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await.filter(repo::Project::protected) else { return not_found(&state, &path, headers).await };
    let back = link(&format!("/portfolio/{}/", p.slug));
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    let key = format!("portfolio_unlock:{ip}");
    if vuuut_api::brute::check(&state.db, &key, &UNLOCK_BUDGET).await.is_err() {
        return Redirect::to(&format!("{back}?wrong=1")).into_response();
    }
    let given = form.get("password").map(String::as_str).unwrap_or("");
    if !rules::password_matches(&p.password_hash, given) {
        vuuut_api::brute::record_failure(&state.db, &key, &UNLOCK_BUDGET).await;
        return Redirect::to(&format!("{back}?wrong=1")).into_response();
    }
    let secret = repo::secret(&state.db).await;
    let mut parts: Vec<String> = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';').map(str::to_string).collect::<Vec<_>>())
        .find_map(|c| c.trim().strip_prefix(&format!("{}=", rules::UNLOCK_COOKIE)).map(str::to_string))
        .map(|c| c.split(',').filter(|x| !x.starts_with(&format!("{}.", p.id))).map(str::to_string).collect())
        .unwrap_or_default();
    parts.push(rules::unlock_value(&secret, &p.id, &p.password_hash));
    parts.truncate(30);
    let secure = if state.site_url().starts_with("https://") { "; Secure" } else { "" };
    let cookie = format!("{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{secure}", rules::UNLOCK_COOKIE, parts.join(","), rules::UNLOCK_DAYS * 86400);
    ([(header::SET_COOKIE, cookie), (header::LOCATION, back), (header::CACHE_CONTROL, "no-store".into())], StatusCode::SEE_OTHER).into_response()
}

// ---------------------------------------------------------------------------
// services
// ---------------------------------------------------------------------------

async fn services_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}services/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let list = repo::services(&state.db, true).await;
    if list.is_empty() {
        return not_found(&state, &path, headers).await;
    }
    let title = if s.studio() { "Services" } else { "What I do" };
    let templates = theme_templates(&state, &["portfolio/services"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("services".into(), json!(list.iter().map(|v| repo::service_json(&s, v)).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), title, "", String::new(), None, false, &templates, data, &["portfolio"]).await;
    }
    let rows: String = list.iter().map(|v| format!(
        r#"<article class="vf-service-row"><span class="vf-service-icon vf-i-{icon}" aria-hidden="true"></span><div><h2><a href="{u}">{n}</a></h2><p>{sum}</p>{del}</div><div class="vf-service-side">{price}<a class="vf-btn vf-btn-ghost" href="{u}">Details</a></div></article>"#,
        icon = esc(&v.icon), u = esc(&repo::service_url(&s, v)), n = esc(&v.name), sum = esc(&v.summary),
        del = if v.deliverables().is_empty() { String::new() } else { format!("<ul class=\"vf-ticks\">{}</ul>", v.deliverables().iter().take(5).map(|d| format!("<li>{}</li>", esc(d))).collect::<String>()) },
        price = { let p = repo::price_label(&s, v); if p.is_empty() { String::new() } else { format!(r#"<b class="vf-price">{}</b><small>{}</small>"#, esc(&p), esc(&v.price_note)) } },
    )).collect();
    let body = format!(r#"{assets}<div class="vf"><header class="vf-hero vf-hero-small"><nav class="vf-crumbs"><a href="{home}">{ht}</a></nav><h1>{title}</h1></header><div class="vf-service-list">{rows}</div>{cta}</div>"#,
        assets = assets(), home = esc(&link(BASE)), ht = esc(&s.title), cta = cta(&state, &s));
    render_virtual_page_as(&state, &headers, &public(&path), title, "", body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

async fn service_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}services/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(v) = repo::service_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await.filter(|v| v.status == "active") else { return not_found(&state, &path, headers).await };
    let x = repo::Lookup::load(&state.db).await;
    let (work, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, service_id: Some(&v.id), limit: 9, ..Default::default() }).await;
    let templates = theme_templates(&state, &["portfolio/service"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("service".into(), repo::service_json(&s, &v));
        data.insert("projects".into(), json!(work.iter().map(|p| repo::project_json(&s, p, &x, false)).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), &v.name, &v.summary, String::new(), (!v.image.is_empty()).then(|| v.image.clone()), false, &templates, data, &["portfolio"]).await;
    }
    let price = repo::price_label(&s, &v);
    let body = format!(
        r#"{assets}<div class="vf"><nav class="vf-crumbs"><a href="{home}">{ht}</a> · <a href="{all}">Services</a></nav>
<header class="vf-service-head"><span class="vf-service-icon vf-i-{icon}" aria-hidden="true"></span><h1>{n}</h1>{sum}{price}</header>
{img}<div class="vf-project-main"><div class="vf-project-text"><div class="vf-body">{desc}</div></div>{del}</div>
{work}{cta}</div>"#,
        assets = assets(), home = esc(&link(BASE)), ht = esc(&s.title), all = esc(&link("/portfolio/services/")), icon = esc(&v.icon), n = esc(&v.name),
        sum = if v.summary.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&v.summary)) },
        price = if price.is_empty() { String::new() } else { format!(r#"<p class="vf-price">{} <small>{}</small></p>"#, esc(&price), esc(&v.price_note)) },
        img = if v.image.is_empty() { String::new() } else { format!(r#"<figure class="vf-cover"><img src="{}" alt=""></figure>"#, esc(&v.image)) },
        desc = vuuut_core::directory::paragraphs(&v.description),
        del = if v.deliverables().is_empty() { String::new() } else { format!(r#"<aside class="vf-facts"><h3>What you get</h3><ul class="vf-ticks">{}</ul></aside>"#, v.deliverables().iter().map(|d| format!("<li>{}</li>", esc(d))).collect::<String>()) },
        work = if work.is_empty() { String::new() } else { format!(r#"<section class="vf-section"><h2>Selected {} work</h2><div class="vf-grid vf-grid-3">{}</div></section>"#, esc(&v.name.to_lowercase()), work.iter().map(|p| card(&s, p, &x, false)).collect::<String>()) },
        cta = cta(&state, &s),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &v.name, &v.summary, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

// ---------------------------------------------------------------------------
// about, résumé, team
// ---------------------------------------------------------------------------

fn timeline(entries: &[&repo::Entry]) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                r#"<li class="vf-entry"><span class="vf-when">{span}</span><div><b>{t}</b>{org}{desc}</div></li>"#,
                span = esc(&e.span()),
                t = esc(&e.title),
                org = if e.organisation.is_empty() && e.location.is_empty() { String::new() } else {
                    let org = if e.url.is_empty() { esc(&e.organisation) } else { format!(r#"<a href="{}" target="_blank" rel="noopener">{}</a>"#, esc(&e.url), esc(&e.organisation)) };
                    format!(r#"<span class="vf-org">{org}{loc}</span>"#, loc = if e.location.is_empty() { String::new() } else { format!(" · {}", esc(&e.location)) })
                },
                desc = if e.description.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&e.description)) },
            )
        })
        .collect()
}

fn entries_by_kind(entries: &[repo::Entry]) -> String {
    let mut out = String::new();
    for kind in rules::EntryKind::ALL {
        let list: Vec<&repo::Entry> = entries.iter().filter(|e| e.kind == kind.key()).collect();
        if list.is_empty() {
            continue;
        }
        let heading = match kind {
            rules::EntryKind::Work => "Experience",
            rules::EntryKind::Education => "Education",
            rules::EntryKind::Award => "Awards",
            rules::EntryKind::Press => "Press",
            rules::EntryKind::Talk => "Talks",
            rules::EntryKind::Publication => "Publications",
            rules::EntryKind::Certification => "Certifications",
            rules::EntryKind::Volunteer => "Volunteering",
        };
        out.push_str(&format!(r#"<section class="vf-section vf-cv-block"><h2>{heading}</h2><ol class="vf-timeline">{}</ol></section>"#, timeline(&list)));
    }
    out
}

fn skills_html(skills: &[repo::Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut groups: Vec<(String, Vec<&repo::Skill>)> = Vec::new();
    for k in skills {
        match groups.iter_mut().find(|(g, _)| *g == k.grp) {
            Some((_, v)) => v.push(k),
            None => groups.push((k.grp.clone(), vec![k])),
        }
    }
    format!(
        r#"<section class="vf-section vf-cv-block"><h2>Skills</h2><div class="vf-skills">{}</div></section>"#,
        groups.iter().map(|(g, list)| format!(
            r#"<div class="vf-skill-group">{}<ul>{}</ul></div>"#,
            if g.is_empty() { String::new() } else { format!("<h3>{}</h3>", esc(g)) },
            list.iter().map(|k| if k.level > 0 {
                format!(r#"<li class="vf-skill"><span>{}</span><i style="--l:{}%"></i></li>"#, esc(&k.name), k.level.clamp(0, 100))
            } else {
                format!(r#"<li class="vf-tag">{}</li>"#, esc(&k.name))
            }).collect::<String>()
        )).collect::<String>()
    )
}

fn team_html(s: &Settings, team: &[repo::Member]) -> String {
    if team.is_empty() {
        return String::new();
    }
    format!(r#"<div class="vf-team">{}</div>"#, team.iter().map(|m| format!(
        r#"<a class="vf-member" href="{u}">{img}<b>{n}</b><span>{r}</span></a>"#,
        u = esc(&repo::link(s, &format!("/portfolio/team/{}/", m.slug))),
        img = if m.photo.is_empty() { format!(r#"<i class="vf-member-photo vf-member-blank">{}</i>"#, esc(&m.initials())) } else { format!(r#"<img class="vf-member-photo" src="{}" alt="" loading="lazy">"#, esc(&m.photo)) },
        n = esc(&m.name), r = esc(&m.role),
    )).collect::<String>())
}

async fn about_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}about/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let name = name_of(&state, &s);
    let x = repo::Lookup::load(&state.db).await;
    let team = repo::team(&state.db, true).await;
    let entries = repo::entries(&state.db).await;
    let skills = repo::skills(&state.db).await;
    let (projects, total) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, limit: 1, ..Default::default() }).await;
    let _ = projects;
    let title = if s.studio() { format!("About {name}") } else { "About".to_string() };
    let templates = theme_templates(&state, &["portfolio/about"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("team".into(), json!(team.iter().map(|m| repo::member_json(&s, m)).collect::<Vec<_>>()));
        data.insert("entries".into(), json!(entries.iter().map(repo::entry_json).collect::<Vec<_>>()));
        data.insert("skills".into(), json!(skills.iter().map(repo::skill_json).collect::<Vec<_>>()));
        data.insert("clients".into(), json!(x.clients.iter().map(repo::client_json).collect::<Vec<_>>()));
        data.insert("testimonials".into(), json!(x.testimonials.iter().map(repo::testimonial_json).collect::<Vec<_>>()));
        data.insert("stats".into(), json!({ "projects": total, "clients": x.clients.len(), "years": if s.founded > 0 { json!(chrono::Datelike::year(&chrono::Utc::now()) as i64 - s.founded) } else { Value::Null } }));
        return render_virtual_page_as(&state, &headers, &public(&path), &title, &s.headline, String::new(), (!s.portrait.is_empty()).then(|| s.portrait.clone()), false, &templates, data, &["portfolio"]).await;
    }
    let years = if s.founded > 0 { chrono::Datelike::year(&chrono::Utc::now()) as i64 - s.founded } else { 0 };
    let mut stats: Vec<(String, &str)> = Vec::new();
    if total > 0 {
        stats.push((total.to_string(), "projects"));
    }
    if !x.clients.is_empty() {
        stats.push((x.clients.len().to_string(), "clients"));
    }
    if years > 0 {
        stats.push((years.to_string(), if s.studio() { "years in business" } else { "years of practice" }));
    }
    if s.studio() && !team.is_empty() {
        stats.push((team.len().to_string(), "people"));
    }
    let recognition: Vec<repo::Entry> = entries.iter().filter(|e| matches!(e.kind.as_str(), "award" | "press" | "talk" | "publication")).cloned().collect();
    let work: Vec<repo::Entry> = entries.iter().filter(|e| matches!(e.kind.as_str(), "work" | "education")).cloned().collect();
    let body = format!(
        r#"{assets}<div class="vf"><header class="vf-about-head">{portrait}<div>{avail}<h1>{title}</h1>{headline}{loc}{socials}</div></header>
{about}{stats}{team}{cv}{skills}{recognition}{clients}{quotes}{resume}{cta}</div>"#,
        assets = assets(),
        portrait = if s.portrait.is_empty() { String::new() } else { format!(r#"<img class="vf-portrait vf-portrait-lg" src="{}" alt="{}">"#, esc(&s.portrait), esc(&name)) },
        avail = availability_badge(&s),
        title = esc(&if s.studio() { name.clone() } else { format!("Hi, I'm {name}") }),
        headline = if s.headline.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&s.headline)) },
        loc = if s.location.is_empty() { String::new() } else { format!(r#"<p class="vf-muted">{}</p>"#, esc(&s.location)) },
        socials = socials(&s),
        about = if s.about.is_empty() { String::new() } else { format!(r#"<div class="vf-body vf-about">{}</div>"#, vuuut_core::directory::paragraphs(&s.about)) },
        stats = if stats.is_empty() { String::new() } else { format!(r#"<div class="vf-metrics">{}</div>"#, stats.iter().map(|(n, l)| format!("<div><b>{}</b><span>{}</span></div>", esc(n), l)).collect::<String>()) },
        team = if team.is_empty() { String::new() } else { format!(r#"<section class="vf-section"><h2>{}</h2>{}</section>"#, if s.studio() { "The team" } else { "Collaborators" }, team_html(&s, &team)) },
        cv = if s.studio() { String::new() } else { entries_by_kind(&work) },
        skills = if s.studio() { String::new() } else { skills_html(&skills) },
        recognition = entries_by_kind(&recognition),
        clients = { let c = clients_html(&x.clients); if c.is_empty() { c } else { format!(r#"<section class="vf-section"><h2>Clients</h2>{c}</section>"#) } },
        quotes = { let q = testimonials_html(&x.testimonials.iter().take(6).cloned().collect::<Vec<_>>()); if q.is_empty() { q } else { format!(r#"<section class="vf-section"><h2>Testimonials</h2>{q}</section>"#) } },
        resume = if !s.studio() && (!entries.is_empty() || !s.resume_url.is_empty()) { format!(r#"<p class="vf-actions"><a class="vf-btn vf-btn-ghost" href="{}">View my résumé</a></p>"#, esc(&link("/portfolio/resume/"))) } else { String::new() },
        cta = cta(&state, &s),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &title, &s.headline, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

async fn resume_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}resume/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let entries = repo::entries(&state.db).await;
    let skills = repo::skills(&state.db).await;
    if entries.is_empty() && skills.is_empty() && s.resume_url.is_empty() {
        return not_found(&state, &path, headers).await;
    }
    let name = name_of(&state, &s);
    let templates = theme_templates(&state, &["portfolio/resume"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("entries".into(), json!(entries.iter().map(repo::entry_json).collect::<Vec<_>>()));
        data.insert("skills".into(), json!(skills.iter().map(repo::skill_json).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), &format!("{name} — Résumé"), &s.headline, String::new(), None, false, &templates, data, &["portfolio"]).await;
    }
    let contact: Vec<String> = [
        (!s.email.is_empty()).then(|| format!(r#"<a href="mailto:{0}">{0}</a>"#, esc(&s.email))),
        (!s.phone.is_empty()).then(|| esc(&s.phone)),
        (!s.location.is_empty()).then(|| esc(&s.location)),
    ].into_iter().flatten().chain(s.socials.iter().map(|(k, u)| format!(r#"<a href="{}">{}</a>"#, esc(u), esc(rules::social_label(k))))).collect();
    let body = format!(
        r#"{assets}<div class="vf vf-resume"><header class="vf-cv-head"><div><h1>{n}</h1>{h}<p class="vf-cv-contact">{c}</p></div><div class="vf-cv-actions"><button class="vf-btn vf-btn-ghost" type="button" data-vf-print>Print</button>{pdf}</div></header>
{about}{cv}{skills}</div>"#,
        assets = assets(),
        n = esc(&name),
        h = if s.headline.is_empty() { String::new() } else { format!(r#"<p class="vf-lede">{}</p>"#, esc(&s.headline)) },
        c = contact.join(" · "),
        pdf = if s.resume_url.is_empty() { String::new() } else { format!(r#"<a class="vf-btn" href="{}" download>Download PDF</a>"#, esc(&s.resume_url)) },
        about = if s.about.is_empty() { String::new() } else { format!(r#"<section class="vf-section vf-cv-block"><h2>Profile</h2><div class="vf-body">{}</div></section>"#, vuuut_core::directory::paragraphs(&s.about.split("\n\n").next().unwrap_or(""))) },
        cv = entries_by_kind(&entries),
        skills = skills_html(&skills),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &format!("{name} — Résumé"), &s.headline, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

async fn member_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}team/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(m) = repo::member_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await.filter(|m| m.status == "active") else { return not_found(&state, &path, headers).await };
    let x = repo::Lookup::load(&state.db).await;
    let (all, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, limit: 500, ..Default::default() }).await;
    let theirs: Vec<&repo::Project> = all.iter().filter(|p| p.credits().iter().any(|c| c.name.eq_ignore_ascii_case(&m.name))).collect();
    let templates = theme_templates(&state, &["portfolio/member"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("member".into(), repo::member_json(&s, &m));
        data.insert("projects".into(), json!(theirs.iter().map(|p| repo::project_json(&s, p, &x, false)).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), &m.name, &m.role, String::new(), (!m.photo.is_empty()).then(|| m.photo.clone()), false, &templates, data, &["portfolio"]).await;
    }
    let body = format!(
        r#"{assets}<div class="vf"><nav class="vf-crumbs"><a href="{about}">About</a></nav><header class="vf-about-head">{img}<div><h1>{n}</h1><p class="vf-lede">{r}</p><div class="vf-socials">{links}</div></div></header>
<div class="vf-body vf-about">{bio}</div>{work}</div>"#,
        assets = assets(),
        about = esc(&link("/portfolio/about/")),
        img = if m.photo.is_empty() { format!(r#"<i class="vf-portrait vf-member-blank">{}</i>"#, esc(&m.initials())) } else { format!(r#"<img class="vf-portrait" src="{}" alt="">"#, esc(&m.photo)) },
        n = esc(&m.name),
        r = esc(&m.role),
        links = m.links().iter().map(|u| format!(r#"<a href="{0}" target="_blank" rel="noopener">{1}</a>"#, esc(u), esc(u.split("//").nth(1).unwrap_or(u).split('/').next().unwrap_or(u).trim_start_matches("www.")))).collect::<String>(),
        bio = vuuut_core::directory::paragraphs(&m.bio),
        work = if theirs.is_empty() { String::new() } else { format!(r#"<section class="vf-section"><h2>Projects</h2><div class="vf-grid vf-grid-3">{}</div></section>"#, theirs.iter().map(|p| card(&s, p, &x, false)).collect::<String>()) },
    );
    render_virtual_page_as(&state, &headers, &public(&path), &m.name, &m.role, body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

// ---------------------------------------------------------------------------
// contact / project brief
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn contact_form(s: &Settings, services: &[repo::Service], project: &str) -> String {
    let budgets: String = s.budgets.iter().map(|b| format!(r#"<label class="vf-radio"><input type="radio" name="budget" value="{0}"><span>{0}</span></label>"#, esc(b))).collect();
    let svc: String = services.iter().map(|v| format!(r#"<label class="vf-radio"><input type="checkbox" value="{0}" data-vf-service><span>{0}</span></label>"#, esc(&v.name))).collect();
    format!(
        r#"<form class="vf-form" method="post" action="{action}" data-vf-contact><input type="hidden" name="t" value="{t}"><input type="hidden" name="services" value="" data-vf-services>
<div class="vf-hp" aria-hidden="true"><label>Leave this empty <input name="website_url" tabindex="-1" autocomplete="off"></label></div>
<div class="vf-form-2"><label>Your name<input name="name" required maxlength="120" autocomplete="name"></label><label>Email<input type="email" name="email" required maxlength="191" autocomplete="email"></label></div>
<div class="vf-form-2"><label>Company <small>(optional)</small><input name="company" maxlength="120" autocomplete="organization"></label><label>Phone <small>(optional)</small><input type="tel" name="phone" maxlength="40" autocomplete="tel"></label></div>
{svc}{budget}
<label>Timeline <small>(optional)</small><input name="timeline" maxlength="80" placeholder="e.g. launch in March"></label>
<label>About the project<textarea name="message" rows="7" required minlength="10" maxlength="6000" placeholder="What are you making, who is it for, and what does success look like?"></textarea></label>
{proj}<button class="vf-btn" type="submit">Send</button></form>"#,
        action = esc(&link("/portfolio/contact/")),
        t = now_ms(),
        svc = if svc.is_empty() { String::new() } else { format!(r#"<fieldset class="vf-fieldset"><legend>What do you need?</legend><div class="vf-radios">{svc}</div></fieldset>"#) },
        budget = if s.studio() || !budgets.is_empty() { format!(r#"<fieldset class="vf-fieldset"><legend>Budget</legend><div class="vf-radios">{budgets}</div></fieldset>"#) } else { String::new() },
        proj = if project.is_empty() { String::new() } else { format!(r#"<input type="hidden" name="project" value="{}">"#, esc(project)) },
    )
}

async fn contact_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}contact/");
    if !s.enabled || !s.enquiries {
        return not_found(&state, &path, headers).await;
    }
    let services = repo::services(&state.db, true).await;
    let project = q.get("project").map(|p| vuuut_core::directory::clean(p, 200)).unwrap_or_default();
    let banner = if q.contains_key("sent") {
        format!(r#"<div class="vf-note vf-note-ok">Thank you — your message is in. {} reply within a couple of working days.</div>"#, if s.studio() { "We'll" } else { "I'll" })
    } else if let Some(e) = q.get("error") {
        format!(r#"<div class="vf-note vf-note-err">{}</div>"#, esc(&vuuut_core::directory::clean(e, 200)))
    } else {
        String::new()
    };
    let form = contact_form(&s, &services, &project);
    let templates = theme_templates(&state, &["portfolio/contact"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("portfolio".into(), portfolio_json(&state, &s));
        data.insert("forms".into(), json!({ "contact": form, "sent": q.contains_key("sent"), "error": q.get("error") }));
        return render_virtual_page_as(&state, &headers, &public(&path), "Contact", "", String::new(), None, false, &templates, data, &["portfolio"]).await;
    }
    let direct: Vec<String> = [
        (!s.email.is_empty()).then(|| format!(r#"<a href="mailto:{0}">{0}</a>"#, esc(&s.email))),
        (!s.phone.is_empty()).then(|| format!(r#"<a href="tel:{}">{}</a>"#, esc(&s.phone.replace(' ', "")), esc(&s.phone))),
        (!s.location.is_empty()).then(|| esc(&s.location)),
    ].into_iter().flatten().collect();
    let body = format!(
        r#"{assets}<div class="vf vf-contact"><div class="vf-contact-side">{avail}<h1>{h}</h1><p class="vf-lede">{p}</p>{direct}{socials}</div><div class="vf-contact-main">{banner}{form}</div></div>"#,
        assets = assets(),
        avail = availability_badge(&s),
        h = if s.studio() { "Start a project" } else { "Let's talk" },
        p = if s.studio() { "Tell us a little about what you're planning. The more we know, the better our first reply." } else { "Got a project, a role or a question? Send me a note." },
        direct = if direct.is_empty() { String::new() } else { format!(r#"<p class="vf-direct">{}</p>"#, direct.join("<br>")) },
        socials = socials(&s),
    );
    render_virtual_page_as(&state, &headers, &public(&path), "Contact", "", body, None, false, &[], serde_json::Map::new(), &["portfolio"]).await
}

const PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 5, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };

async fn contact_submit(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}contact/");
    if !s.enabled || !s.enquiries {
        return not_found(&state, &path, headers).await;
    }
    if !vuuut_api::forms::same_origin(&headers, &state.site_url()) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let back = |msg: &str| Redirect::to(&format!("{}?error={}", link(&path), vuuut_core::social_media::encode(msg))).into_response();
    let done = Redirect::to(&format!("{}?sent=1", link(&path))).into_response();
    if form.get("website_url").is_some_and(|v| !v.is_empty()) {
        return done;
    }
    let started = form.get("t").and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    if now_ms() - started < rules::MIN_FILL_MS {
        return done;
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("portfolio_enquiry:{ip}"), &PER_IP).await.is_err() {
        return back("Too many messages from here. Please wait a little.");
    }
    let services: Vec<String> = repo::services(&state.db, true).await.into_iter().map(|v| v.name).collect();
    let input = match rules::EnquiryInput::from_form(&form, &s.budgets, &services) {
        Ok(i) => i,
        Err(m) => return back(&m),
    };
    let day_ago = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(1)));
    if repo::enquiries_from_since(&state.db, &input.email, &day_ago).await >= 3 {
        return back("That address has sent three messages today. Please try again tomorrow.");
    }
    match repo::insert_enquiry(&state.db, &input).await {
        Ok(id) => {
            if let Some(e) = repo::find_enquiry(&state.db, &id).await {
                vuuut_api::portfolio::notify_enquiry(&state, &e).await;
            }
            done
        }
        Err(e) => {
            tracing::error!("portfolio: storing an enquiry failed: {}", e.message());
            back("That could not be sent. Please try again.")
        }
    }
}
