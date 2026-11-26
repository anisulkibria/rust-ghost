//! Tools → Portfolio: the admin screens.
//!
//! The rules are [`vuuut_core::portfolio`], the queries
//! [`vuuut_db::repos::portfolio`], and the public pages at `/portfolio/` are
//! `vuuut_front::portfolio`. One tool for both kinds of portfolio — the
//! mode in Settings decides which words the site uses and which sections it
//! leads with (a person's résumé and skills, or a studio's team and
//! services). Here:
//!
//! * **Overview** (`/ghost/portfolio/`) — what a visitor would find
//!   missing, new enquiries, the most-viewed projects.
//! * **Projects** — the grid and the case-study editor (`portfolio/projects.rs`).
//! * **Services**, **Disciplines** and **Clients** (`portfolio/catalogue.rs`).
//! * **Team** and **Testimonials** (`portfolio/people.rs`).
//! * **Résumé** — experience, education, awards, press, talks, skills
//!   (`portfolio/resume.rs`).
//! * **Enquiries** — the contact / project-brief inbox (`portfolio/enquiries.rs`).
//! * **Settings** — Administrator only (`portfolio/settings.rs`).
//!
//! Editor and above for the rest. Every write is origin-checked. Enquiries
//! hold visitors' contact details and are shown to staff only; the CSV
//! export defuses formulas.

use super::*;
use chrono::{Duration, Utc};
use vuuut_core::portfolio::{self as rules, Settings};
use vuuut_db::repos::portfolio as repo;

mod catalogue;
mod enquiries;
mod people;
mod projects;
mod resume;
mod sample;
mod settings;
pub(crate) use catalogue::*;
pub(crate) use enquiries::*;
pub(crate) use people::*;
pub(crate) use projects::*;
pub(crate) use resume::*;
pub(crate) use sample::sample_load;
pub use sample::seed_sample_portfolio;
pub(crate) use settings::*;

const BASE: &str = "/ghost/portfolio/";
const PROJECTS: &str = "/ghost/portfolio/projects/";

async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

pub(crate) fn portfolio_settings(state: &AppState) -> Settings {
    vuuut_api::portfolio::settings(state)
}

fn public_href(s: &Settings, path: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(path))
}

fn tabs(active: &str, admin: bool, studio: bool, new_enquiries: i64) -> String {
    let tab = |key: &str, label: &str, href: &str, badge: i64| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{b}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            b = if badge > 0 { format!(r#" <span class="lm-badge">{badge}</span>"#) } else { String::new() },
        )
    };
    let mut out = String::from(r#"<div class="seg-bar rise" style="--i:1">"#);
    out.push_str(&tab("overview", "Overview", BASE, 0));
    out.push_str(&tab("projects", "Projects", PROJECTS, 0));
    out.push_str(&tab("services", "Services", "/ghost/portfolio/services/", 0));
    out.push_str(&tab("disciplines", "Disciplines", "/ghost/portfolio/disciplines/", 0));
    out.push_str(&tab("clients", "Clients", "/ghost/portfolio/clients/", 0));
    out.push_str(&tab("testimonials", "Testimonials", "/ghost/portfolio/testimonials/", 0));
    if studio {
        out.push_str(&tab("team", "Team", "/ghost/portfolio/team/", 0));
        out.push_str(&tab("resume", "Awards & press", "/ghost/portfolio/resume/", 0));
    } else {
        out.push_str(&tab("resume", "Résumé", "/ghost/portfolio/resume/", 0));
        out.push_str(&tab("team", "Collaborators", "/ghost/portfolio/team/", 0));
    }
    out.push_str(&tab("enquiries", "Enquiries", "/ghost/portfolio/enquiries/", new_enquiries));
    if admin {
        out.push_str(&tab("settings", "Settings", "/ghost/portfolio/settings/", 0));
    }
    out.push_str("</div>");
    out
}

async fn tabs_for(state: &AppState, active: &str, actor: &Actor) -> String {
    let n = repo::enquiry_counts(&state.db).await.get("new").copied().unwrap_or(0);
    tabs(active, actor.role.is_admin(), portfolio_settings(state).studio(), n)
}

fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{ic}</span><span class="dr-tile-value">{value}</span><span class="dr-tile-label">{label}</span><span class="dr-tile-detail">{detail}</span>"#,
        ic = svg(icon), value = esc(value), label = esc(label), detail = esc(detail),
    );
    match href {
        Some(h) => format!(r#"<a class="card padded dr-tile" href="{h}">{body}</a>"#),
        None => format!(r#"<div class="card padded dr-tile">{body}</div>"#),
    }
}

/// What the Overview's checklist judges.
pub(crate) async fn measure(state: &AppState) -> rules::Facts {
    let s = portfolio_settings(state);
    let (listed, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, limit: 1000, ..Default::default() }).await;
    rules::Facts {
        studio: s.studio(),
        enabled: s.enabled,
        published: listed.len() as i64,
        without_cover: listed.iter().filter(|p| p.cover.is_empty()).count() as i64,
        has_name: !s.name.is_empty() || !state.settings.get_str("title").unwrap_or_default().is_empty(),
        has_headline: !s.headline.is_empty(),
        has_about: s.about.chars().count() >= 40,
        has_contact: s.enquiries || !s.email.is_empty(),
        services: repo::services(&state.db, true).await.len() as i64,
        testimonials: repo::testimonials(&state.db).await.len() as i64,
        clients: repo::clients(&state.db).await.len() as i64,
        team: repo::team(&state.db, true).await.len() as i64,
        entries: repo::entries(&state.db).await.len() as i64,
    }
}

