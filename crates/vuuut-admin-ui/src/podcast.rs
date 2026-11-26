//! Tools → Podcast: the admin screens.
//!
//! The rules are [`vuuut_core::podcast`], the queries
//! [`vuuut_db::repos::podcast`], the shared flow (feed import, notices)
//! [`vuuut_api::podcast`], and the public pages, feeds and download counter
//! at `/podcast/` are `vuuut_front::podcast`. Here:
//!
//! * **Overview** (`/ghost/podcast/`) — downloads over thirty days, the
//!   latest episodes, what is scheduled, new requests, and the feed check:
//!   what would make Apple Podcasts or Spotify refuse each show.
//! * **Episodes** — the list and the episode editor: audio (upload or
//!   link, length read from the file), show notes in Koenig, season and
//!   number, trailer/bonus, members-only, chapters, transcript, hosts and
//!   guests, sponsors, scheduling, search listing (`podcast/episodes.rs`).
//! * **Shows** — the show editor (artwork, Apple category, owner, the
//!   listening-app links, Podcasting 2.0 tags) and import from another host
//!   by RSS (`podcast/shows.rs`).
//! * **People** and **Sponsors** (`podcast/people.rs`).
//! * **Inbox** — guest applications and listener questions; **Reviews**
//!   (`podcast/inbox.rs`).
//! * **Analytics** — downloads by day, episode, app and country.
//! * **Settings** — Administrator only (`podcast/settings.rs`).
//!
//! Editor and above for the rest. Every write is origin-checked. Requests
//! hold visitors' names and email addresses; they are shown to staff only.

use super::*;
use chrono::{Duration, Utc};
use vuuut_core::podcast::{self as rules, Settings};
use vuuut_db::repos::podcast as repo;

mod episodes;
mod inbox;
mod people;
mod sample;
mod settings;
mod shows;
pub(crate) use episodes::*;
pub(crate) use inbox::*;
pub(crate) use people::*;
pub(crate) use sample::sample_load;
pub use sample::seed_sample_podcast;
pub(crate) use settings::*;
pub(crate) use shows::*;

const BASE: &str = "/ghost/podcast/";
const EPISODES: &str = "/ghost/podcast/episodes/";
const SHOWS: &str = "/ghost/podcast/shows/";

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

pub(crate) fn podcast_settings(state: &AppState) -> Settings {
    vuuut_api::podcast::settings(state)
}

fn public_href(s: &Settings, path: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(path))
}

/// The site's time zone, for showing and reading publish times.
fn zone(state: &AppState) -> chrono_tz::Tz {
    state.settings.get_str("timezone").unwrap_or_default().trim_matches('"').parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC)
}

fn tabs(active: &str, admin: bool, new_requests: i64) -> String {
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
    out.push_str(&tab("episodes", "Episodes", EPISODES, 0));
    out.push_str(&tab("shows", "Shows", SHOWS, 0));
    out.push_str(&tab("people", "Hosts & guests", "/ghost/podcast/people/", 0));
    out.push_str(&tab("sponsors", "Sponsors", "/ghost/podcast/sponsors/", 0));
    out.push_str(&tab("requests", "Inbox", "/ghost/podcast/requests/", new_requests));
    out.push_str(&tab("reviews", "Reviews", "/ghost/podcast/reviews/", 0));
    out.push_str(&tab("analytics", "Analytics", "/ghost/podcast/analytics/", 0));
    if admin {
        out.push_str(&tab("settings", "Settings", "/ghost/podcast/settings/", 0));
    }
    out.push_str("</div>");
    out
}

