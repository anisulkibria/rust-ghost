//! Booking & Appointment Scheduling: the rules.
//!
//! What the popular schedulers share — Calendly, Acuity, Cal.com, Bookly,
//! Amelia, SimplyBook, Square Appointments — in one tool: services with a
//! length, buffers and a price; the people (or rooms, courts, chairs) who
//! are booked and their weekly hours; time off; one-to-one appointments and
//! group sessions with seats; "anyone available" or a chosen person;
//! approval; payment online, at the visit, or not at all; custom intake
//! questions; and a hand-off to an external scheduler for sites that keep
//! one.
//!
//! Pure. The heart is [`slots_for_day`]: given a date in the business's time
//! zone, the service's rules, each person's hours, time off and bookings, it
//! answers which start times are free — honouring buffers, notice, how far
//! ahead booking is open, DST, and seats. Everything a visitor types is
//! rebuilt by [`ServiceInput`], [`StaffInput`] or [`BookingInput`].

use crate::directory::{clean, clean_text, is_email, safe_image, safe_url, slugify, Day, Hours};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

macro_rules! keyed {
    ($name:ident { $($variant:ident => $key:literal, $label:literal;)+ } default $def:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn key(self) -> &'static str { match self { $($name::$variant => $key),+ } }
            pub fn label(self) -> &'static str { match self { $($name::$variant => $label),+ } }
            pub fn from_key(k: &str) -> $name { Self::ALL.iter().copied().find(|x| x.key() == k.trim()).unwrap_or($name::$def) }
        }
    };
}

keyed!(Payment {
    None => "none", "Free, or settled later";
    Onsite => "onsite", "Pay at the appointment";
    Online => "online", "Pay online when booking";
} default None);

keyed!(Location {
    InPerson => "in_person", "In person";
    Video => "video", "Video call";
    Phone => "phone", "Phone call";
    Customer => "customer", "At the customer's address";
} default InPerson);

keyed!(StaffMode {
    Any => "any", "First available person";
    Choose => "choose", "Customer chooses who";
} default Any);

keyed!(ServiceStatus {
    Active => "active", "Bookable";
    Hidden => "hidden", "Hidden — bookable by direct link";
    Draft => "draft", "Draft";
} default Draft);

keyed!(Status {
    AwaitingPayment => "awaiting_payment", "Awaiting payment";
    Pending => "pending", "Waiting for approval";
    Confirmed => "confirmed", "Confirmed";
    Completed => "completed", "Completed";
    Cancelled => "cancelled", "Cancelled";
    NoShow => "no_show", "No-show";
} default Pending);

impl Status {
    /// Takes up the time: nobody else can have it.
    pub fn holds_time(self) -> bool {
        matches!(self, Status::AwaitingPayment | Status::Pending | Status::Confirmed | Status::Completed)
    }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "booking_enabled";
pub const SETTING_TITLE: &str = "booking_title";
pub const SETTING_INTRO: &str = "booking_intro";
pub const SETTING_BASE_PATH: &str = "booking_base_path";
pub const SETTING_TIMEZONE: &str = "booking_timezone";
pub const SETTING_HOURS: &str = "booking_hours";
pub const SETTING_CANCEL_HOURS: &str = "booking_cancel_hours";
pub const SETTING_REMINDER_HOURS: &str = "booking_reminder_hours";
pub const SETTING_NOTIFY_EMAIL: &str = "booking_notify_email";
pub const SETTING_CURRENCY: &str = "booking_currency";
pub const SETTING_REQUIRE_PHONE: &str = "booking_require_phone";
pub const SETTING_TERMS: &str = "booking_terms";
pub const SETTING_ADDRESS: &str = "booking_address";

pub const DEFAULT_BASE: &str = "book";
pub const DEFAULT_TITLE: &str = "Book an appointment";
/// The booking pages' own addresses; never a service's slug.
pub const RESERVED_SLUGS: &[&str] = &["assets", "category", "staff", "manage", "paid"];
/// Unpaid online bookings hold their slot this long.
pub const PAYMENT_HOLD_MINUTES: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub base: String,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { base: DEFAULT_BASE.into() }
    }
}