fn status_chip(status: &str) -> (&'static str, &'static str) {
    match rules::ProjectStatus::from_key(status) {
        rules::ProjectStatus::Published => ("published", "Published"),
        rules::ProjectStatus::Protected => ("tonal", "Password"),
        rules::ProjectStatus::Unlisted => ("scheduled", "Unlisted"),
        rules::ProjectStatus::Draft => ("draft", "Draft"),
    }
}

pub(crate) async fn portfolio_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let f = measure(&state).await;
    let list = rules::checklist(&f);
    let done = list.iter().filter(|(d, _, _)| *d).count();
    let counts = repo::status_counts(&state.db).await;
    let enq = repo::enquiry_counts(&state.db).await;
    let new_enq = enq.get("new").copied().unwrap_or(0);
    let won = enq.get("won").copied().unwrap_or(0);
    let (popular, _) = repo::projects(&state.db, &repo::ProjectQuery { listed: true, order: "popular", limit: 6, ..Default::default() }).await;
    let views: i64 = repo::projects(&state.db, &repo::ProjectQuery { limit: 1000, ..Default::default() }).await.0.iter().map(|p| p.views).sum();
    let recent = repo::enquiries(&state.db, None, 6).await;
    let since = (Utc::now() - Duration::days(89)).format("%Y-%m-%d").to_string();
    let per: HashMap<String, i64> = repo::enquiries_per_day(&state.db, &since).await.into_iter().collect();
    let series: Vec<(String, i64)> = (0..90).rev().map(|i| { let d = (Utc::now() - Duration::days(i)).format("%Y-%m-%d").to_string(); let v = per.get(&d).copied().unwrap_or(0); (d, v) }).collect();
    let peak = series.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let total90: i64 = series.iter().map(|(_, v)| v).sum();
    let chart = format!(
        r#"<div class="card padded lm-chart-card"><div class="lm-chart-head"><b>{total90}</b><span class="muted">enquiries in the last 90 days</span></div><div class="lm-chart">{}</div></div>"#,
        series.iter().map(|(d, v)| format!(r#"<span title="{d}: {v}" style="--h:{}%"></span>"#, (v * 100 / peak).max(if *v > 0 { 6 } else { 2 }))).collect::<String>()
    );
    let checklist = format!(
        r#"<div class="card padded pf-checklist"><div class="pf-progress"><b>{done} of {n}</b><span class="muted">done</span><i style="--p:{pct}%"></i></div><ul>{items}</ul></div>"#,
        n = list.len(),
        pct = done * 100 / list.len().max(1),
        items = list.iter().map(|(ok, label, place)| {
            let href = match *place { "settings" => "/ghost/portfolio/settings/", "projects" => PROJECTS, "services" => "/ghost/portfolio/services/", "clients" => "/ghost/portfolio/clients/", "team" => "/ghost/portfolio/team/", "testimonials" => "/ghost/portfolio/testimonials/", _ => "/ghost/portfolio/resume/" };
            format!(r#"<li class="{cls}"><span class="pf-tick">{ic}</span><a href="{href}">{l}</a></li>"#, cls = if *ok { "done" } else { "" }, ic = if *ok { svg("check") } else { String::new() }, l = esc(label))
        }).collect::<String>(),
    );
    let live = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>Your portfolio is live</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{path}</a> · {mode}</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/portfolio/"),
            site = esc(state.site_url().trim_end_matches('/')),
            path = esc(&s.paths.public_path("/portfolio/")),
            mode = if s.studio() { "studio portfolio" } else { "personal portfolio" },
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>The portfolio isn't public yet.</b>
<p class="muted">Add a few projects and say who you are, then switch it on in Settings.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/portfolio/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };
    let total_projects: i64 = counts.values().sum();
    let empty_state = if total_projects == 0 {
        format!(
            r#"<div class="card padded lm-empty rise" style="--i:3"><span class="ic tint-blue">{ic}</span><h3>Show your work</h3>
<p class="muted">Case studies with galleries, video and results; services and clients; a résumé or a team; testimonials; and a contact form that asks the right questions. Start from scratch, or load a sample you can edit.</p>
<div class="lm-empty-actions"><a class="btn" href="{PROJECTS}new/">{add} New project</a>{sample}</div></div>"#,
            ic = svg("work"),
            add = svg("add"),
            sample = if actor.role.is_admin() { format!(
                r#"<form method="post" action="/ghost/portfolio/sample/"><input type="hidden" name="kind" value="personal"><button class="btn tonal" type="submit">{b} Sample: a designer's portfolio</button></form>
<form method="post" action="/ghost/portfolio/sample/"><input type="hidden" name="kind" value="studio"><button class="btn tonal" type="submit">{b} Sample: a design studio</button></form>"#, b = svg("box")) } else { String::new() },
        )
    } else {
        String::new()
    };
    let tz = state.settings.get_str("timezone").unwrap_or_default().trim_matches('"').parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    let enquiry_rows = if recent.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No enquiries yet.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, recent.iter().map(|e| enquiries::row(e, tz)).collect::<String>())
    };
    let popular_rows = if popular.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">Nothing published yet.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, popular.iter().map(|p| format!(
            r#"<a class="row dr-listing" href="{PROJECTS}{id}/">{thumb}<div class="body"><span class="title">{t}</span><span class="detail">{v} views</span></div></a>"#,
            id = esc(&p.id),
            thumb = if p.cover.is_empty() { format!(r#"<span class="ic tint-blue">{}</span>"#, svg("work")) } else { format!(r#"<img class="dr-thumb" src="{}" alt="" loading="lazy">"#, esc(&p.cover)) },
            t = esc(&p.title),
            v = p.views,
        )).collect::<String>())
    };
    page_for_role(
        actor.role,
        "Portfolio",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{live}{empty_state}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}{e}</div>
