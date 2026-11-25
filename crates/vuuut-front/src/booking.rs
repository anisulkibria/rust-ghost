//! The public booking pages: `/book/` (or wherever Settings moves them).
//!
//! Rendered inside the active theme like the store, the directory and the
//! LMS. A theme can take over any screen — `booking/services.hbs`,
//! `booking/service.hbs`, `booking/staff.hbs`, `booking/manage.hbs` — and is
//! handed ready-made HTML for the parts that post (`forms.*`), so it never
//! rebuilds the checks around them. While booking is off every address here
//! is the theme's 404.
//!
//! * `GET /book/` — every bookable service, by category, and the team.
//! * `GET /book/staff/{slug}/` — one person and what they offer.
//! * `GET /book/{service}/` — the service and the booking form: who (when
//!   the customer chooses), a month calendar of days with free times, the
//!   times on the chosen day, and the customer's details. The calendar is
//!   plain links (`?date=`, `?month=`, `?staff=`); `…/picker/` returns just
//!   the picker so the script can swap it in place.
//! * `POST /book/{service}/` — take the booking; the slot is checked again.
//! * `GET /book/manage/{token}/` — the customer's booking: add to a
//!   calendar (`event.ics`, Google), reschedule and cancel within the
//!   policy. `GET /book/paid/{token}/` — back from Stripe.

use crate::routes::render_virtual_page_as;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_api::AppState;
use vuuut_core::booking::{self as rules, Settings};
use vuuut_db::repos::booking as repo;

const BASE: &str = "/book/";
static CSS: &str = include_str!("../assets/booking.css");
static JS: &str = include_str!("../assets/booking.js");

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
        .route("/book/", get(index))
        .route("/book", get(|| async { Redirect::permanent(&link(BASE)) }))
        .route("/book/assets/{v}/booking.css", get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response() }))
        .route("/book/assets/{v}/booking.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], JS).into_response() }))
        .route("/book/staff/{slug}/", get(staff_page))
        .route("/book/manage/{token}/", get(manage))
        .route("/book/manage/{token}/cancel/", post(cancel))
        .route("/book/manage/{token}/reschedule/", get(reschedule_page).post(reschedule))
        .route("/book/manage/{token}/event.ics", get(ics))
        .route("/book/paid/{token}/", get(paid))
        .route("/book/{slug}/", get(service_page).post(book))
        .route("/book/{slug}/picker/", get(picker_fragment))
}

