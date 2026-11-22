//! Tools → Advertising & Ad Management: the admin screens.
//!
//! The rules are [`ghost_core::ads`], the queries [`ghost_db::repos::ads`],
//! and choosing, placing and counting ads on the public site is
//! `ghost_api::ads`. What is here, by tab:
//!
//! * **Overview** (`/ghost/ads/`) — what is running, what it was seen and
//!   clicked over 30 days, what it earned, and which ads did best.
//! * **Ads** (`/ghost/ads/list/`) and the editor (`/ghost/ads/new/`,
//!   `/ghost/ads/{id}/`) — the creative, where it goes, who sees it, when,
//!   and what it costs; with a preview drawn by the same function that draws
//!   it on the site, and the ad's own report.
//! * **Advertisers** — `ads/advertisers.rs`.
//! * **Settings** — the master switch, the disclosure label, paying members,
//!   ads.txt — `ads/settings.rs`.
//!
//! ## The security contract
//!
//! Administrator and above, origin-checked on every write, through `guard`.
//! Ad network code is staff-pasted HTML and is trusted exactly as Code
//! injection is; every other field is escaped where it is shown, links are
//! http(s) or site paths only (`ghost_core::ads::safe_url`), and ids are
//! validated before they reach a query or a link. Reports and exports hold
//! numbers and names staff typed, and the CSV defuses formulas anyway.

use super::*;
use ghost_core::ads::{self as rules, Audience, Device, Kind, Placement, Status, Targeting};
use ghost_db::repos::ads as repo;

mod advertisers;
mod settings;
pub(crate) use advertisers::*;
pub(crate) use settings::*;

const PATH: &str = "/ghost/ads/";

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
        format!(
            r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}{d}</div>"#,
        a = tab("overview", "Overview", PATH),
        b = tab("ads", "Ads", "/ghost/ads/list/"),
        c = tab("advertisers", "Advertisers", "/ghost/ads/advertisers/"),
        d = tab("settings", "Settings", "/ghost/ads/settings/"),
    )
}

fn head(extra: &str) -> String {
    crate::tools::tool_head("ads", None, extra)
}

fn tile(label: &str, value: &str, detail: &str, icon: &str, tint: &str) -> String {
    format!(
        r#"<div class="card padded ad-tile"><span class="ic tint-{tint}">{ic}</span><span class="ad-tile-value">{value}</span>
<span class="ad-tile-label">{label}</span><span class="ad-tile-detail">{detail}</span></div>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    )
}

fn pct(rate: Option<f64>) -> String {
    rate.map(|r| format!("{r:.2}%")).unwrap_or_else(|| "—".into())
}

/// `YYYY-MM-DD` of `days` ago, for the daily table.
fn day_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days)).format("%Y-%m-%d").to_string()
}

