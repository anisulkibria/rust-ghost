//! Cookie Consent & Privacy Compliance: the admin screens.
//!
//! The rules are [`vuuut_core::cookies`], the queries are
//! [`vuuut_db::repos::cookies`], and the visitor-facing half — the banner,
//! the server-side blocking of trackers, the consent log endpoint — is
//! `vuuut_api::cookies`. What is here:
//!
//! * **Overview** (`/ghost/cookies/`) — is the site asking, what did people
//!   answer, what trackers are on the site and whether each is held back,
//!   and a compliance check in order of how much each gap exposes the site.
//! * **Cookies** (`/ghost/cookies/declarations/`) — the declaration visitors
//!   read: the CMS's own cookies, which cannot be edited, and the ones staff
//!   add, by hand or in one click from what the scan found.
//! * **Consent log** (`/ghost/cookies/log/`) — the proof of consent, with a
//!   search by consent id (the id a visitor sees in their choices panel and
//!   can quote in a request), a CSV export and a retention sweep.
//! * **Cookies → Settings** (`/ghost/cookies/settings/`) — the banner itself.
//!
//! ## Where the scan looks, and why there
//!
//! The scan reads the places a tracker can actually come from: site-wide
//! code injection, each published post's and page's own code injection and
//! body, the active theme's templates, and the Vuuut Click connection. It
//! runs `vuuut_core::cookies::detect` over each — the same library the
//! renderer blocks with — so a tracker the report calls "held back" is one
//! the renderer holds back. It runs per request: there is nothing to go
//! stale, and a site has tens of templates, not thousands.
//!
//! ## The security contract
//!
//! Administrator and above, with an origin check on every write — `guard`
//! takes a `writes` flag for that reason. The consent log holds no names,
//! addresses or IPs, but it is still a record about visitors, so it is gated
//! at the same level as Members; its export runs every cell through
//! `csv_safe` and quotes it, and its search is reduced to hex before it is
//! echoed or queried.

use super::*;
use vuuut_core::cookies::{self as rules, Action, Category, Layout, Level, Mode, Settings, Tracker};
use vuuut_db::repos::cookies as repo;

const PATH: &str = "/ghost/cookies/";
const SETTINGS_PATH: &str = "/ghost/cookies/settings/";
const TITLE: &str = "Cookie Consent & Privacy";
const SUB: &str = "Ask before tracking, block what has not been agreed to, and keep the proof.";
/// Enough to read; the export has everything.
const LOG_SHOWN: i64 = 200;
/// Templates and posts the scan will read at most. A site past these is a
/// site whose scan should not hold a request open.
const MAX_TEMPLATES: usize = 400;
const MAX_TEMPLATE_BYTES: u64 = 512 * 1024;
const MAX_POSTS: i64 = 2_000;

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn settings(state: &AppState) -> Settings {
    vuuut_api::cookies::settings(state)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("{path}?{encoded}")).into_response()
}

// ---------------------------------------------------------------------------
// the scan
// ---------------------------------------------------------------------------

/// Where a tracker was found.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub(crate) label: String,
    pub(crate) href: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub(crate) tracker: &'static Tracker,
    pub(crate) places: Vec<Place>,
}

/// What the site would do to a visitor right now, tracker by tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    HeldBack,
    LoadsFirst,
    Cookieless,
    Necessary,
}

fn status_of(t: &Tracker, s: &Settings) -> Status {
    if t.cookieless {
        Status::Cookieless
    } else if t.category == Category::Necessary {
        Status::Necessary
    } else if s.enabled && s.mode.gates() && (!t.embed || s.block_embeds) {
        Status::HeldBack
    } else {
        Status::LoadsFirst
    }
}

pub(crate) struct Scan {
    pub(crate) found: Vec<Found>,
    /// `Some(true)` when Vuuut Click is connected and waits for consent.
    pub(crate) click_needs_consent: Option<bool>,
    pub(crate) templates: usize,
    pub(crate) posts: usize,
}

pub(crate) async fn scan(state: &AppState) -> Scan {
    let mut found: Vec<Found> = Vec::new();
    let mut add = |source: &str, place: Place| {
        for t in rules::detect(source) {
            match found.iter_mut().find(|f| f.tracker.key == t.key) {
                Some(f) => {
                    if f.places.len() < 6 {
                        f.places.push(place.clone());
                    }
                }
                None => found.push(Found { tracker: t, places: vec![place.clone()] }),
            }
        }
    };

    for (key, label) in [("codeinjection_head", "Site header code injection"), ("codeinjection_foot", "Site footer code injection")] {
        if let Some(code) = state.settings.get_str(key).filter(|c| !c.trim().is_empty()) {
            add(&code, Place { label: label.into(), href: Some("/ghost/settings/advanced/".into()) });
        }
    }

    let posts = repo::scannable(&state.db, MAX_POSTS).await.unwrap_or_default();
    for p in &posts {
        let kind = if p.kind == "page" { "Page" } else { "Post" };
        add(&p.markup, Place {
            label: format!("{kind}: {}", p.title),
            href: vuuut_core::ids::is_objectid(&p.id).then(|| format!("/ghost/editor/{}/{}/", p.kind, p.id)),
        });
    }

    // The active theme's templates, read from disk. Bounded by count and
    // size; a symlink or a name with `..` is never followed out of the theme.
    let theme = vuuut_api::themes::active_theme_name(state);
    let mut templates = 0;
    if !theme.contains("..") && !theme.contains('/') {
        let root = state.config.themes_dir().join(&theme);
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let Ok(meta) = entry.metadata() else { continue };
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if meta.is_dir() && !name.starts_with('.') && name != "node_modules" {
                    stack.push(path);
                } else if meta.is_file() && name.ends_with(".hbs") && meta.len() <= MAX_TEMPLATE_BYTES && templates < MAX_TEMPLATES {
                    templates += 1;
                    if let Ok(source) = std::fs::read_to_string(&path) {
                        let rel = path.strip_prefix(&root).map(|p| p.display().to_string()).unwrap_or(name);
                        add(&source, Place { label: format!("Theme “{theme}”: {rel}"), href: Some("/ghost/settings/design/".into()) });
                    }
                }
            }
        }
    }

    let click_needs_consent = match vuuut_api::click::tracker(state).await {
        Ok(Some(t)) => Some(t.get("consent").and_then(|v| v.as_bool()) == Some(true)),
        _ => None,
    };
    Scan { found, click_needs_consent, templates, posts: posts.len() }
}

