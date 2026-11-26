//! The public podcast pages: `/podcast/` (or wherever Settings moves them).
//!
//! Rendered inside the active theme like the store, the directory, the LMS
//! and booking. A theme can take over any screen — `podcast/index.hbs`,
//! `podcast/show.hbs`, `podcast/episode.hbs`, `podcast/person.hbs` — and is
//! handed `podcast.style_url` / `podcast.script_url` so its markup can use
//! the same player (see the `data-vp-*` attributes in `podcast.js`). While
//! the tool is off every address here is the theme's 404.
//!
//! * `GET /podcast/` — every show; with one show, that show's page.
//! * `GET /podcast/{show}/` — artwork, "listen on", hosts, the latest
//!   episode, seasons, search and the episode list (`?season=`, `?q=`,
//!   `?sort=oldest`, `?page=`), reviews and sponsors.
//! * `GET /podcast/{show}/feed.xml` — the RSS feed: public episodes only.
//! * `GET /podcast/{show}/{episode}/` — the player, notes, chapters,
//!   transcript, guests, sponsors, and the episodes either side. A
//!   members-only episode shows its summary and a sign-in or upgrade
//!   prompt; the audio address is never in the page.
//! * `…/chapters.json`, `…/transcript.txt` — Podcasting 2.0 files.
//! * `GET /podcast/play/{id}.{ext}` — counts the listen (one per listener,
//!   episode and day; bots never) and redirects to the audio. Gated
//!   episodes need a signed-in member who may listen, or the `k` token from
//!   that member's private feed. The redirect hides the address from the
//!   feed; it does not make a public file private.
//! * `GET /podcast/private/{token}/{show}.xml` — a member's private feed,
//!   with every episode their membership opens. `GET /podcast/{show}/private/`
//!   shows a signed-in member their address.
//! * `GET|POST /podcast/{show}/contact/` — guest applications and listener
//!   questions: same-origin, a honeypot, a fill time, a per-address budget
//!   and three a day per email.
//! * `GET /podcast/people/{slug}/` — a host or guest and their episodes.

use crate::routes::render_virtual_page_as;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Form, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use vuuut_api::AppState;
use vuuut_core::podcast::{self as rules, Settings};
use vuuut_db::repos::podcast as repo;

const BASE: &str = "/podcast/";
static CSS: &str = include_str!("../assets/podcast.css");
static JS: &str = include_str!("../assets/podcast.js");

fn version(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}
fn css_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| version(CSS));
    &V
}
fn js_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| version(JS));
    &V
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/podcast/", get(index))
        .route("/podcast", get(|| async { Redirect::permanent(&link(BASE)) }))
        .route("/podcast/assets/{v}/podcast.css", get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response() }))
        .route("/podcast/assets/{v}/podcast.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], JS).into_response() }))
        .route("/podcast/play/{file}", get(play))
        .route("/podcast/people/{slug}/", get(person_page))
        .route("/podcast/private/{token}/{file}", get(private_feed))
        .route("/podcast/{show}/", get(show_page))
        .route("/podcast/{show}", get(|Path(show): Path<String>| async move { Redirect::permanent(&link(&format!("/podcast/{show}/"))) }))
        .route("/podcast/{show}/feed.xml", get(feed))
        .route("/podcast/{show}/feed/", get(|Path(show): Path<String>| async move { Redirect::permanent(&link(&format!("/podcast/{show}/feed.xml"))) }))
        .route("/podcast/{show}/private/", get(private_page))
        .route("/podcast/{show}/contact/", get(contact_page).post(contact_submit))
        .route("/podcast/{show}/{episode}/", get(episode_page))
        .route("/podcast/{show}/{episode}/chapters.json", get(chapters_json))
        .route("/podcast/{show}/{episode}/transcript.txt", get(transcript))
}

