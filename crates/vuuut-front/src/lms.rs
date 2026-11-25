//! The public LMS: `/courses/` (or wherever Settings moves it).
//!
//! Every screen is rendered inside the active theme as a transient page,
//! like the store and the directory, so it carries the site's header and
//! footer with no theme changes. A theme can take over any screen with its
//! own template — `lms/courses.hbs` (catalogue and categories),
//! `lms/course.hbs`, `lms/lesson.hbs`, `lms/dashboard.hbs` — and gets the
//! data below plus ready-made HTML for the parts that post forms
//! (`forms.*`), so no theme has to rebuild the security around them.
//! While the LMS is switched off every address here is the theme's 404.
//!
//! * `GET /courses/` — the catalogue: search, category, level, price, sort.
//! * `GET /courses/category/{slug}/`
//! * `GET /courses/{slug}/` — the course: curriculum, instructor, reviews,
//!   and the one button that matters (enrol, buy, sign in, continue).
//! * `POST /courses/{slug}/enroll/` — free, member and tier enrolment;
//!   a paid course sends the visitor to its store product instead.
//! * `GET /courses/{slug}/learn/` — continue where you left off.
//! * `GET /courses/{slug}/learn/{lesson}/` — the lesson player.
//! * `POST …/complete/`, `…/quiz/`, `…/submit/` — mark done, answer a
//!   quiz, hand in an assignment. `POST /courses/{slug}/review/`.
//! * `GET /courses/dashboard/` — My learning.
//! * `GET /courses/certificate/{code}/` — a certificate anyone can verify.
//!
//! Access is decided here, on every request: drip dates, lessons that
//! unlock in order, free previews, expiry. Every write is same-origin and
//! needs the signed-in member it is for. Everything shown is escaped except
//! lesson and course bodies, which are the editor's own rendered HTML.

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
use vuuut_core::lms::{self as rules, Access, LessonKind, Settings};
use vuuut_db::repos::lms as repo;

const BASE: &str = "/courses/";
static CSS: &str = include_str!("../assets/lms.css");
static JS: &str = include_str!("../assets/lms.js");

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
        .route("/courses/", get(catalog))
        .route("/courses", get(|| async { Redirect::permanent(&link(BASE)) }))
        .route("/courses/assets/{v}/lms.css", get(|| async {
            ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response()
        }))
        .route("/courses/assets/{v}/lms.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], JS).into_response()
        }))
        .route("/courses/category/{slug}/", get(category))
        .route("/courses/dashboard/", get(dashboard))
        .route("/courses/certificate/{code}/", get(certificate))
        .route("/courses/{slug}/", get(course_page))
        .route("/courses/{slug}/enroll/", post(enroll))
        .route("/courses/{slug}/review/", post(review))
        .route("/courses/{slug}/learn/", get(resume))
        .route("/courses/{slug}/learn/{lesson}/", get(lesson_page))
        .route("/courses/{slug}/learn/{lesson}/complete/", post(complete))
        .route("/courses/{slug}/learn/{lesson}/quiz/", post(quiz))
        .route("/courses/{slug}/learn/{lesson}/submit/", post(submit))
}

fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

pub(crate) fn esc(s: &str) -> String {
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

/// Moves the catalogue to the address chosen in Settings: the public address
/// is served by the `/courses/` routes, and `/courses/…` redirects there.
pub(crate) async fn addresses(State(state): State<AppState>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let paths = rules::Paths::read(|k| state.settings.get_str(k));
    if paths != rules::Paths::default() {
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        if let Some(internal) = paths.internal_path(&path) {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        } else if (path == "/courses" || path.starts_with(BASE)) && req.method() == axum::http::Method::GET {
            return Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(&paths.public_path(&path)))).into_response();
        }
    }
    PATHS.scope(paths, next.run(req)).await
}

async fn not_found(state: &AppState, path: &str, headers: HeaderMap) -> Response {
    crate::render::error_page(state, StatusCode::NOT_FOUND, &public(path), headers).await
}

fn assets() -> String {
    format!(
        r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script>"#,
        link(&format!("/courses/assets/{}/lms.css", css_version())),
        link(&format!("/courses/assets/{}/lms.js", js_version())),
    )
}

async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Who is looking: the member, their tiers, and whether staff.
struct Viewer {
    member: Option<vuuut_api::members::Member>,
    tiers: Vec<String>,
    staff: bool,
}

impl Viewer {
    fn id(&self) -> Option<&str> {
        self.member.as_ref().map(|m| m.id.as_str())
    }
    fn paid(&self) -> bool {
        self.member.as_ref().is_some_and(|m| m.status != "free")
    }
    fn name(&self) -> String {
        self.member.as_ref().map(|m| m.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| m.email.clone())).unwrap_or_default()
    }
}

async fn viewer(state: &AppState, headers: &HeaderMap) -> Viewer {
    let member = vuuut_api::members::current_member(state, headers).await;
    let tiers = match &member {
        Some(m) => vuuut_db::repos::tiers::member_tier_ids(&state.db, &m.id).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let staff = vuuut_api::admin::current_user(state, headers).await.is_some();
    Viewer { member, tiers, staff }
}

fn lms_json(s: &Settings, v: &Viewer) -> Value {
    json!({
        "title": s.title,
        "intro": s.intro,
        "url": link(BASE),
        "dashboard_url": link("/courses/dashboard/"),
        "style_url": link(&format!("/courses/assets/{}/lms.css", css_version())),
        "script_url": link(&format!("/courses/assets/{}/lms.js", js_version())),
        "signed_in": v.member.is_some(),
        "student": v.member.as_ref().map(|_| json!({ "name": v.name(), "initial": v.name().chars().next().map(|c| c.to_uppercase().to_string()) })),
        "reviews": s.reviews,
        "certificates": s.certificates,
    })
}

// ---------------------------------------------------------------------------
// the catalogue
// ---------------------------------------------------------------------------

async fn catalog(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let wanted = q.get("category").map(|c| vuuut_core::directory::slugify(c)).filter(|c| !c.is_empty());
    let cat = match wanted {
        Some(slug) => repo::category_by_slug(&state.db, &slug).await,
        None => None,
    };
    browse(state, headers, q, cat).await
}

async fn category(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}category/{slug}/");
    if !settings(&state).enabled {
        return not_found(&state, &path, headers).await;
    }
    match repo::category_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await {
        Some(c) => browse(state, headers, q, Some(c)).await,
        None => not_found(&state, &path, headers).await,
    }
}