/// Found trackers whose known cookies are not in the declaration. A tracker
/// with no known cookies (Tag Manager, a cookieless counter) has nothing to
/// declare and is never listed.
fn undeclared<'a>(found: &'a [Found], declared: &[repo::Declaration]) -> Vec<&'a Found> {
    found
        .iter()
        .filter(|f| !f.tracker.cookieless && !f.tracker.cookies.is_empty())
        .filter(|f| {
            !f.tracker.cookies.iter().any(|(name, _, _)| {
                declared.iter().any(|d| d.name == *name || d.tracker_key == f.tracker.key)
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// chrome
// ---------------------------------------------------------------------------

fn head(back: Option<&str>, title: &str, sub: &str, actions: &str) -> String {
    let back = back
        .map(|href| {
            format!(
                r#"<a class="icon-btn" style="transform:rotate(180deg)" href="{href}" title="Back">{chev}</a>"#,
                href = esc(href),
                chev = svg("chevron"),
            )
        })
        .unwrap_or_default();
    format!(
        r#"<header class="page-head rise">{back}<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{actions}</div></header>"#,
        title = esc(title),
        sub = esc(sub),
    )
}

fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}</div>"#,
        a = tab("overview", "Overview", PATH),
        b = tab("declarations", "Cookies", "/ghost/cookies/declarations/"),
        c = tab("log", "Consent log", "/ghost/cookies/log/"),
    )
}

fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{icon}</span>
<span class="ck-tile-value">{value}</span>
<span class="ck-tile-label">{label}</span>
<span class="ck-tile-detail">{detail}</span>"#,
        icon = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(href) => format!(r#"<a class="card padded ck-tile" href="{href}">{body}</a>"#),
        None => format!(r#"<div class="card padded ck-tile">{body}</div>"#),
    }
}

fn category_tint(c: Category) -> &'static str {
    match c {
        Category::Necessary => "slate",
        Category::Preferences => "teal",
        Category::Analytics => "blue",
        Category::Marketing => "amber",
    }
}

fn percent(rate: Option<f64>) -> String {
    rate.map(|r| format!("{r:.0}%")).unwrap_or_else(|| "—".into())
}

fn timezone(state: &AppState) -> String {
    state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into())
}

