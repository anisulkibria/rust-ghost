//! The team (a studio's people, or a person's collaborators) and
//! testimonials.

use super::*;
use crate::podcast::{card, check, field, media_box, select, textarea};

const TEAM: &str = "/ghost/portfolio/team/";
const TESTIMONIALS: &str = "/ghost/portfolio/testimonials/";

fn photo_from(pairs: &[(String, String)], name: &str) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image" && k != name).cloned().collect();
    all.push((name.into(), pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default()));
    all
}

// ---------------------------------------------------------------------------
// team
// ---------------------------------------------------------------------------

pub(crate) async fn team_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let list = repo::team(&state.db, false).await;
    let cards: String = list.iter().map(|m| format!(
        r#"<a class="pf-card pf-card-person" href="{TEAM}{id}/"><span class="pf-card-art">{img}{hidden}</span><span class="pf-card-body"><b>{n}</b><span>{r}</span></span></a>"#,
        id = esc(&m.id),
        img = if m.photo.is_empty() { format!(r#"<i class="pf-initials">{}</i>"#, esc(&m.initials())) } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&m.photo)) },
        hidden = if m.status != "active" { r#"<span class="chip draft">Hidden</span>"# } else { "" },
        n = esc(&m.name), r = esc(&m.role),
    )).collect();
    let title = if s.studio() { "Team" } else { "Collaborators" };
    page_for_role(
        actor.role,
        title,
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">{lead}</p><div class="pf-grid pf-grid-people rise" style="--i:3">{cards}</div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, &format!(r#"<a class="btn" href="{TEAM}new/">{} Add a person</a>"#, svg("add"))),
            tabs = tabs_for(&state, "team", &actor).await,
            banner = flash_banner(&q),
            lead = if s.studio() { "The people of the studio, on the About page, each with their own page and the projects that credit them." } else { "People you work with often — shown on your About page." },
            cards = if cards.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">Nobody yet.</p><a class="btn" href="{TEAM}new/">{} Add a person</a></div>"#, svg("add")) } else { cards },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn team_member_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    member_editor(&actor, None, &q)
}

pub(crate) async fn team_member_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(m) = repo::find_member(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That person could not be found.") };
    member_editor(&actor, Some(m), &q)
}

fn member_editor(actor: &Actor, m: Option<repo::Member>, q: &HashMap<String, String>) -> Response {
    let get = |f: fn(&repo::Member) -> &str| m.as_ref().map(f).unwrap_or("").to_string();
    let action = match &m { Some(x) => format!("{TEAM}{}/", esc(&x.id)), None => format!("{TEAM}new/") };
    let heading = m.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| "Add a person".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{TEAM}" title="Back">{back}</a><div><h1>{h}</h1><p class="sub">Credit them by this exact name on a project and it appears on their page.</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pf-member">{check} Save</button></div></header>{banner}{del}
<form id="pf-member" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main"><section class="card padded ec-card ec-titlecard">{name}{role}{bio}</section>{links}</div>
<aside class="ec-edit-side">{photo}{status}{delbtn}</aside></form><script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        check = svg("check"),
        banner = flash_banner(q),
        del = m.as_ref().map(|x| format!(r#"<form method="post" action="{TEAM}{}/delete/" id="pf-delete" data-confirm="Remove {}?|Their page goes; project credits keep the name.">"#, esc(&x.id), esc(&x.name)) + "</form>").unwrap_or_default(),
        name = field("name", "Name", &get(|x| &x.name), "text", r#" required maxlength="120" id="ec-title""#),
        role = field("role", "Role", &get(|x| &x.role), "text", r#" maxlength="120" placeholder="Creative director""#),
        bio = textarea("bio", "Bio", &get(|x| &x.bio), 6, r#" maxlength="3000""#),
        links = card("Contact & links", "link", "blue", &format!("{}{}{}",
            field("email", "Email (not shown publicly)", &get(|x| &x.email), "email", r#" maxlength="191""#),
            textarea("links", "Links — one per line (LinkedIn, Instagram, a personal site…)", &get(|x| &x.links), 4, r#" maxlength="5000""#),
            field("slug", "Address on the site", &get(|x| &x.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#)), "pf-links"),
        photo = card("Photo", "image", "pink", &media_box(&get(|x| &x.photo), "Photo"), "pf-photo"),
        status = card("Status", "eye", "green", &format!("{}{}",
            select("status", "Shown", &[("active".to_string(), "On the site".to_string()), ("hidden".to_string(), "Hidden".to_string())], m.as_ref().map(|x| x.status.as_str()).unwrap_or("active")),
            field("sort_order", "Order", &m.as_ref().map(|x| x.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="9999""#)), "pf-status"),
        delbtn = if m.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pf-delete">{} Remove</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

pub(crate) async fn team_member_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::MemberInput::from_form(&photo_from(&form, "photo")) {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{TEAM}new/"), "error", &m),
    };
    match repo::save_member(&state.db, None, &input).await {
        Ok(_) => flash_to(TEAM, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(&format!("{TEAM}new/"), "error", "That could not be saved."),
    }
}

pub(crate) async fn team_member_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{TEAM}{id}/");
    if repo::find_member(&state.db, &id).await.is_none() {
        return flash_to(TEAM, "error", "That person could not be found.");
    }
    let input = match rules::MemberInput::from_form(&photo_from(&form, "photo")) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_member(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn team_member_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_member(&state.db, &id).await {
        Ok(()) => flash_to(TEAM, "ok", "Removed."),
        Err(_) => flash_to(TEAM, "error", "That could not be removed."),
    }
}

// ---------------------------------------------------------------------------
// testimonials
// ---------------------------------------------------------------------------

pub(crate) async fn testimonials_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let list = repo::testimonials(&state.db).await;
    let rows: String = list.iter().map(|t| format!(
        r#"<div class="row dr-listing">{img}<div class="body"><span class="title"><a href="{TESTIMONIALS}{id}/">{a}</a>{feat}</span><span class="detail">{by}“{q}”</span></div><div class="trailing"><a class="icon-btn" href="{TESTIMONIALS}{id}/" title="Edit">{edit}</a></div></div>"#,
        img = if t.photo.is_empty() { format!(r#"<span class="ic tint-amber">{}</span>"#, svg("comment")) } else { format!(r#"<img class="dr-thumb pc-round" src="{}" alt="" loading="lazy">"#, esc(&t.photo)) },
        id = esc(&t.id), a = esc(&t.author),
        feat = if t.featured != 0 { r#" <span class="chip published">On the home page</span>"# } else { "" },
        by = { let b = [t.role.as_str(), t.company.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(", "); if b.is_empty() { b } else { format!("{} · ", esc(&b)) } },
        q = esc(&t.quote.chars().take(140).collect::<String>()),
        edit = svg("edit"),
    )).collect();
    page_for_role(
        actor.role,
        "Testimonials",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">Featured testimonials appear on the portfolio home; every one is on the About page; one can sit under its project.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, &format!(r#"<a class="btn" href="{TESTIMONIALS}new/">{} New testimonial</a>"#, svg("add"))),
            tabs = tabs_for(&state, "testimonials", &actor).await,
            banner = flash_banner(&q),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No testimonials yet.</p><a class="btn" href="{TESTIMONIALS}new/">{} New testimonial</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn testimonial_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    testimonial_editor(&state, &actor, None, &q).await
}

pub(crate) async fn testimonial_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(t) = repo::find_testimonial(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That testimonial could not be found.") };
    testimonial_editor(&state, &actor, Some(t), &q).await
}

async fn testimonial_editor(state: &AppState, actor: &Actor, t: Option<repo::Testimonial>, q: &HashMap<String, String>) -> Response {
    let get = |f: fn(&repo::Testimonial) -> &str| t.as_ref().map(f).unwrap_or("").to_string();
    let action = match &t { Some(x) => format!("{TESTIMONIALS}{}/", esc(&x.id)), None => format!("{TESTIMONIALS}new/") };
    let (projects, _) = repo::projects(&state.db, &repo::ProjectQuery { limit: 1000, order: "title", ..Default::default() }).await;
    let mut p_opts = vec![(String::new(), "None".to_string())];
    p_opts.extend(projects.iter().map(|p| (p.id.clone(), p.title.clone())));
    let stars: Vec<(String, String)> = [("0", "No rating"), ("5", "★★★★★"), ("4", "★★★★"), ("3", "★★★")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let heading = t.as_ref().map(|x| x.author.clone()).unwrap_or_else(|| "New testimonial".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{TESTIMONIALS}" title="Back">{back}</a><div><h1>{h}</h1><p class="sub">In their words. Ask before you publish.</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pf-testimonial">{check} Save</button></div></header>{banner}{del}
<form id="pf-testimonial" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main"><section class="card padded ec-card ec-titlecard">{quote}<div class="ec-grid3">{author}{role}{company}</div></section></div>
<aside class="ec-edit-side">{photo}{opts}{delbtn}</aside></form><script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        check = svg("check"),
        banner = flash_banner(q),
        del = t.as_ref().map(|x| format!(r#"<form method="post" action="{TESTIMONIALS}{}/delete/" id="pf-delete" data-confirm="Delete this testimonial?|It comes off the site.">"#, esc(&x.id)) + "</form>").unwrap_or_default(),
        quote = textarea("quote", "What they said", &get(|x| &x.quote), 6, r#" required maxlength="2000""#),
        author = field("author", "Name", &get(|x| &x.author), "text", r#" required maxlength="120" id="ec-title""#),
        role = field("role", "Role", &get(|x| &x.role), "text", r#" maxlength="120""#),
        company = field("company", "Company", &get(|x| &x.company), "text", r#" maxlength="120""#),
        photo = card("Photo", "image", "pink", &media_box(&get(|x| &x.photo), "Photo"), "pf-photo"),
        opts = card("Where it shows", "eye", "green", &format!("{}{}{}{}",
            select("project_id", "Project", &p_opts, t.as_ref().and_then(|x| x.project_id.as_deref()).unwrap_or("")),
            select("rating", "Rating", &stars, &t.as_ref().map(|x| x.rating).unwrap_or(0).to_string()),
            check("featured", "1", "Feature on the portfolio home", t.as_ref().is_some_and(|x| x.featured != 0)),
            field("sort_order", "Order", &t.as_ref().map(|x| x.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="9999""#)), "pf-opts"),
        delbtn = if t.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pf-delete">{} Delete</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

async fn read_testimonial(state: &AppState, pairs: &[(String, String)]) -> Result<rules::TestimonialInput, String> {
    let (projects, _) = repo::projects(&state.db, &repo::ProjectQuery { limit: 1000, ..Default::default() }).await;
    rules::TestimonialInput::from_form(&photo_from(pairs, "photo"), |id| projects.iter().any(|p| p.id == id))
}

pub(crate) async fn testimonial_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_testimonial(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{TESTIMONIALS}new/"), "error", &m),
    };
    match repo::save_testimonial(&state.db, None, &input).await {
        Ok(_) => flash_to(TESTIMONIALS, "ok", "Testimonial added."),
        Err(_) => flash_to(&format!("{TESTIMONIALS}new/"), "error", "That could not be saved."),
    }
}

pub(crate) async fn testimonial_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{TESTIMONIALS}{id}/");
    if repo::find_testimonial(&state.db, &id).await.is_none() {
        return flash_to(TESTIMONIALS, "error", "That testimonial could not be found.");
    }
    let input = match read_testimonial(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_testimonial(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn testimonial_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_testimonial(&state.db, &id).await {
        Ok(()) => flash_to(TESTIMONIALS, "ok", "Deleted."),
        Err(_) => flash_to(TESTIMONIALS, "error", "That could not be deleted."),
    }
}