fn settings(state: &AppState) -> Settings {
    vuuut_api::podcast::settings(state)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

tokio::task_local! {
    static PATHS: rules::Paths;
}

fn public(path: &str) -> String {
    PATHS.try_with(|p| p.public_path(path)).unwrap_or_else(|_| path.into())
}

fn link(path: &str) -> String {
    vuuut_core::url::base(&public(path))
}

fn absolute(state: &AppState, url: &str) -> String {
    vuuut_core::url::absolute(&state.site_url(), url)
}

/// Moves the podcast pages to the address chosen in Settings.
pub(crate) async fn addresses(State(state): State<AppState>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let paths = rules::Paths::read(|k| state.settings.get_str(k));
    if paths != rules::Paths::default() {
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        if let Some(internal) = paths.internal_path(&path) {
            if let Ok(uri) = format!("{internal}{query}").parse::<axum::http::Uri>() {
                *req.uri_mut() = uri;
            }
        } else if (path == "/podcast" || path.starts_with(BASE)) && req.method() == axum::http::Method::GET {
            return Redirect::permanent(&format!("{}{query}", vuuut_core::url::base(&paths.public_path(&path)))).into_response();
        }
    }
    PATHS.scope(paths, next.run(req)).await
}

async fn not_found(state: &AppState, path: &str, headers: HeaderMap) -> Response {
    crate::render::error_page(state, StatusCode::NOT_FOUND, &public(path), headers).await
}

fn style_url() -> String {
    link(&format!("/podcast/assets/{}/podcast.css", css_version()))
}

fn script_url() -> String {
    link(&format!("/podcast/assets/{}/podcast.js", js_version()))
}

fn assets() -> String {
    format!(r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script>"#, style_url(), script_url())
}

async fn theme_templates(state: &AppState, names: &[&str]) -> Vec<String> {
    let theme = state.theme.read().await.clone();
    names.iter().filter(|n| theme.has_template(n)).map(|n| n.to_string()).collect()
}

fn podcast_json(s: &Settings) -> Value {
    json!({ "title": s.title, "intro": s.intro, "url": link(BASE), "style_url": style_url(), "script_url": script_url(), "requests": s.requests })
}

/// `podcast` on every page while the tool is on, so a theme's homepage and
/// header can link to it and load the player.
pub(crate) fn site_podcast(state: &AppState) -> Option<Value> {
    let s = settings(state);
    s.enabled.then(|| podcast_json(&s))
}

fn member_status(member: &Value) -> Option<String> {
    member.get("status").and_then(Value::as_str).map(str::to_string)
}

/// What the visitor in hand may hear.
fn can_listen(e: &repo::Episode, member: &Value) -> bool {
    e.access().allows(member_status(member).as_deref())
}

/// A live show by its address (hidden shows answer by link too).
async fn live_show(state: &AppState, slug: &str) -> Option<repo::Show> {
    repo::show_by_slug(&state.db, &vuuut_core::directory::slugify(slug)).await.filter(|s| s.public())
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

fn art(url: &str, title: &str, class: &str) -> String {
    if url.is_empty() {
        format!(r#"<span class="{class} vp-art-blank" aria-hidden="true">{}</span>"#, esc(&initials(title)))
    } else {
        format!(r#"<img class="{class}" src="{}" alt="" loading="lazy">"#, esc(url))
    }
}

fn icon(name: &str) -> &'static str {
    match name {
        "play" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.14v13.72a1 1 0 0 0 1.5.86l11-6.86a1 1 0 0 0 0-1.72l-11-6.86A1 1 0 0 0 8 5.14z"/></svg>"#,
        "lock" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M17 9h-1V7a4 4 0 0 0-8 0v2H7a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2zm-7-2a2 2 0 1 1 4 0v2h-4V7z"/></svg>"#,
        "rss" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.18 15.64a2.18 2.18 0 1 1 0 4.36 2.18 2.18 0 0 1 0-4.36zM4 4.44A15.56 15.56 0 0 1 19.56 20h-2.83A12.73 12.73 0 0 0 4 7.27V4.44zm0 5.66a9.9 9.9 0 0 1 9.9 9.9h-2.83A7.07 7.07 0 0 0 4 12.93V10.1z"/></svg>"#,
        "heart" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09A6 6 0 0 1 16.5 3C19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/></svg>"#,
        "mic" => r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11h-2z"/></svg>"#,
        _ => "",
    }
}

/// The `data-*` that make an element play an episode in the dock.
fn play_attrs(s: &Settings, show: &repo::Show, e: &repo::Episode) -> String {
    let art = if e.artwork.is_empty() { show.artwork.as_str() } else { e.artwork.as_str() };
    format!(
        r#"data-vp-play data-id="{id}" data-src="{src}?s=web" data-title="{t}" data-show="{sh}" data-art="{a}" data-url="{u}" data-dur="{d}""#,
        id = esc(&e.id),
        src = esc(&repo::play_url(s, e)),
        t = esc(&e.title),
        sh = esc(&show.title),
        a = esc(art),
        u = esc(&repo::episode_url(s, show, e)),
        d = e.duration,
    )
}

fn date_label(e: &repo::Episode) -> String {
    e.published_at.as_deref().and_then(vuuut_core::dates::from_db_string).map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default()
}

fn episode_row(s: &Settings, show: &repo::Show, e: &repo::Episode, member: &Value) -> String {
    let open = can_listen(e, member);
    let label = e.label();
    let button = if open && !e.audio_url.is_empty() {
        format!(r#"<button class="vp-row-play" type="button" {attrs} aria-label="Play {t}">{i}</button>"#, attrs = play_attrs(s, show, e), t = esc(&e.title), i = icon("play"))
    } else {
        format!(r#"<a class="vp-row-play vp-locked" href="{u}" aria-label="Members only">{i}</a>"#, u = esc(&repo::episode_url(s, show, e)), i = icon("lock"))
    };
    format!(
        r#"<article class="vp-row">{art}<div class="vp-row-body"><div class="vp-meta">{label}<span>{date}</span>{dur}{lock}</div>
<h3><a href="{url}">{title}</a></h3>{summary}</div>{button}</article>"#,
        art = art(if e.artwork.is_empty() { &show.artwork } else { &e.artwork }, &e.title, "vp-row-art"),
        label = if label.is_empty() { String::new() } else { format!(r#"<span class="vp-chip">{}</span>"#, esc(&label)) },
        date = esc(&date_label(e)),
        dur = if e.duration > 0 { format!("<span>{}</span>", esc(&rules::duration_label(e.duration))) } else { String::new() },
        lock = if e.access != "public" { format!(r#"<span class="vp-chip vp-chip-lock">{} {}</span>"#, icon("lock"), if e.access == "paid" { "Paid" } else { "Members" }) } else { String::new() },
        url = esc(&repo::episode_url(s, show, e)),
        title = esc(&e.title),
        summary = if e.summary.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&e.summary)) },
    )
}

fn listen_on(s: &Settings, show: &repo::Show) -> String {
    let feed = repo::feed_url(s, show);
    let mut out: String = show.links().iter().map(|(k, u)| format!(r#"<a class="vp-pill" href="{u}" target="_blank" rel="noopener">{l}</a>"#, u = esc(u), l = esc(rules::platform_label(k)))).collect();
    out.push_str(&format!(r#"<a class="vp-pill vp-pill-rss" href="{f}" data-vp-copy="{f}" title="Copy the RSS feed address">{i} RSS</a>"#, f = esc(&feed), i = icon("rss")));
    format!(r#"<div class="vp-listen" aria-label="Listen on">{out}</div>"#)
}

fn people_strip(s: &Settings, people: &[(repo::Person, String)]) -> String {
    if people.is_empty() {
        return String::new();
    }
    let items: String = people
        .iter()
        .map(|(p, role)| {
            format!(
                r#"<a class="vp-person" href="{u}">{img}<span><b>{n}</b><small>{r}</small></span></a>"#,
                u = esc(&repo::person_url(s, p)),
                img = if p.photo.is_empty() { format!(r#"<i class="vp-face">{}</i>"#, esc(&p.initials())) } else { format!(r#"<img class="vp-face" src="{}" alt="" loading="lazy">"#, esc(&p.photo)) },
                n = esc(&p.name),
                r = esc(if p.title.is_empty() { rules::Role::from_key(role).label() } else { &p.title }),
            )
        })
        .collect();
    format!(r#"<div class="vp-people">{items}</div>"#)
}

fn pager(base: &str, page: i64, pages: i64, keep: &str) -> String {
    if pages <= 1 {
        return String::new();
    }
    let href = |p: i64| format!("{base}?page={p}{keep}#episodes");
    format!(
        r#"<nav class="vp-pager">{prev}<span>Page {page} of {pages}</span>{next}</nav>"#,
        prev = if page > 1 { format!(r#"<a href="{}">← Newer</a>"#, esc(&href(page - 1))) } else { "<span></span>".into() },
        next = if page < pages { format!(r#"<a href="{}">Older →</a>"#, esc(&href(page + 1))) } else { "<span></span>".into() },
    )
}

// ---------------------------------------------------------------------------
// the hub and a show
// ---------------------------------------------------------------------------

async fn index(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    if !s.enabled {
        return not_found(&state, BASE, headers).await;
    }
    let shows = repo::shows(&state.db, true).await;
    if shows.len() == 1 {
        return render_show(&state, headers, shows[0].clone(), &q, true).await;
    }
    let counts = repo::episode_counts(&state.db).await;
    let member = crate::render::member_context(&state, &headers).await;
    let (latest, _) = repo::episodes(&state.db, &repo::EpisodeQuery { live: true, limit: 8, ..Default::default() }).await;
    let templates = theme_templates(&state, &["podcast/index"]).await;
    if !templates.is_empty() {
        let mut list = Vec::new();
        for sh in &shows {
            let hosts = repo::credits(&state.db, "show", &sh.id).await;
            list.push(repo::show_json(&s, sh, &hosts, counts.get(&sh.id).map(|c| c.0).unwrap_or(0)));
        }
        let mut eps = Vec::new();
        for e in &latest {
            if let Some(sh) = shows.iter().find(|x| x.id == e.show_id) {
                eps.push(repo::episode_json(&s, sh, e, &[], &[], can_listen(e, &member)));
            }
        }
        let mut data = serde_json::Map::new();
        data.insert("podcast".into(), podcast_json(&s));
        data.insert("shows".into(), json!(list));
        data.insert("episodes".into(), json!(eps));
        return render_virtual_page_as(&state, &headers, &public(BASE), &s.title, &s.intro, String::new(), None, false, &templates, data, &["podcast"]).await;
    }
    let cards: String = shows
        .iter()
        .map(|sh| {
            let n = counts.get(&sh.id).map(|c| c.0).unwrap_or(0);
            format!(
                r#"<a class="vp-show-card" href="{u}">{art}<span class="vp-show-card-body"><b>{t}</b><small>{a}</small><span>{sub}</span><em>{n} episode{pl}</em></span></a>"#,
                u = esc(&repo::show_url(&s, sh)),
                art = art(&sh.artwork, &sh.title, "vp-show-art"),
                t = esc(&sh.title),
                a = esc(&sh.author),
                sub = esc(if sh.subtitle.is_empty() { &sh.description } else { &sh.subtitle }),
                pl = if n == 1 { "" } else { "s" },
            )
        })
        .collect();
    let rows: String = latest.iter().filter_map(|e| shows.iter().find(|x| x.id == e.show_id).map(|sh| episode_row(&s, sh, e, &member))).collect();
    let body = format!(
        r#"{assets}<div class="vp"><header class="vp-hub-head"><h1>{title}</h1>{intro}</header>
{shows}{latest}</div>"#,
        assets = assets(),
        title = esc(&s.title),
        intro = if s.intro.is_empty() { String::new() } else { format!(r#"<p class="vp-lede">{}</p>"#, esc(&s.intro)) },
        shows = if cards.is_empty() { r#"<p class="vp-empty">No shows yet.</p>"#.to_string() } else { format!(r#"<div class="vp-shows">{cards}</div>"#) },
        latest = if rows.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>Latest episodes</h2><div class="vp-list">{rows}</div></section>"#) },
    );
    render_virtual_page_as(&state, &headers, &public(BASE), &s.title, &s.intro, body, None, false, &[], serde_json::Map::new(), &["podcast"]).await
}

async fn show_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(show) = live_show(&state, &slug).await else { return not_found(&state, &path, headers).await };
    render_show(&state, headers, show, &q, false).await
}

fn show_structured_data(state: &AppState, s: &Settings, show: &repo::Show, hosts: &[(repo::Person, String)]) -> String {
    let mut v = json!({
        "@context": "https://schema.org", "@type": "PodcastSeries", "name": show.title, "description": show.description,
        "url": absolute(state, &repo::show_url(s, show)), "webFeed": absolute(state, &repo::feed_url(s, show)),
    });
    if !show.artwork.is_empty() {
        v["image"] = json!(absolute(state, &show.artwork));
    }
    if !show.author.is_empty() {
        v["author"] = json!({ "@type": "Person", "name": show.author });
    }
    if !hosts.is_empty() {
        v["actor"] = json!(hosts.iter().map(|(p, _)| json!({ "@type": "Person", "name": p.name })).collect::<Vec<_>>());
    }
    format!(r#"<script type="application/ld+json">{}</script>"#, v.to_string().replace("</", "<\\/"))
}

async fn render_show(state: &AppState, headers: HeaderMap, show: repo::Show, q: &HashMap<String, String>, at_hub: bool) -> Response {
    let s = settings(state);
    let member = crate::render::member_context(state, &headers).await;
    let hosts = repo::credits(&state.db, "show", &show.id).await;
    let seasons = repo::seasons(&state.db, &show.id).await;
    let season = q.get("season").and_then(|v| v.parse::<i64>().ok()).filter(|n| seasons.contains(n));
    let search = q.get("q").map(|v| vuuut_core::directory::clean(v, 80)).filter(|v| !v.is_empty());
    let oldest = q.get("sort").map(String::as_str) == Some("oldest") || (show.show_type == "serial" && q.get("sort").map(String::as_str) != Some("newest"));
    let page = q.get("page").and_then(|p| p.parse::<i64>().ok()).unwrap_or(1).clamp(1, 10_000);
    let (rows, total) = repo::episodes(&state.db, &repo::EpisodeQuery {
        show_id: Some(&show.id), live: true, season, search: search.as_deref(), order: if oldest { "oldest" } else { "newest" },
        limit: s.per_page, offset: (page - 1) * s.per_page, ..Default::default()
    }).await;
    let pages = (total + s.per_page - 1) / s.per_page;
    let (newest, _) = repo::episodes(&state.db, &repo::EpisodeQuery { show_id: Some(&show.id), live: true, limit: 1, ..Default::default() }).await;
    let (trailers, _) = repo::episodes(&state.db, &repo::EpisodeQuery { show_id: Some(&show.id), live: true, limit: 50, ..Default::default() }).await;
    let trailer = trailers.iter().find(|e| e.episode_type == "trailer").cloned();
    let reviews = repo::reviews(&state.db, Some(&show.id)).await;
    let counts = repo::episode_counts(&state.db).await;
    let live_count = counts.get(&show.id).map(|c| c.0).unwrap_or(0);
    let gated = trailers.iter().any(|e| e.access != "public");
    let url = if at_hub { link(BASE) } else { repo::show_url(&s, &show) };

    let templates = theme_templates(state, &["podcast/show"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("podcast".into(), podcast_json(&s));
        data.insert("show".into(), repo::show_json(&s, &show, &hosts, live_count));
        data.insert("episodes".into(), json!(rows.iter().map(|e| repo::episode_json(&s, &show, e, &[], &[], can_listen(e, &member))).collect::<Vec<_>>()));
        data.insert("latest".into(), newest.first().map(|e| repo::episode_json(&s, &show, e, &[], &[], can_listen(e, &member))).unwrap_or(Value::Null));
        data.insert("trailer".into(), trailer.as_ref().map(|e| repo::episode_json(&s, &show, e, &[], &[], can_listen(e, &member))).unwrap_or(Value::Null));
        data.insert("seasons".into(), json!(seasons.iter().map(|n| json!({ "number": n, "url": format!("{url}?season={n}#episodes"), "current": season == Some(*n) })).collect::<Vec<_>>()));
        data.insert("reviews".into(), json!(reviews.iter().map(repo::review_json).collect::<Vec<_>>()));
        data.insert("pagination".into(), json!({ "page": page, "pages": pages, "total": total, "prev": if page > 1 { json!(format!("{url}?page={}", page - 1)) } else { Value::Null }, "next": if page < pages { json!(format!("{url}?page={}", page + 1)) } else { Value::Null } }));
        data.insert("private_url".into(), if gated { json!(link(&format!("/podcast/{}/private/", show.slug))) } else { Value::Null });
        data.insert("contact_url".into(), if s.requests { json!(link(&format!("/podcast/{}/contact/", show.slug))) } else { Value::Null });
        return render_virtual_page_as(state, &headers, &public(&url), &show.title, &show.subtitle, String::new(), (!show.artwork.is_empty()).then(|| show.artwork.clone()), false, &templates, data, &["podcast"]).await;
    }

    let latest = newest.first().filter(|_| page == 1 && season.is_none() && search.is_none()).map(|e| {
        let open = can_listen(e, &member);
        format!(
            r#"<section class="vp-latest">{art}<div class="vp-latest-body"><span class="vp-kicker">Latest episode{label}</span><h2><a href="{u}">{t}</a></h2>
<p class="vp-meta-line">{date}{dur}</p>{sum}<div class="vp-actions">{btn}<a class="vp-btn vp-btn-ghost" href="{u}">Show notes</a></div></div></section>"#,
            art = art(if e.artwork.is_empty() { &show.artwork } else { &e.artwork }, &e.title, "vp-latest-art"),
            label = if e.label().is_empty() { String::new() } else { format!(" · {}", esc(&e.label())) },
            u = esc(&repo::episode_url(&s, &show, e)),
            t = esc(&e.title),
            date = esc(&date_label(e)),
            dur = if e.duration > 0 { format!(" · {}", esc(&rules::duration_label(e.duration))) } else { String::new() },
            sum = if e.summary.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&e.summary)) },
            btn = if open && !e.audio_url.is_empty() { format!(r#"<button class="vp-btn" type="button" {}>{} Play episode</button>"#, play_attrs(&s, &show, e), icon("play")) } else { format!(r#"<a class="vp-btn" href="{}">{} Members only</a>"#, esc(&repo::episode_url(&s, &show, e)), icon("lock")) },
        )
    }).unwrap_or_default();
    let keep = format!("{}{}{}", season.map(|n| format!("&season={n}")).unwrap_or_default(), search.as_ref().map(|v| format!("&q={}", vuuut_core::social_media::encode(v))).unwrap_or_default(), if oldest && show.show_type != "serial" { "&sort=oldest" } else { "" });
    let season_tabs = if seasons.len() > 1 {
        let mut t = format!(r#"<a class="vp-tab{}" href="{}#episodes">All</a>"#, if season.is_none() { " on" } else { "" }, esc(&url));
        for n in &seasons {
            t.push_str(&format!(r#"<a class="vp-tab{on}" href="{u}?season={n}#episodes">Season {n}</a>"#, on = if season == Some(*n) { " on" } else { "" }, u = esc(&url)));
        }
        format!(r#"<nav class="vp-tabs" aria-label="Seasons">{t}</nav>"#)
    } else {
        String::new()
    };
    let list: String = rows.iter().map(|e| episode_row(&s, &show, e, &member)).collect();
    let review_html = if reviews.is_empty() {
        String::new()
    } else {
        format!(
            r#"<section class="vp-section"><h2>What listeners say</h2><div class="vp-reviews">{}</div></section>"#,
            reviews.iter().take(6).map(|r| format!(
                r#"<figure class="vp-review"><span class="vp-stars" aria-label="{n} out of 5">{stars}</span>{title}<blockquote>{body}</blockquote><figcaption>{author}<small>{src}</small></figcaption></figure>"#,
                n = r.rating,
                stars = "★".repeat(r.rating.clamp(0, 5) as usize),
                title = if r.title.is_empty() { String::new() } else { format!("<b>{}</b>", esc(&r.title)) },
                body = esc(&r.body),
                author = esc(if r.author.is_empty() { "A listener" } else { &r.author }),
                src = esc(rules::REVIEW_SOURCES.iter().find(|(k, _)| *k == r.source).map(|(_, l)| *l).unwrap_or("")),
            )).collect::<String>()
        )
    };
    let cta = {
        let mut c = String::new();
        if s.requests {
            c.push_str(&format!(r#"<a class="vp-cta" href="{}">{} <span><b>Be on the show</b><small>Pitch yourself as a guest, or send a question.</small></span></a>"#, esc(&link(&format!("/podcast/{}/contact/", show.slug))), icon("mic")));
        }
        if gated {
            c.push_str(&format!(r#"<a class="vp-cta" href="{}">{} <span><b>Members' feed</b><small>Every episode, in your podcast app.</small></span></a>"#, esc(&link(&format!("/podcast/{}/private/", show.slug))), icon("lock")));
        }
        if !show.funding_url.is_empty() {
            c.push_str(&format!(r#"<a class="vp-cta" href="{}" target="_blank" rel="noopener">{} <span><b>{}</b><small>Keep the show going.</small></span></a>"#, esc(&show.funding_url), icon("heart"), esc(if show.funding_label.is_empty() { "Support the show" } else { &show.funding_label })));
        }
        if c.is_empty() { c } else { format!(r#"<div class="vp-ctas">{c}</div>"#) }
    };
    let body = format!(
        r#"{assets}{ld}<div class="vp vp-tint-{tint}">
<header class="vp-show-head">{art}<div class="vp-show-head-body">{kicker}<h1>{title}</h1>{sub}<p class="vp-meta-line">{author}{cat}{count}</p>{listen}</div></header>
{desc}{hosts}{latest}{trailer}
<section class="vp-section" id="episodes"><div class="vp-section-head"><h2>{eps_title}</h2>
<form class="vp-search" method="get" action="{url}#episodes">{season_keep}<input type="search" name="q" value="{qv}" placeholder="Search episodes" aria-label="Search episodes"></form></div>
{season_tabs}{found}<div class="vp-list">{list}</div>{pager}</section>
{reviews}{cta}</div>"#,
        assets = assets(),
        ld = show_structured_data(state, &s, &show, &hosts),
        tint = esc(&show.tint),
        art = art(&show.artwork, &show.title, "vp-show-cover"),
        kicker = if show.category.is_empty() { String::new() } else { format!(r#"<span class="vp-kicker">{}</span>"#, esc(&show.category.replace(" > ", " · "))) },
        title = esc(&show.title),
        sub = if show.subtitle.is_empty() { String::new() } else { format!(r#"<p class="vp-lede">{}</p>"#, esc(&show.subtitle)) },
        author = esc(&show.author),
        cat = if show.explicit != 0 { r#" · <span class="vp-chip">Explicit</span>"#.to_string() } else { String::new() },
        count = format!("{}{} episode{}", if show.author.is_empty() { "" } else { " · " }, live_count, if live_count == 1 { "" } else { "s" }),
        listen = listen_on(&s, &show),
        desc = if show.description.is_empty() { String::new() } else { format!(r#"<div class="vp-about">{}</div>"#, vuuut_core::directory::paragraphs(&show.description)) },
        hosts = if hosts.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>Hosted by</h2>{}</section>"#, people_strip(&s, &hosts)) },
        trailer = trailer.as_ref().filter(|t| newest.first().is_none_or(|n| n.id != t.id) && page == 1 && season.is_none()).filter(|e| can_listen(e, &member) && !e.audio_url.is_empty()).map(|e| format!(
            r#"<p class="vp-trailer"><button class="vp-btn vp-btn-ghost" type="button" {}>{} New here? Play the trailer</button></p>"#, play_attrs(&s, &show, e), icon("play"))).unwrap_or_default(),
        eps_title = if season.is_some() { format!("Season {}", season.unwrap_or(0)) } else { "All episodes".into() },
        url = esc(&url),
        season_keep = season.map(|n| format!(r#"<input type="hidden" name="season" value="{n}">"#)).unwrap_or_default(),
        qv = esc(search.as_deref().unwrap_or("")),
        found = match &search { Some(v) => format!(r#"<p class="vp-found">{total} episode{} matching “{}” · <a href="{}#episodes">Clear</a></p>"#, if total == 1 { "" } else { "s" }, esc(v), esc(&url)), None => String::new() },
        list = if list.is_empty() { r#"<p class="vp-empty">No episodes here yet.</p>"#.to_string() } else { list },
        pager = pager(&url, page, pages, &keep),
        reviews = review_html,
    );
    render_virtual_page_as(state, &headers, &public(&url), &show.title, if show.subtitle.is_empty() { &show.description } else { &show.subtitle }, body, None, false, &[], serde_json::Map::new(), &["podcast"]).await
}

// ---------------------------------------------------------------------------
// an episode
// ---------------------------------------------------------------------------

async fn find(state: &AppState, show: &str, episode: &str) -> Option<(repo::Show, repo::Episode)> {
    let sh = live_show(state, show).await?;
    let e = repo::episode_by_slug(&state.db, &sh.id, &vuuut_core::directory::slugify(episode)).await.filter(repo::Episode::is_live)?;
    Some((sh, e))
}

fn transcript_html(t: &str) -> String {
    let cues = rules::cues(t);
    if cues.is_empty() {
        return String::new();
    }
    let body: String = cues
        .iter()
        .map(|c| {
            format!(
                r#"<p class="vp-cue">{time}{who}<span>{text}</span></p>"#,
                time = c.start.map(|n| format!(r#"<button type="button" class="vp-ts" data-vp-seek="{n}">{}</button>"#, rules::clock(n))).unwrap_or_default(),
                who = if c.speaker.is_empty() { String::new() } else { format!("<b>{}</b>", esc(&c.speaker)) },
                text = esc(&c.text),
            )
        })
        .collect();
    format!(
        r#"<details class="vp-transcript" id="transcript"><summary>Transcript</summary><div class="vp-transcript-tools"><input type="search" placeholder="Search the transcript" data-vp-filter aria-label="Search the transcript"></div><div class="vp-cues">{body}</div></details>"#
    )
}

fn episode_structured_data(state: &AppState, s: &Settings, show: &repo::Show, e: &repo::Episode, open: bool) -> String {
    let mut v = json!({
        "@context": "https://schema.org", "@type": "PodcastEpisode", "name": e.title, "description": e.summary,
        "url": absolute(state, &repo::episode_url(s, show, e)),
        "partOfSeries": { "@type": "PodcastSeries", "name": show.title, "url": absolute(state, &repo::show_url(s, show)) },
    });
    if let Some(d) = e.published_at.as_deref().and_then(vuuut_core::dates::from_db_string) {
        v["datePublished"] = json!(d.format("%Y-%m-%d").to_string());
    }
    if e.duration > 0 {
        v["timeRequired"] = json!(format!("PT{}M{}S", e.duration / 60, e.duration % 60));
    }
    if e.number > 0 {
        v["episodeNumber"] = json!(e.number);
    }
    if open && !e.audio_url.is_empty() {
        v["associatedMedia"] = json!({ "@type": "MediaObject", "contentUrl": absolute(state, &repo::play_url(s, e)) });
    }
    format!(r#"<script type="application/ld+json">{}</script>"#, v.to_string().replace("</", "<\\/"))
}

async fn episode_page(State(state): State<AppState>, headers: HeaderMap, Path((show_slug, ep_slug)): Path<(String, String)>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{show_slug}/{ep_slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some((show, e)) = find(&state, &show_slug, &ep_slug).await else { return not_found(&state, &path, headers).await };
    let member = crate::render::member_context(&state, &headers).await;
    let open = can_listen(&e, &member);
    let people = repo::credits(&state.db, "episode", &e.id).await;
    let sponsors = repo::episode_sponsors(&state.db, &e.id).await;
    let (older, newer) = repo::adjacent(&state.db, &e).await;
    let url = repo::episode_url(&s, &show, &e);
    let artwork = if e.artwork.is_empty() { show.artwork.clone() } else { e.artwork.clone() };
    let title = if e.seo_title.is_empty() { e.title.clone() } else { e.seo_title.clone() };
    let excerpt = if e.seo_description.is_empty() { e.summary.clone() } else { e.seo_description.clone() };

    let templates = theme_templates(&state, &["podcast/episode"]).await;
    if !templates.is_empty() {
        let hosts = repo::credits(&state.db, "show", &show.id).await;
        let mut ej = repo::episode_json(&s, &show, &e, &people, &sponsors, open);
        if open {
            ej["transcript"] = json!(rules::cues(&e.transcript).iter().map(|c| json!({ "start": c.start, "time": c.start.map(rules::clock), "speaker": c.speaker, "text": c.text })).collect::<Vec<_>>());
        }
        let mut data = serde_json::Map::new();
        data.insert("podcast".into(), podcast_json(&s));
        data.insert("show".into(), repo::show_json(&s, &show, &hosts, 0));
        data.insert("episode".into(), ej);
        data.insert("older".into(), older.as_ref().map(|x| repo::episode_json(&s, &show, x, &[], &[], can_listen(x, &member))).unwrap_or(Value::Null));
        data.insert("newer".into(), newer.as_ref().map(|x| repo::episode_json(&s, &show, x, &[], &[], can_listen(x, &member))).unwrap_or(Value::Null));
        data.insert("signin_url".into(), json!(vuuut_core::url::base("/#/portal/signin")));
        data.insert("upgrade_url".into(), json!(vuuut_core::url::base(if member.is_null() { "/#/portal/signup" } else { "/#/portal/account/plans" })));
        return render_virtual_page_as(&state, &headers, &public(&url), &title, &excerpt, String::new(), (!artwork.is_empty()).then(|| artwork.clone()), false, &templates, data, &["podcast"]).await;
    }

    let label = e.label();
    let player = if open && !e.audio_url.is_empty() {
        format!(
            r#"<div class="vp-stage" data-vp-stage {attrs}><button class="vp-stage-play" type="button" data-vp-toggle aria-label="Play">{play}</button>
<div class="vp-stage-body"><b>{t}</b><span>{dur}</span><div class="vp-stage-bar" data-vp-bar><span></span></div></div>
<audio class="vp-native" controls preload="none" src="{src}"></audio></div>
<div class="vp-tools"><a href="{src}" download>Download{size}</a><button type="button" data-vp-share>Share from current time</button>{transcript_link}</div>"#,
            attrs = play_attrs(&s, &show, &e),
            play = icon("play"),
            t = esc(&e.title),
            dur = esc(&rules::clock(e.duration)),
            src = esc(&repo::play_url(&s, &e)),
            size = if e.audio_bytes > 0 { format!(" · {}", esc(&rules::size_label(e.audio_bytes))) } else { String::new() },
            transcript_link = if e.transcript.trim().is_empty() { String::new() } else { r##"<a href="#transcript" data-vp-open-transcript>Transcript</a>"##.to_string() },
        )
    } else {
        let signed_in = !member.is_null();
        format!(
            r#"<div class="vp-gate">{lock}<div><b>{head}</b><p>{text}</p></div><a class="vp-btn" href="{href}">{btn}</a></div>"#,
            lock = icon("lock"),
            head = if e.access == "paid" { "This episode is for paying members" } else { "This episode is for members" },
            text = if signed_in { "Upgrade your membership to listen here and in your own podcast app." } else { "Sign in or join to listen here and in your own podcast app." },
            href = esc(&vuuut_core::url::base(if signed_in { "/#/portal/account/plans" } else if e.access == "paid" { "/#/portal/signup" } else { "/#/portal/signin" })),
            btn = if signed_in { "Upgrade" } else if e.access == "paid" { "Join" } else { "Sign in" },
        )
    };
    let chapters = e.chapters();
    let chapters_html = if open && !chapters.is_empty() {
        format!(
            r#"<section class="vp-section"><h2>Chapters</h2><ol class="vp-chapters">{}</ol></section>"#,
            chapters.iter().map(|c| format!(
                r#"<li><button type="button" class="vp-ts" data-vp-seek="{n}">{t}</button><span>{title}</span>{link}</li>"#,
                n = c.start,
                t = rules::clock(c.start),
                title = esc(&c.title),
                link = if c.url.is_empty() { String::new() } else { format!(r#"<a href="{}" target="_blank" rel="noopener">Link</a>"#, esc(&c.url)) },
            )).collect::<String>()
        )
    } else {
        String::new()
    };
    let video = if open { vuuut_core::lms::video_embed(&e.video_url).map(|(u, frame)| if frame {
        format!(r#"<div class="vp-video"><iframe src="{}" title="Video" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen loading="lazy"></iframe></div>"#, esc(&u))
    } else {
        format!(r#"<div class="vp-video"><video src="{}" controls preload="metadata"></video></div>"#, esc(&u))
    }).unwrap_or_default() } else { String::new() };
    let sponsor_html = if sponsors.is_empty() { String::new() } else {
        format!(
            r#"<section class="vp-section"><h2>Thanks to our sponsors</h2><div class="vp-sponsors">{}</div></section>"#,
            sponsors.iter().map(|sp| format!(
                r#"<{tag} class="vp-sponsor"{href}>{logo}<span><b>{n}</b>{tag_line}{code}</span></{tag}>"#,
                tag = if sp.url.is_empty() { "div" } else { "a" },
                href = if sp.url.is_empty() { String::new() } else { format!(r#" href="{}" target="_blank" rel="sponsored noopener""#, esc(&sp.url)) },
                logo = if sp.logo.is_empty() { String::new() } else { format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(&sp.logo)) },
                n = esc(&sp.name),
                tag_line = if sp.tagline.is_empty() { String::new() } else { format!("<small>{}</small>", esc(&sp.tagline)) },
                code = if sp.promo_code.is_empty() { String::new() } else { format!(r#"<em>Code <code>{}</code></em>"#, esc(&sp.promo_code)) },
            )).collect::<String>()
        )
    };
    let nav = |e2: &Option<repo::Episode>, dir: &str| e2.as_ref().map(|x| format!(r#"<a class="vp-next" href="{}"><small>{dir}</small><b>{}</b></a>"#, esc(&repo::episode_url(&s, &show, x)), esc(&x.title))).unwrap_or_else(|| "<span></span>".into());
    let guests: Vec<(repo::Person, String)> = people.iter().filter(|(_, r)| r == "guest").cloned().collect();
    let crew: Vec<(repo::Person, String)> = people.iter().filter(|(_, r)| r != "guest").cloned().collect();
    let body = format!(
        r#"{assets}{ld}<div class="vp vp-tint-{tint}"><nav class="vp-crumbs"><a href="{show_url}">{show_title}</a>{label}</nav>
<header class="vp-ep-head">{art}<div><h1>{title}</h1><p class="vp-meta-line">{date}{dur}{explicit}</p>{summary}</div></header>
{player}{video}
{guests}{notes}{chapters}{transcript}{crew}{sponsors}
<nav class="vp-nextprev">{older}{newer}</nav>
<div class="vp-subscribe"><b>Never miss an episode</b>{listen}</div></div>"#,
        assets = assets(),
        ld = episode_structured_data(&state, &s, &show, &e, open),
        tint = esc(&show.tint),
        show_url = esc(&repo::show_url(&s, &show)),
        show_title = esc(&show.title),
        label = if label.is_empty() { String::new() } else { format!(" <span>· {}</span>", esc(&label)) },
        art = art(&artwork, &e.title, "vp-ep-art"),
        title = esc(&e.title),
        date = esc(&date_label(&e)),
        dur = if e.duration > 0 { format!(" · {}", esc(&rules::duration_label(e.duration))) } else { String::new() },
        explicit = if e.explicit != 0 { r#" · <span class="vp-chip">Explicit</span>"# } else { "" },
        summary = if e.summary.is_empty() { String::new() } else { format!(r#"<p class="vp-lede">{}</p>"#, esc(&e.summary)) },
        guests = if guests.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>{}</h2>{}</section>"#, if guests.len() == 1 { "Guest" } else { "Guests" }, people_strip(&s, &guests)) },
        notes = if open { let n = e.notes_markup(); if n.trim().is_empty() { String::new() } else { format!(r#"<section class="vp-section vp-notes"><h2>Show notes</h2><div class="vp-notes-body gh-content">{n}</div></section>"#) } } else { String::new() },
        chapters = chapters_html,
        transcript = if open { transcript_html(&e.transcript) } else { String::new() },
        crew = if crew.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>On this episode</h2>{}</section>"#, people_strip(&s, &crew)) },
        sponsors = sponsor_html,
        older = nav(&older, "Previous episode"),
        newer = nav(&newer, "Next episode"),
        listen = listen_on(&s, &show),
    );
    render_virtual_page_as(&state, &headers, &public(&url), &title, &excerpt, body, None, false, &[], serde_json::Map::new(), &["podcast"]).await
}

async fn chapters_json(State(state): State<AppState>, headers: HeaderMap, Path((show_slug, ep_slug)): Path<(String, String)>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{show_slug}/{ep_slug}/chapters.json");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some((_, e)) = find(&state, &show_slug, &ep_slug).await.filter(|(_, e)| e.access == "public" && !e.chapters().is_empty()) else { return not_found(&state, &path, headers).await };
    ([(header::CONTENT_TYPE, "application/json+chapters; charset=utf-8"), (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], rules::chapters_document(&e.chapters()).to_string()).into_response()
}

async fn transcript(State(state): State<AppState>, headers: HeaderMap, Path((show_slug, ep_slug)): Path<(String, String)>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{show_slug}/{ep_slug}/transcript.txt");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let member = crate::render::member_context(&state, &headers).await;
    let Some((_, e)) = find(&state, &show_slug, &ep_slug).await.filter(|(_, e)| !e.transcript.trim().is_empty() && can_listen(e, &member)) else { return not_found(&state, &path, headers).await };
    // The stored format is served as stored (apps read VTT/SRT for captions);
    // the address stays .txt so one link works for every format.
    let fmt = rules::transcript_format(&e.transcript);
    ([(header::CONTENT_TYPE, format!("{}; charset=utf-8", fmt.mime())), (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string())], e.transcript.clone()).into_response()
}

// ---------------------------------------------------------------------------
// people
// ---------------------------------------------------------------------------

async fn person_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}people/{slug}/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(p) = repo::person_by_slug(&state.db, &vuuut_core::directory::slugify(&slug)).await else { return not_found(&state, &path, headers).await };
    let shows = repo::shows(&state.db, false).await;
    let apps: Vec<(repo::Episode, String)> = repo::appearances(&state.db, &p.id).await.into_iter().filter(|(e, _)| shows.iter().any(|sh| sh.id == e.show_id && sh.public())).collect();
    let hosting: Vec<&repo::Show> = {
        let all = repo::all_credits(&state.db, "show").await;
        shows.iter().filter(|sh| sh.public() && all.get(&sh.id).is_some_and(|v| v.iter().any(|(pid, _)| pid == &p.id))).collect()
    };
    if apps.is_empty() && hosting.is_empty() {
        return not_found(&state, &path, headers).await;
    }
    let member = crate::render::member_context(&state, &headers).await;
    let templates = theme_templates(&state, &["podcast/person"]).await;
    if !templates.is_empty() {
        let mut data = serde_json::Map::new();
        data.insert("podcast".into(), podcast_json(&s));
        data.insert("person".into(), repo::person_json(&s, &p, None));
        data.insert("shows".into(), json!(hosting.iter().map(|sh| repo::show_json(&s, sh, &[], 0)).collect::<Vec<_>>()));
        data.insert("episodes".into(), json!(apps.iter().filter_map(|(e, r)| shows.iter().find(|sh| sh.id == e.show_id).map(|sh| { let mut j = repo::episode_json(&s, sh, e, &[], &[], can_listen(e, &member)); j["role"] = json!(rules::Role::from_key(r).label()); j })).collect::<Vec<_>>()));
        return render_virtual_page_as(&state, &headers, &public(&path), &p.name, &p.title, String::new(), (!p.photo.is_empty()).then(|| p.photo.clone()), false, &templates, data, &["podcast"]).await;
    }
    let rows: String = apps.iter().filter_map(|(e, _)| shows.iter().find(|sh| sh.id == e.show_id).map(|sh| episode_row(&s, sh, e, &member))).collect();
    let links: String = p.links().iter().chain(std::iter::once(&p.website).filter(|w| !w.is_empty())).map(|u| {
        let host = u.split("//").nth(1).unwrap_or(u).split('/').next().unwrap_or(u).trim_start_matches("www.");
        format!(r#"<a class="vp-pill" href="{}" target="_blank" rel="noopener">{}</a>"#, esc(u), esc(host))
    }).collect();
    let body = format!(
        r#"{assets}<div class="vp"><header class="vp-person-head">{img}<div><h1>{n}</h1>{title}<div class="vp-listen">{links}</div></div></header>
{bio}{hosting}{eps}</div>"#,
        assets = assets(),
        img = if p.photo.is_empty() { format!(r#"<i class="vp-face vp-face-xl">{}</i>"#, esc(&p.initials())) } else { format!(r#"<img class="vp-face vp-face-xl" src="{}" alt="">"#, esc(&p.photo)) },
        n = esc(&p.name),
        title = if p.title.is_empty() { String::new() } else { format!(r#"<p class="vp-lede">{}</p>"#, esc(&p.title)) },
        bio = if p.bio.is_empty() { String::new() } else { format!(r#"<div class="vp-about">{}</div>"#, vuuut_core::directory::paragraphs(&p.bio)) },
        hosting = if hosting.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>Hosts</h2><div class="vp-shows">{}</div></section>"#, hosting.iter().map(|sh| format!(r#"<a class="vp-show-card" href="{}">{}<span class="vp-show-card-body"><b>{}</b><span>{}</span></span></a>"#, esc(&repo::show_url(&s, sh)), art(&sh.artwork, &sh.title, "vp-show-art"), esc(&sh.title), esc(&sh.subtitle))).collect::<String>()) },
        eps = if rows.is_empty() { String::new() } else { format!(r#"<section class="vp-section"><h2>Episodes</h2><div class="vp-list">{rows}</div></section>"#) },
    );
    render_virtual_page_as(&state, &headers, &public(&path), &p.name, &p.title, body, None, false, &[], serde_json::Map::new(), &["podcast"]).await
}

// ---------------------------------------------------------------------------
// feeds
// ---------------------------------------------------------------------------

async fn build_feed(state: &AppState, s: &Settings, show: &repo::Show, member_status: Option<&str>, token: Option<&str>) -> String {
    let hosts = repo::credits(&state.db, "show", &show.id).await;
    let (eps, _) = repo::episodes(&state.db, &repo::EpisodeQuery { show_id: Some(&show.id), live: true, limit: 5000, ..Default::default() }).await;
    let abs = |u: &str| absolute(state, u);
    // Stored addresses are site-relative without the base path.
    let abs_asset = |u: &str| if u.starts_with('/') { absolute(state, &vuuut_core::url::base(u)) } else { u.to_string() };
    let person = |p: &repo::Person, role: &str| (p.name.clone(), match role { "cohost" => "host".to_string(), r => r.to_string() }, if p.photo.is_empty() { String::new() } else { abs_asset(&p.photo) }, abs(&repo::person_url(s, p)));
    let credits = repo::all_credits(&state.db, "episode").await;
    let people = repo::people(&state.db).await;
    let mut items = Vec::new();
    for e in eps.iter().filter(|e| !e.audio_url.is_empty() && e.access().allows(member_status)) {
        let enclosure = if s.count_downloads || e.access != "public" {
            let mut u = abs(&repo::play_url(s, e));
            if let Some(t) = token.filter(|_| e.access != "public") {
                u.push_str(&format!("?k={t}"));
            }
            u
        } else {
            abs_asset(&e.audio_url)
        };
        let has_tx = !e.transcript.trim().is_empty();
        items.push(rules::FeedEpisode {
            guid: if e.guid.is_empty() { e.id.clone() } else { e.guid.clone() },
            title: e.title.clone(),
            link: abs(&repo::episode_url(s, show, e)),
            summary: e.summary.clone(),
            html: vuuut_core::url::absolutize_html(&state.site_url(), &e.notes_markup()),
            enclosure_url: enclosure,
            enclosure_type: e.audio_type.clone(),
            enclosure_bytes: e.audio_bytes,
            duration: e.duration,
            artwork: if e.artwork.is_empty() { String::new() } else { abs_asset(&e.artwork) },
            season: e.season,
            number: e.number,
            episode_type: e.episode_type.clone(),
            explicit: e.explicit != 0,
            pub_date: e.published_at.as_deref().map(rules::rfc2822).unwrap_or_default(),
            transcript_url: if has_tx && e.access == "public" { abs(&link(&format!("/podcast/{}/{}/transcript.txt", show.slug, e.slug))) } else { String::new() },
            transcript_type: rules::transcript_format(&e.transcript).mime().into(),
            chapters_url: if e.access == "public" && !e.chapters().is_empty() { abs(&link(&format!("/podcast/{}/{}/chapters.json", show.slug, e.slug))) } else { String::new() },
            people: credits.get(&e.id).map(|v| v.iter().filter_map(|(pid, r)| people.iter().find(|p| &p.id == pid).map(|p| person(p, r))).collect()).unwrap_or_default(),
        });
    }
    let feed_show = rules::FeedShow {
        title: if token.is_some() { format!("{} (members)", show.title) } else { show.title.clone() },
        subtitle: show.subtitle.clone(),
        description: show.description.clone(),
        link: abs(&repo::show_url(s, show)),
        feed_url: match token {
            Some(t) => abs(&link(&format!("/podcast/private/{t}/{}.xml", show.slug))),
            None => abs(&repo::feed_url(s, show)),
        },
        artwork: if show.artwork.is_empty() { String::new() } else { abs_asset(&show.artwork) },
        author: show.author.clone(),
        owner_name: show.owner_name.clone(),
        owner_email: show.owner_email.clone(),
        category: show.category.clone(),
        category2: show.category2.clone(),
        language: show.language.clone(),
        explicit: show.explicit != 0,
        show_type: show.show_type.clone(),
        copyright: show.copyright.clone(),
        funding_url: show.funding_url.clone(),
        funding_label: show.funding_label.clone(),
        guid: show.guid.clone(),
        // A private feed must never be imported into a directory.
        locked: show.locked != 0 || token.is_some(),
        complete: show.complete != 0,
        new_feed_url: if token.is_some() { String::new() } else { show.new_feed_url.clone() },
        people: hosts.iter().map(|(p, r)| person(p, r)).collect(),
        generator: "Vuuut".into(),
    };
    rules::rss(&feed_show, &items)
}

async fn feed(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/feed.xml");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(show) = live_show(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let xml = build_feed(&state, &s, &show, None, None).await;
    ([(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=300")], xml).into_response()
}

async fn private_feed(State(state): State<AppState>, headers: HeaderMap, Path((token, file)): Path<(String, String)>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}private/{token}/{file}");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(slug) = file.strip_suffix(".xml") else { return not_found(&state, &path, headers).await };
    let (secret, _) = repo::config(&state.db).await;
    let Some(uuid) = rules::feed_token_member(&secret, &token) else { return not_found(&state, &path, headers).await };
    let Some(status) = repo::member_status(&state.db, &uuid).await else { return not_found(&state, &path, headers).await };
    let Some(show) = live_show(&state, slug).await else { return not_found(&state, &path, headers).await };
    let xml = build_feed(&state, &s, &show, Some(&status), Some(&token)).await;
    ([(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8"), (header::CACHE_CONTROL, "private, no-store"), (header::HeaderName::from_static("x-robots-tag"), "noindex")], xml).into_response()
}

async fn private_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/private/");
    if !s.enabled {
        return not_found(&state, &path, headers).await;
    }
    let Some(show) = live_show(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let member = crate::render::member_context(&state, &headers).await;
    let feed = match member.get("uuid").and_then(Value::as_str) {
        Some(uuid) => {
            let (secret, _) = repo::config(&state.db).await;
            Some(absolute(&state, &link(&format!("/podcast/private/{}/{}.xml", rules::feed_token(&secret, uuid), show.slug))))
        }
        None => None,
    };
    let body = match &feed {
        Some(f) => format!(
            r#"{assets}<div class="vp vp-narrow"><nav class="vp-crumbs"><a href="{su}">{st}</a></nav><h1>Your private feed</h1>
<p class="vp-lede">Add this address to your podcast app to hear every episode your membership includes. It is yours alone — please don't share it.</p>
<div class="vp-copy"><input readonly value="{f}" aria-label="Private feed address"><button class="vp-btn" type="button" data-vp-copy="{f}">Copy</button></div>
<ol class="vp-steps"><li><b>Apple Podcasts</b> — Library → ••• → Follow a Show by URL.</li><li><b>Overcast, Pocket Casts, Castro</b> — Add → paste the address.</li><li><b>Spotify</b> doesn't take private feeds; use another app for members' episodes.</li></ol>
<p><a href="podcast://{bare}">Open in my podcast app</a></p></div>"#,
            assets = assets(),
            su = esc(&repo::show_url(&s, &show)),
            st = esc(&show.title),
            f = esc(f),
            bare = esc(f.split("://").nth(1).unwrap_or(f)),
        ),
        None => format!(
            r#"{assets}<div class="vp vp-narrow"><nav class="vp-crumbs"><a href="{su}">{st}</a></nav><h1>Members' feed</h1>
<p class="vp-lede">Members get a private feed with every episode — including the ones only members can hear — in their own podcast app.</p>
<div class="vp-actions"><a class="vp-btn" href="{signin}">Sign in</a><a class="vp-btn vp-btn-ghost" href="{signup}">Become a member</a></div></div>"#,
            assets = assets(),
            su = esc(&repo::show_url(&s, &show)),
            st = esc(&show.title),
            signin = esc(&vuuut_core::url::base("/#/portal/signin")),
            signup = esc(&vuuut_core::url::base("/#/portal/signup")),
        ),
    };
    let mut resp = render_virtual_page_as(&state, &headers, &public(&path), "Private feed", "", body, None, false, &[], serde_json::Map::new(), &["podcast"]).await;
    resp.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("private, no-store"));
    resp
}

// ---------------------------------------------------------------------------
// the download counter
// ---------------------------------------------------------------------------

async fn play(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    method: axum::http::Method,
    headers: HeaderMap,
    Path(file): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let s = settings(&state);
    let id = file.split('.').next().unwrap_or("");
    if !s.enabled || !vuuut_core::ids::is_objectid(id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(e) = repo::find_episode(&state.db, id).await.filter(|e| e.is_live() && !e.audio_url.is_empty()) else { return StatusCode::NOT_FOUND.into_response() };
    let Some(show) = repo::find_show(&state.db, &e.show_id).await.filter(repo::Show::public) else { return StatusCode::NOT_FOUND.into_response() };
    let mut source = if q.get("s").map(String::as_str) == Some("web") { "web" } else { "feed" };
    if e.access != "public" {
        let by_token = match q.get("k") {
            Some(t) => {
                let (secret, _) = repo::config(&state.db).await;
                match rules::feed_token_member(&secret, t) {
                    Some(uuid) => repo::member_status(&state.db, &uuid).await.is_some_and(|st| e.access().allows(Some(&st))),
                    None => false,
                }
            }
            None => false,
        };
        if by_token {
            source = "private";
        } else {
            let member = crate::render::member_context(&state, &headers).await;
            if !can_listen(&e, &member) {
                return (StatusCode::FORBIDDEN, "This episode is for members.").into_response();
            }
        }
    }
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    // A HEAD, a bot, or a ranged request that is not the start of the file
    // (an app resuming) is not a new listen.
    let resuming = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).is_some_and(|r| !r.starts_with("bytes=0-"));
    if s.count_downloads && method == axum::http::Method::GET && !rules::is_bot(ua) && !resuming {
        let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
        let (_, salt) = repo::config(&state.db).await;
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let listener = rules::listener_key(&ip, ua, &day, &salt);
        let country = crate::analytics::visitor_country(&state, &headers, &ip).unwrap_or_default();
        repo::record_download(&state.db, &repo::Download {
            episode_id: &e.id, show_id: &show.id, day: &day, listener: &listener, app: if source == "web" { "Web player" } else { rules::app_from_user_agent(ua) }, country: &country, source,
        }).await;
    }
    let target = vuuut_core::url::absolute(&state.site_url(), &if e.audio_url.starts_with('/') { vuuut_core::url::base(&e.audio_url) } else { e.audio_url.clone() });
    ([(header::LOCATION, target), (header::CACHE_CONTROL, "no-store".to_string())], StatusCode::FOUND).into_response()
}

// ---------------------------------------------------------------------------
// guest applications and listener questions
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn contact_page(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/contact/");
    if !s.enabled || !s.requests {
        return not_found(&state, &path, headers).await;
    }
    let Some(show) = live_show(&state, &slug).await else { return not_found(&state, &path, headers).await };
    let kind = q.get("kind").map(|k| rules::RequestKind::from_key(k).key()).unwrap_or("guest");
    let banner = if q.contains_key("sent") {
        r#"<div class="vp-note vp-note-ok">Thank you — that's with the team. We read everything, and we'll reply if it's a fit.</div>"#.to_string()
    } else if let Some(e) = q.get("error") {
        format!(r#"<div class="vp-note vp-note-err">{}</div>"#, esc(&vuuut_core::directory::clean(e, 200)))
    } else {
        String::new()
    };
    let kinds: String = rules::RequestKind::ALL.iter().map(|k| format!(r#"<option value="{}"{}>{}</option>"#, k.key(), if k.key() == kind { " selected" } else { "" }, esc(k.label()))).collect();
    let body = format!(
        r#"{assets}<div class="vp vp-narrow"><nav class="vp-crumbs"><a href="{su}">{st}</a></nav><h1>Get in touch with the show</h1>
<p class="vp-lede">Want to come on as a guest, or have a question you'd like us to answer on air? Tell us here.</p>{banner}
<form class="vp-form" method="post" action="{action}"><input type="hidden" name="t" value="{t}">
<div class="vp-hp" aria-hidden="true"><label>Leave this empty <input name="website_url" tabindex="-1" autocomplete="off"></label></div>
<label>What is this about?<select name="kind">{kinds}</select></label>
<div class="vp-form-2"><label>Your name<input name="name" required maxlength="120" autocomplete="name"></label><label>Email<input type="email" name="email" required maxlength="191" autocomplete="email"></label></div>
<label>Topic or question<input name="topic" maxlength="200" placeholder="In a line"></label>
<label>Message<textarea name="message" rows="7" required minlength="10" maxlength="5000" placeholder="Who you are, what you'd talk about, why now…"></textarea></label>
<label>A link about you (optional)<input type="url" name="link" maxlength="500" placeholder="https://"></label>
<button class="vp-btn" type="submit">Send</button></form></div>"#,
        assets = assets(),
        su = esc(&repo::show_url(&s, &show)),
        st = esc(&show.title),
        action = esc(&link(&path)),
        t = now_ms(),
    );
    render_virtual_page_as(&state, &headers, &public(&path), &format!("Contact {}", show.title), "", body, None, false, &[], serde_json::Map::new(), &["podcast"]).await
}

const PER_IP: vuuut_api::brute::Budget = vuuut_api::brute::Budget { free_retries: 5, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };

async fn contact_submit(
    State(state): State<AppState>,
    vuuut_api::brute::ClientAddr(peer): vuuut_api::brute::ClientAddr,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = settings(&state);
    let path = format!("{BASE}{slug}/contact/");
    if !s.enabled || !s.requests {
        return not_found(&state, &path, headers).await;
    }
    let Some(show) = live_show(&state, &slug).await else { return not_found(&state, &path, headers).await };
    if !vuuut_api::forms::same_origin(&headers, &state.site_url()) {
        return (StatusCode::FORBIDDEN, "This did not come from the site.").into_response();
    }
    let back = |msg: &str| Redirect::to(&format!("{}?error={}", link(&path), vuuut_core::social_media::encode(msg))).into_response();
    let done = Redirect::to(&format!("{}?sent=1", link(&path))).into_response();
    if form.get("website_url").is_some_and(|v| !v.is_empty()) {
        return done;
    }
    let started = form.get("t").and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    if now_ms() - started < rules::MIN_FILL_MS {
        return done;
    }
    let ip = vuuut_api::brute::client_ip_for(&state, &headers, peer);
    if vuuut_api::brute::check(&state.db, &format!("podcast_request:{ip}"), &PER_IP).await.is_err() {
        return back("Too many messages from here. Please wait a little.");
    }
    let input = match rules::RequestInput::from_form(&form) {
        Ok(i) => i,
        Err(m) => return back(&m),
    };
    let day_ago = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(1)));
    if repo::requests_from_since(&state.db, &input.email, &day_ago).await >= 3 {
        return back("That address has sent three messages today. Please try again tomorrow.");
    }
    match repo::insert_request(&state.db, Some(&show.id), &input).await {
        Ok(id) => {
            if let Some(r) = repo::find_request(&state.db, &id).await {
                vuuut_api::podcast::notify_request(&state, &r, &show.title).await;
            }
            done
        }
        Err(e) => {
            tracing::error!("podcast: storing a request failed: {}", e.message());
            back("That could not be sent. Please try again.")
        }
    }
}