async fn tabs_for(state: &AppState, active: &str, actor: &Actor) -> String {
    tabs(active, actor.role.is_admin(), repo::new_requests(&state.db).await)
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

fn compact(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

fn day(n: i64) -> String {
    (Utc::now() - Duration::days(n)).format("%Y-%m-%d").to_string()
}

/// Bars for `days` days ending today.
fn bars(per: &[(String, i64)], days: i64, what: &str) -> String {
    let map: HashMap<&str, i64> = per.iter().map(|(d, n)| (d.as_str(), *n)).collect();
    let series: Vec<(String, i64)> = (0..days).rev().map(|i| { let d = day(i); let v = map.get(d.as_str()).copied().unwrap_or(0); (d, v) }).collect();
    let peak = series.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let total: i64 = series.iter().map(|(_, v)| v).sum();
    format!(
        r#"<div class="card padded lm-chart-card"><div class="lm-chart-head"><b>{}</b><span class="muted">{what} in the last {days} days</span></div><div class="lm-chart">{}</div></div>"#,
        compact(total),
        series.iter().map(|(d, v)| format!(r#"<span title="{d}: {v}" style="--h:{}%"></span>"#, (v * 100 / peak).max(if *v > 0 { 6 } else { 2 }))).collect::<String>()
    )
}

/// A list of `(label, count)` as bars across the card.
fn breakdown(rows: &[(String, i64)], empty: &str) -> String {
    if rows.is_empty() {
        return format!(r#"<div class="card padded"><p class="muted" style="margin:0">{}</p></div>"#, esc(empty));
    }
    let top = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
    format!(
        r#"<div class="card padded pc-breakdown">{}</div>"#,
        rows.iter().map(|(l, n)| format!(r#"<div class="pc-bd-row"><span>{}</span><b>{}</b><i style="--w:{}%"></i></div>"#, esc(if l.is_empty() { "Unknown" } else { l }), compact(*n), n * 100 / top)).collect::<String>()
    )
}

/// What `rules::check` judges, for one show.
pub(crate) async fn facts(state: &AppState, show: &repo::Show) -> rules::ShowFacts {
    let (eps, _) = repo::episodes(&state.db, &repo::EpisodeQuery { show_id: Some(&show.id), live: true, limit: 5000, ..Default::default() }).await;
    let hosts = repo::credits(&state.db, "show", &show.id).await;
    rules::ShowFacts {
        title: show.title.clone(),
        has_artwork: !show.artwork.is_empty(),
        has_description: show.description.trim().chars().count() >= 20,
        has_category: !show.category.is_empty(),
        has_owner_email: !show.owner_email.is_empty(),
        has_author: !show.author.is_empty(),
        published: eps.len() as i64,
        missing_audio: eps.iter().filter(|e| e.audio_url.is_empty()).count() as i64,
        missing_duration: eps.iter().filter(|e| e.duration == 0).count() as i64,
        has_links: !show.links().is_empty(),
        has_hosts: !hosts.is_empty(),
    }
}

fn check_html(findings: &[(rules::Level, String)]) -> String {
    if findings.is_empty() {
        return format!(r#"<p class="pc-ok">{} Ready for Apple Podcasts, Spotify and every other app.</p>"#, svg("check"));
    }
    format!(
        r#"<ul class="pc-check">{}</ul>"#,
        findings.iter().map(|(l, m)| {
            let (cls, label) = match l { rules::Level::Blocking => ("error", "Blocks listing"), rules::Level::Warning => ("scheduled", "Warning"), rules::Level::Tip => ("tonal", "Tip") };
            format!(r#"<li><span class="chip {cls}">{label}</span> {}</li>"#, esc(m))
        }).collect::<String>()
    )
}

fn episode_line(e: &repo::Episode, shows: &[repo::Show], tz: chrono_tz::Tz) -> String {
    let show = shows.iter().find(|s| s.id == e.show_id);
    let (chip, label) = if e.status == "draft" { ("draft", "Draft") } else if e.is_scheduled() { ("scheduled", "Scheduled") } else { ("published", "Published") };
    let when = e.published_at.as_deref().and_then(vuuut_core::dates::from_db_string).map(|d| d.with_timezone(&tz).format("%-d %b %Y, %H:%M").to_string()).unwrap_or_default();
    let art = if !e.artwork.is_empty() { e.artwork.clone() } else { show.map(|s| s.artwork.clone()).unwrap_or_default() };
    format!(
        r#"<div class="row dr-listing">{thumb}<div class="body"><span class="title"><a href="{EPISODES}{id}/">{t}</a> <span class="chip {chip}">{label}</span>{lock}</span>
<span class="detail">{show}{lbl}{when}{dur} · {dl} downloads</span></div><div class="trailing"><a class="icon-btn" href="{EPISODES}{id}/" title="Edit">{edit}</a></div></div>"#,
        thumb = if art.is_empty() { format!(r#"<span class="ic tint-purple">{}</span>"#, svg("mic")) } else { format!(r#"<img class="dr-thumb" src="{}" alt="" loading="lazy">"#, esc(&art)) },
        id = esc(&e.id),
        t = esc(&e.title),
        lock = if e.access != "public" { format!(r#" <span class="chip tonal">{}</span>"#, if e.access == "paid" { "Paid" } else { "Members" }) } else { String::new() },
        show = esc(show.map(|s| s.title.as_str()).unwrap_or("No show")),
        lbl = if e.label().is_empty() { String::new() } else { format!(" · {}", esc(&e.label())) },
        when = if when.is_empty() { String::new() } else { format!(" · {}", esc(&when)) },
        dur = if e.duration > 0 { format!(" · {}", esc(&rules::clock(e.duration))) } else { String::new() },
        dl = compact(e.downloads),
        edit = svg("edit"),
    )
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn podcast_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = podcast_settings(&state);
    let tz = zone(&state);
    let shows = repo::shows(&state.db, false).await;
    let counts = repo::episode_counts(&state.db).await;
    let live: i64 = counts.values().map(|c| c.0).sum();
    let scheduled: i64 = counts.values().map(|c| c.2).sum();
    let d30 = day(29);
    let downloads_30 = repo::downloads_since(&state.db, None, &d30).await;
    let downloads_7 = repo::downloads_since(&state.db, None, &day(6)).await;
    let per = repo::downloads_per_day(&state.db, None, &d30).await;
    let (recent, _) = repo::episodes(&state.db, &repo::EpisodeQuery { limit: 6, ..Default::default() }).await;
    let (upcoming, _) = repo::episodes(&state.db, &repo::EpisodeQuery { status: Some("scheduled"), order: "oldest", limit: 6, ..Default::default() }).await;
    let requests = repo::requests(&state.db, Some("new"), None, 5).await;
    let top = repo::top_episodes(&state.db, None, &d30, 5).await;
    let mut top_rows = Vec::new();
    for (id, n) in &top {
        if let Some(e) = repo::find_episode(&state.db, id).await {
            top_rows.push((e.title, *n));
        }
    }

    let live_banner = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>Your podcast pages are live</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{path}</a>{feeds}</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/podcast/"),
            site = esc(state.site_url().trim_end_matches('/')),
            path = esc(&s.paths.public_path("/podcast/")),
            feeds = if shows.is_empty() { String::new() } else { format!(" · {} RSS feed{}", shows.iter().filter(|x| x.public()).count(), if shows.iter().filter(|x| x.public()).count() == 1 { "" } else { "s" }) },
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>The podcast pages and feeds are not public yet.</b>
<p class="muted">Set up a show and its first episode, then switch the podcast on in Settings.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/podcast/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };
    let empty_state = if shows.is_empty() {
        format!(
            r#"<div class="card padded lm-empty rise" style="--i:3"><span class="ic tint-purple">{ic}</span><h3>Start your podcast</h3>
<p class="muted">Create a show, add episodes, and you get a show website, an Apple- and Spotify-ready RSS feed, a player and download stats. Moving from another host? Import its feed — audio stays where it is.</p>
<div class="lm-empty-actions"><a class="btn" href="{SHOWS}new/">{add} New show</a><a class="btn tonal" href="{SHOWS}#import">{imp} Import a feed</a>{sample}</div></div>"#,
            ic = svg("mic"),
            add = svg("add"),
            imp = svg("import"),
            sample = if actor.role.is_admin() { format!(r#"<form method="post" action="/ghost/podcast/sample/"><button class="btn tonal" type="submit">{} Load a sample show</button></form>"#, svg("box")) } else { String::new() },
        )
    } else {
        String::new()
    };
    let mut checks = String::new();
    for sh in shows.iter().filter(|x| x.status != "draft") {
        let f = rules::check(&facts(&state, sh).await);
        checks.push_str(&format!(
            r#"<div class="card padded pc-check-card"><div class="pc-check-head"><b><a href="{SHOWS}{id}/">{t}</a></b><a class="btn text small" href="{feed}" target="_blank" rel="noopener">{rss} Feed</a></div>{list}</div>"#,
            id = esc(&sh.id),
            t = esc(&sh.title),
            feed = esc(&public_href(&s, &format!("/podcast/{}/feed.xml", sh.slug))),
            rss = svg("link"),
            list = check_html(&f),
        ));
    }
    let req_html = if requests.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No new requests.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, requests.iter().map(|r| format!(
            r#"<a class="row" href="/ghost/podcast/requests/#r-{id}"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{n}</span><span class="detail">{k} · {topic}</span></div></a>"#,
            id = esc(&r.id), tint = if r.kind == "guest" { "purple" } else { "blue" }, ic = svg(if r.kind == "guest" { "mic" } else { "comment" }),
            n = esc(&r.name), k = esc(rules::RequestKind::from_key(&r.kind).label()), topic = esc(if r.topic.is_empty() { &r.message } else { &r.topic }),
        )).collect::<String>())
    };
    page_for_role(
        actor.role,
        "Podcast",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{live_banner}{empty_state}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}{e}</div>
<div class="lm-overview rise" style="--i:4"><div><div class="section">Downloads</div>{chart}
<div class="section">Latest episodes</div>{recent}
<div class="section">Scheduled</div>{upcoming}</div>
<div><div class="section">Feed check</div>{checks}<div class="section">Top episodes · 30 days</div>{top}<div class="section">New in the inbox</div>{req}</div></div>"#,
            head = crate::tools::tool_head("podcast", None, &format!(r#"<a class="btn text" href="{SHOWS}new/">{show} New show</a><a class="btn" href="{EPISODES}new/">{add} New episode</a>"#, show = svg("mic"), add = svg("add"))),
            tabs = tabs_for(&state, "overview", &actor).await,
            banner = flash_banner(&q),
            a = tile("Downloads", &compact(downloads_30), &format!("{} in the last 7 days", compact(downloads_7)), Some("/ghost/podcast/analytics/"), "trending", "purple"),
            b = tile("Episodes", &live.to_string(), &format!("{} scheduled", scheduled), Some(EPISODES), "mic", "pink"),
            c = tile("Shows", &shows.len().to_string(), "On this site", Some(SHOWS), "library", "blue"),
            d = tile("Average", &compact(if live > 0 { downloads_30 / live.max(1) } else { 0 }), "Downloads per episode, 30 days", None, "chart", "teal"),
            e = tile("Inbox", &repo::new_requests(&state.db).await.to_string(), "Guest pitches and questions", Some("/ghost/podcast/requests/"), "mail", "amber"),
            chart = bars(&per, 30, "downloads"),
            recent = if recent.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No episodes yet.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, recent.iter().map(|e| episode_line(e, &shows, tz)).collect::<String>()) },
            upcoming = if upcoming.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing scheduled. Set a future publish time on an episode to release it automatically.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{}</div>"#, upcoming.iter().map(|e| episode_line(e, &shows, tz)).collect::<String>()) },
            checks = if checks.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">No live shows yet.</p></div>"#.to_string() } else { checks },
            top = breakdown(&top_rows, "No downloads yet."),
            req = req_html,
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

pub(crate) async fn analytics_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let shows = repo::shows(&state.db, false).await;
    let show = q.get("show").filter(|v| shows.iter().any(|s| &s.id == *v)).cloned();
    let days: i64 = match q.get("days").map(String::as_str) { Some("7") => 7, Some("90") => 90, Some("365") => 365, _ => 30 };
    let since = day(days - 1);
    let sid = show.as_deref();
    let per = repo::downloads_per_day(&state.db, sid, &since).await;
    let apps = repo::downloads_by(&state.db, "app", sid, &since, 12).await;
    let countries: Vec<(String, i64)> = repo::downloads_by(&state.db, "country", sid, &since, 12).await.into_iter()
        .map(|(c, n)| (vuuut_core::countries::country_name(&c).map(str::to_string).unwrap_or(c), n)).collect();
    let sources: Vec<(String, i64)> = repo::downloads_by(&state.db, "source", sid, &since, 5).await.into_iter()
        .map(|(c, n)| (match c.as_str() { "web" => "This site's player".to_string(), "private" => "Members' private feeds".to_string(), _ => "Podcast apps".to_string() }, n)).collect();
    let top = repo::top_episodes(&state.db, sid, &since, 15).await;
    let mut top_rows = String::new();
    for (id, n) in &top {
        if let Some(e) = repo::find_episode(&state.db, id).await {
            let first7 = repo::first_days(&state.db, &e, 7).await;
            top_rows.push_str(&format!(
                r#"<tr><td><a href="{EPISODES}{id}/">{t}</a><small>{l}</small></td><td class="num">{n}</td><td class="num">{f}</td><td class="num">{all}</td></tr>"#,
                id = esc(&e.id), t = esc(&e.title), l = esc(&e.label()), n = compact(*n), f = compact(first7), all = compact(e.downloads),
            ));
        }
    }
    let show_opts: String = std::iter::once((String::new(), "All shows".to_string())).chain(shows.iter().map(|s| (s.id.clone(), s.title.clone())))
        .map(|(v, l)| format!(r#"<option value="{}"{}>{}</option>"#, esc(&v), if show.as_deref().unwrap_or("") == v { " selected" } else { "" }, esc(&l))).collect();
    let day_opts: String = [(7, "7 days"), (30, "30 days"), (90, "90 days"), (365, "12 months")].iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{}>{l}</option>"#, if *v == days { " selected" } else { "" })).collect();
    page_for_role(
        actor.role,
        "Podcast analytics",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}
<form class="pc-filters rise" style="--i:2" method="get"><label class="field fixed"><select name="show" onchange="this.form.submit()">{show_opts}</select><span>Show</span>{caret}</label>
<label class="field fixed"><select name="days" onchange="this.form.submit()">{day_opts}</select><span>Period</span>{caret}</label><noscript><button class="btn tonal" type="submit">Apply</button></noscript></form>
<div class="rise" style="--i:3">{chart}</div>
<div class="dr-split dr-split-even rise" style="--i:4"><div><div class="section">Listening apps</div>{apps}</div><div><div class="section">Countries</div>{countries}</div></div>
<div class="section">How people listen</div>{sources}
<div class="section">Episodes</div><div class="card pc-table-card"><table class="pc-table"><thead><tr><th>Episode</th><th class="num">In period</th><th class="num">First 7 days</th><th class="num">All time</th></tr></thead><tbody>{top}</tbody></table></div>
<p class="muted ec-hint">One download is one listener, one episode, one day — repeat requests from the same app on the same day count once, and known bots and crawlers never count. Countries need a geolocation header from your proxy or the bundled database.</p>"#,
            head = crate::tools::tool_head_back("podcast", BASE, None, ""),
            tabs = tabs_for(&state, "analytics", &actor).await,
            caret = SELECT_CARET,
            chart = bars(&per, days.min(90), "downloads"),
            apps = breakdown(&apps, "No downloads in this period."),
            countries = breakdown(&countries, "No countries recorded."),
            sources = breakdown(&sources, "No downloads in this period."),
            top = if top_rows.is_empty() { r#"<tr><td colspan="4" class="muted">No downloads in this period.</td></tr>"#.to_string() } else { top_rows },
        ),
        true,
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compacted_for_tiles() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_250), "1.2k");
        assert_eq!(compact(48_000), "48k");
        assert_eq!(compact(2_500_000), "2.5M");
    }

    #[test]
    fn a_finding_is_escaped_and_labelled() {
        let html = check_html(&[(rules::Level::Blocking, "<b>x</b>".into())]);
        assert!(html.contains("Blocks listing") && html.contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(check_html(&[]).contains("Ready"));
    }

    #[test]
    fn the_inbox_tab_carries_a_count_only_when_there_is_one() {
        assert!(tabs("overview", true, 3).contains(r#"<span class="lm-badge">3</span>"#));
        assert!(!tabs("overview", false, 0).contains("lm-badge"));
        assert!(!tabs("overview", false, 0).contains("Settings"));
    }
}