fn when(db: &str, tz: &str) -> String {
    format!("{} {}", vuuut_core::dates::short_date_in_zone(db, tz), vuuut_core::dates::clock_in_zone(db, tz))
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn cookies_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let site = settings(&state);
    let declared = repo::declarations(&state.db).await.unwrap_or_default();
    let scan = scan(&state).await;
    let missing = undeclared(&scan.found, &declared);
    let since = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(30)));
    let summary = rules::Summary::from_rows(&repo::tally(&state.db, &since).await.unwrap_or_default());
    let report = rules::audit(&rules::Inputs {
        settings: site.clone(),
        found: scan.found.iter().map(|f| f.tracker.key).collect(),
        undeclared: missing.iter().map(|f| f.tracker.key).collect(),
        click_needs_consent: scan.click_needs_consent,
    });

    let held = scan.found.iter().filter(|f| status_of(f.tracker, &site) == Status::HeldBack).count();
    let loose = scan.found.iter().filter(|f| status_of(f.tracker, &site) == Status::LoadsFirst).count();

    // ---- the hero: is the site asking? ----
    let critical = report.iter().filter(|f| f.level == Level::Critical).count();
    let hero = format!(
        r#"<section class="card ck-hero rise" style="--i:2">
<div class="ck-hero-text">
<span class="chip {chip}">{state_label}</span>
<h2>{headline}</h2>
<p class="muted">{line}</p>
<div class="ck-hero-actions">{toggle}<a class="btn tonal" href="{SETTINGS_PATH}">{gear} Banner settings</a></div>
</div>
{preview}
</section>"#,
        chip = if site.enabled { "published" } else { "draft" },
        state_label = if site.enabled { format!("On · {}", site.mode.short()) } else { "Off".to_string() },
        headline = esc(&match (site.enabled, critical) {
            (true, 0) => "Visitors are asked before anything optional runs.".to_string(),
            (true, _) => "The banner is on, but something still needs fixing.".to_string(),
            (false, 0) => "The consent banner is off.".to_string(),
            (false, _) => "This site sets cookies nobody agreed to.".to_string(),
        }),
        line = esc(&match (site.enabled, held, loose) {
            (true, h, 0) if h > 0 => format!("{} held back until a visitor says yes.", countable(h as i64, "tracker")),
            (true, _, l) if l > 0 => format!("{} still load before anyone agrees.", countable(l as i64, "tracker")),
            (true, _, _) => "No third-party trackers found. The banner still lets visitors see what the site stores.".to_string(),
            (false, _, l) if l > 0 => format!("{} load for every visitor. Switch the banner on and they wait for consent.", countable(l as i64, "tracker")),
            (false, _, _) => "Nothing here needs consent yet. Switch it on if you add analytics, ads or embeds.".to_string(),
        }),
        toggle = format!(
            r#"<form method="post" action="{PATH}switch/"><input type="hidden" name="on" value="{v}"><button class="btn{cls}" type="submit"{confirm}>{ic} {label}</button></form>"#,
            v = if site.enabled { "0" } else { "1" },
            cls = if site.enabled { " text" } else { "" },
            confirm = if site.enabled {
                r#" data-confirm="Switch the banner off|Trackers will load for every visitor without asking, and nothing new is added to the consent log.""#
            } else {
                ""
            },
            ic = svg(if site.enabled { "hide" } else { "eye" }),
            label = if site.enabled { "Switch off" } else { "Switch on" },
        ),
        gear = svg("settings"),
        preview = banner_preview(&site),
    );

    // ---- the figures ----
    let tiles = format!(
        r#"<div class="ck-tiles rise" style="--i:3">{a}{b}{c}{d}</div>"#,
        a = tile(
            "Decisions",
            &summary.total.to_string(),
            "Answers to the banner, last 30 days",
            (summary.total > 0).then_some("/ghost/cookies/log/"),
            "check",
            "pink",
        ),
        b = tile(
            "Accepted all",
            &percent(summary.rate(summary.accept_all)),
            &match summary.total {
                0 => "Nobody has answered yet".to_string(),
                _ => format!("{} chose some, {} via GPC", summary.custom, summary.gpc),
            },
            None,
            "heart",
            "green",
        ),
        c = tile(
            "Rejected all",
            &percent(summary.rate(summary.reject_all)),
            "Refused everything optional",
            None,
            "close",
            "red",
        ),
        d = tile(
            "Trackers found",
            &scan.found.len().to_string(),
            &match (scan.found.len(), loose) {
                (0, _) => format!("Across {} and {}", countable(scan.posts as i64, "post"), countable(scan.templates as i64, "template")),
                (_, 0) => "None loads before consent".to_string(),
                (_, l) => format!("{l} load before consent"),
            },
            None,
            "search",
            if loose > 0 { "amber" } else { "blue" },
        ),
    );

    // ---- compliance check ----
    let checks = if report.is_empty() {
        format!(
            r#"<div class="card rows rise" style="--i:4"><div class="row"><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title">Nothing to fix</span><span class="detail">{detail}</span></div></div></div>"#,
            ic = svg("check"),
            detail = esc(if site.enabled {
                "The banner asks first, links your policy, keeps a record and every tracker found is held back."
            } else {
                "No tracker that needs consent was found, so the site does not need a banner today."
            }),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:4">{}</div>"#, report.iter().map(finding_row).collect::<String>())
    };

    // ---- consent by category ----
    let bars: String = summary
        .granted
        .iter()
        .map(|(c, n)| {
            let rate = summary.rate(*n);
            format!(
                r#"<div class="ck-bar"><div class="ck-bar-head"><span>{label}</span><b>{pct}</b></div>
<div class="ck-bar-track"><span class="tint-{tint}" style="width:{w:.1}%"></span></div></div>"#,
                label = esc(c.label()),
                pct = esc(&percent(rate)),
                tint = category_tint(*c),
                w = rate.unwrap_or(0.0),
            )
        })
        .collect();
    let by_category = format!(
        r#"<div class="card padded ck-bars rise" style="--i:5">{bars}<p class="muted" style="margin:6px 0 0;font-size:12.5px">{note}</p></div>"#,
        note = esc(&match summary.total {
            0 => "No decisions in the last 30 days.".to_string(),
            n => format!("Share of {} that allowed each category.", countable(n, "decision")),
        }),
    );

    // ---- found on the site ----
    let found_rows: String = scan
        .found
        .iter()
        .map(|f| {
            let t = f.tracker;
            let (chip, chip_label) = match status_of(t, &site) {
                Status::HeldBack => ("published", "Held back until consent"),
                Status::LoadsFirst => ("error", "Loads before consent"),
                Status::Cookieless => ("tonal", "Cookieless"),
                Status::Necessary => ("tonal", "Necessary"),
            };
            let places = f
                .places
                .iter()
                .map(|p| match &p.href {
                    Some(h) => format!(r#"<a href="{}">{}</a>"#, esc(h), esc(&p.label)),
                    None => esc(&p.label),
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let declare = if missing.iter().any(|m| m.tracker.key == t.key) {
                format!(
                    r#"<form method="post" action="{PATH}declarations/import/"><input type="hidden" name="tracker" value="{key}"><button class="btn text" type="submit">{add} Declare</button></form>"#,
                    key = esc(t.key),
                    add = svg("add"),
                )
            } else {
                String::new()
            };
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name} <span class="chip {chip}">{chip_label}</span></span>
<span class="detail">{vendor} · {cat}{embed} — found in {places}</span></div>
<div class="trailing">{declare}</div></div>"#,
                tint = category_tint(t.category),
                ic = svg(if t.embed { "play" } else { "code" }),
                name = esc(t.name),
                vendor = esc(t.vendor),
                cat = esc(t.category.label()),
                embed = if t.embed { " embed" } else { "" },
                places = places,
            )
        })
        .collect();
    let found = if scan.found.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:6"><p class="muted" style="margin:0">No known trackers in site code injection, {posts}, or {templates} of the active theme.
Something the library does not recognise can still be held back by hand: give its tag <code>data-cookie-category="analytics"</code> and <code>type="text/plain"</code>.</p></div>"#,
            posts = countable(scan.posts as i64, "published post or page with scripts"),
            templates = countable(scan.templates as i64, "template"),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:6">{found_rows}</div>"#)
    };

    // ---- ask again ----
    let renew = format!(
        r#"<div class="card rows ck-wrap rise" style="--i:7"><div class="row" style="align-items:center"><span class="ic tint-purple">{ic}</span>
<div class="body"><span class="title">Ask everyone again</span>
<span class="detail">Revision {rev}. Use after adding a tracker: stored choices are set aside.</span></div>
<div class="trailing"><form method="post" action="{PATH}renew/"><button class="btn tonal" type="submit"
 data-confirm="Ask everyone again|Every visitor sees the banner on their next page view. The consent log keeps the old answers.">{ic2} Ask again</button></form></div></div></div>"#,
        ic = svg("repeat"),
        ic2 = svg("repeat"),
        rev = site.revision,
    );

    page_for_role(
        actor.role,
        TITLE,
        "cookies",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{hero}{tiles}
<div class="section">Compliance check</div>{checks}
<div class="ck-split"><div><div class="section">Consent by category</div>{by_category}</div>
<div><div class="section">Ask again</div>{renew}</div></div>
<div class="section">Found on this site</div>{found}
<p class="muted rise" style="--i:8;margin:14px 4px 0">A guide, not legal advice. The check reads what this site loads and how the banner is set up; whether a cookie is lawful also depends on what you do with the data.</p>"#,
            head = head(
                None,
                TITLE,
                SUB,
                &format!(
                    r#"<a class="btn text" href="/" target="_blank" rel="noopener">{open} View site</a>"#,
                    open = svg("open"),
                ),
            ),
            tabs = tabs("overview"),
            banner = flash_banner(&q),
        ),
        true,
    )
    .into_response()
}

