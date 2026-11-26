//! Hosts & guests — one list of people, each with a public page listing the
//! episodes they are on — and sponsors, read on episodes.

use super::episodes::{card, field, media_box, textarea};
use super::*;

const PEOPLE: &str = "/ghost/podcast/people/";
const SPONSORS: &str = "/ghost/podcast/sponsors/";

pub(crate) async fn people_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = podcast_settings(&state);
    let people = repo::people(&state.db).await;
    let eps = repo::all_credits(&state.db, "episode").await;
    let shows = repo::all_credits(&state.db, "show").await;
    let rows: String = people.iter().map(|p| {
        let n_eps = eps.values().filter(|v| v.iter().any(|(id, _)| id == &p.id)).count();
        let hosting = shows.values().filter(|v| v.iter().any(|(id, _)| id == &p.id)).count();
        format!(
            r#"<div class="row dr-listing">{img}<div class="body"><span class="title"><a href="{PEOPLE}{id}/">{n}</a>{host}</span><span class="detail">{t}{eps}</span></div>
<div class="trailing dr-actions">{view}<a class="icon-btn" href="{PEOPLE}{id}/" title="Edit">{edit}</a></div></div>"#,
            img = if p.photo.is_empty() { format!(r#"<span class="ic tint-pink">{}</span>"#, esc(&p.initials())) } else { format!(r#"<img class="dr-thumb pc-round" src="{}" alt="" loading="lazy">"#, esc(&p.photo)) },
            id = esc(&p.id),
            n = esc(&p.name),
            host = if hosting > 0 { r#" <span class="chip published">Host</span>"# } else { "" },
            t = if p.title.is_empty() { String::new() } else { format!("{} · ", esc(&p.title)) },
            eps = countable(n_eps as i64, "episode"),
            view = if s.enabled && (n_eps > 0 || hosting > 0) { format!(r#"<a class="icon-btn" href="{}" target="_blank" rel="noopener" title="View page">{}</a>"#, public_href(&s, &format!("/podcast/people/{}/", p.slug)), svg("open")) } else { String::new() },
            edit = svg("edit"),
        )
    }).collect();
    page_for_role(
        actor.role,
        "Hosts & guests",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">{count}. Each has a public page with every episode they're on.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, &format!(r#"<a class="btn" href="{PEOPLE}new/">{} Add a person</a>"#, svg("add"))),
            tabs = tabs_for(&state, "people", &actor).await,
            banner = flash_banner(&q),
            count = esc(&countable(people.len() as i64, "person")),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">Nobody yet. Add your hosts, then guests as they come on.</p><a class="btn" href="{PEOPLE}new/">{} Add a person</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn person_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    person_editor(&actor, None, &q)
}

pub(crate) async fn person_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(p) = repo::find_person(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That person could not be found.") };
    person_editor(&actor, Some(p), &q)
}

fn person_editor(actor: &Actor, p: Option<repo::Person>, q: &HashMap<String, String>) -> Response {
    let get = |f: fn(&repo::Person) -> &str| p.as_ref().map(f).unwrap_or("").to_string();
    let action = match &p { Some(x) => format!("{PEOPLE}{}/", esc(&x.id)), None => format!("{PEOPLE}new/") };
    let heading = p.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| "Add a person".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{PEOPLE}" title="Back">{back}</a><div><h1>{h}</h1><p class="sub">A host, co-host, guest or producer.</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="pc-person">{check} Save</button></div></header>{banner}{del}
<form id="pc-person" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main"><section class="card padded ec-card ec-titlecard">{name}{title}{bio}</section>
{links}</div><aside class="ec-edit-side">{photo}{delbtn}</aside></form>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        check = svg("check"),
        banner = flash_banner(q),
        del = p.as_ref().map(|x| format!(r#"<form method="post" action="{PEOPLE}{}/delete/" id="pc-delete" data-confirm="Remove {}?|They come off every show and episode they're credited on.">"#, esc(&x.id), esc(&x.name)) + "</form>").unwrap_or_default(),
        name = field("name", "Name", &get(|x| &x.name), "text", r#" required maxlength="120" id="ec-title""#),
        title = field("title", "What they do (e.g. Author of “Rivers”, Founder at Acme)", &get(|x| &x.title), "text", r#" maxlength="160""#),
        bio = textarea("bio", "Bio", &get(|x| &x.bio), 6, r#" maxlength="3000""#),
        links = card("Links", "link", "blue", &format!("{}{}{}",
            field("website", "Website", &get(|x| &x.website), "url", r#" maxlength="500""#),
            textarea("links", "Other links — one per line (X, LinkedIn, Instagram, a book…)", &get(|x| &x.links), 4, r#" maxlength="6000""#),
            field("slug", "Address on the site", &get(|x| &x.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#)), "pc-links"),
        photo = card("Photo", "image", "pink", &media_box(&get(|x| &x.photo), "Photo"), "pc-photo"),
        delbtn = if p.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="pc-delete">{} Remove</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

fn read_person(pairs: &[(String, String)]) -> Result<rules::PersonInput, String> {
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image" && k != "photo").cloned().collect();
    all.push(("photo".into(), pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default()));
    rules::PersonInput::from_form(&all)
}

pub(crate) async fn person_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_person(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{PEOPLE}new/"), "error", &m),
    };
    match repo::save_person(&state.db, None, &input).await {
        Ok(_) => flash_to(PEOPLE, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(&format!("{PEOPLE}new/"), "error", "That could not be saved."),
    }
}

pub(crate) async fn person_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{PEOPLE}{id}/");
    if repo::find_person(&state.db, &id).await.is_none() {
        return flash_to(PEOPLE, "error", "That person could not be found.");
    }
    let input = match read_person(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_person(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "That could not be saved."),
    }
}

pub(crate) async fn person_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_person(&state.db, &id).await {
        Ok(()) => flash_to(PEOPLE, "ok", "Removed."),
        Err(_) => flash_to(PEOPLE, "error", "That could not be removed."),
    }
}

// ---------------------------------------------------------------------------
// sponsors
// ---------------------------------------------------------------------------

fn sponsor_fields(sp: Option<&repo::Sponsor>) -> String {
    let get = |f: fn(&repo::Sponsor) -> &str| sp.map(f).unwrap_or("").to_string();
    format!(
        r#"<div class="ec-grid2">{n}{u}</div><div class="ec-grid2">{t}{c}</div>{l}<label class="ec-check"><input type="checkbox" name="active" value="1"{on}> Active — offered on new episodes</label>"#,
        n = field("name", "Sponsor", &get(|x| &x.name), "text", r#" required maxlength="120""#),
        u = field("url", "Link (with your tracking code)", &get(|x| &x.url), "url", r#" maxlength="500""#),
        t = field("tagline", "One line about them", &get(|x| &x.tagline), "text", r#" maxlength="240""#),
        c = field("promo_code", "Promo code", &get(|x| &x.promo_code), "text", r#" maxlength="40""#),
        l = field("logo", "Logo address", &get(|x| &x.logo), "text", r#" maxlength="500" placeholder="/content/images/… or https://…""#),
        on = if sp.is_none_or(|x| x.active != 0) { " checked" } else { "" },
    )
}

pub(crate) async fn sponsors_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let list = repo::sponsors(&state.db).await;
    let reads = repo::sponsor_reads(&state.db).await;
    let rows: String = list.iter().map(|sp| format!(
        r#"<details class="dr-cat"><summary>{logo}<b>{n}</b><span class="muted">{reads}{code}</span>{state}{chev}</summary>
<form method="post" action="{SPONSORS}{id}/" class="dr-cat-form">{fields}<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">Delete</button><button class="btn small" type="submit">Save</button></div></form>
<form method="post" action="{SPONSORS}{id}/delete/" id="del-{id}" data-confirm="Delete {n}?|It comes off every episode."></form></details>"#,
        logo = if sp.logo.is_empty() { format!(r#"<span class="ic tint-teal">{}</span>"#, svg("campaign")) } else { format!(r#"<img class="dr-thumb" src="{}" alt="">"#, esc(&sp.logo)) },
        n = esc(&sp.name),
        reads = countable(reads.get(&sp.id).copied().unwrap_or(0), "episode"),
        code = if sp.promo_code.is_empty() { String::new() } else { format!(" · code {}", esc(&sp.promo_code)) },
        state = if sp.active == 0 { r#" <span class="chip draft">Inactive</span>"# } else { "" },
        chev = svg("chevron"),
        id = esc(&sp.id),
        fields = sponsor_fields(Some(sp)),
    )).collect();
    page_for_role(
        actor.role,
        "Sponsors",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead">Sponsors appear on the episodes they're ticked on, with their link and promo code.</p>
<div class="dr-split dr-split-even"><div class="card padded rise" style="--i:3">{rows}</div>
<form class="card padded rise" style="--i:3" method="post" action="{SPONSORS}"><h3 style="margin-top:0">New sponsor</h3>{fields}<div style="display:flex;justify-content:flex-end;margin-top:12px"><button class="btn" type="submit">{add} Add sponsor</button></div></form></div>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, ""),
            tabs = tabs_for(&state, "sponsors", &actor).await,
            banner = flash_banner(&q),
            rows = if rows.is_empty() { r#"<p class="muted" style="margin:0">No sponsors yet.</p>"#.to_string() } else { rows },
            fields = sponsor_fields(None),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn sponsor_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match rules::SponsorInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(SPONSORS, "error", &m),
    };
    match repo::save_sponsor(&state.db, None, &input).await {
        Ok(_) => flash_to(SPONSORS, "ok", &format!("{} added.", input.name)),
        Err(_) => flash_to(SPONSORS, "error", "That could not be saved."),
    }
}

pub(crate) async fn sponsor_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if repo::find_sponsor(&state.db, &id).await.is_none() {
        return flash_to(SPONSORS, "error", "That sponsor could not be found.");
    }
    let input = match rules::SponsorInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return flash_to(SPONSORS, "error", &m),
    };
    match repo::save_sponsor(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(SPONSORS, "ok", "Saved."),
        Err(_) => flash_to(SPONSORS, "error", "That could not be saved."),
    }
}

pub(crate) async fn sponsor_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_sponsor(&state.db, &id).await {
        Ok(()) => flash_to(SPONSORS, "ok", "Sponsor deleted."),
        Err(_) => flash_to(SPONSORS, "error", "That could not be deleted."),
    }
}
