//! Tools → Booking & Appointment Scheduling: the admin screens.
//!
//! The rules are [`vuuut_core::booking`], the queries
//! [`vuuut_db::repos::booking`], the shared flow (availability, taking a
//! booking, email, payment) [`vuuut_api::booking`], and the public pages at
//! `/book/` are `vuuut_front::booking`. Here:
//!
//! * **Overview** (`/ghost/booking/`) — today's agenda, what waits for
//!   approval, the week ahead, bookings made over thirty days.
//! * **Calendar** — a week, one column a day, every booking in place.
//! * **Bookings** — the list, one booking (confirm, decline, cancel,
//!   complete, no-show, move, mark paid, note), and a booking made by staff.
//! * **Services**, **Staff** (people or resources, with weekly hours),
//!   **Time off**, **Customers** — `booking/*.rs`.
//! * **Settings** and categories — Administrator only.
//!
//! Editor and above for the rest. Every write is origin-checked; customer
//! details are shown to staff only; the CSV export defuses formulas.

use super::*;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use vuuut_core::booking::{self as rules, Settings};
use vuuut_db::repos::booking as repo;

mod sample;
mod services;
mod settings;
mod staff;
pub(crate) use sample::sample_load;
pub use sample::{load_business, seed_sample_business, Loaded};
pub(crate) use services::*;
pub(crate) use settings::*;
pub(crate) use staff::*;

const BASE: &str = "/ghost/booking/";
const BOOKINGS: &str = "/ghost/booking/bookings/";

async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

pub(crate) fn booking_settings(state: &AppState) -> Settings {
    vuuut_api::booking::settings(state)
}

fn zone(state: &AppState) -> chrono_tz::Tz {
    vuuut_api::booking::tz(state, &booking_settings(state))
}

fn public_href(s: &Settings, path: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(path))
}

