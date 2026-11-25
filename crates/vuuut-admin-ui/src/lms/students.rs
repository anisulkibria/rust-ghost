//! Students: every enrolment, filterable; one student's courses with their
//! progress, quiz results and certificates; enrolling someone by hand
//! (a member account is made for a new address); and per-enrolment
//! actions — cancel, restore, reset progress, mark complete, remove.

use super::*;

pub(crate) async fn students_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (courses, _) = repo::browse(&state.db, &repo::Browse { order: "title", limit: 500, ..Default::default() }).await.unwrap_or_default();
    let course = q.get("course").filter(|c| courses.iter().any(|x| &x.id == *c)).cloned();
    let status = q.get("status").map(String::as_str).filter(|v| matches!(*v, "active" | "completed" | "cancelled" | "expired"));
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = vuuut_core::directory::search_terms(&query);
    let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).max(1);
    let (rows, total) = repo::browse_enrollments(&state.db, &repo::EnrollmentQuery { course_id: course.as_deref(), status, terms: &terms, limit: 50, offset: (page - 1) * 50, member_id: None }).await;
    let opt = |v: &str, l: &str, cur: Option<&str>| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), l = esc(l), s = if cur == Some(v) { " selected" } else { "" });
    let course_opts: String = std::iter::once(opt("", "All courses", course.as_deref().or(Some(""))))
        .chain(courses.iter().map(|c| opt(&c.id, &c.title, course.as_deref())))
        .collect();
    let status_opts: String = [("", "Any status"), ("active", "Learning"), ("completed", "Completed"), ("cancelled", "Cancelled"), ("expired", "Expired")]
        .iter()
        .map(|(v, l)| opt(v, l, status.or(Some(""))))
        .collect();
    let enrol_course_opts: String = courses.iter().filter(|c| c.status != "archived").map(|c| opt(&c.id, &c.title, course.as_deref())).collect();
    let pages = (total + 49) / 50;
    let pager = if pages > 1 {
        let link = |n: i64| {
            let mut p = vec![format!("page={n}")];
            if let Some(c) = &course { p.push(format!("course={c}")); }
            if let Some(s) = status { p.push(format!("status={s}")); }
            if !query.is_empty() { p.push(format!("q={}", vuuut_core::social_media::encode(&query))); }
            format!("{STUDENTS}?{}", p.join("&amp;"))
        };
        format!(
            r#"<nav class="ec-pager">{prev}<span class="muted">Page {page} of {pages}</span>{next}</nav>"#,
            prev = if page > 1 { format!(r#"<a class="btn text small" href="{}">← Previous</a>"#, link(page - 1)) } else { String::new() },
            next = if page < pages { format!(r#"<a class="btn text small" href="{}">Next →</a>"#, link(page + 1)) } else { String::new() },
        )
    } else {
        String::new()
    };
    page_for_role(
        actor.role,
        "Students",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="dr-split"><div>
<form class="dr-filter rise" style="--i:2" method="get" action="{STUDENTS}">
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{q}" placeholder=" " maxlength="100"><span>Name, email or course</span></label>
<label class="field fixed" style="margin:0"><select name="course">{course_opts}</select><span>Course</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="status">{status_opts}</select><span>Status</span>{caret}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>{pager}</div>
<div><div class="section">Enrol a student</div>
<form class="card padded rise" style="--i:3" method="post" action="{STUDENTS}enrol/">
<label class="field"><input type="email" name="email" required maxlength="191" placeholder=" "><span>Email address</span></label>
<label class="field"><input name="name" maxlength="80" placeholder=" "><span>Name (for a new member)</span></label>
<label class="field fixed"><select name="course_id" required>{enrol_course_opts}</select><span>Course</span>{caret}</label>
<label class="ec-check"><input type="checkbox" name="notify" value="1" checked> Email them a sign-in link</label>
<div style="display:flex;justify-content:flex-end;margin-top:12px"><button class="btn" type="submit">{add} Enrol</button></div>
<p class="muted ec-hint">Someone without an account becomes a free member. Staff enrolments ignore price, seats and enrolment dates.</p></form></div></div>"#,
            head = crate::tools::tool_head_back("lms", BASE, None, &format!(r#"<a class="btn text" href="{BASE}export.csv">{} Export CSV</a>"#, svg("import"))),
            tabs = tabs_for(&state, "students", &actor).await,
            banner = flash_banner(&q),
            q = esc(&query),
            caret = SELECT_CARET,
            search = svg("search"),
            count = esc(&countable(total, "enrolment")),
            body = if rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No enrolments match.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, rows.iter().map(enrollment_row).collect::<String>()) },
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct EnrolForm {
    email: String,
    name: String,
    course_id: String,
    notify: String,
}

pub(crate) async fn student_enrol(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<EnrolForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let email = f.email.trim().to_lowercase();
    if !vuuut_core::directory::is_email(&email) {
        return flash_to(STUDENTS, "error", "That email address does not look right.");
    }
    let Some(course) = repo::find_course(&state.db, f.course_id.trim()).await else {
        return flash_to(STUDENTS, "error", "Choose a course.");
    };
    let name = vuuut_core::directory::clean(&f.name, 80);
    let Some((member_id, created)) = vuuut_api::members::ensure_member_for_purchase(&state, &email, (!name.is_empty()).then_some(name.as_str())).await else {
        return flash_to(STUDENTS, "error", "That member could not be found or made.");
    };
    if let Err(e) = repo::enroll(&state.db, &course, &member_id, "staff", None).await {
        tracing::error!("lms: staff enrolment failed: {}", e.message());
        return flash_to(STUDENTS, "error", "The enrolment could not be saved.");
    }
    if f.notify == "1" {
        let s = lms_settings(&state);
        let to = s.paths.public_path(&format!("/courses/{}/", course.slug));
        if let Some(link) = vuuut_api::members::magic_link_for(&state, &email, if created { "signup" } else { "signin" }, &to).await {
            let (bg, mail) = (state.clone(), email.clone());
            tokio::spawn(async move {
                let r = if created { vuuut_api::members::send_signup_email(&bg, &mail, &link).await } else { vuuut_api::members::send_signin_email(&bg, &mail, &link).await };
                if let Err(e) = r {
                    tracing::warn!("lms: enrolment email to {mail} failed: {}", e.message());
                }
            });
        }
    }
    flash_to(&format!("{STUDENTS}{member_id}/"), "ok", &format!("{email} is enrolled in {}{}.", course.title, if created { " (a new member)" } else { "" }))
}

pub(crate) async fn student_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That student could not be found.");
    }
    let Some(m) = repo::member(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That student could not be found.");
    };
    let s = lms_settings(&state);
    let enrolments = repo::enrollments_for_member(&state.db, &id).await;
    let mut cards = String::new();
    for e in &enrolments {
        let Some(c) = repo::find_course(&state.db, &e.course_id).await else { continue };
        let lessons = repo::lessons(&state.db, &c.id).await;
        let done = repo::completed_lessons(&state.db, &e.id).await;
        let mut items = String::new();
        for l in lessons.iter().filter(|l| l.status == "published") {
            let finished = done.contains(&l.id);
            let extra = match l.kind() {
                rules::LessonKind::Quiz => {
                    let tries = repo::attempts(&state.db, &e.id, &l.id).await;
                    tries.iter().map(|a| a.score).max().map(|best| format!(" · best {best}% in {}", countable(tries.len() as i64, "attempt"))).unwrap_or_default()
                }
                rules::LessonKind::Assignment => repo::submission(&state.db, &e.id, &l.id).await.map(|sub| format!(" · {}", match sub.status.as_str() { "passed" => format!("passed, {}/{}", sub.grade, l.points), "failed" => "needs another go".into(), _ => "waiting for a grade".into() })).unwrap_or_default(),
                _ => String::new(),
            };
            items.push_str(&format!(
                r#"<li class="{cls}"><span class="lm-tick">{tick}</span>{t}<span class="muted">{extra}</span></li>"#,
                cls = if finished { "done" } else { "" },
                tick = if finished { "✓" } else { "" },
                t = esc(&l.title),
                extra = esc(&extra),
            ));
        }
        let (chip, label) = match e.status.as_str() {
            "completed" => ("published", "Completed"),
            "cancelled" => ("error", "Cancelled"),
            "expired" => ("draft", "Expired"),
            _ => ("scheduled", "Learning"),
        };
        let action = |act: &str, label: &str, confirm: &str, cls: &str| {
            format!(
                r#"<form method="post" action="{STUDENTS}enrolments/{eid}/"{conf}><input type="hidden" name="action" value="{act}"><button class="btn {cls} small" type="submit">{label}</button></form>"#,
                eid = esc(&e.id),
                conf = if confirm.is_empty() { String::new() } else { format!(r#" data-confirm="{}""#, esc(confirm)) },
            )
        };
        let cert = e.certificate_code.as_deref().filter(|_| s.certificates).map(|code| format!(r#" · <a href="{}" target="_blank" rel="noopener">Certificate {}</a>"#, public_href(&s, &format!("/courses/certificate/{code}/")), esc(code))).unwrap_or_default();
        cards.push_str(&format!(
            r#"<section class="card padded lm-enrol"><div class="lm-enrol-head"><div><h3><a href="{COURSES}{cid}/">{title}</a> <span class="chip {chip}">{label}</span></h3>
<p class="muted">Enrolled {when} ({source}){expires}{completed}{cert}</p></div><div class="lm-progress-cell lm-big">{bar}<b>{p}%</b></div></div>
<ol class="lm-checklist">{items}</ol>
<div class="lm-enrol-actions">{a1}{a2}{a3}{a4}</div></section>"#,
            cid = esc(&c.id),
            title = esc(&c.title),
            when = date(&e.enrolled_at),
            source = esc(&e.source),
            expires = e.expires_at.as_deref().map(|x| format!(" · access until {}", date(x))).unwrap_or_default(),
            completed = e.completed_at.as_deref().map(|x| format!(" · finished {}", date(x))).unwrap_or_default(),
            bar = progress_bar(e.progress),
            p = e.progress,
            a1 = if e.status == "cancelled" || e.status == "expired" { action("restore", "Restore access", "", "tonal") } else { action("cancel", "Cancel access", "Cancel this enrolment?|They lose access to the course. Their progress is kept.", "text") },
            a2 = if e.status != "completed" { action("complete", "Mark complete", "Mark the whole course complete?|Every lesson is ticked off and a certificate is issued if certificates are on.", "text") } else { String::new() },
            a3 = action("reset", "Reset progress", "Reset this student's progress?|Completed lessons, quiz tries, submissions and the certificate are cleared.", "text"),
            a4 = action("remove", "Remove", "Remove the enrolment?|It and everything recorded for it are deleted.", "text danger"),
        ));
    }
    let who = if m.name.is_empty() { m.email.clone() } else { m.name.clone() };
    page_for_role(
        actor.role,
        &who,
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="card padded lm-student rise" style="--i:2"><span class="lm-avatar lm-avatar-lg">{initial}</span><div><h2>{who}</h2><p class="muted"><a href="mailto:{email}">{email}</a> · {status} member · <a href="/ghost/members/{id}/">Member profile</a></p></div></div>
<div class="section">Courses</div><div class="rise" style="--i:3">{cards}</div>"#,
            head = crate::tools::tool_head_back("lms", STUDENTS, None, ""),
            tabs = tabs_for(&state, "students", &actor).await,
            banner = flash_banner(&q),
            initial = esc(&who.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
            who = esc(&who),
            email = esc(&m.email),
            status = esc(&m.status),
            id = esc(&m.id),
            cards = if cards.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Not enrolled in anything.</p></div>"#.to_string() } else { cards },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ActionForm {
    action: String,
}

pub(crate) async fn enrolment_action(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ActionForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(STUDENTS, "error", "That enrolment could not be found.");
    }
    let Some(e) = repo::find_enrollment(&state.db, &id).await else { return flash_to(STUDENTS, "error", "That enrolment could not be found.") };
    let back = format!("{STUDENTS}{}/", e.member_id);
    let s = lms_settings(&state);
    let result = match f.action.as_str() {
        "cancel" => repo::set_enrollment_status(&state.db, &id, "cancelled").await.map(|_| "Access cancelled. Progress is kept."),
        "restore" => {
            let status = if e.completed_at.is_some() { "completed" } else { "active" };
            repo::set_enrollment_status(&state.db, &id, status).await.map(|_| "Access restored.")
        }
        "reset" => repo::reset_enrollment(&state.db, &id).await.map(|_| "Progress reset."),
        "remove" => repo::delete_enrollment(&state.db, &id).await.map(|_| "Enrolment removed."),
        "complete" => {
            let Some(c) = repo::find_course(&state.db, &e.course_id).await else { return flash_to(&back, "error", "That course is gone.") };
            let ids: Vec<String> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").map(|l| l.id).collect();
            let mut cur = e.clone();
            let mut r = Ok(());
            for lid in &ids {
                match repo::complete_lesson(&state.db, &cur, lid, &ids, c.certificate != 0 && s.certificates).await {
                    Ok(next) => cur = next,
                    Err(err) => { r = Err(err); break; }
                }
            }
            r.map(|_| "Marked complete.")
        }
        _ => return flash_to(&back, "error", "That is not something an enrolment can do."),
    };
    match result {
        Ok(msg) => flash_to(&back, "ok", msg),
        Err(err) => {
            tracing::error!("lms: enrolment {id} {} failed: {}", f.action, err.message());
            flash_to(&back, "error", "That could not be saved.")
        }
    }
}
