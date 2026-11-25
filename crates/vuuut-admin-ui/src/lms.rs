//! Tools → LMS & Online Courses: the admin screens.
//!
//! The rules are [`vuuut_core::lms`], the queries [`vuuut_db::repos::lms`],
//! and the public catalogue at `/courses/` is `vuuut_front::lms`. Here:
//!
//! * **Overview** (`/ghost/lms/`) — what is live, enrolments over the last
//!   thirty days, what needs grading or approving, the busiest courses.
//! * **Courses** — every course, and the course editor with its
//!   curriculum builder (`lms/courses.rs`); lessons, quizzes and
//!   assignments open in their own editor (`lms/lessons.rs`).
//! * **Students** — enrolments, one student's progress, enrolling by hand
//!   (`lms/students.rs`).
//! * **Grading** — assignments waiting for a grade and recent quiz results
//!   (`lms/grading.rs`); **Reviews** (`lms/reviews.rs`).
//! * **Categories** and **Settings** — Administrator only.
//!
//! ## The security contract
//!
//! Editor and above for courses, students, grading and reviews;
//! Administrator for categories and settings. Every write is
//! origin-checked. Course text goes through the same Koenig renderer as
//! posts; everything else is rebuilt by `vuuut_core::lms` and escaped where
//! it is shown. Student names and addresses are shown to staff only, and the
//! CSV export defuses formulas.

use super::*;
use vuuut_core::lms::{self as rules, Access, CourseStatus, Settings};
use vuuut_db::repos::lms as repo;

mod categories;
mod courses;
mod grading;
mod language_demo;
mod lessons;
mod reviews;
mod sample;
mod settings;
mod students;
pub(crate) use categories::*;
pub(crate) use courses::*;
pub(crate) use grading::*;
pub use language_demo::seed_language_academy;
pub(crate) use lessons::*;
pub(crate) use reviews::*;
pub(crate) use sample::sample_load;
pub use sample::{load_courses, seed_sample_academy, Loaded};
pub(crate) use settings::*;
pub(crate) use students::*;

const BASE: &str = "/ghost/lms/";
const COURSES: &str = "/ghost/lms/courses/";
const LESSONS: &str = "/ghost/lms/lessons/";
const STUDENTS: &str = "/ghost/lms/students/";

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

pub(crate) fn lms_settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

/// The site's public address for an internal `/courses/…` path.
fn public_href(s: &Settings, path: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(path))
}

