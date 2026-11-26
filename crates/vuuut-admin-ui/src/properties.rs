//! Tools → Real Estate & Property Listings: the admin screens.
//!
//! The rules are [`vuuut_core::properties`], the queries
//! [`vuuut_db::repos::properties`], and the public catalogue at
//! `/properties/` is `vuuut_front::properties`. Here:
//!
//! * **Overview** (`/ghost/properties/`) — what is on the market, enquiries
//!   over the last thirty days, the newest enquiries, the most viewed
//!   properties.
//! * **Properties** — every property, filtered, and the property editor
//!   (`properties/editor.rs`): the same two-column editor as a store
//!   product, a directory listing or a course — the description in Koenig,
//!   cards for photos, details, location, features, floor plans, open
//!   houses and media, status, price and organisation down the side.
//! * **Enquiries** — questions and viewing requests from property pages
//!   (`properties/enquiries.rs`).
//! * **Agents** (`properties/agents.rs`), **Types** (`properties/types.rs`)
//!   and **Settings** (`properties/settings.rs`).
//!
//! ## The security contract
//!
//! Editor and above for properties, agents and enquiries; Administrator for
//! types, settings and the sample loader. Every write is origin-checked.
//! The description goes through the same Koenig renderer as posts;
//! everything else is rebuilt by `vuuut_core::properties` and escaped where
//! it is shown. Enquiries are other people's personal data: they are gated
//! like members (Editor and up), never shown on the site, and the CSV export
//! defuses formulas.

use super::*;
use vuuut_core::properties::{self as rules, Market, Purpose, Settings, Status};
use vuuut_db::repos::properties as repo;

mod agents;
mod editor;
mod enquiries;
mod sample;
mod settings;
mod types;
pub(crate) use agents::*;
pub(crate) use editor::*;
pub(crate) use enquiries::*;
pub(crate) use sample::sample_load;
pub use sample::{seed_sample_properties, Loaded};
pub(crate) use settings::*;
pub(crate) use types::*;

const BASE: &str = "/ghost/properties/";
const LISTINGS: &str = "/ghost/properties/listings/";
const ENQUIRIES: &str = "/ghost/properties/enquiries/";
const AGENTS: &str = "/ghost/properties/agents/";

async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let (path, anchor) = path.split_once('#').map(|(p, a)| (p, format!("#{a}"))).unwrap_or((path, String::new()));
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}{anchor}")).into_response()
}

pub(crate) fn property_settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

/// The site's public address for an internal `/properties/…` path.
fn public_href(s: &Settings, path: &str) -> String {
    vuuut_core::url::base(&s.paths.public_path(path))
}

