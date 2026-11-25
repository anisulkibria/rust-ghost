//! Tools → Directory & Classified Listings: the admin screens.
//!
//! The rules are [`vuuut_core::directory`], the queries
//! [`vuuut_db::repos::directory`], and the public directory at
//! `/directory/` is `vuuut_front::directory`. What is here:
//!
//! * **Overview** (`/ghost/directories/`) — what is live, what is waiting
//!   for review (with Approve and Reject on each), what expires soon.
//! * **Listings** — every listing, filterable, and the editor.
//! * **Categories** — `directories/categories.rs`.
//! * **Settings** — `directories/settings.rs`: the public directory, the
//!   submission form, moderation, lifetimes.
//!
//! ## The security contract
//!
//! Editor and above for listings and moderation; Administrator for
//! categories and settings. Every write is origin-checked. Submissions come
//! from the public and are rebuilt by `ListingInput::from_form`; everything
//! is escaped where it is shown. A submitter's name and address are
//! personal data, shown to staff only, never on the site, and the CSV
//! export defuses formulas.

use super::*;
use vuuut_core::directory::{self as rules, Kind, PriceType, Settings, Status};
use vuuut_db::repos::directory as repo;

mod categories;
mod settings;
pub(crate) use categories::*;
pub(crate) use settings::*;

const BASE: &str = "/ghost/directories/";
const LISTINGS: &str = "/ghost/directories/listings/";

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

pub(crate) fn dir_settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

fn tabs(active: &str, admin: bool) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#, on = if key == active { " on" } else { "" }, label = esc(label))
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{a}{b}{c}{d}</div>"#,
        a = tab("overview", "Overview", BASE),
        b = tab("listings", "Listings", LISTINGS),
        c = if admin { tab("categories", "Categories", "/ghost/directories/categories/") } else { String::new() },
        d = if admin { tab("settings", "Settings", "/ghost/directories/settings/") } else { String::new() },
    )
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

fn status_chip(l: &repo::Listing, now: &str) -> (&'static str, &'static str) {
    if l.status == "published" && l.expires_at.as_deref().is_some_and(|e| e <= now) {
        return ("draft", "Expired");
    }
    match Status::from_key(&l.status) {
        Status::Published => ("published", "Live"),
        Status::Pending => ("scheduled", "Waiting for review"),
        Status::Rejected => ("error", "Rejected"),
        Status::Draft => ("tonal", "Draft"),
    }
}

/// The site's public address for a listing.
fn public_href(settings: &rules::Settings, path: &str) -> String {
    vuuut_core::url::base(&settings.paths.public_path(path))
}