/// Thirty days of impressions as bars and clicks as a line, every day
/// present whether or not anything happened on it — a gap in the axis reads
/// as a gap in the data.
pub(crate) fn chart(rows: &[(String, i64, i64)], days: i64) -> String {
    let all: Vec<(String, i64, i64)> = (0..days)
        .rev()
        .map(|d| {
            let day = day_ago(d);
            let (i, c) = rows.iter().find(|r| r.0 == day).map(|r| (r.1, r.2)).unwrap_or((0, 0));
            (day, i, c)
        })
        .collect();
    let max_i = all.iter().map(|r| r.1).max().unwrap_or(0).max(1);
    let max_c = all.iter().map(|r| r.2).max().unwrap_or(0).max(1);
    let (w, h) = (720.0_f64, 180.0_f64);
    let step = w / days as f64;
    let bars: String = all
        .iter()
        .enumerate()
        .map(|(i, (day, imp, clk))| {
            let bh = (*imp as f64 / max_i as f64) * (h - 20.0);
            format!(
                r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" rx="3" class="ad-bar"><title>{day}: {imp} impressions, {clk} clicks</title></rect>"#,
                x = i as f64 * step + step * 0.15,
                y = h - bh,
                bw = step * 0.7,
                bh = bh.max(if *imp > 0 { 2.0 } else { 0.0 }),
            )
        })
        .collect();
    let line: String = all
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{:.1},{:.1}", i as f64 * step + step / 2.0, h - (r.2 as f64 / max_c as f64) * (h - 20.0)))
        .collect::<Vec<_>>()
        .join(" ");
    let total_i: i64 = all.iter().map(|r| r.1).sum();
    format!(
        r#"<div class="card padded ad-chart"><div class="ad-chart-key"><span><i class="k-imp"></i>Impressions</span><span><i class="k-clk"></i>Clicks</span></div>
<svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" role="img" aria-label="Impressions and clicks over {days} days: {total_i} impressions">{bars}<polyline points="{line}" class="ad-line"/></svg>
<div class="ad-chart-axis"><span>{first}</span><span>Today</span></div></div>"#,
        first = esc(&all.first().map(|r| r.0.clone()).unwrap_or_default()),
    )
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn ads_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let site = ghost_api::ads::settings(&state);
    let ads = repo::list(&state.db).await.unwrap_or_default();
    let now = ghost_core::dates::now_db();
    let running = ads.iter().filter(|a| rules::running(&ghost_api::ads::rule_of(a, &a.rule_list()), &now).is_ok()).count();
    let since = day_ago(29);
    let daily = repo::daily(&state.db, &since, None).await.unwrap_or_default();
    let per_ad = repo::totals_since(&state.db, &since).await.unwrap_or_default();
    let (imp, clk): (i64, i64) = daily.iter().fold((0, 0), |(i, c), r| (i + r.1, c + r.2));
    let revenue: i64 = per_ad
        .iter()
        .filter_map(|(id, i, c)| ads.iter().find(|a| &a.id == id).map(|a| rules::revenue_cents(*i, *c, a.cpm_cents, a.cpc_cents)))
        .sum();

    let off = if site.enabled {
        String::new()
    } else {
        format!(
            r#"<div class="card padded ad-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>Ads are switched off.</b>
<p class="muted">Nothing is shown on the site and ads.txt is not served. Build ads as drafts first, then switch on when they are ready.</p></div>
<form method="post" action="/ghost/ads/settings/switch/"><input type="hidden" name="on" value="1"><button class="btn" type="submit">{eye} Switch on</button></form></div>"#,
            ic = svg("warning"),
            eye = svg("eye"),
        )
    };

    let mut top: Vec<(&repo::Ad, i64, i64)> = per_ad
        .iter()
        .filter_map(|(id, i, c)| ads.iter().find(|a| &a.id == id).map(|a| (a, *i, *c)))
        .collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    let top_rows: String = top
        .iter()
        .take(8)
        .map(|(a, i, c)| {
            format!(
                r#"<a class="row" href="{PATH}{id}/"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{place}</span></div>
<div class="trailing ad-nums"><span><b>{i}</b> seen</span><span><b>{c}</b> clicks</span><span><b>{ctr}</b> CTR</span>{chev}</div></a>"#,
                id = esc(&a.id),
                tint = kind_tint(&a.kind),
                ic = svg(kind_icon(&a.kind)),
                name = esc(&a.name),
                place = esc(Placement::from_key(&a.placement).label()),
                ctr = pct(rules::ctr(*i, *c)),
                chev = svg("chevron"),
            )
        })
        .collect();
    let top_block = if top_rows.is_empty() {
        format!(
            r#"<div class="card padded" style="text-align:center"><p class="muted" style="margin:8px 0 14px">{}</p><a class="btn" href="{PATH}new/">{} Create an ad</a></div>"#,
            if ads.is_empty() { "No ads yet. Start with an image banner at the end of every post." } else { "Nothing has been seen in the last 30 days." },
            svg("add"),
        )
    } else {
        format!(r#"<div class="card rows">{top_rows}</div>"#)
    };

    page_for_role(
        actor.role,
        "Advertising",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{off}
<div class="ad-tiles rise" style="--i:3">{a}{b}{c}{d}</div>
<div class="section">Last 30 days</div>
<div class="rise" style="--i:4">{chart}</div>
<div class="section">Best performing</div>
<div class="rise" style="--i:5">{top_block}</div>
<p class="muted rise" style="--i:6;margin:14px 4px 0">An impression is counted once half the ad has been on screen for a second, so these figures are ads that could be seen — not pages that loaded.</p>"#,
            head = head(&format!(
                r#"<a class="btn text" href="{PATH}report.csv">{dl} Export CSV</a><a class="btn" href="{PATH}new/">{add} New ad</a>"#,
                dl = svg("import"),
                add = svg("add"),
            )),
            tabs = tabs("overview"),
            banner = flash_banner(&q),
            a = tile("Running now", &running.to_string(), &format!("of {}", countable(ads.len() as i64, "ad")), "campaign", if running > 0 { "green" } else { "slate" }),
            b = tile("Impressions", &imp.to_string(), "Seen, last 30 days", "eye", "blue"),
            c = tile("Clicks", &clk.to_string(), &format!("{} click-through", pct(rules::ctr(imp, clk))), "trending", "purple"),
            d = tile("Earned", &rules::money(revenue), "Estimated from CPM and CPC prices", "chart", "amber"),
            chart = chart(&daily, 30),
        ),
        true,
    )
    .into_response()
}