<div class="lm-overview rise" style="--i:4"><div><div class="section">Getting it right</div>{checklist}<div class="section">Enquiries</div>{chart}{enquiry_rows}</div>
<div><div class="section">Most viewed</div>{popular_rows}</div></div>"#,
            head = crate::tools::tool_head("portfolio", None, &format!(r#"<a class="btn" href="{PROJECTS}new/">{} New project</a>"#, svg("add"))),
            tabs = tabs_for(&state, "overview", &actor).await,
            banner = flash_banner(&q),
            a = tile("Projects", &(counts.get("published").copied().unwrap_or(0) + counts.get("protected").copied().unwrap_or(0)).to_string(), &format!("{} drafts", counts.get("draft").copied().unwrap_or(0)), Some(PROJECTS), "work", "blue"),
            b = tile("Views", &views.to_string(), "Case study page views", None, "eye", "teal"),
            c = tile("New enquiries", &new_enq.to_string(), "Waiting for a reply", Some("/ghost/portfolio/enquiries/?status=new"), "mail", if new_enq > 0 { "amber" } else { "slate" }),
            d = tile("Won", &won.to_string(), "Enquiries that became work", Some("/ghost/portfolio/enquiries/?status=won"), "check", "green"),
            e = tile("Testimonials", &f.testimonials.to_string(), "Kind words on the site", Some("/ghost/portfolio/testimonials/"), "star", "pink"),
        ),
        true,
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studios_and_people_get_their_own_tabs() {
        let p = tabs("overview", true, false, 0);
        assert!(p.contains("Résumé") && p.contains("Collaborators") && p.contains("Settings"));
        let s = tabs("overview", false, true, 2);
        assert!(s.contains("Awards &amp; press") && s.contains(r#">Team<"#) && !s.contains("Settings"));
        assert!(s.contains(r#"<span class="lm-badge">2</span>"#));
    }

    #[test]
    fn every_status_has_a_chip() {
        for st in rules::ProjectStatus::ALL {
            assert!(!status_chip(st.key()).1.is_empty());
        }
    }
}