fn tabs(active: &str, admin: bool, pending: i64) -> String {
    let tab = |key: &str, label: &str, href: &str, badge: i64| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{b}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            b = if badge > 0 { format!(r#" <span class="lm-badge">{badge}</span>"#) } else { String::new() },
        )
    };
    let mut out = String::from(r#"<div class="seg-bar rise" style="--i:1">"#);
    out.push_str(&tab("overview", "Overview", BASE, 0));
    out.push_str(&tab("calendar", "Calendar", "/ghost/booking/calendar/", 0));
    out.push_str(&tab("bookings", "Bookings", BOOKINGS, pending));
    out.push_str(&tab("services", "Services", "/ghost/booking/services/", 0));
    out.push_str(&tab("staff", "Staff", "/ghost/booking/staff/", 0));
    out.push_str(&tab("time-off", "Time off", "/ghost/booking/time-off/", 0));
    out.push_str(&tab("customers", "Customers", "/ghost/booking/customers/", 0));
    if admin {
        out.push_str(&tab("settings", "Settings", "/ghost/booking/settings/", 0));
    }
    out.push_str("</div>");
    out
}

async fn tabs_for(state: &AppState, active: &str, actor: &Actor) -> String {
    let pending = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM booking_bookings WHERE status = 'pending'").fetch_one(&state.db).await.unwrap_or(0);
    tabs(active, actor.role.is_admin(), pending)
}

fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{ic}</span><span class="dr-tile-value">{value}</span><span class="dr-tile-label">{label}</span><span class="dr-tile-detail">{detail}</span>"#,
        ic = svg(icon), value = esc(value), label = esc(label), detail = esc(detail),
    );
    match href {
        Some(h) => format!(r#"<a class="card padded dr-tile" href="{h}">{body}</a>"#),
        None => format!(r#"<div class="card padded dr-tile">{body}</div>"#),
    }
}

pub(crate) fn status_chip(status: &str) -> (&'static str, &'static str) {
    match rules::Status::from_key(status) {
        rules::Status::Confirmed => ("published", "Confirmed"),
        rules::Status::Pending => ("scheduled", "Needs approval"),
        rules::Status::AwaitingPayment => ("tonal", "Awaiting payment"),
        rules::Status::Completed => ("published", "Completed"),
        rules::Status::Cancelled => ("error", "Cancelled"),
        rules::Status::NoShow => ("draft", "No-show"),
    }
}

/// The bits a list of bookings needs to name things.
struct Names {
    services: Vec<repo::Service>,
    staff: Vec<repo::Staff>,
}

impl Names {
    async fn load(state: &AppState) -> Names {
        Names { services: repo::services(&state.db, false).await, staff: repo::staff(&state.db, false).await }
    }
    fn service(&self, id: &str) -> Option<&repo::Service> {
        self.services.iter().find(|s| s.id == id)
    }
    fn staff(&self, id: Option<&str>) -> Option<&repo::Staff> {
        id.and_then(|id| self.staff.iter().find(|p| p.id == id))
    }
}

fn booking_row(b: &repo::Booking, n: &Names, tz: chrono_tz::Tz, s: &Settings, actions: bool) -> String {
    let (chip, label) = status_chip(&b.status);
    let svc = n.service(&b.service_id);
    let who = n.staff(b.staff_id.as_deref());
    let local = b.start().with_timezone(&tz);
    let quick = if actions && b.status == "pending" {
        format!(
            r#"<form method="post" action="{BOOKINGS}{id}/action/"><input type="hidden" name="action" value="confirm"><button class="btn tonal small" type="submit">{ok} Confirm</button></form>
<form method="post" action="{BOOKINGS}{id}/action/" data-confirm="Decline this request?|The customer is emailed that it could not go ahead."><input type="hidden" name="action" value="decline"><button class="btn text small" type="submit">Decline</button></form>"#,
            id = esc(&b.id),
            ok = svg("check"),
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="row dr-listing bk-row"><span class="bk-when tint-{tint}"><b>{time}</b><small>{day}</small></span>
<div class="body"><span class="title"><a href="{BOOKINGS}{id}/">{name}</a> <span class="chip {chip}">{label}</span>{paid}</span>
<span class="detail">{svc}{seats}{who} · {dur} · {ref}</span></div>
<div class="trailing dr-actions">{quick}<a class="icon-btn" href="{BOOKINGS}{id}/" title="Open">{open}</a></div></div>"#,
        tint = svc.map(|x| x.tint.as_str()).unwrap_or("slate"),
        time = esc(&rules::time_label(b.start(), tz)),
        day = esc(&local.format("%a %-d %b").to_string()),
        id = esc(&b.id),
        name = esc(&b.name),
        paid = match b.payment_status.as_str() { "paid" => r#" <span class="chip published">Paid</span>"#.to_string(), "unpaid" if b.price_cents > 0 => format!(r#" <span class="chip tonal">{} due</span>"#, esc(&rules::format_price(b.price_cents, &s.currency))), _ => String::new() },
        svc = esc(svc.map(|x| x.name.as_str()).unwrap_or("Deleted service")),
        seats = if b.seats > 1 { format!(" × {}", b.seats) } else { String::new() },
        who = who.map(|p| format!(" · with {}", esc(&p.name))).unwrap_or_default(),
        dur = esc(&rules::duration_label((b.end() - b.start()).num_minutes())),
        ref = esc(&b.r#ref),
        open = svg("edit"),
    )
}

fn today_start(tz: chrono_tz::Tz) -> DateTime<Utc> {
    let d = Utc::now().with_timezone(&tz).date_naive();
    rules::local(tz, d, 0).unwrap_or_else(Utc::now)
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn booking_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let tz = zone(&state);
    let day0 = today_start(tz);
    let st = repo::stats(&state.db, day0).await;
    let n = Names::load(&state).await;
    let (today, _) = repo::browse(&state.db, &repo::Query { from: Some(&repo::db_time(day0)), to: Some(&repo::db_time(day0 + Duration::days(1))), limit: 200, ..Default::default() }).await;
    let today: Vec<&repo::Booking> = today.iter().filter(|b| !matches!(b.status.as_str(), "cancelled")).collect();
    let (pending, _) = repo::browse(&state.db, &repo::Query { status: Some("pending"), from: Some(&repo::db_time(Utc::now())), limit: 30, ..Default::default() }).await;
    let (upcoming, _) = repo::browse(&state.db, &repo::Query { status: Some("confirmed"), from: Some(&repo::db_time(day0 + Duration::days(1))), limit: 8, ..Default::default() }).await;

    let live = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>Online booking is open</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{path}</a> · times shown in {tz}.</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/book/"),
            site = esc(state.site_url().trim_end_matches('/')),
            path = esc(&s.paths.public_path("/book/")),
            tz = esc(tz.name()),
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>Online booking is not open yet.</b>
<p class="muted">Add your services and hours, then switch booking on in Settings.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/booking/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };

    // Today as a timeline.
    let agenda = if today.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">Nothing booked today.</p></div>"#.to_string()
    } else {
        let now = Utc::now();
        format!(
            r#"<div class="card padded bk-agenda">{}</div>"#,
            today.iter().map(|b| {
                let svc = n.service(&b.service_id);
                let state_cls = if b.end() < now { "past" } else if b.start() <= now { "now" } else { "" };
                format!(
                    r#"<a class="bk-slot {state_cls} tint-{tint}" href="{BOOKINGS}{id}/"><span class="bk-slot-time">{t1}<small>{t2}</small></span><span class="bk-slot-bar"></span>
<span class="bk-slot-body"><b>{name}</b><span>{svc}{who}</span></span><span class="chip {chip}">{label}</span></a>"#,
                    tint = svc.map(|x| x.tint.as_str()).unwrap_or("slate"),
                    id = esc(&b.id),
                    t1 = esc(&rules::time_label(b.start(), tz)),
                    t2 = esc(&rules::time_label(b.end(), tz)),
                    name = esc(&b.name),
                    svc = esc(svc.map(|x| x.name.as_str()).unwrap_or("")),
                    who = n.staff(b.staff_id.as_deref()).map(|p| format!(" · {}", esc(&p.name))).unwrap_or_default(),
                    chip = status_chip(&b.status).0,
                    label = status_chip(&b.status).1,
                )
            }).collect::<String>()
        )
    };

    // Thirty bars: bookings made per day.
    let since = repo::db_time(Utc::now() - Duration::days(30));
    let per: HashMap<String, i64> = repo::per_day(&state.db, &since).await.into_iter().collect();
    let days: Vec<(String, i64)> = (0..30).rev().map(|i| { let d = (Utc::now() - Duration::days(i)).format("%Y-%m-%d").to_string(); let v = per.get(&d).copied().unwrap_or(0); (d, v) }).collect();
    let peak = days.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let made: i64 = days.iter().map(|(_, v)| v).sum();
    let chart = format!(
        r#"<div class="card padded lm-chart-card"><div class="lm-chart-head"><b>{made}</b><span class="muted">bookings made in the last 30 days</span></div><div class="lm-chart">{}</div></div>"#,
        days.iter().map(|(d, v)| format!(r#"<span title="{d}: {v}" style="--h:{}%"></span>"#, (v * 100 / peak).max(if *v > 0 { 6 } else { 2 }))).collect::<String>()
    );
    let empty_state = if n.services.is_empty() {
        format!(
            r#"<div class="card padded lm-empty rise" style="--i:3"><span class="ic tint-teal">{ic}</span><h3>Set up booking</h3>
<p class="muted">Add what people can book, who gives it and when you're open. Or start from a sample studio you can edit.</p>
<div class="lm-empty-actions"><a class="btn" href="/ghost/booking/services/new/">{add} New service</a>{sample}</div></div>"#,
            ic = svg("calendar"),
            add = svg("add"),
            sample = if actor.role.is_admin() { format!(r#"<form method="post" action="/ghost/booking/sample/"><button class="btn tonal" type="submit">{} Load a sample business</button></form>"#, svg("box")) } else { String::new() },
        )
    } else {
        String::new()
    };
    page_for_role(
        actor.role,
        "Booking & Appointment Scheduling",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{live}{empty_state}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}{e}</div>
<div class="lm-overview rise" style="--i:4"><div><div class="section">Today · {today_label}</div>{agenda}
<div class="section">Coming up</div>{upcoming}</div>
<div><div class="section">Needs approval</div>{pending}<div class="section">Bookings made</div>{chart}</div></div>"#,
            head = crate::tools::tool_head("booking", None, &format!(
                r#"<a class="btn text" href="{BASE}export.csv">{dl} Export CSV</a><a class="btn" href="{BOOKINGS}new/">{add} New booking</a>"#,
                dl = svg("import"),
                add = svg("add"),
            )),
            tabs = tabs_for(&state, "overview", &actor).await,
            banner = flash_banner(&q),
            today_label = esc(&Utc::now().with_timezone(&tz).format("%A %-d %B").to_string()),
            a = tile("Today", &st.today.to_string(), "Appointments today", Some("/ghost/booking/calendar/"), "calendar", "teal"),
            b = tile("Next 7 days", &st.week.to_string(), "Confirmed or waiting", Some(BOOKINGS), "trending", "blue"),
            c = tile("To approve", &st.pending.to_string(), "Requests waiting for you", Some(&format!("{BOOKINGS}?status=pending")), "check", if st.pending > 0 { "amber" } else { "slate" }),
            d = tile("Revenue", &rules::format_price(st.revenue_30d, &s.currency), "Booked in the last 30 days", None, "chart", "green"),
            e = tile("Customers", &st.customers.to_string(), &format!("{}% cancelled or no-show", st.cancel_rate), Some("/ghost/booking/customers/"), "group", "purple"),
            pending = if pending.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing waiting.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, pending.iter().map(|b| booking_row(b, &n, tz, &s, true)).collect::<String>()) },
            upcoming = if upcoming.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing else booked yet.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, upcoming.iter().map(|b| booking_row(b, &n, tz, &s, false)).collect::<String>()) },
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

pub(crate) async fn calendar_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let tz = zone(&state);
    let today = Utc::now().with_timezone(&tz).date_naive();
    let picked = q.get("week").and_then(|w| NaiveDate::parse_from_str(w, "%Y-%m-%d").ok()).unwrap_or(today);
    let monday = picked - Duration::days(picked.weekday().num_days_from_monday() as i64);
    let n = Names::load(&state).await;
    let staff_filter = q.get("staff").filter(|v| n.staff.iter().any(|p| &p.id == *v)).cloned();
    let from = rules::local(tz, monday, 0).unwrap_or_else(Utc::now);
    let to = from + Duration::days(7);
    let (rows, _) = repo::browse(&state.db, &repo::Query { from: Some(&repo::db_time(from)), to: Some(&repo::db_time(to)), staff_id: staff_filter.as_deref(), limit: 2000, ..Default::default() }).await;
    let rows: Vec<&repo::Booking> = rows.iter().filter(|b| b.status != "cancelled").collect();
    // The visible hours: the business's, widened to fit every booking.
    let mut first_hour = 24;
    let mut last_hour = 0;
    for d in &s.hours.0 {
        if let vuuut_core::directory::Day::Open(r) = d {
            for (a, b) in r {
                first_hour = first_hour.min((*a / 60) as i64);
                last_hour = last_hour.max(((*b + 59) / 60) as i64);
            }
        }
    }
    for b in &rows {
        let l = b.start().with_timezone(&tz);
        let e = b.end().with_timezone(&tz);
        first_hour = first_hour.min(chrono::Timelike::hour(&l) as i64);
        last_hour = last_hour.max(chrono::Timelike::hour(&e) as i64 + 1);
    }
    if first_hour >= last_hour {
        first_hour = 8;
        last_hour = 18;
    }
    let span = (last_hour - first_hour) * 60;
    let cols: String = (0..7)
        .map(|i| {
            let day = monday + Duration::days(i);
            let items: String = rows
                .iter()
                .filter(|b| b.start().with_timezone(&tz).date_naive() == day)
                .map(|b| {
                    let l = b.start().with_timezone(&tz);
                    let mins = chrono::Timelike::hour(&l) as i64 * 60 + chrono::Timelike::minute(&l) as i64 - first_hour * 60;
                    let len = (b.end() - b.start()).num_minutes().max(15);
                    let svc = n.service(&b.service_id);
                    format!(
                        r#"<a class="bk-ev tint-{tint} st-{st}" href="{BOOKINGS}{id}/" style="--top:{top:.3}%;--len:{len:.3}%" title="{name} · {svc}"><b>{time} {name}</b><span>{svc}{who}</span></a>"#,
                        tint = svc.map(|x| x.tint.as_str()).unwrap_or("slate"),
                        st = esc(&b.status),
                        id = esc(&b.id),
                        top = mins as f64 * 100.0 / span as f64,
                        len = len as f64 * 100.0 / span as f64,
                        name = esc(&b.name),
                        time = esc(&rules::time_label(b.start(), tz)),
                        svc = esc(svc.map(|x| x.name.as_str()).unwrap_or("")),
                        who = n.staff(b.staff_id.as_deref()).map(|p| format!(" · {}", esc(&p.name))).unwrap_or_default(),
                    )
                })
                .collect();
            format!(
                r#"<div class="bk-col{today}"><div class="bk-col-head"><span>{dow}</span><b>{dom}</b></div><div class="bk-col-body">{items}</div></div>"#,
                today = if day == today { " is-today" } else { "" },
                dow = day.format("%a"),
                dom = day.format("%-d"),
            )
        })
        .collect();
    let hours: String = (first_hour..last_hour).map(|h| format!(r#"<span>{}</span>"#, vuuut_core::directory::clock_label((h * 60) as u16))).collect();
    let nav = |d: NaiveDate| {
        let mut u = format!("/ghost/booking/calendar/?week={}", d.format("%Y-%m-%d"));
        if let Some(sf) = &staff_filter {
            u.push_str(&format!("&amp;staff={sf}"));
        }
        u
    };
    let staff_opts: String = std::iter::once(format!(r#"<option value="">Everyone</option>"#))
        .chain(n.staff.iter().map(|p| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&p.id), s = if staff_filter.as_deref() == Some(p.id.as_str()) { " selected" } else { "" }, n = esc(&p.name))))
        .collect();
    page_for_role(
        actor.role,
        "Calendar",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="bk-cal-bar rise" style="--i:2"><div class="bk-cal-nav"><a class="icon-btn" style="transform:rotate(180deg)" href="{prev}" title="Previous week">{chev}</a><a class="btn text small" href="/ghost/booking/calendar/">Today</a><a class="icon-btn" href="{next}" title="Next week">{chev}</a>
<h2>{range}</h2></div>
<form method="get" action="/ghost/booking/calendar/" class="bk-cal-filter"><input type="hidden" name="week" value="{week}"><label class="field fixed" style="margin:0"><select name="staff" onchange="this.form.submit()">{staff_opts}</select><span>Staff</span>{SELECT_CARET}</label></form></div>
<div class="card bk-cal rise" style="--i:3;--rows:{rows_n}"><div class="bk-hours" style="--rows:{rows_n}">{hours}</div><div class="bk-week" style="--rows:{rows_n}">{cols}</div></div>
<p class="muted rise" style="--i:4;margin:10px 4px">Times in {tz}. {count}.</p>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, &format!(r#"<a class="btn" href="{BOOKINGS}new/">{} New booking</a>"#, svg("add"))),
            tabs = tabs_for(&state, "calendar", &actor).await,
            banner = flash_banner(&q),
            prev = nav(monday - Duration::days(7)),
            next = nav(monday + Duration::days(7)),
            chev = svg("chevron"),
            range = esc(&format!("{} – {}", monday.format("%-d %b"), (monday + Duration::days(6)).format("%-d %b %Y"))),
            week = monday.format("%Y-%m-%d"),
            rows_n = last_hour - first_hour,
            tz = esc(tz.name()),
            count = countable(rows.len() as i64, "booking"),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// Bookings
// ---------------------------------------------------------------------------

pub(crate) async fn bookings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let tz = zone(&state);
    let n = Names::load(&state).await;
    let when = q.get("when").map(String::as_str).filter(|v| matches!(*v, "upcoming" | "past" | "all")).unwrap_or("upcoming");
    let status = q.get("status").map(String::as_str).filter(|v| rules::Status::ALL.iter().any(|x| x.key() == *v));
    let service = q.get("service").filter(|v| n.services.iter().any(|x| &x.id == *v)).cloned();
    let staff = q.get("staff").filter(|v| n.staff.iter().any(|x| &x.id == *v)).cloned();
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = vuuut_core::directory::search_terms(&query);
    let now = repo::db_time(today_start(tz));
    let (from, to) = match when {
        "upcoming" => (Some(now.as_str()), None),
        "past" => (None, Some(now.as_str())),
        _ => (None, None),
    };
    let (rows, total) = repo::browse(&state.db, &repo::Query { from, to, status, staff_id: staff.as_deref(), service_id: service.as_deref(), terms: &terms, limit: 300, ..Default::default() }).await;
    let rows: Vec<repo::Booking> = if when == "past" { rows.into_iter().rev().collect() } else { rows };
    let opt = |v: &str, l: &str, cur: &str| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), l = esc(l), s = if cur == v { " selected" } else { "" });
    let status_opts: String = std::iter::once(opt("", "Any status", status.unwrap_or(""))).chain(rules::Status::ALL.iter().map(|x| opt(x.key(), x.label(), status.unwrap_or("")))).collect();
    let svc_opts: String = std::iter::once(opt("", "All services", service.as_deref().unwrap_or(""))).chain(n.services.iter().map(|x| opt(&x.id, &x.name, service.as_deref().unwrap_or("")))).collect();
    let staff_opts: String = std::iter::once(opt("", "Everyone", staff.as_deref().unwrap_or(""))).chain(n.staff.iter().map(|x| opt(&x.id, &x.name, staff.as_deref().unwrap_or("")))).collect();
    let when_opts: String = [("upcoming", "Upcoming"), ("past", "Past"), ("all", "All")].iter().map(|(v, l)| opt(v, l, when)).collect();
    page_for_role(
        actor.role,
        "Bookings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="dr-filter rise" style="--i:2" method="get" action="{BOOKINGS}">
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{qv}" placeholder=" " maxlength="100"><span>Name, email, phone or reference</span></label>
<label class="field fixed" style="margin:0"><select name="when">{when_opts}</select><span>When</span>{SELECT_CARET}</label>
<label class="field fixed" style="margin:0"><select name="status">{status_opts}</select><span>Status</span>{SELECT_CARET}</label>
<label class="field fixed" style="margin:0"><select name="service">{svc_opts}</select><span>Service</span>{SELECT_CARET}</label>
<label class="field fixed" style="margin:0"><select name="staff">{staff_opts}</select><span>Staff</span>{SELECT_CARET}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, &format!(r#"<a class="btn text" href="{BASE}export.csv">{dl} Export CSV</a><a class="btn" href="{BOOKINGS}new/">{add} New booking</a>"#, dl = svg("import"), add = svg("add"))),
            tabs = tabs_for(&state, "bookings", &actor).await,
            banner = flash_banner(&q),
            qv = esc(&query),
            search = svg("search"),
            count = esc(&countable(total, "booking")),
            body = if rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No bookings match.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, rows.iter().map(|b| booking_row(b, &n, tz, &s, true)).collect::<String>()) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn booking_detail(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That booking could not be found.");
    }
    let Some(b) = repo::find_booking(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That booking could not be found.") };
    let s = booking_settings(&state);
    let tz = zone(&state);
    let n = Names::load(&state).await;
    let svc = n.service(&b.service_id).cloned().unwrap_or_default();
    let (chip, label) = status_chip(&b.status);
    let local = b.start().with_timezone(&tz);
    // Quick actions post their own small forms (rendered outside the
    // editor form); the buttons sit in the Status card via `form=`.
    let act = |a: &str, l: &str, cls: &str, confirm: &str| (a.to_string(), l.to_string(), cls.to_string(), confirm.to_string());
    let mut actions = Vec::new();
    match b.status.as_str() {
        "pending" => {
            actions.push(act("confirm", "Confirm", "", ""));
            actions.push(act("decline", "Decline", "tonal", "Decline this request?|The customer is emailed that it could not go ahead."));
        }
        "confirmed" | "awaiting_payment" => {
            if b.start() <= Utc::now() {
                actions.push(act("complete", "Mark completed", "", ""));
                actions.push(act("no_show", "No-show", "tonal", ""));
            }
            actions.push(act("cancel", "Cancel booking", "text danger", "Cancel this booking?|The customer is emailed and the time is freed."));
        }
        "cancelled" | "no_show" | "completed" => actions.push(act("restore", "Restore as confirmed", "tonal", "")),
        _ => {}
    }
    if b.price_cents > 0 && b.payment_status != "paid" {
        actions.push(act("paid", "Mark paid", "tonal", ""));
    }
    if matches!(b.status.as_str(), "confirmed" | "pending") {
        actions.push(act("resend", "Resend confirmation", "text", ""));
    }
    let staff_opts: String = std::iter::once(r#"<option value="">Nobody in particular</option>"#.to_string())
        .chain(n.staff.iter().map(|p| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&p.id), s = if b.staff_id.as_deref() == Some(p.id.as_str()) { " selected" } else { "" }, n = esc(&p.name))))
        .collect();
    let answers: String = b.answers().iter().map(|(k, v)| format!("<dt>{}</dt><dd>{}</dd>", esc(k), esc(v))).collect();
    let others = repo::browse(&state.db, &repo::Query { email: Some(&b.email), limit: 20, newest: true, ..Default::default() }).await.0;
    let history: String = others.iter().filter(|x| x.id != b.id).take(6).map(|x| booking_row(x, &n, tz, &s, false)).collect();
    let status_opts: String = rules::Status::ALL.iter().map(|x| format!(r#"<option value="{k}"{s}>{l}</option>"#, k = x.key(), s = if x.key() == b.status { " selected" } else { "" }, l = x.label())).collect();
    let pay_opts: String = [("none", "Nothing to pay"), ("unpaid", "Not paid yet"), ("paid", "Paid"), ("refunded", "Refunded")].iter().map(|(k, l)| format!(r#"<option value="{k}"{s}>{l}</option>"#, s = if *k == b.payment_status { " selected" } else { "" })).collect();
    let card = |title: &str, icon: &str, tint: &str, body: &str| format!(r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title));
    let when_card = card("When & with whom", "calendar", "teal", &format!(
        r#"<div class="bk-detail-head"><span class="bk-big tint-{tint}"><b>{dom}</b><small>{mon}</small></span><div><h3 class="bk-when-t">{day}</h3><p class="muted">{t1} – {t2} ({tzn}){seats}</p><p class="muted">{where_}</p></div></div>
<div class="ec-grid3"><label class="field"><input type="date" name="date" value="{date}" required><span>Date</span></label>
<label class="field"><input type="time" name="time" value="{time}" required step="300"><span>Start time</span></label>
<label class="field fixed"><select name="staff_id">{staff_opts}</select><span>With</span>{SELECT_CARET}</label></div>
<p class="muted ec-hint">Change the date, time or person and save to move the booking. Staff may choose any time; the customer's page and reminder follow.</p>"#,
        tint = esc(&svc.tint),
        dom = local.format("%-d"),
        mon = local.format("%b"),
        day = esc(&rules::day_label(b.start(), tz)),
        t1 = esc(&rules::time_label(b.start(), tz)),
        t2 = esc(&rules::time_label(b.end(), tz)),
        tzn = esc(tz.name()),
        seats = if b.seats > 1 { format!(" · {} places", b.seats) } else { String::new() },
        where_ = esc(&format!("{}{}", svc.where_label(&s), n.staff(b.staff_id.as_deref()).map(|p| format!(" · with {}", p.name)).unwrap_or_default())),
        date = local.format("%Y-%m-%d"),
        time = local.format("%H:%M"),
    ));
    let customer_card = card("Customer", "person", "blue", &format!(
        r#"<div class="ec-grid2"><label class="field"><input name="name" value="{name}" required maxlength="120" placeholder=" "><span>Name</span></label>
<label class="field"><input type="email" name="email" value="{email}" required maxlength="191" placeholder=" "><span>Email</span></label></div>
<div class="ec-grid2"><label class="field"><input name="phone" value="{phone}" maxlength="40" placeholder=" "><span>Phone</span></label>{tzc}</div>
<label class="field"><textarea name="notes" rows="3" maxlength="2000" placeholder=" ">{notes}</textarea><span>Customer's notes</span></label>
{answers}<p class="ec-hint"><a href="mailto:{email}">Email {first}</a>{tel}</p>"#,
        name = esc(&b.name),
        email = esc(&b.email),
        phone = esc(&b.phone),
        tzc = if b.timezone.is_empty() { String::new() } else { format!(r#"<label class="field"><input value="{}" disabled placeholder=" "><span>Their time zone</span></label>"#, esc(&b.timezone)) },
        notes = esc(&b.notes),
        answers = if answers.is_empty() { String::new() } else { format!(r#"<div class="ec-sublabel">Answers</div><dl class="bk-dl">{answers}</dl>"#) },
        first = esc(b.name.split_whitespace().next().unwrap_or("")),
        tel = if b.phone.is_empty() { String::new() } else { format!(r#" · <a href="tel:{p}">Call {p}</a>"#, p = esc(&b.phone)) },
    ));
    let main = format!(
        "{when_card}{customer_card}{history}",
        history = if history.is_empty() { String::new() } else { format!(r#"<section class="card ec-card bk-hist"><h2 class="ec-card-title" style="padding:18px 20px 4px"><span class="ic tint-purple">{}</span>Other bookings by {}</h2><div class="rows">{history}</div></section>"#, svg("history"), esc(b.name.split_whitespace().next().unwrap_or(""))) },
    );
    let side = format!(
        "{st}{pay}{note}{det}",
        st = card("Status", "eye", "green", &format!(
            r#"<p style="margin:0 0 10px"><span class="chip {chip}">{label}</span></p><label class="field fixed"><select name="status">{status_opts}</select><span>Status</span>{SELECT_CARET}</label>
<label class="ec-check"><input type="checkbox" name="notify" value="1" checked> Email the customer about changes</label><div class="bk-actions">{actions}</div>"#,
            actions = actions.iter().map(|(a, l, cls, _)| format!(r#"<button class="btn {cls} small" type="submit" form="bk-act-{a}">{l}</button>"#)).collect::<String>(),
        )),
        pay = card("Payment", "sell", "amber", &format!(
            r#"<p class="bk-price">{price}</p><label class="field fixed"><select name="payment_status">{pay_opts}</select><span>Payment</span>{SELECT_CARET}</label><p class="muted ec-hint">{how}{pref}</p>"#,
            price = esc(&rules::format_price(b.price_cents, &s.currency)),
            how = esc(if svc.payment == "online" { "Paid online through Stripe." } else if svc.payment == "onsite" { "Paid at the appointment." } else { "Free, or settled another way." }),
            pref = if b.payment_ref.is_empty() { String::new() } else { format!(" Reference {}.", esc(&b.payment_ref)) },
        )),
        note = card("Internal note", "notes", "slate", &format!(r#"<label class="field"><textarea name="internal_note" rows="4" placeholder=" " maxlength="4000">{}</textarea><span>Only staff see this</span></label>"#, esc(&b.internal_note))),
        det = card("Details", "receipt", "blue", &format!(
            r#"<dl class="bk-dl"><dt>Reference</dt><dd>{r}</dd><dt>Booked</dt><dd>{created} · {source}</dd>{cancel}<dt>Customer page</dt><dd><a href="{manage}" target="_blank" rel="noopener">Open</a></dd></dl>
<div class="ec-side-actions"><button class="btn text danger" type="submit" form="bk-delete">{del} Delete booking</button></div>"#,
            r = esc(&b.r#ref),
            created = esc(b.created_at.get(..16).unwrap_or("")),
            source = if b.source == "staff" { "by staff" } else { "online" },
            cancel = if b.cancel_reason.is_empty() { String::new() } else { format!("<dt>Cancelled</dt><dd>{}</dd>", esc(&b.cancel_reason)) },
            manage = esc(&vuuut_api::booking::manage_url(&state, &s, &b)),
            del = svg("delete"),
        )),
    );
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{BOOKINGS}" title="Back to bookings">{back}</a>
<div><h1>{name}</h1><p class="sub">{svc_name} · {when} · <span class="chip {chip}">{label}</span></p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="bk-booking">{check} Save</button></div></header>
{banner}
<form method="post" action="{BOOKINGS}{id}/delete/" id="bk-delete" data-confirm="Delete this booking?|It is removed for good. Cancel it instead to keep the record."></form>{quick}
<form id="bk-booking" class="ec-edit" method="post" action="{BOOKINGS}{id}/"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        name = esc(&b.name),
        svc_name = esc(&svc.name),
        when = esc(&format!("{} {}", local.format("%a %-d %b"), rules::time_label(b.start(), tz))),
        check = svg("check"),
        banner = flash_banner(&q),
        id = esc(&b.id),
        quick = actions.iter().map(|(a, _, _, c)| format!(
            r#"<form method="post" action="{BOOKINGS}{id}/action/" id="bk-act-{a}"{c}><input type="hidden" name="action" value="{a}"></form>"#,
            id = esc(&b.id),
            c = if c.is_empty() { String::new() } else { format!(r#" data-confirm="{}""#, esc(c)) },
        )).collect::<String>(),
    );
    page_for_role(actor.role, &format!("{} · {}", b.name, svc.name), "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ActionForm {
    action: String,
}

pub(crate) async fn booking_action(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back_list = headers.get(header::REFERER).and_then(|v| v.to_str().ok()).map(|r| r.contains("/booking/bookings/") && !r.contains(&id) || r.ends_with("/booking/")).unwrap_or(false);
    let back = if back_list { BASE.to_string() } else { format!("{BOOKINGS}{id}/") };
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(BOOKINGS, "error", "That booking could not be found.");
    }
    let Some(b) = repo::find_booking(&state.db, &id).await else { return flash_to(BOOKINGS, "error", "That booking could not be found.") };
    let db = &state.db;
    let r = match f.action.as_str() {
        "confirm" => repo::set_status(db, &id, "confirmed", "").await.map(|_| "Confirmed. The customer has been emailed."),
        "decline" => repo::set_status(db, &id, "cancelled", "The request could not be accepted.").await.map(|_| "Declined. The customer has been emailed."),
        "cancel" => repo::set_status(db, &id, "cancelled", "Cancelled by the business.").await.map(|_| "Cancelled. The customer has been emailed."),
        "complete" => repo::set_status(db, &id, "completed", "").await.map(|_| "Marked completed."),
        "no_show" => repo::set_status(db, &id, "no_show", "").await.map(|_| "Marked as a no-show."),
        "restore" => repo::set_status(db, &id, "confirmed", "").await.map(|_| "Restored."),
        "paid" => repo::set_payment_status(db, &id, "paid").await.map(|_| "Marked paid."),
        "resend" => Ok("Confirmation sent again."),
        _ => return flash_to(&back, "error", "That is not something a booking can do."),
    };
    match r {
        Ok(msg) => {
            if let Some(after) = repo::find_booking(db, &id).await {
                match f.action.as_str() {
                    "confirm" | "resend" => vuuut_api::booking::announce(&state, &after, false).await,
                    "decline" | "cancel" if b.status != "cancelled" => vuuut_api::booking::notify_cancelled(&state, &after, false).await,
                    _ => {}
                }
            }
            flash_to(&back, "ok", msg)
        }
        Err(e) => {
            tracing::error!("booking: {} on {id} failed: {}", f.action, e.message());
            flash_to(&back, "error", "That could not be saved.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct MoveForm {
    date: String,
    time: String,
    staff_id: String,
    notify: String,
}

pub(crate) async fn booking_move(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<MoveForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BOOKINGS}{id}/");
    let Some(b) = repo::find_booking(&state.db, &id).await else { return flash_to(BOOKINGS, "error", "That booking could not be found.") };
    let tz = zone(&state);
    let (Ok(d), Some(m)) = (NaiveDate::parse_from_str(f.date.trim(), "%Y-%m-%d"), vuuut_core::directory::parse_clock(&f.time)) else {
        return flash_to(&back, "error", "Choose a date and a start time.");
    };
    let Some(start) = rules::local(tz, d, m as i64) else { return flash_to(&back, "error", "That time does not exist on that day (a clock change).") };
    let len = b.end() - b.start();
    let staff = Some(f.staff_id.trim()).filter(|s| vuuut_core::ids::is_objectid(s));
    match repo::reschedule(&state.db, &id, start, start + len, staff).await {
        Ok(()) => {
            if f.notify == "1" {
                if let Some(after) = repo::find_booking(&state.db, &id).await {
                    vuuut_api::booking::notify_moved(&state, &after).await;
                }
            }
            flash_to(&back, "ok", "Moved.")
        }
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct NoteForm {
    note: String,
}

pub(crate) async fn booking_note(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<NoteForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BOOKINGS}{id}/");
    match repo::set_note(&state.db, &id, &vuuut_core::directory::clean_text(&f.note, 4000)).await {
        Ok(()) => flash_to(&back, "ok", "Note saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn booking_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_booking(&state.db, &id).await {
        Ok(()) => flash_to(BOOKINGS, "ok", "Booking deleted."),
        Err(_) => flash_to(BOOKINGS, "error", "That could not be deleted."),
    }
}

/// A booking made by staff: for a phone call, a walk-in, a regular.
pub(crate) async fn booking_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let n = Names::load(&state).await;
    let tz = zone(&state);
    let now = Utc::now().with_timezone(&tz);
    let svc_opts: String = n.services.iter().filter(|x| x.status != "draft").map(|x| format!(r#"<option value="{}">{} · {}</option>"#, esc(&x.id), esc(&x.name), esc(&rules::duration_label(x.duration_minutes)))).collect();
    let staff_opts: String = std::iter::once(r#"<option value="">Nobody in particular</option>"#.to_string()).chain(n.staff.iter().filter(|p| p.status == "active").map(|p| format!(r#"<option value="{}">{}</option>"#, esc(&p.id), esc(&p.name)))).collect();
    let card = |title: &str, icon: &str, tint: &str, body: &str| format!(r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title));
    let main = format!(
        "{}{}",
        card("What & when", "calendar", "teal", &format!(
            r#"<label class="field fixed"><select name="service_id" required>{svc_opts}</select><span>Service</span>{SELECT_CARET}</label>
<div class="ec-grid3"><label class="field"><input type="date" name="date" value="{date}" required><span>Date</span></label>
<label class="field"><input type="time" name="time" value="10:00" step="300" required><span>Start time ({tzn})</span></label>
<label class="field"><input type="number" name="seats" value="1" min="1" max="20"><span>Places</span></label></div>
<p class="muted ec-hint">Staff bookings are confirmed at once and may go outside the hours customers can choose.</p>"#,
            date = now.format("%Y-%m-%d"),
            tzn = esc(tz.name()),
        )),
        card("Customer", "person", "blue", r#"<div class="ec-grid2"><label class="field"><input name="name" required maxlength="120" placeholder=" "><span>Name</span></label>
<label class="field"><input type="email" name="email" required maxlength="191" placeholder=" "><span>Email</span></label></div>
<label class="field"><input name="phone" maxlength="40" placeholder=" "><span>Phone</span></label>
<label class="field"><textarea name="notes" rows="3" maxlength="2000" placeholder=" "></textarea><span>Notes</span></label>"#),
    );
    let side = card("With", "group", "purple", &format!(r#"<label class="field fixed"><select name="staff_id">{staff_opts}</select><span>Staff</span>{SELECT_CARET}</label><p class="muted ec-hint">The customer is emailed a confirmation with a link to change or cancel.</p>"#));
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{BOOKINGS}" title="Back to bookings">{back}</a>
<div><h1>New booking</h1><p class="sub">For a phone call, a walk-in or a regular.</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="bk-booking-new">{check} Create</button></div></header>
{banner}
<form id="bk-booking-new" class="ec-edit" method="post" action="{BOOKINGS}new/"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        check = svg("check"),
        banner = flash_banner(&q),
    );
    page_for_role(actor.role, "New booking", "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

pub(crate) async fn booking_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BOOKINGS}new/");
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let Some(svc) = repo::find_service(&state.db, get("service_id")).await else { return flash_to(&back, "error", "Choose a service.") };
    let tz = zone(&state);
    let (Ok(d), Some(m)) = (NaiveDate::parse_from_str(get("date").trim(), "%Y-%m-%d"), vuuut_core::directory::parse_clock(get("time"))) else {
        return flash_to(&back, "error", "Choose a date and a start time.");
    };
    let Some(start) = rules::local(tz, d, m as i64) else { return flash_to(&back, "error", "That time does not exist on that day.") };
    let input = match rules::BookingInput::from_form(&form, &[], false, svc.capacity) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    let staff = Some(get("staff_id").trim()).filter(|s| vuuut_core::ids::is_objectid(s));
    match vuuut_api::booking::book(&state, &svc, start, staff, &input, tz.name(), None, "staff").await {
        Ok(vuuut_api::booking::Booked::Done(b)) | Ok(vuuut_api::booking::Booked::Pay(b, _)) => {
            flash_to(&format!("{BOOKINGS}{}/", b.id), "ok", "Booking created.")
        }
        Err(m) => flash_to(&back, "error", &m),
    }
}

// ---------------------------------------------------------------------------
// customers and export
// ---------------------------------------------------------------------------

pub(crate) async fn customers_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let tz = zone(&state);
    let list = repo::customers(&state.db, 500).await;
    let rows: String = list
        .iter()
        .map(|(email, name, n, last, spent)| {
            format!(
                r#"<div class="row dr-listing"><span class="lm-avatar">{i}</span><div class="body"><span class="title"><a href="{BOOKINGS}?when=all&amp;q={qe}">{name}</a></span><span class="detail">{email} · {nb} · last {last}</span></div><div class="trailing"><b>{spent}</b></div></div>"#,
                i = esc(&name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
                qe = esc(&vuuut_core::social_media::encode(email)),
                name = esc(name),
                email = esc(email),
                nb = countable(*n, "booking"),
                last = esc(&repo::parse_time(last).map(|d| rules::day_label(d, tz)).unwrap_or_default()),
                spent = esc(&rules::format_price(*spent, &s.currency)),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Customers",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead" style="margin-top:16px">{count}, from every booking made.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, &format!(r#"<a class="btn text" href="{BASE}export.csv">{} Export CSV</a>"#, svg("import"))),
            tabs = tabs_for(&state, "customers", &actor).await,
            banner = flash_banner(&q),
            count = esc(&countable(list.len() as i64, "customer")),
            body = if rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No customers yet.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn export_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let tz = zone(&state);
    let n = Names::load(&state).await;
    let (rows, _) = repo::browse(&state.db, &repo::Query { limit: 2000, newest: true, ..Default::default() }).await;
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("reference,status,service,staff,date,start,end,name,email,phone,seats,price,payment,notes,created\n");
    for b in &rows {
        out.push_str(&[
            cell(&b.r#ref), cell(&b.status), cell(n.service(&b.service_id).map(|x| x.name.as_str()).unwrap_or("")), cell(n.staff(b.staff_id.as_deref()).map(|p| p.name.as_str()).unwrap_or("")),
            cell(&b.start().with_timezone(&tz).format("%Y-%m-%d").to_string()), cell(&b.start().with_timezone(&tz).format("%H:%M").to_string()), cell(&b.end().with_timezone(&tz).format("%H:%M").to_string()),
            cell(&b.name), cell(&b.email), cell(&b.phone), cell(&b.seats.to_string()), cell(&format!("{}.{:02}", b.price_cents / 100, b.price_cents % 100)), cell(&b.payment_status), cell(&b.notes), cell(&b.created_at),
        ].join(","));
        out.push('\n');
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"bookings-{}.csv\"", crate::export_date_stamp()))], out).into_response()
}

/// The booking editor's Save: time and person, customer details, status,
/// payment and the internal note, in one go. The customer is emailed when
/// the time moves or the booking is confirmed or cancelled, if asked.
pub(crate) async fn booking_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{BOOKINGS}{id}/");
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(BOOKINGS, "error", "That booking could not be found.");
    }
    let Some(before) = repo::find_booking(&state.db, &id).await else { return flash_to(BOOKINGS, "error", "That booking could not be found.") };
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let tz = zone(&state);
    // Customer.
    let name = vuuut_core::directory::clean(get("name"), 120);
    let email = vuuut_core::directory::clean(get("email"), 191).to_lowercase();
    if name.chars().count() < 2 || !vuuut_core::directory::is_email(&email) {
        return flash_to(&back, "error", "The customer needs a name and a valid email address.");
    }
    let Some(phone) = vuuut_core::directory::clean_phone(get("phone")) else { return flash_to(&back, "error", "That phone number does not look right.") };
    let notes = vuuut_core::directory::clean_text(get("notes"), 2000);
    // Time and person.
    let (Ok(d), Some(m)) = (NaiveDate::parse_from_str(get("date").trim(), "%Y-%m-%d"), vuuut_core::directory::parse_clock(get("time"))) else {
        return flash_to(&back, "error", "Choose a date and a start time.");
    };
    let Some(start) = rules::local(tz, d, m as i64) else { return flash_to(&back, "error", "That time does not exist on that day (a clock change).") };
    let staff = Some(get("staff_id").trim()).filter(|s| vuuut_core::ids::is_objectid(s)).map(str::to_string);
    let moved = start != before.start() || staff != before.staff_id;
    let status = rules::Status::from_key(get("status")).key();
    let payment = match get("payment_status") { p @ ("none" | "unpaid" | "paid" | "refunded") => p, _ => before.payment_status.as_str() };
    let db = &state.db;
    let saved = async {
        repo::update_customer(db, &id, &name, &email, &phone, &notes).await?;
        if moved {
            repo::reschedule(db, &id, start, start + (before.end() - before.start()), staff.as_deref()).await?;
        }
        if status != before.status {
            repo::set_status(db, &id, status, if status == "cancelled" { "Cancelled by the business." } else { "" }).await?;
        }
        if payment != before.payment_status {
            repo::set_payment_status(db, &id, payment).await?;
        }
        repo::set_note(db, &id, &vuuut_core::directory::clean_text(get("internal_note"), 4000)).await
    }
    .await;
    if let Err(e) = saved {
        tracing::error!("booking: saving {id} failed: {}", e.message());
        return flash_to(&back, "error", "The booking could not be saved.");
    }
    if get("notify") == "1" {
        if let Some(after) = repo::find_booking(db, &id).await {
            if status == "cancelled" && before.status != "cancelled" {
                vuuut_api::booking::notify_cancelled(&state, &after, false).await;
            } else if status == "confirmed" && before.status != "confirmed" {
                vuuut_api::booking::announce(&state, &after, false).await;
            } else if moved && matches!(status, "confirmed" | "pending") {
                vuuut_api::booking::notify_moved(&state, &after).await;
            }
        }
    }
    flash_to(&back, "ok", if moved { "Saved — the booking has moved." } else { "Saved." })
}
