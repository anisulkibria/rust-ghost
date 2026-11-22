//! Tools → Caching & Performance: the admin screens.
//!
//! The rules are [`ghost_core::caching`]; the page cache, headers, timing
//! and CDN calls are `ghost_api::caching`; the CDN connection is stored by
//! [`ghost_db::repos::caching`]. What is here:
//!
//! * **Overview** (`/ghost/caching/`) — what the site is doing right now,
//!   measured rather than estimated: hit rate, median and slow response
//!   times, the slowest pages, why requests were not cached, the home page's
//!   weight — and a ranked list of what to change. "Purge everything" empties
//!   this server's cache and, when one is connected, the CDN's.
//! * **Cached pages** (`/ghost/caching/pages/`) — every page held in memory,
//!   its size, age and hits, each purgeable on its own.
//! * **Settings** (`/ghost/caching/settings/`) — the page cache, browser and
//!   CDN headers, compression, front-end optimisations and the Cloudflare
//!   connection. Everything ships off.
//!
//! ## The security contract
//!
//! Administrator and above; every write origin-checked. The Cloudflare token
//! is a secret: kept in `cdn_config` (not `settings`), never rendered back,
//! and a blank field keeps it. A purge key from the form is only ever looked
//! up in the in-memory map, never used as a path or a URL.

use super::*;
use ghost_core::caching::{self as rules, Level, Settings, Speculation};
use ghost_db::repos::caching as repo;

const BASE: &str = "/ghost/caching/";

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    crate::tools::tool_flash(path, kind, message)
}

fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#, on = if key == active { " on" } else { "" }, label = esc(label))
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}</div>"#,
        a = tab("overview", "Overview", BASE),
        b = tab("pages", "Cached pages", "/ghost/caching/pages/"),
        c = tab("settings", "Settings", "/ghost/caching/settings/"),
    )
}

fn tile(label: &str, value: &str, detail: &str, icon: &str, tint: &str) -> String {
    format!(
        r#"<div class="card padded cp-tile"><span class="ic tint-{tint}">{ic}</span><span class="cp-tile-value">{value}</span>
<span class="cp-tile-label">{label}</span><span class="cp-tile-detail">{detail}</span></div>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    )
}