fn tabs(active: &str, admin: bool, new_enquiries: i64) -> String {
    let tab = |key: &str, label: &str, href: &str, badge: i64| {
        format!(
            r#"<a class="seg{on}" href="{href}">{label}{b}<span style="width:8px"></span></a>"#,
            on = if key == active { " on" } else { "" },
            label = esc(label),
            b = if badge > 0 { format!(r#" <span class="lm-badge">{badge}</span>"#) } else { String::new() },
        )
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{}{}{}{}{}{}</div>"#,
        tab("overview", "Overview", BASE, 0),
        tab("listings", "Properties", LISTINGS, 0),
        tab("enquiries", "Enquiries", ENQUIRIES, new_enquiries),
        tab("agents", "Agents", AGENTS, 0),
        if admin { tab("types", "Types", "/ghost/properties/types/", 0) } else { String::new() },
        if admin { tab("settings", "Settings", "/ghost/properties/settings/", 0) } else { String::new() },
    )
}

async fn tabs_for(state: &AppState, active: &str, actor: &Actor) -> String {
    let n = repo::enquiry_counts(&state.db).await.get("new").copied().unwrap_or(0);
    tabs(active, actor.role.is_admin(), n)
}

fn tile(label: &str, value: &str, detail: &str, href: Option<&str>, icon: &str, tint: &str) -> String {
    let body = format!(
        r#"<span class="ic tint-{tint}">{ic}</span><span class="dr-tile-value">{value}</span><span class="dr-tile-label">{label}</span><span class="dr-tile-detail">{detail}</span>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    );
    match href {
        Some(h) => format!(r#"<a class="card padded dr-tile" href="{h}">{body}</a>"#),
        None => format!(r#"<div class="card padded dr-tile">{body}</div>"#),
    }
}

fn status_chip(status: &str) -> (&'static str, &'static str) {
    match Status::from_key(status) {
        Status::Published => ("published", "Live"),
        Status::Draft => ("tonal", "Draft"),
        Status::Archived => ("draft", "Archived"),
    }
}

fn market_chip(p: &repo::Property) -> String {
    let m = p.market();
    let class = match m {
        Market::Available => "scheduled",
        Market::UnderOffer => "tonal",
        Market::Sold | Market::Let => "error",
    };
    format!(r#"<span class="chip {class}">{}</span>"#, esc(m.badge(p.purpose())))
}

fn date(d: &str) -> String {
    esc(d.get(..10).unwrap_or(d))
}

/// "3 bd · 2.5 ba · 1,850 sq ft"
fn specs(s: &Settings, p: &repo::Property) -> String {
    let mut bits = Vec::new();
    if p.bedrooms > 0 {
        bits.push(format!("{} bd", p.bedrooms));
    }
    let b = p.baths_label();
    if !b.is_empty() {
        bits.push(format!("{b} ba"));
    }
    let a = rules::area_label(p.area, s.area_unit);
    if !a.is_empty() {
        bits.push(a);
    }
    bits.join(" · ")
}

fn property_row(s: &Settings, p: &repo::Property, types: &[repo::PropertyType], agents: &[repo::Agent]) -> String {
    let (chip, label) = status_chip(&p.status);
    let t = p.type_id.as_deref().and_then(|id| types.iter().find(|t| t.id == id)).map(|t| t.name.clone());
    let a = p.agent_id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id)).map(|a| a.name.clone());
    let detail = [Some(p.price_label(&s.currency)), t, Some(p.short_location()).filter(|l| !l.is_empty()), Some(specs(s, p)).filter(|x| !x.is_empty()), a]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    format!(
        r#"<div class="row dr-listing pr-row">{thumb}
<div class="body"><span class="title"><a href="{LISTINGS}{id}/">{title}</a> <span class="chip {chip}">{label}</span> {market}{feat}</span>
<span class="detail">{detail}</span></div>
<div class="trailing pr-row-nums"><span title="Views">{eye} {views}</span><span title="Enquiries">{mail} {enq}</span><a class="icon-btn" href="{LISTINGS}{id}/" title="Edit">{edit}</a></div></div>"#,
        thumb = if p.image.is_empty() { format!(r#"<span class="ic tint-blue">{}</span>"#, svg("house")) } else { format!(r#"<img class="dr-thumb pr-thumb" src="{}" alt="" loading="lazy">"#, esc(&p.image)) },
        id = esc(&p.id),
        title = esc(&p.title),
        market = market_chip(p),
        feat = if p.is_featured() { r#" <span class="chip tonal">Featured</span>"# } else { "" },
        detail = esc(&detail),
        eye = svg("eye"),
        views = p.views,
        mail = svg("mail"),
        enq = p.enquiries,
        edit = svg("edit"),
    )
}

fn median(prices: &[i64]) -> i64 {
    match prices.len() {
        0 => 0,
        n if n % 2 == 1 => prices[n / 2],
        n => (prices[n / 2 - 1] + prices[n / 2]) / 2,
    }
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn properties_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = property_settings(&state);
    let since = vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(30)));
    let counts = repo::counts(&state.db).await;
    let n = |k: &str| counts.get(k).copied().unwrap_or(0);
    let total: i64 = ["draft", "published", "archived"].iter().map(|k| n(&format!("status:{k}"))).sum();
    let enquiry_counts = repo::enquiry_counts(&state.db).await;
    let new_enquiries = enquiry_counts.get("new").copied().unwrap_or(0);
    let (closed, closed_value) = repo::closed_since(&state.db, &since).await;
    let views = repo::total_views(&state.db).await;
    let med = median(&repo::sale_prices(&state.db).await);
    let types = repo::types(&state.db).await;
    let agents = repo::agents(&state.db, false).await;

    let live = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>The property catalogue is live</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{home}</a> · visitors search, save, compare and enquire there.</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/properties/"),
            site = esc(state.site_url().trim_end_matches('/')),
            home = esc(&s.paths.public_path("/properties/")),
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>The property catalogue is not public yet.</b>
<p class="muted">Switch it on in Settings to publish your properties at {home}.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            home = esc(&s.paths.public_path("/properties/")),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/properties/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };

    // Thirty bars, one a day.
    let per_day: HashMap<String, i64> = repo::enquiries_per_day(&state.db, &since).await.into_iter().collect();
    let days: Vec<(String, i64)> = (0..30)
        .rev()
        .map(|i| {
            let d = (chrono::Utc::now() - chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
            let v = per_day.get(&d).copied().unwrap_or(0);
            (d, v)
        })
        .collect();
    let month: i64 = days.iter().map(|(_, v)| v).sum();
    let peak = days.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let chart = format!(
        r#"<div class="card padded lm-chart-card"><div class="lm-chart-head"><b>{month}</b><span class="muted">enquiries in the last 30 days</span></div>
<div class="lm-chart pr-chart" role="img" aria-label="Enquiries per day">{bars}</div></div>"#,
        bars = days.iter().map(|(d, v)| format!(r#"<span title="{d}: {v}" style="--h:{h}%"></span>"#, h = (v * 100 / peak).max(if *v > 0 { 6 } else { 2 }))).collect::<String>(),
    );

    let (recent, _) = repo::browse_enquiries(&state.db, &repo::EnquiryQuery { limit: 6, ..Default::default() }).await;
    let recent_list = if recent.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No enquiries yet. Questions and viewing requests from property pages arrive here.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, recent.iter().map(enquiry_row).collect::<String>())
    };
    let (top, _) = repo::browse(&state.db, &repo::Browse { sort: "views", limit: 6, ..Default::default() }).await.unwrap_or_default();
    let top_list = if top.is_empty() {
        format!(
            r#"<div class="card padded lm-empty"><span class="ic tint-blue">{ic}</span><h3>List your first property</h3>
<p class="muted">Add photos, the price, rooms and sizes, the address and pin, features, floor plans and open-house times. Or start from a sample agency you can edit.</p>
<div class="lm-empty-actions"><a class="btn" href="{LISTINGS}new/">{add} New property</a>{sample}</div></div>"#,
            ic = svg("house"),
            add = svg("add"),
            sample = if actor.role.is_admin() { format!(r#"<form method="post" action="/ghost/properties/sample/"><button class="btn tonal" type="submit">{} Load sample properties</button></form>"#, svg("box")) } else { String::new() },
        )
    } else {
        format!(r#"<div class="card rows">{}</div>"#, top.iter().map(|p| property_row(&s, p, &types, &agents)).collect::<String>())
    };

    page_for_role(
        actor.role,
        "Real Estate & Property Listings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{live}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}{e}</div>
<div class="lm-overview rise" style="--i:4"><div><div class="section">Enquiries</div>{chart}
<div class="section">Latest enquiries</div>{recent_list}</div>
<div><div class="section">Most viewed</div>{top_list}</div></div>"#,
            head = crate::tools::tool_head("properties", None, &format!(
                r#"<a class="btn text" href="{BASE}export.csv">{dl} Export CSV</a><a class="btn" href="{LISTINGS}new/">{add} New property</a>"#,
                dl = svg("import"),
                add = svg("add"),
            )),
            tabs = tabs("overview", actor.role.is_admin(), new_enquiries),
            banner = flash_banner(&q),
            a = tile("For sale", &n("live:sale").to_string(), &if med > 0 { format!("Median asking {}", rules::amount(med, &s.currency)) } else { "On the site now".into() }, Some(&format!("{LISTINGS}?purpose=sale&status=published")), "house", "blue"),
            b = tile("To rent", &n("live:rent").to_string(), "On the site now", Some(&format!("{LISTINGS}?purpose=rent&status=published")), "key", "teal"),
            c = tile("Sold & let", &closed.to_string(), &if closed_value > 0 { format!("{} in sales · 30 days", rules::amount(closed_value, &s.currency)) } else { "In the last 30 days".into() }, Some(&format!("{LISTINGS}?market=sold")), "check", "green"),
            d = tile("New enquiries", &new_enquiries.to_string(), &format!("{} in all", enquiry_counts.iter().filter(|(k, _)| k.as_str() != "spam").map(|(_, v)| v).sum::<i64>()), Some(&format!("{ENQUIRIES}?status=new")), "mail", if new_enquiries > 0 { "amber" } else { "slate" }),
            e = tile("Property views", &views.to_string(), &format!("{} · {} drafts", countable(total, "property"), n("status:draft")), Some(LISTINGS), "eye", "purple"),
        ),
        true,
    )
    .into_response()
}

fn enquiry_row(e: &repo::Enquiry) -> String {
    let (chip, label) = enquiry_chip(&e.status);
    format!(
        r#"<div class="row dr-listing"><span class="lm-avatar">{initial}</span>
<div class="body"><span class="title"><a href="{ENQUIRIES}{id}/">{who}</a> <span class="chip {chip}">{label}</span>{kind}</span>
<span class="detail">{what} · {when}</span></div></div>"#,
        initial = esc(&e.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()),
        id = esc(&e.id),
        who = esc(&e.name),
        kind = if e.kind == "viewing" { r#" <span class="chip tonal">Viewing</span>"# } else { "" },
        what = esc(if e.property_title.is_empty() { "General enquiry" } else { &e.property_title }),
        when = date(&e.created_at),
    )
}

fn enquiry_chip(status: &str) -> (&'static str, &'static str) {
    match status {
        "new" => ("scheduled", "New"),
        "contacted" => ("tonal", "Contacted"),
        "viewing" => ("published", "Viewing booked"),
        "spam" => ("error", "Spam"),
        _ => ("draft", "Closed"),
    }
}

// ---------------------------------------------------------------------------
// the property list
// ---------------------------------------------------------------------------

/// The list's filters, each reduced to a known value so the links rebuilt
/// from them cannot carry anything else.
struct ListFilters {
    query: String,
    status: Option<&'static str>,
    purpose: Option<&'static str>,
    market: Option<&'static str>,
    type_id: Option<String>,
    agent_id: Option<String>,
}

fn list_filters(q: &HashMap<String, String>, types: &[repo::PropertyType], agents: &[repo::Agent]) -> ListFilters {
    ListFilters {
        query: q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default(),
        status: Status::ALL.iter().map(|s| s.key()).find(|k| q.get("status").map(String::as_str) == Some(*k)),
        purpose: Purpose::ALL.iter().map(|s| s.key()).find(|k| q.get("purpose").map(String::as_str) == Some(*k)),
        market: Market::ALL.iter().map(|s| s.key()).find(|k| q.get("market").map(String::as_str) == Some(*k)),
        type_id: q.get("type").filter(|t| types.iter().any(|x| &x.id == *t)).cloned(),
        agent_id: q.get("agent").filter(|t| agents.iter().any(|x| &x.id == *t)).cloned(),
    }
}

pub(crate) async fn listings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = property_settings(&state);
    let types = repo::types(&state.db).await;
    let agents = repo::agents(&state.db, false).await;
    let f = list_filters(&q, &types, &agents);
    let terms = vuuut_core::directory::search_terms(&f.query);
    let (items, total) = repo::browse(
        &state.db,
        &repo::Browse {
            terms: &terms,
            status: f.status,
            purpose: f.purpose,
            market: f.market,
            type_id: f.type_id.as_deref(),
            agent_id: f.agent_id.as_deref(),
            sort: "updated",
            limit: 500,
            ..Default::default()
        },
    )
    .await
    .unwrap_or_default();
    let opts = |list: &[(&str, &str)], current: Option<&str>| -> String {
        list.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if current.unwrap_or("") == *v { " selected" } else { "" })).collect()
    };
    let type_opts: String = std::iter::once(r#"<option value="">All types</option>"#.to_string())
        .chain(types.iter().map(|t| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&t.id), s = if f.type_id.as_deref() == Some(t.id.as_str()) { " selected" } else { "" }, n = esc(&t.name))))
        .collect();
    let agent_opts: String = std::iter::once(r#"<option value="">All agents</option>"#.to_string())
        .chain(agents.iter().map(|t| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&t.id), s = if f.agent_id.as_deref() == Some(t.id.as_str()) { " selected" } else { "" }, n = esc(&t.name))))
        .collect();
    page_for_role(
        actor.role,
        "Properties",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="dr-filter pr-filter rise" style="--i:2" method="get" action="{LISTINGS}">
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{q}" placeholder=" " maxlength="100"><span>Search title, town, postcode, reference</span></label>
<label class="field fixed" style="margin:0"><select name="purpose">{pu}</select><span>Sale or rent</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="market">{mk}</select><span>Market</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="status">{st}</select><span>Status</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="type">{type_opts}</select><span>Type</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="agent">{agent_opts}</select><span>Agent</span>{caret}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("properties", BASE, None, &format!(r#"<a class="btn" href="{LISTINGS}new/">{} New property</a>"#, svg("add"))),
            tabs = tabs_for(&state, "listings", &actor).await,
            banner = flash_banner(&q),
            q = esc(&f.query),
            pu = opts(&[("", "Both"), ("sale", "For sale"), ("rent", "To rent")], f.purpose),
            mk = opts(&[("", "Any"), ("available", "Available"), ("under_offer", "Under offer"), ("sold", "Sold"), ("let", "Let")], f.market),
            st = opts(&[("", "Any status"), ("published", "Live"), ("draft", "Draft"), ("archived", "Archived")], f.status),
            caret = SELECT_CARET,
            search = svg("search"),
            count = esc(&countable(total, "property")),
            body = if items.is_empty() {
                format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No properties match.</p><a class="btn" href="{LISTINGS}new/">{} New property</a></div>"#, svg("add"))
            } else {
                format!(r#"<div class="card rows">{}</div>"#, items.iter().map(|p| property_row(&s, p, &types, &agents)).collect::<String>())
            },
        ),
        true,
    )
    .into_response()
}

/// Every property, for a spreadsheet. Formulas defused, every cell quoted.
pub(crate) async fn export_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let s = property_settings(&state);
    let types = repo::types(&state.db).await;
    let agents = repo::agents(&state.db, false).await;
    let (rows, _) = repo::browse(&state.db, &repo::Browse { sort: "title", limit: 1000, ..Default::default() }).await.unwrap_or_default();
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("reference,title,status,purpose,market,type,agent,price,currency,rent_period,bedrooms,bathrooms,area,area_unit,address,city,postal_code,country,latitude,longitude,features,views,enquiries,url\n");
    for p in &rows {
        let t = p.type_id.as_deref().and_then(|id| types.iter().find(|t| t.id == id)).map(|t| t.name.clone()).unwrap_or_default();
        let a = p.agent_id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id)).map(|a| a.name.clone()).unwrap_or_default();
        let price = if p.price_poa != 0 { "POA".to_string() } else { format!("{}.{:02}", p.price_cents / 100, p.price_cents % 100) };
        out.push_str(
            &[
                cell(&p.reference), cell(&p.title), cell(&p.status), cell(&p.purpose), cell(&p.market), cell(&t), cell(&a), cell(&price), cell(&s.currency),
                cell(&p.rent_period), cell(&p.bedrooms.to_string()), cell(&p.baths_label()), cell(&p.area.to_string()), cell(s.area_unit.label()),
                cell(&p.address), cell(&p.city), cell(&p.postal_code), cell(&p.country), cell(&p.latitude), cell(&p.longitude), cell(&p.features),
                cell(&p.views.to_string()), cell(&p.enquiries.to_string()),
                cell(&vuuut_core::url::absolute(&state.site_url(), &public_href(&s, &format!("/properties/{}/", p.slug)))),
            ]
            .join(","),
        );
        out.push('\n');
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"properties-{}.csv\"", crate::export_date_stamp())),
        ],
        out,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forged_list_filter_collapses_to_the_default() {
        let q: HashMap<String, String> = [("status", "<script>"), ("purpose", "rent"), ("market", "x"), ("type", "nope")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let f = list_filters(&q, &[], &[]);
        assert_eq!((f.status, f.purpose, f.market, f.type_id), (None, Some("rent"), None, None));
    }

    #[test]
    fn the_median_is_the_middle_price() {
        assert_eq!(median(&[]), 0);
        assert_eq!(median(&[1, 5, 9]), 5);
        assert_eq!(median(&[2, 4]), 3);
    }

    #[test]
    fn a_hostile_title_is_escaped_in_the_list() {
        let p = repo::Property { id: "x".into(), title: "<img src=x onerror=alert(1)>".into(), status: "published".into(), purpose: "sale".into(), market: "available".into(), ..Default::default() };
        let html = property_row(&Settings::default(), &p, &[], &[]);
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img src=x"));
    }
}