fn finding_row(f: &rules::Finding) -> String {
    format!(
        r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title} <span class="chip {chip}">{level}</span></span>
<span class="detail">{detail}</span></div>
<div class="trailing">{fix}</div></div>"#,
        tint = match f.level {
            Level::Critical => "red",
            Level::Warning => "amber",
            Level::Note => "slate",
        },
        ic = svg(if f.level == Level::Note { "notes" } else { "warning" }),
        title = esc(&f.title),
        chip = match f.level {
            Level::Critical => "error",
            Level::Warning => "draft",
            Level::Note => "tonal",
        },
        level = esc(f.level.label()),
        detail = esc(&f.detail),
        fix = f
            .fix
            .map(|(href, label)| format!(
                r#"<a class="btn text" href="{href}">{label} {chev}</a>"#,
                label = esc(label),
                chev = svg("chevron"),
            ))
            .unwrap_or_default(),
    )
}

/// A small, static drawing of the banner as a visitor will see it, drawn
/// from the saved settings. Not the real banner (that is the site's
/// stylesheet, under the site's theme), so it is labelled as a preview.
fn banner_preview(site: &Settings) -> String {
    let buttons = match site.mode {
        Mode::OptIn => r#"<span class="ck-pv-btn text">Choose</span><span class="ck-pv-btn tonal">Reject all</span><span class="ck-pv-btn tonal">Accept all</span>"#,
        Mode::OptOut => r#"<span class="ck-pv-btn text">Do not sell or share</span><span class="ck-pv-btn tonal">Choose</span><span class="ck-pv-btn filled">OK</span>"#,
        Mode::Notice => r#"<span class="ck-pv-btn text">Settings</span><span class="ck-pv-btn filled">OK</span>"#,
    };
    format!(
        r#"<div class="ck-preview ck-pv-{layout}" aria-label="Preview of the banner"><div class="ck-pv-page"><i></i><i></i><i></i><i></i></div>
<div class="ck-pv-card"><b>{title}</b><p>{message}{policy}</p><div class="ck-pv-actions">{buttons}</div></div>
<span class="ck-pv-label">Preview</span></div>"#,
        layout = site.layout.key(),
        title = esc(&site.title),
        message = esc(&site.message),
        policy = if site.policy_url.is_empty() { String::new() } else { r#" <u>Privacy policy</u>"#.to_string() },
    )
}

pub(crate) async fn cookies_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let on = form.get("on").is_some_and(|v| v == "1");
    match state.settings.set(&state.db, rules::SETTING_ENABLED, &serde_json::Value::Bool(on)).await {
        Ok(_) => flash_to(PATH, "ok", if on { "The banner is on." } else { "The banner is off." }),
        Err(e) => {
            tracing::error!("cookies: switching the banner failed: {}", e.message());
            flash_to(PATH, "error", "That could not be saved — check the logs.")
        }
    }
}

pub(crate) async fn cookies_renew(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let next = settings(&state).revision + 1;
    match state.settings.set(&state.db, rules::SETTING_REVISION, &serde_json::json!(next)).await {
        Ok(_) => flash_to(PATH, "ok", &format!("Everyone will be asked again (revision {next}).")),
        Err(e) => {
            tracing::error!("cookies: bumping the revision failed: {}", e.message());
            flash_to(PATH, "error", "That could not be saved — check the logs.")
        }
    }
}

// ---------------------------------------------------------------------------
// Cookies: the declaration
// ---------------------------------------------------------------------------