fn settings(state: &AppState) -> Settings {
    vuuut_api::booking::settings(state)
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

/// Moves the booking pages to the address chosen in Settings.
pub(crate) async fn addresses(State(state): State<AppState>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let paths = rules::Paths::read(|k| state.settings.get_str(k));
    if paths != rules::Paths::default() {
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        if let Some(internal) = paths.internal_path(&path) {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        } else if (path == "/book" || path.starts_with(BASE)) && req.method() == axum::http::Method::GET {
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
        link(&format!("/book/assets/{}/booking.css", css_version())),
        link(&format!("/book/assets/{}/booking.js", js_version())),
    )
}

async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

fn same_origin(state: &AppState, headers: &HeaderMap) -> bool {
    vuuut_api::forms::same_origin(headers, &state.site_url())
}

fn back_to(path: &str, kind: &str, msg: &str) -> Response {
    let j = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{}{j}{kind}={}", link(path), vuuut_core::social_media::encode(msg))).into_response()
}

fn day_hours(d: &vuuut_core::directory::Day) -> String {
    let l = d.label();
    if l.is_empty() { "Closed".into() } else { l }
}

fn booking_json(s: &Settings, tz: chrono_tz::Tz) -> Value {
    json!({
        "title": s.title,
        "intro": s.intro,
        "url": link(BASE),
        "address": s.address,
        "timezone": tz.name(),
        "style_url": link(&format!("/book/assets/{}/booking.css", css_version())),
        "script_url": link(&format!("/book/assets/{}/booking.js", js_version())),
        "hours": vuuut_core::directory::DAYS_OF_WEEK.iter().enumerate().map(|(i, (_, name, short))| json!({ "day": name, "short": short, "hours": day_hours(&s.hours.0[i]), "closed": matches!(s.hours.0[i], vuuut_core::directory::Day::Closed | vuuut_core::directory::Day::Unknown) })).collect::<Vec<_>>(),
        "cancel_hours": s.cancel_hours,
    })
}

struct Catalog {
    services: Vec<repo::Service>,
    staff: Vec<repo::Staff>,
    cats: Vec<repo::Category>,
    links: Vec<(String, String)>,
}

impl Catalog {
    async fn load(state: &AppState) -> Catalog {
        Catalog { services: repo::services(&state.db, true).await, staff: repo::staff(&state.db, true).await, cats: repo::categories(&state.db).await, links: repo::links(&state.db).await }
    }
    fn people_for(&self, service_id: &str) -> Vec<&repo::Staff> {
        let linked: Vec<&String> = self.links.iter().filter(|(s, _)| s == service_id).map(|(_, p)| p).collect();
        self.staff.iter().filter(|p| linked.is_empty() || linked.contains(&&p.id)).collect()
    }
    fn services_of(&self, staff_id: &str) -> Vec<&repo::Service> {
        self.services.iter().filter(|v| { let l: Vec<&String> = self.links.iter().filter(|(s, _)| s == &v.id).map(|(_, p)| p).collect(); l.is_empty() || l.iter().any(|p| *p == staff_id) }).collect()
    }
    fn service_json(&self, s: &Settings, v: &repo::Service) -> Value {
        repo::service_json(s, v, &self.cats, &self.people_for(&v.id))
    }
    fn staff_json(&self, s: &Settings, p: &repo::Staff) -> Value {
        repo::staff_json(s, p, &self.services_of(&p.id))
    }
}

fn service_card(v: &Value) -> String {
    let st = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    format!(
        r#"<a class="vk-card" href="{href}"{ext}>{img}<span class="vk-card-body"><span class="vk-card-meta">{cat}<span>{dur}</span>{group}</span>
<b>{name}</b><span class="vk-card-sub">{sub}</span><span class="vk-card-foot"><span class="vk-price">{price}</span><span class="vk-go">{go}</span></span></span></a>"#,
        href = esc(&st("book_url")),
        ext = if v["external"].as_bool() == Some(true) { r#" target="_blank" rel="noopener""# } else { "" },
        img = v.get("feature_image").and_then(Value::as_str).map(|u| format!(r#"<span class="vk-card-art"><img src="{}" alt="" loading="lazy"></span>"#, esc(u))).unwrap_or_default(),
        cat = v["category"]["name"].as_str().map(|n| format!("<span>{}</span>", esc(n))).unwrap_or_default(),
        dur = esc(&st("duration")),
        group = if v["group"].as_bool() == Some(true) { format!("<span>Group · {} places</span>", v["capacity"]) } else { String::new() },
        name = esc(&st("name")),
        sub = esc(&st("summary")),
        price = esc(&st("price_label")),
        go = if v["external"].as_bool() == Some(true) { "Book ↗" } else { "Book →" },
    )
}

// ---------------------------------------------------------------------------
// the list of services
// ---------------------------------------------------------------------------

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = settings(&state);
    if !s.enabled {
        return not_found(&state, BASE, headers).await;
    }
    let tz = vuuut_api::booking::tz(&state, &s);
    let c = Catalog::load(&state).await;
    let services: Vec<Value> = c.services.iter().map(|v| c.service_json(&s, v)).collect();
    let mut groups: Vec<Value> = c
        .cats
        .iter()
        .map(|k| {
            let list: Vec<Value> = c.services.iter().filter(|v| v.category_id.as_deref() == Some(k.id.as_str())).map(|v| c.service_json(&s, v)).collect();
            let mut j = repo::category_json(&s, k, list.len());
            j["services"] = json!(list);
            j
        })
        .filter(|g| g["count"].as_u64().unwrap_or(0) > 0)
        .collect();
    let loose: Vec<Value> = c.services.iter().filter(|v| v.category_id.as_ref().is_none_or(|id| !c.cats.iter().any(|k| &k.id == id))).map(|v| c.service_json(&s, v)).collect();
    if !loose.is_empty() {
        groups.push(json!({ "name": if groups.is_empty() { "Services" } else { "More" }, "slug": "more", "count": loose.len(), "services": loose, "tint": "slate", "description": "" }));
    }
    let team: Vec<Value> = c.staff.iter().map(|p| c.staff_json(&s, p)).collect();

    let templates = theme_templates(&state, &["booking/services"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("booking".into(), booking_json(&s, tz));
        data.insert("services".into(), json!(services));
        data.insert("groups".into(), json!(groups));
        data.insert("featured".into(), json!(services.iter().filter(|v| v["featured"].as_bool() == Some(true)).cloned().collect::<Vec<_>>()));
        data.insert("team".into(), json!(team));
        return render_virtual_page_as(&state, &headers, &public(BASE), &s.title, &s.intro, String::new(), None, false, &templates, data, &["booking"]).await;
    }
    let body = format!(
        r#"{assets}<div class="vk"><header class="vk-hero"><h1>{title}</h1>{intro}</header>
{groups}
{team}</div>"#,
        assets = assets(),
        title = esc(&s.title),
        intro = if s.intro.is_empty() { String::new() } else { format!(r#"<p class="vk-lede">{}</p>"#, esc(&s.intro)) },
        groups = groups.iter().map(|g| format!(
            r#"<section class="vk-group" id="{slug}"><h2>{name}</h2>{desc}<div class="vk-grid">{cards}</div></section>"#,
            slug = esc(g["slug"].as_str().unwrap_or("")),
            name = esc(g["name"].as_str().unwrap_or("")),
            desc = g["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(r#"<p class="vk-note">{}</p>"#, esc(d))).unwrap_or_default(),
            cards = g["services"].as_array().map(|a| a.iter().map(service_card).collect::<String>()).unwrap_or_default(),
        )).collect::<String>(),
        team = if team.is_empty() { String::new() } else { format!(r#"<section class="vk-group"><h2>Our team</h2><div class="vk-team">{}</div></section>"#, team.iter().map(|p| format!(
            r#"<a class="vk-person" href="{u}">{img}<b>{n}</b><span>{t}</span></a>"#,
            u = esc(p["url"].as_str().unwrap_or("")),
            img = p["photo"].as_str().map(|x| format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(x))).unwrap_or_else(|| format!(r#"<i>{}</i>"#, esc(p["initials"].as_str().unwrap_or("")))),
            n = esc(p["name"].as_str().unwrap_or("")),
            t = esc(p["title"].as_str().unwrap_or("")),
        )).collect::<String>()) },
    );
    render_virtual_page_as(&state, &headers, &public(BASE), &s.title, &s.intro, body, None, false, &[], serde_json::Map::new(), &["booking"]).await
}

async fn staff_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}staff/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(p) = repo::staff_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await.filter(|p| p.status == "active") else { return not_found(&state, &path, headers).await };
    let tz = vuuut_api::booking::tz(&state, &s);
    let c = Catalog::load(&state).await;
    let pj = c.staff_json(&s, &p);
    let offered: Vec<Value> = c.services_of(&p.id).iter().map(|v| c.service_json(&s, v)).collect();
    let templates = theme_templates(&state, &["booking/staff"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("booking".into(), booking_json(&s, tz));
        data.insert("person".into(), pj);
        data.insert("services".into(), json!(offered));
        return render_virtual_page_as(&state, &headers, &public(&path), &p.name, &p.title, String::new(), (!p.photo.is_empty()).then(|| p.photo.clone()), false, &templates, data, &["booking"]).await;
    }
    let body = format!(
        r#"{assets}<div class="vk"><nav class="vk-crumbs"><a href="{home}">{t}</a></nav><header class="vk-person-head">{img}<div><h1>{n}</h1><p class="vk-lede">{title}</p></div></header>
{bio}<h2>Book with {first}</h2><div class="vk-grid">{cards}</div></div>"#,
        assets = assets(),
        home = link(BASE),
        t = esc(&s.title),
        img = if p.photo.is_empty() { format!(r#"<i class="vk-face">{}</i>"#, esc(&p.initials())) } else { format!(r#"<img class="vk-face" src="{}" alt="">"#, esc(&p.photo)) },
        n = esc(&p.name),
        title = esc(&p.title),
        bio = vuuut_core::directory::paragraphs(&p.bio),
        first = esc(p.name.split_whitespace().next().unwrap_or("")),
        cards = offered.iter().map(|v| { let mut v = v.clone(); if let Some(u) = v["url"].as_str() { if v["external"].as_bool() != Some(true) { v["book_url"] = json!(format!("{u}?staff={}", p.id)); } } service_card(&v) }).collect::<String>(),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &p.name, &p.title, body, None, false, &[], serde_json::Map::new(), &["booking"]).await
}

// ---------------------------------------------------------------------------
// the picker
// ---------------------------------------------------------------------------

/// What the visitor has picked so far, from the query.
struct Pick {
    staff: Option<String>,
    date: Option<NaiveDate>,
    month: NaiveDate,
    at: Option<i64>,
}

fn read_pick(q: &HashMap<String, String>, today: NaiveDate, staff_ok: impl Fn(&str) -> bool) -> Pick {
    let staff = q.get("staff").map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && staff_ok(s));
    let date = q.get("date").and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).filter(|d| *d >= today);
    let month = q.get("month").and_then(|m| NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d").ok()).or_else(|| date.map(|d| d.with_day(1).unwrap_or(d))).unwrap_or_else(|| today.with_day(1).unwrap_or(today));
    let at = q.get("at").and_then(|a| a.parse::<i64>().ok());
    Pick { staff, date, month, at }
}

/// The picker: who, a month of days, and the times on the chosen day. Plain
/// links and radio buttons; `action` is the page the links point at.
async fn picker_html(state: &AppState, s: &Settings, svc: &repo::Service, people: &[&repo::Staff], pick: &Pick, action: &str) -> String {
    let tz = vuuut_api::booking::tz(state, s);
    let today = Utc::now().with_timezone(&tz).date_naive();
    let staff_id = pick.staff.as_deref();
    let q = |date: Option<NaiveDate>, month: NaiveDate, staff: Option<&str>| {
        let mut parts = vec![format!("month={}", month.format("%Y-%m"))];
        if let Some(d) = date {
            parts.push(format!("date={}", d.format("%Y-%m-%d")));
        }
        if let Some(st) = staff {
            parts.push(format!("staff={st}"));
        }
        format!("{action}?{}", parts.join("&amp;"))
    };
    let who = if svc.staff_mode == "choose" && people.len() > 1 {
        format!(
            r#"<div class="vk-step"><p class="vk-step-h"><span>1</span>Who would you like?</p><div class="vk-who">{any}{list}</div></div>"#,
            any = format!(r#"<a class="vk-who-chip{on}" href="{h}" data-pick><i>✦</i><span><b>Anyone</b><small>First available</small></span></a>"#, on = if staff_id.is_none() { " on" } else { "" }, h = q(pick.date, pick.month, None)),
            list = people.iter().map(|p| format!(
                r#"<a class="vk-who-chip{on}" href="{h}" data-pick>{face}<span><b>{n}</b><small>{t}</small></span></a>"#,
                on = if staff_id == Some(p.id.as_str()) { " on" } else { "" },
                h = q(pick.date, pick.month, Some(&p.id)),
                face = if p.photo.is_empty() { format!("<i>{}</i>", esc(&p.initials())) } else { format!(r#"<img src="{}" alt="">"#, esc(&p.photo)) },
                n = esc(&p.name),
                t = esc(&p.title),
            )).collect::<String>(),
        )
    } else {
        String::new()
    };
    let step = |n: usize| if who.is_empty() { n - 1 } else { n };
    let free_days = vuuut_api::booking::month_days(state, svc, pick.month.year(), pick.month.month(), staff_id).await;
    let date = pick.date.filter(|d| free_days.contains(d)).or_else(|| if pick.date.is_none() { free_days.first().copied() } else { None });
    // Weekday headers, then the month starting on Monday.
    let first_wd = pick.month.weekday().num_days_from_monday() as i64;
    let mut cells = String::new();
    for _ in 0..first_wd {
        cells.push_str("<span></span>");
    }
    let mut d = pick.month;
    while d.month() == pick.month.month() {
        let free = free_days.contains(&d);
        let cls = format!("{}{}{}", if free { " free" } else { "" }, if Some(d) == date { " on" } else { "" }, if d == today { " today" } else { "" });
        cells.push_str(&if free {
            format!(r#"<a class="vk-day{cls}" href="{h}" data-pick aria-label="{l}">{n}</a>"#, h = q(Some(d), pick.month, staff_id), l = d.format("%A %-d %B"), n = d.day())
        } else {
            format!(r#"<span class="vk-day{cls}" aria-hidden="true">{}</span>"#, d.day())
        });
        d = match d.succ_opt() { Some(n) => n, None => break };
    }
    let prev = pick.month.checked_sub_months(chrono::Months::new(1)).filter(|m| *m >= today.with_day(1).unwrap_or(today));
    let next = pick.month.checked_add_months(chrono::Months::new(1)).filter(|m| *m <= today + Duration::days(svc.max_advance));
    let slots = match date { Some(d) => vuuut_api::booking::day_slots(state, svc, d, staff_id).await, None => Vec::new() };
    let times = if let Some(d) = date {
        if slots.is_empty() {
            r#"<p class="vk-note">No free times that day.</p>"#.to_string()
        } else {
            let part = |lo: u32, hi: u32, label: &str| {
                let list: Vec<&rules::Slot> = slots.iter().filter(|x| { let h = chrono::Timelike::hour(&x.start.with_timezone(&tz)); h >= lo && h < hi }).collect();
                if list.is_empty() {
                    return String::new();
                }
                format!(
                    r#"<div class="vk-times-part"><small>{label}</small><div class="vk-times">{}</div></div>"#,
                    list.iter().map(|x| format!(
                        r#"<label class="vk-time"><input type="radio" name="at" value="{ts}"{c} required><span>{t}{seats}</span></label>"#,
                        ts = x.start.timestamp(),
                        c = if pick.at == Some(x.start.timestamp()) { " checked" } else { "" },
                        t = esc(&rules::time_label(x.start, tz)),
                        seats = if svc.capacity > 1 { format!("<small>{} left</small>", x.seats_left) } else { String::new() },
                    )).collect::<String>()
                )
            };
            format!("{}{}{}", part(0, 12, "Morning"), part(12, 17, "Afternoon"), part(17, 24, "Evening"))
        }
        .to_string()
            + format!(r#"<input type="hidden" name="date" value="{}">"#, d.format("%Y-%m-%d")).as_str()
    } else {
        r#"<p class="vk-note">No free days this month — try the next.</p>"#.to_string()
    };
    format!(
        r#"<div class="vk-picker" data-picker="{frag}">{who}
<div class="vk-step"><p class="vk-step-h"><span>{n1}</span>Choose a day</p>
<div class="vk-cal"><div class="vk-cal-head">{prev_l}<b>{month}</b>{next_l}</div>
<div class="vk-cal-grid"><i>Mo</i><i>Tu</i><i>We</i><i>Th</i><i>Fr</i><i>Sa</i><i>Su</i>{cells}</div></div></div>
<div class="vk-step"><p class="vk-step-h"><span>{n2}</span>{day_label}</p>{times}<p class="vk-tz">Times in {tzname}</p></div>
<input type="hidden" name="staff" value="{staff}"></div>"#,
        frag = esc(&format!("{}picker/", action)),
        n1 = step(2),
        n2 = step(3),
        prev_l = prev.map(|m| format!(r#"<a href="{}" data-pick aria-label="Previous month">‹</a>"#, q(None, m, staff_id))).unwrap_or_else(|| "<span></span>".into()),
        next_l = next.map(|m| format!(r#"<a href="{}" data-pick aria-label="Next month">›</a>"#, q(None, m, staff_id))).unwrap_or_else(|| "<span></span>".into()),
        month = pick.month.format("%B %Y"),
        day_label = date.map(|d| format!("Choose a time · {}", d.format("%a %-d %b"))).unwrap_or_else(|| "Choose a time".into()),
        tzname = esc(&tz.name().replace('_', " ")),
        staff = esc(staff_id.unwrap_or("")),
    )
}

fn details_html(s: &Settings, svc: &repo::Service, error: Option<&str>, form: &HashMap<String, String>, step_n: usize) -> String {
    let v = |k: &str| esc(form.get(k).map(String::as_str).unwrap_or(""));
    let qs = svc.questions();
    let questions: String = qs.iter().enumerate().map(|(i, q)| {
        let name = format!("q{i}");
        let req = if q.required { " required" } else { "" };
        let input = match q.kind.as_str() {
            "long" => format!(r#"<textarea name="{name}" rows="3" maxlength="2000"{req}>{}</textarea>"#, v(&name)),
            "yesno" => format!(r#"<select name="{name}"{req}><option value="">Choose…</option><option{y}>Yes</option><option{n}>No</option></select>"#, y = if form.get(&name).map(String::as_str) == Some("Yes") { " selected" } else { "" }, n = if form.get(&name).map(String::as_str) == Some("No") { " selected" } else { "" }),
            _ => format!(r#"<input name="{name}" maxlength="300" value="{}"{req}>"#, v(&name)),
        };
        format!(r#"<label class="vk-field vk-wide"><span>{}{}</span>{input}</label>"#, esc(&q.label), if q.required { " *" } else { "" })
    }).collect();
    let pay = if svc.price_cents > 0 {
        let p = rules::format_price(svc.price_cents, &s.currency);
        match svc.payment() {
            rules::Payment::Online => format!("{p} — you'll pay securely on the next step."),
            rules::Payment::Onsite => format!("{p} — pay at your appointment."),
            rules::Payment::None => p,
        }
    } else {
        "Free".into()
    };
    format!(
        r#"<div class="vk-step vk-details"><p class="vk-step-h"><span>{step_n}</span>Your details</p>{err}
<div class="vk-fields"><label class="vk-field"><span>Name *</span><input name="name" required maxlength="120" autocomplete="name" value="{name}"></label>
<label class="vk-field"><span>Email *</span><input name="email" type="email" required maxlength="191" autocomplete="email" value="{email}"></label>
<label class="vk-field"><span>Phone{req_phone}</span><input name="phone" type="tel" maxlength="40" autocomplete="tel" value="{phone}"{req_attr}></label>
{seats}{questions}
<label class="vk-field vk-wide"><span>Anything else?</span><textarea name="notes" rows="2" maxlength="2000">{notes}</textarea></label></div>
<div class="vk-hp" aria-hidden="true"><label>Leave this empty <input name="website_url" tabindex="-1" autocomplete="off"></label></div>
<input type="hidden" name="tz" value="" data-tz><input type="hidden" name="t" value="{t}">
{terms}<div class="vk-confirm"><p class="vk-pay">{pay}</p><button class="vk-btn" type="submit">{cta}</button></div></div>"#,
        err = error.map(|e| format!(r#"<p class="vk-error" role="alert">{}</p>"#, esc(e))).unwrap_or_default(),
        name = v("name"),
        email = v("email"),
        phone = v("phone"),
        req_phone = if s.require_phone { " *" } else { "" },
        req_attr = if s.require_phone { " required" } else { "" },
        seats = if svc.capacity > 1 { format!(r#"<label class="vk-field"><span>Places</span><input name="seats" type="number" min="1" max="{}" value="{}"></label>"#, svc.capacity.min(20), form.get("seats").cloned().unwrap_or_else(|| "1".into())) } else { String::new() },
        notes = v("notes"),
        t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
        terms = if s.terms.is_empty() { String::new() } else { format!(r#"<div class="vk-terms">{}</div>"#, vuuut_core::directory::paragraphs(&s.terms)) },
        cta = if svc.payment() == rules::Payment::Online && svc.price_cents > 0 { "Continue to payment" } else if svc.approval != 0 { "Request booking" } else { "Confirm booking" },
    )
}

async fn service_ctx(state: &AppState, slug: &str) -> Option<(Settings, repo::Service, Catalog)> {
    let s = settings(state);
    if !s.enabled {
        return None;
    }
    let svc = repo::service_by_slug(&state.db, &vuuut_core::directory::slugify(slug)).await.filter(|v| v.status != "draft")?;
    let c = Catalog::load(state).await;
    Some((s, svc, c))
}

async fn service_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}{slug}/");
    let Some((s, svc, c)) = service_ctx(&state, &slug).await else { return not_found(&state, &path, headers).await };
    render_service(&state, headers, &s, &svc, &c, &q, None, &HashMap::new()).await
}

#[allow(clippy::too_many_arguments)]
async fn render_service(state: &AppState, headers: HeaderMap, s: &Settings, svc: &repo::Service, c: &Catalog, q: &HashMap<String, String>, error: Option<&str>, form: &HashMap<String, String>) -> Response {
    let path = format!("{BASE}{}/", svc.slug);
    let tz = vuuut_api::booking::tz(state, s);
    let today = Utc::now().with_timezone(&tz).date_naive();
    let people = c.people_for(&svc.id);
    let mut pick = read_pick(q, today, |id| people.iter().any(|p| p.id == id));
    if pick.date.is_none() && !q.contains_key("month") {
        if let Some(d) = vuuut_api::booking::next_free_day(state, svc, pick.staff.as_deref(), svc.max_advance.min(90)).await {
            pick.date = Some(d);
            pick.month = d.with_day(1).unwrap_or(d);
        }
    }
    let action = link(&path);
    let sj = c.service_json(s, svc);
    let booking_form = if !svc.external_url.is_empty() {
        format!(r#"<div class="vk-external"><p>Bookings for this service are taken on our scheduling page.</p><a class="vk-btn" href="{}" target="_blank" rel="noopener">Choose a time ↗</a></div>"#, esc(&svc.external_url))
    } else {
        let picker = picker_html(state, s, svc, &people, &pick, &action).await;
        let steps = if svc.staff_mode == "choose" && people.len() > 1 { 4 } else { 3 };
        format!(
            r#"<form class="vk-form" method="post" action="{action}" data-booking-form>{err_top}{picker}{details}</form>"#,
            err_top = if error.is_some() && form.is_empty() { format!(r#"<p class="vk-error" role="alert">{}</p>"#, esc(error.unwrap_or(""))) } else { String::new() },
            details = details_html(s, svc, if form.is_empty() { None } else { error }, form, steps),
        )
    };
    let title = if svc.seo_title.is_empty() { svc.name.clone() } else { svc.seo_title.clone() };
    let excerpt = if svc.seo_description.is_empty() { svc.summary.clone() } else { svc.seo_description.clone() };
    let flash = q.get("error").map(|e| e.chars().take(200).collect::<String>());
    let templates = theme_templates(state, &["booking/service"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("booking".into(), booking_json(s, tz));
        data.insert("service".into(), sj);
        data.insert("forms".into(), json!({ "booking": booking_form }));
        data.insert("flash".into(), flash.map(|m| json!({ "error": true, "message": m })).unwrap_or(Value::Null));
        data.insert("related".into(), json!(c.services.iter().filter(|x| x.id != svc.id && x.category_id == svc.category_id).take(3).map(|x| c.service_json(s, x)).collect::<Vec<_>>()));
        return render_virtual_page_as(state, &headers, &public(&path), &title, &excerpt, String::new(), (!svc.image.is_empty()).then(|| svc.image.clone()), false, &templates, data, &["booking"]).await;
    }
    let body = format!(
        r#"{assets}<div class="vk vk-service"><nav class="vk-crumbs"><a href="{home}">{t}</a></nav>
{flash}<div class="vk-service-grid"><aside class="vk-summary">{img}<h1>{name}</h1><p class="vk-facts"><span>{dur}</span><span>{price}</span>{group}</p><p class="vk-where">{where_}</p>
<div class="vk-prose">{desc}</div></aside><div class="vk-booker">{booking_form}</div></div></div>"#,
        assets = assets(),
        home = link(BASE),
        t = esc(&s.title),
        flash = flash.as_ref().map(|m| format!(r#"<p class="vk-error" role="alert">{}</p>"#, esc(m))).unwrap_or_default(),
        img = if svc.image.is_empty() { String::new() } else { format!(r#"<img class="vk-cover" src="{}" alt="">"#, esc(&svc.image)) },
        name = esc(&svc.name),
        dur = esc(&rules::duration_label(svc.duration_minutes)),
        price = esc(&rules::format_price(svc.price_cents, &s.currency)),
        group = if svc.capacity > 1 { format!("<span>Group · {} places</span>", svc.capacity) } else { String::new() },
        where_ = esc(&svc.where_label(s)),
        desc = svc.description_markup(),
    );
    render_virtual_page_as(state, &headers, &public(&path), &title, &excerpt, body, (!svc.image.is_empty()).then(|| svc.image.clone()), false, &[], serde_json::Map::new(), &["booking"]).await
}

async fn picker_fragment(State(state): State<AppState>, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some((s, svc, c)) = service_ctx(&state, &slug).await else { return StatusCode::NOT_FOUND.into_response() };
    let tz = vuuut_api::booking::tz(&state, &s);
    let today = Utc::now().with_timezone(&tz).date_naive();
    let people = c.people_for(&svc.id);
    let pick = read_pick(&q, today, |id| people.iter().any(|p| p.id == id));
    let target = q.get("target").map(String::as_str).unwrap_or("");
    let action = if rules::is_token(target) { link(&format!("{BASE}manage/{target}/reschedule/")) } else { link(&format!("{BASE}{}/", svc.slug)) };
    let html = picker_html(&state, &s, &svc, &people, &pick, &action).await;
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], html).into_response()
}

const PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 8, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };

async fn book(State(state): State<AppState>, vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr, headers: HeaderMap, Path(slug): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}{slug}/");
    let Some((s, svc, c)) = service_ctx(&state, &slug).await else { return not_found(&state, &path, headers).await };
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    if !svc.external_url.is_empty() {
        return Redirect::to(&svc.external_url).into_response();
    }
    let mut q: HashMap<String, String> = HashMap::new();
    for k in ["date", "staff", "at"] {
        if let Some(v) = form.get(k) {
            q.insert(k.into(), v.clone());
        }
    }
    // A bot that fills the decoy, or a form sent in under two seconds, is
    // shown the page again with nothing booked.
    let started = form.get("t").and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    if form.get("website_url").is_some_and(|v| !v.is_empty()) || now_ms - started < 2000 {
        return render_service(&state, headers, &s, &svc, &c, &q, Some("Please try again."), &form).await;
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("booking:{ip}"), &PER_IP).await.is_err() {
        return render_service(&state, headers, &s, &svc, &c, &q, Some("Too many bookings from here. Please wait a little."), &form).await;
    }
    let Some(at) = form.get("at").and_then(|a| a.parse::<i64>().ok()).and_then(|a| DateTime::<Utc>::from_timestamp(a, 0)) else {
        return render_service(&state, headers, &s, &svc, &c, &q, Some("Please choose a time."), &form).await;
    };
    let input = match rules::BookingInput::from_form(&form, &svc.questions(), s.require_phone, svc.capacity) {
        Ok(i) => i,
        Err(m) => return render_service(&state, headers, &s, &svc, &c, &q, Some(&m), &form).await,
    };
    let customer_tz = form.get("tz").map(|t| t.trim()).filter(|t| t.parse::<chrono_tz::Tz>().is_ok()).unwrap_or("").to_string();
    let member = vuuut_api::members::current_member(&state, &headers).await.map(|m| m.id);
    let staff = form.get("staff").map(|s| s.trim()).filter(|s| !s.is_empty());
    match vuuut_api::booking::book(&state, &svc, at, staff, &input, &customer_tz, member.as_deref(), "site").await {
        Ok(vuuut_api::booking::Booked::Done(b)) => Redirect::to(&format!("{}?new=1", link(&format!("{BASE}manage/{}/", b.token)))).into_response(),
        Ok(vuuut_api::booking::Booked::Pay(_, url)) => Redirect::to(&url).into_response(),
        Err(m) => {
            q.remove("at");
            render_service(&state, headers, &s, &svc, &c, &q, Some(&m), &form).await
        }
    }
}

// ---------------------------------------------------------------------------
// the customer's booking
// ---------------------------------------------------------------------------

async fn by_token(state: &AppState, token: &str) -> Option<(Settings, repo::Booking, repo::Service)> {
    let s = settings(state);
    if !s.enabled || !rules::is_token(token) {
        return None;
    }
    let b = repo::booking_by_token(&state.db, token).await?;
    let svc = repo::find_service(&state.db, &b.service_id).await.unwrap_or_default();
    Some((s, b, svc))
}

fn can_change(s: &Settings, b: &repo::Booking) -> bool {
    matches!(b.status.as_str(), "confirmed" | "pending") && b.start() - Duration::hours(s.cancel_hours) > Utc::now()
}

async fn manage(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}manage/{token}/");
    let Some((s, b, svc)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    let tz = vuuut_api::booking::tz(&state, &s);
    let staff = match &b.staff_id { Some(id) => repo::find_staff(&state.db, id).await, None => None };
    let site = state.settings.get_str("title").unwrap_or_default();
    let summary = format!("{} — {}", svc.name, site);
    let place = if svc.location_type == "video" && !svc.location.is_empty() && b.status == "confirmed" { svc.location.clone() } else { svc.where_label(&s) };
    let manage_abs = vuuut_api::booking::manage_url(&state, &s, &b);
    let gcal = rules::google_calendar_url(b.start(), b.end(), &summary, &format!("Reference {}\n{}", b.r#ref, manage_abs), &place);
    let changeable = can_change(&s, &b);
    let (head, lede) = match b.status.as_str() {
        "confirmed" if q.contains_key("new") || q.contains_key("paid") => ("You're booked!", "A confirmation is on its way to your inbox."),
        "confirmed" => ("Your booking", "Everything you need, in one place."),
        "pending" => ("Request received", "We'll confirm shortly and email you."),
        "awaiting_payment" => ("Payment not finished", "Your time is held for a few minutes. Complete the payment to confirm it."),
        "cancelled" => ("Booking cancelled", "This booking is cancelled."),
        "completed" => ("Thanks for visiting", "We hope to see you again soon."),
        _ => ("Your booking", ""),
    };
    let local = b.start().with_timezone(&tz);
    let bj = json!({
        "ref": b.r#ref, "status": b.status, "status_label": rules::Status::from_key(&b.status).label(),
        "heading": head, "lede": lede, "new": q.contains_key("new") || q.contains_key("paid"),
        "service": svc.name, "service_url": link(&format!("{BASE}{}/", svc.slug)), "image": if svc.image.is_empty() { Value::Null } else { json!(svc.image) },
        "day": rules::day_label(b.start(), tz), "time": rules::time_label(b.start(), tz), "end_time": rules::time_label(b.end(), tz),
        "dom": local.format("%-d").to_string(), "month": local.format("%b").to_string(), "weekday": local.format("%A").to_string(),
        "timezone": tz.name(), "duration": rules::duration_label((b.end() - b.start()).num_minutes()),
        "where": place, "staff": staff.as_ref().map(|p| json!({ "name": p.name, "title": p.title, "photo": if p.photo.is_empty() { Value::Null } else { json!(p.photo) }, "initials": p.initials() })),
        "name": b.name, "email": b.email, "seats": b.seats,
        "price": if b.price_cents > 0 { json!(rules::format_price(b.price_cents, &s.currency)) } else { Value::Null },
        "paid": b.payment_status == "paid",
        "ics_url": link(&format!("{BASE}manage/{}/event.ics", b.token)), "google_url": gcal,
        "can_change": changeable, "reschedule_url": link(&format!("{BASE}manage/{}/reschedule/", b.token)),
        "cancel_hours": s.cancel_hours, "book_url": link(BASE),
    });
    let cancel_form = if changeable {
        format!(
            r#"<form method="post" action="{}" class="vk-cancel" data-confirm-cancel><button class="vk-btn vk-btn-ghost" type="submit">Cancel booking</button></form>"#,
            link(&format!("{BASE}manage/{}/cancel/", b.token))
        )
    } else {
        String::new()
    };
    let flash = q.get("ok").or_else(|| q.get("error")).map(|m| (q.contains_key("error"), m.chars().take(200).collect::<String>()));
    let templates = theme_templates(&state, &["booking/manage"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("booking".into(), booking_json(&s, tz));
        data.insert("appointment".into(), bj);
        data.insert("forms".into(), json!({ "cancel": cancel_form }));
        data.insert("flash".into(), flash.map(|(e, m)| json!({ "error": e, "message": m })).unwrap_or(Value::Null));
        return render_virtual_page_as(&state, &headers, &public(&path), head, "", String::new(), None, false, &templates, data, &["booking"]).await;
    }
    let body = format!(
        r#"{assets}<div class="vk vk-manage{celebrate}">{flash}<header class="vk-done"><span class="vk-done-ic">{ic}</span><h1>{head}</h1><p class="vk-lede">{lede}</p></header>
<section class="vk-ticket"><div class="vk-ticket-date"><b>{dom}</b><span>{mon}</span></div><div class="vk-ticket-body"><h2>{svc}</h2>
<p><b>{day}</b> · {t1} – {t2} <small>({tzn})</small></p><p>{place}</p>{with}{price}<p class="vk-ref">Reference <b>{r}</b></p></div></section>
<div class="vk-actions"><a class="vk-btn" href="{ics}">Add to calendar</a><a class="vk-btn vk-btn-ghost" href="{gcal}" target="_blank" rel="noopener">Google Calendar</a>{resched}{cancel_form}</div>
<p class="vk-note">{policy}</p></div>"#,
        assets = assets(),
        celebrate = if q.contains_key("new") || q.contains_key("paid") { " vk-new" } else { "" },
        flash = flash.as_ref().map(|(e, m)| format!(r#"<p class="{}" role="status">{}</p>"#, if *e { "vk-error" } else { "vk-ok" }, esc(m))).unwrap_or_default(),
        ic = if b.status == "cancelled" { "✕" } else { "✓" },
        dom = local.format("%-d"),
        mon = local.format("%b"),
        svc = esc(&svc.name),
        day = esc(&rules::day_label(b.start(), tz)),
        t1 = esc(&rules::time_label(b.start(), tz)),
        t2 = esc(&rules::time_label(b.end(), tz)),
        tzn = esc(tz.name()),
        place = esc(&place),
        with = staff.as_ref().map(|p| format!("<p>With {}</p>", esc(&p.name))).unwrap_or_default(),
        price = if b.price_cents > 0 { format!("<p>{}{}</p>", esc(&rules::format_price(b.price_cents, &s.currency)), if b.payment_status == "paid" { " · paid" } else { "" }) } else { String::new() },
        r = esc(&b.r#ref),
        ics = link(&format!("{BASE}manage/{}/event.ics", b.token)),
        gcal = esc(&gcal),
        resched = if changeable { format!(r#"<a class="vk-btn vk-btn-ghost" href="{}">Reschedule</a>"#, link(&format!("{BASE}manage/{}/reschedule/", b.token))) } else { String::new() },
        policy = if changeable { if s.cancel_hours > 0 { format!("You can reschedule or cancel online until {} hours before.", s.cancel_hours) } else { "You can reschedule or cancel online any time before.".into() } } else if matches!(b.status.as_str(), "confirmed" | "pending") { "It's too close to the time to change online — please get in touch.".into() } else { String::new() },
    );
    render_virtual_page_as(&state, &headers, &public(&path), head, "", body, None, false, &[], serde_json::Map::new(), &["booking"]).await
}

async fn cancel(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>) -> Response {
    let path = format!("{BASE}manage/{token}/");
    let Some((s, b, _)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    if !can_change(&s, &b) {
        return back_to(&path, "error", "This booking can no longer be cancelled online.");
    }
    if repo::set_status(&state.db, &b.id, "cancelled", "Cancelled by the customer.").await.is_err() {
        return back_to(&path, "error", "That did not work. Please try again.");
    }
    if let Some(after) = repo::find_booking(&state.db, &b.id).await {
        vuuut_api::booking::notify_cancelled(&state, &after, true).await;
    }
    back_to(&path, "ok", "Your booking is cancelled. We've emailed you a note.")
}

async fn reschedule_page(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}manage/{token}/reschedule/");
    let Some((s, b, svc)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    if !can_change(&s, &b) {
        return back_to(&format!("{BASE}manage/{token}/"), "error", "This booking can no longer be moved online.");
    }
    let tz = vuuut_api::booking::tz(&state, &s);
    let today = Utc::now().with_timezone(&tz).date_naive();
    let c = Catalog::load(&state).await;
    let people = c.people_for(&svc.id);
    let mut q2 = q.clone();
    if !q2.contains_key("staff") {
        if let Some(st) = &b.staff_id {
            q2.insert("staff".into(), st.clone());
        }
    }
    let mut pick = read_pick(&q2, today, |id| people.iter().any(|p| p.id == id));
    if pick.date.is_none() && !q.contains_key("month") {
        if let Some(d) = vuuut_api::booking::next_free_day(&state, &svc, pick.staff.as_deref(), svc.max_advance.min(90)).await {
            pick.date = Some(d);
            pick.month = d.with_day(1).unwrap_or(d);
        }
    }
    let action = link(&path);
    let picker = picker_html(&state, &s, &svc, &people, &pick, &action).await.replace(&format!("{action}picker/"), &format!("{}?target={token}", link(&format!("{BASE}{}/picker/", svc.slug))));
    let err = q.get("error").map(|e| format!(r#"<p class="vk-error" role="alert">{}</p>"#, esc(&e.chars().take(200).collect::<String>()))).unwrap_or_default();
    let body = format!(
        r#"{assets}<div class="vk vk-service"><nav class="vk-crumbs"><a href="{back}">← Your booking</a></nav>{err}
<div class="vk-service-grid"><aside class="vk-summary"><h1>Choose a new time</h1><p class="vk-facts"><span>{svc}</span><span>{dur}</span></p><p class="vk-where">Now: {now}</p></aside>
<div class="vk-booker"><form class="vk-form" method="post" action="{action}" data-booking-form>{picker}<div class="vk-confirm"><span></span><button class="vk-btn" type="submit">Move my booking</button></div></form></div></div></div>"#,
        assets = assets(),
        back = link(&format!("{BASE}manage/{token}/")),
        svc = esc(&svc.name),
        dur = esc(&rules::duration_label(svc.duration_minutes)),
        now = esc(&format!("{}, {}", rules::day_label(b.start(), tz), rules::time_label(b.start(), tz))),
    );
    let templates = theme_templates(&state, &["booking/reschedule"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("booking".into(), booking_json(&s, tz));
        data.insert("service".into(), c.service_json(&s, &svc));
        data.insert("forms".into(), json!({ "picker": format!(r#"<form class="vk-form" method="post" action="{action}" data-booking-form>{picker}<div class="vk-confirm"><span></span><button class="vk-btn" type="submit">Move my booking</button></div></form>"#) }));
        return render_virtual_page_as(&state, &headers, &public(&path), "Reschedule", "", String::new(), None, false, &templates, data, &["booking"]).await;
    }
    render_virtual_page_as(&state, &headers, &public(&path), "Reschedule", "", body, None, false, &[], serde_json::Map::new(), &["booking"]).await
}

async fn reschedule(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}manage/{token}/reschedule/");
    let Some((s, b, svc)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    if !same_origin(&state, &headers) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    if !can_change(&s, &b) {
        return back_to(&format!("{BASE}manage/{token}/"), "error", "This booking can no longer be moved online.");
    }
    let Some(at) = form.get("at").and_then(|a| a.parse::<i64>().ok()).and_then(|a| DateTime::<Utc>::from_timestamp(a, 0)) else {
        return back_to(&path, "error", "Please choose a new time.");
    };
    let tz = vuuut_api::booking::tz(&state, &s);
    let staff_pref = form.get("staff").map(|s| s.trim()).filter(|s| !s.is_empty());
    // The booking's own time does not count against it.
    let _ = repo::set_status(&state.db, &b.id, "cancelled", "").await;
    let slots = vuuut_api::booking::day_slots(&state, &svc, at.with_timezone(&tz).date_naive(), staff_pref).await;
    let _ = repo::set_status(&state.db, &b.id, &b.status, "").await;
    let Some(slot) = slots.iter().find(|x| x.start == at) else { return back_to(&path, "error", "Sorry — that time has just been taken. Please choose another.") };
    let staff = staff_pref.map(str::to_string).or_else(|| slot.staff.first().cloned()).filter(|x| !x.is_empty());
    if repo::reschedule(&state.db, &b.id, at, at + Duration::minutes(svc.duration_minutes), staff.as_deref()).await.is_err() {
        return back_to(&path, "error", "That did not work. Please try again.");
    }
    if let Some(after) = repo::find_booking(&state.db, &b.id).await {
        vuuut_api::booking::notify_moved(&state, &after).await;
    }
    back_to(&format!("{BASE}manage/{token}/"), "ok", "Moved. We've emailed you the new time.")
}

async fn ics(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>) -> Response {
    let path = format!("{BASE}manage/{token}/event.ics");
    let Some((s, b, svc)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    let site = state.settings.get_str("title").unwrap_or_default();
    let place = if svc.location_type == "video" && !svc.location.is_empty() { svc.location.clone() } else { svc.where_label(&s) };
    let manage = vuuut_api::booking::manage_url(&state, &s, &b);
    let host = state.site_url().split("://").nth(1).unwrap_or("vuuut").split('/').next().unwrap_or("vuuut").to_string();
    let body = rules::ics(&format!("{}@{host}", b.r#ref), b.start(), b.end(), &format!("{} — {site}", svc.name), &format!("Reference {}. Manage: {manage}", b.r#ref), &place, &manage);
    ([(header::CONTENT_TYPE, "text/calendar; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.ics\"", b.r#ref))], body).into_response()
}

async fn paid(State(state): State<AppState>, headers: HeaderMap, Path(token): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let path = format!("{BASE}paid/{token}/");
    let Some((_, b, _)) = by_token(&state, &token).await else { return not_found(&state, &path, headers).await };
    let session = q.get("session").map(String::as_str).unwrap_or("");
    if vuuut_api::booking::settle_payment(&state, &b, session).await {
        return Redirect::to(&format!("{}?paid=1", link(&format!("{BASE}manage/{token}/")))).into_response();
    }
    back_to(&format!("{BASE}manage/{token}/"), "error", "We could not confirm the payment yet. If you were charged, it will show here shortly.")
}