fn row(l: &repo::Listing, cats: &HashMap<String, String>, now: &str, moderate: bool, s: &Settings) -> String {
    let (chip, label) = status_chip(l, now);
    let price = rules::format_price(l.price_cents, &s.currency, PriceType::from_key(&l.price_type));
    let who = if l.submitter_email.is_empty() { "Staff".to_string() } else { format!("Sent by {}", if l.submitter_name.is_empty() { &l.submitter_email } else { &l.submitter_name }) };
    let actions = if moderate && l.status == "pending" {
        format!(
            r#"<form method="post" action="{LISTINGS}{id}/status/"><input type="hidden" name="action" value="publish"><button class="btn tonal small" type="submit">{ok} Approve</button></form>
<form method="post" action="{LISTINGS}{id}/status/" class="dr-reject"><input type="hidden" name="action" value="reject"><input name="reason" placeholder="Reason (optional)" maxlength="300" aria-label="Reason"><button class="btn text small" type="submit">Reject</button></form>"#,
            id = esc(&l.id),
            ok = svg("check"),
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="row dr-listing">{thumb}
<div class="body"><span class="title"><a href="{LISTINGS}{id}/">{title}</a> <span class="chip {chip}">{label}</span>{feat}</span>
<span class="detail">{kind}{cat}{loc}{price} · {who} · {views} · {clicks}</span></div>
<div class="trailing dr-actions">{actions}<a class="icon-btn" href="{LISTINGS}{id}/" title="Edit">{edit}</a></div></div>"#,
        thumb = if l.image.is_empty() {
            format!(r#"<span class="ic tint-{}">{}</span>"#, if l.kind == "classified" { "amber" } else { "teal" }, svg(if l.kind == "classified" { "tags" } else { "library" }))
        } else {
            format!(r#"<img class="dr-thumb" src="{}" alt="" loading="lazy">"#, esc(&l.image))
        },
        id = esc(&l.id),
        title = esc(&l.title),
        feat = if l.is_featured() { r#" <span class="chip tonal">Featured</span>"# } else { "" },
        kind = esc(Kind::from_key(&l.kind).label()),
        cat = l.category_id.as_ref().and_then(|c| cats.get(c)).map(|c| format!(" · {}", esc(c))).unwrap_or_default(),
        loc = if l.location.is_empty() { String::new() } else { format!(" · {}", esc(&l.location)) },
        price = price.map(|p| format!(" · {}", esc(&p))).unwrap_or_default(),
        who = esc(&who),
        views = countable(l.views, "view"),
        clicks = countable(l.contact_clicks, "contact"),
        edit = svg("edit"),
    )
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn directories_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = dir_settings(&state);
    let now = vuuut_core::dates::now_db();
    let week = vuuut_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(7)));
    let counts: HashMap<String, i64> = repo::counts(&state.db).await.into_iter().collect();
    let (expired, soon) = repo::expiry_counts(&state.db, &now, &week).await;
    let cats: HashMap<String, String> = repo::categories(&state.db).await.unwrap_or_default().into_iter().map(|c| (c.id, c.name)).collect();
    let (pending, _) = repo::browse(&state.db, &repo::Browse { status: Some("pending"), limit: 20, ..Default::default() }).await.unwrap_or_default();
    let (recent, _) = repo::browse(&state.db, &repo::Browse { visible_at: Some(&now), limit: 8, ..Default::default() }).await.unwrap_or_default();
    let n = |k: &str| counts.get(k).copied().unwrap_or(0);

    let home = s.paths.public_path("/directory/");
    let off = if s.enabled {
        format!(
            r#"<div class="card padded dr-live rise" style="--i:2"><span class="ic tint-green">{ic}</span><div><b>The directory is live</b>
<p class="muted">At <a href="{href}" target="_blank" rel="noopener">{site}{home}</a>{sub}.</p></div></div>"#,
            ic = svg("globe"),
            href = public_href(&s, "/directory/"),
            site = esc(state.site_url().trim_end_matches('/')),
            sub = if s.submissions { format!(r#" · visitors can <a href="{}" target="_blank" rel="noopener">add listings</a>"#, public_href(&s, "/directory/submit/")) } else { String::new() },
        )
    } else {
        format!(
            r#"<div class="card padded dr-live dr-off rise" style="--i:2"><span class="ic tint-amber">{ic}</span><div><b>The directory is not public yet.</b>
<p class="muted">Switch on the directory to publish it at {home}.</p></div>{btn}</div>"#,
            ic = svg("hide"),
            btn = if actor.role.is_admin() { format!(r#"<a class="btn" href="/ghost/directories/settings/">{} Settings</a>"#, svg("settings")) } else { String::new() },
        )
    };
    let queue = if pending.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">Nothing waiting for review.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{}</div>"#, pending.iter().map(|l| row(l, &cats, &now, true, &s)).collect::<String>())
    };
    let latest = if recent.is_empty() {
        format!(r#"<div class="card padded" style="text-align:center"><p class="muted" style="margin:8px 0 14px">No live listings yet.</p><a class="btn" href="{LISTINGS}new/">{} Add a listing</a></div>"#, svg("add"))
    } else {
        format!(r#"<div class="card rows">{}</div>"#, recent.iter().map(|l| row(l, &cats, &now, false, &s)).collect::<String>())
    };

    page_for_role(
        actor.role,
        "Directory & Classified Listings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{off}
<div class="dr-tiles rise" style="--i:3">{a}{b}{c}{d}</div>
<div class="section">Waiting for review</div><div class="rise" style="--i:4">{queue}</div>
<div class="section">Latest live listings</div><div class="rise" style="--i:5">{latest}</div>"#,
            head = crate::tools::tool_head("directories", None, &format!(
                r#"<a class="btn text" href="{BASE}export.csv">{dl} Export CSV</a><a class="btn" href="{LISTINGS}new/">{add} New listing</a>"#,
                dl = svg("import"),
                add = svg("add"),
            )),
            tabs = tabs("overview", actor.role.is_admin()),
            banner = flash_banner(&q),
            a = tile("Live", &n("published").to_string(), "Published listings", Some(&format!("{LISTINGS}?status=published")), "eye", "green"),
            b = tile("To review", &n("pending").to_string(), if s.moderate { "Sent from the public form" } else { "Moderation is off" }, Some(&format!("{LISTINGS}?status=pending")), "check", if n("pending") > 0 { "amber" } else { "slate" }),
            c = tile("Expiring", &soon.to_string(), &format!("Classifieds ending this week · {expired} expired"), None, "history", "purple"),
            d = tile("Categories", &cats.len().to_string(), "Ways to browse", if actor.role.is_admin() { Some("/ghost/directories/categories/") } else { None }, "folder", "blue"),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

pub(crate) async fn listings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = dir_settings(&state);
    let now = vuuut_core::dates::now_db();
    let status = q.get("status").map(|v| Status::from_key(v).key()).filter(|_| q.get("status").is_some_and(|v| matches!(v.as_str(), "pending" | "published" | "rejected" | "draft")));
    let kind = q.get("kind").map(String::as_str).filter(|k| matches!(*k, "directory" | "classified"));
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let category = q.get("category").filter(|c| categories.iter().any(|x| &x.id == *c)).cloned();
    let (items, total) = repo::browse(&state.db, &repo::Browse {
        terms: &terms,
        status,
        kind,
        category_id: category.as_deref(),
        limit: 200,
        ..Default::default()
    })
    .await
    .unwrap_or_default();
    let cats: HashMap<String, String> = categories.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
    let rows: String = items.iter().map(|l| row(l, &cats, &now, true, &s)).collect();
    let opts = |list: &[(&str, &str)], current: Option<&str>| -> String {
        list.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if current.unwrap_or("") == *v { " selected" } else { "" })).collect()
    };
    let cat_opts: String = std::iter::once(r#"<option value="">All categories</option>"#.to_string())
        .chain(categories.iter().map(|c| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&c.id), s = if category.as_deref() == Some(c.id.as_str()) { " selected" } else { "" }, n = esc(&c.name))))
        .collect();
    page_for_role(
        actor.role,
        "Listings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form class="dr-filter rise" style="--i:2" method="get" action="{LISTINGS}">
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{q}" placeholder=" " maxlength="100"><span>Search</span></label>
<label class="field fixed" style="margin:0"><select name="status">{st}</select><span>Status</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="kind">{kd}</select><span>Kind</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="category">{cat_opts}</select><span>Category</span>{caret}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted dr-lead">{count}</p>
<div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("directories", BASE, None, &format!(r#"<a class="btn" href="{LISTINGS}new/">{} New listing</a>"#, svg("add"))),
            tabs = tabs("listings", actor.role.is_admin()),
            banner = flash_banner(&q),
            q = esc(&query),
            st = opts(&[("", "Any status"), ("published", "Live"), ("pending", "Waiting for review"), ("draft", "Draft"), ("rejected", "Rejected")], status),
            kd = opts(&[("", "Both kinds"), ("directory", "Directory entries"), ("classified", "Classifieds")], kind),
            caret = SELECT_CARET,
            search = svg("search"),
            count = esc(&countable(total, "listing")),
            body = if items.is_empty() { r#"<div class="card padded"><p class="muted" style="margin:0">Nothing matches.</p></div>"#.to_string() } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

pub(crate) async fn listing_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn listing_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !vuuut_core::ids::is_objectid(&id) {
        return not_found_page_for(actor.role, &actor.user, "That listing could not be found.");
    }
    let Ok(Some(l)) = repo::find(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That listing could not be found.");
    };
    editor(&state, &actor, Some(l), &q).await
}

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

fn check(name: &str, value: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="{v}"{c}> {l}</label>"#, v = esc(value), c = if on { " checked" } else { "" }, l = esc(label))
}

/// One day's row in the opening-hours card.
fn hours_row(key: &str, name: &str, day: &rules::Day) -> String {
    let (state, shifts) = match day {
        rules::Day::Unknown => ("", Vec::new()),
        rules::Day::Closed => ("closed", Vec::new()),
        rules::Day::AllDay => ("24h", Vec::new()),
        rules::Day::Open(r) => ("open", r.clone()),
    };
    let t = |m: Option<&(u16, u16)>, close: bool| m.map(|(a, b)| { let v = if close { *b } else { *a }; format!("{:02}:{:02}", (v / 60) % 24, v % 60) }).unwrap_or_default();
    let opts: String = [("", "Not set"), ("open", "Open"), ("closed", "Closed"), ("24h", "Open 24 hours")]
        .iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if *v == state { " selected" } else { "" }))
        .collect();
    let second = shifts.get(1);
    format!(
        r#"<div class="dr-day" data-day="{key}" data-state="{state}"><b>{name}</b>
<select class="ec-in" name="hours_{key}" aria-label="{name}">{opts}</select>
<span class="dr-times"><input class="ec-in" type="time" name="hours_{key}_open" value="{o1}" aria-label="{name} opens"><span>–</span><input class="ec-in" type="time" name="hours_{key}_close" value="{c1}" aria-label="{name} closes"></span>
<span class="dr-times dr-shift2"{hide2}><input class="ec-in" type="time" name="hours_{key}_open2" value="{o2}" aria-label="{name} opens again"><span>–</span><input class="ec-in" type="time" name="hours_{key}_close2" value="{c2}" aria-label="{name} closes again"></span>
<button class="btn text small dr-add-shift" type="button"{hide_add}>+ Split</button></div>"#,
        o1 = t(shifts.first(), false),
        c1 = t(shifts.first(), true),
        o2 = t(second, false),
        c2 = t(second, true),
        hide2 = if second.is_none() { " hidden" } else { "" },
        hide_add = if second.is_some() { " hidden" } else { "" },
    )
}

async fn editor(state: &AppState, actor: &Actor, l: Option<repo::Listing>, q: &HashMap<String, String>) -> Response {
    let s = dir_settings(state);
    let now = vuuut_core::dates::now_db();
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let get = |f: fn(&repo::Listing) -> &str| l.as_ref().map(f).unwrap_or("").to_string();
    let kind = Kind::from_key(&get(|l| &l.kind));
    let status = Status::from_key(l.as_ref().map(|l| l.status.as_str()).unwrap_or("draft"));
    let cat_opts: String = std::iter::once(r#"<option value="">No category</option>"#.to_string())
        .chain(categories.iter().map(|c| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&c.id), s = if l.as_ref().and_then(|l| l.category_id.as_deref()) == Some(c.id.as_str()) { " selected" } else { "" }, n = esc(&c.name))))
        .collect();
    let pt = PriceType::from_key(&get(|l| &l.price_type));
    let price_opts: String = PriceType::ALL.iter().map(|p| format!(r#"<option value="{}"{}>{}</option>"#, p.key(), if *p == pt { " selected" } else { "" }, p.label())).collect();
    let price = l.as_ref().filter(|l| l.price_cents > 0).map(|l| format!("{}.{:02}", l.price_cents / 100, l.price_cents % 100)).unwrap_or_default();
    let action = match &l {
        Some(l) => format!("{LISTINGS}{}/", esc(&l.id)),
        None => format!("{LISTINGS}new/"),
    };

    // The description in Koenig, opened exactly as the post editor opens a
    // post; a listing that only has plain text is wrapped into the editor.
    let lexical = get(|l| &l.description_lexical);
    let legacy_html = l.as_ref().map(|l| l.description_markup()).unwrap_or_default();
    let initial = if !lexical.trim().is_empty() { Some(lexical) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(&legacy_html)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": "Tell people about this place — what it is, what it is like, what to try first…",
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false,
        "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null,
        "pinturaConfig": serde_json::Value::Null,
    });

    let images = l.as_ref().map(|l| l.images()).unwrap_or_default();
    let media = format!(
        r#"<div class="ec-media" id="ec-media">{items}<label class="ec-media-add" title="Upload photos"><input type="file" accept="image/*" multiple id="ec-upload">{plus}<span>Add photos</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<p class="muted ec-hint">The first photo is the cover on cards and at the top of the page; the rest make the gallery. Drag to reorder.</p>
<div class="ec-sublabel">Logo</div>
<div class="dr-logo" id="dr-logo"><span class="dr-logo-preview">{logo_img}</span>
<label class="field" style="flex:1;margin:0"><input name="logo" id="dr-logo-url" value="{logo}" placeholder=" " maxlength="500"><span>Logo address</span></label>
<label class="btn tonal small dr-logo-upload"><input type="file" accept="image/*" id="dr-logo-file" hidden>{up} Upload</label></div>"#,
        items = images.iter().map(|u| crate::ecom::media_item(u)).collect::<String>(),
        plus = svg("add"),
        logo = esc(&get(|l| &l.logo)),
        logo_img = if get(|l| &l.logo).is_empty() { String::new() } else { format!(r#"<img src="{}" alt="">"#, esc(&get(|l| &l.logo))) },
        up = svg("upload"),
    );

    let lat = get(|l| &l.latitude);
    let lng = get(|l| &l.longitude);
    let map = if !lat.is_empty() && !lng.is_empty() {
        let (a, b) = (lat.parse::<f64>().unwrap_or(0.0), lng.parse::<f64>().unwrap_or(0.0));
        format!(
            r#"<iframe class="dr-map" title="Map" loading="lazy" src="https://www.openstreetmap.org/export/embed.html?bbox={},{},{},{}&amp;layer=mapnik&amp;marker={a},{b}"></iframe>"#,
            b - 0.006, a - 0.004, b + 0.006, a + 0.004
        )
    } else {
        String::new()
    };
    let location = format!(
        r##"{address}
<div class="ec-grid2">{town}{region}</div>
<div class="ec-grid2">{postal}{country}</div>
<div class="ec-sublabel">On the map</div>
<div class="ec-grid2">{lat}{lng}</div>
<p class="muted ec-hint">Coordinates put a map and an exact pin on the listing. <a href="#" id="dr-find-coords" target="_blank" rel="noopener">Find them on OpenStreetMap</a> — right-click the spot, choose “Show address”, and copy the two numbers.</p>
{map}"##,
        address = field("address", "Street address", &get(|l| &l.address), "text", r#" maxlength="120" id="dr-address" autocomplete="off""#),
        town = field("location", "Town or city", &get(|l| &l.location), "text", r#" maxlength="80" id="dr-town" list="dr-towns""#),
        region = field("region", "State, province or region", &get(|l| &l.region), "text", r#" maxlength="80""#),
        postal = field("postal_code", "Postal code", &get(|l| &l.postal_code), "text", r#" maxlength="20""#),
        country = field("country", "Country", &get(|l| &l.country), "text", r#" maxlength="80" id="dr-country""#),
        lat = field("latitude", "Latitude", &lat, "text", r#" inputmode="decimal" maxlength="24" placeholder="40.7128""#),
        lng = field("longitude", "Longitude", &lng, "text", r#" inputmode="decimal" maxlength="24""#),
    );
    let contact = format!(
        r#"<div class="ec-grid2">{phone}{email}</div>
{website}{booking}
<p class="muted ec-hint">{shown}</p>"#,
        phone = field("phone", "Phone", &get(|l| &l.phone), "tel", r#" maxlength="40""#),
        email = field("email", "Email", &get(|l| &l.email), "email", r#" maxlength="191""#),
        website = field("website", "Website", &get(|l| &l.website), "url", r#" maxlength="500" inputmode="url" placeholder="https://""#),
        booking = field("booking_url", "Booking or reservation link", &get(|l| &l.booking_url), "url", r#" maxlength="500" inputmode="url""#),
        shown = if s.show_contact { "Phone and email are shown on the listing. Clicks on each are counted." } else { "Directory settings hide phone and email on the site; only the website is shown." },
    );
    let hours = l.as_ref().map(|l| l.hours()).unwrap_or_default();
    let hours_card = format!(
        r#"<div class="dr-week">{rows}</div>
<div class="dr-week-tools"><button class="btn text small" type="button" id="dr-copy-weekdays">Copy Monday to Tuesday–Friday</button><button class="btn text small" type="button" id="dr-copy-all">Copy Monday to every day</button></div>
{note}<p class="muted ec-hint">Themes use these for “Open now” and “Closes at 9 pm”, in the site's time zone. A closing time after midnight works (5 pm – 1 am).</p>"#,
        rows = rules::DAYS_OF_WEEK.iter().enumerate().map(|(i, (k, n, _))| hours_row(k, n, &hours.0[i])).collect::<String>(),
        note = field("hours_note", "Note about hours (holidays, kitchen times…)", &get(|l| &l.hours_note), "text", r#" maxlength="160""#),
    );
    let level = l.as_ref().map(|l| l.price_level).unwrap_or(0);
    let amen = rules::split_keys(&get(|l| &l.amenities));
    let pays = rules::split_keys(&get(|l| &l.payments));
    let details = format!(
        r#"<div class="ec-sublabel">Price range</div>
<div class="dr-levels" role="radiogroup" aria-label="Price range"><label><input type="radio" name="price_level" value="0"{l0}><span>Not set</span></label>{levels}</div>
<div class="ec-sublabel">Amenities and features</div>
<div class="dr-checks">{amenities}</div>
<div class="ec-sublabel">Payment accepted</div>
<div class="dr-checks">{payments}</div>
<div class="ec-grid2">{established}{video}</div>"#,
        l0 = if level == 0 { " checked" } else { "" },
        levels = rules::PRICE_LEVELS.iter().map(|(n, sign, label)| format!(r#"<label title="{label}"><input type="radio" name="price_level" value="{n}"{c}><span>{sign}</span></label>"#, c = if level == *n { " checked" } else { "" })).collect::<String>(),
        amenities = rules::AMENITIES.iter().map(|(k, lb)| check("amenity", k, lb, amen.iter().any(|a| a == k))).collect::<String>(),
        payments = rules::PAYMENTS.iter().map(|(k, lb)| check("payment", k, lb, pays.iter().any(|a| a == k))).collect::<String>(),
        established = field("established", "Year established", &get(|l| &l.established), "text", r#" inputmode="numeric" maxlength="4" pattern="[0-9]{4}""#),
        video = field("video_url", "Video (YouTube or Vimeo link)", &get(|l| &l.video_url), "url", r#" maxlength="500""#),
    );
    let socials_now = l.as_ref().map(|l| l.social_list()).unwrap_or_default();
    let socials = format!(
        r#"<div class="ec-grid2">{}</div>"#,
        rules::SOCIALS
            .iter()
            .map(|(k, label, _)| field(&format!("social_{k}"), label, socials_now.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or(""), "url", r#" maxlength="500" placeholder="https://""#))
            .collect::<String>()
    );
    let classified = format!(
        r#"<div class="ec-grid2"><label class="field fixed"><select name="price_type">{price_opts}</select><span>Price type</span>{caret}</label>{price}</div>
<p class="muted ec-hint">Shown in {currency}. A classified comes down after {days} days unless it is renewed.</p>"#,
        caret = SELECT_CARET,
        price = field("price", "Price", &price, "text", r#" inputmode="decimal""#),
        currency = esc(&s.currency),
        days = s.classified_days,
    );
    let listing_path = s.paths.public_path("/directory/x/");
    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>
{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(listing_path.trim_end_matches("x/")),
        slug = esc(&get(|l| &l.slug)),
        st = esc(&{ let t = get(|l| &l.seo_title); if t.is_empty() { get(|l| &l.title) } else { t } }),
        sd = esc(&{ let d = get(|l| &l.seo_description); if d.is_empty() { get(|l| &l.summary) } else { d } }),
        t = field("seo_title", "Page title", &get(|l| &l.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = format!(r#"<label class="field"><textarea name="seo_description" rows="2" placeholder=" " maxlength="320" id="ec-seo-desc">{}</textarea><span>Meta description</span></label>"#, esc(&get(|l| &l.seo_description))),
        h = field("slug", "Address on the site", &get(|l| &l.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
    );

    let status_opts: String = Status::ALL.iter().map(|x| format!(r#"<option value="{}"{}>{}</option>"#, x.key(), if *x == status { " selected" } else { "" }, x.label())).collect();
    let numbers = match &l {
        Some(l) => card("Numbers", "chart", "green", &format!(
            r#"<div class="ec-kpis"><div><b>{v}</b><span class="muted">views</span></div><div><b>{c}</b><span class="muted">contact clicks</span></div></div><p class="muted ec-hint">Created {created}{published}.</p>"#,
            v = l.views,
            c = l.contact_clicks,
            created = esc(l.created_at.get(..10).unwrap_or("")),
            published = l.published_at.as_deref().map(|p| format!(", published {}", esc(p.get(..10).unwrap_or(p)))).unwrap_or_default(),
        ), "dr-numbers"),
        None => String::new(),
    };
    let sent_by = match &l {
        Some(l) if !l.submitter_email.is_empty() => card("Sent by", "person", "amber", &format!(
            r#"<p style="margin:0"><b>{n}</b><br><a href="mailto:{e}">{e}</a></p><p class="muted ec-hint">From the public form. Not shown on the site.</p>"#,
            n = esc(if l.submitter_name.is_empty() { "Someone" } else { &l.submitter_name }),
            e = esc(&l.submitter_email),
        ), "dr-sent"),
        _ => String::new(),
    };
    let status_extra = match &l {
        Some(l) => {
            let (chip, label) = status_chip(l, &now);
            format!(
                r#"<p style="margin:0 0 10px"><span class="chip {chip}">{label}</span></p>{expires}{reason}"#,
                expires = l.expires_at.as_deref().map(|e| format!(r#"<p class="muted ec-hint">Listed until {}{}</p>"#, esc(e.get(..10).unwrap_or(e)), if l.kind == "classified" && l.status == "published" { format!(r#" · <button class="btn text small" type="submit" form="dr-renew">Renew for {} days</button>"#, s.classified_days) } else { String::new() })).unwrap_or_default(),
                reason = if l.status == "rejected" && !l.reject_reason.is_empty() { format!(r#"<p class="muted ec-hint">Rejected: {}</p>"#, esc(&l.reject_reason)) } else { String::new() },
            )
        }
        None => String::new(),
    };
    let actions = match &l {
        Some(x) => format!(
            r#"<section class="card padded ec-card"><div class="ec-side-actions">{view}<button class="btn text danger" type="submit" form="dr-delete">{del} Delete listing</button></div></section>"#,
            view = if rules::visible(&x.status, x.expires_at.as_deref(), &now) && s.enabled { format!(r#"<a class="btn text" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/directory/{}/", x.slug)), svg("open")) } else { String::new() },
            del = svg("delete"),
        ),
        None => String::new(),
    };
    let towns = repo::locations(&state.db, "9999").await;
    let side = format!(
        r#"{status}{org}{numbers}{sent_by}{actions}"#,
        status = card("Status", "eye", "green", &format!(
            r#"{status_extra}<label class="field fixed"><select name="status">{status_opts}</select><span>Status</span>{SELECT_CARET}</label>
<p class="muted ec-hint">Only published listings are on the site{off}.</p>{feat}"#,
            off = if s.enabled { "" } else { " — and the directory itself is switched off in Settings" },
            feat = check("featured", "1", "Featured — shown first and marked", l.as_ref().is_some_and(|l| l.is_featured())),
        ), "dr-status"),
        org = card("Organization", "folder", "blue", &format!(
            r#"<div class="dr-kinds"><label class="dr-kind"><input type="radio" name="kind" value="directory"{kd}><b>Directory entry</b><span>A business, place or service</span></label>
<label class="dr-kind"><input type="radio" name="kind" value="classified"{kc}><b>Classified</b><span>For sale, wanted, a job</span></label></div>
<label class="field fixed"><select name="category_id">{cat_opts}</select><span>Category</span>{SELECT_CARET}</label>
{keywords}<datalist id="dr-towns">{towns}</datalist>"#,
            kd = if kind == Kind::Directory { " checked" } else { "" },
            kc = if kind == Kind::Classified { " checked" } else { "" },
            keywords = field("keywords", "Keywords for search", &get(|l| &l.keywords), "text", r#" maxlength="200""#),
            towns = towns.iter().map(|t| format!(r#"<option value="{}">"#, esc(t))).collect::<String>(),
        ), "dr-org"),
    );
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}{summary}
<div class="ec-sublabel">Description</div>
<div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="description_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="description" rows="8" placeholder=" " maxlength="{md}">{plain}</textarea><span>Description (plain text)</span></label></div>
</section>
{media}{location}{contact}{hours}{details}{classified}{socials}{seo}"#,
        title = field("title", "Name", &get(|l| &l.title), "text", r#" required maxlength="120" id="ec-title""#),
        summary = field("summary", "Tagline — one line shown on cards", &get(|l| &l.summary), "text", r#" maxlength="240""#),
        config = esc(&config.to_string()),
        md = rules::MAX_DESCRIPTION,
        plain = esc(&get(|l| &l.description)),
        media = card("Photos & logo", "image", "pink", &media, "dr-media"),
        location = card("Location", "globe", "teal", &location, "dr-location"),
        contact = card("Contact", "mail", "blue", &contact, "dr-contact"),
        hours = card("Opening hours", "history", "purple", &hours_card, "dr-hours"),
        details = card("Details", "tune", "amber", &details, "dr-details"),
        classified = format!(r#"<div class="dr-only-classified">{}</div>"#, card("Classified price", "sell", "green", &classified, "dr-price")),
        socials = card("Social profiles", "link", "blue", &socials, "dr-socials"),
        seo = card("Search engine listing", "search", "slate", &seo, "dr-seo"),
    );
    let forms = match &l {
        Some(x) => format!(
            r#"<form method="post" action="{LISTINGS}{id}/delete/" id="dr-delete" data-confirm="Delete “{t}”|It is removed from the site and from here. It cannot be undone."></form><form method="post" action="{LISTINGS}{id}/status/" id="dr-renew"><input type="hidden" name="action" value="renew"></form>"#,
            id = esc(&x.id),
            t = esc(&x.title)
        ),
        None => String::new(),
    };
    let heading = l.as_ref().map(|l| l.title.clone()).unwrap_or_else(|| "New listing".into());
    let sub = match &l {
        Some(x) => format!("{} · {}", Kind::from_key(&x.kind).label(), status.label()),
        None => "A business, a place, or something for sale.".into(),
    };
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{LISTINGS}" title="Back to listings">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="dr-listing">{check} {save}</button></div></header>
{banner}{forms}
<form id="dr-listing" class="ec-edit dr-edit{classified_on}" method="post" action="{action}">
<div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script>
<script src="{ASSETS}/react-jsx.js" defer></script>
<script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script>
<script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script>
<script src="{ASSETS}/directory-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&sub),
        check = svg("check"),
        save = if l.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        classified_on = if kind == Kind::Classified { " is-classified" } else { "" },
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

/// The editor's form, rebuilt: the basics through the same rules a public
/// submission goes through, then the description and the rich fields.
async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::ListingInput, String> {
    let form: HashMap<String, String> = pairs.iter().filter(|(k, _)| k != "image").cloned().collect();
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let images: Vec<String> = pairs.iter().filter(|(k, _)| k == "image").filter_map(|(_, v)| rules::safe_image(v)).filter(|v| !v.is_empty()).collect();
    let mut form_main = form.clone();
    form_main.insert("image".into(), images.first().cloned().unwrap_or_default());
    let mut input = rules::ListingInput::from_form(&form_main, true, |id| categories.iter().any(|c| c.id == id))?;
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "gallery").cloned().collect();
    all.extend(images.iter().skip(1).map(|u| ("gallery".to_string(), u.clone())));
    input.details = rules::Details::from_form(&form, &all)?;
    let lexical = form.get("description_lexical").cloned().unwrap_or_default();
    if lexical.trim().starts_with('{') {
        let lexical = vuuut_core::url::unbase_assets(&lexical);
        let html = vuuut_render::lexical::to_html(&lexical);
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        if lexical.len() > rules::MAX_RICH || html.len() > rules::MAX_RICH {
            return Err("The description is too long.".into());
        }
        input.description = rules::clean_text(&plain, rules::MAX_DESCRIPTION);
        input.description_html = if plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe") { String::new() } else { html };
        input.description_lexical = if input.description_html.is_empty() { String::new() } else { lexical };
    }
    Ok(input)
}

fn expiry(s: &Settings) -> String {
    vuuut_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(s.classified_days)))
}

/// The status chosen in the editor's form, applied with the same rules as
/// the Publish / Reject buttons.
async fn apply_status(state: &AppState, id: &str, before: Option<&repo::Listing>, wanted: &str, kind: &str) -> vuuut_core::Result<()> {
    let s = dir_settings(state);
    let wanted = Status::from_key(wanted).key();
    if before.is_some_and(|b| b.status == wanted) && !(wanted == "published" && kind == "classified" && before.is_some_and(|b| b.expires_at.is_none())) {
        return Ok(());
    }
    let expires = match wanted {
        "published" if kind == "classified" => Some(before.and_then(|b| b.expires_at.clone()).filter(|e| *e > vuuut_core::dates::now_db()).unwrap_or_else(|| expiry(&s))),
        _ => before.and_then(|b| b.expires_at.clone()),
    };
    let reason = before.filter(|_| wanted == "rejected").map(|b| b.reject_reason.clone()).unwrap_or_default();
    repo::set_status(&state.db, id, wanted, &reason, expires.as_deref()).await
}

/// The status the form asks for; none when the form has no status field,
/// which leaves a saved listing's status as it is.
fn wanted_status(pairs: &[(String, String)]) -> Option<String> {
    pairs.iter().find(|(k, _)| k == "status").map(|(_, v)| v.clone())
}

pub(crate) async fn listing_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{LISTINGS}new/"), "error", &m),
    };
    match repo::insert(&state.db, &input, "draft", None, None).await {
        Ok(id) => {
            let wanted = wanted_status(&form).unwrap_or_else(|| "draft".into());
            if let Err(e) = apply_status(&state, &id, None, &wanted, &input.kind).await {
                tracing::error!("directory: publishing {id} failed: {}", e.message());
            }
            let msg = if wanted == "published" { "Created and published." } else { "Saved as a draft. Set the status to Published when it is ready." };
            flash_to(&format!("{LISTINGS}{id}/"), "ok", msg)
        }
        Err(e) => {
            tracing::error!("directory: creating a listing failed: {}", e.message());
            flash_to(&format!("{LISTINGS}new/"), "error", "The listing could not be saved.")
        }
    }
}

pub(crate) async fn listing_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LISTINGS, "error", "That listing could not be found.");
    }
    let back = format!("{LISTINGS}{id}/");
    let Ok(Some(before)) = repo::find(&state.db, &id).await else {
        return flash_to(LISTINGS, "error", "That listing could not be found.");
    };
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    let saved = match repo::update(&state.db, &id, &input).await {
        Ok(()) => match wanted_status(&form) {
            Some(wanted) => apply_status(&state, &id, Some(&before), &wanted, &input.kind).await,
            None => Ok(()),
        },
        Err(e) => Err(e),
    };
    match saved {
        Ok(()) => flash_to(&back, "ok", "Saved."),
        Err(e) => {
            tracing::error!("directory: saving {id} failed: {}", e.message());
            flash_to(&back, "error", "The listing could not be saved.")
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct StatusForm {
    action: String,
    reason: String,
}

pub(crate) async fn listing_status(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<StatusForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = headers.get(header::REFERER).and_then(|v| v.to_str().ok()).and_then(|r| r.find("/ghost/").map(|i| r[i..].to_string())).filter(|p| !p.contains("//")).unwrap_or_else(|| LISTINGS.to_string());
    let back = back.split('?').next().unwrap_or(LISTINGS).to_string();
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(&back, "error", "That listing could not be found.");
    }
    let Ok(Some(l)) = repo::find(&state.db, &id).await else {
        return flash_to(&back, "error", "That listing could not be found.");
    };
    let s = dir_settings(&state);
    let classified = l.kind == "classified";
    let (status, reason, expires, message) = match form.action.as_str() {
        "publish" => ("published", String::new(), classified.then(|| expiry(&s)), "Published.".to_string()),
        "renew" if classified => ("published", String::new(), Some(expiry(&s)), format!("Renewed for {} days.", s.classified_days)),
        "reject" => ("rejected", rules::clean(&form.reason, 300), None, "Rejected. It is not shown on the site.".to_string()),
        "draft" => ("draft", String::new(), l.expires_at.clone(), "Unpublished.".to_string()),
        _ => return flash_to(&back, "error", "That is not something a listing can do."),
    };
    match repo::set_status(&state.db, &id, status, &reason, expires.as_deref()).await {
        Ok(()) => flash_to(&back, "ok", &message),
        Err(e) => {
            tracing::error!("directory: changing {id} failed: {}", e.message());
            flash_to(&back, "error", "That could not be saved.")
        }
    }
}

pub(crate) async fn listing_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LISTINGS, "error", "That listing could not be found.");
    }
    match repo::delete(&state.db, &id).await {
        Ok(()) => flash_to(LISTINGS, "ok", "Listing deleted."),
        Err(_) => flash_to(LISTINGS, "error", "That could not be deleted."),
    }
}

/// Every listing, for a spreadsheet. Formulas defused, every cell quoted.
pub(crate) async fn export_csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let (items, _) = repo::browse(&state.db, &repo::Browse { limit: 500, ..Default::default() }).await.unwrap_or_default();
    let cats: HashMap<String, String> = repo::categories(&state.db).await.unwrap_or_default().into_iter().map(|c| (c.id, c.name)).collect();
    let cell = |v: &str| format!("\"{}\"", vuuut_core::forms::csv_safe(v).replace('"', "\"\""));
    let mut out = String::from("title,kind,status,category,location,price,website,email,phone,submitted_by,views,contact_clicks,created,expires\n");
    for l in &items {
        out.push_str(&[
            cell(&l.title), cell(&l.kind), cell(&l.status),
            cell(l.category_id.as_ref().and_then(|c| cats.get(c)).map(String::as_str).unwrap_or("")),
            cell(&l.location), cell(&format!("{}.{:02}", l.price_cents / 100, l.price_cents % 100)),
            cell(&l.website), cell(&l.email), cell(&l.phone), cell(&l.submitter_email),
            cell(&l.views.to_string()), cell(&l.contact_clicks.to_string()), cell(&l.created_at), cell(l.expires_at.as_deref().unwrap_or("")),
        ].join(","));
        out.push('\n');
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"directory-{}.csv\"", crate::export_date_stamp())),
        ],
        out,
    )
        .into_response()
}
