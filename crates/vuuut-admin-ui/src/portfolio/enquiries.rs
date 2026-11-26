//! Enquiries: the contact / project-brief inbox, a pipeline from new to won
//! or lost, private notes, and a CSV export (formulas defused).
//!
//! Enquiries are visitors' personal details. Staff only.

use super::*;
use crate::podcast::{select, textarea};

const ENQUIRIES: &str = "/ghost/portfolio/enquiries/";

fn chip(status: &str) -> (&'static str, &'static str) {
    match rules::EnquiryStatus::from_key(status) {
        rules::EnquiryStatus::New => ("scheduled", "New"),
        rules::EnquiryStatus::Replied => ("tonal", "Replied"),
        rules::EnquiryStatus::Proposal => ("tonal", "Proposal sent"),
        rules::EnquiryStatus::Won => ("published", "Won"),
        rules::EnquiryStatus::Lost => ("error", "Lost"),
        rules::EnquiryStatus::Archived => ("draft", "Archived"),
        rules::EnquiryStatus::Spam => ("draft", "Spam"),
    }
}

pub(super) fn row(e: &repo::Enquiry, tz: chrono_tz::Tz) -> String {
    let (c, l) = chip(&e.status);
    let when = vuuut_core::dates::from_db_string(&e.created_at).map(|d| d.with_timezone(&tz).format("%-d %b").to_string()).unwrap_or_default();
    format!(
        r#"<a class="row dr-listing" href="{ENQUIRIES}{id}/"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{n}{co} <span class="chip {c}">{l}</span></span><span class="detail">{when}{budget}{svc} · {msg}</span></div></a>"#,
        id = esc(&e.id),
        tint = if e.status == "new" { "amber" } else { "slate" },
        ic = svg("mail"),
        n = esc(&e.name),
        co = if e.company.is_empty() { String::new() } else { format!(" · {}", esc(&e.company)) },
        when = esc(&when),
        budget = if e.budget.is_empty() { String::new() } else { format!(" · {}", esc(&e.budget)) },
        svc = if e.services.is_empty() { String::new() } else { format!(" · {}", esc(&e.services)) },
        msg = esc(&e.message.chars().take(90).collect::<String>()),
    )
}

