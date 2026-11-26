//! The inbox — guest applications, listener questions, feedback and
//! sponsorship enquiries sent from the site — and listener reviews the team
//! chooses to show on the show's page.
//!
//! Requests hold a visitor's name and email address. They are shown to
//! staff only and can be deleted outright.

use super::episodes::{field, select, textarea};
use super::*;

const REQUESTS: &str = "/ghost/podcast/requests/";
const REVIEWS: &str = "/ghost/podcast/reviews/";

pub(crate) async fn requests_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let tz = zone(&state);
    let status = q.get("status").map(|v| rules::RequestStatus::from_key(v).key()).filter(|_| q.get("status").is_some_and(|v| !v.is_empty()));
    let kind = q.get("kind").map(|v| rules::RequestKind::from_key(v).key()).filter(|_| q.get("kind").is_some_and(|v| !v.is_empty()));
    let list = repo::requests(&state.db, status, kind, 300).await;
    let shows = repo::shows(&state.db, false).await;
    let statuses = rules::RequestStatus::options();
    let rows: String = list.iter().map(|r| {
        let when = vuuut_core::dates::from_db_string(&r.created_at).map(|d| d.with_timezone(&tz).format("%-d %b %Y, %H:%M").to_string()).unwrap_or_default();
        let opts: String = statuses.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), l = esc(l), s = if *v == r.status { " selected" } else { "" })).collect();
        format!(
            r#"<details class="dr-cat pc-request" id="r-{id}"{open}><summary><span class="ic tint-{tint}">{ic}</span><b>{n}</b><span class="muted">{k}{topic}</span><span class="chip {chip}">{st}</span>{chev}</summary>
<div class="pc-request-body"><p class="muted">{email}{show} · {when}</p><blockquote>{msg}</blockquote>{link}
<form method="post" action="{REQUESTS}{id}/" class="dr-cat-form"><div class="ec-grid2"><label class="field fixed"><select name="status">{opts}</select><span>Status</span>{caret}</label>{reply}</div>
{note}<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">Delete</button><button class="btn small" type="submit">Save</button></div></form>
<form method="post" action="{REQUESTS}{id}/delete/" id="del-{id}" data-confirm="Delete this message?|It cannot be brought back."></form></div></details>"#,
            id = esc(&r.id),
            open = if r.status == "new" { " open" } else { "" },
            tint = match r.kind.as_str() { "guest" => "purple", "sponsor" => "green", "feedback" => "amber", _ => "blue" },
            ic = svg(match r.kind.as_str() { "guest" => "mic", "sponsor" => "campaign", _ => "comment" }),
            n = esc(&r.name),
            k = esc(rules::RequestKind::from_key(&r.kind).label()),
            topic = if r.topic.is_empty() { String::new() } else { format!(" · {}", esc(&r.topic)) },
            chip = match r.status.as_str() { "new" => "scheduled", "accepted" | "answered" => "published", "declined" => "error", _ => "draft" },
            st = esc(rules::RequestStatus::from_key(&r.status).label()),
            chev = svg("chevron"),
            email = if vuuut_core::directory::is_email(&r.email) { format!(r#"<a href="mailto:{0}">{0}</a>"#, esc(&r.email)) } else { esc(&r.email) },
            show = r.show_id.as_deref().and_then(|id| shows.iter().find(|s| s.id == id)).map(|s| format!(" · {}", esc(&s.title))).unwrap_or_default(),
            when = esc(&when),
            msg = esc(&r.message),
            link = if r.link.is_empty() { String::new() } else { format!(r#"<p><a href="{0}" target="_blank" rel="noopener nofollow">{0}</a></p>"#, esc(&r.link)) },
            caret = SELECT_CARET,
            reply = if vuuut_core::directory::is_email(&r.email) { format!(r#"<a class="btn tonal small" style="align-self:center" href="mailto:{}?subject={}">{} Reply</a>"#, esc(&r.email), esc(&vuuut_core::social_media::encode(&format!("Re: {}", if r.topic.is_empty() { "your message" } else { &r.topic }))), svg("mail")) } else { String::new() },
            note = textarea("note", "Private note", &r.note, 2, r#" maxlength="4000""#),
        )
    }).collect();
    let kind_opts: Vec<(String, String)> = std::iter::once((String::new(), "Everything".to_string())).chain(rules::RequestKind::options()).collect();
    let status_opts: Vec<(String, String)> = std::iter::once((String::new(), "Any status".to_string())).chain(rules::RequestStatus::options()).collect();
    let s = podcast_settings(&state);
    page_for_role(
        actor.role,
        "Podcast inbox",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="pc-filters rise" style="--i:2" method="get">{k}{st}<button class="btn tonal" type="submit">Filter</button></form>
<p class="muted dr-lead">{lead}</p><div class="card padded rise" style="--i:3">{rows}</div>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, ""),
            tabs = tabs_for(&state, "requests", &actor).await,
            banner = flash_banner(&q),
            k = select("kind", "Kind", &kind_opts, kind.unwrap_or("")),
            st = select("status", "Status", &status_opts, status.unwrap_or("")),
            lead = if s.requests { "Sent from the “Be on the show” form on each show's page." } else { "The public form is switched off in Settings." },
            rows = if rows.is_empty() { r#"<p class="muted" style="margin:0">Nothing here.</p>"#.to_string() } else { rows },
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RequestForm {
    status: String,
    note: String,
}

pub(crate) async fn request_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<RequestForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if repo::find_request(&state.db, &id).await.is_none() {
        return flash_to(REQUESTS, "error", "That message could not be found.");
    }
    let status = rules::RequestStatus::from_key(&f.status).key();
    match repo::update_request(&state.db, &id, status, &vuuut_core::directory::clean_text(&f.note, 4000)).await {
        Ok(()) => flash_to(REQUESTS, "ok", "Saved."),
        Err(_) => flash_to(REQUESTS, "error", "That could not be saved."),
    }
}

pub(crate) async fn request_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_request(&state.db, &id).await {
        Ok(()) => flash_to(REQUESTS, "ok", "Deleted."),
        Err(_) => flash_to(REQUESTS, "error", "That could not be deleted."),
    }
}

// ---------------------------------------------------------------------------
// reviews
// ---------------------------------------------------------------------------

pub(crate) async fn reviews_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let shows = repo::shows(&state.db, false).await;
    let list = repo::reviews(&state.db, None).await;
    let show_opts: Vec<(String, String)> = shows.iter().map(|s| (s.id.clone(), s.title.clone())).collect();
    let src_opts: Vec<(String, String)> = rules::REVIEW_SOURCES.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
    let rows: String = list.iter().map(|r| format!(
        r#"<div class="row"><span class="ic tint-amber">{star}</span><div class="body"><span class="title">{stars} {t}</span><span class="detail">“{b}” — {a} · {src}{show}{feat}</span></div>
<div class="trailing"><form method="post" action="{REVIEWS}{id}/delete/" data-confirm="Delete this review?|It comes off the show's page."><button class="icon-btn" type="submit" title="Delete">{del}</button></form></div></div>"#,
        star = svg("star"),
        stars = "★".repeat(r.rating.clamp(0, 5) as usize),
        t = esc(&r.title),
        b = esc(&r.body.chars().take(160).collect::<String>()),
        a = esc(if r.author.is_empty() { "A listener" } else { &r.author }),
        src = esc(rules::REVIEW_SOURCES.iter().find(|(k, _)| *k == r.source).map(|(_, l)| *l).unwrap_or("")),
        show = shows.iter().find(|s| s.id == r.show_id).map(|s| format!(" · {}", esc(&s.title))).unwrap_or_default(),
        feat = if r.featured != 0 { " · featured" } else { "" },
        id = esc(&r.id),
        del = svg("delete"),
    )).collect();
    page_for_role(
        actor.role,
        "Podcast reviews",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">Paste the reviews you're proudest of — from Apple Podcasts, Spotify, email or social media. They show on the show's page.</p>
<div class="dr-split dr-split-even"><div class="card rows rise" style="--i:3">{rows}</div>
<form class="card padded rise" style="--i:3" method="post" action="{REVIEWS}"><h3 style="margin-top:0">Add a review</h3>{show}<div class="ec-grid2">{author}{rating}</div>{title}{body}{src}
<label class="ec-check"><input type="checkbox" name="featured" value="1" checked> Show it first</label><div style="display:flex;justify-content:flex-end;margin-top:12px"><button class="btn" type="submit">{add} Add review</button></div></form></div>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, ""),
            tabs = tabs_for(&state, "reviews", &actor).await,
            banner = flash_banner(&q),
            rows = if rows.is_empty() { r#"<div class="row"><div class="body"><span class="detail">No reviews yet.</span></div></div>"#.to_string() } else { rows },
            show = if show_opts.is_empty() { r#"<p class="muted">Create a show first.</p>"#.to_string() } else { select("show_id", "Show", &show_opts, "") },
            author = field("author", "Who wrote it", "", "text", r#" maxlength="120""#),
            rating = select("rating", "Stars", &[("5", "★★★★★"), ("4", "★★★★"), ("3", "★★★"), ("0", "No rating")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>(), "5"),
            title = field("title", "Headline (optional)", "", "text", r#" maxlength="160""#),
            body = textarea("body", "The review", "", 4, r#" required maxlength="2000""#),
            src = select("source", "Where it came from", &src_opts, "apple"),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn review_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let show_id = form.iter().find(|(k, _)| k == "show_id").map(|(_, v)| v.clone()).unwrap_or_default();
    if repo::find_show(&state.db, &show_id).await.is_none() {
        return flash_to(REVIEWS, "error", "Choose the show the review is for.");
    }
    let input = match rules::ReviewInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(REVIEWS, "error", &m),
    };
    match repo::save_review(&state.db, None, &show_id, &input).await {
        Ok(_) => flash_to(REVIEWS, "ok", "Review added."),
        Err(_) => flash_to(REVIEWS, "error", "That could not be saved."),
    }
}

pub(crate) async fn review_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_review(&state.db, &id).await {
        Ok(()) => flash_to(REVIEWS, "ok", "Review deleted."),
        Err(_) => flash_to(REVIEWS, "error", "That could not be deleted."),
    }
}
