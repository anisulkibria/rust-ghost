//! Agents: the people (or branches, or landlords) properties are listed by.
//! Each has a public page with their properties, and gets the enquiries
//! about them by email. Editor and above.

use super::*;

pub(crate) async fn agents_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = property_settings(&state);
    let agents = repo::agents(&state.db, false).await;
    let counts = repo::agent_counts(&state.db).await;
    let cards: String = agents
        .iter()
        .map(|a| {
            format!(
                r#"<a class="card padded pr-agent{hidden}" href="{AGENTS}{id}/">{photo}<div><b>{name}</b><span class="muted">{title}</span>
<span class="pr-agent-meta">{n}{contact}</span></div>{chip}</a>"#,
                hidden = if a.is_active() { "" } else { " pr-agent-hidden" },
                id = esc(&a.id),
                photo = if a.photo.is_empty() { format!(r#"<span class="pr-avatar">{}</span>"#, esc(&a.initials())) } else { format!(r#"<img class="pr-avatar" src="{}" alt="">"#, esc(&a.photo)) },
                name = esc(&a.name),
                title = esc(&[a.title.as_str(), a.agency.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" · ")),
                n = countable(counts.get(&a.id).copied().unwrap_or(0), "property"),
                contact = if a.email.is_empty() { String::new() } else { format!(" · {}", esc(&a.email)) },
                chip = if a.is_active() { String::new() } else { r#"<span class="chip draft">Hidden</span>"#.to_string() },
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Agents",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<p class="muted dr-lead">Each agent has a page at {path} listing their properties, and receives the enquiries about them by email.</p>
<div class="pr-agents rise" style="--i:2">{body}</div>"#,
            head = crate::tools::tool_head_back("properties", BASE, None, &format!(r#"<a class="btn" href="{AGENTS}new/">{} New agent</a>"#, svg("add"))),
            tabs = tabs_for(&state, "agents", &actor).await,
            banner = flash_banner(&q),
            path = esc(&s.paths.public_path("/properties/agent/name/")),
            body = if agents.is_empty() {
                format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No agents yet.</p><a class="btn" href="{AGENTS}new/">{} New agent</a></div>"#, svg("add"))
            } else {
                cards
            },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn agent_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    agent_editor(&state, &actor, None, &q).await
}

pub(crate) async fn agent_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That agent could not be found.");
    }
    let Some(a) = repo::find_agent(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That agent could not be found.");
    };
    agent_editor(&state, &actor, Some(a), &q).await
}

async fn agent_editor(state: &AppState, actor: &Actor, a: Option<repo::Agent>, q: &HashMap<String, String>) -> Response {
    let s = property_settings(state);
    let get = |f: fn(&repo::Agent) -> &str| a.as_ref().map(f).unwrap_or("").to_string();
    let field = |name: &str, label: &str, value: &str, kind: &str, extra: &str| {
        format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
    };
    let action = match &a {
        Some(a) => format!("{AGENTS}{}/", esc(&a.id)),
        None => format!("{AGENTS}new/"),
    };
    let photo = get(|a| &a.photo);
    let properties = match &a {
        Some(agent) => {
            let (list, total) = repo::browse(&state.db, &repo::Browse { agent_id: Some(&agent.id), sort: "updated", limit: 8, ..Default::default() }).await.unwrap_or_default();
            if list.is_empty() {
                String::new()
            } else {
                let types = repo::types(&state.db).await;
                format!(
                    r#"<div class="section">Their properties</div><div class="card rows">{}</div>{more}"#,
                    list.iter().map(|p| property_row(&s, p, &types, &[])).collect::<String>(),
                    more = if total > 8 { format!(r#"<p class="muted"><a href="{LISTINGS}?agent={}">All {total}</a></p>"#, esc(&agent.id)) } else { String::new() },
                )
            }
        }
        None => String::new(),
    };
    let heading = a.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "New agent".into());
    let body = format!(
        r#"{head}{banner}
<div class="dr-split">
<form class="card padded rise pr-agent-form" style="--i:2" method="post" action="{action}" id="pr-agent">
<div class="pr-agent-photo"><div class="pr-agent-photo-pic" id="pr-photo-preview">{preview}</div>
<div style="flex:1">{photo_field}<label class="btn tonal small pr-upload" data-target="pr-photo">{up} Upload a photo<input type="file" accept="image/*" hidden></label></div></div>
<div class="ec-grid2">{name}{slug}</div>
<div class="ec-grid2">{title}{agency}</div>
<div class="ec-grid2">{email}{phone}</div>
<div class="ec-grid2">{mobile}{license}</div>
<div class="ec-grid2">{website}{languages}</div>
<label class="field"><textarea name="bio" rows="5" maxlength="4000" placeholder=" ">{bio}</textarea><span>About them, on their page</span></label>
<label class="ec-check"><input type="checkbox" name="status" value="hidden"{hidden}> Hidden — keep them here, but not on the site or on their properties</label>
<div class="pr-form-actions">{delete}<button class="btn" type="submit">{check} {save}</button></div>
</form>
<div class="rise" style="--i:3">{properties}</div></div>
{del_form}
<script src="{ASSETS}/properties-editor.js?v={hash}" defer></script>"#,
        head = crate::tools::tool_head_back("properties", AGENTS, None, ""),
        banner = flash_banner(q),
        preview = if photo.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="">"#, esc(&photo)) },
        photo_field = field("photo", "Photo address", &photo, "text", r#" maxlength="500" id="pr-photo""#),
        up = svg("upload"),
        name = field("name", "Name", &get(|a| &a.name), "text", r#" required maxlength="120""#),
        slug = field("slug", "Page address (blank: from the name)", &get(|a| &a.slug), "text", r#" maxlength="80""#),
        title = field("title", "Job title", &get(|a| &a.title), "text", r#" maxlength="120" placeholder="Sales negotiator""#),
        agency = field("agency", "Agency / branch", &get(|a| &a.agency), "text", r#" maxlength="120""#),
        email = field("email", "Email — enquiries go here", &get(|a| &a.email), "email", r#" maxlength="191""#),
        phone = field("phone", "Phone", &get(|a| &a.phone), "tel", r#" maxlength="40""#),
        mobile = field("mobile", "Mobile", &get(|a| &a.mobile), "tel", r#" maxlength="40""#),
        license = field("license", "Licence number", &get(|a| &a.license), "text", r#" maxlength="60""#),
        website = field("website", "Website", &get(|a| &a.website), "url", r#" maxlength="500" placeholder="https://""#),
        languages = field("languages", "Languages spoken", &get(|a| &a.languages), "text", r#" maxlength="120""#),
        bio = esc(&get(|a| &a.bio)),
        hidden = if a.as_ref().is_some_and(|a| !a.is_active()) { " checked" } else { "" },
        delete = if a.is_some() { format!(r#"<button class="btn text danger" type="submit" form="pr-agent-delete">{} Delete</button>"#, svg("delete")) } else { String::new() },
        check = svg("check"),
        save = if a.is_some() { "Save" } else { "Create agent" },
        del_form = a.as_ref().map(|a| format!(r#"<form method="post" action="{AGENTS}{id}/delete/" id="pr-agent-delete" data-confirm="Delete {n}|Their properties stay, with no agent shown."></form>"#, id = esc(&a.id), n = esc(&a.name))).unwrap_or_default(),
        hash = editor_script_hash_agents(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &body, true).into_response()
}

fn editor_script_hash_agents() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../../assets/properties-editor.js")));
    &HASH
}

pub(crate) async fn agent_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::AgentInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{AGENTS}new/"), "error", &m),
    };
    match repo::save_agent(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{AGENTS}{id}/"), "ok", "Agent added."),
        Err(e) => {
            tracing::error!("properties: adding an agent failed: {}", e.message());
            flash_to(&format!("{AGENTS}new/"), "error", "The agent could not be saved.")
        }
    }
}

pub(crate) async fn agent_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) || repo::find_agent(&state.db, &id).await.is_none() {
        return flash_to(AGENTS, "error", "That agent could not be found.");
    }
    let back = format!("{AGENTS}{id}/");
    let input = match rules::AgentInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_agent(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(e) => {
            tracing::error!("properties: saving agent {id} failed: {}", e.message());
            flash_to(&back, "error", "The agent could not be saved.")
        }
    }
}

pub(crate) async fn agent_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(AGENTS, "error", "That agent could not be found.");
    }
    match repo::delete_agent(&state.db, &id).await {
        Ok(()) => flash_to(AGENTS, "ok", "Agent deleted. Their properties now show no agent."),
        Err(_) => flash_to(AGENTS, "error", "That could not be deleted."),
    }
}