fn kind_icon(kind: &str) -> &'static str {
    match Kind::from_key(kind) {
        Kind::Image => "image",
        Kind::Text => "notes",
        Kind::Code => "code",
    }
}

fn kind_tint(kind: &str) -> &'static str {
    match Kind::from_key(kind) {
        Kind::Image => "pink",
        Kind::Text => "blue",
        Kind::Code => "slate",
    }
}

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

pub(crate) async fn list_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let ads = repo::list(&state.db).await.unwrap_or_default();
    let names: HashMap<String, String> = repo::advertisers(&state.db).await.unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let now = ghost_core::dates::now_db();
    let rows: String = ads
        .iter()
        .map(|a| {
            let (chip, label) = match rules::running(&ghost_api::ads::rule_of(a, &a.rule_list()), &now) {
                Ok(()) => ("published", "Running".to_string()),
                Err(rules::Blocked::NotLive) => match Status::from_key(&a.status) {
                    Status::Paused => ("draft", "Paused".to_string()),
                    _ => ("tonal", "Draft".to_string()),
                },
                Err(b) => ("draft", b.label().to_string()),
            };
            let who = a.advertiser_id.as_ref().and_then(|id| names.get(id)).map(String::as_str).unwrap_or("House ad");
            format!(
                r#"<a class="row" href="{PATH}{id}/"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name} <span class="chip {chip}">{label}</span></span>
<span class="detail">{who} · {kind} · {place}</span></div>
<div class="trailing ad-nums"><span><b>{imp}</b> seen</span><span><b>{ctr}</b> CTR</span>{chev}</div></a>"#,
                id = esc(&a.id),
                tint = kind_tint(&a.kind),
                ic = svg(kind_icon(&a.kind)),
                name = esc(&a.name),
                label = esc(&label),
                who = esc(who),
                kind = esc(Kind::from_key(&a.kind).label()),
                place = esc(&placement_line(a)),
                imp = a.impressions,
                ctr = pct(rules::ctr(a.impressions, a.clicks)),
                chev = svg("chevron"),
            )
        })
        .collect();
    let body = if ads.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:2;text-align:center"><p style="margin:10px 0 4px;font-weight:600">No ads yet</p>
