//! The lesson editor — one screen for a lesson, a quiz or an assignment.
//! The content is written in Koenig like a post; a video, downloads,
//! release (drip) and the free-preview switch sit beside it. A quiz adds
//! the question builder and its rules (pass mark, tries, time limit,
//! shuffling, showing answers); an assignment adds its maximum grade.

use super::courses::{card, check};
use super::*;
use vuuut_core::lms::{LessonKind, QuestionKind};

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

pub(crate) async fn lesson_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That lesson could not be found.");
    }
    let Some(l) = repo::find_lesson(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That lesson could not be found.");
    };
    let Some(c) = repo::find_course(&state.db, &l.course_id).await else {
        return not_found_page_for(actor.role, &actor.user, "That lesson's course could not be found.");
    };
    let s = lms_settings(&state);
    let kind = l.kind();
    let sections = repo::sections(&state.db, &c.id).await;
    let all = repo::lessons(&state.db, &c.id).await;
    let pos = all.iter().position(|x| x.id == l.id);
    let prev = pos.and_then(|p| p.checked_sub(1)).and_then(|p| all.get(p));
    let next = pos.and_then(|p| all.get(p + 1));

    let initial = if !l.content_lexical.trim().is_empty() { Some(l.content_lexical.clone()) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(&l.content_html)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": match kind {
            LessonKind::Quiz => "Introduce the quiz — what it covers and what to revise first…",
            LessonKind::Assignment => "Set the assignment — what to make, how it will be graded, and what to hand in…",
            LessonKind::Lesson => "Write the lesson. Add images, code, callouts, toggles, embeds…",
        },
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false,
        "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null,
        "pinturaConfig": serde_json::Value::Null,
    });

    let video_preview = rules::video_embed(&l.video_url)
        .map(|(u, frame)| if frame { format!(r#"<div class="lm-video"><iframe src="{}" title="Video" loading="lazy" allowfullscreen></iframe></div>"#, esc(&u)) } else { format!(r#"<div class="lm-video"><video src="{}" controls preload="metadata"></video></div>"#, esc(&u)) })
        .unwrap_or_default();
    let video = format!(
        r#"{f}<p class="muted ec-hint">YouTube, Vimeo, Loom or Wistia links play in a privacy-friendly player; a link to an .mp4 or .webm file plays in the browser's own.</p>{video_preview}"#,
        f = field("video_url", "Video link", &l.video_url, "url", r#" maxlength="500" placeholder="https://" id="lm-video-url""#),
    );
    let atts = rules::attachments(&l.attachments);
    let att_row = |label: &str, url: &str| format!(
        r#"<div class="lm-att"><input class="ec-in" name="att_label" value="{l}" placeholder="Label — e.g. Slides (PDF)" maxlength="120" aria-label="Label"><input class="ec-in" name="att_url" value="{u}" placeholder="https://… or upload" maxlength="500" aria-label="Address"><button type="button" class="icon-btn lm-att-del" title="Remove">{del}</button></div>"#,
        l = esc(label), u = esc(url), del = svg("delete"),
    );
    let attachments = format!(
        r#"<div class="lm-atts" id="lm-atts">{rows}</div>
<div class="lm-att-tools"><button type="button" class="btn text small" id="lm-att-add">{add} Add a link</button><label class="btn tonal small"><input type="file" id="lm-att-file" hidden>{up} Upload a file</label></div>
<template id="lm-att-tpl">{blank}</template>
<p class="muted ec-hint">Worksheets, slides, starter files. Shown under the lesson to enrolled students.</p>"#,
        rows = atts.iter().map(|(a, b)| att_row(a, b)).collect::<String>(),
        add = svg("add"),
        up = svg("upload"),
        blank = att_row("", ""),
    );

    let quiz = if kind == LessonKind::Quiz {
        let qs = repo::questions(&state.db, &l.id).await;
        let json: Vec<serde_json::Value> = qs.iter().map(|q| serde_json::json!({
            "id": q.id, "kind": q.kind, "prompt": q.prompt, "options": q.options, "answer": q.answers.join("|"), "explanation": q.explanation, "points": q.points,
        })).collect();
        let kinds: Vec<serde_json::Value> = QuestionKind::ALL.iter().map(|k| serde_json::json!({ "key": k.key(), "label": k.label() })).collect();
        format!(
            "{questions}{rules}",
            questions = card("Questions", "check", "amber", &format!(
                r#"<div id="lm-quiz" data-questions="{data}" data-kinds="{kinds}"></div>
<input type="hidden" name="questions" id="lm-questions" value="{data}">
<noscript><p class="muted">The question builder needs JavaScript.</p></noscript>
<p class="muted ec-hint">Each question scores its points only when answered exactly right. Typed answers ignore capitals, accents and extra spaces; separate several accepted answers with “|”.</p>"#,
                data = esc(&serde_json::Value::Array(json).to_string()),
                kinds = esc(&serde_json::Value::Array(kinds).to_string()),
            ), "lm-questions-card"),
            rules = card("Quiz rules", "tune", "teal", &format!(
                r#"<div class="ec-grid3">{pass}{tries}{time}</div>{shuffle}{reveal}<p class="muted ec-hint">Passing the quiz completes it. Blank tries is unlimited; blank time is untimed.</p>"#,
                pass = field("pass_mark", "Pass mark %", &l.pass_mark.to_string(), "number", r#" min="0" max="100""#),
                tries = field("attempts", "Tries allowed", &if l.attempts > 0 { l.attempts.to_string() } else { String::new() }, "number", r#" min="0" max="100""#),
                time = field("time_limit", "Time limit (minutes)", &if l.time_limit > 0 { l.time_limit.to_string() } else { String::new() }, "number", r#" min="0" max="600""#),
                shuffle = check("shuffle", "1", "Shuffle the questions for each try", l.shuffle != 0),
                reveal = check("reveal", "1", "Show the right answers and explanations after a try", l.reveal != 0),
            ), "lm-quiz-rules"),
        )
    } else {
        String::new()
    };
    let assignment = if kind == LessonKind::Assignment {
        card("Grading", "edit", "pink", &format!(
            r#"{pts}<p class="muted ec-hint">Students hand in written work and/or a link. It counts as complete when you pass it in <a href="/ghost/lms/grading/">Grading</a>.</p>"#,
            pts = field("points", "Maximum grade (points)", &l.points.to_string(), "number", r#" min="1" max="1000""#),
        ), "lm-grading")
    } else {
        format!(r#"<input type="hidden" name="points" value="{}">"#, l.points)
    };

    let mut sec_opts = String::from(r#"<option value="">Not in a section</option>"#);
    for sec in &sections {
        sec_opts.push_str(&format!(r#"<option value="{id}"{sel}>{t}</option>"#, id = esc(&sec.id), sel = if l.section_id.as_deref() == Some(sec.id.as_str()) { " selected" } else { "" }, t = esc(&sec.title)));
    }
    let kinds: String = LessonKind::ALL.iter().map(|k| format!(
        r#"<label class="dr-kind"><input type="radio" name="kind" value="{key}"{c}><b>{label}</b></label>"#,
        key = k.key(), c = if *k == kind { " checked" } else { "" }, label = k.label(),
    )).collect();
    let view = if c.is_published() && s.enabled && l.status == "published" {
        format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View as a student</a>"#, public_href(&s, &format!("/courses/{}/learn/{}/", c.slug, l.slug)), svg("open"))
    } else {
        String::new()
    };
    let side = format!(
        "{status}{place}{release}{nav}",
        status = card("Status", "eye", "green", &format!(
            r#"<label class="field fixed"><select name="status"><option value="published"{p}>Published</option><option value="draft"{d}>Draft — hidden from students</option></select><span>Status</span>{SELECT_CARET}</label>{preview}{view}"#,
            p = if l.status != "draft" { " selected" } else { "" },
            d = if l.status == "draft" { " selected" } else { "" },
            preview = check("preview", "1", "Free preview — anyone can open it", l.is_preview()),
        ), "lm-l-status"),
        place = card("Place in the course", "list", "purple", &format!(
            r#"<div class="lm-kinds lm-kinds-3">{kinds}</div><label class="field fixed"><select name="section_id">{sec_opts}</select><span>Section</span>{SELECT_CARET}</label>{slug}"#,
            slug = field("slug", "Address", &l.slug, "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#),
        ), "lm-l-place"),
        release = card("Timing & release", "history", "blue", &format!(
            r#"<div class="ec-grid2">{dur}{drip}</div><p class="muted ec-hint">A dripped lesson opens that many days after each student enrols. {seq}</p>"#,
            dur = field("duration_minutes", "Length (minutes)", &if l.duration_minutes > 0 { l.duration_minutes.to_string() } else { String::new() }, "number", r#" min="0" max="10000""#),
            drip = field("drip_days", "Opens on day", &if l.drip_days > 0 { l.drip_days.to_string() } else { String::new() }, "number", r#" min="0" max="3650""#),
            seq = if c.sequential != 0 { "This course also unlocks lessons in order." } else { "" },
        ), "lm-l-release"),
        nav = format!(
            r#"<section class="card padded ec-card"><div class="ec-side-actions lm-l-nav">{prev}{next}<button class="btn text danger" type="submit" form="lm-l-delete">{del} Delete {what}</button></div></section>"#,
            prev = prev.map(|p| format!(r#"<a class="btn text small" href="{LESSONS}{}/">← {}</a>"#, esc(&p.id), esc(&p.title))).unwrap_or_default(),
            next = next.map(|n| format!(r#"<a class="btn text small" href="{LESSONS}{}/">{} →</a>"#, esc(&n.id), esc(&n.title))).unwrap_or_default(),
            del = svg("delete"),
            what = kind.label().to_lowercase(),
        ),
    );
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{summary}
<div class="ec-sublabel">{content_label}</div>
<div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="content_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="content_plain" rows="10" placeholder=" " readonly>{plain}</textarea><span>Content (the editor could not start)</span></label></div>
</section>{quiz}{assignment}{video}{atts}"#,
        title = field("title", "Title", &l.title, "text", r#" required maxlength="160" id="ec-title""#),
        summary = field("summary", "Summary — a line shown in the curriculum", &l.summary, "text", r#" maxlength="300""#),
        content_label = match kind { LessonKind::Quiz => "Introduction", LessonKind::Assignment => "Instructions", LessonKind::Lesson => "Content" },
        config = esc(&config.to_string()),
        plain = esc(&vuuut_render::markdown::html_to_plaintext(&l.content_html)),
        video = if kind == LessonKind::Quiz { String::new() } else { card("Video", "play", "red", &video, "lm-l-video") },
        atts = card("Downloads", "file", "slate", &attachments, "lm-l-atts"),
    );
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{COURSES}{cid}/#lm-curriculum-card" title="Back to the course">{back}</a>
<div><h1>{h}</h1><p class="sub"><a href="{COURSES}{cid}/">{course}</a> · {kind}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="lm-lesson">{check} Save</button></div></header>
{banner}
<form method="post" action="{LESSONS}{id}/delete/" id="lm-l-delete" data-confirm="Delete “{t}”|Students' progress, tries and submissions for it are deleted too."></form>
<form id="lm-lesson" class="ec-edit lm-edit lm-kind-{kk}" method="post" action="{LESSONS}{id}/">
<div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script>
<script src="{ASSETS}/react-jsx.js" defer></script>
<script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script>
<script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script>
<script src="{ASSETS}/lms-editor.js" defer></script>"#,
        cid = esc(&c.id),
        back = svg("chevron"),
        h = esc(&l.title),
        course = esc(&c.title),
        kind = kind.label(),
        check = svg("check"),
        banner = flash_banner(&q),
        id = esc(&l.id),
        t = esc(&l.title),
        kk = kind.key(),
    );
    page_for_role(actor.role, &l.title, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

pub(crate) async fn lesson_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(COURSES, "error", "That lesson could not be found.");
    }
    let Some(l) = repo::find_lesson(&state.db, &id).await else { return flash_to(COURSES, "error", "That lesson could not be found.") };
    let back = format!("{LESSONS}{id}/");
    let sections = repo::sections(&state.db, &l.course_id).await;
    let mut input = match rules::LessonInput::from_form(&form, |sid| sections.iter().any(|s| s.id == sid)) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    // A form without the editor keeps the content it had.
    let lexical = form.iter().find(|(k, _)| k == "content_lexical").map(|(_, v)| v.clone()).unwrap_or_default();
    if lexical.trim().starts_with('{') {
        let lexical = vuuut_core::url::unbase_assets(&lexical);
        let html = vuuut_render::lexical::to_html(&lexical);
        if lexical.len() > rules::MAX_RICH || html.len() > rules::MAX_RICH {
            return flash_to(&back, "error", "The content is too long.");
        }
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        let empty = plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe") && !html.contains("<figure");
        input.content_html = if empty { String::new() } else { html };
        input.content_lexical = if empty { String::new() } else { lexical };
    } else {
        input.content_html = l.content_html.clone();
        input.content_lexical = l.content_lexical.clone();
    }
    let questions = if input.kind == "quiz" {
        match form.iter().find(|(k, _)| k == "questions") {
            Some((_, raw)) => match rules::Question::list_from_json(raw) {
                Ok(q) => Some(q),
                Err(m) => return flash_to(&back, "error", &m),
            },
            None => None,
        }
    } else {
        None
    };
    if let Err(e) = repo::update_lesson(&state.db, &id, &l.course_id, &input).await {
        tracing::error!("lms: saving lesson {id} failed: {}", e.message());
        return flash_to(&back, "error", "The lesson could not be saved.");
    }
    if let Some(qs) = questions {
        if let Err(e) = repo::save_questions(&state.db, &id, &qs).await {
            tracing::error!("lms: saving questions of {id} failed: {}", e.message());
            return flash_to(&back, "error", "The lesson was saved, but not its questions.");
        }
    }
    let note = if input.kind == "quiz" && repo::questions(&state.db, &id).await.is_empty() { "Saved. Add some questions — a quiz without any cannot be passed." } else { "Saved." };
    flash_to(&back, "ok", note)
}

pub(crate) async fn lesson_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(COURSES, "error", "That lesson could not be found.");
    }
    let Some(l) = repo::find_lesson(&state.db, &id).await else { return flash_to(COURSES, "error", "That lesson could not be found.") };
    match repo::delete_lesson(&state.db, &id).await {
        Ok(()) => flash_to(&format!("{COURSES}{}/", l.course_id), "ok", &format!("“{}” deleted.", l.title)),
        Err(_) => flash_to(&format!("{LESSONS}{id}/"), "error", "That could not be deleted."),
    }
}