fn kb(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn caching_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::caching::settings(&state);
    let snap = ghost_api::caching::snapshot(&state);
    let cdn = repo::cdn(&state.db).await;
    let host = reqwest::Url::parse(&state.site_url()).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let home = snap.home.as_deref().map(|h| rules::page_facts(h, &host));
    let times: Vec<u64> = snap.samples.iter().filter(|s| s.outcome != "hit").map(|s| s.ms).collect();
    let all_times: Vec<u64> = snap.samples.iter().map(|s| s.ms).collect();
    let rendered = rules::percentiles(times);
    let overall = rules::percentiles(all_times);
    let cacheable = snap.hits + snap.misses;
    let hit_rate = (cacheable > 0).then(|| snap.hits as f64 * 100.0 / cacheable as f64);
    let report = rules::audit(&rules::Inputs {
        settings: s.clone(),
        home: home.clone(),
        p50_ms: rendered.map(|p| p.0),
        p95_ms: rendered.map(|p| p.1),
        hit_rate,
        cdn_connected: cdn.as_ref().is_some_and(|c| c.connected()),
        images_optimized: state.settings.get_bool("images_optimize"),
    });

    let findings: String = report
        .iter()
        .map(|f| {
            let (tint, chip, label, icon) = match f.level {
                Level::Important => ("red", "error", "Important", "warning"),
                Level::Suggestion => ("amber", "draft", "Suggestion", "trending"),
                Level::Good => ("green", "published", "Good", "check"),
            };
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{title} <span class="chip {chip}">{label}</span></span>
<span class="detail">{detail}</span></div>{fix}</div>"#,
                ic = svg(icon),
                title = esc(&f.title),
                detail = esc(&f.detail),
                fix = if f.level == Level::Good {
                    String::new()
                } else if f.title.contains("Optimise uploaded images") {
                    format!(r#"<div class="trailing"><a class="btn text" href="/ghost/settings/images/">Images {}</a></div>"#, svg("chevron"))
                } else if f.title.contains("Scripts block") || f.title.contains("Images without") || f.title.contains("heavy") || f.title.contains("outside hosts") {
                    String::new()
                } else {
                    format!(r#"<div class="trailing"><a class="btn text" href="{BASE}settings/">Settings {}</a></div>"#, svg("chevron"))
                },
            )
        })
        .collect();

    // The slowest pages, by their slowest render.
    let mut by_path: HashMap<&str, (u64, u64, u64)> = HashMap::new();
    for smp in snap.samples.iter().filter(|s| s.outcome != "hit") {
        let e = by_path.entry(smp.path.as_str()).or_insert((0, 0, 0));
        e.0 += 1;
        e.1 += smp.ms;
        e.2 = e.2.max(smp.ms);
    }
    let mut slow: Vec<(&str, (u64, u64, u64))> = by_path.into_iter().collect();
    slow.sort_by(|a, b| b.1 .2.cmp(&a.1 .2));
    let max_ms = slow.first().map(|s| s.1 .2).unwrap_or(1).max(1);
    let slow_rows: String = slow
        .iter()
        .take(10)
        .map(|(path, (n, total, max))| {
            format!(
                r#"<div class="cp-slow"><a href="{p}" target="_blank" rel="noopener">{p}</a>
<span class="cp-bar"><span style="width:{w:.0}%"></span></span><span class="cp-ms">{avg} ms avg · {max} ms max · {n}×</span></div>"#,
                p = esc(path),
                w = *max as f64 * 100.0 / max_ms as f64,
                avg = total / n.max(&1),
            )
        })
        .collect();
    let bypass_rows: String = snap
        .bypasses
        .iter()
        .take(8)
        .map(|(why, n)| format!(r#"<li><span>{}</span><b>{n}</b></li>"#, esc(why)))
        .collect();
    let home_block = match &home {
        Some(h) => format!(
            r#"<ul class="cp-facts"><li><span>HTML</span><b>{size}</b></li><li><span>Blocking scripts in &lt;head&gt;</span><b>{bs}</b></li>
<li><span>Stylesheets</span><b>{ss}</b></li><li><span>Images</span><b>{im}</b></li><li><span>Images without a size</span><b>{ns}</b></li>
<li><span>Other hosts</span><b>{tp}</b></li></ul>"#,
            size = kb(h.bytes),
            bs = h.blocking_scripts,
            ss = h.stylesheets,
            im = h.images,
            ns = h.images_without_size,
            tp = h.third_party.len(),
        ),
        None => r#"<p class="muted" style="margin:0">Measured the next time somebody opens the home page. <a href="/" target="_blank" rel="noopener">Open it now</a>, then reload this screen.</p>"#.to_string(),
    };

    page_for_role(
        actor.role,
        "Caching & Performance",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="cp-tiles rise" style="--i:2">{a}{b}{c}{d}{e}</div>
<div class="section">What to do</div>
<div class="card rows rise" style="--i:3">{findings}</div>
<div class="cp-split">
<div><div class="section">Slowest pages</div><div class="card padded rise" style="--i:4">{slow}</div></div>
<div><div class="section">Home page</div><div class="card padded rise" style="--i:4">{home_block}</div>
<div class="section">Not served from the cache</div><div class="card padded rise" style="--i:5">{bypass}</div></div>
</div>
<p class="muted rise" style="--i:6;margin:14px 4px 0">Measured on this server since it last started: the last {n} page responses. Times are how long the server took, not the reader's whole page load.</p>"#,
            head = crate::tools::tool_head("caching", None, &format!(
                r#"<form method="post" action="{BASE}stats/reset/"><button class="btn text" type="submit">{r} Reset numbers</button></form>
<form method="post" action="{BASE}purge/" data-confirm="Purge everything|Every stored page is dropped{cdn_note}. Pages are rendered fresh on their next view."><button class="btn" type="submit">{p} Purge everything</button></form>"#,
                r = svg("history"),
                p = svg("delete"),
                cdn_note = if cdn.as_ref().is_some_and(|c| c.connected()) { ", here and on Cloudflare" } else { "" },
            )),
            tabs = tabs("overview"),
            banner = flash_banner(&q),
            a = tile("Page cache", if s.page_cache { "On" } else { "Off" }, &if s.page_cache {
                format!("{} pages · {} · {} lifetime", snap.pages.len(), kb(snap.bytes), ago(s.ttl as u64))
            } else {
                "Every page is rendered for every reader".to_string()
            }, "library", if s.page_cache { "green" } else { "slate" }),
            b = tile("Hit rate", &hit_rate.map(|r| format!("{r:.0}%")).unwrap_or_else(|| "—".into()), &format!("{} served from memory, {} rendered", snap.hits, snap.misses), "trending", "purple"),
            c = tile("Typical response", &overall.map(|p| if p.0 == 0 { "<1 ms".to_string() } else { format!("{} ms", p.0) }).unwrap_or_else(|| "—".into()), &rendered.map(|p| format!("{} ms when rendered", p.0)).unwrap_or_else(|| "Nothing measured yet".into()), "history", "blue"),
            d = tile("Slow responses", &overall.map(|p| format!("{} ms", p.1)).unwrap_or_else(|| "—".into()), "The slowest 1 in 20", "warning", "amber"),
            e = tile("CDN", if cdn.as_ref().is_some_and(|c| c.connected()) { "Cloudflare" } else { "None" }, &match &cdn {
                Some(c) if c.connected() => c.last_purge.as_deref().map(|p| format!("Last purged {p} UTC")).unwrap_or_else(|| "Not purged yet".into()),
                _ => "Connect one in Settings".into(),
            }, "globe", "teal"),
            slow = if slow_rows.is_empty() { r#"<p class="muted" style="margin:0">No pages measured yet.</p>"#.to_string() } else { slow_rows },
            bypass = if bypass_rows.is_empty() { r#"<p class="muted" style="margin:0">Nothing yet.</p>"#.to_string() } else { format!(r#"<ul class="cp-facts">{bypass_rows}</ul>"#) },
            n = snap.samples.len(),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn purge_all(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let n = ghost_api::caching::purge_all(&state);
    let cdn = repo::cdn(&state.db).await.filter(|c| c.connected());
    let cdn_note = match cdn {
        Some(c) => match ghost_api::caching::purge_cdn(&c.zone, &c.token, None).await {
            Ok(()) => {
                repo::record_purge(&state.db, "").await;
                " Cloudflare purged too.".to_string()
            }
            Err(e) => {
                repo::record_purge(&state.db, &e).await;
                format!(" Cloudflare was not purged: {e}")
            }
        },
        None => String::new(),
    };
    flash_to(BASE, "ok", &format!("{} dropped.{cdn_note}", countable(n as i64, "cached page")))
}

pub(crate) async fn reset_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    ghost_api::caching::reset_stats(&state);
    flash_to(BASE, "ok", "Numbers reset. New measurements start now.")
}

// ---------------------------------------------------------------------------
// Cached pages
// ---------------------------------------------------------------------------

pub(crate) async fn pages_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::caching::settings(&state);
    let snap = ghost_api::caching::snapshot(&state);
    let rows: String = snap
        .pages
        .iter()
        .map(|(key, size, age, hits)| {
            format!(
                r#"<div class="row"><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title"><a href="{k}" target="_blank" rel="noopener">{k}</a></span>
<span class="detail">{size} · stored {age} ago · expires in {left} · {hits}</span></div>
<div class="trailing"><form method="post" action="{BASE}pages/purge/"><input type="hidden" name="key" value="{k}"><button class="btn text small" type="submit">{del} Purge</button></form></div></div>"#,
                ic = svg("file"),
                k = esc(key),
                size = kb(*size),
                age = ago(*age),
                left = ago((s.ttl as u64).saturating_sub(*age)),
                hits = countable(*hits as i64, "hit"),
                del = svg("delete"),
            )
        })
        .collect();
    let body = if snap.pages.is_empty() {
        format!(
            r#"<div class="card padded" style="text-align:center"><p class="muted" style="margin:8px 0">{}</p></div>"#,
            if s.page_cache { "Nothing stored right now. Pages are stored as anonymous readers open them." } else { "The page cache is off. Switch it on in Settings." }
        )
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        "Cached pages",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted cp-lead rise" style="--i:2">{count} in memory, {size}. Every page is dropped automatically when a post, tag or setting changes.</p>
<div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head("caching", None, ""),
            tabs = tabs("pages"),
            banner = flash_banner(&q),
            count = countable(snap.pages.len() as i64, "page"),
            size = kb(snap.bytes),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct KeyForm {
    key: String,
}

pub(crate) async fn purge_page(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<KeyForm>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = "/ghost/caching/pages/";
    if ghost_api::caching::purge_one(&state, &form.key) {
        flash_to(back, "ok", "Purged. It is rendered fresh on its next view.")
    } else {
        flash_to(back, "error", "That page was not in the cache.")
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing cp-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

fn select(name: &str, label: &str, options: &[(i64, String)], current: i64) -> String {
    let opts: String = options
        .iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if *v == current { " selected" } else { "" }, l = esc(l)))
        .collect();
    format!(r#"<label class="field fixed" style="margin:0"><select name="{name}">{opts}</select><span>{label}</span>{caret}</label>"#, label = esc(label), caret = SELECT_CARET)
}

fn duration(secs: i64) -> String {
    match secs {
        0 => "Always check (0)".into(),
        s if s < 3600 => format!("{} minutes", s / 60),
        s if s < 86_400 => format!("{} hours", s / 3600),
        s => format!("{} day{}", s / 86_400, if s >= 172_800 { "s" } else { "" }),
    }
}

pub(crate) async fn cp_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::caching::settings(&state);
    let cdn = repo::cdn(&state.db).await;
    let ttls: Vec<(i64, String)> = rules::TTLS.iter().map(|t| (*t, duration(*t))).collect();
    let browser: Vec<(i64, String)> = rules::BROWSER_TTLS.iter().map(|t| (*t, duration(*t))).collect();
    let cdn_ttls: Vec<(i64, String)> = rules::CDN_TTLS.iter().map(|t| (*t, if *t == 0 { "Not cached by a CDN".to_string() } else { duration(*t) })).collect();
    let spec = [(0_i64, "Off".to_string()), (1, "When a link is hovered or pressed".to_string()), (2, "As soon as the page is idle".to_string())];
    let spec_current = match s.speculation {
        Speculation::Off => 0,
        Speculation::Moderate => 1,
        Speculation::Eager => 2,
    };
    let connected = cdn.as_ref().is_some_and(|c| c.connected());
    page_for_role(
        actor.role,
        "Caching settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{BASE}settings/" id="cp-settings">
<div class="section">Page cache</div>
<p class="muted cp-lead">Finished pages for readers who are not signed in, kept in this server's memory. Members, staff, previews, pages with forms and private sites are always rendered fresh, and every stored page is dropped the moment a post, tag or setting changes.</p>
<div class="card rows rise" style="--i:2">
{cache}
{ttl}
{max}
<div class="row"><span class="ic tint-red">{hide}</span><div class="body"><span class="title">Never cache</span><span class="detail">Path prefixes, one per line — pages that must always be fresh.</span>
<label class="field" style="margin:10px 0 0"><textarea name="{ex_key}" rows="3" placeholder="/live/&#10;/scores/" spellcheck="false">{exclude}</textarea><span>Paths</span></label></div></div>
</div>
<div class="section">Browsers and CDNs</div>
<div class="card rows rise" style="--i:3">
{browser}
{cdn_ttl}
{etag}
{gzip}
</div>
<div class="section">Faster pages</div>
<div class="card rows rise" style="--i:4">
{lazy}
{spec}
<div class="row"><span class="ic tint-teal">{globe}</span><div class="body"><span class="title">Preconnect</span><span class="detail">Hosts every page loads from — fonts, a CDN, analytics. One https://origin per line; up to eight.</span>
<label class="field" style="margin:10px 0 0"><textarea name="{pre_key}" rows="3" placeholder="https://fonts.gstatic.com" spellcheck="false">{pre}</textarea><span>Origins</span></label></div></div>
</div>
<div class="section">Cloudflare</div>
<p class="muted cp-lead">When the site sits behind Cloudflare, connect it so edits reach readers at once instead of waiting for Cloudflare's copy to expire.</p>
<div class="card padded rise" style="--i:5">
<div class="cp-two">
<label class="field"><input name="cf_zone" value="{zone}" maxlength="32" spellcheck="false" placeholder=" "><span>Zone ID</span></label>
<label class="field"><input name="cf_token" type="password" autocomplete="off" placeholder="{token_ph}"><span>API token (Zone → Cache Purge)</span></label>
</div>
<p class="cp-hint">Zone ID: the domain's Overview page in Cloudflare, right-hand column. Token: My Profile → API Tokens → Create, with permission Zone · Cache Purge · Purge for this zone. {state}</p>
<label class="cp-check"><input type="checkbox" name="{auto_key}" value="1"{auto}> Purge Cloudflare whenever content changes (at most once every 15 seconds)</label>
</div>
<div class="cp-save">{disconnect}<button class="btn" type="submit">{check} Save</button></div>
</form>
{disconnect_form}"#,
            head = crate::tools::tool_head("caching", Some("cp-settings"), ""),
            tabs = tabs("settings"),
            banner = flash_banner(&q),
            cache = row("library", "green", "Cache pages", "Serve anonymous readers from memory.", &switch(rules::SETTING_PAGE_CACHE, s.page_cache)),
            ttl = row("history", "blue", "Keep pages for", "The longest a page is served before it is rendered again, even with no change.", &select(rules::SETTING_TTL, "Lifetime", &ttls, s.ttl)),
            max = row("file", "purple", "Pages kept at most", "The oldest go first when the cache is full.", &format!(r#"<label class="field" style="margin:0"><input type="number" name="{k}" value="{v}" min="{lo}" max="{hi}" placeholder=" "><span>Pages</span></label>"#, k = rules::SETTING_MAX_PAGES, v = s.max_pages, lo = rules::MAX_PAGES_RANGE.0, hi = rules::MAX_PAGES_RANGE.1)),
            hide = svg("hide"),
            ex_key = rules::SETTING_EXCLUDE,
            exclude = esc(&s.exclude.join("\n")),
            browser = row("layout", "slate", "Browsers keep a page for", "0 means they check with the server each time — the right choice for most sites.", &select(rules::SETTING_BROWSER_TTL, "Browser", &browser, s.browser_ttl)),
            cdn_ttl = row("globe", "teal", "A CDN keeps a page for", "Sends s-maxage, for Cloudflare and other CDNs in front of the site. Connect Cloudflare below so edits purge it.", &select(rules::SETTING_CDN_TTL, "CDN", &cdn_ttls, s.cdn_ttl)),
            etag = row("check", "green", "Answer “not modified”", "Sends an ETag, so a browser checking an unchanged page gets a 304 and no body.", &switch(rules::SETTING_ETAG, s.etag)),
            gzip = row("import", "amber", "Compress responses (gzip)", "Text shrinks to about a quarter. Leave off if a proxy in front already compresses.", &switch(rules::SETTING_GZIP, s.gzip)),
            lazy = row("image", "pink", "Lazy-load images in posts", "Images below the first load as the reader scrolls. The first image is left alone.", &switch(rules::SETTING_LAZY, s.lazy_images)),
            spec = row("trending", "purple", "Open the next page early", "Speculation rules: Chrome and Edge prerender a same-site link before it is clicked. Never the admin or members pages.", &select("perf_speculation_level", "Prerender", &spec.to_vec(), spec_current)),
            globe = svg("globe"),
            pre_key = rules::SETTING_PRECONNECT,
            pre = esc(&s.preconnect.join("\n")),
            zone = esc(cdn.as_ref().map(|c| c.zone.as_str()).unwrap_or("")),
            token_ph = if connected { "•••••••• saved — leave blank to keep" } else { " " },
            state = match &cdn {
                Some(c) if c.connected() && !c.last_error.is_empty() => format!("Last purge failed: {}.", esc(&c.last_error)),
                Some(c) if c.connected() => "Connected.".to_string(),
                _ => String::new(),
            },
            auto_key = rules::SETTING_CDN_AUTO,
            auto = if s.cdn_auto_purge { " checked" } else { "" },
            disconnect = if connected { format!(r#"<button class="btn tonal small danger" type="submit" form="cp-disconnect">{} Disconnect Cloudflare</button>"#, svg("delete")) } else { String::new() },
            disconnect_form = if connected { format!(r#"<form method="post" action="{BASE}cdn/disconnect/" id="cp-disconnect" data-confirm="Disconnect Cloudflare|The zone and token are removed from this server."></form>"#) } else { String::new() },
            check = svg("check"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn cp_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let back = "/ghost/caching/settings/";
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let speculation = match form.get("perf_speculation_level").map(String::as_str) {
        Some("1") => "moderate",
        Some("2") => "eager",
        _ => "off",
    };
    let s = Settings::read(|key| match key {
        rules::SETTING_PAGE_CACHE | rules::SETTING_ETAG | rules::SETTING_GZIP | rules::SETTING_LAZY | rules::SETTING_CDN_AUTO => Some(flag(key).to_string()),
        rules::SETTING_SPECULATION => Some(speculation.into()),
        _ => form.get(key).cloned(),
    });
    // The CDN first: a token that does not work is said now, before anything
    // depends on it.
    let zone = form.get("cf_zone").map(|z| z.trim().to_string()).unwrap_or_default();
    let token = form.get("cf_token").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let mut cdn_note = String::new();
    if !zone.is_empty() || token.is_some() {
        if !ghost_api::caching::valid_zone(&zone) {
            return flash_to(back, "error", "A Cloudflare zone ID is 32 letters and digits.");
        }
        if token.as_deref().is_some_and(|t| !ghost_api::caching::valid_token(t)) {
            return flash_to(back, "error", "That does not look like a Cloudflare API token.");
        }
        let stored = repo::cdn(&state.db).await.map(|c| c.token).filter(|t| !t.is_empty());
        let Some(use_token) = token.clone().or(stored) else {
            return flash_to(back, "error", "Add the API token as well as the zone.");
        };
        if repo::save_cdn(&state.db, &zone, token.as_deref()).await.is_err() {
            return flash_to(back, "error", "The Cloudflare connection could not be saved.");
        }
        cdn_note = match ghost_api::caching::test_cdn(&zone, &use_token).await {
            Ok(name) => format!(" Cloudflare connected: {name}."),
            Err(e) => format!(" Cloudflare saved, but the test failed: {e}"),
        };
    }
    let writes = [
        (rules::SETTING_PAGE_CACHE, serde_json::json!(s.page_cache)),
        (rules::SETTING_TTL, serde_json::json!(s.ttl)),
        (rules::SETTING_MAX_PAGES, serde_json::json!(s.max_pages)),
        (rules::SETTING_EXCLUDE, serde_json::json!(s.exclude.join("\n"))),
        (rules::SETTING_BROWSER_TTL, serde_json::json!(s.browser_ttl)),
        (rules::SETTING_CDN_TTL, serde_json::json!(s.cdn_ttl)),
        (rules::SETTING_ETAG, serde_json::json!(s.etag)),
        (rules::SETTING_GZIP, serde_json::json!(s.gzip)),
        (rules::SETTING_LAZY, serde_json::json!(s.lazy_images)),
        (rules::SETTING_SPECULATION, serde_json::json!(s.speculation.key())),
        (rules::SETTING_PRECONNECT, serde_json::json!(s.preconnect.join("\n"))),
        (rules::SETTING_CDN_AUTO, serde_json::json!(s.cdn_auto_purge)),
    ];
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("caching: saving {key} failed: {}", e.message());
            return flash_to(back, "error", "Saving failed — check the logs.");
        }
    }
    // A change of rules is a change of every page: the stored ones go.
    ghost_api::caching::purge_all(&state);
    let kind = if cdn_note.contains("failed") { "error" } else { "ok" };
    crate::tools::tool_flash(back, kind, &format!("Caching settings saved.{cdn_note}"))
}

pub(crate) async fn cdn_disconnect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let _ = repo::forget_cdn(&state.db).await;
    let _ = state.settings.set(&state.db, rules::SETTING_CDN_AUTO, &serde_json::json!(false)).await;
    flash_to("/ghost/caching/settings/", "ok", "Cloudflare disconnected.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_ages_read_like_people_say_them() {
        assert_eq!(kb(1500), "2 KB");
        assert_eq!(kb(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(ago(90), "1m");
        assert_eq!(duration(0), "Always check (0)");
        assert_eq!(duration(86_400), "1 day");
    }
}
