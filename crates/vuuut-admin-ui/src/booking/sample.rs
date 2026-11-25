//! "Load a sample business": a wellness studio with massage, skin care,
//! group classes and a video consultation, four people with their own
//! weekly hours, and two weeks of bookings either side of today — so the
//! Overview, the Calendar and a theme can be seen working at once. The
//! customers are `@example.com` addresses and are never emailed.

use super::*;
use vuuut_core::dummies::{document, Block, Block::*};

const IMG: &str = "https://images.unsplash.com/photo-";
const ADDRESS: &str = "14 Willow Lane, Bath BA1 2QT";

struct Svc {
    name: &'static str,
    cat: usize,
    image: &'static str,
    tint: &'static str,
    minutes: i64,
    after: i64,
    price: i64,
    payment: &'static str,
    capacity: i64,
    location: &'static str,
    summary: &'static str,
    featured: bool,
    staff: &'static [usize],
    body: &'static [Block],
}

const CATS: &[(&str, &str, &str)] = &[
    ("Massage", "teal", "Hands-on treatments to release tension and restore."),
    ("Skin", "pink", "Facials and skin rituals with clean, gentle products."),
    ("Classes", "amber", "Small-group movement sessions — six to ten people."),
    ("Consultations", "purple", "One-to-one guidance, in the studio or by video."),
];

const STAFF: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("Maya Chen", "Senior massage therapist", "1573496359142-b8d87734a5a2", "teal", r#"{"mon":"09:00-17:00","tue":"09:00-17:00","wed":"12:00-20:00","thu":"09:00-17:00","fri":"09:00-15:00","sat":"closed","sun":"closed"}"#,
        "Maya has practised massage for twelve years, specialising in deep tissue and hot stone work for people who sit at desks or run long distances."),
    ("Sofia Reyes", "Skin therapist", "1494790108377-be9c29b29330", "pink", r#"{"mon":"closed","tue":"10:00-18:00","wed":"10:00-18:00","thu":"10:00-19:00","fri":"10:00-18:00","sat":"09:00-14:00","sun":"closed"}"#,
        "Sofia trained in clinical aesthetics and keeps her facials calm, unhurried and tailored to what your skin needs that day."),
    ("Daniel Okoro", "Movement coach", "1500648767791-00dcc994a43e", "amber", r#"{"mon":"07:00-10:00,17:00-20:00","tue":"07:00-10:00","wed":"07:00-10:00,17:00-20:00","thu":"07:00-10:00","fri":"07:00-10:00","sat":"09:00-12:00","sun":"closed"}"#,
        "Daniel teaches Pilates and barre with an eye for alignment, so every class works whether it is your first or your fiftieth."),
    ("Ella Morgan", "Nutritionist", "1438761681033-6461ffad8d80", "purple", r#"{"mon":"10:00-16:00","tue":"closed","wed":"10:00-16:00","thu":"closed","fri":"10:00-16:00","sat":"closed","sun":"closed"}"#,
        "Ella is a registered nutritionist who helps people build simple, lasting habits — no fads, no food guilt."),
];