<p class="muted" style="margin:0 0 16px">An image banner, a text ad, or an ad network's code — placed in posts, in a sticky bar, or in a slot your theme provides.</p>
<a class="btn" href="{PATH}new/">{add} Create an ad</a></div>"#,
            add = svg("add"),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:2">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        "Ads",
        "tools",
        Some(&actor.user),
        &format!(
            "{head}{tabs}{banner}{body}",
            head = head(&format!(r#"<a class="btn" href="{PATH}new/">{} New ad</a>"#, svg("add"))),
            tabs = tabs("ads"),
            banner = flash_banner(&q),
        ),
        true,
    )
    .into_response()
}

fn placement_line(a: &repo::Ad) -> String {
    match Placement::from_key(&a.placement) {
        Placement::PostParagraph => format!("Inside posts, after paragraph {}", a.paragraph),
        Placement::Slot => format!("Slot “{}”", a.slot),
        p => p.label().to_string(),
    }
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

fn blank() -> repo::Ad {
    repo::Ad {
        id: String::new(),
        advertiser_id: None,
        name: String::new(),
        status: "draft".into(),
        kind: "image".into(),
        placement: "post_bottom".into(),
        paragraph: 3,
        slot: String::new(),
        image: String::new(),
        link: String::new(),
        alt: String::new(),
        headline: String::new(),
        body: String::new(),
        cta: String::new(),
        code: String::new(),
        audience: "everyone".into(),
        device: "all".into(),
        targeting: "everywhere".into(),
        rules: String::new(),
        weight: 1,
        starts_at: None,
        ends_at: None,
        max_impressions: 0,
        max_clicks: 0,
        cpm_cents: 0,
        cpc_cents: 0,
        impressions: 0,
        clicks: 0,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

pub(crate) async fn new_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let mut ad = blank();
    // "New ad for this advertiser" arrives with the id; it is checked below
    // against the list before it is selected.
    ad.advertiser_id = q.get("advertiser").cloned();
    editor(&state, &actor, ad, true, &q).await
}

pub(crate) async fn edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That ad could not be found.");
    }
    let Ok(Some(ad)) = repo::find(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That ad could not be found.");
    };
    editor(&state, &actor, ad, false, &q).await
}