impl Paths {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        Self::new(&get(SETTING_BASE_PATH).unwrap_or_default()).unwrap_or_default()
    }
    pub fn new(base: &str) -> Option<Self> {
        let b = base.trim().trim_matches('"').trim_matches('/');
        if b.is_empty() {
            return Some(Self::default());
        }
        let clean = crate::ecom::clean_address(b)?;
        if matches!(clean.split('/').next().unwrap_or_default(), "shop" | "directory" | "courses") {
            return None;
        }
        Some(Paths { base: clean })
    }
    pub fn public_path(&self, internal: &str) -> String {
        if self.base == DEFAULT_BASE {
            return internal.into();
        }
        match internal.strip_prefix("/book/") {
            Some(tail) => format!("/{}/{tail}", self.base),
            None if internal == "/book" => format!("/{}", self.base),
            None => internal.into(),
        }
    }
    pub fn internal_path(&self, public: &str) -> Option<String> {
        if self.base == DEFAULT_BASE {
            return None;
        }
        let rest = public.strip_prefix('/')?.strip_prefix(self.base.as_str())?;
        if rest.is_empty() {
            return Some("/book".into());
        }
        rest.strip_prefix('/').map(|t| format!("/book/{t}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub title: String,
    pub intro: String,
    /// IANA zone the business keeps its hours in; empty = the site's.
    pub timezone: String,
    /// Opening hours used for any day a person has no hours of their own.
    pub hours: Hours,
    /// Customers can cancel or move a booking until this many hours before.
    pub cancel_hours: i64,
    /// Reminder email this many hours before; 0 = none.
    pub reminder_hours: i64,
    /// Where new-booking notices go (besides the person booked).
    pub notify_email: String,
    pub currency: String,
    pub require_phone: bool,
    /// Shown above the confirm button: policies, cancellation terms.
    pub terms: String,
    /// The business address, for in-person services with none of their own.
    pub address: String,
    pub paths: Paths,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let raw = |k: &str| get(k).map(|v| v.trim().trim_matches('"').to_string());
        let flag = |k: &str, d: bool| match raw(k).as_deref() {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => d,
        };
        let num = |k: &str, d: i64, lo: i64, hi: i64| raw(k).and_then(|v| v.parse::<i64>().ok()).map(|n| n.clamp(lo, hi)).unwrap_or(d);
        let title = clean(&raw(SETTING_TITLE).unwrap_or_default(), 80);
        let hours = get(SETTING_HOURS).map(|h| Hours::from_json(&h)).filter(|h| !h.is_empty()).unwrap_or_else(default_hours);
        let currency = raw(SETTING_CURRENCY).map(|c| c.to_uppercase()).filter(|c| crate::directory::CURRENCIES.contains(&c.as_str())).unwrap_or_else(|| "USD".into());
        let tz = raw(SETTING_TIMEZONE).unwrap_or_default();
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
            intro: clean(&raw(SETTING_INTRO).unwrap_or_default(), 400),
            timezone: if tz.parse::<chrono_tz::Tz>().is_ok() { tz } else { String::new() },
            hours,
            cancel_hours: num(SETTING_CANCEL_HOURS, 24, 0, 24 * 30),
            reminder_hours: num(SETTING_REMINDER_HOURS, 24, 0, 24 * 7),
            notify_email: raw(SETTING_NOTIFY_EMAIL).filter(|e| is_email(e)).unwrap_or_default(),
            currency,
            require_phone: flag(SETTING_REQUIRE_PHONE, false),
            terms: clean_text(&raw(SETTING_TERMS).unwrap_or_default(), 2000),
            address: clean(&raw(SETTING_ADDRESS).unwrap_or_default(), 300),
            paths: Paths::read(get),
        }
    }

    /// The business's zone: its own, else the site's, else UTC.
    pub fn tz(&self, site_timezone: &str) -> chrono_tz::Tz {
        self.timezone.parse().or_else(|_| site_timezone.trim().trim_matches('"').parse()).unwrap_or(chrono_tz::UTC)
    }
}

