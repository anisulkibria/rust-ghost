//! The résumé: experience, education, awards, press, talks, publications,
//! certifications and volunteering — and skills. A personal portfolio
//! shows all of it on About and the printable résumé page; a studio uses
//! the awards, press and talks.

use super::*;
use crate::podcast::{field, select, textarea};

const RESUME: &str = "/ghost/portfolio/resume/";

fn entry_form(e: Option<&repo::Entry>, action: &str, button: &str) -> String {
    let get = |f: fn(&repo::Entry) -> &str| e.map(f).unwrap_or("").to_string();
    format!(
        r#"<form method="post" action="{action}" class="dr-cat-form"><div class="ec-grid2">{kind}{title}</div><div class="ec-grid2">{org}{loc}</div>
<div class="ec-grid3">{from}{to}<label class="ec-check" style="align-self:center"><input type="checkbox" name="current" value="1"{cur}> To the present</label></div>{url}{desc}
<div class="ec-grid2">{order}<div class="dr-cat-actions" style="align-self:center">{extra}<button class="btn small" type="submit">{button}</button></div></div></form>"#,
        kind = select("kind", "Kind", &rules::EntryKind::options(), e.map(|x| x.kind.as_str()).unwrap_or("work")),
        title = field("title", "Title — a role, a degree, an award", &get(|x| &x.title), "text", r#" required maxlength="160""#),
        org = field("organisation", "Organisation", &get(|x| &x.organisation), "text", r#" maxlength="160""#),
        loc = field("location", "Location", &get(|x| &x.location), "text", r#" maxlength="120""#),
        from = field("start", "From (2021 or 2021-03)", &get(|x| &x.date_from), "text", r#" maxlength="7" pattern="[0-9]{4}(-[0-9]{1,2})?""#),
        to = field("end", "To", &get(|x| &x.date_to), "text", r#" maxlength="7" pattern="[0-9]{4}(-[0-9]{1,2})?""#),
        cur = if e.is_some_and(|x| x.ongoing != 0) { " checked" } else { "" },
        url = field("url", "Link (optional)", &get(|x| &x.url), "url", r#" maxlength="500""#),
        desc = textarea("description", "What you did, what you learned", &get(|x| &x.description), 3, r#" maxlength="3000""#),
        order = field("sort_order", "Order within its kind (lower first)", &e.map(|x| x.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="9999""#),
        extra = e.map(|x| format!(r#"<button class="btn tonal small danger" type="submit" form="del-{}">Delete</button>"#, esc(&x.id))).unwrap_or_default(),
    )
}

pub(crate) async fn resume_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let entries = repo::entries(&state.db).await;
    let skills = repo::skills(&state.db).await;
    let mut groups = String::new();
    for kind in rules::EntryKind::ALL {
        let list: Vec<&repo::Entry> = entries.iter().filter(|e| e.kind == kind.key()).collect();
        if list.is_empty() {
            continue;
        }
        groups.push_str(&format!(r#"<div class="section">{}</div><div class="card padded">"#, esc(kind.label())));
        for e in list {
            groups.push_str(&format!(
                r#"<details class="dr-cat"><summary><b>{t}</b><span class="muted">{org}{span}</span>{chev}</summary>{form}<form method="post" action="{RESUME}entries/{id}/delete/" id="del-{id}" data-confirm="Delete this entry?|"></form></details>"#,
                t = esc(&e.title),
                org = if e.organisation.is_empty() { String::new() } else { format!("{} · ", esc(&e.organisation)) },
                span = esc(&e.span()),
                chev = svg("chevron"),
                form = entry_form(Some(e), &format!("{RESUME}entries/{}/", esc(&e.id)), "Save"),
                id = esc(&e.id),
            ));
        }
        groups.push_str("</div>");
    }
    let skill_rows: String = skills.iter().map(|k| format!(
        r#"<div class="pf-skill-row"><span>{n}</span><small class="muted">{g}</small>{bar}<form method="post" action="{RESUME}skills/{id}/delete/"><button class="icon-btn" type="submit" title="Remove">{del}</button></form></div>"#,
        n = esc(&k.name), g = esc(&k.grp),
        bar = if k.level > 0 { format!(r#"<i class="pf-level" style="--l:{}%"></i>"#, k.level.clamp(0, 100)) } else { "<i></i>".into() },
        id = esc(&k.id), del = svg("close"),
    )).collect();
    let preview = if s.enabled && !entries.is_empty() { format!(r#" <a href="{}" target="_blank" rel="noopener">See the résumé page ↗</a>"#, public_href(&s, "/portfolio/resume/")) } else { String::new() };
    page_for_role(
        actor.role,
        if s.studio() { "Awards & press" } else { "Résumé" },
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">{lead}{preview}</p>
<div class="dr-split"><div class="rise" style="--i:3">{groups}</div>
<div class="rise" style="--i:3"><div class="section">Add an entry</div><div class="card padded">{new}</div>
<div class="section">Skills</div><div class="card padded"><div class="pf-skills">{skills}</div>
<form method="post" action="{RESUME}skills/" class="pf-skill-add"><div class="ec-grid3">{sn}{sg}{sl}</div><div style="display:flex;justify-content:flex-end"><button class="btn small" type="submit">{add} Add skill</button></div></form>
<p class="muted ec-hint">Group skills (Design, Engineering, Languages…). A level from 1–100 draws a bar; leave it blank for a simple tag.</p></div></div></div>
<datalist id="pf-skill-groups">{datalist}</datalist>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, ""),
            tabs = tabs_for(&state, "resume", &actor).await,
            banner = flash_banner(&q),
            lead = if s.studio() { "Awards, press coverage and talks appear on the About page." } else { "Everything here appears on your About page and the printable résumé." },
            groups = if groups.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing yet. Start with your current role.</p></div>"#.to_string() } else { groups },
            new = entry_form(None, &format!("{RESUME}entries/"), "Add"),
            skills = if skill_rows.is_empty() { r#"<p class="muted" style="margin:0 0 12px">No skills yet.</p>"#.to_string() } else { skill_rows },
            sn = field("name", "Skill", "", "text", r#" required maxlength="80""#),
            sg = field("group", "Group", "", "text", r#" maxlength="60" list="pf-skill-groups""#),
            sl = field("level", "Level (1–100)", "", "number", r#" min="0" max="100""#),
            add = svg("add"),
            datalist = { let mut g: Vec<&str> = skills.iter().map(|k| k.grp.as_str()).filter(|g| !g.is_empty()).collect(); g.dedup(); g.iter().map(|x| format!(r#"<option value="{}">"#, esc(x))).collect::<String>() },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn entry_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::EntryInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(RESUME, "error", &m),
    };
    match repo::save_entry(&state.db, None, &input).await {
        Ok(_) => flash_to(RESUME, "ok", "Added."),
        Err(_) => flash_to(RESUME, "error", "That could not be saved."),
    }
}

pub(crate) async fn entry_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if repo::find_entry(&state.db, &id).await.is_none() {
        return flash_to(RESUME, "error", "That entry could not be found.");
    }
    let input = match rules::EntryInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(RESUME, "error", &m),
    };
    match repo::save_entry(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(RESUME, "ok", "Saved."),
        Err(_) => flash_to(RESUME, "error", "That could not be saved."),
    }
}

pub(crate) async fn entry_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_entry(&state.db, &id).await {
        Ok(()) => flash_to(RESUME, "ok", "Deleted."),
        Err(_) => flash_to(RESUME, "error", "That could not be deleted."),
    }
}

pub(crate) async fn skill_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::SkillInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(RESUME, "error", &m),
    };
    match repo::save_skill(&state.db, None, &input).await {
        Ok(_) => flash_to(RESUME, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(RESUME, "error", "That could not be saved."),
    }
}

pub(crate) async fn skill_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_skill(&state.db, &id).await {
        Ok(()) => flash_to(RESUME, "ok", "Removed."),
        Err(_) => flash_to(RESUME, "error", "That could not be removed."),
    }
}
