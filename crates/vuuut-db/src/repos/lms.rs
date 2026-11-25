//! LMS & Online Courses: categories, courses, sections, lessons, questions,
//! enrolments, progress, quiz attempts, assignment submissions and reviews.
//!
//! Every value is bound; search words go into `LIKE ?` placeholders. Long
//! text is MEDIUMTEXT on MySQL and read through `COALESCE`. Anything that
//! changes what the public catalogue shows calls `content::touch()` so the
//! page cache empties.

use super::dberr;
use crate::Db;
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_core::lms::{self as rules, CourseInput, LessonInput, Question, Settings};
use vuuut_core::{dates, ids, Result};

fn touch() {
    crate::content::touch();
}

fn flag(b: bool) -> i64 {
    i64::from(b)
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
    pub image: String,
    pub sort_order: i64,
}

const CAT_COLS: &str = "id, name, slug, description, tint, image, sort_order";

pub async fn categories(pool: &Db) -> Result<Vec<Category>> {
    sqlx::query_as::<_, Category>(&format!("SELECT {CAT_COLS} FROM lms_categories ORDER BY sort_order, name")).fetch_all(pool).await.map_err(dberr)
}

pub async fn category_by_slug(pool: &Db, slug: &str) -> Option<Category> {
    sqlx::query_as::<_, Category>(&format!("SELECT {CAT_COLS} FROM lms_categories WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

#[allow(clippy::too_many_arguments)]
pub async fn save_category(pool: &Db, id: Option<&str>, name: &str, slug: &str, description: &str, tint: &str, image: &str, sort: i64) -> Result<String> {
    touch();
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE lms_categories SET name = ?, slug = ?, description = ?, tint = ?, image = ?, sort_order = ?, updated_at = ? WHERE id = ?")
                .bind(name).bind(slug).bind(description).bind(tint).bind(image).bind(sort).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query("INSERT INTO lms_categories (id, name, slug, description, tint, image, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(name).bind(slug).bind(description).bind(tint).bind(image).bind(sort).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_category(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE lms_courses SET category_id = NULL WHERE category_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM lms_categories WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn category_slug_taken(pool: &Db, slug: &str, except: Option<&str>) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lms_categories WHERE slug = ? AND id <> ?").bind(slug).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0) > 0
}

/// `(category_id, published courses)`.
pub async fn category_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT category_id, COUNT(*) FROM lms_courses WHERE status = 'published' AND category_id IS NOT NULL GROUP BY category_id")
        .fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

// ---------------------------------------------------------------------------
// courses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Course {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub description_lexical: String,
    pub description_html: String,
    pub image: String,
    pub intro_video: String,
    pub category_id: Option<String>,
    pub instructor_id: Option<String>,
    pub level: String,
    pub language: String,
    pub duration_minutes: i64,
    pub status: String,
    pub access: String,
    pub price_cents: i64,
    pub compare_cents: i64,
    pub product_id: Option<String>,
    pub tier_ids: String,
    pub outcomes: String,
    pub requirements: String,
    pub audience: String,
    pub keywords: String,
    pub featured: i64,
    pub sequential: i64,
    pub certificate: i64,
    pub capacity: i64,
    pub access_days: i64,
    pub opens_at: Option<String>,
    pub closes_at: Option<String>,
    pub seo_title: String,
    pub seo_description: String,
    pub views: i64,
    pub sort_order: i64,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Course {
    pub fn access(&self) -> rules::Access {
        rules::Access::from_key(&self.access)
    }
    pub fn tiers(&self) -> Vec<String> {
        self.tier_ids.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
    }
    pub fn is_published(&self) -> bool {
        self.status == "published"
    }
    /// The description as HTML: the editor's, or the plain copy as paragraphs.
    pub fn description_markup(&self) -> String {
        if self.description_html.trim().is_empty() { vuuut_core::directory::paragraphs(&self.description) } else { self.description_html.clone() }
    }
    /// The price a student pays: nothing unless access is Paid.
    pub fn price(&self) -> i64 {
        if self.access() == rules::Access::Paid { self.price_cents } else { 0 }
    }
}

const COURSE_COLS: &str = "id, slug, title, subtitle, COALESCE(description, '') AS description, COALESCE(description_lexical, '') AS description_lexical, \
COALESCE(description_html, '') AS description_html, image, intro_video, category_id, instructor_id, level, language, duration_minutes, status, access, \
price_cents, compare_cents, product_id, tier_ids, COALESCE(outcomes, '') AS outcomes, COALESCE(requirements, '') AS requirements, \
COALESCE(audience, '') AS audience, keywords, featured, sequential, certificate, capacity, access_days, opens_at, closes_at, seo_title, seo_description, \
views, sort_order, published_at, created_at, updated_at";

pub async fn find_course(pool: &Db, id: &str) -> Option<Course> {
    sqlx::query_as::<_, Course>(&format!("SELECT {COURSE_COLS} FROM lms_courses WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn course_by_slug(pool: &Db, slug: &str) -> Option<Course> {
    sqlx::query_as::<_, Course>(&format!("SELECT {COURSE_COLS} FROM lms_courses WHERE slug = ?")).bind(slug).fetch_optional(pool).await.ok().flatten()
}

pub async fn courses_by_ids(pool: &Db, ids: &[String]) -> Vec<Course> {
    if ids.is_empty() {
        return Vec::new();
    }
    let marks = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT {COURSE_COLS} FROM lms_courses WHERE id IN ({marks})");
    let mut q = sqlx::query_as::<_, Course>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.unwrap_or_default()
}

/// Courses a store product unlocks.
pub async fn courses_for_products(pool: &Db, product_ids: &[String]) -> Vec<Course> {
    if product_ids.is_empty() {
        return Vec::new();
    }
    let marks = vec!["?"; product_ids.len()].join(",");
    let sql = format!("SELECT {COURSE_COLS} FROM lms_courses WHERE access = 'paid' AND product_id IN ({marks})");
    let mut q = sqlx::query_as::<_, Course>(&sql);
    for id in product_ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.unwrap_or_default()
}

pub async fn unique_course_slug(pool: &Db, wanted: &str, except: Option<&str>) -> String {
    let base = match wanted {
        "" => "course".to_string(),
        w if rules::RESERVED_SLUGS.contains(&w) => format!("{w}-course"),
        w => w.to_string(),
    };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lms_courses WHERE slug = ? AND id <> ?").bind(&candidate).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

pub async fn insert_course(pool: &Db, i: &CourseInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query("INSERT INTO lms_courses (id, slug, title, status, created_at, updated_at) VALUES (?,?,?,?,?,?)")
        .bind(&id).bind(ids::new_id()).bind(&i.title).bind("draft").bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    update_course(pool, &id, i).await?;
    Ok(id)
}

pub async fn update_course(pool: &Db, id: &str, i: &CourseInput) -> Result<()> {
    touch();
    let slug = unique_course_slug(pool, &i.slug, Some(id)).await;
    let now = dates::now_db();
    sqlx::query(
        "UPDATE lms_courses SET slug = ?, title = ?, subtitle = ?, description = ?, description_lexical = ?, description_html = ?, image = ?, \
         intro_video = ?, category_id = ?, instructor_id = ?, level = ?, language = ?, duration_minutes = ?, status = ?, access = ?, price_cents = ?, \
         compare_cents = ?, product_id = ?, tier_ids = ?, outcomes = ?, requirements = ?, audience = ?, keywords = ?, featured = ?, sequential = ?, \
         certificate = ?, capacity = ?, access_days = ?, opens_at = ?, closes_at = ?, seo_title = ?, seo_description = ?, \
         published_at = CASE WHEN ? = 'published' THEN COALESCE(published_at, ?) ELSE published_at END, updated_at = ? WHERE id = ?",
    )
    .bind(&slug).bind(&i.title).bind(&i.subtitle).bind(&i.description).bind(&i.description_lexical).bind(&i.description_html).bind(&i.image)
    .bind(&i.intro_video).bind(i.category_id.clone()).bind(i.instructor_id.clone()).bind(&i.level).bind(&i.language).bind(i.duration_minutes)
    .bind(&i.status).bind(&i.access).bind(i.price_cents).bind(i.compare_cents).bind(i.product_id.clone()).bind(i.tier_ids.join(","))
    .bind(i.outcomes.join("\n")).bind(i.requirements.join("\n")).bind(i.audience.join("\n")).bind(&i.keywords).bind(flag(i.featured))
    .bind(flag(i.sequential)).bind(flag(i.certificate)).bind(i.capacity).bind(i.access_days).bind(i.opens_at.clone()).bind(i.closes_at.clone())
    .bind(&i.seo_title).bind(&i.seo_description).bind(&i.status).bind(&now).bind(&now).bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

pub async fn set_course_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    touch();
    let now = dates::now_db();
    sqlx::query("UPDATE lms_courses SET status = ?, published_at = CASE WHEN ? = 'published' THEN COALESCE(published_at, ?) ELSE published_at END, updated_at = ? WHERE id = ?")
        .bind(status).bind(status).bind(&now).bind(&now).bind(id)
        .execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// The course and everything in it, enrolments and their records included.
pub async fn delete_course(pool: &Db, id: &str) -> Result<()> {
    touch();
    let lesson_sub = "SELECT id FROM lms_lessons WHERE course_id = ?";
    let enrol_sub = "SELECT id FROM lms_enrollments WHERE course_id = ?";
    for sql in [
        format!("DELETE FROM lms_questions WHERE lesson_id IN ({lesson_sub})"),
        format!("DELETE FROM lms_progress WHERE enrollment_id IN ({enrol_sub})"),
        format!("DELETE FROM lms_attempts WHERE enrollment_id IN ({enrol_sub})"),
        format!("DELETE FROM lms_submissions WHERE enrollment_id IN ({enrol_sub})"),
        "DELETE FROM lms_lessons WHERE course_id = ?".to_string(),
        "DELETE FROM lms_sections WHERE course_id = ?".to_string(),
        "DELETE FROM lms_enrollments WHERE course_id = ?".to_string(),
        "DELETE FROM lms_reviews WHERE course_id = ?".to_string(),
        "DELETE FROM lms_courses WHERE id = ?".to_string(),
    ] {
        sqlx::query(&sql).bind(id).execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

/// A copy as a draft, with its sections, lessons and questions; no students.
pub async fn duplicate_course(pool: &Db, id: &str) -> Result<Option<String>> {
    let Some(c) = find_course(pool, id).await else { return Ok(None) };
    let input = input_of(&c, &format!("{} (copy)", c.title));
    let new_id = insert_course(pool, &input).await?;
    let mut section_map: HashMap<String, String> = HashMap::new();
    for s in sections(pool, id).await {
        let sid = add_section(pool, &new_id, &s.title).await?;
        section_map.insert(s.id.clone(), sid);
    }
    for l in lessons(pool, id).await {
        let mut li = lesson_input_of(&l);
        li.section_id = l.section_id.as_ref().and_then(|s| section_map.get(s).cloned());
        let lid = insert_lesson(pool, &new_id, &li).await?;
        let mut qs = questions(pool, &l.id).await;
        for q in &mut qs {
            q.id.clear();
        }
        save_questions(pool, &lid, &qs).await?;
    }
    Ok(Some(new_id))
}

/// A saved course as the form that would save it again.
pub fn input_of(c: &Course, title: &str) -> CourseInput {
    CourseInput {
        title: title.into(),
        slug: vuuut_core::directory::slugify(title),
        subtitle: c.subtitle.clone(),
        description: c.description.clone(),
        description_lexical: c.description_lexical.clone(),
        description_html: c.description_html.clone(),
        image: c.image.clone(),
        intro_video: c.intro_video.clone(),
        category_id: c.category_id.clone(),
        instructor_id: c.instructor_id.clone(),
        level: c.level.clone(),
        language: c.language.clone(),
        duration_minutes: c.duration_minutes,
        status: "draft".into(),
        access: c.access.clone(),
        price_cents: c.price_cents,
        compare_cents: c.compare_cents,
        product_id: None,
        tier_ids: c.tiers(),
        outcomes: rules::lines(&c.outcomes),
        requirements: rules::lines(&c.requirements),
        audience: rules::lines(&c.audience),
        keywords: c.keywords.clone(),
        featured: false,
        sequential: c.sequential != 0,
        certificate: c.certificate != 0,
        capacity: c.capacity,
        access_days: c.access_days,
        opens_at: None,
        closes_at: None,
        seo_title: c.seo_title.clone(),
        seo_description: c.seo_description.clone(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Browse<'a> {
    pub terms: &'a [String],
    pub category_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub level: Option<&'a str>,
    /// `free` (no price to pay) or `paid`.
    pub price: Option<&'a str>,
    pub instructor_id: Option<&'a str>,
    pub featured: bool,
    /// `newest` (default: featured, then order, then newest), `popular`, `title`, `price`.
    pub order: &'a str,
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse(pool: &Db, b: &Browse<'_>) -> Result<(Vec<Course>, i64)> {
    let mut sql = String::from(" WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    for t in b.terms {
        sql.push_str(" AND (LOWER(title) LIKE ? OR LOWER(subtitle) LIKE ? OR LOWER(COALESCE(description, '')) LIKE ? OR LOWER(keywords) LIKE ?)");
        for _ in 0..4 {
            binds.push(format!("%{}%", t.replace(['%', '_'], "")));
        }
    }
    for (col, v) in [("category_id", b.category_id), ("status", b.status), ("level", b.level), ("instructor_id", b.instructor_id)] {
        if let Some(v) = v {
            sql.push_str(&format!(" AND {col} = ?"));
            binds.push(v.to_string());
        }
    }
    match b.price {
        Some("free") => sql.push_str(" AND NOT (access = 'paid' AND price_cents > 0)"),
        Some("paid") => sql.push_str(" AND access = 'paid' AND price_cents > 0"),
        _ => {}
    }
    if b.featured {
        sql.push_str(" AND featured = 1");
    }
    let total = {
        let count_sql = format!("SELECT COUNT(*) FROM lms_courses{sql}");
        let mut q = sqlx::query_scalar::<_, i64>(&count_sql);
        for v in &binds {
            q = q.bind(v.clone());
        }
        q.fetch_one(pool).await.map_err(dberr)?
    };
    let order = match b.order {
        "popular" => "views DESC, published_at DESC",
        "title" => "LOWER(title), id",
        "price" => "CASE WHEN access = 'paid' THEN price_cents ELSE 0 END, LOWER(title)",
        "newest" => "COALESCE(published_at, created_at) DESC, id DESC",
        _ => "featured DESC, sort_order, COALESCE(published_at, created_at) DESC, id DESC",
    };
    let sql = format!("SELECT {COURSE_COLS} FROM lms_courses{sql} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut q = sqlx::query_as::<_, Course>(&sql);
    for v in &binds {
        q = q.bind(v.clone());
    }
    let rows = q.bind(b.limit.clamp(1, 500)).bind(b.offset.max(0)).fetch_all(pool).await.map_err(dberr)?;
    Ok((rows, total))
}

/// `(status, count)`.
pub async fn course_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM lms_courses GROUP BY status").fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

pub async fn add_view(pool: &Db, id: &str) {
    let _ = sqlx::query("UPDATE lms_courses SET views = views + 1 WHERE id = ?").bind(id).execute(pool).await;
}

// ---------------------------------------------------------------------------
// sections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Section {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub summary: String,
    pub sort_order: i64,
}

pub async fn sections(pool: &Db, course_id: &str) -> Vec<Section> {
    sqlx::query_as::<_, Section>("SELECT id, course_id, title, summary, sort_order FROM lms_sections WHERE course_id = ? ORDER BY sort_order, created_at")
        .bind(course_id).fetch_all(pool).await.unwrap_or_default()
}

pub async fn add_section(pool: &Db, course_id: &str, title: &str) -> Result<String> {
    touch();
    let id = ids::new_id();
    let now = dates::now_db();
    let next: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM lms_sections WHERE course_id = ?").bind(course_id).fetch_one(pool).await.unwrap_or(0);
    sqlx::query("INSERT INTO lms_sections (id, course_id, title, summary, sort_order, created_at, updated_at) VALUES (?,?,?,'',?,?,?)")
        .bind(&id).bind(course_id).bind(title).bind(next).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

/// The section is removed; its lessons stay in the course, outside any section.
pub async fn delete_section(pool: &Db, course_id: &str, id: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE lms_lessons SET section_id = NULL WHERE section_id = ? AND course_id = ?").bind(id).bind(course_id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM lms_sections WHERE id = ? AND course_id = ?").bind(id).bind(course_id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// The curriculum as the builder sends it: sections in order with their
/// titles, each with its lessons in order. Ids not in this course are
/// ignored, so the form cannot move another course's lessons.
pub async fn save_curriculum(pool: &Db, course_id: &str, plan: &[(String, String, Vec<String>)], loose: &[String]) -> Result<()> {
    touch();
    let now = dates::now_db();
    for (si, (sid, title, lessons)) in plan.iter().enumerate() {
        sqlx::query("UPDATE lms_sections SET title = ?, sort_order = ?, updated_at = ? WHERE id = ? AND course_id = ?")
            .bind(title).bind(si as i64).bind(&now).bind(sid).bind(course_id)
            .execute(pool).await.map_err(dberr)?;
        for (li, lid) in lessons.iter().enumerate() {
            sqlx::query("UPDATE lms_lessons SET section_id = ?, sort_order = ? WHERE id = ? AND course_id = ?")
                .bind(sid).bind(li as i64).bind(lid).bind(course_id)
                .execute(pool).await.map_err(dberr)?;
        }
    }
    for (li, lid) in loose.iter().enumerate() {
        sqlx::query("UPDATE lms_lessons SET section_id = NULL, sort_order = ? WHERE id = ? AND course_id = ?")
            .bind(li as i64).bind(lid).bind(course_id)
            .execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// lessons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Lesson {
    pub id: String,
    pub course_id: String,
    pub section_id: Option<String>,
    pub slug: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub content_lexical: String,
    pub content_html: String,
    pub video_url: String,
    pub duration_minutes: i64,
    pub preview: i64,
    pub drip_days: i64,
    pub attachments: String,
    pub pass_mark: i64,
    pub attempts: i64,
    pub time_limit: i64,
    pub shuffle: i64,
    pub reveal: i64,
    pub points: i64,
    pub sort_order: i64,
}

impl Lesson {
    pub fn kind(&self) -> rules::LessonKind {
        rules::LessonKind::from_key(&self.kind)
    }
    pub fn is_preview(&self) -> bool {
        self.preview != 0
    }
}

const LESSON_COLS: &str = "id, course_id, section_id, slug, title, kind, status, summary, COALESCE(content_lexical, '') AS content_lexical, \
COALESCE(content_html, '') AS content_html, video_url, duration_minutes, preview, drip_days, COALESCE(attachments, '') AS attachments, pass_mark, \
attempts, time_limit, shuffle, reveal, points, sort_order";

/// Every lesson of a course in curriculum order: by section, then within it;
/// lessons outside a section last.
pub async fn lessons(pool: &Db, course_id: &str) -> Vec<Lesson> {
    let mut all = sqlx::query_as::<_, Lesson>(&format!("SELECT {LESSON_COLS} FROM lms_lessons WHERE course_id = ? ORDER BY sort_order, created_at"))
        .bind(course_id).fetch_all(pool).await.unwrap_or_default();
    let order: HashMap<String, i64> = sections(pool, course_id).await.into_iter().map(|s| (s.id, s.sort_order)).collect();
    all.sort_by_key(|l| (l.section_id.as_ref().and_then(|s| order.get(s).copied()).unwrap_or(i64::MAX), l.sort_order));
    all
}

pub async fn find_lesson(pool: &Db, id: &str) -> Option<Lesson> {
    sqlx::query_as::<_, Lesson>(&format!("SELECT {LESSON_COLS} FROM lms_lessons WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn unique_lesson_slug(pool: &Db, course_id: &str, wanted: &str, except: Option<&str>) -> String {
    let base = if wanted.is_empty() { "lesson".to_string() } else { wanted.to_string() };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lms_lessons WHERE course_id = ? AND slug = ? AND id <> ?")
            .bind(course_id).bind(&candidate).bind(except.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

pub fn lesson_input_of(l: &Lesson) -> LessonInput {
    LessonInput {
        title: l.title.clone(),
        slug: l.slug.clone(),
        kind: l.kind.clone(),
        status: l.status.clone(),
        section_id: l.section_id.clone(),
        summary: l.summary.clone(),
        content_lexical: l.content_lexical.clone(),
        content_html: l.content_html.clone(),
        video_url: l.video_url.clone(),
        duration_minutes: l.duration_minutes,
        preview: l.preview != 0,
        drip_days: l.drip_days,
        attachments: rules::attachments(&l.attachments),
        pass_mark: l.pass_mark,
        attempts: l.attempts,
        time_limit: l.time_limit,
        shuffle: l.shuffle != 0,
        reveal: l.reveal != 0,
        points: l.points,
    }
}

pub async fn insert_lesson(pool: &Db, course_id: &str, i: &LessonInput) -> Result<String> {
    let id = ids::new_id();
    let now = dates::now_db();
    let next: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM lms_lessons WHERE course_id = ? AND COALESCE(section_id, '') = ?")
        .bind(course_id).bind(i.section_id.clone().unwrap_or_default()).fetch_one(pool).await.unwrap_or(0);
    sqlx::query("INSERT INTO lms_lessons (id, course_id, slug, title, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?)")
        .bind(&id).bind(course_id).bind(ids::new_id()).bind(&i.title).bind(next).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    update_lesson(pool, &id, course_id, i).await?;
    Ok(id)
}

pub async fn update_lesson(pool: &Db, id: &str, course_id: &str, i: &LessonInput) -> Result<()> {
    touch();
    let slug = unique_lesson_slug(pool, course_id, &i.slug, Some(id)).await;
    let attachments = i.attachments.iter().map(|(l, u)| format!("{l}|{u}")).collect::<Vec<_>>().join("\n");
    sqlx::query(
        "UPDATE lms_lessons SET section_id = ?, slug = ?, title = ?, kind = ?, status = ?, summary = ?, content_lexical = ?, content_html = ?, \
         video_url = ?, duration_minutes = ?, preview = ?, drip_days = ?, attachments = ?, pass_mark = ?, attempts = ?, time_limit = ?, shuffle = ?, \
         reveal = ?, points = ?, updated_at = ? WHERE id = ? AND course_id = ?",
    )
    .bind(i.section_id.clone()).bind(&slug).bind(&i.title).bind(&i.kind).bind(&i.status).bind(&i.summary).bind(&i.content_lexical)
    .bind(&i.content_html).bind(&i.video_url).bind(i.duration_minutes).bind(flag(i.preview)).bind(i.drip_days).bind(attachments)
    .bind(i.pass_mark).bind(i.attempts).bind(i.time_limit).bind(flag(i.shuffle)).bind(flag(i.reveal)).bind(i.points)
    .bind(dates::now_db()).bind(id).bind(course_id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    Ok(())
}

pub async fn delete_lesson(pool: &Db, id: &str) -> Result<()> {
    touch();
    for sql in [
        "DELETE FROM lms_questions WHERE lesson_id = ?",
        "DELETE FROM lms_progress WHERE lesson_id = ?",
        "DELETE FROM lms_attempts WHERE lesson_id = ?",
        "DELETE FROM lms_submissions WHERE lesson_id = ?",
        "DELETE FROM lms_lessons WHERE id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

/// `(course_id, published lessons, minutes)`, for cards.
pub async fn lesson_totals(pool: &Db) -> HashMap<String, (i64, i64)> {
    sqlx::query_as::<_, (String, i64, i64)>("SELECT course_id, COUNT(*), COALESCE(SUM(duration_minutes), 0) FROM lms_lessons WHERE status = 'published' GROUP BY course_id")
        .fetch_all(pool).await.unwrap_or_default().into_iter().map(|(c, n, m)| (c, (n, m))).collect()
}

// ---------------------------------------------------------------------------
// questions
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct QuestionRow {
    id: String,
    kind: String,
    prompt: String,
    options: String,
    answer: String,
    explanation: String,
    points: i64,
}

pub async fn questions(pool: &Db, lesson_id: &str) -> Vec<Question> {
    sqlx::query_as::<_, QuestionRow>("SELECT id, kind, prompt, COALESCE(options, '') AS options, answer, explanation, points FROM lms_questions WHERE lesson_id = ? ORDER BY sort_order, created_at")
        .bind(lesson_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| Question {
            id: r.id,
            kind: r.kind,
            prompt: r.prompt,
            options: serde_json::from_str(&r.options).unwrap_or_default(),
            answers: r.answer.split('|').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect(),
            explanation: r.explanation,
            points: r.points,
        })
        .collect()
}

/// Replace a quiz's questions with `qs`, keeping the ids of those that
/// already had one (so past attempts still line up).
pub async fn save_questions(pool: &Db, lesson_id: &str, qs: &[Question]) -> Result<()> {
    let keep: Vec<&str> = qs.iter().map(|q| q.id.as_str()).filter(|i| !i.is_empty()).collect();
    let existing: Vec<String> = sqlx::query_scalar("SELECT id FROM lms_questions WHERE lesson_id = ?").bind(lesson_id).fetch_all(pool).await.unwrap_or_default();
    for old in existing.iter().filter(|o| !keep.contains(&o.as_str())) {
        sqlx::query("DELETE FROM lms_questions WHERE id = ?").bind(old).execute(pool).await.map_err(dberr)?;
    }
    let now = dates::now_db();
    for (n, q) in qs.iter().enumerate() {
        let options = if q.options.is_empty() { String::new() } else { serde_json::to_string(&q.options).unwrap_or_default() };
        let answer = q.answers.join("|");
        if !q.id.is_empty() && existing.contains(&q.id) {
            sqlx::query("UPDATE lms_questions SET kind = ?, prompt = ?, options = ?, answer = ?, explanation = ?, points = ?, sort_order = ?, updated_at = ? WHERE id = ? AND lesson_id = ?")
                .bind(&q.kind).bind(&q.prompt).bind(&options).bind(&answer).bind(&q.explanation).bind(q.points).bind(n as i64).bind(&now).bind(&q.id).bind(lesson_id)
                .execute(pool).await.map_err(dberr)?;
        } else {
            sqlx::query("INSERT INTO lms_questions (id, lesson_id, kind, prompt, options, answer, explanation, points, sort_order, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)")
                .bind(ids::new_id()).bind(lesson_id).bind(&q.kind).bind(&q.prompt).bind(&options).bind(&answer).bind(&q.explanation).bind(q.points).bind(n as i64).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
        }
    }
    Ok(())
}

pub async fn question_counts(pool: &Db, course_id: &str) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT q.lesson_id, COUNT(*) FROM lms_questions q JOIN lms_lessons l ON l.id = q.lesson_id WHERE l.course_id = ? GROUP BY q.lesson_id")
        .bind(course_id).fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

// ---------------------------------------------------------------------------
// enrolments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Enrollment {
    pub id: String,
    pub course_id: String,
    pub member_id: String,
    pub status: String,
    pub source: String,
    pub order_id: Option<String>,
    pub progress: i64,
    pub last_lesson_id: Option<String>,
    pub certificate_code: Option<String>,
    pub enrolled_at: String,
    pub completed_at: Option<String>,
    pub expires_at: Option<String>,
    pub last_active_at: Option<String>,
}

impl Enrollment {
    /// Active or completed, and not past its expiry.
    pub fn grants_access(&self, now: &str) -> bool {
        matches!(self.status.as_str(), "active" | "completed") && self.expires_at.as_deref().is_none_or(|e| e > now)
    }
}

const ENROL_COLS: &str = "id, course_id, member_id, status, source, order_id, progress, last_lesson_id, certificate_code, enrolled_at, completed_at, expires_at, last_active_at";

pub async fn enrollment(pool: &Db, course_id: &str, member_id: &str) -> Option<Enrollment> {
    sqlx::query_as::<_, Enrollment>(&format!("SELECT {ENROL_COLS} FROM lms_enrollments WHERE course_id = ? AND member_id = ?"))
        .bind(course_id).bind(member_id).fetch_optional(pool).await.ok().flatten()
}

pub async fn find_enrollment(pool: &Db, id: &str) -> Option<Enrollment> {
    sqlx::query_as::<_, Enrollment>(&format!("SELECT {ENROL_COLS} FROM lms_enrollments WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn enrollment_by_certificate(pool: &Db, code: &str) -> Option<Enrollment> {
    sqlx::query_as::<_, Enrollment>(&format!("SELECT {ENROL_COLS} FROM lms_enrollments WHERE certificate_code = ?")).bind(code).fetch_optional(pool).await.ok().flatten()
}

pub async fn enrollments_for_member(pool: &Db, member_id: &str) -> Vec<Enrollment> {
    sqlx::query_as::<_, Enrollment>(&format!("SELECT {ENROL_COLS} FROM lms_enrollments WHERE member_id = ? ORDER BY COALESCE(last_active_at, enrolled_at) DESC"))
        .bind(member_id).fetch_all(pool).await.unwrap_or_default()
}

/// Enrol (or re-enrol) a member. An existing enrolment that still grants
/// access is left as it is, so buying twice never resets progress; a
/// cancelled or expired one is made active again with a fresh expiry.
pub async fn enroll(pool: &Db, course: &Course, member_id: &str, source: &str, order_id: Option<&str>) -> Result<Enrollment> {
    let now = dates::now_db();
    let expires = (course.access_days > 0).then(|| dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(course.access_days))));
    if let Some(e) = enrollment(pool, &course.id, member_id).await {
        if e.grants_access(&now) {
            return Ok(e);
        }
        let status = if e.completed_at.is_some() { "completed" } else { "active" };
        sqlx::query("UPDATE lms_enrollments SET status = ?, source = ?, order_id = COALESCE(?, order_id), expires_at = ?, updated_at = ? WHERE id = ?")
            .bind(status).bind(source).bind(order_id).bind(&expires).bind(&now).bind(&e.id)
            .execute(pool).await.map_err(dberr)?;
        return Ok(find_enrollment(pool, &e.id).await.unwrap_or(e));
    }
    let id = ids::new_id();
    sqlx::query(&format!("INSERT INTO lms_enrollments ({ENROL_COLS}, created_at, updated_at) VALUES (?,?,?,'active',?,?,0,NULL,NULL,?,NULL,?,?,?,?)"))
        .bind(&id).bind(&course.id).bind(member_id).bind(source).bind(order_id).bind(&now).bind(&expires).bind(&now).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    Ok(find_enrollment(pool, &id).await.unwrap_or_default())
}

pub async fn set_enrollment_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE lms_enrollments SET status = ?, updated_at = ? WHERE id = ?").bind(status).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Forget everything this enrolment did: progress, attempts, submissions,
/// completion and certificate.
pub async fn reset_enrollment(pool: &Db, id: &str) -> Result<()> {
    for sql in ["DELETE FROM lms_progress WHERE enrollment_id = ?", "DELETE FROM lms_attempts WHERE enrollment_id = ?", "DELETE FROM lms_submissions WHERE enrollment_id = ?"] {
        sqlx::query(sql).bind(id).execute(pool).await.map_err(dberr)?;
    }
    sqlx::query("UPDATE lms_enrollments SET status = 'active', progress = 0, completed_at = NULL, certificate_code = NULL, last_lesson_id = NULL, updated_at = ? WHERE id = ?")
        .bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_enrollment(pool: &Db, id: &str) -> Result<()> {
    reset_enrollment(pool, id).await?;
    sqlx::query("DELETE FROM lms_enrollments WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `(course_id, students)` — active and completed.
pub async fn student_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT course_id, COUNT(*) FROM lms_enrollments WHERE status IN ('active', 'completed') GROUP BY course_id")
        .fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

/// An enrolment with who and what, for the admin's lists.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct EnrollmentRow {
    pub id: String,
    pub course_id: String,
    pub member_id: String,
    pub status: String,
    pub source: String,
    pub progress: i64,
    pub enrolled_at: String,
    pub completed_at: Option<String>,
    pub expires_at: Option<String>,
    pub last_active_at: Option<String>,
    pub certificate_code: Option<String>,
    pub email: String,
    pub name: String,
    pub course_title: String,
}

#[derive(Debug, Clone, Default)]
pub struct EnrollmentQuery<'a> {
    pub course_id: Option<&'a str>,
    pub member_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub terms: &'a [String],
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse_enrollments(pool: &Db, q: &EnrollmentQuery<'_>) -> (Vec<EnrollmentRow>, i64) {
    let mut sql = String::from(" FROM lms_enrollments e JOIN members m ON m.id = e.member_id JOIN lms_courses c ON c.id = e.course_id WHERE 1 = 1");
    let mut binds: Vec<String> = Vec::new();
    for (col, v) in [("e.course_id", q.course_id), ("e.member_id", q.member_id), ("e.status", q.status)] {
        if let Some(v) = v {
            sql.push_str(&format!(" AND {col} = ?"));
            binds.push(v.to_string());
        }
    }
    for t in q.terms {
        sql.push_str(" AND (LOWER(m.email) LIKE ? OR LOWER(COALESCE(m.name, '')) LIKE ? OR LOWER(c.title) LIKE ?)");
        for _ in 0..3 {
            binds.push(format!("%{}%", t.replace(['%', '_'], "")));
        }
    }
    let count_sql = format!("SELECT COUNT(*){sql}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &binds {
        cq = cq.bind(b.clone());
    }
    let total = cq.fetch_one(pool).await.unwrap_or(0);
    let list_sql = format!(
        "SELECT e.id, e.course_id, e.member_id, e.status, e.source, e.progress, e.enrolled_at, e.completed_at, e.expires_at, e.last_active_at, \
         e.certificate_code, m.email, COALESCE(m.name, '') AS name, c.title AS course_title{sql} ORDER BY e.enrolled_at DESC, e.id DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query_as::<_, EnrollmentRow>(&list_sql);
    for b in &binds {
        lq = lq.bind(b.clone());
    }
    let rows = lq.bind(q.limit.clamp(1, 1000)).bind(q.offset.max(0)).fetch_all(pool).await.unwrap_or_default();
    (rows, total)
}

/// Overview numbers: `(students, active enrolments, completions, enrolments since)`.
pub async fn enrollment_stats(pool: &Db, since: &str) -> (i64, i64, i64, i64) {
    let one = |sql: &'static str| async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.unwrap_or(0) };
    let students = one("SELECT COUNT(DISTINCT member_id) FROM lms_enrollments").await;
    let active = one("SELECT COUNT(*) FROM lms_enrollments WHERE status = 'active'").await;
    let done = one("SELECT COUNT(*) FROM lms_enrollments WHERE status = 'completed'").await;
    let recent = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lms_enrollments WHERE enrolled_at >= ?").bind(since).fetch_one(pool).await.unwrap_or(0);
    (students, active, done, recent)
}

/// Enrolments per day since `since`: `(YYYY-MM-DD, n)`.
pub async fn enrollments_per_day(pool: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>("SELECT SUBSTR(enrolled_at, 1, 10) AS d, COUNT(*) FROM lms_enrollments WHERE enrolled_at >= ? GROUP BY d ORDER BY d")
        .bind(since).fetch_all(pool).await.unwrap_or_default()
}

// ---------------------------------------------------------------------------
// progress
// ---------------------------------------------------------------------------

pub async fn completed_lessons(pool: &Db, enrollment_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT lesson_id FROM lms_progress WHERE enrollment_id = ?").bind(enrollment_id).fetch_all(pool).await.unwrap_or_default()
}

pub async fn touch_enrollment(pool: &Db, enrollment_id: &str, lesson_id: &str) {
    let _ = sqlx::query("UPDATE lms_enrollments SET last_lesson_id = ?, last_active_at = ? WHERE id = ?")
        .bind(lesson_id).bind(dates::now_db()).bind(enrollment_id).execute(pool).await;
}

/// Mark a lesson done and recompute the enrolment. Returns the enrolment
/// afterwards; when this finished the course, `completed_at` is set and —
/// if `certificate` — a certificate code is issued (once).
pub async fn complete_lesson(pool: &Db, e: &Enrollment, lesson_id: &str, course_lessons: &[String], certificate: bool) -> Result<Enrollment> {
    let now = dates::now_db();
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lms_progress WHERE enrollment_id = ? AND lesson_id = ?").bind(&e.id).bind(lesson_id).fetch_one(pool).await.unwrap_or(0);
    if exists == 0 {
        sqlx::query("INSERT INTO lms_progress (id, enrollment_id, lesson_id, completed_at) VALUES (?,?,?,?)")
            .bind(ids::new_id()).bind(&e.id).bind(lesson_id).bind(&now)
            .execute(pool).await.map_err(dberr)?;
    }
    recompute(pool, e, course_lessons, certificate, Some(lesson_id)).await
}

pub async fn uncomplete_lesson(pool: &Db, e: &Enrollment, lesson_id: &str, course_lessons: &[String]) -> Result<Enrollment> {
    sqlx::query("DELETE FROM lms_progress WHERE enrollment_id = ? AND lesson_id = ?").bind(&e.id).bind(lesson_id).execute(pool).await.map_err(dberr)?;
    recompute(pool, e, course_lessons, false, Some(lesson_id)).await
}

async fn recompute(pool: &Db, e: &Enrollment, course_lessons: &[String], certificate: bool, last: Option<&str>) -> Result<Enrollment> {
    let now = dates::now_db();
    let done = completed_lessons(pool, &e.id).await;
    let n = course_lessons.iter().filter(|l| done.contains(l)).count();
    let pct = rules::progress(n, course_lessons.len());
    let finished = !course_lessons.is_empty() && n == course_lessons.len();
    let (status, completed_at) = if finished {
        ("completed".to_string(), Some(e.completed_at.clone().unwrap_or_else(|| now.clone())))
    } else {
        (if e.status == "completed" { "active".into() } else { e.status.clone() }, None)
    };
    let code = if finished && certificate { Some(e.certificate_code.clone().unwrap_or_else(rules::certificate_code)) } else if finished { e.certificate_code.clone() } else { None };
    sqlx::query("UPDATE lms_enrollments SET progress = ?, status = ?, completed_at = ?, certificate_code = ?, last_lesson_id = COALESCE(?, last_lesson_id), last_active_at = ?, updated_at = ? WHERE id = ?")
        .bind(pct).bind(&status).bind(completed_at).bind(code).bind(last).bind(&now).bind(&now).bind(&e.id)
        .execute(pool).await.map_err(dberr)?;
    Ok(find_enrollment(pool, &e.id).await.unwrap_or_else(|| e.clone()))
}

// ---------------------------------------------------------------------------
// quiz attempts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Attempt {
    pub id: String,
    pub enrollment_id: String,
    pub lesson_id: String,
    pub score: i64,
    pub points: i64,
    pub max_points: i64,
    pub passed: i64,
    pub answers: String,
    pub started_at: Option<String>,
    pub submitted_at: String,
}

const ATTEMPT_COLS: &str = "id, enrollment_id, lesson_id, score, points, max_points, passed, COALESCE(answers, '') AS answers, started_at, submitted_at";

#[allow(clippy::too_many_arguments)]
pub async fn add_attempt(pool: &Db, enrollment_id: &str, lesson_id: &str, score: i64, points: i64, max: i64, passed: bool, answers: &str, started_at: Option<&str>) -> Result<String> {
    let id = ids::new_id();
    sqlx::query(&format!("INSERT INTO lms_attempts ({ATTEMPT_COLS}) VALUES (?,?,?,?,?,?,?,?,?,?)").replace("COALESCE(answers, '') AS answers", "answers"))
        .bind(&id).bind(enrollment_id).bind(lesson_id).bind(score).bind(points).bind(max).bind(flag(passed)).bind(answers).bind(started_at).bind(dates::now_db())
        .execute(pool).await.map_err(dberr)?;
    Ok(id)
}

pub async fn attempts(pool: &Db, enrollment_id: &str, lesson_id: &str) -> Vec<Attempt> {
    sqlx::query_as::<_, Attempt>(&format!("SELECT {ATTEMPT_COLS} FROM lms_attempts WHERE enrollment_id = ? AND lesson_id = ? ORDER BY submitted_at DESC, id DESC"))
        .bind(enrollment_id).bind(lesson_id).fetch_all(pool).await.unwrap_or_default()
}

/// Recent attempts across the site with who and which quiz.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct AttemptRow {
    pub id: String,
    pub score: i64,
    pub passed: i64,
    pub submitted_at: String,
    pub email: String,
    pub name: String,
    pub lesson_title: String,
    pub course_title: String,
    pub member_id: String,
}

pub async fn recent_attempts(pool: &Db, limit: i64) -> Vec<AttemptRow> {
    sqlx::query_as::<_, AttemptRow>(
        "SELECT a.id, a.score, a.passed, a.submitted_at, m.email, COALESCE(m.name, '') AS name, l.title AS lesson_title, c.title AS course_title, e.member_id \
         FROM lms_attempts a JOIN lms_enrollments e ON e.id = a.enrollment_id JOIN members m ON m.id = e.member_id \
         JOIN lms_lessons l ON l.id = a.lesson_id JOIN lms_courses c ON c.id = e.course_id ORDER BY a.submitted_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// assignment submissions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Submission {
    pub id: String,
    pub enrollment_id: String,
    pub lesson_id: String,
    pub body: String,
    pub link: String,
    pub status: String,
    pub grade: i64,
    pub feedback: String,
    pub submitted_at: String,
    pub reviewed_at: Option<String>,
}

const SUB_COLS: &str = "s.id, s.enrollment_id, s.lesson_id, COALESCE(s.body, '') AS body, s.link, s.status, s.grade, s.feedback, s.submitted_at, s.reviewed_at";

pub async fn submission(pool: &Db, enrollment_id: &str, lesson_id: &str) -> Option<Submission> {
    sqlx::query_as::<_, Submission>(&format!("SELECT {SUB_COLS} FROM lms_submissions s WHERE s.enrollment_id = ? AND s.lesson_id = ?"))
        .bind(enrollment_id).bind(lesson_id).fetch_optional(pool).await.ok().flatten()
}

pub async fn find_submission(pool: &Db, id: &str) -> Option<Submission> {
    sqlx::query_as::<_, Submission>(&format!("SELECT {SUB_COLS} FROM lms_submissions s WHERE s.id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

/// Hand in (again). A resubmission goes back to waiting for a grade.
pub async fn submit(pool: &Db, enrollment_id: &str, lesson_id: &str, body: &str, link: &str) -> Result<()> {
    let now = dates::now_db();
    match submission(pool, enrollment_id, lesson_id).await {
        Some(s) => {
            sqlx::query("UPDATE lms_submissions SET body = ?, link = ?, status = 'submitted', submitted_at = ?, reviewed_at = NULL WHERE id = ?")
                .bind(body).bind(link).bind(&now).bind(&s.id).execute(pool).await.map_err(dberr)?;
        }
        None => {
            sqlx::query("INSERT INTO lms_submissions (id, enrollment_id, lesson_id, body, link, status, grade, feedback, submitted_at) VALUES (?,?,?,?,?,'submitted',0,'',?)")
                .bind(ids::new_id()).bind(enrollment_id).bind(lesson_id).bind(body).bind(link).bind(&now).execute(pool).await.map_err(dberr)?;
        }
    }
    Ok(())
}

pub async fn grade_submission(pool: &Db, id: &str, status: &str, grade: i64, feedback: &str, staff_id: &str) -> Result<()> {
    sqlx::query("UPDATE lms_submissions SET status = ?, grade = ?, feedback = ?, reviewed_at = ?, reviewed_by = ? WHERE id = ?")
        .bind(status).bind(grade).bind(feedback).bind(dates::now_db()).bind(staff_id).bind(id)
        .execute(pool).await.map_err(dberr)?;
    Ok(())
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct SubmissionRow {
    pub id: String,
    pub status: String,
    pub grade: i64,
    pub body: String,
    pub link: String,
    pub feedback: String,
    pub submitted_at: String,
    pub email: String,
    pub name: String,
    pub member_id: String,
    pub lesson_title: String,
    pub points: i64,
    pub course_title: String,
}

pub async fn submissions(pool: &Db, status: Option<&str>, limit: i64) -> Vec<SubmissionRow> {
    let filter = if status.is_some() { " WHERE s.status = ?" } else { "" };
    let sql = format!(
        "SELECT s.id, s.status, s.grade, COALESCE(s.body, '') AS body, s.link, s.feedback, s.submitted_at, m.email, COALESCE(m.name, '') AS name, e.member_id, \
         l.title AS lesson_title, l.points, c.title AS course_title FROM lms_submissions s JOIN lms_enrollments e ON e.id = s.enrollment_id \
         JOIN members m ON m.id = e.member_id JOIN lms_lessons l ON l.id = s.lesson_id JOIN lms_courses c ON c.id = e.course_id{filter} \
         ORDER BY s.submitted_at DESC LIMIT ?"
    );
    let mut q = sqlx::query_as::<_, SubmissionRow>(&sql);
    if let Some(s) = status {
        q = q.bind(s);
    }
    q.bind(limit).fetch_all(pool).await.unwrap_or_default()
}

pub async fn waiting_submissions(pool: &Db) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lms_submissions WHERE status = 'submitted'").fetch_one(pool).await.unwrap_or(0)
}

// ---------------------------------------------------------------------------
// reviews
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Review {
    pub id: String,
    pub course_id: String,
    pub member_id: String,
    pub rating: i64,
    pub title: String,
    pub body: String,
    pub status: String,
    pub created_at: String,
    pub name: String,
    pub email: String,
    pub course_title: String,
}

const REVIEW_SELECT: &str = "SELECT r.id, r.course_id, r.member_id, r.rating, r.title, r.body, r.status, r.created_at, COALESCE(m.name, '') AS name, m.email, \
c.title AS course_title FROM lms_reviews r JOIN members m ON m.id = r.member_id JOIN lms_courses c ON c.id = r.course_id";

pub async fn reviews_for_course(pool: &Db, course_id: &str, limit: i64) -> Vec<Review> {
    sqlx::query_as::<_, Review>(&format!("{REVIEW_SELECT} WHERE r.course_id = ? AND r.status = 'published' ORDER BY r.created_at DESC LIMIT ?"))
        .bind(course_id).bind(limit).fetch_all(pool).await.unwrap_or_default()
}

pub async fn review_by(pool: &Db, course_id: &str, member_id: &str) -> Option<Review> {
    sqlx::query_as::<_, Review>(&format!("{REVIEW_SELECT} WHERE r.course_id = ? AND r.member_id = ?")).bind(course_id).bind(member_id).fetch_optional(pool).await.ok().flatten()
}

pub async fn all_reviews(pool: &Db, status: Option<&str>, limit: i64) -> Vec<Review> {
    let filter = if status.is_some() { " WHERE r.status = ?" } else { "" };
    let sql = format!("{REVIEW_SELECT}{filter} ORDER BY r.created_at DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, Review>(&sql);
    if let Some(s) = status {
        q = q.bind(s);
    }
    q.bind(limit).fetch_all(pool).await.unwrap_or_default()
}

pub async fn save_review(pool: &Db, course_id: &str, member_id: &str, rating: i64, title: &str, body: &str, status: &str) -> Result<()> {
    touch();
    let now = dates::now_db();
    let n = sqlx::query("UPDATE lms_reviews SET rating = ?, title = ?, body = ?, status = ?, updated_at = ? WHERE course_id = ? AND member_id = ?")
        .bind(rating).bind(title).bind(body).bind(status).bind(&now).bind(course_id).bind(member_id)
        .execute(pool).await.map_err(dberr)?.rows_affected();
    if n == 0 {
        sqlx::query("INSERT INTO lms_reviews (id, course_id, member_id, rating, title, body, status, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
            .bind(ids::new_id()).bind(course_id).bind(member_id).bind(rating).bind(title).bind(body).bind(status).bind(&now).bind(&now)
            .execute(pool).await.map_err(dberr)?;
    }
    Ok(())
}

pub async fn set_review_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    touch();
    sqlx::query("UPDATE lms_reviews SET status = ?, updated_at = ? WHERE id = ?").bind(status).bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn delete_review(pool: &Db, id: &str) -> Result<()> {
    touch();
    sqlx::query("DELETE FROM lms_reviews WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `(course_id, (average × 10, count))` over published reviews.
pub async fn ratings(pool: &Db) -> HashMap<String, (i64, i64)> {
    sqlx::query_as::<_, (String, i64, i64)>("SELECT course_id, SUM(rating), COUNT(*) FROM lms_reviews WHERE status = 'published' GROUP BY course_id")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(c, sum, n)| (c, ((sum * 10 + n / 2) / n.max(1), n)))
        .collect()
}

pub async fn pending_reviews(pool: &Db) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lms_reviews WHERE status = 'pending'").fetch_one(pool).await.unwrap_or(0)
}

// ---------------------------------------------------------------------------
// people
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct Instructor {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub profile_image: String,
    pub bio: String,
}

/// Active staff, for the instructor picker and course pages.
pub async fn instructors(pool: &Db) -> Vec<Instructor> {
    sqlx::query_as::<_, Instructor>("SELECT id, name, slug, COALESCE(profile_image, '') AS profile_image, COALESCE(bio, '') AS bio FROM users WHERE status <> 'inactive' AND status <> 'locked' ORDER BY name")
        .fetch_all(pool).await.unwrap_or_default()
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct MemberLite {
    pub id: String,
    pub email: String,
    pub name: String,
    pub status: String,
}

pub async fn member(pool: &Db, id: &str) -> Option<MemberLite> {
    sqlx::query_as::<_, MemberLite>("SELECT id, email, COALESCE(name, '') AS name, status FROM members WHERE id = ?").bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn member_by_email(pool: &Db, email: &str) -> Option<MemberLite> {
    sqlx::query_as::<_, MemberLite>("SELECT id, email, COALESCE(name, '') AS name, status FROM members WHERE LOWER(email) = ?").bind(email.trim().to_lowercase()).fetch_optional(pool).await.ok().flatten()
}

// ---------------------------------------------------------------------------
// what a theme sees
// ---------------------------------------------------------------------------

/// Counts and names every course card needs, read once per page.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub categories: Vec<Category>,
    pub lessons: HashMap<String, (i64, i64)>,
    pub students: HashMap<String, i64>,
    pub ratings: HashMap<String, (i64, i64)>,
    pub instructors: Vec<Instructor>,
}

pub async fn extras(pool: &Db) -> Extras {
    Extras {
        categories: categories(pool).await.unwrap_or_default(),
        lessons: lesson_totals(pool).await,
        students: student_counts(pool).await,
        ratings: ratings(pool).await,
        instructors: instructors(pool).await,
    }
}

fn link(s: &Settings, internal: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(internal))
}

pub fn category_json(s: &Settings, c: &Category, count: i64) -> Value {
    json!({
        "id": c.id, "name": c.name, "slug": c.slug, "description": c.description, "tint": c.tint,
        "feature_image": if c.image.is_empty() { Value::Null } else { json!(c.image) },
        "count": count,
        "url": link(s, &format!("/courses/category/{}/", c.slug)),
    })
}

pub fn instructor_json(i: &Instructor) -> Value {
    json!({
        "id": i.id, "name": i.name, "slug": i.slug, "bio": i.bio,
        "profile_image": if i.profile_image.is_empty() { Value::Null } else { json!(vuuut_core::url::base_assets(&i.profile_image)) },
        "initials": i.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase(),
        "url": vuuut_core::url::base(&format!("/author/{}/", i.slug)),
    })
}

/// One course as a theme sees it — `{{#get "courses"}}` and `course` in the
/// `lms/*.hbs` templates. Every value is data (escaped by Handlebars) except
/// `description`, the HTML the editor rendered.
pub fn course_json(s: &Settings, c: &Course, x: &Extras) -> Value {
    let cat = c.category_id.as_deref().and_then(|id| x.categories.iter().find(|k| k.id == id));
    let (lessons, lesson_minutes) = x.lessons.get(&c.id).copied().unwrap_or((0, 0));
    let minutes = if c.duration_minutes > 0 { c.duration_minutes } else { lesson_minutes };
    let (rating10, reviews) = x.ratings.get(&c.id).copied().unwrap_or((0, 0));
    let students = x.students.get(&c.id).copied().unwrap_or(0);
    let price = c.price();
    let level = rules::Level::from_key(&c.level);
    let instructor = c.instructor_id.as_deref().and_then(|id| x.instructors.iter().find(|i| i.id == id));
    let plain: String = if c.subtitle.is_empty() { c.description.chars().take(180).collect() } else { c.subtitle.clone() };
    json!({
        "id": c.id,
        "slug": c.slug,
        "title": c.title,
        "subtitle": c.subtitle,
        "excerpt": plain,
        "custom_excerpt": c.subtitle,
        "url": link(s, &format!("/courses/{}/", c.slug)),
        "catalog_url": link(s, "/courses/"),
        "feature_image": if c.image.is_empty() { Value::Null } else { json!(c.image) },
        "description": c.description_markup(),
        "intro_video": rules::video_embed(&c.intro_video).map(|(u, frame)| json!({ "url": u, "iframe": frame })),
        "category": cat.map(|k| json!({ "name": k.name, "slug": k.slug, "tint": k.tint, "url": link(s, &format!("/courses/category/{}/", k.slug)) })),
        "level": level.key(),
        "level_label": level.label(),
        "language": c.language,
        "duration_minutes": minutes,
        "duration": rules::duration_label(minutes),
        "lessons": lessons,
        "access": c.access,
        "free": price == 0,
        "price_cents": price,
        "price": rules::format_price(price, &s.currency),
        "price_label": rules::format_price(price, &s.currency),
        "compare_price": (c.compare_cents > price && price > 0).then(|| rules::format_price(c.compare_cents, &s.currency)),
        "rating": if reviews > 0 { json!(format!("{}.{}", rating10 / 10, rating10 % 10)) } else { Value::Null },
        "rating_percent": rating10 * 2,
        "reviews": reviews,
        "students": if s.show_students { json!(students) } else { Value::Null },
        "featured": c.featured != 0,
        "certificate": c.certificate != 0 && s.certificates,
        "instructor": instructor.map(instructor_json),
        "outcomes": rules::lines(&c.outcomes),
        "requirements": rules::lines(&c.requirements),
        "audience": rules::lines(&c.audience),
        "published_at": c.published_at,
    })
}

// ---------------------------------------------------------------------------
// the store
// ---------------------------------------------------------------------------

/// Keep a linked store product's price in step with the course, when the
/// product is a plain one (exactly one variant). A product with options is
/// left alone — its prices are the store's business.
pub async fn sync_product_price(pool: &Db, product_id: &str, price: i64, compare: i64) {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_variants WHERE product_id = ?").bind(product_id).fetch_one(pool).await.unwrap_or(0);
    if n == 1 {
        touch();
        let _ = sqlx::query("UPDATE ecom_variants SET price_cents = ?, compare_at_cents = ? WHERE product_id = ?").bind(price).bind(compare).bind(product_id).execute(pool).await;
    }
}

/// Store products for the "Sold as" picker: `(id, title, status)`.
pub async fn store_products(pool: &Db) -> Vec<(String, String, String)> {
    sqlx::query_as::<_, (String, String, String)>("SELECT id, title, status FROM ecom_products ORDER BY title LIMIT 500").fetch_all(pool).await.unwrap_or_default()
}

/// Membership tiers: `(id, name)`.
pub async fn tiers(pool: &Db) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, String)>("SELECT id, name FROM products WHERE type = 'paid' ORDER BY name").fetch_all(pool).await.unwrap_or_default()
}