pub(crate) async fn declarations_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let declared = repo::declarations(&state.db).await.unwrap_or_default();
    let builtin = rules::builtin_cookies();

    let sections: String = Category::ALL
        .into_iter()
        .map(|c| {
            let own: String = builtin
                .iter()
                .filter(|b| b.category == c)
                .map(|b| cookie_row(&b.name, &b.provider, &b.duration, &b.purpose, c, None))
                .collect();
            let theirs: String = declared
                .iter()
                .filter(|d| d.category == c.key())
                .map(|d| cookie_row(&d.name, &d.provider, &d.duration, &d.purpose, c, Some(&d.id)))
                .collect();
            let rows = format!("{own}{theirs}");
            let rows = if rows.is_empty() {
                r#"<div class="row"><div class="body"><span class="detail">Nothing declared.</span></div></div>"#.to_string()
            } else {
                rows
            };
            format!(
                r#"<div class="section">{label}</div><p class="muted ck-cat-desc">{desc}</p><div class="card rows">{rows}</div>"#,
                label = esc(c.label()),
                desc = esc(c.description()),
            )
        })
        .collect();

    let categories: String = Category::ALL
        .into_iter()
        .map(|c| format!(r#"<option value="{}"{}>{}</option>"#, c.key(), if c == Category::Analytics { " selected" } else { "" }, esc(c.label())))
        .collect();
    let add = format!(
        r#"<div class="section">Add a cookie</div>
<form class="card padded ck-add" method="post" action="{PATH}declarations/">
<label class="field"><input name="name" required maxlength="{mn}" placeholder=" "><span>Name</span></label>
<label class="field"><input name="provider" maxlength="{mp}" placeholder=" "><span>Provider</span></label>
<label class="field fixed"><select name="category">{categories}</select><span>Category</span>{caret}</label>
<label class="field"><input name="duration" maxlength="{md}" placeholder=" "><span>Kept for</span></label>
<label class="field ck-wide"><input name="purpose" maxlength="{mpu}" placeholder=" "><span>What it is for</span></label>
<div class="ck-wide" style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{plus} Add cookie</button></div>
</form>"#,
        mn = rules::MAX_COOKIE_NAME,
        mp = rules::MAX_PROVIDER,
        md = rules::MAX_DURATION,
        mpu = rules::MAX_PURPOSE,
        caret = SELECT_CARET,
        plus = svg("add"),
    );

    page_for_role(
        actor.role,
        "Cookies",
        "cookies",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="card padded ck-embed-hint rise" style="--i:2"><span class="ic tint-blue">{code}</span><div>
<b>Show this list on your privacy page</b>
<p class="muted">Add an HTML card containing <code>&lt;div data-cookie-declaration&gt;&lt;/div&gt;</code>. It becomes this table, grouped by category, with a button to change choices. Any link to <code>#cookie-settings</code> opens the choices too.</p></div></div>
{sections}{add}"#,
            head = head(None, TITLE, "What this site stores on a visitor's device, as visitors read it in the banner.", ""),
            tabs = tabs("declarations"),
            banner = flash_banner(&q),
            code = svg("code"),
        ),
        true,
    )
    .into_response()
}