pub fn default_hours() -> Hours {
    Hours::from_json(r#"{"mon":"09:00-17:00","tue":"09:00-17:00","wed":"09:00-17:00","thu":"09:00-17:00","fri":"09:00-17:00","sat":"closed","sun":"closed"}"#)
}

pub fn format_price(cents: i64, currency: &str) -> String {
    if cents <= 0 {
        return "Free".into();
    }
    crate::directory::format_price(cents, currency, crate::directory::PriceType::Fixed).unwrap_or_default()
}

/// `45` → `45 min`, `90` → `1 hr 30 min`, `120` → `2 hr`.
pub fn duration_label(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

// ---------------------------------------------------------------------------
// input
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub label: String,
    /// `text` | `long` | `yesno`
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
}

pub fn questions_from_json(raw: &str) -> Vec<Question> {
    let v: Vec<Question> = serde_json::from_str(raw).unwrap_or_default();
    v.into_iter()
        .filter_map(|q| {
            let label = clean(&q.label, 160);
            (!label.is_empty()).then(|| Question { label, kind: if matches!(q.kind.as_str(), "long" | "yesno") { q.kind } else { "text".into() }, required: q.required })
        })
        .take(12)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServiceInput {
    pub name: String,
    pub slug: String,
    pub category_id: Option<String>,
    pub summary: String,
    pub description: String,
    pub description_lexical: String,
    pub description_html: String,
    pub image: String,
    pub tint: String,
    pub duration_minutes: i64,
    pub buffer_before: i64,
    pub buffer_after: i64,
    pub slot_interval: i64,
    pub capacity: i64,
    pub price_cents: i64,
    pub payment: String,
    pub location_type: String,
    pub location: String,
    pub staff_mode: String,
    pub approval: bool,
    pub min_notice: i64,
    pub max_advance: i64,
    pub external_url: String,
    pub questions: Vec<Question>,
    pub status: String,
    pub featured: bool,
    pub seo_title: String,
    pub seo_description: String,
    pub sort_order: i64,
    pub staff_ids: Vec<String>,
}

pub const TINTS: [&str; 8] = ["blue", "teal", "green", "amber", "purple", "pink", "red", "slate"];

impl ServiceInput {
    pub fn from_form(pairs: &[(String, String)], exists: impl Fn(&str, &str) -> bool) -> Result<ServiceInput, String> {
        let form: HashMap<&str, &str> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let get = |k: &str| form.get(k).copied().unwrap_or("");
        let num = |k: &str, d: i64, lo: i64, hi: i64| get(k).trim().parse::<i64>().ok().map(|n| n.clamp(lo, hi)).unwrap_or(d);
        let name = clean(get("name"), 160);
        if name.chars().count() < 2 {
            return Err("Give the service a name.".into());
        }
        let price_cents = crate::directory::parse_price(get("price")).ok_or("The price must be a number, like 45 or 19.50.")?;
        let mut payment = Payment::from_key(get("payment"));
        if price_cents == 0 {
            payment = Payment::None;
        }
        let external_url = safe_url(get("external_url")).ok_or("The external booking link must be an http(s) address.")?;
        let location_type = Location::from_key(get("location_type"));
        let location = clean(get("location"), 300);
        if location_type == Location::Video && !location.is_empty() && safe_url(&location).filter(|u| !u.is_empty()).is_none() {
            return Err("The video link must be an http(s) address, or leave it empty to send it later.".into());
        }
        let duration = num("duration_minutes", 60, 5, 24 * 60);
        let slug = slugify(get("slug"));
        let tint = get("tint");
        Ok(ServiceInput {
            slug: if slug.is_empty() { slugify(&name) } else { slug },
            name,
            category_id: Some(get("category_id").trim().to_string()).filter(|c| !c.is_empty() && exists("category", c)),
            summary: clean(get("summary"), 240),
            description: clean_text(get("description"), 20_000),
            description_lexical: String::new(),
            description_html: String::new(),
            image: safe_image(get("image")).ok_or("The image must be an image address.")?,
            tint: if TINTS.contains(&tint) { tint.into() } else { "blue".into() },
            duration_minutes: duration,
            buffer_before: num("buffer_before", 0, 0, 240),
            buffer_after: num("buffer_after", 0, 0, 240),
            slot_interval: num("slot_interval", 0, 0, 240),
            capacity: num("capacity", 1, 1, 1000),
            price_cents,
            payment: payment.key().into(),
            location_type: location_type.key().into(),
            location,
            staff_mode: StaffMode::from_key(get("staff_mode")).key().into(),
            approval: get("approval") == "1",
            min_notice: num("min_notice", 120, 0, 60 * 24 * 60),
            max_advance: num("max_advance", 60, 1, 730),
            external_url,
            questions: questions_from_json(get("questions")),
            status: ServiceStatus::from_key(get("status")).key().into(),
            featured: get("featured") == "1",
            seo_title: clean(get("seo_title"), 70),
            seo_description: clean(get("seo_description"), 320),
            sort_order: num("sort_order", 0, 0, 999),
            staff_ids: pairs.iter().filter(|(k, _)| k == "staff").map(|(_, v)| v.trim().to_string()).filter(|v| exists("staff", v)).take(100).collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StaffInput {
    pub name: String,
    pub slug: String,
    pub title: String,
    pub email: String,
    pub phone: String,
    pub bio: String,
    pub photo: String,
    pub tint: String,
    pub hours: Hours,
    pub status: String,
    pub sort_order: i64,
    pub service_ids: Vec<String>,
}

impl StaffInput {
    pub fn from_form(pairs: &[(String, String)], service_exists: impl Fn(&str) -> bool) -> Result<StaffInput, String> {
        let form: HashMap<String, String> = pairs.iter().cloned().collect();
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let name = clean(get("name"), 120);
        if name.chars().count() < 2 {
            return Err("Give this person (or resource) a name.".into());
        }
        let email = clean(get("email"), 191).to_lowercase();
        if !email.is_empty() && !is_email(&email) {
            return Err("That email address does not look right.".into());
        }
        let phone = crate::directory::clean_phone(get("phone")).ok_or("That phone number does not look right.")?;
        let slug = slugify(get("slug"));
        let tint = get("tint");
        Ok(StaffInput {
            slug: if slug.is_empty() { slugify(&name) } else { slug },
            name,
            title: clean(get("title"), 120),
            email,
            phone,
            bio: clean_text(get("bio"), 4000),
            photo: safe_image(get("photo")).ok_or("The photo must be an image address.")?,
            tint: if TINTS.contains(&tint) { tint.into() } else { "blue".into() },
            hours: Hours::from_form(|k| form.get(k).cloned().unwrap_or_default()),
            status: if get("status") == "inactive" { "inactive".into() } else { "active".into() },
            sort_order: get("sort_order").trim().parse::<i64>().ok().map(|n| n.clamp(0, 999)).unwrap_or(0),
            service_ids: pairs.iter().filter(|(k, _)| k == "service").map(|(_, v)| v.trim().to_string()).filter(|v| service_exists(v)).take(200).collect(),
        })
    }
}

/// The customer's details from the booking form.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookingInput {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub notes: String,
    pub answers: Vec<(String, String)>,
    pub seats: i64,
}

impl BookingInput {
    pub fn from_form(form: &HashMap<String, String>, questions: &[Question], require_phone: bool, max_seats: i64) -> Result<BookingInput, String> {
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let name = clean(get("name"), 120);
        if name.chars().count() < 2 {
            return Err("Please add your name.".into());
        }
        let email = clean(get("email"), 191).to_lowercase();
        if !is_email(&email) {
            return Err("Please add a valid email address — your confirmation goes there.".into());
        }
        let phone = crate::directory::clean_phone(get("phone")).ok_or("That phone number does not look right.")?;
        if require_phone && phone.is_empty() {
            return Err("Please add a phone number.".into());
        }
        let mut answers = Vec::new();
        for (i, q) in questions.iter().enumerate() {
            let v = clean_text(get(&format!("q{i}")), 2000);
            if q.required && v.trim().is_empty() {
                return Err(format!("Please answer “{}”.", q.label));
            }
            if !v.is_empty() {
                answers.push((q.label.clone(), v));
            }
        }
        let seats = get("seats").trim().parse::<i64>().ok().unwrap_or(1).clamp(1, max_seats.clamp(1, 20));
        Ok(BookingInput { name, email, phone, notes: clean_text(get("notes"), 2000), answers, seats })
    }
}

// ---------------------------------------------------------------------------
// availability
// ---------------------------------------------------------------------------

/// What a service asks of the calendar.
#[derive(Debug, Clone)]
pub struct Rule {
    pub service_id: String,
    pub duration: i64,
    pub buffer_before: i64,
    pub buffer_after: i64,
    /// Minutes between start times (0 = the duration).
    pub interval: i64,
    pub capacity: i64,
    pub min_notice: i64,
    pub max_advance_days: i64,
}

/// Time somebody is already taken: a booking (with its service's buffers
/// already added) or time off.
#[derive(Debug, Clone)]
pub struct Busy {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The booking's service and exact start, for group seats; empty for time off.
    pub service_id: String,
    pub booked_start: Option<DateTime<Utc>>,
    pub seats: i64,
}

#[derive(Debug, Clone)]
pub struct Person {
    pub id: String,
    pub hours: Hours,
    pub busy: Vec<Busy>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Who is free then, in the order given.
    pub staff: Vec<String>,
    /// For a group service, the seats left (with the freest person).
    pub seats_left: i64,
}

fn ranges(day: &Day) -> Vec<(i64, i64)> {
    match day {
        Day::AllDay => vec![(0, 24 * 60)],
        Day::Open(r) => r.iter().map(|(a, b)| { let (a, b) = (*a as i64, *b as i64); (a, if b <= a { 24 * 60 } else { b }) }).collect(),
        _ => Vec::new(),
    }
}

/// A local wall-clock time on `date`; in a DST gap, none.
pub fn local(tz: chrono_tz::Tz, date: NaiveDate, minutes: i64) -> Option<DateTime<Utc>> {
    let (d, m) = if minutes >= 24 * 60 { (date.succ_opt()?, minutes - 24 * 60) } else { (date, minutes) };
    let t = NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0)?;
    tz.from_local_datetime(&d.and_time(t)).earliest().map(|x| x.with_timezone(&Utc))
}

/// Every free start time on `date` (the business's local date).
pub fn slots_for_day(date: NaiveDate, tz: chrono_tz::Tz, now: DateTime<Utc>, rule: &Rule, business: &Hours, people: &[Person]) -> Vec<Slot> {
    let wd = date.weekday().num_days_from_monday() as usize;
    let step = if rule.interval > 0 { rule.interval } else { rule.duration }.max(5);
    let earliest = now + Duration::minutes(rule.min_notice);
    let latest = now + Duration::days(rule.max_advance_days);
    let mut out: Vec<Slot> = Vec::new();
    for p in people {
        let day = if p.hours.0[wd] == Day::Unknown { &business.0[wd] } else { &p.hours.0[wd] };
        for (open, close) in ranges(day) {
            let mut t = open;
            while t + rule.duration <= close {
                let (Some(start), Some(end)) = (local(tz, date, t), local(tz, date, t + rule.duration)) else {
                    t += step;
                    continue;
                };
                t += step;
                if start < earliest || start > latest {
                    continue;
                }
                let (ws, we) = (start - Duration::minutes(rule.buffer_before), end + Duration::minutes(rule.buffer_after));
                let mut seats_left = rule.capacity;
                let mut free = true;
                for b in &p.busy {
                    if !(b.start < we && ws < b.end) {
                        continue;
                    }
                    if rule.capacity > 1 && b.service_id == rule.service_id && b.booked_start == Some(start) {
                        seats_left -= b.seats;
                    } else {
                        free = false;
                        break;
                    }
                }
                if !free || seats_left <= 0 {
                    continue;
                }
                match out.iter_mut().find(|s| s.start == start) {
                    Some(s) => {
                        s.staff.push(p.id.clone());
                        s.seats_left = s.seats_left.max(seats_left);
                    }
                    None => out.push(Slot { start, end, staff: vec![p.id.clone()], seats_left }),
                }
            }
        }
    }
    out.sort_by_key(|s| s.start);
    out
}

// ---------------------------------------------------------------------------
// references, calendars
// ---------------------------------------------------------------------------

/// `BK-7H2Q-9XKM`: short, no look-alikes.
pub fn new_ref() -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let pick = || A[crate::ids::random_below(A.len() as u32) as usize] as char;
    let part = |n: usize| (0..n).map(|_| pick()).collect::<String>();
    format!("BK-{}-{}", part(4), part(4))
}

pub fn new_token() -> String {
    crate::ids::random_hex(24)
}

pub fn is_token(s: &str) -> bool {
    s.len() == 48 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn ics_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n").replace('\r', "")
}

/// An iCalendar file for one booking.
pub fn ics(uid: &str, start: DateTime<Utc>, end: DateTime<Utc>, summary: &str, description: &str, location: &str, url: &str) -> String {
    let f = |d: DateTime<Utc>| d.format("%Y%m%dT%H%M%SZ").to_string();
    let lines = [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Vuuut//Booking//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", f(Utc::now())),
        format!("DTSTART:{}", f(start)),
        format!("DTEND:{}", f(end)),
        format!("SUMMARY:{}", ics_escape(summary)),
        format!("DESCRIPTION:{}", ics_escape(description)),
        format!("LOCATION:{}", ics_escape(location)),
        format!("URL:{url}"),
        "BEGIN:VALARM".into(),
        "TRIGGER:-PT1H".into(),
        "ACTION:DISPLAY".into(),
        format!("DESCRIPTION:{}", ics_escape(summary)),
        "END:VALARM".into(),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
    ];
    lines.join("\r\n") + "\r\n"
}

/// "Add to Google Calendar".
pub fn google_calendar_url(start: DateTime<Utc>, end: DateTime<Utc>, summary: &str, details: &str, location: &str) -> String {
    let f = |d: DateTime<Utc>| d.format("%Y%m%dT%H%M%SZ").to_string();
    let enc = crate::social_media::encode;
    format!(
        "https://calendar.google.com/calendar/render?action=TEMPLATE&text={}&dates={}/{}&details={}&location={}",
        enc(summary), f(start), f(end), enc(details), enc(location)
    )
}

/// `Tuesday 2 December 2026`, `9:30 am` in a zone.
pub fn day_label(d: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    d.with_timezone(&tz).format("%A %-d %B %Y").to_string()
}

pub fn time_label(d: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    let l = d.with_timezone(&tz);
    crate::directory::clock_label((l.hour_min()) as u16)
}

trait HourMin {
    fn hour_min(&self) -> u32;
}
impl<T: chrono::Timelike> HourMin for T {
    fn hour_min(&self) -> u32 {
        self.hour() * 60 + self.minute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(cap: i64) -> Rule {
        Rule { service_id: "s".into(), duration: 60, buffer_before: 0, buffer_after: 15, interval: 30, capacity: cap, min_notice: 0, max_advance_days: 365 }
    }

    #[test]
    fn slots_respect_hours_buffers_bookings_and_time_off() {
        let tz: chrono_tz::Tz = "Europe/London".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(); // Monday
        let now = Utc.with_ymd_and_hms(2027, 2, 1, 0, 0, 0).unwrap();
        let hours = Hours::from_json(r#"{"mon":"09:00-12:00"}"#);
        let p = Person { id: "a".into(), hours: hours.clone(), busy: vec![] };
        let s = slots_for_day(date, tz, now, &rule(1), &Hours::default(), &[p]);
        let times: Vec<String> = s.iter().map(|x| time_label(x.start, tz)).collect();
        assert_eq!(times, ["9 am", "9:30 am", "10 am", "10:30 am", "11 am"]);

        // A booking 10:30–11:45 (its buffer included): only 9:00 still fits,
        // because 9:00 + 60 + its own 15-minute buffer ends at 10:15.
        let b = Busy { start: local(tz, date, 630).unwrap(), end: local(tz, date, 705).unwrap(), service_id: "x".into(), booked_start: None, seats: 1 };
        let p = Person { id: "a".into(), hours, busy: vec![b] };
        let s = slots_for_day(date, tz, now, &rule(1), &Hours::default(), &[p]);
        let times: Vec<String> = s.iter().map(|x| time_label(x.start, tz)).collect();
        assert_eq!(times, ["9 am"]);
    }

    #[test]
    fn groups_share_a_start_and_staff_merge() {
        let tz = chrono_tz::UTC;
        let date = NaiveDate::from_ymd_opt(2027, 3, 2).unwrap(); // Tuesday
        let now = Utc.with_ymd_and_hms(2027, 2, 1, 0, 0, 0).unwrap();
        let h = Hours::from_json(r#"{"tue":"10:00-11:00"}"#);
        let start = local(tz, date, 600).unwrap();
        let busy = vec![Busy { start, end: start + Duration::minutes(75), service_id: "s".into(), booked_start: Some(start), seats: 3 }];
        let a = Person { id: "a".into(), hours: h.clone(), busy };
        let b = Person { id: "b".into(), hours: h, busy: vec![] };
        let s = slots_for_day(date, tz, now, &rule(4), &Hours::default(), &[a, b]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].staff, ["a", "b"]);
        assert_eq!(s[0].seats_left, 4, "the freest person");
    }

    #[test]
    fn notice_and_business_hours_fallback() {
        let tz = chrono_tz::UTC;
        let date = NaiveDate::from_ymd_opt(2027, 3, 3).unwrap(); // Wednesday
        let now = local(tz, date, 9 * 60 + 10).unwrap();
        let mut r = rule(1);
        r.min_notice = 60;
        let p = Person { id: "a".into(), hours: Hours::default(), busy: vec![] };
        let s = slots_for_day(date, tz, now, &r, &default_hours(), &[p]);
        assert_eq!(time_label(s[0].start, tz), "10:30 am", "9:10 + 60 min notice → first half-hour after 10:10");
        assert_eq!(time_label(s.last().unwrap().start, tz), "4 pm");
    }

    #[test]
    fn dst_gap_is_skipped() {
        let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2027, 3, 14).unwrap(); // spring forward, Sunday
        let now = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        let p = Person { id: "a".into(), hours: Hours::from_json(r#"{"sun":"01:00-04:00"}"#), busy: vec![] };
        let s = slots_for_day(date, tz, now, &rule(1), &Hours::default(), &[p]);
        assert!(s.iter().all(|x| x.start.with_timezone(&tz).format("%H").to_string() != "02"));
    }

    #[test]
    fn forms_and_helpers() {
        let pairs: Vec<(String, String)> = [("name", "Deep tissue massage"), ("price", "80"), ("payment", "online"), ("duration_minutes", "90"), ("capacity", "0"), ("staff", "s1"), ("staff", "nope")]
            .iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let s = ServiceInput::from_form(&pairs, |k, v| k == "staff" && v == "s1").unwrap();
        assert_eq!((s.slug.as_str(), s.price_cents, s.payment.as_str(), s.capacity), ("deep-tissue-massage", 8000, "online", 1));
        assert_eq!(s.staff_ids, ["s1"]);
        let free: Vec<(String, String)> = [("name", "Intro call"), ("payment", "online")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(ServiceInput::from_form(&free, |_, _| false).unwrap().payment, "none", "free cannot be paid online");
        let qs = questions_from_json(r#"[{"label":"Allergies?","kind":"long","required":true},{"label":""}]"#);
        let mut f: HashMap<String, String> = [("name", "Ada"), ("email", "ADA@x.io")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert!(BookingInput::from_form(&f, &qs, false, 1).unwrap_err().contains("Allergies"));
        f.insert("q0".into(), "None".into());
        let b = BookingInput::from_form(&f, &qs, false, 1).unwrap();
        assert_eq!((b.email.as_str(), b.answers.len()), ("ada@x.io", 1));
        assert!(new_ref().starts_with("BK-") && is_token(&new_token()));
        assert_eq!(duration_label(90), "1 hr 30 min");
        let ics = ics("u", Utc::now(), Utc::now(), "A, B; C", "x\ny", "", "https://e.x");
        assert!(ics.contains("SUMMARY:A\\, B\\; C") && ics.contains("DESCRIPTION:x\\ny"));
        assert_eq!(Paths::new("appointments").unwrap().public_path("/book/x/"), "/appointments/x/");
        assert!(Paths::new("shop/x").is_none());
    }
}
