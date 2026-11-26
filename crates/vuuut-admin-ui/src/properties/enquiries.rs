//! Enquiries: the questions and viewing requests visitors send from
//! property pages. An inbox — filter by status, open one, mark it
//! contacted / viewing booked / closed / spam, keep a private note, reply by
//! email. Personal data, so Editor and above, never public; the export
//! defuses formulas.

use super::*;

pub(crate) async fn enquiries_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let status = rules::ENQUIRY_STATUSES.iter().map(|s| s.0).find(|k| q.get("status").map(String::as_str) == Some(*k));
    let kind = ["info", "viewing"].into_iter().find(|k| q.get("kind").map(String::as_str) == Some(*k));
    let property = match q.get("property").filter(|id| vuuut_core::ids::is_objectid(id)) {
        Some(id) => repo::find(&state.db, id).await,
        None => None,
    };
    let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).clamp(1, 10_000);
    let per = 50;
    let (rows, total) = repo::browse_enquiries(&state.db, &repo::EnquiryQuery { status, kind, property_id: property.as_ref().map(|p| p.id.as_str()), limit: per, offset: (page - 1) * per }).await;
    let counts = repo::enquiry_counts(&state.db).await;
    let seg = |key: Option<&str>, label: &str| {
        let n = match key {
            Some(k) => counts.get(k).copied().unwrap_or(0),
            None => counts.iter().filter(|(k, _)| k.as_str() != "spam").map(|(_, v)| v).sum(),
        };
        let mut href = String::from(ENQUIRIES);
        let mut params = Vec::new();
        if let Some(k) = key {
            params.push(format!("status={k}"));
        }
        if let Some(p) = &property {
            params.push(format!("property={}", p.id));
        }
        if !params.is_empty() {
            href.push('?');
            href.push_str(&params.join("&amp;"));
        }
        format!(r#"<a class="chip{on}" href="{href}">{label} <b>{n}</b></a>"#, on = if status == key { " on" } else { "" }, label = esc(label))
    };
    let filters = format!(
        r#"<div class="pr-chips rise" style="--i:2">{}{}</div>"#,
        seg(None, "All"),
        rules::ENQUIRY_STATUSES.iter().map(|(k, l)| seg(Some(k), l)).collect::<String>()
    );
    let pages = (total + per - 1) / per;
    let pager = if pages > 1 {
        let link = |n: i64| {
            let mut p = vec![format!("page={n}")];
            if let Some(s) = status {
                p.push(format!("status={s}"));
            }
            if let Some(pr) = &property {
                p.push(format!("property={}", pr.id));
            }
            format!("{ENQUIRIES}?{}", p.join("&amp;"))
        };
        format!(
            r#"<nav class="pr-pager">{}<span class="muted">Page {page} of {pages}</span>{}</nav>"#,
            if page > 1 { format!(r#"<a class="btn text small" href="{}">← Newer</a>"#, link(page - 1)) } else { String::new() },
            if page < pages { format!(r#"<a class="btn text small" href="{}">Older →</a>"#, link(page + 1)) } else { String::new() },
        )
    } else {
        String::new()
    };
    page_for_role(
        actor.role,
        "Enquiries",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{about}{filters}
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>{pager}"#,
            head = crate::tools::tool_head_back("properties", BASE, None, &format!(r#"<a class="btn text" href="{ENQUIRIES}export.csv">{} Export CSV</a>"#, svg("import"))),
            tabs = tabs_for(&state, "enquiries", &actor).await,
            banner = flash_banner(&q),
            about = property.as_ref().map(|p| format!(r#"<p class="dr-lead">About <a href="{LISTINGS}{id}/">{t}</a> · <a href="{ENQUIRIES}">all enquiries</a></p>"#, id = esc(&p.id), t = esc(&p.title))).unwrap_or_default(),
            count = esc(&countable(total, "enquiry")),
            body = if rows.is_empty() {
                r#"<div class="card padded"><p class="muted" style="margin:0">Nothing here. Enquiries arrive from the “Contact the agent” and “Request a viewing” forms on property pages.</p></div>"#.to_string()
            } else {
                format!(r#"<div class="card rows">{}</div>"#, rows.iter().map(enquiry_row).collect::<String>())
            },
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
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That enquiry could not be found.");
    }
    let Some(e) = repo::find_enquiry(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That enquiry could not be found.");
    };
    let s = property_settings(&state);
    let property = match &e.property_id {
        Some(pid) => repo::find(&state.db, pid).await,
        None => None,
    };
    let agent = match e.agent_id.as_deref().or(property.as_ref().and_then(|p| p.agent_id.as_deref())) {
        Some(aid) => repo::find_agent(&state.db, aid).await,
        None => None,
    };
    let (chip, label) = enquiry_chip(&e.status);
    let mut facts: Vec<(&str, String)> = vec![("From", e.name.clone()), ("Email", e.email.clone())];
    if !e.phone.is_empty() {
        facts.push(("Phone", e.phone.clone()));
    }
    if e.kind == "viewing" {
        facts.push(("Viewing", match (e.preferred_date.as_str(), e.preferred_time.as_str()) {
            ("", "any" | "") => "Any time".into(),
            ("", t) => format!("Any day, {t}"),
            (d, "any" | "") => d.into(),
            (d, t) => format!("{d}, {t}"),
        }));
    }
    if let Some(a) = &agent {
        facts.push(("Agent", a.name.clone()));
    }
    facts.push(("Received", e.created_at.get(..16).unwrap_or(&e.created_at).to_string()));
    // A mailto only to an address that passed `is_email` when it was stored.
    let reply = if vuuut_core::directory::is_email(&e.email) {
        let subject = format!("Re: {}", if e.property_title.is_empty() { "your enquiry" } else { &e.property_title });
        format!(
            r#"<a class="btn" href="mailto:{to}?subject={sub}">{ic} Reply by email</a>"#,
            to = esc(&e.email),
            sub = esc(&vuuut_core::social_media::encode(&subject)),
            ic = svg("mail"),
        )
    } else {
        String::new()
    };
    let tel = if e.phone.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '+' | '-' | '(' | ')' | '.')) && !e.phone.is_empty() {
        format!(r#"<a class="btn tonal" href="tel:{}">Call</a>"#, esc(&e.phone.replace(' ', "")))
    } else {
        String::new()
    };
    let status_opts: String = rules::ENQUIRY_STATUSES.iter().map(|(k, l)| format!(r#"<option value="{k}"{s}>{l}</option>"#, s = if *k == e.status { " selected" } else { "" })).collect();
    let prop_card = match &property {
        Some(p) => format!(
            r#"<div class="section">Property</div><div class="card rows">{}</div>"#,
            property_row(&s, p, &repo::types(&state.db).await, &repo::agents(&state.db, false).await)
        ),
        None if e.property_id.is_some() => r#"<div class="section">Property</div><div class="card padded"><p class="muted" style="margin:0">The property has since been deleted.</p></div>"#.to_string(),
        None => String::new(),
    };
    let body = format!(
        r#"{head}{banner}
<div class="dr-split">
<div class="rise" style="--i:2"><div class="card padded pr-enquiry">
<p style="margin:0 0 12px"><span class="chip {chip}">{label}</span> <span class="chip tonal">{kind}</span></p>
<dl class="pr-facts-list">{facts}</dl>
{message}
<div class="pr-form-actions" style="justify-content:flex-start">{reply}{tel}</div></div>{prop_card}</div>
<form class="card padded rise" style="--i:3" method="post" action="{ENQUIRIES}{id}/">
<div class="section" style="margin-top:0">Follow-up</div>
<label class="field fixed"><select name="status">{status_opts}</select><span>Status</span>{caret}</label>
<label class="field"><textarea name="note" rows="6" maxlength="4000" placeholder=" ">{note}</textarea><span>Private note — only staff see it</span></label>
<div class="pr-form-actions"><button class="btn text danger" type="submit" form="pr-enquiry-delete">{del} Delete</button><button class="btn" type="submit">{check} Save</button></div></form>
</div>
<form method="post" action="{ENQUIRIES}{id}/delete/" id="pr-enquiry-delete" data-confirm="Delete this enquiry|It cannot be undone."></form>"#,
        head = crate::tools::tool_head_back("properties", ENQUIRIES, None, ""),
        banner = flash_banner(&q),
        kind = esc(rules::EnquiryKind::from_key(&e.kind).label()),
        facts = facts.iter().map(|(k, v)| format!("<dt>{k}</dt><dd>{}</dd>", esc(v))).collect::<String>(),
        message = if e.message.is_empty() { String::new() } else { format!(r#"<blockquote class="pr-message">{}</blockquote>"#, esc(&e.message)) },
        id = esc(&e.id),
        caret = SELECT_CARET,
        note = esc(&e.note),
        del = svg("delete"),
        check = svg("check"),
    );
    page_for_role(actor.role, &format!("Enquiry from {}", e.name), "tools", Some(&actor.user), &body, true).into_response()
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
    if !vuuut_core::ids::is_objectid(&id) || repo::find_enquiry(&state.db, &id).await.is_none() {
        return flash_to(ENQUIRIES, "error", "That enquiry could not be found.");
    }
    let back = format!("{ENQUIRIES}{id}/");
    match repo::save_enquiry(&state.db, &id, &f.status, &vuuut_core::directory::clean_text(&f.note, 4000)).await {
        Ok(()) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn enquiry_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(ENQUIRIES, "error", "That enquiry could not be found.");
    }
    match repo::delete_enquiry(&state.db, &id).await {
        Ok(()) => flash_to(ENQUIRIES, "ok", "Enquiry deleted."),
        Err(_) => flash_to(ENQUIRIES, "error", "That could not be deleted."),
    }
}

/// Every enquiry (spam included), for a spreadsheet.
pub(crate) async fn enquiries_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let mut rows = Vec::new();
    for status in rules::ENQUIRY_STATUSES.iter().map(|s| s.0) {
        rows.extend(repo::browse_enquiries(&state.db, &repo::EnquiryQuery { status: Some(status), limit: 5000, ..Default::default() }).await.0);
    }
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("received,kind,status,name,email,phone,property,preferred_date,preferred_time,message,note\n");
    for e in &rows {
        out.push_str(&[cell(&e.created_at), cell(&e.kind), cell(&e.status), cell(&e.name), cell(&e.email), cell(&e.phone), cell(&e.property_title), cell(&e.preferred_date), cell(&e.preferred_time), cell(&e.message), cell(&e.note)].join(","));
        out.push('\n');
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"property-enquiries-{}.csv\"", crate::export_date_stamp())),
        ],
        out,
    )
        .into_response()
}