fn options<T: Copy + PartialEq>(all: &[T], current: T, key: impl Fn(T) -> &'static str, label: impl Fn(T) -> &'static str) -> String {
    all.iter()
        .map(|v| format!(r#"<option value="{}"{}>{}</option>"#, key(*v), if *v == current { " selected" } else { "" }, esc(label(*v))))
        .collect()
}

fn select(name: &str, label: &str, opts: &str) -> String {
    format!(r#"<label class="field fixed"><select name="{name}">{opts}</select><span>{label}</span>{caret}</label>"#, label = esc(label), caret = SELECT_CARET)
}

fn input(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(
        r#"<label class="field"><input type="{kind}" name="{name}" value="{value}" placeholder=" "{extra}><span>{label}</span></label>"#,
        value = esc(value),
        label = esc(label),
    )
}

fn cents(c: i64) -> String {
    if c == 0 { String::new() } else { format!("{}.{:02}", c / 100, c % 100) }
}

async fn editor(state: &AppState, actor: &Actor, ad: repo::Ad, new: bool, q: &HashMap<String, String>) -> Response {
    let site = ghost_api::ads::settings(state);
    let advertisers = repo::advertisers(&state.db).await.unwrap_or_default();
    let adv_opts: String = std::iter::once(format!(r#"<option value="">House ad — this site</option>"#))
        .chain(advertisers.iter().map(|a| {
            format!(
                r#"<option value="{id}"{sel}>{name}</option>"#,
                id = esc(&a.id),
                sel = if ad.advertiser_id.as_deref() == Some(a.id.as_str()) { " selected" } else { "" },
                name = esc(&a.name),
            )
        }))
        .collect();
    let kind = Kind::from_key(&ad.kind);
    let kinds: String = Kind::ALL
        .iter()
        .map(|k| {
            format!(
                r#"<label class="ad-kind"><input type="radio" name="kind" value="{v}"{c}><span class="ic tint-{tint}">{ic}</span><b>{l}</b></label>"#,
                v = k.key(),
                c = if *k == kind { " checked" } else { "" },
                tint = kind_tint(k.key()),
                ic = svg(kind_icon(k.key())),
                l = esc(k.label()),
            )
        })
        .collect();
    let action = if new { format!("{PATH}new/") } else { format!("{PATH}{}/", esc(&ad.id)) };
    // A code ad is never run inside the admin: it is a third party's script,
    // and here it would run with an administrator's session. Its preview is
    // the code itself, escaped; it runs only on the public site.
    let preview = if new || ad.name.is_empty() {
        r#"<p class="muted" style="margin:0">Save to see the ad as readers will.</p>"#.to_string()
    } else if kind == Kind::Code {
        format!(
            r#"<span class="vad-label" style="display:block;margin-bottom:6px;font-size:10.5px;font-weight:600;letter-spacing:.08em;text-transform:uppercase">{label}</span>
<pre class="ad-code">{code}</pre><p class="muted ad-hint" style="margin-top:8px">Network code runs on the site only — never inside the admin.</p>"#,
            label = esc(&site.label),
            code = esc(&ad.code),
        )
    } else {
        ghost_api::ads::markup(&ad, &site.label)
    };

    // The ad's own numbers, on an existing ad.
    let report = if new {
        String::new()
    } else {
        let daily = repo::daily(&state.db, &day_ago(29), Some(std::slice::from_ref(&ad.id))).await.unwrap_or_default();
        format!(
            r#"<div class="section">How it has done</div>
<div class="ad-tiles">{a}{b}{c}{d}</div><div style="margin-top:12px">{chart}</div>"#,
            a = tile("Impressions", &ad.impressions.to_string(), if ad.max_impressions > 0 { "of the cap" } else { "All time" }, "eye", "blue"),
            b = tile("Clicks", &ad.clicks.to_string(), "All time", "trending", "purple"),
            c = tile("Click-through", &pct(rules::ctr(ad.impressions, ad.clicks)), "Clicks per impression", "chart", "green"),
            d = tile("Earned", &rules::money(rules::revenue_cents(ad.impressions, ad.clicks, ad.cpm_cents, ad.cpc_cents)), "At this ad's prices", "chart", "amber"),
            chart = chart(&daily, 30),
        )
    };

    let status = Status::from_key(&ad.status);
    let actions = format!(
        r#"{dup}<button class="btn text" type="submit" form="ad-form" name="status" value="{keep}">{check} Save</button>
{publish}"#,
        dup = if new {
            String::new()
        } else {
            format!(
                r#"<form method="post" action="{PATH}{id}/duplicate/"><button class="btn text" type="submit">{ic} Duplicate</button></form>"#,
                id = esc(&ad.id),
                ic = svg("copy"),
            )
        },
        keep = if new { "draft" } else { status.key() },
        check = svg("check"),
        publish = if status == Status::Live {
            format!(r#"<button class="btn tonal" type="submit" form="ad-form" name="status" value="paused">{} Pause</button>"#, svg("hide"))
        } else {
            format!(r#"<button class="btn" type="submit" form="ad-form" name="status" value="live">{} Go live</button>"#, svg("eye"))
        },
    );

    page_for_role(
        actor.role,
        if new { "New ad" } else { "Edit ad" },
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<form method="post" action="{action}" id="ad-form" class="ad-form">
<div class="ad-split">
<div>
<div class="section">The ad</div>
<div class="card padded rise" style="--i:1">
{name}
{advertiser}
<div class="ad-kinds" role="radiogroup" aria-label="Kind of ad">{kinds}</div>
<div class="ad-only ad-image ad-text">{image}</div>
<div class="ad-only ad-image ad-text">{link}</div>
<div class="ad-only ad-image">{alt}</div>
<div class="ad-only ad-text">{headline}{body}{cta}</div>
<div class="ad-only ad-code"><label class="field"><textarea name="code" rows="7" placeholder=" " spellcheck="false" class="mono">{code}</textarea><span>Ad network code</span></label>
<p class="muted ad-hint">Paste the tag your network gives you (AdSense, Carbon, EthicalAds…). It runs as pasted, like Code injection; trackers inside it wait for cookie consent when the banner is on.</p></div>
</div>

<div class="section">Where it appears</div>
<div class="card padded rise" style="--i:2">
{placement}
<div class="ad-only-p ad-p-post_paragraph">{paragraph}</div>
<div class="ad-only-p ad-p-slot">{slot}<p class="muted ad-hint">Put <code>&lt;div data-ad-slot="{slot_hint}"&gt;&lt;/div&gt;</code> in a theme template or an HTML card, and this ad fills it.</p></div>
{device}
{targeting}
<label class="field"><textarea name="rules" rows="3" placeholder=" " spellcheck="false">{rules}</textarea><span>Pages and tags</span></label>
<p class="muted ad-hint">One per line: a path such as <code>/guides/</code>, or a tag as <code>tag:news</code>. Used when “Where” is not “Every page”.</p>
</div>

<div class="section">Who sees it</div>
<div class="card padded rise" style="--i:3">
{audience}
<p class="muted ad-hint">{paid_note}</p>
</div>

<div class="section">When, how often and at what price</div>
<div class="card padded rise ad-grid" style="--i:4">
{starts}{ends}{max_i}{max_c}{weight}<span></span>{cpm}{cpc}
</div>
</div>

<aside class="ad-side">
<div class="section">Preview</div>
<div class="card padded ad-preview rise" style="--i:1">{preview}</div>
{delete}
</aside>
</div>
</form>
{report}"#,
            head = crate::tools::tool_head_back("ads", "/ghost/ads/list/", None, &actions),
            banner = flash_banner(q),
            name = input("name", "Name, for you", &ad.name, "text", r#" required maxlength="120""#),
            advertiser = select("advertiser_id", "Advertiser", &adv_opts),
            image = input("image", "Image address", &ad.image, "url", r#" maxlength="1000""#),
            link = input("link", "Where a click goes", &ad.link, "text", r#" maxlength="1000" inputmode="url""#),
            alt = input("alt", "Image description, for screen readers", &ad.alt, "text", r#" maxlength="200""#),
            headline = input("headline", "Headline", &ad.headline, "text", r#" maxlength="90""#),
            body = input("body", "Text", &ad.body, "text", r#" maxlength="280""#),
            cta = input("cta", "Button", &ad.cta, "text", r#" maxlength="30""#),
            code = esc(&ad.code),
            placement = select("placement", "Placement", &options(Placement::ALL, Placement::from_key(&ad.placement), Placement::key, Placement::label)),
            paragraph = input("paragraph", "After paragraph", &ad.paragraph.to_string(), "number", r#" min="1" max="50""#),
            slot = input("slot", "Slot name", &ad.slot, "text", r#" maxlength="40" pattern="[A-Za-z0-9_-]*""#),
            slot_hint = if ad.slot.is_empty() { "sidebar".to_string() } else { esc(&ad.slot) },
            device = select("device", "Devices", &options(Device::ALL, Device::from_key(&ad.device), Device::key, Device::label)),
            targeting = select("targeting", "Where", &options(Targeting::ALL, Targeting::from_key(&ad.targeting), Targeting::key, Targeting::label)),
            rules = esc(&ad.rules),
            audience = select("audience", "Audience", &options(Audience::ALL, Audience::from_key(&ad.audience), Audience::key, Audience::label)),
            paid_note = if site.hide_paid {
                "Paying members never see ads on this site — set in Settings."
            } else {
                "Paying members see ads too, unless the audience above leaves them out."
            },
            starts = input("starts_at", "Starts", ad.starts_at.as_deref().and_then(|d| d.get(..10)).unwrap_or(""), "date", ""),
            ends = input("ends_at", "Ends", ad.ends_at.as_deref().and_then(|d| d.get(..10)).unwrap_or(""), "date", ""),
            max_i = input("max_impressions", "Stop after impressions (0 = no limit)", &ad.max_impressions.to_string(), "number", r#" min="0""#),
            max_c = input("max_clicks", "Stop after clicks (0 = no limit)", &ad.max_clicks.to_string(), "number", r#" min="0""#),
            weight = input("weight", "Weight, when ads share a placement", &ad.weight.to_string(), "number", r#" min="1" max="100""#),
            cpm = input("cpm", "Price per 1,000 impressions (CPM)", &cents(ad.cpm_cents), "text", r#" inputmode="decimal""#),
            cpc = input("cpc", "Price per click (CPC)", &cents(ad.cpc_cents), "text", r#" inputmode="decimal""#),
            preview = preview,
            delete = if new {
                String::new()
            } else {
                format!(
                    r#"<div class="section">Delete</div><div class="card padded"><p class="muted" style="margin:0 0 12px">Deleting removes the ad and its daily figures. Export the report first if anybody will ask for it.</p>
<button class="btn tonal small danger" type="submit" form="ad-delete">{del} Delete ad</button></div>"#,
                    del = svg("delete"),
                )
            },
            report = if new {
                String::new()
            } else {
                format!(
                    r#"{report}<form method="post" action="{PATH}{id}/delete/" id="ad-delete" data-confirm="Delete “{name}”|The ad and its daily figures are deleted. It cannot be undone."></form>"#,
                    id = esc(&ad.id),
                    name = esc(&ad.name),
                )
            },
        ),
        true,
    )
    .into_response()
}

/// Reads the form through `AdInput::from_form`, which rebuilds every field.
async fn read_form(state: &AppState, form: &HashMap<String, String>) -> Result<rules::AdInput, String> {
    let advertisers = repo::advertisers(&state.db).await.unwrap_or_default();
    rules::AdInput::from_form(form, |id| ghost_core::ids::is_objectid(id) && advertisers.iter().any(|a| a.id == id))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(message) => return flash_to(&format!("{PATH}new/"), "error", &message),
    };
    match repo::insert(&state.db, &input).await {
        Ok(id) => flash_to(&format!("{PATH}{id}/"), "ok", if input.status == Status::Live { "Ad created and live." } else { "Ad saved as a draft." }),
        Err(e) => {
            tracing::error!("ads: creating an ad failed: {}", e.message());
            flash_to(&format!("{PATH}new/"), "error", "The ad could not be created — check the logs.")
        }
    }
}

pub(crate) async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return flash_to("/ghost/ads/list/", "error", "That ad could not be found.");
    }
    let back = format!("{PATH}{id}/");
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(message) => return flash_to(&back, "error", &message),
    };
    match repo::update(&state.db, &id, &input).await {
        Ok(()) => flash_to(&back, "ok", match input.status {
            Status::Live => "Saved — the ad is live.",
            Status::Paused => "Saved — the ad is paused.",
            Status::Draft => "Saved as a draft.",
        }),
        Err(e) => {
            tracing::error!("ads: saving {id} failed: {}", e.message());
            flash_to(&back, "error", "The ad could not be saved — check the logs.")
        }
    }
}

pub(crate) async fn duplicate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return flash_to("/ghost/ads/list/", "error", "That ad could not be found.");
    }
    let Ok(Some(ad)) = repo::find(&state.db, &id).await else {
        return flash_to("/ghost/ads/list/", "error", "That ad could not be found.");
    };
    // Through the form reader, so a copy obeys today's rules even if the
    // original was saved under older ones. Always a draft, and its counters
    // start at zero.
    let mut form: HashMap<String, String> = [
        ("name", format!("{} (copy)", ad.name)),
        ("advertiser_id", ad.advertiser_id.clone().unwrap_or_default()),
        ("status", "draft".into()),
        ("kind", ad.kind.clone()),
        ("placement", ad.placement.clone()),
        ("paragraph", ad.paragraph.to_string()),
        ("slot", ad.slot.clone()),
        ("image", ad.image.clone()),
        ("link", ad.link.clone()),
        ("alt", ad.alt.clone()),
        ("headline", ad.headline.clone()),
        ("body", ad.body.clone()),
        ("cta", ad.cta.clone()),
        ("code", ad.code.clone()),
        ("audience", ad.audience.clone()),
        ("device", ad.device.clone()),
        ("targeting", ad.targeting.clone()),
        ("rules", ad.rules.clone()),
        ("weight", ad.weight.to_string()),
        ("max_impressions", ad.max_impressions.to_string()),
        ("max_clicks", ad.max_clicks.to_string()),
        ("cpm", cents(ad.cpm_cents)),
        ("cpc", cents(ad.cpc_cents)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    form.insert("starts_at".into(), ad.starts_at.clone().unwrap_or_default());
    form.insert("ends_at".into(), ad.ends_at.clone().unwrap_or_default());
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(message) => return flash_to(&format!("{PATH}{id}/"), "error", &message),
    };
    match repo::insert(&state.db, &input).await {
        Ok(new_id) => flash_to(&format!("{PATH}{new_id}/"), "ok", "Duplicated as a draft."),
        Err(e) => {
            tracing::error!("ads: duplicating {id} failed: {}", e.message());
            flash_to(&format!("{PATH}{id}/"), "error", "The ad could not be duplicated.")
        }
    }
}

pub(crate) async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    if !ghost_core::ids::is_objectid(&id) {
        return flash_to("/ghost/ads/list/", "error", "That ad could not be found.");
    }
    match repo::destroy(&state.db, &id).await {
        Ok(()) => flash_to("/ghost/ads/list/", "ok", "Ad deleted."),
        Err(e) => {
            tracing::error!("ads: deleting {id} failed: {}", e.message());
            flash_to("/ghost/ads/list/", "error", "The ad could not be deleted.")
        }
    }
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// One quoted, formula-defused cell.
pub(crate) fn cell(v: &str) -> String {
    format!("\"{}\"", ghost_core::forms::csv_safe(v).replace('"', "\"\""))
}

/// Per ad, totals and estimated revenue — the whole programme in one sheet.
pub(crate) async fn report_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    let ads = repo::list(&state.db).await.unwrap_or_default();
    let names: HashMap<String, String> = repo::advertisers(&state.db).await.unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let mut out = String::from("ad,advertiser,status,kind,placement,impressions,clicks,ctr_percent,cpm,cpc,earned\n");
    for a in &ads {
        let who = a.advertiser_id.as_ref().and_then(|id| names.get(id)).cloned().unwrap_or_else(|| "House ad".into());
        out.push_str(&[
            cell(&a.name),
            cell(&who),
            cell(&a.status),
            cell(&a.kind),
            cell(&placement_line(a)),
            cell(&a.impressions.to_string()),
            cell(&a.clicks.to_string()),
            cell(&rules::ctr(a.impressions, a.clicks).map(|r| format!("{r:.2}")).unwrap_or_default()),
            cell(&rules::money(a.cpm_cents)),
            cell(&rules::money(a.cpc_cents)),
            cell(&rules::money(rules::revenue_cents(a.impressions, a.clicks, a.cpm_cents, a.cpc_cents))),
        ].join(","));
        out.push('\n');
    }
    csv_response(out, &format!("ads-{}.csv", crate::export_date_stamp()))
}

pub(crate) fn csv_response(body: String, filename: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chart_has_a_bar_for_every_day_even_empty_ones() {
        let html = chart(&[(day_ago(0), 10, 2)], 30);
        assert_eq!(html.matches("<rect").count(), 30);
        assert!(html.contains("10 impressions, 2 clicks"));
    }

    #[test]
    fn a_csv_cell_is_quoted_and_defused() {
        assert_eq!(cell("=SUM(A1)"), "\"'=SUM(A1)\"");
        assert_eq!(cell("a \"b\", c"), "\"a \"\"b\"\", c\"");
    }
}
