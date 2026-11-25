//! Booking & Appointment Scheduling: what the site and the admin share.
//!
//! * [`day_slots`] / [`month_days`] — free start times, from the rules in
//!   `vuuut_core::booking` fed with the database's staff, bookings and time
//!   off. A business with nobody set up is one "person" with the business
//!   hours, so a sole trader can take bookings with no staff screen at all.
//! * [`book`] — take a booking (re-checking the slot), choose who gives it,
//!   set its status, send the emails.
//! * [`start_payment`] / [`settle_payment`] — Stripe Checkout for services
//!   paid online; the slot is held for `PAYMENT_HOLD_MINUTES`.
//! * [`spawn`] — once a minute: release unpaid holds, send reminders.
//! * the emails: confirmation, request received, staff notice, reminder,
//!   cancellation and change of time.

use crate::AppState;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use vuuut_core::booking::{self as rules, Settings};
use vuuut_db::repos::booking as repo;

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

pub fn tz(state: &AppState, s: &Settings) -> chrono_tz::Tz {
    s.tz(&state.settings.get_str("timezone").unwrap_or_default())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The people who can give `service`, as the scheduler sees them. With
/// nobody linked, everyone active; with nobody at all, the business itself
/// (id ``) on business hours.
pub async fn people(state: &AppState, service: &repo::Service, only: Option<&str>) -> Vec<repo::Staff> {
    let all = repo::staff(&state.db, true).await;
    let linked = repo::staff_ids_for(&state.db, &service.id).await;
    let mut chosen: Vec<repo::Staff> = if linked.is_empty() { all.clone() } else { all.into_iter().filter(|p| linked.contains(&p.id)).collect() };
    if let Some(o) = only.filter(|o| !o.is_empty()) {
        chosen.retain(|p| p.id == o);
    }
    if chosen.is_empty() && only.is_none_or(|o| o.is_empty()) && repo::staff(&state.db, true).await.is_empty() {
        chosen.push(repo::Staff { id: String::new(), name: state.settings.get_str("title").unwrap_or_default(), status: "active".into(), ..Default::default() });
    }
    chosen
}

async fn persons(state: &AppState, service: &repo::Service, people: &[repo::Staff], from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<rules::Person> {
    let services = repo::services(&state.db, false).await;
    let mut busy = repo::busy(&state.db, from, to, &services).await;
    let everyone = busy.remove("*").unwrap_or_default();
    let _ = service;
    people
        .iter()
        .map(|p| {
            let mut b = busy.get(&p.id).cloned().unwrap_or_default();
            b.extend(everyone.iter().cloned());
            rules::Person { id: p.id.clone(), hours: p.hours(), busy: b }
        })
        .collect()
}

/// Free start times on `date` (the business's local date).
pub async fn day_slots(state: &AppState, service: &repo::Service, date: NaiveDate, only: Option<&str>) -> Vec<rules::Slot> {
    let s = settings(state);
    let zone = tz(state, &s);
    let people = people(state, service, only).await;
    if people.is_empty() {
        return Vec::new();
    }
    let from = rules::local(zone, date, 0).unwrap_or_else(Utc::now);
    let to = from + Duration::days(1);
    let persons = persons(state, service, &people, from, to).await;
    rules::slots_for_day(date, zone, Utc::now(), &service.rule(), &s.hours, &persons)
}

/// The days of `year`-`month` with at least one free time.
pub async fn month_days(state: &AppState, service: &repo::Service, year: i32, month: u32, only: Option<&str>) -> Vec<NaiveDate> {
    let s = settings(state);
    let zone = tz(state, &s);
    let people = people(state, service, only).await;
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else { return Vec::new() };
    if people.is_empty() {
        return Vec::new();
    }
    let from = rules::local(zone, first, 0).unwrap_or_else(Utc::now);
    let persons = persons(state, service, &people, from, from + Duration::days(32)).await;
    let now = Utc::now();
    let rule = service.rule();
    let mut out = Vec::new();
    let mut d = first;
    while d.month() == month {
        if !rules::slots_for_day(d, zone, now, &rule, &s.hours, &persons).is_empty() {
            out.push(d);
        }
        d = match d.succ_opt() {
            Some(n) => n,
            None => break,
        };
    }
    out
}

/// The first day from today with a free time, looking `days` ahead.
pub async fn next_free_day(state: &AppState, service: &repo::Service, only: Option<&str>, days: i64) -> Option<NaiveDate> {
    let s = settings(state);
    let zone = tz(state, &s);
    let people = people(state, service, only).await;
    if people.is_empty() {
        return None;
    }
    let today = Utc::now().with_timezone(&zone).date_naive();
    let from = rules::local(zone, today, 0).unwrap_or_else(Utc::now);
    let persons = persons(state, service, &people, from, from + Duration::days(days + 1)).await;
    let rule = service.rule();
    (0..days).filter_map(|i| today.checked_add_signed(Duration::days(i))).find(|d| !rules::slots_for_day(*d, zone, Utc::now(), &rule, &s.hours, &persons).is_empty())
}

pub enum Booked {
    Done(repo::Booking),
    /// Pay first: the URL of Stripe Checkout.
    Pay(repo::Booking, String),
}

/// Take a booking at `start` (UTC). The slot is checked again here — the
/// page the visitor chose it on may be minutes old. `staff_pref` empty means
/// whoever is free (the first listed). Staff bookings (`source` = `staff`)
/// skip the checks and are confirmed.
#[allow(clippy::too_many_arguments)]
pub async fn book(
    state: &AppState,
    service: &repo::Service,
    start: DateTime<Utc>,
    staff_pref: Option<&str>,
    input: &rules::BookingInput,
    customer_tz: &str,
    member_id: Option<&str>,
    source: &str,
) -> Result<Booked, String> {
    let s = settings(state);
    let zone = tz(state, &s);
    let staff_booking = source == "staff";
    let staff_id: Option<String> = if staff_booking {
        staff_pref.filter(|p| !p.is_empty()).map(str::to_string)
    } else {
        let date = start.with_timezone(&zone).date_naive();
        let slots = day_slots(state, service, date, staff_pref).await;
        let Some(slot) = slots.iter().find(|x| x.start == start) else {
            return Err("Sorry — that time has just been taken. Please choose another.".into());
        };
        if slot.seats_left < input.seats {
            return Err(format!("Only {} place{} left at that time.", slot.seats_left, if slot.seats_left == 1 { "" } else { "s" }));
        }
        // "Anyone": spread the work — the free person with the fewest bookings that day.
        let mut pick = slot.staff.first().cloned().unwrap_or_default();
        if staff_pref.is_none_or(|p| p.is_empty()) && slot.staff.len() > 1 {
            let day_from = rules::local(zone, date, 0).unwrap_or(start);
            let (rows, _) = repo::browse(&state.db, &repo::Query { from: Some(&repo::db_time(day_from)), to: Some(&repo::db_time(day_from + Duration::days(1))), status: Some("active"), limit: 500, ..Default::default() }).await;
            pick = slot.staff.iter().min_by_key(|id| rows.iter().filter(|b| b.staff_id.as_deref() == Some(id.as_str())).count()).cloned().unwrap_or(pick);
        }
        Some(pick).filter(|p| !p.is_empty())
    };
    let pay_online = !staff_booking && service.payment() == rules::Payment::Online && service.price_cents > 0 && crate::ecom::stripe(state).is_some();
    let (status, payment_status) = if staff_booking {
        ("confirmed", if service.price_cents > 0 { "unpaid" } else { "none" })
    } else if pay_online {
        ("awaiting_payment", "unpaid")
    } else if service.approval != 0 {
        ("pending", if service.price_cents > 0 { "unpaid" } else { "none" })
    } else {
        ("confirmed", if service.price_cents > 0 { "unpaid" } else { "none" })
    };
    let b = repo::insert_booking(&state.db, &repo::NewBooking {
        service,
        staff_id: staff_id.as_deref(),
        start,
        input,
        status,
        payment_status,
        timezone: customer_tz,
        member_id,
        source,
    })
    .await
    .map_err(|e| {
        tracing::error!("booking: saving failed: {}", e.message());
        "Your booking could not be saved. Please try again.".to_string()
    })?;
    if pay_online {
        return match start_payment(state, service, &b).await {
            Some(url) => Ok(Booked::Pay(b, url)),
            None => {
                let _ = repo::set_status(&state.db, &b.id, if service.approval != 0 { "pending" } else { "confirmed" }, "").await;
                let b = repo::find_booking(&state.db, &b.id).await.unwrap_or(b);
                announce(state, &b, true).await;
                Ok(Booked::Done(b))
            }
        };
    }
    announce(state, &b, true).await;
    Ok(Booked::Done(b))
}

/// Public addresses of a booking's pages.
pub fn manage_url(state: &AppState, s: &Settings, b: &repo::Booking) -> String {
    vuuut_core::url::absolute(&state.site_url(), &vuuut_core::url::base(&s.paths.public_path(&format!("/book/manage/{}/", b.token))))
}

async fn start_payment(state: &AppState, service: &repo::Service, b: &repo::Booking) -> Option<String> {
    let cfg = crate::ecom::stripe(state)?;
    let s = settings(state);
    let manage = manage_url(state, &s, b);
    let success = vuuut_core::url::absolute(&state.site_url(), &vuuut_core::url::base(&s.paths.public_path(&format!("/book/paid/{}/", b.token)))) + "?session={CHECKOUT_SESSION_ID}";
    let label = format!("{} — {}", service.name, rules::day_label(b.start(), tz(state, &s)));
    match vuuut_stripe::create_shop_checkout_session(&cfg, &b.id, &label, b.price_cents, &s.currency, &b.email, &success, &manage).await {
        Ok((_, url)) => Some(url),
        Err(e) => {
            tracing::warn!("booking: Stripe checkout for {} failed: {}", b.r#ref, e.message());
            None
        }
    }
}

/// The visitor is back from Stripe: ask Stripe (never the browser) whether
/// it was paid, for this booking and this amount.
pub async fn settle_payment(state: &AppState, b: &repo::Booking, session: &str) -> bool {
    if b.payment_status == "paid" {
        return true;
    }
    let Some(cfg) = crate::ecom::stripe(state) else { return false };
    let Ok(v) = vuuut_stripe::get_checkout_session(&cfg, session).await else { return false };
    let ok = v["payment_status"].as_str() == Some("paid") && v["client_reference_id"].as_str() == Some(b.id.as_str()) && v["amount_total"].as_i64() == Some(b.price_cents);
    if !ok {
        return false;
    }
    let reference = v["payment_intent"].as_str().unwrap_or(session).to_string();
    let approval = repo::find_service(&state.db, &b.service_id).await.is_some_and(|s| s.approval != 0);
    // A hold that lapsed while paying is revived: they paid for it.
    let status = if approval { "pending" } else { "confirmed" };
    if repo::set_paid(&state.db, &b.id, &reference, status).await.is_err() {
        return false;
    }
    if let Some(b) = repo::find_booking(&state.db, &b.id).await {
        announce(state, &b, true).await;
    }
    true
}

// ---------------------------------------------------------------------------
// email
// ---------------------------------------------------------------------------

fn wrap(site: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html><body style="margin:0;background:#f4f2ee;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1b1f1d">
<div style="max-width:560px;margin:28px auto;background:#fff;border-radius:16px;padding:30px 30px 26px;border:1px solid #e7e2d9">
<p style="margin:0 0 20px;font-weight:700;font-size:17px;letter-spacing:-.01em">{site}</p>{body}</div>
<p style="text-align:center;color:#8a867d;font-size:12px;margin:0 0 28px">Sent by {site}</p></body></html>"#,
        site = esc(site)
    )
}

fn send(state: &AppState, to: &str, subject: String, html: String, text: String) {
    if to.is_empty() {
        return;
    }
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let cfg = vuuut_mail::MailConfig::from_settings(&state.settings, &site_title, &state.site_url());
    let to = to.to_string();
    tokio::spawn(async move {
        let msg = vuuut_mail::Message { to: to.clone(), subject, html, text };
        if let Err(e) = vuuut_mail::send(&cfg, &msg).await {
            tracing::warn!("booking: mail to {to} failed: {}", e.message());
        }
    });
}

/// The facts table every email carries.
async fn facts(state: &AppState, s: &Settings, b: &repo::Booking) -> (String, String, String) {
    let zone = tz(state, s);
    let svc = repo::find_service(&state.db, &b.service_id).await.unwrap_or_default();
    let who = match &b.staff_id {
        Some(id) => repo::find_staff(&state.db, id).await.map(|p| p.name).unwrap_or_default(),
        None => String::new(),
    };
    let mut rows = vec![
        ("When", format!("{}, {} – {} ({})", rules::day_label(b.start(), zone), rules::time_label(b.start(), zone), rules::time_label(b.end(), zone), zone.name())),
        ("What", if b.seats > 1 { format!("{} × {}", svc.name, b.seats) } else { svc.name.clone() }),
        ("Where", if svc.location_type == "video" && !svc.location.is_empty() { svc.location.clone() } else { svc.where_label(s) }),
    ];
    if !who.is_empty() {
        rows.push(("With", who));
    }
    if b.price_cents > 0 {
        rows.push(("Price", format!("{}{}", rules::format_price(b.price_cents, &s.currency), match b.payment_status.as_str() { "paid" => " · paid", "unpaid" if svc.payment == "onsite" => " · pay at the appointment", _ => "" })));
    }
    rows.push(("Reference", b.r#ref.clone()));
    let html = format!(
        r#"<table style="width:100%;border-collapse:collapse;margin:0 0 22px;font-size:15px">{}</table>"#,
        rows.iter().map(|(k, v)| format!(r#"<tr><td style="padding:9px 0;color:#77736b;width:96px;vertical-align:top;border-top:1px solid #efebe4">{k}</td><td style="padding:9px 0;border-top:1px solid #efebe4">{}</td></tr>"#, esc(v))).collect::<String>()
    );
    let text = rows.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
    (html, text, svc.name)
}

fn button(href: &str, label: &str) -> String {
    format!(r#"<p style="margin:0 0 18px"><a href="{}" style="display:inline-block;background:#1f4d3f;color:#fff;text-decoration:none;padding:12px 22px;border-radius:999px;font-weight:600">{}</a></p>"#, esc(href), esc(label))
}

/// After a booking is made or confirmed: the customer's email, and a notice
/// to the person booked and the business.
pub async fn announce(state: &AppState, b: &repo::Booking, staff_too: bool) {
    let s = settings(state);
    let site = state.settings.get_str("title").unwrap_or_default();
    let (table, text, svc) = facts(state, &s, b).await;
    let manage = manage_url(state, &s, b);
    let (head, lede) = match b.status.as_str() {
        "pending" => ("Request received", "Thanks — we'll confirm your booking shortly and email you again."),
        "awaiting_payment" => return,
        _ => ("You're booked", "Your booking is confirmed. Add it to your calendar, and use the button below if anything changes."),
    };
    let body = format!(
        r#"<h1 style="font-size:24px;margin:0 0 6px">{head}</h1><p style="color:#55524b;margin:0 0 20px">{lede}</p>{table}{btn}<p style="color:#77736b;font-size:13px;margin:0">{policy}</p>"#,
        btn = button(&manage, "View, reschedule or cancel"),
        policy = if s.cancel_hours > 0 { format!("You can change or cancel online until {} hours before.", s.cancel_hours) } else { "You can change or cancel online any time before.".into() },
    );
    send(state, &b.email, format!("{head}: {svc}"), wrap(&site, &body), format!("{head}\n\n{text}\n\nManage: {manage}"));
    if staff_too {
        let staff_email = match &b.staff_id {
            Some(id) => repo::find_staff(&state.db, id).await.map(|p| p.email).unwrap_or_default(),
            None => String::new(),
        };
        let admin = format!("{}/{}/booking/bookings/{}/", state.site_url().trim_end_matches('/'), state.admin_slug(), b.id);
        let body = format!(
            r#"<h1 style="font-size:22px;margin:0 0 6px">New booking{p}</h1><p style="color:#55524b;margin:0 0 20px">{n} · <a href="mailto:{e}">{e}</a>{ph}</p>{table}{notes}{btn}"#,
            p = if b.status == "pending" { " — needs approval" } else { "" },
            n = esc(&b.name),
            e = esc(&b.email),
            ph = if b.phone.is_empty() { String::new() } else { format!(" · {}", esc(&b.phone)) },
            notes = if b.notes.is_empty() { String::new() } else { format!(r#"<p style="background:#f6f3ee;border-radius:10px;padding:12px 14px;margin:0 0 18px">{}</p>"#, esc(&b.notes)) },
            btn = button(&admin, "Open in the admin"),
        );
        let subject = format!("New booking: {svc} — {}", b.name);
        let mut sent: Vec<String> = Vec::new();
        for to in [staff_email, s.notify_email.clone()] {
            if !to.is_empty() && !sent.contains(&to) {
                send(state, &to, subject.clone(), wrap(&site, &body), format!("New booking\n\n{}\n{}\n\n{text}\n\n{admin}", b.name, b.email));
                sent.push(to);
            }
        }
    }
}

pub async fn notify_cancelled(state: &AppState, b: &repo::Booking, by_customer: bool) {
    let s = settings(state);
    let site = state.settings.get_str("title").unwrap_or_default();
    let (table, text, svc) = facts(state, &s, b).await;
    let book_again = vuuut_core::url::absolute(&state.site_url(), &vuuut_core::url::base(&s.paths.public_path("/book/")));
    let body = format!(
        r#"<h1 style="font-size:24px;margin:0 0 6px">Booking cancelled</h1><p style="color:#55524b;margin:0 0 20px">{why}</p>{table}{btn}"#,
        why = if by_customer { "As you asked, this booking is cancelled.".to_string() } else if b.cancel_reason.is_empty() { "We're sorry — we've had to cancel this booking.".to_string() } else { format!("We're sorry — we've had to cancel this booking: {}", esc(&b.cancel_reason)) },
        btn = button(&book_again, "Book another time"),
    );
    send(state, &b.email, format!("Cancelled: {svc}"), wrap(&site, &body), format!("Booking cancelled\n\n{text}"));
    if by_customer {
        if let Some(p) = match &b.staff_id { Some(id) => repo::find_staff(&state.db, id).await, None => None } {
            send(state, &p.email, format!("Cancelled by {}: {svc}", b.name), wrap(&site, &format!("<h1 style=\"font-size:22px;margin:0 0 16px\">Cancelled by the customer</h1>{table}")), text.clone());
        }
        if !s.notify_email.is_empty() {
            send(state, &s.notify_email, format!("Cancelled by {}: {svc}", b.name), wrap(&site, &format!("<h1 style=\"font-size:22px;margin:0 0 16px\">Cancelled by the customer</h1>{table}")), text);
        }
    }
}

pub async fn notify_moved(state: &AppState, b: &repo::Booking) {
    let s = settings(state);
    let site = state.settings.get_str("title").unwrap_or_default();
    let (table, text, svc) = facts(state, &s, b).await;
    let manage = manage_url(state, &s, b);
    let body = format!(r#"<h1 style="font-size:24px;margin:0 0 6px">New time confirmed</h1><p style="color:#55524b;margin:0 0 20px">Your booking has moved. Here are the new details.</p>{table}{}"#, button(&manage, "View your booking"));
    send(state, &b.email, format!("Rescheduled: {svc}"), wrap(&site, &body), format!("New time\n\n{text}\n\n{manage}"));
}

async fn remind(state: &AppState, b: &repo::Booking) {
    let s = settings(state);
    let site = state.settings.get_str("title").unwrap_or_default();
    let (table, text, svc) = facts(state, &s, b).await;
    let manage = manage_url(state, &s, b);
    let body = format!(r#"<h1 style="font-size:24px;margin:0 0 6px">See you soon</h1><p style="color:#55524b;margin:0 0 20px">A reminder of your upcoming booking.</p>{table}{}"#, button(&manage, "View or change"));
    send(state, &b.email, format!("Reminder: {svc}"), wrap(&site, &body), format!("Reminder\n\n{text}\n\n{manage}"));
    repo::mark_reminded(&state.db, &b.id).await;
}

/// Once a minute, for the life of the process: release unpaid holds and
/// send the reminders that have come due.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            let s = settings(&state);
            if s.enabled {
                let released = repo::expire_holds(&state.db).await;
                if released > 0 {
                    tracing::info!("booking: released {released} unpaid hold(s)");
                }
                if s.reminder_hours > 0 {
                    for b in repo::due_reminders(&state.db, s.reminder_hours).await {
                        remind(&state, &b).await;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
