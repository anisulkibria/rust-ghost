//! Reviews: every student review, newest first, with Approve (when reviews
//! wait for approval), Hide and Delete.

use super::*;

pub(crate) async fn reviews_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = lms_settings(&state);
    let show = q.get("show").map(String::as_str).filter(|v| matches!(*v, "published" | "pending" | "hidden")).map(str::to_string);
    let list = repo::all_reviews(&state.db, show.as_deref(), 200).await;
    let filter = [("", "All"), ("pending", "Waiting"), ("published", "Published"), ("hidden", "Hidden")]
        .iter()
        .map(|(k, l)| format!(r#"<a class="chip{on}" href="/ghost/lms/reviews/{qs}">{l}</a>"#, on = if show.as_deref().unwrap_or("") == *k { " tonal" } else { "" }, qs = if k.is_empty() { String::new() } else { format!("?show={k}") }))
        .collect::<String>();
    let rows: String = list
        .iter()
        .map(|r| {
            let who = if r.name.is_empty() { r.email.clone() } else { r.name.clone() };
            let (chip, label) = match r.status.as_str() { "pending" => ("scheduled", "Waiting"), "hidden" => ("draft", "Hidden"), _ => ("published", "Published") };
            let act = |a: &str, l: &str, cls: &str| format!(r#"<form method="post" action="/ghost/lms/reviews/{id}/"><input type="hidden" name="action" value="{a}"><button class="btn {cls} small" type="submit">{l}</button></form>"#, id = esc(&r.id));
            format!(
                r#"<section class="card padded lm-review"><div class="lm-review-head"><span class="lm-stars" aria-label="{n} of 5">{stars}</span><b>{title}</b> <span class="chip {chip}">{label}</span></div>
<p class="lm-review-body">{body}</p><p class="muted">{who} · <a href="{COURSES}{cid}/">{course}</a> · {when}</p>
<div class="lm-enrol-actions">{a}{b}{c}</div></section>"#,
                n = r.rating,
                stars = "★".repeat(r.rating.clamp(0, 5) as usize) + "☆".repeat((5 - r.rating.clamp(0, 5)) as usize).as_str(),
                title = esc(&r.title),
                body = esc(&r.body),
                who = esc(&who),
                cid = esc(&r.course_id),
                course = esc(&r.course_title),
                when = date(&r.created_at),
                a = if r.status != "published" { act("publish", "Approve", "tonal") } else { String::new() },
                b = if r.status != "hidden" { act("hide", "Hide", "text") } else { String::new() },
                c = act("delete", "Delete", "text danger"),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Reviews",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<p class="muted rise" style="--i:2;margin:14px 4px 6px">{state}</p><div class="lm-chips rise" style="--i:2">{filter}</div>
<div class="lm-reviews rise" style="--i:3">{rows}</div>"#,
            head = crate::tools::tool_head_back("lms", BASE, None, ""),
            tabs = tabs_for(&state, "reviews", &actor).await,
            banner = flash_banner(&q),
            state = if !s.reviews { "Reviews are switched off in Settings; existing ones are not shown on the site." } else if s.moderate_reviews { "New reviews wait here until somebody approves them." } else { "Reviews from enrolled students go live at once. Hide any that should not be shown." },
            rows = if rows.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No reviews yet.</p></div>"#.to_string() } else { rows },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ReviewAction {
    action: String,
}

pub(crate) async fn review_action(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<ReviewAction>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    const BACK: &str = "/ghost/lms/reviews/";
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(BACK, "error", "That review could not be found.");
    }
    let r = match f.action.as_str() {
        "publish" => repo::set_review_status(&state.db, &id, "published").await.map(|_| "Review published."),
        "hide" => repo::set_review_status(&state.db, &id, "hidden").await.map(|_| "Review hidden."),
        "delete" => repo::delete_review(&state.db, &id).await.map(|_| "Review deleted."),
        _ => return flash_to(BACK, "error", "That is not something a review can do."),
    };
    match r {
        Ok(m) => flash_to(BACK, "ok", m),
        Err(_) => flash_to(BACK, "error", "That could not be saved."),
    }
}