pub(crate) async fn enquiries_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let tz = state.settings.get_str("timezone").unwrap_or_default().trim_matches('"').parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    let status = q.get("status").map(String::as_str).filter(|v| rules::EnquiryStatus::ALL.iter().any(|x| x.key() == *v));
    let list = repo::enquiries(&state.db, status, 500).await;
    let counts = repo::enquiry_counts(&state.db).await;
    let seg = |key: Option<&str>, label: &str| {
        let n = match key { Some(k) => counts.get(k).copied().unwrap_or(0), None => counts.iter().filter(|(k, _)| k.as_str() != "spam").map(|(_, v)| v).sum() };
        format!(r#"<a class="seg{on}" href="{ENQUIRIES}{qs}">{label} <span class="muted">{n}</span></a>"#, on = if status == key { " on" } else { "" }, qs = key.map(|k| format!("?status={k}")).unwrap_or_default(), label = esc(label))
    };
    let s = portfolio_settings(&state);
    page_for_role(
        actor.role,
        "Enquiries",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<div class="seg-bar rise" style="--i:2;margin-top:12px">{all}{new}{replied}{proposal}{won}{lost}{archived}{spam}</div>
<p class="muted dr-lead">{lead}</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, &format!(r#"<a class="btn text" href="{ENQUIRIES}export.csv">{} Export CSV</a>"#, svg("import"))),
            tabs = tabs_for(&state, "enquiries", &actor).await,
            banner = flash_banner(&q),
            all = seg(None, "All"), new = seg(Some("new"), "New"), replied = seg(Some("replied"), "Replied"), proposal = seg(Some("proposal"), "Proposal"),
            won = seg(Some("won"), "Won"), lost = seg(Some("lost"), "Lost"), archived = seg(Some("archived"), "Archived"), spam = seg(Some("spam"), "Spam"),
            lead = if s.enquiries { format!("From the contact form at {}. New ones are emailed to {}.", esc(&s.paths.public_path("/portfolio/contact/")), esc(if !s.notify_email.is_empty() { &s.notify_email } else if !s.email.is_empty() { &s.email } else { "nobody yet — set an address in Settings" })) } else { "The contact form is switched off in Settings.".to_string() },
            body = if list.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing here.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, list.iter().map(|e| row(e, tz)).collect::<String>()) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn enquiry_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(e) = repo::find_enquiry(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That enquiry could not be found.") };
    let tz = state.settings.get_str("timezone").unwrap_or_default().trim_matches('"').parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    let when = vuuut_core::dates::from_db_string(&e.created_at).map(|d| d.with_timezone(&tz).format("%A %-d %B %Y, %H:%M").to_string()).unwrap_or_default();
    let facts: String = [("Email", if vuuut_core::directory::is_email(&e.email) { format!(r#"<a href="mailto:{0}">{0}</a>"#, esc(&e.email)) } else { esc(&e.email) }), ("Company", esc(&e.company)), ("Phone", esc(&e.phone)), ("Budget", esc(&e.budget)), ("Timeline", esc(&e.timeline)), ("Services", esc(&e.services)), ("About", esc(&e.project)), ("Received", esc(&when))]
        .iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!(r#"<div class="pf-fact"><span class="muted">{k}</span><span>{v}</span></div>"#)).collect();
    let (c, l) = chip(&e.status);
    page_for_role(
        actor.role,
        &e.name,
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}<div class="dr-split"><div class="rise" style="--i:2"><div class="card padded"><h2 style="margin-top:0">{n} <span class="chip {c}">{l}</span></h2><div class="pf-facts">{facts}</div><div class="pf-message">{msg}</div>
{reply}</div></div>
<form class="card padded rise" style="--i:3" method="post" action="{ENQUIRIES}{id}/"><h3 style="margin-top:0">Pipeline</h3>{status}{note}<div style="display:flex;justify-content:space-between;gap:8px"><button class="btn text danger" type="submit" form="pf-delete">Delete</button><button class="btn" type="submit">Save</button></div></form></div>
<form method="post" action="{ENQUIRIES}{id}/delete/" id="pf-delete" data-confirm="Delete this enquiry?|It cannot be brought back."></form>"#,
            head = crate::tools::tool_head_back("portfolio", ENQUIRIES, None, ""),
            banner = flash_banner(&q),
            n = esc(&e.name),
            msg = vuuut_core::directory::paragraphs(&e.message),
            reply = if vuuut_core::directory::is_email(&e.email) { format!(r#"<a class="btn" href="mailto:{}?subject={}">{} Reply by email</a>"#, esc(&e.email), esc(&vuuut_core::social_media::encode(&format!("Re: your enquiry{}", if e.project.is_empty() { String::new() } else { format!(" about {}", e.project) }))), svg("mail")) } else { String::new() },
            id = esc(&e.id),
            status = select("status", "Status", &rules::EnquiryStatus::options(), &e.status),
            note = textarea("note", "Private notes", &e.note, 6, r#" maxlength="6000""#),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct EnquiryForm {
    status: String,
    note: String,
}

pub(crate) async fn enquiry_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<EnquiryForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{ENQUIRIES}{id}/");
    if repo::find_enquiry(&state.db, &id).await.is_none() {
        return flash_to(ENQUIRIES, "error", "That enquiry could not be found.");
    }
    match repo::update_enquiry(&state.db, &id, rules::EnquiryStatus::from_key(&f.status).key(), &vuuut_core::directory::clean_text(&f.note, 6000)).await {
        Ok(()) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn enquiry_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_enquiry(&state.db, &id).await {
        Ok(()) => flash_to(ENQUIRIES, "ok", "Deleted."),
        Err(_) => flash_to(ENQUIRIES, "error", "That could not be deleted."),
    }
}

pub(crate) async fn enquiries_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("received,status,name,email,company,phone,budget,timeline,services,about,message,note\n");
    for e in repo::enquiries(&state.db, None, 10_000).await {
        out.push_str(&[cell(&e.created_at), cell(&e.status), cell(&e.name), cell(&e.email), cell(&e.company), cell(&e.phone), cell(&e.budget), cell(&e.timeline), cell(&e.services), cell(&e.project), cell(&e.message), cell(&e.note)].join(","));
        out.push('\n');
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"enquiries-{}.csv\"", crate::export_date_stamp()))], out).into_response()
}