fn tabs(active: &str, admin: bool, waiting: i64) -> String {
    let tab = |key: &str, label: &str, href: &str, badge: i64| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{b}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            b = if badge > 0 { format!(r#" <span class="lm-badge">{badge}</span>"#) } else { String::new() },
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{}{}{}{}{}{}{}</div>"#,
        tab("overview", "Overview", BASE, 0),
        tab("courses", "Courses", COURSES, 0),
        tab("students", "Students", STUDENTS, 0),
        tab("grading", "Grading", "/ghost/lms/grading/", waiting),
        tab("reviews", "Reviews", "/ghost/lms/reviews/", 0),
        if admin { tab("categories", "Categories", "/ghost/lms/categories/", 0) } else { String::new() },
        if admin { tab("settings", "Settings", "/ghost/lms/settings/", 0) } else { String::new() },
    )
}

async fn tabs_for(state: &AppState, active: &str, actor: &Actor) -> String {
    tabs(active, actor.role.is_admin(), repo::waiting_submissions(&state.db).await)
}

fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{ic}</span><span class="dr-tile-value">{value}</span><span class="dr-tile-label">{label}</span><span class="dr-tile-detail">{detail}</span>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(h) => format!(r#"<a class="card padded dr-tile" href="{h}">{body}</a>"#),
        None => format!(r#"<div class="card padded dr-tile">{body}</div>"#),
    }
}

fn status_chip(status: &str) -> (&'static str, &'static str) {
    match CourseStatus::from_key(status) {
        CourseStatus::Published => ("published", "Live"),
        CourseStatus::Draft => ("tonal", "Draft"),
        CourseStatus::Archived => ("draft", "Archived"),
    }
}

fn progress_bar(pct: i64) -> String {
    format!(r#"<span class="lm-bar" role="progressbar" aria-valuenow="{p}" aria-valuemin="0" aria-valuemax="100"><span style="width:{p}%"></span></span>"#, p = pct.clamp(0, 100))
}

fn date(d: &str) -> String {
    esc(d.get(..10).unwrap_or(d))
}

fn price_label(s: &Settings, c: &repo::Course) -> String {
    match c.access() {
        Access::Paid => rules::format_price(c.price_cents, &s.currency),
        Access::Open => "Open".into(),
        Access::Free => "Free".into(),
        Access::Members => if c.tiers().iter().any(|t| t == "paid") { "Paid members".into() } else { "Members".into() },
        Access::Tiers => "Chosen tiers".into(),
    }
}

fn course_row(s: &Settings, c: &repo::Course, x: &repo::Extras) -> String {
    let (chip, label) = status_chip(&c.status);
    let (lessons, minutes) = x.lessons.get(&c.id).copied().unwrap_or((0, 0));
    let students = x.students.get(&c.id).copied().unwrap_or(0);
    let cat = c.category_id.as_deref().and_then(|id| x.categories.iter().find(|k| k.id == id)).map(|k| format!(" · {}", esc(&k.name))).unwrap_or_default();
    let rating = x.ratings.get(&c.id).map(|(r, n)| format!(" · ★ {}.{} ({n})", r / 10, r % 10)).unwrap_or_default();
    format!(
        r#"<div class="row dr-listing lm-course-row">{thumb}
<div class="body"><span class="title"><a href="{COURSES}{id}/">{title}</a> <span class="chip {chip}">{label}</span>{feat}</span>
<span class="detail">{price}{cat} · {lessons} · {dur}{students}{rating}</span></div>
<div class="trailing dr-actions"><a class="icon-btn" href="{COURSES}{id}/" title="Edit">{edit}</a></div></div>"#,
        thumb = if c.image.is_empty() { format!(r#"<span class="ic tint-purple">{}</span>"#, svg("school")) } else { format!(r#"<img class="dr-thumb lm-thumb" src="{}" alt="" loading="lazy">"#, esc(&c.image)) },
        id = esc(&c.id),
        title = esc(&c.title),
        feat = if c.featured != 0 { r#" <span class="chip tonal">Featured</span>"# } else { "" },
        price = esc(&price_label(s, c)),
        lessons = countable(lessons, "lesson"),
        dur = esc(&{ let d = rules::duration_label(if c.duration_minutes > 0 { c.duration_minutes } else { minutes }); if d.is_empty() { "no duration".into() } else { d } }),
        students = format!(" · {}", countable(students, "student")),
        edit = svg("edit"),
    )
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn lms_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = lms_settings(&state);
    let since = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(30)));
    let counts = repo::course_counts(&state.db).await;
    let n = |k: &str| counts.get(k).copied().unwrap_or(0);
    let (students, active, done, recent) = repo::enrollment_stats(&state.db, &since).await;
    let waiting = repo::waiting_submissions(&state.db).await;
    let pending_reviews = repo::pending_reviews(&state.db).await;
    let x = repo::extras(&state.db).await;
    let total_courses: i64 = counts.values().sum();

    let live = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>The catalogue is live</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{home}</a> · students follow their courses at <a href="{dash}" target="_blank" rel="noopener">My learning</a>.</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/courses/"),
            site = esc(state.site_url().trim_end_matches('/')),
            home = esc(&s.paths.public_path("/courses/")),
            dash = public_href(&s, "/courses/dashboard/"),
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>The catalogue is not public yet.</b>
<p class="muted">Switch it on in Settings to publish your courses at {home}.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            home = esc(&s.paths.public_path("/courses/")),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/lms/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };

    // Thirty bars, one a day.
    let per_day: HashMap<String, i64> = repo::enrollments_per_day(&state.db, &since).await.into_iter().collect();
    let days: Vec<(String, i64)> = (0..30)
        .rev()
        .map(|i| {
            let d = (chrono::Utc::now() - chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
            let v = per_day.get(&d).copied().unwrap_or(0);
            (d, v)
        })
        .collect();
    let peak = days.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let chart = format!(
        r#"<div class="card padded lm-chart-card"><div class="lm-chart-head"><b>{recent}</b><span class="muted">enrolments in the last 30 days</span></div>
<div class="lm-chart" role="img" aria-label="Enrolments per day">{bars}</div></div>"#,
        bars = days.iter().map(|(d, v)| format!(r#"<span title="{d}: {v}" style="--h:{h}%"></span>"#, h = (v * 100 / peak).max(if *v > 0 { 6 } else { 2 }))).collect::<String>(),
    );

    let (recent_rows, _) = repo::browse_enrollments(&state.db, &repo::EnrollmentQuery { limit: 8, ..Default::default() }).await;
    let recent_list = if recent_rows.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No enrolments yet. They appear here as students join.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, recent_rows.iter().map(enrollment_row).collect::<String>())
    };
    let mut top: Vec<&repo::Course> = Vec::new();
    let (all, _) = repo::browse(&state.db, &repo::Browse { limit: 200, ..Default::default() }).await.unwrap_or_default();
    for c in &all {
        top.push(c);
    }
    top.sort_by_key(|c| -x.students.get(&c.id).copied().unwrap_or(0));
    let top_list = if top.is_empty() {
        format!(
            r#"<div class="card padded lm-empty"><span class="ic tint-purple">{ic}</span><h3>Build your first course</h3>
<p class="muted">Write the course, add sections and lessons, then quizzes and assignments. Or start from a ready-made academy you can edit.</p>
<div class="lm-empty-actions"><a class="btn" href="{COURSES}new/">{add} New course</a>{sample}</div></div>"#,
            ic = svg("school"),
            add = svg("add"),
            sample = if actor.role.is_admin() { format!(r#"<form method="post" action="/ghost/lms/sample/"><button class="btn tonal" type="submit">{} Load a sample academy</button></form>"#, svg("box")) } else { String::new() },
        )
    } else {
        format!(r#"<div class="card rows">{}</div>"#, top.iter().take(6).map(|c| course_row(&s, c, &x)).collect::<String>())
    };
    let rate = if active + done > 0 { done * 100 / (active + done) } else { 0 };

    page_for_role(
        actor.role,
        "LMS & Online Courses",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{live}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}{e}</div>
<div class="lm-overview rise" style="--i:4"><div><div class="section">Enrolments</div>{chart}
<div class="section">Latest enrolments</div>{recent_list}</div>
<div><div class="section">Courses by students</div>{top_list}</div></div>"#,
            head = crate::tools::tool_head("lms", None, &format!(
                r#"<a class="btn text" href="{BASE}export.csv">{dl} Export CSV</a><a class="btn" href="{COURSES}new/">{add} New course</a>"#,
                dl = svg("import"),
                add = svg("add"),
            )),
            tabs = tabs(
                "overview",
                actor.role.is_admin(),
                waiting,
            ),
            banner = flash_banner(&q),
            a = tile("Live courses", &n("published").to_string(), &format!("{total_courses} in all · {} drafts", n("draft")), Some(&format!("{COURSES}?status=published")), "school", "purple"),
            b = tile("Students", &students.to_string(), &format!("{active} learning now"), Some(STUDENTS), "group", "blue"),
            c = tile("New enrolments", &recent.to_string(), "In the last 30 days", Some(STUDENTS), "trending", "green"),
            d = tile("Completion", &format!("{rate}%"), &format!("{done} finished a course"), Some(&format!("{STUDENTS}?status=completed")), "check", "teal"),
            e = tile("To grade", &waiting.to_string(), &format!("Assignments · {pending_reviews} reviews to approve"), Some("/ghost/lms/grading/"), "edit", if waiting > 0 { "amber" } else { "slate" }),
        ),
        true,
    )
    .into_response()
}

fn enrollment_row(e: &repo::EnrollmentRow) -> String {
    let (chip, label) = match e.status.as_str() {
        "completed" => ("published", "Completed"),
        "cancelled" => ("error", "Cancelled"),
        "expired" => ("draft", "Expired"),
        _ => ("scheduled", "Learning"),
    };
    let who = if e.name.is_empty() { e.email.clone() } else { e.name.clone() };
    format!(
        r#"<div class="row dr-listing"><span class="lm-avatar">{initial}</span>
<div class="body"><span class="title"><a href="{STUDENTS}{mid}/">{who}</a> <span class="chip {chip}">{label}</span></span>
<span class="detail">{course} · enrolled {when} · {source}</span></div>
<div class="trailing lm-progress-cell">{bar}<span class="muted">{p}%</span></div></div>"#,
        initial = esc(&who.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        mid = esc(&e.member_id),
        who = esc(&who),
        course = esc(&e.course_title),
        when = date(&e.enrolled_at),
        source = esc(match e.source.as_str() { "purchase" => "bought", "staff" => "added by staff", "tier" => "tier member", _ => "free" }),
        bar = progress_bar(e.progress),
        p = e.progress,
    )
}

// ---------------------------------------------------------------------------
// the course list
// ---------------------------------------------------------------------------

pub(crate) async fn courses_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = lms_settings(&state);
    let status = q.get("status").map(String::as_str).filter(|v| matches!(*v, "draft" | "published" | "archived"));
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = vuuut_core::directory::search_terms(&query);
    let x = repo::extras(&state.db).await;
    let category = q.get("category").filter(|c| x.categories.iter().any(|k| &k.id == *c)).cloned();
    let (items, total) = repo::browse(&state.db, &repo::Browse { terms: &terms, status, category_id: category.as_deref(), order: "title", limit: 300, ..Default::default() }).await.unwrap_or_default();
    let opts = |list: &[(&str, &str)], current: Option<&str>| -> String {
        list.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if current.unwrap_or("") == *v { " selected" } else { "" })).collect()
    };
    let cat_opts: String = std::iter::once(r#"<option value="">All categories</option>"#.to_string())
        .chain(x.categories.iter().map(|c| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&c.id), s = if category.as_deref() == Some(c.id.as_str()) { " selected" } else { "" }, n = esc(&c.name))))
        .collect();
    page_for_role(
        actor.role,
        "Courses",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="dr-filter rise" style="--i:2" method="get" action="{COURSES}">
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{q}" placeholder=" " maxlength="100"><span>Search</span></label>
<label class="field fixed" style="margin:0"><select name="status">{st}</select><span>Status</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="category">{cat_opts}</select><span>Category</span>{caret}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("lms", BASE, None, &format!(r#"<a class="btn" href="{COURSES}new/">{} New course</a>"#, svg("add"))),
            tabs = tabs_for(&state, "courses", &actor).await,
            banner = flash_banner(&q),
            q = esc(&query),
            st = opts(&[("", "Any status"), ("published", "Live"), ("draft", "Draft"), ("archived", "Archived")], status),
            caret = SELECT_CARET,
            search = svg("search"),
            count = esc(&countable(total, "course")),
            body = if items.is_empty() {
                format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No courses match.</p><a class="btn" href="{COURSES}new/">{} New course</a></div>"#, svg("add"))
            } else {
                format!(r#"<div class="card rows">{}</div>"#, items.iter().map(|c| course_row(&s, c, &x)).collect::<String>())
            },
        ),
        true,
    )
    .into_response()
}

/// Every enrolment, for a spreadsheet. Formulas defused, every cell quoted.
pub(crate) async fn export_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let (rows, _) = repo::browse_enrollments(&state.db, &repo::EnrollmentQuery { limit: 1000, ..Default::default() }).await;
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("email,name,course,status,source,progress,enrolled,completed,expires,certificate\n");
    for e in &rows {
        out.push_str(&[
            cell(&e.email), cell(&e.name), cell(&e.course_title), cell(&e.status), cell(&e.source), cell(&e.progress.to_string()),
            cell(&e.enrolled_at), cell(e.completed_at.as_deref().unwrap_or("")), cell(e.expires_at.as_deref().unwrap_or("")), cell(e.certificate_code.as_deref().unwrap_or("")),
        ].join(","));
        out.push('\n');
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"enrolments-{}.csv\"", crate::export_date_stamp())),
        ],
        out,
    )
        .into_response()
}
