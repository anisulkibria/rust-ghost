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

async fn editor(state: &AppState, actor: &Actor, l: Option<repo::Listing>, q: &HashMap<String, String>) -> Response {
    let s = dir_settings(state);
    let now = vuuut_core::dates::now_db();
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    let new = l.is_none();
    let get = |f: fn(&repo::Listing) -> &str| l.as_ref().map(f).unwrap_or("");
    let kind = Kind::from_key(get(|l| &l.kind));
    let cat_opts: String = std::iter::once(r#"<option value="">No category</option>"#.to_string())
        .chain(categories.iter().map(|c| format!(r#"<option value="{id}"{s}>{n}</option>"#, id = esc(&c.id), s = if l.as_ref().and_then(|l| l.category_id.as_deref()) == Some(c.id.as_str()) { " selected" } else { "" }, n = esc(&c.name))))
        .collect();
    let pt = PriceType::from_key(get(|l| &l.price_type));
    let price_opts: String = PriceType::ALL.iter().map(|p| format!(r#"<option value="{}"{}>{}</option>"#, p.key(), if *p == pt { " selected" } else { "" }, p.label())).collect();
    let price = l.as_ref().filter(|l| l.price_cents > 0).map(|l| format!("{}.{:02}", l.price_cents / 100, l.price_cents % 100)).unwrap_or_default();
    let action = match &l {
        Some(l) => format!("{LISTINGS}{}/", esc(&l.id)),
        None => format!("{LISTINGS}new/"),
    };
    let side = match &l {
        None => String::new(),
        Some(l) => {
            let (chip, label) = status_chip(l, &now);
            let live = rules::visible(&l.status, l.expires_at.as_deref(), &now);
            let act = |a: &str, label: &str, cls: &str| format!(
                r#"<form method="post" action="{LISTINGS}{id}/status/"><input type="hidden" name="action" value="{a}"><button class="btn {cls}" type="submit">{label}</button></form>"#,
                id = esc(&l.id),
            );
            format!(
                r#"<div class="section">Status</div><div class="card padded dr-status">
<p><span class="chip {chip}">{label}</span></p>
{expires}{reason}
<div class="dr-status-actions">{primary}{secondary}{renew}</div>
{public}
</div>
{who}
<div class="section">Numbers</div><div class="card padded"><ul class="cp-facts"><li><span>Views</span><b>{views}</b></li><li><span>Contact clicks</span><b>{clicks}</b></li><li><span>Created</span><b>{created}</b></li></ul></div>
<div class="section">Delete</div><div class="card padded"><button class="btn tonal small danger" type="submit" form="dr-delete">{del} Delete listing</button></div>"#,
                expires = l.expires_at.as_deref().map(|e| format!(r#"<p class="muted" style="margin:0 0 8px">Expires {}</p>"#, esc(e.get(..10).unwrap_or(e)))).unwrap_or_default(),
                reason = if l.status == "rejected" && !l.reject_reason.is_empty() { format!(r#"<p class="muted" style="margin:0 0 8px">Reason: {}</p>"#, esc(&l.reject_reason)) } else { String::new() },
                primary = if l.status == "published" { act("draft", "Unpublish", "tonal") } else { act("publish", "Publish", "") },
                secondary = if l.status == "pending" { act("reject", "Reject", "text") } else { String::new() },
                renew = if l.kind == "classified" && l.status == "published" { act("renew", &format!("Renew for {} days", s.classified_days), "text") } else { String::new() },
                public = if live && s.enabled { format!(r#"<p style="margin:10px 0 0"><a href="{}" target="_blank" rel="noopener">View on the site {}</a></p>"#, public_href(&s, &format!("/directory/{}/", l.slug)), svg("open")) } else { String::new() },
                who = if l.submitter_email.is_empty() { String::new() } else {
                    format!(
                        r#"<div class="section">Sent by</div><div class="card padded"><p style="margin:0"><b>{n}</b><br><a href="mailto:{e}">{e}</a></p><p class="muted" style="margin:6px 0 0;font-size:12.5px">From the public form. Not shown on the site.</p></div>"#,
                        n = esc(if l.submitter_name.is_empty() { "Someone" } else { &l.submitter_name }),
                        e = esc(&l.submitter_email),
                    )
                },
                views = l.views,
                clicks = l.contact_clicks,
                created = esc(l.created_at.get(..10).unwrap_or("")),
                del = svg("delete"),
            )
        }
    };
    page_for_role(
        actor.role,
        if new { "New listing" } else { "Edit listing" },
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<div class="dr-split">
<form method="post" action="{action}" id="dr-form" class="dr-form">
<div class="section">The listing</div>
<div class="card padded rise" style="--i:1">
<div class="dr-kinds"><label class="dr-kind"><input type="radio" name="kind" value="directory"{kd}><b>Directory entry</b><span>A business, place, person or resource</span></label>
<label class="dr-kind"><input type="radio" name="kind" value="classified"{kc}><b>Classified</b><span>For sale, wanted, a job — expires after {days} days</span></label></div>
{title}
<div class="wf-two">{slug}<label class="field fixed"><select name="category_id">{cat_opts}</select><span>Category</span>{caret}</label></div>
{summary}
<label class="field"><textarea name="description" rows="8" maxlength="{md}" placeholder=" ">{desc}</textarea><span>Description — plain text; a blank line starts a paragraph</span></label>
{image}{keywords}
<label class="dr-check"><input type="checkbox" name="featured" value="1"{feat}> Featured — shown first and marked as featured</label>
</div>
<div class="section dr-only-classified">Price</div>
<div class="card padded rise dr-only-classified" style="--i:2"><div class="wf-two">
<label class="field fixed"><select name="price_type">{price_opts}</select><span>Price type</span>{caret}</label>
{price}</div><p class="muted dr-hint">Prices are shown in {currency}.</p></div>
<div class="section">Where and how to reach them</div>
<div class="card padded rise" style="--i:3">
<div class="wf-two">{location}{address}</div>
{website}
<div class="wf-two">{email}{phone}</div>
</div>
<div class="dr-save"><button class="btn" type="submit">{check} {save_label}</button></div>
</form>
<aside>{side}</aside>
</div>
{delete_form}"#,
            head = crate::tools::tool_head_back("directories", LISTINGS, None, ""),
            banner = flash_banner(q),
            kd = if kind == Kind::Directory { " checked" } else { "" },
            kc = if kind == Kind::Classified { " checked" } else { "" },
            days = s.classified_days,
            title = field("title", "Title", get(|l| &l.title), "text", r#" required maxlength="120""#),
            slug = field("slug", "Address on the site", get(|l| &l.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*""#),
            caret = SELECT_CARET,
            summary = field("summary", "In one line", get(|l| &l.summary), "text", r#" maxlength="240""#),
            md = rules::MAX_DESCRIPTION,
            desc = esc(get(|l| &l.description)),
            image = field("image", "Image address", get(|l| &l.image), "text", r#" maxlength="500" placeholder="https://""#),
            keywords = field("keywords", "Keywords for search", get(|l| &l.keywords), "text", r#" maxlength="200""#),
            feat = if l.as_ref().is_some_and(|l| l.is_featured()) { " checked" } else { "" },
            price = field("price", "Price", &price, "text", r#" inputmode="decimal""#),
            currency = esc(&s.currency),
            location = field("location", "Location — town or area", get(|l| &l.location), "text", r#" maxlength="80""#),
            address = field("address", "Address", get(|l| &l.address), "text", r#" maxlength="120""#),
            website = field("website", "Website", get(|l| &l.website), "text", r#" maxlength="500" inputmode="url""#),
            email = field("email", "Contact email", get(|l| &l.email), "email", r#" maxlength="191""#),
            phone = field("phone", "Phone", get(|l| &l.phone), "text", r#" maxlength="40""#),
            check = svg("check"),
            save_label = if new { "Create" } else { "Save" },
            delete_form = l.as_ref().map(|l| format!(r#"<form method="post" action="{LISTINGS}{id}/delete/" id="dr-delete" data-confirm="Delete “{t}”|It is removed from the site and from here. It cannot be undone."></form>"#, id = esc(&l.id), t = esc(&l.title))).unwrap_or_default(),
        ),
        true,
    )
    .into_response()
}

async fn read_form(state: &AppState, form: &HashMap<String, String>) -> Result<rules::ListingInput, String> {
    let categories = repo::categories(&state.db).await.unwrap_or_default();
    rules::ListingInput::from_form(form, true, |id| categories.iter().any(|c| c.id == id))
}

fn expiry(s: &Settings) -> String {
    vuuut_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(s.classified_days)))
}

pub(crate) async fn listing_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let _ = actor;
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{LISTINGS}new/"), "error", &m),
    };
    // Staff listings start as drafts; publishing is a deliberate second step.
    match repo::insert(&state.db, &input, "draft", None, None).await {
        Ok(id) => flash_to(&format!("{LISTINGS}{id}/"), "ok", "Saved as a draft. Publish it when it is ready."),
        Err(e) => {
            tracing::error!("directory: creating a listing failed: {}", e.message());
            flash_to(&format!("{LISTINGS}new/"), "error", "The listing could not be saved.")
        }
    }
}

pub(crate) async fn listing_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !vuuut_core::ids::is_objectid(&id) {
        return flash_to(LISTINGS, "error", "That listing could not be found.");
    }
    let back = format!("{LISTINGS}{id}/");
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::update(&state.db, &id, &input).await {
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
