//! Grading: assignments handed in, each with what the student wrote, their
//! link, and a form to pass or return it with a grade and feedback; and the
//! latest quiz results. Passing an assignment completes it for the student.

use super::*;

pub(crate) async fn grading_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let show = q.get("show").map(String::as_str).filter(|v| matches!(*v, "submitted" | "passed" | "failed" | "all")).unwrap_or("submitted");
    let subs = repo::submissions(&state.db, (show != "all").then_some(show), 100).await;
    let attempts = repo::recent_attempts(&state.db, 25).await;
    let sub_cards: String = subs
        .iter()
        .map(|s| {
            let who = if s.name.is_empty() { s.email.clone() } else { s.name.clone() };
            let (chip, label) = match s.status.as_str() { "passed" => ("published", "Passed"), "failed" => ("error", "Returned"), _ => ("scheduled", "Waiting") };
            format!(
                r#"<section class="card padded lm-sub"><div class="lm-sub-head"><span class="lm-avatar">{initial}</span><div><b><a href="{STUDENTS}{mid}/">{who}</a></b> <span class="chip {chip}">{label}</span>
<p class="muted">{lesson} · {course} · handed in {when}</p></div></div>
{body}{link}
<form method="post" action="/ghost/lms/grading/{id}/" class="lm-grade"><div class="lm-grade-row">
<label class="field" style="margin:0;max-width:150px"><input type="number" name="grade" min="0" max="{max}" value="{grade}" placeholder=" "><span>Grade / {max}</span></label>
<label class="field" style="margin:0;flex:1"><input name="feedback" value="{fb}" maxlength="2000" placeholder=" "><span>Feedback for the student</span></label></div>
<div class="lm-grade-actions"><button class="btn tonal small" name="status" value="failed" type="submit">Return for another go</button><button class="btn small" name="status" value="passed" type="submit">{ok} Pass</button></div></form></section>"#,
                initial = esc(&who.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
                mid = esc(&s.member_id),
                who = esc(&who),
                lesson = esc(&s.lesson_title),
                course = esc(&s.course_title),
                when = date(&s.submitted_at),
                body = if s.body.is_empty() { String::new() } else { format!(r#"<div class="lm-sub-body">{}</div>"#, vuuut_core::directory::paragraphs(&s.body)) },
                link = if s.link.is_empty() { String::new() } else { format!(r#"<p><a href="{u}" target="_blank" rel="noopener nofollow">{u}</a></p>"#, u = esc(&s.link)) },
                id = esc(&s.id),
                max = s.points,
                grade = if s.status == "submitted" { String::new() } else { s.grade.to_string() },
                fb = esc(&s.feedback),
                ok = svg("check"),
            )
        })
        .collect();
    let filter = [("submitted", "Waiting"), ("passed", "Passed"), ("failed", "Returned"), ("all", "All")]
        .iter()
        .map(|(k, l)| format!(r#"<a class="chip{on}" href="/ghost/lms/grading/?show={k}">{l}</a>"#, on = if *k == show { " tonal" } else { "" }))
        .collect::<String>();
    let attempt_rows: String = attempts
        .iter()
        .map(|a| {
            let who = if a.name.is_empty() { a.email.clone() } else { a.name.clone() };
            format!(
                r#"<div class="row dr-listing"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title"><a href="{STUDENTS}{mid}/">{who}</a> · {score}%</span>
<span class="detail">{lesson} · {course} · {when}</span></div><div class="trailing"><span class="chip {chip}">{label}</span></div></div>"#,
                tint = if a.passed != 0 { "green" } else { "red" },
                ic = svg(if a.passed != 0 { "check" } else { "close" }),
                mid = esc(&a.member_id),
                who = esc(&who),
                score = a.score,
                lesson = esc(&a.lesson_title),
                course = esc(&a.course_title),
                when = date(&a.submitted_at),
                chip = if a.passed != 0 { "published" } else { "error" },
                label = if a.passed != 0 { "Passed" } else { "Not yet" },
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Grading",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="dr-split"><div><div class="section">Assignments</div><div class="lm-chips rise" style="--i:2">{filter}</div>
<div class="rise" style="--i:3">{subs}</div></div>
<div><div class="section">Latest quiz results</div><div class="rise" style="--i:3">{attempts}</div></div></div>"#,
            head = crate::tools::tool_head_back("lms", BASE, None, ""),
            tabs = tabs_for(&state, "grading", &actor).await,
            banner = flash_banner(&q),
            subs = if sub_cards.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing here. Handed-in assignments wait here for a grade.</p></div>"#.to_string() } else { sub_cards },
            attempts = if attempt_rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No quiz tries yet.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{attempt_rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct GradeForm {
    status: String,
    grade: String,
    feedback: String,
}

pub(crate) async fn grade_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<GradeForm>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    const BACK: &str = "/ghost/lms/grading/";
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(BACK, "error", "That submission could not be found.");
    }
    let Some(sub) = repo::find_submission(&state.db, &id).await else { return flash_to(BACK, "error", "That submission could not be found.") };
    let Some(lesson) = repo::find_lesson(&state.db, &sub.lesson_id).await else { return flash_to(BACK, "error", "That assignment is gone.") };
    let status = if f.status == "passed" { "passed" } else { "failed" };
    let grade = f.grade.trim().parse::<i64>().unwrap_or(if status == "passed" { lesson.points } else { 0 }).clamp(0, lesson.points.max(1));
    let feedback = vuuut_core::directory::clean_text(&f.feedback, 2000);
    if let Err(e) = repo::grade_submission(&state.db, &id, status, grade, &feedback, &actor.user.id).await {
        tracing::error!("lms: grading {id} failed: {}", e.message());
        return flash_to(BACK, "error", "The grade could not be saved.");
    }
    if let Some(e) = repo::find_enrollment(&state.db, &sub.enrollment_id).await {
        if let Some(c) = repo::find_course(&state.db, &e.course_id).await {
            let s = lms_settings(&state);
            let ids: Vec<String> = repo::lessons(&state.db, &c.id).await.into_iter().filter(|l| l.status == "published").map(|l| l.id).collect();
            let r = if status == "passed" {
                repo::complete_lesson(&state.db, &e, &lesson.id, &ids, c.certificate != 0 && s.certificates).await
            } else {
                repo::uncomplete_lesson(&state.db, &e, &lesson.id, &ids).await
            };
            if let Err(err) = r {
                tracing::error!("lms: updating progress after grading {id} failed: {}", err.message());
            }
        }
    }
    flash_to(BACK, "ok", if status == "passed" { "Passed. It counts as complete for the student." } else { "Returned. The student can hand it in again." })
}