fn stars(rating10: i64) -> String {
    let pct = (rating10 * 2).clamp(0, 100);
    format!(r#"<span class="vl-stars" style="--r:{pct}%" aria-label="Rated {}.{} out of 5"></span>"#, rating10 / 10, rating10 % 10)
}

/// A course card for the built-in catalogue.
fn card(c: &Value) -> String {
    let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let rating = c.get("rating").and_then(Value::as_str).map(|r| {
        let r10 = (r.parse::<f64>().unwrap_or(0.0) * 10.0).round() as i64;
        format!(r#"<span class="vl-rating">{} <b>{r}</b> <span>({})</span></span>"#, stars(r10), c["reviews"])
    }).unwrap_or_default();
    format!(
        r#"<a class="vl-card{feat}" href="{href}"><span class="vl-card-art">{img}{badge}</span><span class="vl-card-body">
<span class="vl-card-meta">{cat}<span>{level}</span></span>
<b class="vl-card-title">{title}</b><span class="vl-card-sub">{sub}</span>
<span class="vl-card-facts">{lessons}{dur}{students}</span>
<span class="vl-card-foot">{rating}<span class="vl-price">{compare}{price}</span></span></span></a>"#,
        feat = if c["featured"].as_bool() == Some(true) { " vl-featured" } else { "" },
        href = esc(&s("url")),
        img = c.get("feature_image").and_then(Value::as_str).map(|u| format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(u))).unwrap_or_else(|| r#"<span class="vl-card-ph"></span>"#.into()),
        badge = if c["featured"].as_bool() == Some(true) { r#"<span class="vl-badge">Bestseller</span>"# } else { "" },
        cat = c.get("category").and_then(|k| k.get("name")).and_then(Value::as_str).map(|n| format!("<span>{}</span>", esc(n))).unwrap_or_default(),
        level = esc(&s("level_label")),
        title = esc(&s("title")),
        sub = esc(&s("excerpt")),
        lessons = format!("<span>{} lessons</span>", c["lessons"]),
        dur = if s("duration").is_empty() { String::new() } else { format!("<span>{}</span>", esc(&s("duration"))) },
        students = c.get("students").and_then(Value::as_i64).filter(|n| *n > 0).map(|n| format!("<span>{n} students</span>")).unwrap_or_default(),
        compare = c.get("compare_price").and_then(Value::as_str).map(|p| format!("<s>{}</s>", esc(p))).unwrap_or_default(),
        price = esc(&s("price")),
    )
}

async fn browse(state: AppState, headers: HeaderMap, q: HashMap<String, String>, cat: Option<repo::Category>) -> Response {
    let s = settings(&state);
    let path = cat.as_ref().map(|c| format!("{BASE}category/{}/", c.slug)).unwrap_or_else(|| BASE.to_string());
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let v = viewer(&state, &headers).await;
    let query = q.get("q").map(|x| x.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = vuuut_core::directory::search_terms(&query);
    let level = q.get("level").map(String::as_str).filter(|l| matches!(*l, "beginner" | "intermediate" | "advanced" | "all"));
    let price = q.get("price").map(String::as_str).filter(|p| matches!(*p, "free" | "paid"));
    let sort = q.get("sort").map(String::as_str).filter(|p| matches!(*p, "newest" | "popular" | "title" | "price")).unwrap_or("");
    let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).max(1);
    let (items, total) = repo::browse(&state.db, &repo::Browse {
        terms: &terms,
        category_id: cat.as_ref().map(|c| c.id.as_str()),
        status: Some("published"),
        level,
        price,
        order: sort,
        limit: s.per_page,
        offset: (page - 1).saturating_mul(s.per_page),
        ..Default::default()
    })
    .await
    .unwrap_or_default();
    let x = repo::extras(&state.db).await;
    let counts = repo::category_counts(&state.db).await;
    let courses: Vec<Value> = items.iter().map(|c| repo::course_json(&s, c, &x)).collect();
    let pages = (total + s.per_page - 1) / s.per_page.max(1);
    let page_link = |n: i64| {
        let mut p = vec![format!("page={n}")];
        if !query.is_empty() { p.push(format!("q={}", vuuut_core::social_media::encode(&query))); }
        if let Some(l) = level { p.push(format!("level={l}")); }
        if let Some(pr) = price { p.push(format!("price={pr}")); }
        if !sort.is_empty() { p.push(format!("sort={sort}")); }
        format!("{}?{}", link(&path), p.join("&"))
    };
    let title = cat.as_ref().map(|c| format!("{} courses", c.name)).unwrap_or_else(|| s.title.clone());
    let intro = cat.as_ref().map(|c| c.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| s.intro.clone());
    let cats_json: Vec<Value> = x.categories.iter().map(|c| {
        let mut j = repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0));
        j["current"] = json!(cat.as_ref().is_some_and(|k| k.id == c.id));
        j
    }).collect();

    let templates = theme_templates(&state, &["lms/courses"]).await;
    if !templates.is_empty() {
        let (featured, _) = repo::browse(&state.db, &repo::Browse { status: Some("published"), featured: true, limit: 6, ..Default::default() }).await.unwrap_or_default();
        let (students, active, done, _) = repo::enrollment_stats(&state.db, "9999").await;
        let _ = (active, done);
        let mut data = serde_json::Map::new();
        data.insert("lms".into(), lms_json(&s, &v));
        data.insert("courses".into(), json!(courses));
        data.insert("featured".into(), json!(featured.iter().map(|c| repo::course_json(&s, c, &x)).collect::<Vec<_>>()));
        data.insert("categories".into(), json!(cats_json));
        data.insert("category".into(), cat.as_ref().map(|c| repo::category_json(&s, c, counts.get(&c.id).copied().unwrap_or(0))).unwrap_or(Value::Null));
        data.insert("instructors".into(), json!(x.instructors.iter().filter(|i| items.iter().chain(featured.iter()).any(|c| c.instructor_id.as_deref() == Some(i.id.as_str()))).map(repo::instructor_json).collect::<Vec<_>>()));
        data.insert("stats".into(), json!({ "courses": total, "students": students, "categories": x.categories.len() }));
        data.insert("filters".into(), json!({ "query": query, "level": level.unwrap_or(""), "price": price.unwrap_or(""), "sort": sort, "searching": !query.is_empty() || level.is_some() || price.is_some() || cat.is_some() }));
        data.insert("heading".into(), json!(title));
        data.insert("intro".into(), json!(intro));
        data.insert("total".into(), json!(total));
        data.insert("pagination".into(), json!({ "page": page, "pages": pages.max(1), "prev_url": (page > 1).then(|| page_link(page - 1)), "next_url": (page < pages).then(|| page_link(page + 1)) }));
        return render_virtual_page_as(&state, &headers, &public(&path), &title, &intro, String::new(), None, false, &templates, data, &["lms"]).await;
    }

    let opt = |v: &str, l: &str, cur: &str| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if v == cur { " selected" } else { "" });
    let chips: String = std::iter::once(format!(r#"<a class="vl-chip{}" href="{}">All</a>"#, if cat.is_none() { " on" } else { "" }, link(BASE)))
        .chain(x.categories.iter().filter(|c| counts.get(&c.id).copied().unwrap_or(0) > 0).map(|c| format!(
            r#"<a class="vl-chip{on}" href="{href}">{n} <span>{count}</span></a>"#,
            on = if cat.as_ref().is_some_and(|k| k.id == c.id) { " on" } else { "" },
            href = link(&format!("{BASE}category/{}/", c.slug)),
            n = esc(&c.name),
            count = counts.get(&c.id).copied().unwrap_or(0),
        )))
        .collect();
    let body = format!(
        r#"{assets}<div class="vl vl-catalog">
<header class="vl-hero"><p class="vl-eyebrow">{eyebrow}</p><h1>{title}</h1>{intro}
<form class="vl-search" method="get" action="{action}" role="search"><input type="search" name="q" value="{q}" placeholder="What do you want to learn?" aria-label="Search courses" maxlength="100">
<select name="level" aria-label="Level">{levels}</select><select name="price" aria-label="Price">{prices}</select><select name="sort" aria-label="Sort">{sorts}</select><button type="submit">Search</button></form></header>
<nav class="vl-chips" aria-label="Categories">{chips}</nav>
<p class="vl-count">{count}{dash}</p>
<div class="vl-grid">{cards}</div>{empty}{pager}</div>"#,
        assets = assets(),
        eyebrow = if cat.is_some() { esc(&s.title) } else { "Learn something new".into() },
        title = esc(&title),
        intro = if intro.is_empty() { String::new() } else { format!(r#"<p class="vl-lede">{}</p>"#, esc(&intro)) },
        action = link(&path),
        q = esc(&query),
        levels = [("", "Any level"), ("beginner", "Beginner"), ("intermediate", "Intermediate"), ("advanced", "Advanced")].iter().map(|(a, b)| opt(a, b, level.unwrap_or(""))).collect::<String>(),
        prices = [("", "Any price"), ("free", "Free"), ("paid", "Paid")].iter().map(|(a, b)| opt(a, b, price.unwrap_or(""))).collect::<String>(),
        sorts = [("", "Recommended"), ("newest", "Newest"), ("popular", "Most viewed"), ("price", "Price"), ("title", "A–Z")].iter().map(|(a, b)| opt(a, b, sort)).collect::<String>(),
        count = if total == 1 { "1 course".to_string() } else { format!("{total} courses") },
        dash = if v.member.is_some() { format!(r#" · <a href="{}">My learning</a>"#, link("/courses/dashboard/")) } else { String::new() },
        cards = courses.iter().map(card).collect::<String>(),
        empty = if courses.is_empty() { r#"<p class="vl-empty">No courses match. Try fewer words or another category.</p>"# } else { "" },
        pager = if pages > 1 { format!(r#"<nav class="vl-pager">{}<span>Page {page} of {pages}</span>{}</nav>"#, if page > 1 { format!(r#"<a href="{}">← Previous</a>"#, esc(&page_link(page - 1))) } else { String::new() }, if page < pages { format!(r#"<a href="{}">Next →</a>"#, esc(&page_link(page + 1))) } else { String::new() }) } else { String::new() },
    );
    render_virtual_page_as(&state, &headers, &public(&path), &title, &intro, body, None, false, &[], serde_json::Map::new(), &["lms"]).await
}

// ---------------------------------------------------------------------------
// access
// ---------------------------------------------------------------------------

/// Why someone can or cannot enrol, as the course page's button.
#[derive(Debug, Clone)]
enum Cta {
    /// Already in: continue.
    Continue,
    /// Open course: start reading.
    Start,
    Enroll,
    SignUp,
    /// `(cart add address, variant id)`.
    Buy(String, String),
    Upgrade,
    Closed(String),
}

async fn enrollment_for(state: &AppState, course: &repo::Course, v: &Viewer) -> Option<repo::Enrollment> {
    let id = v.id()?;
    repo::enrollment(&state.db, &course.id, id).await
}

async fn cta_for(state: &AppState, s: &Settings, c: &repo::Course, v: &Viewer, e: Option<&repo::Enrollment>) -> Cta {
    let now = vuuut_core::dates::now_db();
    if e.is_some_and(|e| e.grants_access(&now)) {
        return Cta::Continue;
    }
    if c.access() == Access::Open {
        return Cta::Start;
    }
    if c.status != "published" {
        return Cta::Closed("This course is not taking new students.".into());
    }
    if let Some(o) = &c.opens_at {
        if *o > now {
            return Cta::Closed(format!("Enrolment opens {}.", o.get(..10).unwrap_or(o)));
        }
    }
    if c.closes_at.as_ref().is_some_and(|x| *x <= now) {
        return Cta::Closed("Enrolment has closed.".into());
    }
    if c.capacity > 0 {
        let taken = repo::student_counts(&state.db).await.get(&c.id).copied().unwrap_or(0);
        if taken >= c.capacity {
            return Cta::Closed("This course is full.".into());
        }
    }
    let _ = s;
    match c.access() {
        Access::Paid => match &c.product_id {
            Some(pid) => match vuuut_db::repos::ecom::find_product(&state.db, pid).await.ok().flatten() {
                Some(p) if p.status == "active" && vuuut_api::ecom::settings(state).enabled => {
                    // Straight into the cart: the course's first variant on sale.
                    let variants = vuuut_db::repos::ecom::variants(&state.db, &p.id).await;
                    match variants.iter().find(|v| v.available(1)) {
                        Some(v) => {
                            let ep = vuuut_api::ecom::paths(state);
                            Cta::Buy(vuuut_core::url::base(&ep.public_path("/shop/cart/add/")), v.id.clone())
                        }
                        None => Cta::Closed("This course is sold out.".into()),
                    }
                }
                _ => Cta::Closed("This course is not on sale right now.".into()),
            },
            None => Cta::Closed("This course is not on sale yet.".into()),
        },
        _ if v.member.is_none() => Cta::SignUp,
        Access::Members if c.tiers().iter().any(|t| t == "paid") && !v.paid() => Cta::Upgrade,
        Access::Tiers if !c.tiers().iter().any(|t| v.tiers.contains(t) || (t == "paid" && v.paid())) => Cta::Upgrade,
        _ => Cta::Enroll,
    }
}

/// The course's call to action as HTML — a form, a link or a note.
fn cta_html(cta: &Cta, c: &repo::Course, s: &Settings, e: Option<&repo::Enrollment>, first_lesson: Option<&str>) -> String {
    let price = rules::format_price(c.price(), &s.currency);
    match cta {
        Cta::Continue => {
            let done = e.is_some_and(|e| e.status == "completed");
            format!(
                r#"<a class="vl-btn vl-btn-lg" href="{href}">{label}</a>{cert}"#,
                href = link(&format!("{BASE}{}/learn/", c.slug)),
                label = if done { "Review the course" } else if e.is_some_and(|e| e.progress > 0) { "Continue learning" } else { "Start learning" },
                cert = e.and_then(|e| e.certificate_code.as_deref()).filter(|_| s.certificates).map(|code| format!(r#"<a class="vl-btn vl-btn-ghost" href="{}">View certificate</a>"#, link(&format!("{BASE}certificate/{code}/")))).unwrap_or_default(),
            )
        }
        Cta::Start => format!(r#"<a class="vl-btn vl-btn-lg" href="{}">Start the course — free</a>"#, first_lesson.map(|l| link(&format!("{BASE}{}/learn/{l}/", c.slug))).unwrap_or_else(|| link(&format!("{BASE}{}/", c.slug)))),
        Cta::Enroll => format!(
            r#"<form method="post" action="{}" class="vl-enroll"><button class="vl-btn vl-btn-lg" type="submit">{}</button></form>"#,
            link(&format!("{BASE}{}/enroll/", c.slug)),
            if c.access() == Access::Free { "Enrol for free" } else { "Enrol now" },
        ),
        Cta::SignUp => r##"<a class="vl-btn vl-btn-lg" href="#/portal/signup" data-portal="signup">Sign up to enrol — free</a><p class="vl-note">Already a member? <a href="#/portal/signin" data-portal="signin">Sign in</a></p>"##.to_string(),
        Cta::Buy(action, variant) => format!(
            r#"<form method="post" action="{}" class="vl-enroll"><input type="hidden" name="variant_id" value="{}"><input type="hidden" name="qty" value="1"><input type="hidden" name="once" value="1"><button class="vl-btn vl-btn-lg" type="submit">Buy now · {}</button></form><p class="vl-note">{} access. Secure checkout.</p>"#,
            esc(action),
            esc(variant),
            esc(&price),
            if c.access_days > 0 { format!("{}-day", c.access_days) } else { "Lifetime".into() },
        ),
        Cta::Upgrade => r##"<a class="vl-btn vl-btn-lg" href="#/portal/account/plans" data-portal="account/plans">Upgrade to enrol</a><p class="vl-note">This course is included with a paid membership.</p>"##.to_string(),
        Cta::Closed(why) => format!(r#"<p class="vl-closed">{}</p>"#, esc(why)),
    }
}

/// Whether a lesson can be opened, and why not.
enum Gate {
    Open { tracked: bool },
    Locked(String),
}

fn gate(c: &repo::Course, l: &repo::Lesson, all: &[repo::Lesson], e: Option<&repo::Enrollment>, done: &[String], staff: bool) -> Gate {
    let now = vuuut_core::dates::now_db();
    if staff && e.is_none() {
        return Gate::Open { tracked: false };
    }
    if c.access() == Access::Open && e.is_none() {
        return Gate::Open { tracked: false };
    }
    let Some(e) = e.filter(|e| e.grants_access(&now)) else {
        if l.is_preview() {
            return Gate::Open { tracked: false };
        }
        return Gate::Locked(if e.is_some() { "Your access to this course has ended.".into() } else { "Enrol to open this lesson.".into() });
    };
    if let Some(at) = rules::unlocks_at(&e.enrolled_at, l.drip_days) {
        if at > now {
            return Gate::Locked(format!("This lesson opens on {}.", at.get(..10).unwrap_or(&at)));
        }
    }
    if c.sequential != 0 {
        if let Some(blocker) = all.iter().take_while(|x| x.id != l.id).find(|x| !done.contains(&x.id)) {
            return Gate::Locked(format!("Finish “{}” first.", blocker.title));
        }
    }
    Gate::Open { tracked: true }
}

// ---------------------------------------------------------------------------
// the course page
// ---------------------------------------------------------------------------

async fn published_course(state: &AppState, slug: &str) -> Option<repo::Course> {
    let c = repo::course_by_slug(&state.db, &vuuut_core::directory::slugify(slug)).await?;
    (c.status == "published" || c.status == "archived").then_some(c)
}

fn curriculum_json(c: &repo::Course, sections: &[repo::Section], lessons: &[repo::Lesson], e: Option<&repo::Enrollment>, done: &[String], current: Option<&str>, staff: bool) -> Vec<Value> {
    let item = |l: &repo::Lesson| {
        let g = gate(c, l, lessons, e, done, staff);
        json!({
            "id": l.id,
            "title": l.title,
            "summary": l.summary,
            "kind": l.kind,
            "kind_label": l.kind().label(),
            "duration": rules::duration_label(l.duration_minutes),
            "video": !l.video_url.is_empty(),
            "preview": l.is_preview(),
            "url": link(&format!("{BASE}{}/learn/{}/", c.slug, l.slug)),
            "done": done.contains(&l.id),
            "locked": matches!(g, Gate::Locked(_)),
            "current": current == Some(l.id.as_str()),
        })
    };
    let mut out: Vec<Value> = sections
        .iter()
        .map(|s| {
            let ls: Vec<&repo::Lesson> = lessons.iter().filter(|l| l.section_id.as_deref() == Some(s.id.as_str())).collect();
            let minutes: i64 = ls.iter().map(|l| l.duration_minutes).sum();
            json!({
                "title": s.title,
                "count": ls.len(),
                "duration": rules::duration_label(minutes),
                "done": ls.iter().filter(|l| done.contains(&l.id)).count(),
                "lessons": ls.iter().map(|l| item(l)).collect::<Vec<_>>(),
                "open": current.is_some_and(|cur| ls.iter().any(|l| l.id == cur)),
            })
        })
        .filter(|s| s["count"].as_u64().unwrap_or(0) > 0)
        .collect();
    let loose: Vec<&repo::Lesson> = lessons.iter().filter(|l| l.section_id.as_ref().is_none_or(|sid| !sections.iter().any(|s| &s.id == sid))).collect();
    if !loose.is_empty() {
        out.push(json!({ "title": if sections.is_empty() { "Lessons" } else { "More" }, "count": loose.len(), "duration": "", "done": loose.iter().filter(|l| done.contains(&l.id)).count(), "lessons": loose.iter().map(|l| item(l)).collect::<Vec<_>>(), "open": true }));
    }
    if let Some(first) = out.first_mut() {
        if current.is_none() {
            first["open"] = json!(true);
        }
    }
    out
}

fn icon(kind: &str, done: bool, locked: bool) -> &'static str {
    if done {
        return r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 16.2 4.8 12l-1.4 1.4L9 19 21 7l-1.4-1.4z"/></svg>"#;
    }
    if locked {
        return r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 8h-1V6A5 5 0 0 0 7 6v2H6a2 2 0 0 0-2 2v10c0 1.1.9 2 2 2h12a2 2 0 0 0 2-2V10a2 2 0 0 0-2-2zM9 6a3 3 0 0 1 6 0v2H9V6z"/></svg>"#;
    }
    match kind {
        "quiz" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 18h2v-2h-2v2zm1-16a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 18a8 8 0 1 1 0-16 8 8 0 0 1 0 16zm0-14a4 4 0 0 0-4 4h2a2 2 0 1 1 4 0c0 2-3 1.75-3 5h2c0-2.25 3-2.5 3-5a4 4 0 0 0-4-4z"/></svg>"#,
        "assignment" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 17.25V21h3.75L17.8 9.94l-3.75-3.75L3 17.25zM20.7 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>"#,
        _ => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>"#,
    }
}

fn curriculum_html(cur: &[Value], compact: bool) -> String {
    cur.iter()
        .enumerate()
        .map(|(i, sec)| {
            let items: String = sec["lessons"].as_array().map(|a| a.iter().map(|l| {
                let locked = l["locked"].as_bool() == Some(true);
                let done = l["done"].as_bool() == Some(true);
                let inner = format!(
                    r#"<span class="vl-li-ic">{ic}</span><span class="vl-li-t">{t}</span><span class="vl-li-m">{pv}{d}</span>"#,
                    ic = icon(l["kind"].as_str().unwrap_or(""), done, locked),
                    t = esc(l["title"].as_str().unwrap_or("")),
                    pv = if l["preview"].as_bool() == Some(true) && locked == false && !compact { r#"<em>Preview</em>"# } else { "" },
                    d = esc(l["duration"].as_str().unwrap_or("")),
                );
                let cls = format!("vl-li{}{}{}", if done { " done" } else { "" }, if locked { " locked" } else { "" }, if l["current"].as_bool() == Some(true) { " current" } else { "" });
                if locked && !compact {
                    format!(r#"<li class="{cls}">{inner}</li>"#)
                } else {
                    format!(r#"<li class="{cls}"><a href="{}"{}>{inner}</a></li>"#, esc(l["url"].as_str().unwrap_or("")), if l["current"].as_bool() == Some(true) { r#" aria-current="page""# } else { "" })
                }
            }).collect()).unwrap_or_default();
            format!(
                r#"<details class="vl-sec"{open}><summary><span class="vl-sec-n">{n:02}</span><span class="vl-sec-t">{t}</span><span class="vl-sec-m">{m}</span></summary><ol>{items}</ol></details>"#,
                open = if sec["open"].as_bool() == Some(true) || !compact { " open" } else { "" },
                n = i + 1,
                t = esc(sec["title"].as_str().unwrap_or("")),
                m = esc(&format!("{}{}", if compact { format!("{}/{}", sec["done"], sec["count"]) } else { format!("{} lessons", sec["count"]) }, sec["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {d}")).unwrap_or_default())),
            )
        })
        .collect()
}

fn review_form(c: &repo::Course, mine: Option<&repo::Review>) -> String {
    let r = mine.map(|r| r.rating).unwrap_or(0);
    format!(
        r#"<form class="vl-review-form" method="post" action="{action}"><p class="vl-rate" role="radiogroup" aria-label="Your rating">{radios}</p>
<input name="title" maxlength="120" placeholder="Sum it up in a few words" value="{t}"><textarea name="body" rows="3" maxlength="2000" placeholder="What did you think? What would you tell a friend?">{b}</textarea>
<button class="vl-btn" type="submit">{label}</button></form>"#,
        action = link(&format!("{BASE}{}/review/", c.slug)),
        radios = (1..=5).rev().map(|n| format!(r#"<input type="radio" id="vl-r{n}" name="rating" value="{n}"{c}><label for="vl-r{n}" title="{n} star{p}">★</label>"#, c = if n == r { " checked" } else { "" }, p = if n == 1 { "" } else { "s" })).collect::<String>(),
        t = esc(mine.map(|r| r.title.as_str()).unwrap_or("")),
        b = esc(mine.map(|r| r.body.as_str()).unwrap_or("")),
        label = if mine.is_some() { "Update review" } else { "Post review" },
    )
}

async fn course_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(c) = published_course(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let v = viewer(&state, &headers).await;
    let e = enrollment_for(&state, &c, &v).await;
    if c.status == "archived" && e.is_none() && !v.staff {
        return not_found(&state, &path, headers).await;
    }
    let agent = headers.get(header::USER_AGENT).and_then(|x| x.to_str().ok()).unwrap_or("").to_lowercase();
    if !["bot", "crawler", "spider", "preview"].iter().any(|b| agent.contains(b)) {
        repo::add_view(&state.db, &c.id).await;
    }
    let x = repo::extras(&state.db).await;
    let sections = repo::sections(&state.db, &c.id).await;
    let lessons: Vec<repo::Lesson> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").collect();
    let done = match &e {
        Some(e) => repo::completed_lessons(&state.db, &e.id).await,
        None => Vec::new(),
    };
    let cta = cta_for(&state, &s, &c, &v, e.as_ref()).await;
    let first = lessons.first().map(|l| l.slug.as_str());
    let cur = curriculum_json(&c, &sections, &lessons, e.as_ref(), &done, None, v.staff);
    let reviews = if s.reviews { repo::reviews_for_course(&state.db, &c.id, 30).await } else { Vec::new() };
    let mine = match (&e, v.id()) {
        (Some(_), Some(m)) if s.reviews => repo::review_by(&state.db, &c.id, m).await,
        _ => None,
    };
    let mut breakdown = [0i64; 5];
    for r in &reviews {
        if (1..=5).contains(&r.rating) {
            breakdown[(5 - r.rating) as usize] += 1;
        }
    }
    let mut cj = repo::course_json(&s, &c, &x);
    let absolute = vuuut_core::url::absolute(&state.site_url(), &link(&path));
    let schema = {
        let mut o = json!({
            "@context": "https://schema.org", "@type": "Course", "name": c.title, "description": if c.subtitle.is_empty() { c.description.chars().take(300).collect::<String>() } else { c.subtitle.clone() },
            "url": absolute, "provider": { "@type": "Organization", "name": state.settings.get_str("title").unwrap_or_default(), "sameAs": state.site_url() },
        });
        if !c.image.is_empty() {
            o["image"] = json!(c.image);
        }
        o["offers"] = json!({ "@type": "Offer", "category": if c.price() > 0 { "Paid" } else { "Free" }, "price": format!("{}.{:02}", c.price() / 100, c.price() % 100), "priceCurrency": s.currency });
        o["hasCourseInstance"] = json!({ "@type": "CourseInstance", "courseMode": "Online", "courseWorkload": format!("PT{}M", cj["duration_minutes"].as_i64().unwrap_or(0)) });
        if let Some((r, n)) = x.ratings.get(&c.id) {
            o["aggregateRating"] = json!({ "@type": "AggregateRating", "ratingValue": format!("{}.{}", r / 10, r % 10), "reviewCount": n });
        }
        o.to_string().replace("</", "<\\/")
    };
    let enrolled = matches!(cta, Cta::Continue);
    let reviews_json: Vec<Value> = reviews.iter().map(|r| json!({
        "rating": r.rating, "title": r.title, "body": r.body, "date": r.created_at.get(..10),
        "name": if r.name.is_empty() { r.email.split('@').next().unwrap_or("Student").to_string() } else { r.name.clone() },
        "initial": r.name.chars().next().or_else(|| r.email.chars().next()).map(|c| c.to_uppercase().to_string()),
        "stars": "★".repeat(r.rating.clamp(0, 5) as usize),
    })).collect();
    let forms = json!({
        "cta": cta_html(&cta, &c, &s, e.as_ref(), first),
        "review": if enrolled && s.reviews && v.member.is_some() { json!(review_form(&c, mine.as_ref())) } else { Value::Null },
    });
    cj["curriculum"] = json!(cur);
    cj["sections_count"] = json!(cur.len());
    cj["schema"] = json!(schema);
    cj["reviews_list"] = json!(reviews_json);
    cj["breakdown"] = json!(breakdown.iter().enumerate().map(|(i, n)| json!({ "stars": 5 - i, "count": n, "percent": if reviews.is_empty() { 0 } else { n * 100 / reviews.len() as i64 } })).collect::<Vec<_>>());
    cj["enrolment"] = json!({
        "enrolled": enrolled,
        "progress": e.as_ref().map(|e| e.progress).unwrap_or(0),
        "completed": e.as_ref().is_some_and(|e| e.status == "completed"),
        "continue_url": link(&format!("{BASE}{}/learn/", c.slug)),
        "certificate_url": e.as_ref().and_then(|e| e.certificate_code.as_deref()).filter(|_| s.certificates).map(|code| link(&format!("{BASE}certificate/{code}/"))),
        "done": done.len(),
        "total": lessons.len(),
    });
    let page_title = if c.seo_title.is_empty() { c.title.clone() } else { c.seo_title.clone() };
    let excerpt = if !c.seo_description.is_empty() { c.seo_description.clone() } else { c.subtitle.clone() };
    let flash = q.get("ok").or_else(|| q.get("error")).map(|m| (q.contains_key("error"), m.chars().take(200).collect::<String>()));

    let templates = theme_templates(&state, &["lms/course"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("lms".into(), lms_json(&s, &v));
        data.insert("course".into(), cj);
        data.insert("forms".into(), forms);
        data.insert("flash".into(), flash.map(|(err, m)| json!({ "error": err, "message": m })).unwrap_or(Value::Null));
        let related = if let Some(cat) = &c.category_id { repo::browse(&state.db, &repo::Browse { status: Some("published"), category_id: Some(cat), limit: 4, ..Default::default() }).await.map(|r| r.0).unwrap_or_default() } else { Vec::new() };
        data.insert("related".into(), json!(related.iter().filter(|r| r.id != c.id).take(3).map(|r| repo::course_json(&s, r, &x)).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), &page_title, &excerpt, String::new(), (!c.image.is_empty()).then(|| c.image.clone()), false, &templates, data, &["lms"]).await;
    }

    let list = |k: &str, title: &str, cls: &str| -> String {
        let items: Vec<String> = cj[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| format!("<li>{}</li>", esc(x))).collect()).unwrap_or_default();
        if items.is_empty() { String::new() } else { format!(r#"<section class="vl-box {cls}"><h2>{title}</h2><ul>{}</ul></section>"#, items.join("")) }
    };
    let instructor = cj["instructor"].clone();
    let rating_html = cj["rating"].as_str().map(|r| format!(r#"<span class="vl-rating">{} <b>{r}</b> <span>({} reviews)</span></span>"#, stars(cj["rating_percent"].as_i64().unwrap_or(0) / 2), cj["reviews"])).unwrap_or_default();
    let video = cj["intro_video"].clone();
    let media = if video.is_object() {
        if video["iframe"].as_bool() == Some(true) { format!(r#"<div class="vl-video"><iframe src="{}" title="Introduction" loading="lazy" allowfullscreen allow="autoplay; encrypted-media; picture-in-picture"></iframe></div>"#, esc(video["url"].as_str().unwrap_or(""))) } else { format!(r#"<div class="vl-video"><video src="{}" controls preload="metadata"></video></div>"#, esc(video["url"].as_str().unwrap_or(""))) }
    } else {
        cj["feature_image"].as_str().map(|u| format!(r#"<img class="vl-cover" src="{}" alt="">"#, esc(u))).unwrap_or_default()
    };
    let progress = if enrolled { format!(r#"<div class="vl-progress"><span style="width:{p}%"></span></div><p class="vl-note">{p}% complete · {d} of {t} lessons</p>"#, p = e.as_ref().map(|e| e.progress).unwrap_or(0), d = done.len(), t = lessons.len()) } else { String::new() };
    let reviews_html = if !s.reviews { String::new() } else {
        format!(
            r#"<section class="vl-reviews" id="reviews"><h2>Student reviews</h2>{summary}{form}<div class="vl-review-list">{list}</div></section>"#,
            summary = cj["rating"].as_str().map(|r| format!(
                r#"<div class="vl-review-sum"><b>{r}</b>{st}<span>{n} reviews</span><div class="vl-bars">{bars}</div></div>"#,
                st = stars(cj["rating_percent"].as_i64().unwrap_or(0) / 2),
                n = reviews.len(),
                bars = cj["breakdown"].as_array().map(|a| a.iter().map(|b| format!(r#"<span><i>{}★</i><em style="--w:{}%"></em><i>{}</i></span>"#, b["stars"], b["percent"], b["count"])).collect::<String>()).unwrap_or_default(),
            )).unwrap_or_else(|| r#"<p class="vl-note">No reviews yet.</p>"#.into()),
            form = forms["review"].as_str().map(|f| format!(r#"<details class="vl-write"{}><summary>{}</summary>{f}</details>"#, if mine.is_none() { "" } else { "" }, if mine.is_some() { "Edit your review" } else { "Write a review" })).unwrap_or_default(),
            list = reviews_json.iter().map(|r| format!(r#"<article class="vl-review"><header><span class="vl-ava">{i}</span><b>{n}</b><span class="vl-stars-t">{s}</span><time>{d}</time></header>{t}<p>{b}</p></article>"#, i = esc(r["initial"].as_str().unwrap_or("")), n = esc(r["name"].as_str().unwrap_or("")), s = r["stars"].as_str().unwrap_or(""), d = esc(r["date"].as_str().unwrap_or("")), t = r["title"].as_str().filter(|t| !t.is_empty()).map(|t| format!("<h3>{}</h3>", esc(t))).unwrap_or_default(), b = esc(r["body"].as_str().unwrap_or("")))).collect::<String>(),
        )
    };
    let body = format!(
        r#"{assets}<div class="vl vl-course">{flash}
<nav class="vl-crumbs"><a href="{home}">{home_t}</a>{cat_crumb}</nav>
<header class="vl-course-hero"><div class="vl-course-intro"><p class="vl-eyebrow">{level}{cat}</p><h1>{title}</h1><p class="vl-lede">{sub}</p>
<p class="vl-course-meta">{rating_html}{students}<span>{lessons} lessons</span>{dur}{lang}</p>
{ins}</div>
<aside class="vl-buy">{media}<div class="vl-buy-body"><p class="vl-buy-price">{compare}<b>{price}</b></p>{progress}{cta}
<ul class="vl-includes"><li>{lessons} lessons{dur_inc}</li>{quizzes}<li>Learn at your own pace{access}</li>{cert}</ul></div></aside></header>
<div class="vl-course-grid"><div class="vl-course-main">
{learn}
<section class="vl-box"><h2>About this course</h2><div class="vl-prose">{desc}</div></section>
<section class="vl-curriculum" id="curriculum"><div class="vl-cur-head"><h2>Course content</h2><p>{nsec} sections · {lessons} lessons{dur_dot}</p></div>{cur}</section>
{req}{aud}
{reviews_html}</div></div>
<script type="application/ld+json">{schema}</script></div>"#,
        assets = assets(),
        flash = flash.as_ref().map(|(err, m)| format!(r#"<p class="vl-flash{}" role="status">{}</p>"#, if *err { " vl-flash-err" } else { "" }, esc(m))).unwrap_or_default(),
        home = link(BASE),
        home_t = esc(&s.title),
        cat_crumb = cj["category"].as_object().map(|k| format!(r#" › <a href="{}">{}</a>"#, esc(k["url"].as_str().unwrap_or("")), esc(k["name"].as_str().unwrap_or("")))).unwrap_or_default(),
        level = esc(cj["level_label"].as_str().unwrap_or("")),
        cat = cj["category"]["name"].as_str().map(|n| format!(" · {}", esc(n))).unwrap_or_default(),
        title = esc(&c.title),
        sub = esc(&c.subtitle),
        students = cj["students"].as_i64().filter(|n| *n > 0).map(|n| format!("<span>{n} students</span>")).unwrap_or_default(),
        lessons = lessons.len(),
        dur = cj["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!("<span>{}</span>", esc(d))).unwrap_or_default(),
        lang = if c.language.is_empty() { String::new() } else { format!("<span>{}</span>", esc(&c.language)) },
        ins = if instructor.is_object() { format!(r#"<p class="vl-by">{img}<span>Created by <b>{n}</b></span></p>"#, img = instructor["profile_image"].as_str().map(|u| format!(r#"<img src="{}" alt="">"#, esc(u))).unwrap_or_else(|| format!(r#"<span class="vl-ava">{}</span>"#, esc(instructor["initials"].as_str().unwrap_or("")))), n = esc(instructor["name"].as_str().unwrap_or(""))) } else { String::new() },
        compare = cj["compare_price"].as_str().map(|p| format!("<s>{}</s> ", esc(p))).unwrap_or_default(),
        price = esc(cj["price"].as_str().unwrap_or("")),
        cta = forms["cta"].as_str().unwrap_or(""),
        dur_inc = cj["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {}", esc(d))).unwrap_or_default(),
        quizzes = { let n = lessons.iter().filter(|l| l.kind() != LessonKind::Lesson).count(); if n > 0 { format!("<li>{n} quizzes and assignments</li>") } else { String::new() } },
        access = if c.access_days > 0 { format!(" · {} days of access", c.access_days) } else { " · lifetime access".into() },
        cert = if cj["certificate"].as_bool() == Some(true) { "<li>Certificate of completion</li>" } else { "" },
        learn = list("outcomes", "What you'll learn", "vl-learn"),
        desc = c.description_markup(),
        nsec = cur.len(),
        dur_dot = cj["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {} total", esc(d))).unwrap_or_default(),
        cur = curriculum_html(&cur, false),
        req = list("requirements", "Requirements", ""),
        aud = list("audience", "Who this course is for", ""),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &page_title, &excerpt, body, (!c.image.is_empty()).then(|| c.image.clone()), false, &[], serde_json::Map::new(), &["lms"]).await
}

// ---------------------------------------------------------------------------
// enrolling and reviewing
// ---------------------------------------------------------------------------

fn same_origin(state: &AppState, headers: &HeaderMap) -> bool {
    vuuut_api::forms::same_origin(headers, &state.site_url())
}

fn back_to(path: &str, kind: &str, msg: &str) -> Response {
    Redirect::to(&format!("{}?{kind}={}", link(path), vuuut_core::social_media::encode(msg))).into_response()
}

async fn enroll(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let Some(c) = published_course(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let v = viewer(&state, &headers).await;
    let e = enrollment_for(&state, &c, &v).await;
    match cta_for(&state, &s, &c, &v, e.as_ref()).await {
        Cta::Enroll => {
            let Some(mid) = v.id() else { return back_to(&path, "error", "Sign in to enrol.") };
            let source = match c.access() { Access::Tiers | Access::Members => "tier", _ => "free" };
            match repo::enroll(&state.db, &c, mid, source, None).await {
                Ok(_) => Redirect::to(&link(&format!("{BASE}{}/learn/", c.slug))).into_response(),
                Err(e) => {
                    tracing::error!("lms: enrolling in {} failed: {}", c.slug, e.message());
                    back_to(&path, "error", "That did not work. Please try again.")
                }
            }
        }
        Cta::Continue => Redirect::to(&link(&format!("{BASE}{}/learn/", c.slug))).into_response(),
        Cta::Buy(..) => Redirect::to(&vuuut_core::url::base(&vuuut_api::ecom::paths(&state).public_path("/shop/cart/"))).into_response(),
        Cta::SignUp => back_to(&path, "error", "Sign up or sign in first — it is free."),
        Cta::Upgrade => back_to(&path, "error", "This course is part of a paid membership."),
        Cta::Start => Redirect::to(&link(&path)).into_response(),
        Cta::Closed(why) => back_to(&path, "error", &why),
    }
}

async fn review(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled || !s.reviews {
        return not_found(&state, &path, headers).await;
    }
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let Some(c) = published_course(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let v = viewer(&state, &headers).await;
    let now = vuuut_core::dates::now_db();
    let Some(e) = enrollment_for(&state, &c, &v).await.filter(|e| e.grants_access(&now)) else {
        return back_to(&path, "error", "Only students of this course can review it.");
    };
    let (rating, title, body) = match rules::review_from_form(&form) {
        Ok(r) => r,
        Err(m) => return back_to(&format!("{path}#reviews"), "error", &m),
    };
    let status = if s.moderate_reviews { "pending" } else { "published" };
    match repo::save_review(&state.db, &c.id, &e.member_id, rating, &title, &body, status).await {
        Ok(()) => back_to(&path, "ok", if status == "pending" { "Thank you! Your review will appear once it has been approved." } else { "Thank you for your review!" }),
        Err(_) => back_to(&path, "error", "Your review could not be saved."),
    }
}

// ---------------------------------------------------------------------------
// learning
// ---------------------------------------------------------------------------

async fn resume(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/learn/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(c) = published_course(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let v = viewer(&state, &headers).await;
    let e = enrollment_for(&state, &c, &v).await;
    let lessons: Vec<repo::Lesson> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").collect();
    let done = match &e { Some(e) => repo::completed_lessons(&state.db, &e.id).await, None => Vec::new() };
    let next = lessons.iter().find(|l| !done.contains(&l.id)).or_else(|| e.as_ref().and_then(|e| e.last_lesson_id.as_ref()).and_then(|id| lessons.iter().find(|l| &l.id == id))).or(lessons.first());
    match next {
        Some(l) => Redirect::to(&link(&format!("{BASE}{}/learn/{}/", c.slug, l.slug))).into_response(),
        None => Redirect::to(&link(&format!("{BASE}{}/", c.slug))).into_response(),
    }
}

struct LessonCtx {
    s: Settings,
    c: repo::Course,
    l: repo::Lesson,
    lessons: Vec<repo::Lesson>,
    v: Viewer,
    e: Option<repo::Enrollment>,
    done: Vec<String>,
}

async fn lesson_ctx(state: &AppState, headers: &HeaderMap, slug: &str, lesson: &str) -> Option<LessonCtx> {
    let s = settings(state);
    if !s.enabled {
        return None;
    }
    let c = published_course(state, slug).await?;
    let lessons: Vec<repo::Lesson> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").collect();
    let wanted = vuuut_core::directory::slugify(lesson);
    let l = lessons.iter().find(|l| l.slug == wanted)?.clone();
    let v = viewer(state, headers).await;
    let e = enrollment_for(state, &c, &v).await;
    let done = match &e { Some(e) => repo::completed_lessons(&state.db, &e.id).await, None => Vec::new() };
    Some(LessonCtx { s, c, l, lessons, v, e, done })
}

fn lesson_url(c: &repo::Course, l: &repo::Lesson) -> String {
    link(&format!("{BASE}{}/learn/{}/", c.slug, l.slug))
}

/// A seeded shuffle: the same try sees the same order on reload.
fn shuffled<T: Clone>(items: &[T], seed: &str) -> Vec<T> {
    let mut keyed: Vec<(u64, T)> = items
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let mut h: u64 = 0xcbf29ce484222325 ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            for b in seed.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            (h, x.clone())
        })
        .collect();
    keyed.sort_by_key(|(k, _)| *k);
    keyed.into_iter().map(|(_, x)| x).collect()
}

async fn quiz_html(state: &AppState, x: &LessonCtx, tracked: bool, q: &HashMap<String, String>) -> String {
    let questions = repo::questions(&state.db, &x.l.id).await;
    if questions.is_empty() {
        return r#"<p class="vl-note">This quiz has no questions yet.</p>"#.into();
    }
    let attempts = match &x.e { Some(e) => repo::attempts(&state.db, &e.id, &x.l.id).await, None => Vec::new() };
    let passed = attempts.iter().any(|a| a.passed != 0);
    let left = if x.l.attempts > 0 { (x.l.attempts - attempts.len() as i64).max(0) } else { i64::MAX };
    let shown = q.get("attempt").and_then(|id| attempts.iter().find(|a| &a.id == id)).or(if q.contains_key("attempt") { attempts.first() } else { None });
    let result = shown.map(|a| {
        let marks: Vec<Value> = serde_json::from_str(&a.answers).unwrap_or_default();
        let review = if x.l.reveal != 0 {
            questions.iter().enumerate().map(|(i, qq)| {
                let m = marks.iter().find(|m| m["id"].as_str() == Some(qq.id.as_str()));
                let ok = m.and_then(|m| m["correct"].as_bool()).unwrap_or(false);
                let given = m.and_then(|m| m["given"].as_array()).map(|g| g.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let right = match qq.kind() { rules::QuestionKind::Text => qq.answers.first().cloned().unwrap_or_default(), _ => qq.options.iter().filter(|o| o.c).map(|o| o.t.clone()).collect::<Vec<_>>().join(", ") };
                format!(
                    r#"<li class="{cls}"><b>{n}. {p}</b><span>Your answer: {g}</span>{r}{ex}</li>"#,
                    cls = if ok { "ok" } else { "no" },
                    n = i + 1,
                    p = esc(&qq.prompt),
                    g = if given.is_empty() { "—".into() } else { esc(&given) },
                    r = if ok { String::new() } else { format!("<span>Right answer: {}</span>", esc(&right)) },
                    ex = if qq.explanation.is_empty() { String::new() } else { format!("<em>{}</em>", esc(&qq.explanation)) },
                )
            }).collect::<String>()
        } else {
            String::new()
        };
        format!(
            r#"<section class="vl-result {cls}"><div class="vl-score" style="--p:{score}"><b>{score}%</b></div><div><h2>{head}</h2><p>{pts} of {max} points · pass mark {pass}%</p></div></section>{rev}"#,
            cls = if a.passed != 0 { "pass" } else { "fail" },
            score = a.score,
            head = if a.passed != 0 { "Passed — well done!" } else { "Not quite yet" },
            pts = a.points,
            max = a.max_points,
            pass = x.l.pass_mark,
            rev = if review.is_empty() { String::new() } else { format!(r#"<ol class="vl-answers">{review}</ol>"#) },
        )
    }).unwrap_or_default();
    if !tracked {
        let list: String = questions.iter().enumerate().map(|(i, qq)| format!("<li>{}. {}</li>", i + 1, esc(&qq.prompt))).collect();
        return format!(r#"<div class="vl-quiz-preview"><p class="vl-note">{} questions · pass mark {}%. Enrol to take the quiz.</p><ol>{list}</ol></div>"#, questions.len(), x.l.pass_mark);
    }
    let history = if attempts.is_empty() { String::new() } else {
        format!(r#"<p class="vl-note">Your tries: {}</p>"#, attempts.iter().rev().map(|a| format!(r#"<a href="?attempt={}">{}%</a>"#, esc(&a.id), a.score)).collect::<Vec<_>>().join(" · "))
    };
    if left == 0 && !passed {
        return format!(r#"{result}<p class="vl-closed">You have used all {} tries for this quiz.</p>{history}"#, x.l.attempts);
    }
    if shown.is_some() {
        let again = if left > 0 { format!(r#"<a class="vl-btn vl-btn-ghost" href="{}">Try again{}</a>"#, lesson_url(&x.c, &x.l), if x.l.attempts > 0 { format!(" ({left} left)") } else { String::new() }) } else { String::new() };
        return format!(r#"{result}<div class="vl-actions">{again}</div>{history}"#);
    }
    let seed = format!("{}{}", x.e.as_ref().map(|e| e.id.as_str()).unwrap_or(""), attempts.len());
    let order = if x.l.shuffle != 0 { shuffled(&questions, &seed) } else { questions.clone() };
    let items: String = order.iter().enumerate().map(|(i, qq)| {
        let name = format!("q_{}", qq.id);
        let body = match qq.kind() {
            rules::QuestionKind::Text => format!(r#"<input class="vl-in" name="{name}" maxlength="100" autocomplete="off" placeholder="Type your answer">"#),
            k => qq.options.iter().enumerate().map(|(j, o)| format!(
                r#"<label class="vl-opt"><input type="{t}" name="{name}" value="{j}"><span>{txt}</span></label>"#,
                t = if k == rules::QuestionKind::Multiple { "checkbox" } else { "radio" },
                txt = esc(&o.t),
            )).collect::<String>(),
        };
        format!(
            r#"<fieldset class="vl-q"><legend><span class="vl-q-n">{n}</span>{p}{hint}</legend><div class="vl-opts">{body}</div></fieldset>"#,
            n = i + 1,
            p = esc(&qq.prompt),
            hint = if qq.kind() == rules::QuestionKind::Multiple { r#"<small>Choose all that apply</small>"# } else { "" },
        )
    }).collect();
    format!(
        r#"{history}<form class="vl-quiz" method="post" action="{action}" data-limit="{limit}"><input type="hidden" name="started" value="{t}">
<div class="vl-quiz-bar"><span>{n} questions · pass mark {pass}%{tries}</span>{timer}</div>{items}
<div class="vl-actions"><button class="vl-btn vl-btn-lg" type="submit">Submit answers</button></div></form>"#,
        action = link(&format!("{BASE}{}/learn/{}/quiz/", x.c.slug, x.l.slug)),
        limit = x.l.time_limit,
        t = now_ms(),
        n = questions.len(),
        pass = x.l.pass_mark,
        tries = if x.l.attempts > 0 { format!(" · {left} of {} tries left", x.l.attempts) } else { String::new() },
        timer = if x.l.time_limit > 0 { format!(r#"<span class="vl-timer" aria-live="polite">{}:00</span>"#, x.l.time_limit) } else { String::new() },
    )
}

async fn assignment_html(state: &AppState, x: &LessonCtx, tracked: bool) -> String {
    if !tracked {
        return r#"<p class="vl-note">Enrol to hand in this assignment.</p>"#.into();
    }
    let Some(e) = &x.e else { return String::new() };
    let sub = repo::submission(&state.db, &e.id, &x.l.id).await;
    let status = sub.as_ref().map(|s| match s.status.as_str() {
        "passed" => format!(r#"<section class="vl-result pass"><div class="vl-score" style="--p:{p}"><b>{g}</b></div><div><h2>Passed</h2><p>Graded {g} of {max}.{fb}</p></div></section>"#, p = s.grade * 100 / x.l.points.max(1), g = s.grade, max = x.l.points, fb = if s.feedback.is_empty() { String::new() } else { format!(" “{}”", esc(&s.feedback)) }),
        "failed" => format!(r#"<section class="vl-result fail"><div><h2>Returned for another go</h2><p>{}</p></div></section>"#, if s.feedback.is_empty() { "Have another look and hand it in again.".into() } else { esc(&s.feedback) }),
        _ => format!(r#"<section class="vl-result wait"><div><h2>Handed in — waiting for a grade</h2><p>Sent {}. You can still change it below.</p></div></section>"#, esc(s.submitted_at.get(..10).unwrap_or(""))),
    }).unwrap_or_default();
    if sub.as_ref().is_some_and(|s| s.status == "passed") {
        return status;
    }
    format!(
        r#"{status}<form class="vl-assign" method="post" action="{action}"><label>Your answer<textarea name="body" rows="8" maxlength="20000" placeholder="Write your answer, or describe what you made…">{b}</textarea></label>
<label>Link to your work (optional)<input class="vl-in" name="link" type="url" maxlength="500" placeholder="https://" value="{lk}"></label>
<div class="vl-actions"><button class="vl-btn vl-btn-lg" type="submit">{label}</button></div></form>"#,
        action = link(&format!("{BASE}{}/learn/{}/submit/", x.c.slug, x.l.slug)),
        b = esc(sub.as_ref().map(|s| s.body.as_str()).unwrap_or("")),
        lk = esc(sub.as_ref().map(|s| s.link.as_str()).unwrap_or("")),
        label = if sub.is_some() { "Hand in again" } else { "Hand in assignment" },
    )
}

async fn lesson_page(State(state): State<AppState>, headers: HeaderMap, Path((slug, lesson)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}{slug}/learn/{lesson}/");
    let Some(x) = lesson_ctx(&state, &headers, &slug, &lesson).await else { return not_found(&state, &path, headers).await };
    let g = gate(&x.c, &x.l, &x.lessons, x.e.as_ref(), &x.done, x.v.staff);
    let tracked = matches!(g, Gate::Open { tracked: true });
    if let (true, Some(e)) = (tracked, &x.e) {
        repo::touch_enrollment(&state.db, &e.id, &x.l.id).await;
    }
    let pos = x.lessons.iter().position(|l| l.id == x.l.id).unwrap_or(0);
    let prev = pos.checked_sub(1).and_then(|p| x.lessons.get(p));
    let next = x.lessons.get(pos + 1);
    let sections = repo::sections(&state.db, &x.c.id).await;
    let cur = curriculum_json(&x.c, &sections, &x.lessons, x.e.as_ref(), &x.done, Some(&x.l.id), x.v.staff);
    let is_done = x.done.contains(&x.l.id);
    let kind = x.l.kind();
    let section_title = x.l.section_id.as_ref().and_then(|sid| sections.iter().find(|s| &s.id == sid)).map(|s| s.title.clone()).unwrap_or_default();
    let locked = match &g { Gate::Locked(why) => Some(why.clone()), _ => None };
    let cta = if locked.is_some() { Some(cta_html(&cta_for(&state, &x.s, &x.c, &x.v, x.e.as_ref()).await, &x.c, &x.s, x.e.as_ref(), x.lessons.first().map(|l| l.slug.as_str()))) } else { None };
    let video = rules::video_embed(&x.l.video_url);
    let video_html = video.as_ref().map(|(u, frame)| if *frame { format!(r#"<div class="vl-video"><iframe src="{}" title="{}" allowfullscreen allow="autoplay; encrypted-media; picture-in-picture"></iframe></div>"#, esc(u), esc(&x.l.title)) } else { format!(r#"<div class="vl-video"><video src="{}" controls playsinline preload="metadata"></video></div>"#, esc(u)) }).unwrap_or_default();
    let atts = rules::attachments(&x.l.attachments);
    let atts_html = if atts.is_empty() || locked.is_some() { String::new() } else {
        format!(r#"<section class="vl-files"><h2>Downloads</h2><ul>{}</ul></section>"#, atts.iter().map(|(l, u)| format!(r#"<li><a href="{}" target="_blank" rel="noopener" download>{}</a></li>"#, esc(u), esc(l))).collect::<String>())
    };
    let interactive = if locked.is_some() { String::new() } else {
        match kind {
            LessonKind::Quiz => quiz_html(&state, &x, tracked, &q).await,
            LessonKind::Assignment => assignment_html(&state, &x, tracked).await,
            LessonKind::Lesson => String::new(),
        }
    };
    let complete_form = if tracked && kind == LessonKind::Lesson {
        format!(
            r#"<form method="post" action="{action}" class="vl-complete"><input type="hidden" name="action" value="{act}"><button class="vl-btn{cls}" type="submit">{label}</button></form>"#,
            action = link(&format!("{BASE}{}/learn/{}/complete/", x.c.slug, x.l.slug)),
            act = if is_done { "undo" } else { "done" },
            cls = if is_done { " vl-btn-done" } else { "" },
            label = if is_done { "✓ Completed" } else if next.is_some() { "Complete & continue →" } else { "Complete the course" },
        )
    } else {
        String::new()
    };
    let progress = x.e.as_ref().map(|e| e.progress).unwrap_or(0);
    let celebrate = q.contains_key("finished") && x.e.as_ref().is_some_and(|e| e.status == "completed");
    let cert_url = x.e.as_ref().and_then(|e| e.certificate_code.as_deref()).filter(|_| x.s.certificates).map(|code| link(&format!("{BASE}certificate/{code}/")));
    let flash = q.get("ok").or_else(|| q.get("error")).map(|m| (q.contains_key("error"), m.chars().take(200).collect::<String>()));
    let content = if locked.is_some() { String::new() } else { x.l.content_html.clone() };

    let templates = theme_templates(&state, &["lms/lesson"]).await;
    if !templates.is_empty() {
        let x2 = repo::extras(&state.db).await;
        let mut cj = repo::course_json(&x.s, &x.c, &x2);
        cj["curriculum"] = json!(cur);
        let mut data = serde_json::Map::new();
        data.insert("lms".into(), lms_json(&x.s, &x.v));
        data.insert("course".into(), cj);
        data.insert("lesson".into(), json!({
            "id": x.l.id, "title": x.l.title, "summary": x.l.summary, "kind": x.l.kind, "kind_label": kind.label(),
            "section": section_title, "number": pos + 1, "total": x.lessons.len(),
            "duration": rules::duration_label(x.l.duration_minutes),
            "html": content,
            "video": video.as_ref().map(|(u, f)| json!({ "url": u, "iframe": f })),
            "video_html": video_html,
            "attachments": atts.iter().map(|(l, u)| json!({ "label": l, "url": u })).collect::<Vec<_>>(),
            "done": is_done, "locked": locked, "preview": !tracked && locked.is_none(),
            "staff_preview": x.v.staff && x.e.is_none(),
            "prev_url": prev.map(|p| lesson_url(&x.c, p)), "prev_title": prev.map(|p| p.title.clone()),
            "next_url": next.map(|n| lesson_url(&x.c, n)), "next_title": next.map(|n| n.title.clone()),
        }));
        data.insert("enrolment".into(), json!({ "enrolled": x.e.is_some(), "progress": progress, "done": x.done.len(), "total": x.lessons.len(), "completed": x.e.as_ref().is_some_and(|e| e.status == "completed"), "certificate_url": cert_url, "celebrate": celebrate }));
        data.insert("forms".into(), json!({ "complete": complete_form, "interactive": interactive, "cta": cta, "assets": assets() }));
        data.insert("flash".into(), flash.map(|(err, m)| json!({ "error": err, "message": m })).unwrap_or(Value::Null));
        return render_virtual_page_as(&state, &headers, &public(&path), &format!("{} · {}", x.l.title, x.c.title), &x.l.summary, String::new(), None, false, &templates, data, &["lms"]).await;
    }

    let body = format!(
        r#"{assets}<div class="vl vl-player{celebrate_cls}">
<aside class="vl-side" id="vl-side"><a class="vl-side-back" href="{course_url}">← {course}</a>
<div class="vl-side-prog"><div class="vl-progress"><span style="width:{progress}%"></span></div><span>{progress}% complete · {d}/{t}</span></div>
<nav class="vl-side-nav" aria-label="Course content">{cur}</nav></aside>
<main class="vl-stage">{flash}{staff}
<button class="vl-side-toggle" type="button" aria-controls="vl-side" aria-expanded="false">☰ Course content</button>
<p class="vl-eyebrow">{section}{kind_label} {n} of {t}{dur}</p><h1>{title}</h1>
{celebrate}
{locked}
{video_html}<div class="vl-prose">{content}</div>{interactive}{atts_html}
<footer class="vl-lesson-foot">{prev}{complete_form}{next}</footer></main></div>"#,
        assets = assets(),
        celebrate_cls = if celebrate { " vl-celebrate" } else { "" },
        course_url = link(&format!("{BASE}{}/", x.c.slug)),
        course = esc(&x.c.title),
        d = x.done.len(),
        t = x.lessons.len(),
        cur = curriculum_html(&cur, true),
        flash = flash.as_ref().map(|(err, m)| format!(r#"<p class="vl-flash{}" role="status">{}</p>"#, if *err { " vl-flash-err" } else { "" }, esc(m))).unwrap_or_default(),
        staff = if x.v.staff && x.e.is_none() { r#"<p class="vl-flash">Staff preview — you can open every lesson; nothing is recorded.</p>"# } else { "" },
        section = if section_title.is_empty() { String::new() } else { format!("{} · ", esc(&section_title)) },
        kind_label = kind.label(),
        n = pos + 1,
        dur = if x.l.duration_minutes > 0 { format!(" · {}", rules::duration_label(x.l.duration_minutes)) } else { String::new() },
        title = esc(&x.l.title),
        celebrate = if celebrate { format!(r#"<section class="vl-done-banner"><h2>🎉 You finished the course!</h2><p>Brilliant work.{}</p></section>"#, cert_url.as_ref().map(|u| format!(r#" <a class="vl-btn" href="{}">Get your certificate</a>"#, esc(u))).unwrap_or_default()) } else { String::new() },
        locked = locked.as_ref().map(|why| format!(r#"<section class="vl-locked"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 8h-1V6A5 5 0 0 0 7 6v2H6a2 2 0 0 0-2 2v10c0 1.1.9 2 2 2h12a2 2 0 0 0 2-2V10a2 2 0 0 0-2-2zM9 6a3 3 0 0 1 6 0v2H9V6z"/></svg><h2>{}</h2>{}</section>"#, esc(why), if x.e.is_none() { cta.clone().unwrap_or_default() } else { String::new() })).unwrap_or_default(),
        video_html = if locked.is_some() { String::new() } else { video_html.clone() },
        prev = prev.map(|p| format!(r#"<a class="vl-nav-prev" href="{}"><small>Previous</small>{}</a>"#, lesson_url(&x.c, p), esc(&p.title))).unwrap_or_else(|| "<span></span>".into()),
        next = next.map(|n| format!(r#"<a class="vl-nav-next" href="{}"><small>Next</small>{}</a>"#, lesson_url(&x.c, n), esc(&n.title))).unwrap_or_else(|| "<span></span>".into()),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &format!("{} · {}", x.l.title, x.c.title), &x.l.summary, body, None, false, &[], serde_json::Map::new(), &["lms"]).await
}

async fn tracked_ctx(state: &AppState, headers: &HeaderMap, slug: &str, lesson: &str) -> Result<(LessonCtx, repo::Enrollment), Response> {
    if !same_origin(state, headers) {
        return Err((StatusCode::FORBIDDEN, "This did not come from the site.").into_response());
    }
    let Some(x) = lesson_ctx(state, headers, slug, lesson).await else {
        return Err(not_found(state, &format!("{BASE}{slug}/learn/{lesson}/"), headers.clone()).await);
    };
    let path = format!("{BASE}{}/learn/{}/", x.c.slug, x.l.slug);
    if !matches!(gate(&x.c, &x.l, &x.lessons, x.e.as_ref(), &x.done, false), Gate::Open { tracked: true }) {
        return Err(back_to(&path, "error", "This lesson is not open to you yet."));
    }
    let Some(e) = x.e.clone() else { return Err(back_to(&path, "error", "Enrol first.")) };
    Ok((x, e))
}

fn ids(x: &LessonCtx) -> Vec<String> {
    x.lessons.iter().map(|l| l.id.clone()).collect()
}

async fn finished_redirect(x: &LessonCtx, after: &repo::Enrollment, next: Option<&repo::Lesson>, fallback: &str) -> Response {
    if after.status == "completed" && x.e.as_ref().is_some_and(|e| e.status != "completed") {
        return Redirect::to(&format!("{}?finished=1", lesson_url(&x.c, &x.l))).into_response();
    }
    match next {
        Some(n) => Redirect::to(&lesson_url(&x.c, n)).into_response(),
        None => Redirect::to(fallback).into_response(),
    }
}

async fn complete(State(state): State<AppState>, headers: HeaderMap, Path((slug, lesson)): Path<(String, String)>, Form(form): Form<HashMap<String, String>>) -> Response {
    let (x, e) = match tracked_ctx(&state, &headers, &slug, &lesson).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let all = ids(&x);
    if form.get("action").map(String::as_str) == Some("undo") {
        let _ = repo::uncomplete_lesson(&state.db, &e, &x.l.id, &all).await;
        return Redirect::to(&lesson_url(&x.c, &x.l)).into_response();
    }
    if x.l.kind() != LessonKind::Lesson {
        return back_to(&format!("{BASE}{}/learn/{}/", x.c.slug, x.l.slug), "error", "Quizzes and assignments complete when you pass them.");
    }
    match repo::complete_lesson(&state.db, &e, &x.l.id, &all, x.c.certificate != 0 && x.s.certificates).await {
        Ok(after) => {
            let pos = x.lessons.iter().position(|l| l.id == x.l.id).unwrap_or(0);
            finished_redirect(&x, &after, x.lessons.get(pos + 1), &link(&format!("{BASE}{}/", x.c.slug))).await
        }
        Err(_) => back_to(&format!("{BASE}{}/learn/{}/", x.c.slug, x.l.slug), "error", "That could not be saved."),
    }
}

async fn quiz(State(state): State<AppState>, headers: HeaderMap, Path((slug, lesson)): Path<(String, String)>, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    let (x, e) = match tracked_ctx(&state, &headers, &slug, &lesson).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = format!("{BASE}{}/learn/{}/", x.c.slug, x.l.slug);
    if x.l.kind() != LessonKind::Quiz {
        return back_to(&path, "error", "This is not a quiz.");
    }
    let questions = repo::questions(&state.db, &x.l.id).await;
    if questions.is_empty() {
        return back_to(&path, "error", "This quiz has no questions yet.");
    }
    let tries = repo::attempts(&state.db, &e.id, &x.l.id).await;
    if x.l.attempts > 0 && tries.len() as i64 >= x.l.attempts && !tries.iter().any(|a| a.passed != 0) {
        return back_to(&path, "error", "You have used all your tries.");
    }
    let (points, max, marks) = rules::grade(&questions, |qid| pairs.iter().filter(|(k, _)| k == &format!("q_{qid}")).map(|(_, v)| v.clone()).take(20).collect());
    let score = if max > 0 { points * 100 / max } else { 0 };
    let started = pairs.iter().find(|(k, _)| k == "started").and_then(|(_, v)| v.parse::<i64>().ok()).unwrap_or(0);
    let late = x.l.time_limit > 0 && started > 0 && now_ms() - started > (x.l.time_limit * 60 + 60) * 1000;
    let passed = score >= x.l.pass_mark && !late;
    let started_at = (started > 0).then(|| chrono::DateTime::from_timestamp_millis(started).map(|d| vuuut_core::dates::to_db_string(&d))).flatten();
    let answers = serde_json::to_string(&marks).unwrap_or_default();
    let Ok(attempt) = repo::add_attempt(&state.db, &e.id, &x.l.id, score, points, max, passed, &answers, started_at.as_deref()).await else {
        return back_to(&path, "error", "Your answers could not be saved.");
    };
    if passed {
        if let Ok(after) = repo::complete_lesson(&state.db, &e, &x.l.id, &ids(&x), x.c.certificate != 0 && x.s.certificates).await {
            if after.status == "completed" && e.status != "completed" {
                return Redirect::to(&format!("{}?attempt={attempt}&finished=1", lesson_url(&x.c, &x.l))).into_response();
            }
        }
    }
    let note = if late { "&error=Time%20was%20up%20before%20you%20submitted." } else { "" };
    Redirect::to(&format!("{}?attempt={attempt}{note}", lesson_url(&x.c, &x.l))).into_response()
}

async fn submit(State(state): State<AppState>, headers: HeaderMap, Path((slug, lesson)): Path<(String, String)>, Form(form): Form<HashMap<String, String>>) -> Response {
    let (x, e) = match tracked_ctx(&state, &headers, &slug, &lesson).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = format!("{BASE}{}/learn/{}/", x.c.slug, x.l.slug);
    if x.l.kind() != LessonKind::Assignment {
        return back_to(&path, "error", "This is not an assignment.");
    }
    if repo::submission(&state.db, &e.id, &x.l.id).await.is_some_and(|s| s.status == "passed") {
        return back_to(&path, "error", "This assignment has already passed.");
    }
    let (body, linkv) = match rules::submission_from_form(&form) {
        Ok(v) => v,
        Err(m) => return back_to(&path, "error", &m),
    };
    match repo::submit(&state.db, &e.id, &x.l.id, &body, &linkv).await {
        Ok(()) => back_to(&path, "ok", "Handed in. You will see your grade here."),
        Err(_) => back_to(&path, "error", "Your work could not be saved. Please try again."),
    }
}

// ---------------------------------------------------------------------------
// My learning
// ---------------------------------------------------------------------------

async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}dashboard/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let v = viewer(&state, &headers).await;
    let x = repo::extras(&state.db).await;
    let mut items: Vec<Value> = Vec::new();
    if let Some(mid) = v.id() {
        let now = vuuut_core::dates::now_db();
        for e in repo::enrollments_for_member(&state.db, mid).await {
            let Some(c) = repo::find_course(&state.db, &e.course_id).await else { continue };
            let mut j = repo::course_json(&s, &c, &x);
            j["enrolment"] = json!({
                "progress": e.progress,
                "status": e.status,
                "completed": e.status == "completed",
                "active": e.grants_access(&now),
                "continue_url": link(&format!("{BASE}{}/learn/", c.slug)),
                "certificate_url": e.certificate_code.as_deref().filter(|_| s.certificates).map(|code| link(&format!("{BASE}certificate/{code}/"))),
                "enrolled_at": e.enrolled_at.get(..10),
                "last_active_at": e.last_active_at.as_deref().map(|d| d.get(..10).unwrap_or(d).to_string()),
            });
            items.push(j);
        }
    }
    let (rec, _) = repo::browse(&state.db, &repo::Browse { status: Some("published"), limit: 8, ..Default::default() }).await.unwrap_or_default();
    let recommended: Vec<Value> = rec.iter().filter(|c| !items.iter().any(|i| i["id"].as_str() == Some(c.id.as_str()))).take(3).map(|c| repo::course_json(&s, c, &x)).collect();
    let in_progress = items.iter().filter(|i| i["enrolment"]["completed"].as_bool() != Some(true)).count();
    let completed = items.len() - in_progress;

    let templates = theme_templates(&state, &["lms/dashboard"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("lms".into(), lms_json(&s, &v));
        data.insert("enrolments".into(), json!(items));
        data.insert("recommended".into(), json!(recommended));
        data.insert("stats".into(), json!({ "in_progress": in_progress, "completed": completed, "certificates": items.iter().filter(|i| !i["enrolment"]["certificate_url"].is_null()).count() }));
        return render_virtual_page_as(&state, &headers, &public(&path), "My learning", "", String::new(), None, false, &templates, data, &["lms"]).await;
    }
    let body = if v.member.is_none() {
        format!(r##"{}<div class="vl vl-dash"><section class="vl-locked"><h2>Sign in to see your courses</h2><p><a class="vl-btn" href="#/portal/signin" data-portal="signin">Sign in</a> <a class="vl-btn vl-btn-ghost" href="{}">Browse courses</a></p></section></div>"##, assets(), link(BASE))
    } else {
        let rows: String = items.iter().map(|i| {
            let e = &i["enrolment"];
            format!(
                r#"<article class="vl-enrolled"><a class="vl-enrolled-art" href="{url}">{img}</a><div><p class="vl-eyebrow">{cat}</p><h2><a href="{url}">{t}</a></h2>
<div class="vl-progress"><span style="width:{p}%"></span></div><p class="vl-note">{p}% complete{last}</p>
<p class="vl-actions"><a class="vl-btn" href="{cont}">{label}</a>{cert}</p></div></article>"#,
                url = esc(i["url"].as_str().unwrap_or("")),
                img = i["feature_image"].as_str().map(|u| format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(u))).unwrap_or_default(),
                cat = esc(i["category"]["name"].as_str().unwrap_or("")),
                t = esc(i["title"].as_str().unwrap_or("")),
                p = e["progress"],
                last = e["last_active_at"].as_str().map(|d| format!(" · last studied {d}")).unwrap_or_default(),
                cont = esc(e["continue_url"].as_str().unwrap_or("")),
                label = if e["completed"].as_bool() == Some(true) { "Review" } else if e["progress"].as_i64().unwrap_or(0) > 0 { "Continue" } else { "Start" },
                cert = e["certificate_url"].as_str().map(|u| format!(r#" <a class="vl-btn vl-btn-ghost" href="{}">Certificate</a>"#, esc(u))).unwrap_or_default(),
            )
        }).collect();
        format!(
            r#"{assets}<div class="vl vl-dash"><header class="vl-hero vl-hero-sm"><p class="vl-eyebrow">Welcome back</p><h1>{name}</h1><p class="vl-lede">{ip} in progress · {cp} completed</p></header>
<div class="vl-enrolled-list">{rows}</div>{empty}
{rec}</div>"#,
            assets = assets(),
            name = esc(&v.name()),
            ip = in_progress,
            cp = completed,
            empty = if items.is_empty() { format!(r#"<p class="vl-empty">You are not enrolled in any courses yet. <a href="{}">Find one to start →</a></p>"#, link(BASE)) } else { String::new() },
            rec = if recommended.is_empty() { String::new() } else { format!(r#"<h2 class="vl-h2">You might like</h2><div class="vl-grid">{}</div>"#, recommended.iter().map(card).collect::<String>()) },
        )
    };
    render_virtual_page_as(&state, &headers, &public(&path), "My learning", "", body, None, false, &[], serde_json::Map::new(), &["lms"]).await
}

// ---------------------------------------------------------------------------
// certificates
// ---------------------------------------------------------------------------

/// A certificate is its own document, made to be printed or saved as PDF;
/// the code in its address is the proof, so anyone can verify it.
async fn certificate(State(state): State<AppState>, headers: HeaderMap, Path(code): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}certificate/{code}/");
    let code = code.trim().to_uppercase();
    if !s.enabled || !s.certificates || !rules::is_certificate_code(&code) {
        return not_found(&state, &path, headers).await;
    }
    let Some(e) = repo::enrollment_by_certificate(&state.db, &code).await else { return not_found(&state, &path, headers).await };
    let (Some(c), Some(m)) = (repo::find_course(&state.db, &e.course_id).await, repo::member(&state.db, &e.member_id).await) else {
        return not_found(&state, &path, headers).await;
    };
    let site = state.settings.get_str("title").unwrap_or_default();
    let x = repo::extras(&state.db).await;
    let instructor = c.instructor_id.as_deref().and_then(|id| x.instructors.iter().find(|i| i.id == id)).map(|i| i.name.clone()).unwrap_or_default();
    let signer = if s.signer.is_empty() { site.clone() } else { s.signer.clone() };
    let when = e.completed_at.as_deref().and_then(vuuut_core::dates::from_db_string).map(|d| d.format("%-d %B %Y").to_string()).unwrap_or_default();
    let name = if m.name.trim().is_empty() { m.email.split('@').next().unwrap_or("Student").to_string() } else { m.name.clone() };
    let url = vuuut_core::url::absolute(&state.site_url(), &link(&path));
    let minutes = { let (_, mins) = x.lessons.get(&c.id).copied().unwrap_or((0, 0)); if c.duration_minutes > 0 { c.duration_minutes } else { mins } };
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Certificate · {course} · {name}</title><meta name="robots" content="noindex">
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght@0,9..144,400;0,9..144,600;1,9..144,400&family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
<style>
@page {{ size: A4 landscape; margin: 0; }}
*{{box-sizing:border-box}} body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#ece7df;font-family:Inter,system-ui,sans-serif;color:#1d1a16;padding:24px}}
.cert{{position:relative;width:min(1100px,100%);aspect-ratio:1.414;background:#fffdf8;padding:5.5% 7%;box-shadow:0 30px 80px -30px rgba(40,30,10,.45);display:flex;flex-direction:column;justify-content:space-between;overflow:hidden}}
.cert::before{{content:"";position:absolute;inset:18px;border:1.5px solid #c7a76a;pointer-events:none}}
.cert::after{{content:"";position:absolute;inset:26px;border:.5px solid #c7a76a;pointer-events:none}}
.blob{{position:absolute;width:520px;height:520px;border-radius:50%;background:radial-gradient(circle at 30% 30%,#f6d8a8,transparent 60%);right:-200px;top:-220px;opacity:.6}}
.top{{display:flex;justify-content:space-between;align-items:center;font-size:13px;letter-spacing:.18em;text-transform:uppercase;color:#8a6d3b;position:relative}}
.seal{{width:96px;height:96px;border-radius:50%;display:grid;place-items:center;background:conic-gradient(from 0deg,#c7a76a,#f1dcae,#b08a46,#f1dcae,#c7a76a);color:#5a4216;font-family:Fraunces,serif;font-size:13px;text-align:center;line-height:1.15;box-shadow:inset 0 0 0 6px #fffdf8,inset 0 0 0 7px #c7a76a}}
.mid{{text-align:center;position:relative}} .mid p{{margin:0}}
.k{{font-size:14px;letter-spacing:.22em;text-transform:uppercase;color:#8a6d3b}}
h1{{font-family:Fraunces,serif;font-weight:400;font-size:clamp(34px,6vw,68px);margin:.25em 0 .1em;line-height:1.05}}
.c{{font-family:Fraunces,serif;font-style:italic;font-size:clamp(20px,2.8vw,30px);margin:.3em 0}}
.sub{{color:#5f574a;font-size:15px}}
.foot{{display:grid;grid-template-columns:1fr auto 1fr;gap:24px;align-items:end;position:relative}}
.sig{{border-top:1px solid #1d1a16;padding-top:8px;font-size:13px;color:#5f574a}} .sig b{{display:block;color:#1d1a16;font-family:Fraunces,serif;font-size:19px;font-weight:600;margin-bottom:2px}}
.verify{{font-size:11.5px;color:#8a7b62;text-align:right}} .verify code{{font-size:13px;color:#1d1a16;letter-spacing:.06em}}
.bar{{position:fixed;top:16px;right:16px;display:flex;gap:8px}} .bar a,.bar button{{font:600 14px Inter,sans-serif;border:0;border-radius:999px;padding:10px 18px;background:#1d1a16;color:#fff;cursor:pointer;text-decoration:none}}
.bar a{{background:#fff;color:#1d1a16}}
@media print{{ body{{background:#fff;padding:0}} .cert{{box-shadow:none;width:100%}} .bar{{display:none}} }}
@media (max-width:640px){{ .cert{{aspect-ratio:auto;gap:28px;padding:40px 28px}} .foot{{grid-template-columns:1fr}} .verify{{text-align:left}} .seal{{display:none}} }}
</style></head><body>
<div class="bar"><a href="{course_url}">Course</a><button onclick="window.print()">Print or save as PDF</button></div>
<main class="cert"><span class="blob"></span>
<div class="top"><span>{site}</span><span>Certificate of completion</span></div>
<div class="mid"><p class="k">This certifies that</p><h1>{name}</h1><p class="sub">has successfully completed</p><p class="c">{course}</p><p class="sub">{lessons} lessons{dur} · completed {when}</p></div>
<div class="foot"><div class="sig"><b>{signer}</b>{signer_role}</div><div class="seal">Verified<br>{year}</div>
<div class="verify">{ins}Verify at<br><code>{code}</code><br>{url}</div></div></main></body></html>"#,
        course = esc(&c.title),
        name = esc(&name),
        course_url = link(&format!("{BASE}{}/", c.slug)),
        site = esc(&site),
        lessons = x.lessons.get(&c.id).map(|(n, _)| *n).unwrap_or(0),
        dur = if minutes > 0 { format!(" · {}", rules::duration_label(minutes)) } else { String::new() },
        when = esc(&when),
        signer = esc(&signer),
        signer_role = if s.signer.is_empty() { "" } else { "For the academy" },
        year = esc(when.rsplit(' ').next().unwrap_or("")),
        ins = if instructor.is_empty() { String::new() } else { format!("Instructor: {}<br>", esc(&instructor)) },
        code = esc(&code),
        url = esc(&url),
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], html).into_response()
}

// ---------------------------------------------------------------------------
// the store's product page, for a product that sells a course
// ---------------------------------------------------------------------------

/// What the store shows for a product that sells a course.
pub(crate) struct ProductCourse {
    /// The course panel: what is in it, what you learn, the syllabus.
    pub html: String,
    /// The same for a theme's `shop/product.hbs` (`product.course`).
    pub json: Value,
    /// Already a student with access: show "Continue learning", not a sale.
    pub continue_url: Option<String>,
}

/// The published course `product_id` sells, when the LMS is on.
pub(crate) async fn product_course(state: &AppState, headers: &HeaderMap, product_id: &str) -> Option<ProductCourse> {
    let s = settings(state);
    if !s.enabled {
        return None;
    }
    let c = repo::courses_for_products(&state.db, &[product_id.to_string()]).await.into_iter().find(|c| c.status == "published")?;
    let paths = s.paths.clone();
    PATHS.scope(paths, async {
        let x = repo::extras(&state.db).await;
        let mut cj = repo::course_json(&s, &c, &x);
        let sections = repo::sections(&state.db, &c.id).await;
        let lessons: Vec<repo::Lesson> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").collect();
        let v = viewer(state, headers).await;
        let now = vuuut_core::dates::now_db();
        let enrolled = enrollment_for(state, &c, &v).await.is_some_and(|e| e.grants_access(&now));
        let cur = curriculum_json(&c, &sections, &lessons, None, &[], None, false);
        let quizzes = lessons.iter().filter(|l| l.kind() != LessonKind::Lesson).count();
        let course_url = link(&format!("{BASE}{}/", c.slug));
        let continue_url = enrolled.then(|| link(&format!("{BASE}{}/learn/", c.slug)));
        let facts: Vec<String> = [
            Some(format!("{} lessons", lessons.len())),
            cj["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!("{d} of learning")),
            (quizzes > 0).then(|| format!("{quizzes} quizzes & assignments")),
            cj["level_label"].as_str().map(str::to_string),
            Some(if c.access_days > 0 { format!("{} days of access", c.access_days) } else { "Lifetime access".into() }),
            (cj["certificate"].as_bool() == Some(true)).then(|| "Certificate of completion".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();
        let outcomes: Vec<String> = cj["outcomes"].as_array().map(|a| a.iter().filter_map(|o| o.as_str()).take(6).map(str::to_string).collect()).unwrap_or_default();
        let instructor = cj["instructor"].clone();
        let html = format!(
            r#"<section class="vs-course"><div class="vs-course-head"><span class="vs-course-kicker">Online course</span>{rating}</div>
<ul class="vs-course-facts">{facts}</ul>
{learn}
<div class="vs-course-syl"><b>Course content</b><ol>{syl}</ol></div>
{ins}
<a class="vs-course-link" href="{course_url}">See the full course page — syllabus, previews and reviews →</a></section>"#,
            rating = cj["rating"].as_str().map(|r| format!(r#"<span class="vs-course-rating">★ {r} <span>({} reviews{})</span></span>"#, cj["reviews"], cj["students"].as_i64().filter(|n| *n > 0).map(|n| format!(" · {n} students")).unwrap_or_default())).unwrap_or_default(),
            facts = facts.iter().map(|f| format!("<li>{}</li>", esc(f))).collect::<String>(),
            learn = if outcomes.is_empty() { String::new() } else { format!(r#"<div class="vs-course-learn"><b>What you'll learn</b><ul>{}</ul></div>"#, outcomes.iter().map(|o| format!("<li>{}</li>", esc(o))).collect::<String>()) },
            syl = cur.iter().map(|sec| format!("<li><span>{}</span><em>{}{}</em></li>", esc(sec["title"].as_str().unwrap_or("")), if sec["count"].as_u64() == Some(1) { "1 lesson".to_string() } else { format!("{} lessons", sec["count"]) }, sec["duration"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {d}")).unwrap_or_default())).collect::<String>(),
            ins = if instructor.is_object() { format!(r#"<p class="vs-course-by">Taught by <b>{}</b></p>"#, esc(instructor["name"].as_str().unwrap_or(""))) } else { String::new() },
        );
        cj["course_url"] = json!(course_url);
        cj["facts"] = json!(facts);
        cj["curriculum"] = json!(cur);
        cj["enrolled"] = json!(enrolled);
        cj["continue_url"] = json!(continue_url);
        Some(ProductCourse { html, json: cj, continue_url })
    })
    .await
}