const SERVICES: &[Svc] = &[
    Svc { name: "Signature Swedish Massage", cat: 0, image: "1544161515-4ab6ce6db874", tint: "teal", minutes: 60, after: 15, price: 8500, payment: "onsite", capacity: 1, location: "", featured: true, staff: &[0],
        summary: "Long, flowing strokes to ease tension and quieten a busy mind.",
        body: &[P("Our most-loved treatment. Warm oil and long, rhythmic strokes work through the back, shoulders, legs and feet, easing everyday tension and leaving you lighter."), H("Good to know"), L(&["Arrive ten minutes early to settle in.", "Tell us about injuries or areas to avoid.", "Showers and robes are provided."])] },
    Svc { name: "Deep Tissue Massage", cat: 0, image: "1519823551278-64ac92734fb1", tint: "teal", minutes: 90, after: 15, price: 12000, payment: "online", capacity: 1, location: "", featured: false, staff: &[0],
        summary: "Slow, firm pressure for stubborn knots, desk shoulders and tired legs.",
        body: &[P("Focused work on the deeper layers of muscle. Ideal after training, or when stiffness has built up for weeks. Your therapist checks in on pressure throughout.")] },
    Svc { name: "Hot Stone Ritual", cat: 0, image: "1600334089648-b0d9d3028eb2", tint: "teal", minutes: 75, after: 15, price: 11000, payment: "onsite", capacity: 1, location: "", featured: true, staff: &[0],
        summary: "Smooth basalt stones melt tension as warmth sinks deep into the muscles.",
        body: &[P("Heated stones are placed along the spine and used as an extension of the therapist's hands. Deeply relaxing, especially in the colder months.")] },
    Svc { name: "Glow Facial", cat: 1, image: "1570172619644-dfd03ed5d881", tint: "pink", minutes: 60, after: 10, price: 9500, payment: "onsite", capacity: 1, location: "", featured: true, staff: &[1],
        summary: "Cleanse, exfoliate, mask and massage for skin that looks rested.",
        body: &[P("A thorough, gentle facial matched to your skin on the day: double cleanse, enzyme exfoliation, a nourishing mask and a lymphatic face massage."), L(&["Suitable for all skin types.", "No make-up needed afterwards — though you can."])] },
    Svc { name: "Hydrating Facial", cat: 1, image: "1552693673-1bf958298935", tint: "pink", minutes: 45, after: 10, price: 7000, payment: "onsite", capacity: 1, location: "", featured: false, staff: &[1],
        summary: "A quick, deeply moisturising lift for dry or tired skin.",
        body: &[P("Layered hydration with hyaluronic serums and a cooling sheet mask. The perfect lunchtime refresh.")] },
    Svc { name: "Mat Pilates Class", cat: 2, image: "1518611012118-696072aa579a", tint: "amber", minutes: 50, after: 10, price: 2400, payment: "online", capacity: 10, location: "Studio 2", featured: true, staff: &[2],
        summary: "Core strength, posture and control in a friendly group of up to ten.",
        body: &[P("A balanced class for every level, with options to make each exercise easier or harder. Mats and props are provided — just bring water.")] },
    Svc { name: "Barre Sculpt Class", cat: 2, image: "1518310383802-640c2de311b2", tint: "amber", minutes: 45, after: 15, price: 2200, payment: "online", capacity: 8, location: "Studio 2", featured: false, staff: &[2],
        summary: "Small movements, big burn — ballet-inspired strength to music.",
        body: &[P("Low-impact, high-energy toning at the barre. Expect pulses, holds and a lot of smiling through the shake.")] },
    Svc { name: "Nutrition Consultation", cat: 3, image: "1512621776951-a57141f2eefd", tint: "purple", minutes: 45, after: 15, price: 0, payment: "none", capacity: 1, location: "", featured: false, staff: &[3],
        summary: "A free first conversation about your goals and everyday eating.",
        body: &[P("By video, from wherever you are. We'll talk about your routine, what you enjoy eating and what you'd like to change, and agree two or three simple next steps.")] },
];

const CUSTOMERS: &[&str] = &["Amara Okafor", "Lucas Bernard", "Priya Raman", "Tomás Ortega", "Hannah Weiss", "Kenji Watanabe", "Sofia Lindqvist", "Daniel Mensah", "Leila Haddad", "Ethan Clarke", "Chloe Martin", "Noah Fischer"];

fn html_of(blocks: &[Block]) -> (String, String) {
    let lexical = document(blocks.iter().map(|b| b.node()).collect());
    let html = vuuut_render::lexical::to_html(&lexical);
    (lexical, html)
}