fn cookie_row(name: &str, provider: &str, duration: &str, purpose: &str, c: Category, id: Option<&str>) -> String {
    let trailing = match id {
        Some(id) if vuuut_core::ids::is_objectid(id) => format!(
            r#"<form method="post" action="{PATH}declarations/{id}/delete/"><button class="icon-btn" type="submit" title="Remove" aria-label="Remove {name}"
 data-confirm="Remove {name}|It stops being listed in the banner. If the site still sets it, the scan will say so.">{del}</button></form>"#,
            id = esc(id),
            name = esc(name),
            del = svg("delete"),
        ),
        _ => r#"<span class="chip tonal">Built in</span>"#.to_string(),
    };
    let meta = [provider, duration].iter().filter(|s| !s.trim().is_empty()).map(|s| esc(s)).collect::<Vec<_>>().join(" · ");
    format!(
        r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title"><code class="ck-name">{name}</code></span>
<span class="detail">{meta}{sep}{purpose}</span></div>
<div class="trailing">{trailing}</div></div>"#,
        tint = category_tint(c),
        ic = svg("file"),
        name = esc(name),
        sep = if meta.is_empty() || purpose.trim().is_empty() { "" } else { " — " },
        purpose = esc(purpose),
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct DeclareForm {
    name: String,
    provider: String,
    category: String,
    duration: String,
    purpose: String,
}

pub(crate) async fn declaration_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<DeclareForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = "/ghost/cookies/declarations/";
    let d = match rules::declaration(&form.name, &form.provider, &form.category, &form.duration, &form.purpose) {
        Ok(d) => d,
        Err(message) => return flash_to(back, "error", message),
    };
    if rules::builtin_cookies().iter().any(|b| b.name == d.name) || repo::is_declared(&state.db, &d.name).await {
        return flash_to(back, "error", &format!("{} is already declared.", d.name));
    }
    match repo::declare(&state.db, &d, "").await {
        Ok(_) => flash_to(back, "ok", &format!("{} added.", d.name)),
        Err(e) => {
            tracing::error!("cookies: declaring {} failed: {}", d.name, e.message());
            flash_to(back, "error", "That could not be saved — check the logs.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ImportForm {
    tracker: String,
}

/// "Declare" on a found tracker: every cookie the library knows it sets,
/// skipping any already declared.
pub(crate) async fn declaration_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<ImportForm>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let Some(t) = rules::tracker(&form.tracker) else {
        return flash_to(PATH, "error", "That tracker is not one this module knows.");
    };
    let mut added = 0;
    for (name, duration, purpose) in t.cookies {
        if repo::is_declared(&state.db, name).await {
            continue;
        }
        let d = rules::Declared {
            name: name.to_string(),
            provider: t.vendor.to_string(),
            category: t.category,
            duration: duration.to_string(),
            purpose: purpose.to_string(),
        };
        match repo::declare(&state.db, &d, t.key).await {
            Ok(_) => added += 1,
            Err(e) => {
                tracing::error!("cookies: importing {} failed: {}", name, e.message());
                return flash_to(PATH, "error", "That could not be saved — check the logs.");
            }
        }
    }
    flash_to(PATH, "ok", &format!("{} declared ({}).", t.name, countable(added, "cookie")))
}

pub(crate) async fn declaration_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = "/ghost/cookies/declarations/";
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(back, "error", "That cookie could not be found.");
    }
    match repo::undeclare(&state.db, &id).await {
        Ok(_) => flash_to(back, "ok", "Removed from the declaration."),
        Err(e) => {
            tracing::error!("cookies: removing {id} failed: {}", e.message());
            flash_to(back, "error", "That could not be removed — check the logs.")
        }
    }
}

// ---------------------------------------------------------------------------
// Consent log
// ---------------------------------------------------------------------------

pub(crate) async fn log_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let site = settings(&state);
    let search = rules::log_query(q.get("q").map(String::as_str).unwrap_or(""));
    let rows = repo::log(&state.db, &search, LOG_SHOWN).await.unwrap_or_default();
    let total = repo::count(&state.db).await;
    let tz = timezone(&state);

    let list: String = rows
        .iter()
        .map(|r| {
            let action = Action::from_key(&r.action);
            let cats: Vec<&str> = r
                .categories
                .split(',')
                .filter_map(Category::from_key)
                .filter(|c| *c != Category::Necessary)
                .map(Category::label)
                .collect();
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title"><code class="ck-name">{id}</code> <span class="chip {chip}">{label}</span>{gpc}</span>
<span class="detail">{cats} · revision {rev}{page} · {when}</span></div></div>"#,
                tint = match action {
                    Some(Action::AcceptAll) | Some(Action::Acknowledge) => "green",
                    Some(Action::RejectAll) | Some(Action::Gpc) => "red",
                    _ => "blue",
                },
                ic = svg(match action {
                    Some(Action::RejectAll) | Some(Action::Gpc) => "close",
                    _ => "check",
                }),
                id = esc(&r.consent_id),
                chip = match action {
                    Some(Action::AcceptAll) => "published",
                    Some(Action::RejectAll) | Some(Action::Gpc) => "error",
                    _ => "tonal",
                },
                label = esc(action.map(Action::label).unwrap_or("Unknown")),
                gpc = if r.gpc != 0 { r#" <span class="chip tonal">GPC signal</span>"# } else { "" },
                cats = esc(&if cats.is_empty() { "Strictly necessary only".to_string() } else { format!("Allowed {}", cats.join(", ")) }),
                rev = r.revision,
                page = if r.page.is_empty() { String::new() } else { format!(" · on {}", esc(&r.page)) },
                when = esc(&when(&r.created_at, &tz)),
            )
        })
        .collect();

    let body = if rows.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center"><p class="muted" style="margin:10px 0">{}</p></div>"#,
            esc(match (search.is_empty(), site.enabled, site.log) {
                (false, _, _) => "No record starts with that consent id.",
                (true, false, _) => "Nothing recorded yet. Records appear once the banner is on and visitors answer it.",
                (true, true, false) => "Recording is switched off in Cookie settings.",
                (true, true, true) => "Nobody has answered the banner yet.",
            }),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:3">{list}</div>"#)
    };

    page_for_role(
        actor.role,
        "Consent log",
        "cookies",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="ck-search rise" style="--i:2" method="get" action="{PATH}log/">
<label class="field" style="margin:0;flex:1"><input type="search" name="q" value="{q}" placeholder=" " maxlength="36" autocomplete="off"><span>Find a consent ID</span></label>
<button class="btn tonal" type="submit">{search_ic} Find</button></form>
<p class="muted ck-log-note">{count}. Each record is a random ID the visitor's browser made, what they chose and when — no name, address or IP. Records older than {days} days are deleted automatically.</p>
{body}"#,
            head = head(
                None,
                TITLE,
                "Proof of what each visitor agreed to, and when.",
                &format!(
                    r#"<form method="post" action="{PATH}log/purge/"><button class="btn text" type="submit"
 data-confirm="Delete old records|Every record older than {days} days is deleted now. It cannot be undone.">{del} Delete old records</button></form>
<a class="btn tonal" href="{PATH}log.csv">{dl} Export CSV</a>"#,
                    days = site.log_days,
                    del = svg("delete"),
                    dl = svg("import"),
                ),
            ),
            tabs = tabs("log"),
            banner = flash_banner(&q),
            q = esc(&search),
            search_ic = svg("search"),
            count = esc(&format!(
                "{} in total{}",
                countable(total, "record"),
                if rows.len() as i64 == LOG_SHOWN { ", newest shown" } else { "" }
            )),
            days = site.log_days,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn log_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    let search = rules::log_query(q.get("q").map(String::as_str).unwrap_or(""));
    let rows = repo::log(&state.db, &search, 10_000).await.unwrap_or_default();
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"consent-log-{}.csv\"", crate::export_date_stamp()),
            ),
        ],
        csv(&rows),
    )
        .into_response()
}

/// Every cell defused (`csv_safe`) and quoted: a page path can contain a
/// comma, and every one of these came from a visitor's browser.
fn csv(rows: &[repo::Consent]) -> String {
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("consent_id,action,categories,revision,gpc,page,recorded_at_utc\n");
    for r in rows {
        out.push_str(&[
            cell(&r.consent_id),
            cell(&r.action),
            cell(&r.categories),
            cell(&r.revision.to_string()),
            cell(if r.gpc != 0 { "yes" } else { "no" }),
            cell(&r.page),
            cell(&r.created_at),
        ].join(","));
        out.push('\n');
    }
    out
}

pub(crate) async fn log_purge(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = "/ghost/cookies/log/";
    let days = settings(&state).log_days;
    match repo::purge_older_than(&state.db, days).await {
        Ok(n) => flash_to(back, "ok", &format!("{} older than {days} days deleted.", countable(n as i64, "record"))),
        Err(e) => {
            tracing::error!("cookies: purging the log failed: {}", e.message());
            flash_to(back, "error", "That could not be done — check the logs.")
        }
    }
}

// ---------------------------------------------------------------------------
// Cookie settings
// ---------------------------------------------------------------------------

fn checkbox(name: &str, on: bool) -> String {
    format!(
        r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label>"#,
        on = if on { " checked" } else { "" },
    )
}

