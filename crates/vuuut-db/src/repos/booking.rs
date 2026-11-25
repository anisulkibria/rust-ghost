//! Booking & Appointment Scheduling: categories, services, staff (people or
//! resources), who gives what, time off and bookings.
//!
//! Every value is bound. Long text is MEDIUMTEXT on MySQL and read through
//! COALESCE. Writes that change what the public booking pages show call
//! `content::touch()` so the page cache empties. Times are stored as UTC in
//! the database's usual `YYYY-MM-DD HH:MM:SS` form.

use super::dberr;
use crate::Db;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_core::booking::{self as rules, ServiceInput, Settings, StaffInput};
use vuuut_core::{dates, ids, Result};

fn touch() {
    crate::content::touch();
}

pub fn db_time(d: DateTime<Utc>) -> String {
    dates::to_db_string(&d)
}

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    dates::from_db_string(s)
}

// ---------------------------------------------------------------------------
// categories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub tint: String,
    pub sort_order: i64,
}

pub async fn categories(pool: &Db) -> Vec<Category> {
    sqlx::query_as::<_, Category>("SELECT id, name, slug, description, tint, sort_order FROM booking_categories ORDER BY sort_order, name").fetch_all(pool).await.unwrap_or_default()
}

pub async fn category_by_slug(pool: &Db, slug: &str) -> Option<Category> {
    sqlx::query_as::<_, Category>("SELECT id, name, slug, description, tint, sort_order FROM booking_categories WHERE slug = ?").bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_category(pool: &Db, id: Option<&str>, name: &str, slug: &str, description: &str, tint: &str, sort: i64) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE booking_categories SET name = ?, slug = ?, description = ?, tint = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(name).bind(slug).bind(description).bind(tint).bind(sort).bind(&now).bind(id).execute(pool).await.map_err(dberr)?;
            Ok(id.into())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO booking_categories (id, name, slug, description, tint, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?)")
                .bind(&id).bind(name).bind(slug).bind(description).bind(tint).bind(sort).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_category(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE booking_services SET category_id = NULL WHERE category_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM booking_categories WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn category_slug_taken(pool: &Db, slug: &str, except: Option<&str>) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM booking_categories WHERE slug = ? AND id <> ?").bind(slug).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0) > 0
}

// ---------------------------------------------------------------------------
// services
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Service {
    pub id: String,
    pub slug: String,
    pub name: String,
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
    pub approval: i64,
    pub min_notice: i64,
    pub max_advance: i64,
    pub external_url: String,
    pub questions: String,
    pub status: String,
    pub featured: i64,
    pub seo_title: String,
    pub seo_description: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl Service {
    pub fn is_group(&self) -> bool {
        self.capacity > 1
    }
    pub fn payment(&self) -> rules::Payment {
        rules::Payment::from_key(&self.payment)
    }
    pub fn questions(&self) -> Vec<rules::Question> {
        rules::questions_from_json(&self.questions)
    }
    pub fn description_markup(&self) -> String {
        if self.description_html.trim().is_empty() { vuuut_core::directory::paragraphs(&self.description) } else { self.description_html.clone() }
    }
    pub fn rule(&self) -> rules::Rule {
        rules::Rule {
            service_id: self.id.clone(),
            duration: self.duration_minutes,
            buffer_before: self.buffer_before,
            buffer_after: self.buffer_after,
            interval: self.slot_interval,
            capacity: self.capacity,
            min_notice: self.min_notice,
            max_advance_days: self.max_advance,
        }
    }
    /// Where it happens, in words.
    pub fn where_label(&self, s: &Settings) -> String {
        match rules::Location::from_key(&self.location_type) {
            rules::Location::InPerson => if !self.location.is_empty() { self.location.clone() } else if !s.address.is_empty() { s.address.clone() } else { "In person".into() },
            rules::Location::Video => "Video call — the link comes with your confirmation".into(),
            rules::Location::Phone => "Phone call — we'll ring the number you give".into(),
            rules::Location::Customer => "At your address".into(),
        }
    }
}

const SERVICE_COLS: &str = "id, slug, name, category_id, summary, COALESCE(description, '') AS description, COALESCE(description_lexical, '') AS description_lexical, \
COALESCE(description_html, '') AS description_html, image, tint, duration_minutes, buffer_before, buffer_after, slot_interval, capacity, price_cents, payment, \
location_type, location, staff_mode, approval, min_notice, max_advance, external_url, COALESCE(questions, '') AS questions, status, featured, seo_title, \
seo_description, sort_order, created_at, updated_at";

pub async fn services(pool: &Db, public_only: bool) -> Vec<Service> {
    let filter = if public_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM booking_services{filter} ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_service(pool: &Db, id: &str) -> Option<Service> {
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM booking_services WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn service_by_slug(pool: &Db, slug: &str) -> Option<Service> {
    sqlx::query_as::<_, Service>(&format!("SELECT {SERVICE_COLS} FROM booking_services WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

async fn unique_slug(pool: &Db, table: &str, wanted: &str, fallback: &str, except: Option<&str>) -> String {
    let base = match wanted {
        "" => fallback.to_string(),
        w if rules::RESERVED_SLUGS.contains(&w) => format!("{w}-{fallback}"),
        w => w.to_string(),
    };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE slug = ? AND id <> ?")).bind(&candidate).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

pub async fn save_service(pool: &Db, id: Option<&str>, i: &ServiceInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO booking_services (id, slug, name, created_at, updated_at) VALUES (?,?,?,?,?)")
                .bind(&id).bind(ids::new_id()).bind(&i.name).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "booking_services", &i.slug, "service", Some(&id)).await;
    let questions = if i.questions.is_empty() { String::new() } else { serde_json::to_string(&i.questions).unwrap_or_default() };
    sqlx::query(
        "UPDATE booking_services SET slug = ?, name = ?, category_id = ?, summary = ?, description = ?, description_lexical = ?, description_html = ?, image = ?, tint = ?, \
         duration_minutes = ?, buffer_before = ?, buffer_after = ?, slot_interval = ?, capacity = ?, price_cents = ?, payment = ?, location_type = ?, location = ?, \
         staff_mode = ?, approval = ?, min_notice = ?, max_advance = ?, external_url = ?, questions = ?, status = ?, featured = ?, seo_title = ?, seo_description = ?, \
         sort_order = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.name).bind(i.category_id.clone()).bind(&i.summary).bind(&i.description).bind(&i.description_lexical).bind(&i.description_html)
    .bind(&i.image).bind(&i.tint).bind(i.duration_minutes).bind(i.buffer_before).bind(i.buffer_after).bind(i.slot_interval).bind(i.capacity)
    .bind(i.price_cents).bind(&i.payment).bind(&i.location_type).bind(&i.location).bind(&i.staff_mode).bind(i64::from(i.approval)).bind(i.min_notice)
    .bind(i.max_advance).bind(&i.external_url).bind(questions).bind(&i.status).bind(i64::from(i.featured)).bind(&i.seo_title).bind(&i.seo_description)
    .bind(i.sort_order).bind(&now).bind(&id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    sqlx::query("DELETE FROM booking_service_staff WHERE service_id = ?").bind(&id).execute(pool).await.map_err(dberr)?;
    for s in &i.staff_ids {
        sqlx::query("INSERT INTO booking_service_staff (service_id, staff_id) VALUES (?, ?)").bind(&id).bind(s).execute(pool).await.map_err(dberr)?;
    }
    Ok(id)
}

pub async fn delete_service(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM booking_service_staff WHERE service_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM booking_services WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `(service_id, staff_id)` pairs.
pub async fn links(pool: &Db) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, String)>("SELECT service_id, staff_id FROM booking_service_staff").fetch_all(pool).await.unwrap_or_default()
}

pub async fn staff_ids_for(pool: &Db, service_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT staff_id FROM booking_service_staff WHERE service_id = ?").bind(service_id).fetch_all(pool).await.unwrap_or_default()
}

// ---------------------------------------------------------------------------
// staff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Staff {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub title: String,
    pub email: String,
    pub phone: String,
    pub bio: String,
    pub photo: String,
    pub tint: String,
    pub hours: String,
    pub status: String,
    pub sort_order: i64,
}

impl Staff {
    pub fn hours(&self) -> vuuut_core::directory::Hours {
        vuuut_core::directory::Hours::from_json(&self.hours)
    }
    pub fn initials(&self) -> String {
        self.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
    }
}

const STAFF_COLS: &str = "id, slug, name, title, email, phone, COALESCE(bio, '') AS bio, photo, tint, COALESCE(hours, '') AS hours, status, sort_order";

pub async fn staff(pool: &Db, active_only: bool) -> Vec<Staff> {
    let filter = if active_only { " WHERE status = 'active'" } else { "" };
    sqlx::query_as::<_, Staff>(&format!("SELECT {STAFF_COLS} FROM booking_staff{filter} ORDER BY sort_order, name")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_staff(pool: &Db, id: &str) -> Option<Staff> {
    sqlx::query_as::<_, Staff>(&format!("SELECT {STAFF_COLS} FROM booking_staff WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn staff_by_slug(pool: &Db, slug: &str) -> Option<Staff> {
    sqlx::query_as::<_, Staff>(&format!("SELECT {STAFF_COLS} FROM booking_staff WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_staff(pool: &Db, id: Option<&str>, i: &StaffInput) -> Result<String> {
    touch();
    let now = dates::now_db();
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO booking_staff (id, slug, name, created_at, updated_at) VALUES (?,?,?,?,?)")
                .bind(&id).bind(ids::new_id()).bind(&i.name).bind(&now).bind(&now).execute(pool).await.map_err(dberr)?;
            id
        }
    };
    let slug = unique_slug(pool, "booking_staff", &i.slug, "staff", Some(&id)).await;
    sqlx::query("UPDATE booking_staff SET slug = ?, name = ?, title = ?, email = ?, phone = ?, bio = ?, photo = ?, tint = ?, hours = ?, status = ?, sort_order = ?, updated_at = ? WHERE id = ?")
        .bind(&slug).bind(&i.name).bind(&i.title).bind(&i.email).bind(&i.phone).bind(&i.bio).bind(&i.photo).bind(&i.tint).bind(i.hours.to_json())
        .bind(&i.status).bind(i.sort_order).bind(&now).bind(&id)
        .execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM booking_service_staff WHERE staff_id = ?").bind(&id).execute(pool).await.map_err(dberr)?;
    for s in &i.service_ids {
        sqlx::query("INSERT INTO booking_service_staff (service_id, staff_id) VALUES (?, ?)").bind(s).bind(&id).execute(pool).await.map_err(dberr)?;
    }
    Ok(id)
}

pub async fn delete_staff(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM booking_service_staff WHERE staff_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM booking_time_off WHERE staff_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("UPDATE booking_bookings SET staff_id = NULL WHERE staff_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM booking_staff WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// time off
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct TimeOff {
    pub id: String,
    pub staff_id: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub reason: String,
}

pub async fn time_off(pool: &Db, from: &str, to: &str) -> Vec<TimeOff> {
    sqlx::query_as::<_, TimeOff>("SELECT id, staff_id, starts_at, ends_at, reason FROM booking_time_off WHERE ends_at > ? AND starts_at < ? ORDER BY starts_at")
        .bind(from).bind(to).fetch_all(pool).await.unwrap_or_default()
}

pub async fn add_time_off(pool: &Db, staff_id: Option<&str>, starts: &str, ends: &str, reason: &str) -> Result<()> {
    touch();
    sqlx::query("INSERT INTO booking_time_off (id, staff_id, starts_at, ends_at, reason, created_at) VALUES (?,?,?,?,?,?)")
        .bind(ids::new_id()).bind(staff_id).bind(starts).bind(ends).bind(reason).bind(dates::now_db()).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_time_off(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM booking_time_off WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// bookings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Booking {
    pub id: String,
    pub r#ref: String,
    pub token: String,
    pub service_id: String,
    pub staff_id: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub seats: i64,
    pub status: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub notes: String,
    pub answers: String,
    pub timezone: String,
    pub price_cents: i64,
    pub payment_status: String,
    pub payment_ref: String,
    pub member_id: Option<String>,
    pub source: String,
    pub cancel_reason: String,
    pub internal_note: String,
    pub reminded_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Booking {
    pub fn start(&self) -> DateTime<Utc> {
        parse_time(&self.starts_at).unwrap_or_else(Utc::now)
    }
    pub fn end(&self) -> DateTime<Utc> {
        parse_time(&self.ends_at).unwrap_or_else(Utc::now)
    }
    pub fn status(&self) -> rules::Status {
        rules::Status::from_key(&self.status)
    }
    pub fn answers(&self) -> Vec<(String, String)> {
        let v: Vec<Value> = serde_json::from_str(&self.answers).unwrap_or_default();
        v.iter().map(|a| (a["label"].as_str().unwrap_or("").to_string(), a["value"].as_str().unwrap_or("").to_string())).collect()
    }
}

const BOOKING_COLS: &str = "id, ref, token, service_id, staff_id, starts_at, ends_at, seats, status, name, email, phone, COALESCE(notes, '') AS notes, \
COALESCE(answers, '') AS answers, timezone, price_cents, payment_status, payment_ref, member_id, source, cancel_reason, COALESCE(internal_note, '') AS internal_note, \
reminded_at, created_at, updated_at";

pub async fn find_booking(pool: &Db, id: &str) -> Option<Booking> {
    sqlx::query_as::<_, Booking>(&format!("SELECT {BOOKING_COLS} FROM booking_bookings WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn booking_by_token(pool: &Db, token: &str) -> Option<Booking> {
    sqlx::query_as::<_, Booking>(&format!("SELECT {BOOKING_COLS} FROM booking_bookings WHERE token = ?")).bind(token).fetch_optional(pool).await.ok().flatten()
}

pub struct NewBooking<'a> {
    pub service: &'a Service,
    pub staff_id: Option<&'a str>,
    pub start: DateTime<Utc>,
    pub input: &'a rules::BookingInput,
    pub status: &'a str,
    pub payment_status: &'a str,
    pub timezone: &'a str,
    pub member_id: Option<&'a str>,
    pub source: &'a str,
}

pub async fn insert_booking(pool: &Db, n: &NewBooking<'_>) -> Result<Booking> {
    touch();
    let id = ids::new_id();
    let now = dates::now_db();
    let end = n.start + chrono::Duration::minutes(n.service.duration_minutes);
    let answers = if n.input.answers.is_empty() { String::new() } else { Value::Array(n.input.answers.iter().map(|(l, v)| json!({ "label": l, "value": v })).collect()).to_string() };
    let price = n.service.price_cents * n.input.seats.max(1);
    let mut r#ref = rules::new_ref();
    for _ in 0..5 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_bookings WHERE ref = ?").bind(&r#ref).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            break;
        }
        r#ref = rules::new_ref();
    }
    sqlx::query(
        "INSERT INTO booking_bookings (id, ref, token, service_id, staff_id, starts_at, ends_at, seats, status, name, email, phone, notes, answers, timezone, \
         price_cents, payment_status, payment_ref, member_id, source, cancel_reason, internal_note, reminded_at, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'',?,?,'','',NULL,?,?)",
    )
    .bind(&id).bind(&r#ref).bind(rules::new_token()).bind(&n.service.id).bind(n.staff_id).bind(db_time(n.start)).bind(db_time(end)).bind(n.input.seats.max(1))
    .bind(n.status).bind(&n.input.name).bind(&n.input.email).bind(&n.input.phone).bind(&n.input.notes).bind(answers).bind(n.timezone).bind(price)
    .bind(n.payment_status).bind(n.member_id).bind(n.source).bind(&now).bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(find_booking(pool, &id).await.unwrap_or_default())
}

pub async fn set_status(pool: &Db, id: &str, status: &str, reason: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE booking_bookings SET status = ?, cancel_reason = CASE WHEN ? = '' THEN cancel_reason ELSE ? END, updated_at = ? WHERE id = ?")
        .bind(status).bind(reason).bind(reason).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_paid(pool: &Db, id: &str, reference: &str, status: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE booking_bookings SET payment_status = 'paid', payment_ref = ?, status = ?, updated_at = ? WHERE id = ?")
        .bind(reference).bind(status).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_payment_status(pool: &Db, id: &str, payment_status: &str) -> Result<()> {
    sqlx::query("UPDATE booking_bookings SET payment_status = ?, updated_at = ? WHERE id = ?").bind(payment_status).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn reschedule(pool: &Db, id: &str, start: DateTime<Utc>, end: DateTime<Utc>, staff_id: Option<&str>) -> Result<()> {
    touch();
    sqlx::query("UPDATE booking_bookings SET starts_at = ?, ends_at = ?, staff_id = ?, reminded_at = NULL, updated_at = ? WHERE id = ?")
        .bind(db_time(start)).bind(db_time(end)).bind(staff_id).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_note(pool: &Db, id: &str, note: &str) -> Result<()> {
    sqlx::query("UPDATE booking_bookings SET internal_note = ?, updated_at = ? WHERE id = ?").bind(note).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_booking(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM booking_bookings WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct Query<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub status: Option<&'a str>,
    pub staff_id: Option<&'a str>,
    pub service_id: Option<&'a str>,
    pub email: Option<&'a str>,
    pub terms: &'a [String],
    /// Newest first instead of soonest first.
    pub newest: bool,
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse(pool: &Db, q: &Query<'_>) -> (Vec<Booking>, i64) {
    let mut sql = String::from(" WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    if let Some(f) = q.from {
        sql.push_str(" AND starts_at >= ?");
        binds.push(f.into());
    }
    if let Some(t) = q.to {
        sql.push_str(" AND starts_at < ?");
        binds.push(t.into());
    }
    match q.status {
        Some("active") => sql.push_str(" AND status IN ('pending', 'confirmed', 'awaiting_payment')"),
        Some(s) => {
            sql.push_str(" AND status = ?");
            binds.push(s.into());
        }
        None => {}
    }
    for (col, v) in [("staff_id", q.staff_id), ("service_id", q.service_id), ("email", q.email)] {
        if let Some(v) = v {
            sql.push_str(&format!(" AND {col} = ?"));
            binds.push(v.into());
        }
    }
    for t in q.terms {
        sql.push_str(" AND (LOWER(name) LIKE ? OR LOWER(email) LIKE ? OR LOWER(ref) LIKE ? OR phone LIKE ?)");
        for _ in 0..4 {
            binds.push(format!("%{}%", t.replace(['%', '_'], "")));
        }
    }
    let count_sql = format!("SELECT COUNT(*) FROM booking_bookings{sql}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &binds {
        cq = cq.bind(b.clone());
    }
    let total = cq.fetch_one(pool).await.unwrap_or(0);
    let order = if q.newest { "created_at DESC, id DESC" } else { "starts_at, id" };
    let list_sql = format!("SELECT {BOOKING_COLS} FROM booking_bookings{sql} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut lq = sqlx::query_as::<_, Booking>(&list_sql);
    for b in &binds {
        lq = lq.bind(b.clone());
    }
    let rows = lq.bind(q.limit.clamp(1, 2000)).bind(q.offset.max(0)).fetch_all(pool).await.unwrap_or_default();
    (rows, total)
}

/// Everything that holds time between `from` and `to`, with buffers added
/// from each booking's service, grouped by staff id. A booking with no
/// person blocks nobody in particular (it is shown, not scheduled).
pub async fn busy(pool: &Db, from: DateTime<Utc>, to: DateTime<Utc>, services: &[Service]) -> HashMap<String, Vec<rules::Busy>> {
    let margin = chrono::Duration::hours(12);
    let (f, t) = (db_time(from - margin), db_time(to + margin));
    let rows: Vec<(Option<String>, String, String, String, i64)> = sqlx::query_as(
        "SELECT staff_id, service_id, starts_at, ends_at, seats FROM booking_bookings \
         WHERE status IN ('awaiting_payment', 'pending', 'confirmed', 'completed') AND ends_at > ? AND starts_at < ?",
    )
    .bind(&f)
    .bind(&t)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out: HashMap<String, Vec<rules::Busy>> = HashMap::new();
    for (staff, service, s, e, seats) in rows {
        // A booking with nobody assigned belongs to the business itself (id ``).
        let staff = staff.unwrap_or_default();
        let (Some(s), Some(e)) = (parse_time(&s), parse_time(&e)) else { continue };
        let svc = services.iter().find(|x| x.id == service);
        let (bb, ba) = svc.map(|x| (x.buffer_before, x.buffer_after)).unwrap_or((0, 0));
        out.entry(staff).or_default().push(rules::Busy {
            start: s - chrono::Duration::minutes(bb),
            end: e + chrono::Duration::minutes(ba),
            service_id: service,
            booked_start: Some(s),
            seats,
        });
    }
    for off in time_off(pool, &f, &t).await {
        let (Some(s), Some(e)) = (parse_time(&off.starts_at), parse_time(&off.ends_at)) else { continue };
        let b = rules::Busy { start: s, end: e, service_id: String::new(), booked_start: None, seats: 0 };
        match off.staff_id {
            Some(id) => out.entry(id).or_default().push(b),
            None => out.entry("*".into()).or_default().push(b),
        }
    }
    out
}

/// Unpaid online bookings past their hold: cancelled, freeing the slot.
pub async fn expire_holds(pool: &Db) -> u64 {
    let cutoff = db_time(Utc::now() - chrono::Duration::minutes(rules::PAYMENT_HOLD_MINUTES));
    sqlx::query("UPDATE booking_bookings SET status = 'cancelled', cancel_reason = 'Payment was not completed in time.', updated_at = ? WHERE status = 'awaiting_payment' AND created_at < ?")
        .bind(dates::now_db()).bind(cutoff).execute(pool).await.map(|r| r.rows_affected()).unwrap_or(0)
}

/// Confirmed bookings starting within `hours` that have had no reminder.
pub async fn due_reminders(pool: &Db, hours: i64) -> Vec<Booking> {
    let now = Utc::now();
    sqlx::query_as::<_, Booking>(&format!(
        "SELECT {BOOKING_COLS} FROM booking_bookings WHERE status = 'confirmed' AND reminded_at IS NULL AND starts_at > ? AND starts_at <= ? LIMIT 50"
    ))
    .bind(db_time(now))
    .bind(db_time(now + chrono::Duration::hours(hours)))
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

pub async fn mark_reminded(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE booking_bookings SET reminded_at = ? WHERE id = ?").bind(dates::now_db()).bind(id).execute(pool).await;
}

/// Overview numbers.
pub struct Stats {
    pub today: i64,
    pub week: i64,
    pub pending: i64,
    pub revenue_30d: i64,
    pub customers: i64,
    pub cancel_rate: i64,
}

pub async fn stats(pool: &Db, day_start: DateTime<Utc>) -> Stats {
    let count = |sql: &'static str, a: String, b: String| async move { sqlx::query_scalar::<_, i64>(sql).bind(a).bind(b).fetch_one(pool).await.unwrap_or(0) };
    let today = count("SELECT COUNT(*) FROM booking_bookings WHERE status IN ('pending','confirmed','completed') AND starts_at >= ? AND starts_at < ?", db_time(day_start), db_time(day_start + chrono::Duration::days(1))).await;
    let week = count("SELECT COUNT(*) FROM booking_bookings WHERE status IN ('pending','confirmed') AND starts_at >= ? AND starts_at < ?", db_time(Utc::now()), db_time(Utc::now() + chrono::Duration::days(7))).await;
    let pending = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM booking_bookings WHERE status = 'pending'").fetch_one(pool).await.unwrap_or(0);
    let since = db_time(Utc::now() - chrono::Duration::days(30));
    let revenue_30d = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(price_cents), 0) FROM booking_bookings WHERE status IN ('confirmed','completed') AND created_at >= ?").bind(&since).fetch_one(pool).await.unwrap_or(0);
    let customers = sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT email) FROM booking_bookings").fetch_one(pool).await.unwrap_or(0);
    let all = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM booking_bookings WHERE created_at >= ?").bind(&since).fetch_one(pool).await.unwrap_or(0);
    let cancelled = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM booking_bookings WHERE status IN ('cancelled','no_show') AND created_at >= ?").bind(&since).fetch_one(pool).await.unwrap_or(0);
    Stats { today, week, pending, revenue_30d, customers, cancel_rate: if all > 0 { cancelled * 100 / all } else { 0 } }
}

/// `(YYYY-MM-DD, count)` of bookings made per day since `since`.
pub async fn per_day(pool: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT SUBSTR(created_at, 1, 10) AS d, COUNT(*) FROM booking_bookings WHERE created_at >= ? GROUP BY d ORDER BY d")
        .bind(since).fetch_all(pool).await.unwrap_or_default()
}

/// `(email, name, bookings, last start, spent)`, busiest first.
pub async fn customers(pool: &Db, limit: i64) -> Vec<(String, String, i64, String, i64)> {
    sqlx::query_as::<_, (String, String, i64, String, i64)>(
        "SELECT email, MAX(name), COUNT(*), MAX(starts_at), COALESCE(SUM(CASE WHEN status IN ('confirmed','completed') THEN price_cents ELSE 0 END), 0) \
         FROM booking_bookings GROUP BY email ORDER BY COUNT(*) DESC, MAX(starts_at) DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// what a theme sees
// ---------------------------------------------------------------------------

fn link(s: &Settings, internal: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(internal))
}

pub fn category_json(s: &Settings, c: &Category, count: usize) -> Value {
    json!({ "id": c.id, "name": c.name, "slug": c.slug, "description": c.description, "tint": c.tint, "count": count,
        "url": format!("{}#{}", link(s, "/book/"), c.slug) })
}

pub fn staff_json(s: &Settings, p: &Staff, services: &[&Service]) -> Value {
    let hours = p.hours();
    json!({
        "id": p.id, "slug": p.slug, "name": p.name, "title": p.title, "bio": p.bio,
        "photo": if p.photo.is_empty() { Value::Null } else { json!(p.photo) },
        "initials": p.initials(), "tint": p.tint,
        "url": link(s, &format!("/book/staff/{}/", p.slug)),
        "services": services.iter().map(|x| json!({ "name": x.name, "url": link(s, &format!("/book/{}/", x.slug)) })).collect::<Vec<_>>(),
        "hours": vuuut_core::directory::DAYS_OF_WEEK.iter().enumerate().filter(|(i, _)| hours.0[*i] != vuuut_core::directory::Day::Unknown)
            .map(|(i, (_, name, short))| json!({ "day": name, "short": short, "hours": hours.0[i].label() })).collect::<Vec<_>>(),
    })
}

/// One service as a theme sees it — `{{#get "services"}}` and `service` in
/// the `booking/*.hbs` templates.
pub fn service_json(s: &Settings, v: &Service, cats: &[Category], people: &[&Staff]) -> Value {
    let cat = v.category_id.as_deref().and_then(|id| cats.iter().find(|c| c.id == id));
    json!({
        "id": v.id, "slug": v.slug, "name": v.name, "title": v.name, "summary": v.summary, "excerpt": v.summary,
        "url": link(s, &format!("/book/{}/", v.slug)),
        "book_url": if v.external_url.is_empty() { json!(link(s, &format!("/book/{}/", v.slug))) } else { json!(v.external_url) },
        "external": !v.external_url.is_empty(),
        "feature_image": if v.image.is_empty() { Value::Null } else { json!(v.image) },
        "description": v.description_markup(),
        "tint": v.tint,
        "category": cat.map(|c| json!({ "name": c.name, "slug": c.slug, "tint": c.tint })),
        "duration_minutes": v.duration_minutes,
        "duration": rules::duration_label(v.duration_minutes),
        "price_cents": v.price_cents,
        "price_label": rules::format_price(v.price_cents, &s.currency),
        "free": v.price_cents == 0,
        "pay_online": v.payment() == rules::Payment::Online && v.price_cents > 0,
        "group": v.is_group(),
        "capacity": v.capacity,
        "location_type": v.location_type,
        "location_label": rules::Location::from_key(&v.location_type).label(),
        "where": v.where_label(s),
        "approval": v.approval != 0,
        "featured": v.featured != 0,
        "staff": people.iter().map(|p| json!({ "name": p.name, "initials": p.initials(), "photo": if p.photo.is_empty() { Value::Null } else { json!(p.photo) }, "url": link(s, &format!("/book/staff/{}/", p.slug)) })).collect::<Vec<_>>(),
    })
}

/// The customer's details, as staff correct them.
pub async fn update_customer(pool: &Db, id: &str, name: &str, email: &str, phone: &str, notes: &str) -> Result<()> {
    sqlx::query("UPDATE booking_bookings SET name = ?, email = ?, phone = ?, notes = ?, updated_at = ? WHERE id = ?")
        .bind(name).bind(email).bind(phone).bind(notes).bind(dates::now_db()).bind(id)
        .execute(pool).await.map_err(dberr)?;
    Ok(())
}
