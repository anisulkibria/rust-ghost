//! Services (what can be hired, with a starting price and deliverables),
//! disciplines (the grid's filters) and clients (the logo wall).

use super::*;
use crate::podcast::{card, check, field, media_box, select, textarea};

const SERVICES: &str = "/ghost/portfolio/services/";
const DISCIPLINES: &str = "/ghost/portfolio/disciplines/";
const CLIENTS: &str = "/ghost/portfolio/clients/";

// ---------------------------------------------------------------------------
// services
// ---------------------------------------------------------------------------

pub(crate) async fn services_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let list = repo::services(&state.db, false).await;
    let rows: String = list.iter().map(|v| format!(
        r#"<div class="row dr-listing"><span class="ic tint-blue" title="{icon}">{glyph}</span><div class="body"><span class="title"><a href="{SERVICES}{id}/">{n}</a>{hidden}</span><span class="detail">{price}{sum}</span></div><div class="trailing"><a class="icon-btn" href="{SERVICES}{id}/" title="Edit">{edit}</a></div></div>"#,
        icon = esc(&v.icon), glyph = svg("work"), id = esc(&v.id), n = esc(&v.name),
        hidden = if v.status == "hidden" { r#" <span class="chip draft">Hidden</span>"# } else { "" },
        price = { let p = repo::price_label(&s, v); if p.is_empty() { String::new() } else { format!("{} · ", esc(&p)) } },
        sum = esc(&v.summary), edit = svg("edit"),
    )).collect();
    page_for_role(
        actor.role,
        "Services",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">{count}. Each has its own page with the projects tagged with it, and visitors can pick them on the contact form.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, &format!(r#"<a class="btn" href="{SERVICES}new/">{} New service</a>"#, svg("add"))),
            tabs = tabs_for(&state, "services", &actor).await,
            banner = flash_banner(&q),
            count = esc(&countable(list.len() as i64, "service")),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No services yet — Brand identity, Web design, Photography…</p><a class="btn" href="{SERVICES}new/">{} New service</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn service_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    service_editor(&state, &actor, None, &q)
}

pub(crate) async fn service_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(v) = repo::find_service(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That service could not be found.") };
    service_editor(&state, &actor, Some(v), &q)
}

fn service_editor(state: &AppState, actor: &Actor, v: Option<repo::Service>, q: &HashMap<String, String>) -> Response {
    let s = portfolio_settings(state);
    let get = |f: fn(&repo::Service) -> &str| v.as_ref().map(f).unwrap_or("").to_string();
    let action = match &v { Some(x) => format!("{SERVICES}{}/", esc(&x.id)), None => format!("{SERVICES}new/") };
    let icons: Vec<(String, String)> = rules::SERVICE_ICONS.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
    let price = v.as_ref().filter(|x| x.price_from > 0).map(|x| format!("{}.{:02}", x.price_from / 100, x.price_from % 100)).unwrap_or_default();
    let heading = v.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| "New service".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{SERVICES}" title="Back">{back}</a><div><h1>{h}</h1><p class="sub">Something people can hire you for.</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pf-service">{check} Save</button></div></header>{banner}{del}
<form id="pf-service" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main"><section class="card padded ec-card ec-titlecard">{name}{summary}{desc}</section>
{deliv}{seo}</div><aside class="ec-edit-side">{status}{pricing}{look}{delbtn}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        check = svg("check"),
        banner = flash_banner(q),
        del = v.as_ref().map(|x| format!(r#"<form method="post" action="{SERVICES}{}/delete/" id="pf-delete" data-confirm="Delete {}?|It comes off its projects and the contact form.">"#, esc(&x.id), esc(&x.name)) + "</form>").unwrap_or_default(),
        name = field("name", "Service", &get(|x| &x.name), "text", r#" required maxlength="120" id="ec-title""#),
        summary = field("summary", "One line for the list", &get(|x| &x.summary), "text", r#" maxlength="300""#),
        desc = textarea("description", "What it involves, how you work, what it costs and how long it takes", &get(|x| &x.description), 8, r#" maxlength="8000""#),
        deliv = card("Deliverables", "list", "green", &textarea("deliverables", "What the client gets — one per line", &v.as_ref().map(|x| x.deliverables().join("\n")).unwrap_or_default(), 6, r#" maxlength="4000""#), "pf-deliv"),
        seo = card("Address", "link", "slate", &field("slug", "Address on the site", &get(|x| &x.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#), "pf-seo"),
        status = card("Status", "eye", "green", &format!("{}{}{}",
            select("status", "Status", &rules::ServiceStatus::options(), v.as_ref().map(|x| x.status.as_str()).unwrap_or("active")),
            check("featured", "1", "Feature on the portfolio home", v.as_ref().is_some_and(|x| x.featured != 0)),
            field("sort_order", "Order", &v.as_ref().map(|x| x.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="9999""#)), "pf-status"),
        pricing = card("Price", "sell", "amber", &format!(r#"{}{}<p class="muted ec-hint">In {}. Blank shows no price.</p>"#,
            field("price_from", "Starting from", &price, "text", r#" inputmode="decimal""#),
            field("price_note", "Price note", &get(|x| &x.price_note), "text", r#" maxlength="60" placeholder="per project""#), esc(&s.currency)), "pf-price"),
        look = card("Look", "image", "pink", &format!("{}{}", select("icon", "Icon", &icons, v.as_ref().map(|x| x.icon.as_str()).unwrap_or("spark")), media_box(&get(|x| &x.image), "Picture")), "pf-look"),
        delbtn = if v.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pf-delete">{} Delete</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

fn read_service(pairs: &[(String, String)]) -> Result<rules::ServiceInput, String> {
    rules::ServiceInput::from_form(pairs)
}

pub(crate) async fn service_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_service(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{SERVICES}new/"), "error", &m),
    };
    match repo::save_service(&state.db, None, &input).await {
        Ok(_) => flash_to(SERVICES, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(&format!("{SERVICES}new/"), "error", "That could not be saved."),
    }
}

pub(crate) async fn service_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{SERVICES}{id}/");
    if repo::find_service(&state.db, &id).await.is_none() {
        return flash_to(SERVICES, "error", "That service could not be found.");
    }
    let input = match read_service(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_service(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn service_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_service(&state.db, &id).await {
        Ok(()) => flash_to(SERVICES, "ok", "Service deleted."),
        Err(_) => flash_to(SERVICES, "error", "That could not be deleted."),
    }
}

// ---------------------------------------------------------------------------
// disciplines
// ---------------------------------------------------------------------------

pub(crate) async fn disciplines_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let cats = repo::categories(&state.db).await;
    let counts = repo::category_counts(&state.db).await;
    let tints: Vec<(String, String)> = rules::TINTS.iter().map(|t| (t.to_string(), t[..1].to_uppercase() + &t[1..])).collect();
    let rows: String = cats.iter().map(|c| format!(
        r#"<details class="dr-cat"><summary><span class="ic tint-{tint}">{ic}</span><b>{name}</b><span class="muted">{n}</span>{chev}</summary>
<form method="post" action="{DISCIPLINES}{id}/" class="dr-cat-form"><div class="ec-grid2">{nf}{tints}</div>{df}<div class="ec-grid2">{order}{slug}</div>
<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">Delete</button><button class="btn small" type="submit">Save</button></div></form>
<form method="post" action="{DISCIPLINES}{id}/delete/" id="del-{id}" data-confirm="Delete {name}?|Its projects stay, without this discipline."></form></details>"#,
        tint = esc(&c.tint), ic = svg("folder"), name = esc(&c.name), n = countable(counts.get(&c.id).copied().unwrap_or(0), "project"), chev = svg("chevron"), id = esc(&c.id),
        nf = field("name", "Name", &c.name, "text", r#" required maxlength="80""#),
        tints = select("tint", "Colour", &tints, &c.tint),
        df = field("description", "Description (shown on its page)", &c.description, "text", r#" maxlength="300""#),
        order = field("sort_order", "Order", &c.sort_order.to_string(), "number", r#" min="0" max="999""#),
        slug = field("slug", "Address", &c.slug, "text", r#" maxlength="80""#),
    )).collect();
    page_for_role(
        actor.role,
        "Disciplines",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">The filters above the grid, each with its own page. Branding, Web, Product, Photography, Motion…</p>
<div class="dr-split dr-split-even"><div class="card padded rise" style="--i:3">{rows}</div>
<form class="card padded rise" style="--i:3" method="post" action="{DISCIPLINES}"><div class="ec-grid2">{nf}{tint}</div>{df}<div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add discipline</button></div></form></div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, ""),
            tabs = tabs_for(&state, "disciplines", &actor).await,
            banner = flash_banner(&q),
            rows = if rows.is_empty() { r#"<p class="muted" style="margin:0">None yet.</p>"#.to_string() } else { rows },
            nf = field("name", "New discipline", "", "text", r#" required maxlength="80""#),
            tint = select("tint", "Colour", &tints, "slate"),
            df = field("description", "Description", "", "text", r#" maxlength="300""#),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct DisciplineForm {
    name: String,
    tint: String,
    description: String,
    sort_order: String,
    slug: String,
}

async fn save_discipline(state: &AppState, id: Option<&str>, f: &DisciplineForm) -> Response {
    let name = vuuut_core::directory::clean(&f.name, 80);
    if name.is_empty() {
        return flash_to(DISCIPLINES, "error", "A discipline needs a name.");
    }
    let tint = if rules::TINTS.contains(&f.tint.as_str()) { f.tint.as_str() } else { "slate" };
    let order = f.sort_order.trim().parse::<i64>().ok().map(|n| n.clamp(0, 999)).unwrap_or_else(|| if id.is_none() { 0 } else { 0 });
    let order = if id.is_none() && f.sort_order.trim().is_empty() { repo::categories(&state.db).await.len() as i64 } else { order };
    let slug = vuuut_core::directory::slugify(if f.slug.trim().is_empty() { &name } else { &f.slug });
    match repo::save_category(&state.db, id, &name, &slug, &vuuut_core::directory::clean(&f.description, 300), tint, order).await {
        Ok(_) => flash_to(DISCIPLINES, "ok", &format!("{name} saved.")),
        Err(_) => flash_to(DISCIPLINES, "error", "That could not be saved."),
    }
}

pub(crate) async fn discipline_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<DisciplineForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    save_discipline(&state, None, &f).await
}

pub(crate) async fn discipline_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<DisciplineForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    save_discipline(&state, Some(&id), &f).await
}

pub(crate) async fn discipline_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_category(&state.db, &id).await {
        Ok(()) => flash_to(DISCIPLINES, "ok", "Discipline deleted."),
        Err(_) => flash_to(DISCIPLINES, "error", "That could not be deleted."),
    }
}

// ---------------------------------------------------------------------------
// clients
// ---------------------------------------------------------------------------

fn client_fields(c: Option<&repo::Client>) -> String {
    format!(
        r#"<div class="ec-grid2">{n}{i}</div>{u}{l}<div class="ec-grid2">{o}<label class="ec-check" style="align-self:center"><input type="checkbox" name="featured" value="1"{f}> On the logo wall</label></div>"#,
        n = field("name", "Client", c.map(|x| x.name.as_str()).unwrap_or(""), "text", r#" required maxlength="120""#),
        i = field("industry", "Industry", c.map(|x| x.industry.as_str()).unwrap_or(""), "text", r#" maxlength="80""#),
        u = field("url", "Website", c.map(|x| x.url.as_str()).unwrap_or(""), "url", r#" maxlength="500""#),
        l = field("logo", "Logo address (SVG or PNG on a transparent background works best)", c.map(|x| x.logo.as_str()).unwrap_or(""), "text", r#" maxlength="500" placeholder="/content/images/… or https://…""#),
        o = field("sort_order", "Order", &c.map(|x| x.sort_order).unwrap_or(0).to_string(), "number", r#" min="0" max="9999""#),
        f = if c.is_none_or(|x| x.featured != 0) { " checked" } else { "" },
    )
}

pub(crate) async fn clients_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let list = repo::clients(&state.db).await;
    let (all, _) = repo::projects(&state.db, &repo::ProjectQuery { limit: 1000, ..Default::default() }).await;
    let rows: String = list.iter().map(|c| format!(
        r#"<details class="dr-cat"><summary>{logo}<b>{n}</b><span class="muted">{p}{ind}</span>{wall}{chev}</summary>
<form method="post" action="{CLIENTS}{id}/" class="dr-cat-form">{fields}<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">Delete</button><button class="btn small" type="submit">Save</button></div></form>
<form method="post" action="{CLIENTS}{id}/delete/" id="del-{id}" data-confirm="Delete {n}?|Its projects keep the name."></form></details>"#,
        logo = if c.logo.is_empty() { format!(r#"<span class="ic tint-slate">{}</span>"#, svg("store")) } else { format!(r#"<img class="dr-thumb pf-logo" src="{}" alt="">"#, esc(&c.logo)) },
        n = esc(&c.name),
        p = countable(all.iter().filter(|p| p.client_id.as_deref() == Some(c.id.as_str())).count() as i64, "project"),
        ind = if c.industry.is_empty() { String::new() } else { format!(" · {}", esc(&c.industry)) },
        wall = if c.featured != 0 { r#" <span class="chip published">Logo wall</span>"# } else { "" },
        chev = svg("chevron"),
        id = esc(&c.id),
        fields = client_fields(Some(c)),
    )).collect();
    page_for_role(
        actor.role,
        "Clients",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">Clients ticked for the logo wall appear on the portfolio home and the About page; projects can name their client.</p>
<div class="dr-split dr-split-even"><div class="card padded rise" style="--i:3">{rows}</div>
<form class="card padded rise" style="--i:3" method="post" action="{CLIENTS}"><h3 style="margin-top:0">Add a client</h3>{fields}<div style="display:flex;justify-content:flex-end;margin-top:12px"><button class="btn" type="submit">{add} Add client</button></div></form></div>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, None, ""),
            tabs = tabs_for(&state, "clients", &actor).await,
            banner = flash_banner(&q),
            rows = if rows.is_empty() { r#"<p class="muted" style="margin:0">No clients yet.</p>"#.to_string() } else { rows },
            fields = client_fields(None),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn client_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::ClientInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(CLIENTS, "error", &m),
    };
    match repo::save_client(&state.db, None, &input).await {
        Ok(_) => flash_to(CLIENTS, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(CLIENTS, "error", "That could not be saved."),
    }
}

pub(crate) async fn client_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::ClientInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(CLIENTS, "error", &m),
    };
    match repo::save_client(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(CLIENTS, "ok", "Saved."),
        Err(_) => flash_to(CLIENTS, "error", "That could not be saved."),
    }
}

pub(crate) async fn client_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_client(&state.db, &id).await {
        Ok(()) => flash_to(CLIENTS, "ok", "Client deleted."),
        Err(_) => flash_to(CLIENTS, "error", "That could not be deleted."),
    }
}