fn picker(name: &str, label: &str, options: &[(String, String)], current: &str) -> String {
    let opts: String = options
        .iter()
        .map(|(v, t)| format!(r#"<option value="{v}"{sel}>{t}</option>"#, v = esc(v), sel = if v == current { " selected" } else { "" }, t = esc(t)))
        .collect();
    format!(
        r#"<label class="field fixed" style="margin:0"><select name="{name}">{opts}</select><span>{label}</span>{caret}</label>"#,
        label = esc(label),
        caret = SELECT_CARET,
    )
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing ck-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

pub(crate) async fn cookies_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let group = crate::SETTINGS_GROUPS.iter().find(|g| g.slug == "cookies").expect("cookies settings group");
    let s = settings(&state);
    // The raw stored copy, so a blank field shows blank (and the default as
    // its placeholder) rather than the default pretending to be typed.
    let stored = |key: &str| state.settings.get_str(key).unwrap_or_default();

    let modes: Vec<(String, String)> = Mode::ALL.iter().map(|m| (m.key().into(), m.label().into())).collect();
    let layouts: Vec<(String, String)> = Layout::ALL.iter().map(|l| (l.key().into(), l.label().into())).collect();
    let expiry: Vec<(String, String)> = rules::EXPIRY_MONTHS
        .iter()
        .map(|m| (m.to_string(), format!("{m} months{}", match m { 6 => " (CNIL)", 13 => " (the most allowed)", _ => "" })))
        .collect();
    let retention: Vec<(String, String)> = rules::LOG_DAYS
        .iter()
        .map(|d| (d.to_string(), match d { 90 => "3 months".into(), 180 => "6 months".into(), 365 => "1 year".into(), 730 => "2 years".into(), _ => "5 years".into() }))
        .collect();

    page_for_role(
        actor.role,
        "Cookies",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<form method="post" action="{SETTINGS_PATH}" id="cookies-settings">
<div class="section">On or off</div>
<div class="card rows rise" style="--i:1">
{enabled}
{mode}
</div>
<div class="section">Banner</div>
<div class="ck-settings-split rise" style="--i:2">
<div class="card padded ck-copy">
{layout}
<label class="field"><input name="{title_key}" value="{title}" maxlength="{max_title}" placeholder="{default_title}"><span>Heading</span></label>
<label class="field"><textarea name="{message_key}" rows="4" maxlength="{max_message}" placeholder="{default_message}">{message}</textarea><span>Message</span></label>
<label class="field" style="margin-bottom:0"><input name="{policy_key}" value="{policy}" maxlength="{max_url}" placeholder="/privacy/"><span>Privacy policy link</span></label>
<p class="muted" style="margin:6px 2px 0;font-size:12.5px">A path on this site such as <code>/privacy/</code>, or a full https:// address. Blank fields use the wording shown in grey.</p>
</div>
{preview}
</div>
<div class="section">Choices</div>
<div class="card rows rise" style="--i:3">
{expiry}
{gpc}
{embeds}
{reopen}
</div>
<div class="section">Proof of consent</div>
<div class="card rows rise" style="--i:4">
{log}
{retention}
</div>
</form>
<p class="muted rise" style="--i:5;margin:14px 4px 0">What the site loads, what visitors chose and the compliance check are in
<a href="{PATH}">Tools → Cookie Consent &amp; Privacy</a>.</p>"#,
            head = crate::group_head(group, Some("cookies-settings")),
            banner = flash_banner(&q),
            enabled = row(
                "eye",
                if s.enabled { "green" } else { "slate" },
                "Show the consent banner",
                if s.enabled {
                    "On: recognised trackers are held back and visitors are asked."
                } else {
                    "Off: nothing is added to the site, and trackers load as they always have."
                },
                &checkbox(rules::SETTING_ENABLED, s.enabled),
            ),
            mode = row(
                "globe",
                "blue",
                "How to ask",
                "Ask first is what the GDPR and the UK require. Opt-out suits sites whose readers are only in US states such as California.",
                &picker(rules::SETTING_MODE, "Consent model", &modes, s.mode.key()),
            ),
            layout = format!(r#"<div style="margin-bottom:14px">{}</div>"#, picker(rules::SETTING_LAYOUT, "Where it appears", &layouts, s.layout.key())),
            title_key = rules::SETTING_TITLE,
            title = esc(&stored(rules::SETTING_TITLE)),
            max_title = rules::MAX_TITLE,
            default_title = esc(rules::DEFAULT_TITLE),
            message_key = rules::SETTING_MESSAGE,
            message = esc(&stored(rules::SETTING_MESSAGE)),
            max_message = rules::MAX_MESSAGE,
            default_message = esc(rules::DEFAULT_MESSAGE),
            policy_key = rules::SETTING_POLICY_URL,
            policy = esc(&s.policy_url),
            max_url = rules::MAX_URL,
            preview = banner_preview(&s),
            expiry = row(
                "history",
                "amber",
                "Ask again after",
                "How long a visitor's choice is remembered.",
                &picker(rules::SETTING_EXPIRY_MONTHS, "Remember for", &expiry, &s.expiry_months.to_string()),
            ),
            gpc = row(
                "lock",
                "purple",
                "Honour Global Privacy Control",
                "When the browser sends it, marketing is refused without asking. Required by law in California and Colorado.",
                &checkbox(rules::SETTING_HONOUR_GPC, s.honour_gpc),
            ),
            embeds = row(
                "play",
                "red",
                "Hold back embedded content",
                "YouTube, Vimeo, X, Instagram and Spotify embeds wait for marketing consent, with a one-click placeholder until then.",
                &checkbox(rules::SETTING_BLOCK_EMBEDS, s.block_embeds),
            ),
            reopen = row(
                "repeat",
                "teal",
                "Show a button to change choices",
                "A small cookie in the corner once the banner is gone. Withdrawing must be as easy as agreeing.",
                &checkbox(rules::SETTING_REOPEN, s.reopen),
            ),
            log = row(
                "notes",
                "green",
                "Keep a record of each decision",
                "A random ID, the choice and when — no name, address or IP. Lets you show that someone agreed.",
                &checkbox(rules::SETTING_LOG, s.log),
            ),
            retention = row(
                "delete",
                "slate",
                "Delete records after",
                "Older records are removed automatically.",
                &picker(rules::SETTING_LOG_DAYS, "Keep for", &retention, &s.log_days.to_string()),
            ),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn cookies_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let raw_policy = form.get(rules::SETTING_POLICY_URL).map(String::as_str).unwrap_or("");
    let Some(policy) = rules::policy_url(raw_policy) else {
        return settings_flash("cookies", "error", "The privacy policy link must be a path on this site or an http(s) address.");
    };
    // Normalised through the same reader the public side uses, so what is
    // stored is exactly what will be applied: an unoffered mode reads as the
    // default, a checkbox is present or absent.
    let flag = |key: &str| form.get(key).is_some_and(|v| v == "1");
    let s = Settings::read(|key| match key {
        rules::SETTING_ENABLED | rules::SETTING_HONOUR_GPC | rules::SETTING_BLOCK_EMBEDS | rules::SETTING_REOPEN | rules::SETTING_LOG => {
            Some(flag(key).to_string())
        }
        _ => form.get(key).cloned(),
    });
    // The copy is stored as typed (cleaned), not with the default filled in,
    // so a blank field keeps meaning "use the default".
    let title = rules::clean(form.get(rules::SETTING_TITLE).map(String::as_str).unwrap_or(""), rules::MAX_TITLE);
    let message = rules::clean(form.get(rules::SETTING_MESSAGE).map(String::as_str).unwrap_or(""), rules::MAX_MESSAGE);
    let writes = [
        (rules::SETTING_ENABLED, serde_json::Value::Bool(s.enabled)),
        (rules::SETTING_MODE, serde_json::json!(s.mode.key())),
        (rules::SETTING_LAYOUT, serde_json::json!(s.layout.key())),
        (rules::SETTING_TITLE, serde_json::json!(title)),
        (rules::SETTING_MESSAGE, serde_json::json!(message)),
        (rules::SETTING_POLICY_URL, serde_json::json!(policy)),
        (rules::SETTING_EXPIRY_MONTHS, serde_json::json!(s.expiry_months)),
        (rules::SETTING_HONOUR_GPC, serde_json::Value::Bool(s.honour_gpc)),
        (rules::SETTING_BLOCK_EMBEDS, serde_json::Value::Bool(s.block_embeds)),
        (rules::SETTING_REOPEN, serde_json::Value::Bool(s.reopen)),
        (rules::SETTING_LOG, serde_json::Value::Bool(s.log)),
        (rules::SETTING_LOG_DAYS, serde_json::json!(s.log_days)),
    ];
    let mut ok = true;
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("cookies: saving {key} failed: {}", e.message());
            ok = false;
        }
    }
    if ok {
        settings_saved(&state, "cookies", "Cookie settings saved.")
    } else {
        settings_flash("cookies", "error", "Saving failed — check the logs.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(key: &str) -> Found {
        Found { tracker: rules::tracker(key).unwrap(), places: vec![] }
    }

    #[test]
    fn a_tracker_counts_as_declared_once_any_of_its_cookies_is() {
        let f = vec![found("google_analytics"), found("plausible"), found("google_tag_manager"), found("meta_pixel")];
        let declared = vec![repo::Declaration {
            id: "a".into(), name: "_ga".into(), provider: "".into(), category: "analytics".into(),
            duration: "".into(), purpose: "".into(), tracker_key: "".into(), created_at: "".into(), updated_at: "".into(),
        }];
        let missing: Vec<&str> = undeclared(&f, &declared).iter().map(|f| f.tracker.key).collect();
        // Plausible is cookieless and Tag Manager sets nothing of its own.
        assert_eq!(missing, ["meta_pixel"]);
    }

    #[test]
    fn an_embed_loads_first_unless_embeds_are_held_back() {
        let on = Settings::read(|k| (k == rules::SETTING_ENABLED).then(|| "true".into()));
        let yt = rules::tracker("youtube").unwrap();
        assert_eq!(status_of(yt, &on), Status::LoadsFirst);
        let both = Settings::read(|k| match k {
            rules::SETTING_ENABLED | rules::SETTING_BLOCK_EMBEDS => Some("true".into()),
            _ => None,
        });
        assert_eq!(status_of(yt, &both), Status::HeldBack);
        let notice = Settings::read(|k| match k {
            rules::SETTING_ENABLED => Some("true".into()),
            rules::SETTING_MODE => Some("notice".into()),
            _ => None,
        });
        assert_eq!(status_of(rules::tracker("google_analytics").unwrap(), &notice), Status::LoadsFirst);
    }

    #[test]
    fn the_export_defuses_formulas_and_quotes_commas() {
        let rows = vec![repo::Consent {
            id: "x".into(), consent_id: "=HYPERLINK(\"evil\")".into(), action: "custom".into(),
            categories: "necessary,analytics".into(), revision: 2, gpc: 1, page: "/a,b/".into(), created_at: "2026-11-21 10:00:00".into(),
        }];
        let out = csv(&rows);
        let line = out.lines().nth(1).unwrap();
        assert!(line.starts_with("\"'=HYPERLINK(\"\"evil\"\")\""), "{line}");
        assert!(line.contains("\"necessary,analytics\""));
        assert!(line.contains("\"/a,b/\""));
        assert!(line.contains("\"yes\""));
    }

    #[test]
    fn the_preview_escapes_what_staff_typed() {
        let s = Settings::read(|k| (k == rules::SETTING_TITLE).then(|| "<img src=x onerror=alert(1)>".into()));
        let html = banner_preview(&s);
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;img"));
    }
}