pub(crate) async fn sample_load(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    match seed_sample_business(&state.db, &state.settings).await {
        Ok(n) => crate::tools::tool_saved(&state, BASE, &format!("Sample business loaded: {n} services, ready to edit. Online booking is switched on.")),
        Err(e) => {
            tracing::error!("booking: sample failed: {}", e.message());
            flash_to(BASE, "error", "The sample business could not be loaded.")
        }
    }
}

/// Also the `vuuut seed-booking` command. Services whose address is taken
/// are skipped, so running it twice adds nothing.
pub async fn seed_sample_business(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache) -> vuuut_core::Result<usize> {
    if !repo::services(db, false).await.is_empty() {
        return Ok(0);
    }
    Ok(load_business(db, settings, usize::MAX).await?.services.len())
}

/// What one load created — only rows that did not exist before — so Tools →
/// Dummy content can record them for removal.
#[derive(Default)]
pub struct Loaded {
    pub categories: Vec<String>,
    pub staff: Vec<String>,
    pub services: Vec<String>,
}

/// Up to `limit` sample services with their categories, team and bookings.
/// Anything whose address is already taken is reused or skipped. Switches
/// booking on when it made any services.
pub async fn load_business(db: &vuuut_db::Db, settings: &vuuut_db::SettingsCache, limit: usize) -> vuuut_core::Result<Loaded> {
    let mut out = Loaded::default();
    let mut cat_ids = Vec::new();
    for (i, (name, tint, desc)) in CATS.iter().enumerate() {
        let slug = vuuut_core::directory::slugify(name);
        let id = match repo::category_by_slug(db, &slug).await {
            Some(c) => c.id,
            None => {
                let id = repo::save_category(db, None, name, &slug, desc, tint, i as i64).await?;
                out.categories.push(id.clone());
                id
            }
        };
        cat_ids.push(id);
    }
    let mut staff_ids = Vec::new();
    for (i, (name, title, photo, tint, hours, bio)) in STAFF.iter().enumerate() {
        if let Some(existing) = repo::staff_by_slug(db, &vuuut_core::directory::slugify(name)).await {
            staff_ids.push(existing.id);
            continue;
        }
        let input = rules::StaffInput {
            name: name.to_string(),
            slug: vuuut_core::directory::slugify(name),
            title: title.to_string(),
            email: String::new(),
            phone: String::new(),
            bio: bio.to_string(),
            photo: format!("{IMG}{photo}?w=800&q=80"),
            tint: tint.to_string(),
            hours: vuuut_core::directory::Hours::from_json(hours),
            status: "active".into(),
            sort_order: i as i64,
            service_ids: Vec::new(),
        };
        let id = repo::save_staff(db, None, &input).await?;
        out.staff.push(id.clone());
        staff_ids.push(id);
    }
    let mut made = Vec::new();
    for (i, v) in SERVICES.iter().enumerate().take(limit) {
        if repo::service_by_slug(db, &vuuut_core::directory::slugify(v.name)).await.is_some() {
            continue;
        }
        let (lexical, html) = html_of(v.body);
        let input = rules::ServiceInput {
            name: v.name.into(),
            slug: vuuut_core::directory::slugify(v.name),
            category_id: cat_ids.get(v.cat).cloned(),
            summary: v.summary.into(),
            description: vuuut_render::markdown::html_to_plaintext(&html),
            description_lexical: lexical,
            description_html: html,
            image: format!("{IMG}{}?w=1600&q=80", v.image),
            tint: v.tint.into(),
            duration_minutes: v.minutes,
            buffer_before: 0,
            buffer_after: v.after,
            slot_interval: if v.capacity > 1 { 60 } else { 15 },
            capacity: v.capacity,
            price_cents: v.price,
            payment: v.payment.into(),
            location_type: if v.cat == 3 { "video".into() } else { "in_person".into() },
            location: v.location.into(),
            staff_mode: if v.cat == 2 { "any".into() } else { "choose".into() },
            approval: false,
            min_notice: 120,
            max_advance: 60,
            external_url: String::new(),
            questions: if v.cat == 0 || v.cat == 1 {
                vec![rules::Question { label: "Anything we should know — injuries, allergies, pregnancy?".into(), kind: "long".into(), required: false }]
            } else if v.cat == 3 {
                vec![rules::Question { label: "What would you most like help with?".into(), kind: "long".into(), required: true }]
            } else {
                Vec::new()
            },
            status: "active".into(),
            featured: v.featured,
            seo_title: String::new(),
            seo_description: v.summary.into(),
            sort_order: i as i64,
            staff_ids: v.staff.iter().filter_map(|s| staff_ids.get(*s).cloned()).collect(),
        };
        let id = repo::save_service(db, None, &input).await?;
        out.services.push(id.clone());
        made.push((id, v));
    }
    // Two weeks of bookings either side of today, in the site's time zone.
    let site_tz = settings.get_str("timezone").unwrap_or_default();
    let s = Settings::read(|k| settings.get_str(k));
    let tz = s.tz(&site_tz);
    let today = Utc::now().with_timezone(&tz).date_naive();
    if made.is_empty() {
        return Ok(out);
    }
    let mut n = 0usize;
    for day in -10..=10i64 {
        let date = today + Duration::days(day);
        let wd = date.weekday().num_days_from_monday() as usize;
        for (k, (sid, v)) in made.iter().enumerate() {
            if (day + k as i64) % 3 != 0 {
                continue;
            }
            let Some(&who) = v.staff.first() else { continue };
            let Some(svc) = repo::find_service(db, sid).await else { continue };
            let person = vuuut_core::directory::Hours::from_json(STAFF[who].4);
            let start_min = match &person.0[wd] {
                vuuut_core::directory::Day::Open(r) => r[0].0 as i64 + ((k as i64 % 3) * 60),
                _ => continue,
            };
            let Some(start) = rules::local(tz, date, start_min) else { continue };
            let name = CUSTOMERS[(n * 5 + k) % CUSTOMERS.len()];
            let email = format!("{}@example.com", vuuut_core::directory::slugify(name).replace('-', "."));
            let seats = if v.capacity > 1 { 1 + (n as i64 % 3) } else { 1 };
            let input = rules::BookingInput { name: name.into(), email, phone: String::new(), notes: String::new(), answers: Vec::new(), seats };
            let status = if day < 0 { if n % 9 == 0 { "no_show" } else if n % 7 == 0 { "cancelled" } else { "completed" } } else if v.payment == "online" { "confirmed" } else if n % 6 == 0 { "pending" } else { "confirmed" };
            let pay = if svc.price_cents == 0 { "none" } else if v.payment == "online" || day < 0 { "paid" } else { "unpaid" };
            let b = repo::insert_booking(db, &repo::NewBooking { service: &svc, staff_id: staff_ids.get(who).map(String::as_str), start, input: &input, status, payment_status: pay, timezone: tz.name(), member_id: None, source: "site" }).await?;
            // Back-date when it was made, for the Overview's chart.
            let made_at = repo::db_time(start - Duration::days(3 + (n as i64 % 9)));
            let _ = sqlx::query("UPDATE booking_bookings SET created_at = ?, reminded_at = ? WHERE id = ?").bind(&made_at).bind(&made_at).bind(&b.id).execute(db).await;
            n += 1;
        }
    }
    settings.set_many(db, &[
        (rules::SETTING_ENABLED, serde_json::json!(true)),
        (rules::SETTING_TITLE, serde_json::json!("Book a treatment or class")),
        (rules::SETTING_INTRO, serde_json::json!("Choose what you'd like, pick a time that suits you, and you'll have a confirmation in your inbox in seconds.")),
        (rules::SETTING_ADDRESS, serde_json::json!(ADDRESS)),
    ]).await?;
    Ok(out)
}
