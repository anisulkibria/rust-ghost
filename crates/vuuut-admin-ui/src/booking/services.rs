//! Services: the list, and the service editor — the same two-column editor
//! as a product or a course: the description in Koenig, scheduling, place,
//! intake questions and search listing down the middle; status, price and
//! payment, who gives it and its category down the side.

use super::*;
use vuuut_core::booking::{Location, Payment, ServiceStatus, StaffMode, TINTS};

const SERVICES: &str = "/ghost/booking/services/";

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

pub(super) fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

pub(super) fn check(name: &str, value: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="{v}"{c}> {l}</label>"#, v = esc(value), c = if on { " checked" } else { "" }, l = esc(label))
}

pub(super) fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, v = esc(v), s = if v == current { " selected" } else { "" }, l = esc(l))).collect();
    format!(r#"<label class="field fixed"><select name="{name}">{o}</select><span>{l}</span>{SELECT_CARET}</label>"#, l = esc(label))
}

pub(crate) async fn services_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let list = repo::services(&state.db, false).await;
    let cats = repo::categories(&state.db).await;
    let links = repo::links(&state.db).await;
    let staff = repo::staff(&state.db, false).await;
    let rows: String = list
        .iter()
        .map(|v| {
            let who: Vec<&str> = links.iter().filter(|(sv, _)| sv == &v.id).filter_map(|(_, st)| staff.iter().find(|p| &p.id == st).map(|p| p.name.as_str())).collect();
            let (chip, label) = match v.status.as_str() { "active" => ("published", "Bookable"), "hidden" => ("tonal", "Link only"), _ => ("draft", "Draft") };
            format!(
                r#"<div class="row dr-listing">{thumb}<div class="body"><span class="title"><a href="{SERVICES}{id}/">{name}</a> <span class="chip {chip}">{label}</span>{ext}</span>
<span class="detail">{dur} · {price}{group}{cat} · {who}</span></div><div class="trailing"><a class="icon-btn" href="{SERVICES}{id}/" title="Edit">{edit}</a></div></div>"#,
                thumb = if v.image.is_empty() { format!(r#"<span class="ic tint-{}">{}</span>"#, esc(&v.tint), svg("calendar")) } else { format!(r#"<img class="dr-thumb" src="{}" alt="" loading="lazy">"#, esc(&v.image)) },
                id = esc(&v.id),
                name = esc(&v.name),
                ext = if v.external_url.is_empty() { "" } else { r#" <span class="chip tonal">External</span>"# },
                dur = esc(&rules::duration_label(v.duration_minutes)),
                price = esc(&rules::format_price(v.price_cents, &s.currency)),
                group = if v.capacity > 1 { format!(" · group of {}", v.capacity) } else { String::new() },
                cat = v.category_id.as_deref().and_then(|c| cats.iter().find(|x| x.id == c)).map(|c| format!(" · {}", esc(&c.name))).unwrap_or_default(),
                who = if who.is_empty() { "anyone on the team".to_string() } else { esc(&who.join(", ")) },
                edit = svg("edit"),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Services",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}<p class="muted dr-lead" style="margin-top:16px">{count}. Customers see bookable services in this order.</p><div class="rise" style="--i:3">{body}</div>"#,
            head = crate::tools::tool_head_back("booking", BASE, None, &format!(r#"<a class="btn" href="{SERVICES}new/">{} New service</a>"#, svg("add"))),
            tabs = tabs_for(&state, "services", &actor).await,
            banner = flash_banner(&q),
            count = esc(&countable(list.len() as i64, "service")),
            body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0 0 12px">No services yet.</p><a class="btn" href="{SERVICES}new/">{} New service</a></div>"#, svg("add")) } else { format!(r#"<div class="card rows">{rows}</div>"#) },
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn service_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    editor(&state, &actor, None, &q).await
}

pub(crate) async fn service_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(v) = repo::find_service(&state.db, &id).await else { return not_found_page_for(actor.role, &actor.user, "That service could not be found.") };
    editor(&state, &actor, Some(v), &q).await
}

async fn editor(state: &AppState, actor: &Actor, v: Option<repo::Service>, q: &HashMap<String, String>) -> Response {
    let s = booking_settings(state);
    let get = |f: fn(&repo::Service) -> &str| v.as_ref().map(f).unwrap_or("").to_string();
    let num = |f: fn(&repo::Service) -> i64, d: i64| v.as_ref().map(f).unwrap_or(d);
    let action = match &v { Some(v) => format!("{SERVICES}{}/", esc(&v.id)), None => format!("{SERVICES}new/") };
    let lexical = get(|v| &v.description_lexical);
    let legacy = v.as_ref().map(|v| v.description_markup()).unwrap_or_default();
    let initial = if !lexical.trim().is_empty() { Some(lexical) } else { vuuut_render::lexical::wrap_existing(Some(""), Some(&legacy)) };
    let initial = initial.map(|x| vuuut_core::url::base_assets(&vuuut_core::url::unbase_assets(&x)));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": "Describe the service — what happens, who it's for, how to prepare…",
        "siteUrl": state.site_url(),
        "siteTitle": state.settings.get_str("title").unwrap_or_default(),
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false, "stripeEnabled": false,
        "basePath": vuuut_core::url::subdir(),
        "unsplash": serde_json::Value::Null, "pinturaConfig": serde_json::Value::Null,
    });
    let image = get(|v| &v.image);
    let media = format!(
        r#"<div class="ec-media lm-cover" id="ec-media">{items}<label class="ec-media-add" title="Upload a picture"><input type="file" accept="image/*" id="ec-upload">{plus}<span>Picture</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>"#,
        items = if image.is_empty() { String::new() } else { crate::ecom::media_item(&image) },
        plus = svg("add"),
    );
    let mins = |n: i64| if n > 0 { n.to_string() } else { String::new() };
    let scheduling = format!(
        r#"<div class="ec-grid3">{dur}{bb}{ba}</div>
<div class="ec-grid3">{interval}{notice}{advance}</div>
<div class="ec-grid2">{cap}{mode}</div>
<p class="muted ec-hint">Buffers keep time free around each booking for travel, cleaning or notes. Start times step by the interval (blank: the length). Places above 1 make it a group session — several people book the same time.</p>"#,
        dur = field("duration_minutes", "Length (minutes)", &num(|v| v.duration_minutes, 60).to_string(), "number", r#" min="5" max="1440" required"#),
        bb = field("buffer_before", "Buffer before", &mins(num(|v| v.buffer_before, 0)), "number", r#" min="0" max="240""#),
        ba = field("buffer_after", "Buffer after", &mins(num(|v| v.buffer_after, 0)), "number", r#" min="0" max="240""#),
        interval = field("slot_interval", "Start every … minutes", &mins(num(|v| v.slot_interval, 0)), "number", r#" min="0" max="240""#),
        notice = field("min_notice", "Minimum notice (minutes)", &num(|v| v.min_notice, 120).to_string(), "number", r#" min="0""#),
        advance = field("max_advance", "Bookable up to … days ahead", &num(|v| v.max_advance, 60).to_string(), "number", r#" min="1" max="730""#),
        cap = field("capacity", "Places per time", &num(|v| v.capacity, 1).to_string(), "number", r#" min="1" max="1000""#),
        mode = select("staff_mode", "Who gives it", &StaffMode::ALL.iter().map(|m| (m.key().to_string(), m.label().to_string())).collect::<Vec<_>>(), v.as_ref().map(|v| v.staff_mode.as_str()).unwrap_or("any")),
    );
    let place = format!(
        r#"{lt}{loc}<p class="muted ec-hint">In person with no address uses the business address from Settings{addr}. A video link is sent in the confirmation.</p>"#,
        lt = select("location_type", "Where", &Location::ALL.iter().map(|m| (m.key().to_string(), m.label().to_string())).collect::<Vec<_>>(), v.as_ref().map(|v| v.location_type.as_str()).unwrap_or("in_person")),
        loc = field("location", "Address, room or video link", &get(|v| &v.location), "text", r#" maxlength="300""#),
        addr = if s.address.is_empty() { String::new() } else { format!(" ({})", esc(&s.address)) },
    );
    let questions = format!(
        r#"<div id="bk-questions" data-questions="{data}"></div><input type="hidden" name="questions" id="bk-questions-input" value="{data}">
<noscript><p class="muted">The question builder needs JavaScript.</p></noscript>
<p class="muted ec-hint">Name, email and phone are always asked. Add what else you need before the visit — allergies, goals, an address.</p>"#,
        data = esc(&serde_json::to_string(&v.as_ref().map(|v| v.questions()).unwrap_or_default()).unwrap_or_else(|_| "[]".into())),
    );
    let external = format!(
        r#"{f}<p class="muted ec-hint">Already use Calendly, Acuity, Cal.com, SimplyBook or Square? Paste the booking link: the service shows on your site and its button opens that scheduler instead of this one.</p>"#,
        f = field("external_url", "External booking link (optional)", &get(|v| &v.external_url), "url", r#" maxlength="500" placeholder="https://calendly.com/…""#),
    );
    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}{prefix}<b>{slug}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        prefix = esc(&s.paths.public_path("/book/")),
        slug = esc(&get(|v| &v.slug)),
        st = esc(&{ let t = get(|v| &v.seo_title); if t.is_empty() { get(|v| &v.name) } else { t } }),
        sd = esc(&{ let d = get(|v| &v.seo_description); if d.is_empty() { get(|v| &v.summary) } else { d } }),
        t = field("seo_title", "Page title", &get(|v| &v.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = format!(r#"<label class="field"><textarea name="seo_description" rows="2" placeholder=" " maxlength="320" id="ec-seo-desc">{}</textarea><span>Meta description</span></label>"#, esc(&get(|v| &v.seo_description))),
        h = field("slug", "Address on the site", &get(|v| &v.slug), "text", r#" maxlength="100" pattern="[A-Za-z0-9\-]*" id="ec-handle""#),
    );
    let status = select("status", "Status", &ServiceStatus::ALL.iter().map(|m| (m.key().to_string(), m.label().to_string())).collect::<Vec<_>>(), v.as_ref().map(|v| v.status.as_str()).unwrap_or("active"));
    let price = if num(|v| v.price_cents, 0) > 0 { let c = num(|v| v.price_cents, 0); format!("{}.{:02}", c / 100, c % 100) } else { String::new() };
    let stripe = vuuut_api::ecom::stripe(state).is_some();
    let pricing = format!(
        r#"{p}{pay}<p class="muted ec-hint">In {cur}; blank is free. {stripe}</p>{appr}"#,
        p = field("price", "Price", &price, "text", r#" inputmode="decimal""#),
        pay = select("payment", "Payment", &Payment::ALL.iter().map(|m| (m.key().to_string(), m.label().to_string())).collect::<Vec<_>>(), v.as_ref().map(|v| v.payment.as_str()).unwrap_or("onsite")),
        cur = esc(&s.currency),
        stripe = if stripe { "Online payment uses your Stripe account; the time is held for 30 minutes while they pay." } else { "Connect Stripe in Memberships to take payment online." },
        appr = check("approval", "1", "Approve each booking before it's confirmed", num(|v| v.approval, 0) != 0),
    );
    let cats = repo::categories(&state.db).await;
    let mut cat_opts = vec![(String::new(), "No category".to_string())];
    cat_opts.extend(cats.iter().map(|c| (c.id.clone(), c.name.clone())));
    let tints: Vec<(String, String)> = TINTS.iter().map(|t| (t.to_string(), t[..1].to_uppercase() + &t[1..])).collect();
    let staff = repo::staff(&state.db, false).await;
    let linked = match &v { Some(v) => repo::staff_ids_for(&state.db, &v.id).await, None => Vec::new() };
    let staff_boxes: String = staff.iter().map(|p| check("staff", &p.id, &format!("{}{}", p.name, if p.title.is_empty() { String::new() } else { format!(" — {}", p.title) }), linked.contains(&p.id))).collect();
    let org = format!(
        r#"{cat}{tint}{ord}<div class="ec-sublabel">Given by</div><div class="dr-checks bk-checks">{staff}</div><p class="muted ec-hint">None ticked: anyone on the team.</p>{feat}"#,
        cat = select("category_id", "Category", &cat_opts, &v.as_ref().and_then(|v| v.category_id.clone()).unwrap_or_default()),
        tint = select("tint", "Colour in the calendar", &tints, v.as_ref().map(|v| v.tint.as_str()).unwrap_or("blue")),
        ord = field("sort_order", "Order", &num(|v| v.sort_order, 0).to_string(), "number", r#" min="0" max="999""#),
        staff = if staff.is_empty() { r#"<p class="muted" style="margin:0">No staff yet — the business's own hours are used.</p>"#.to_string() } else { staff_boxes },
        feat = check("featured", "1", "Featured on the booking page", num(|v| v.featured, 0) != 0),
    );
    let view = v.as_ref().filter(|v| v.status != "draft" && s.enabled).map(|v| format!(r#"<a class="btn text small" href="{}" target="_blank" rel="noopener">{} View on the site</a>"#, public_href(&s, &format!("/book/{}/", v.slug)), svg("open"))).unwrap_or_default();
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{name}{summary}
<div class="ec-sublabel">Description</div><div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="description_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="description" rows="6" placeholder=" " maxlength="20000">{plain}</textarea><span>Description (plain text)</span></label></div></section>
{sched}{media}{place}{qs}{ext}{seo}"#,
        name = field("name", "Service name", &get(|v| &v.name), "text", r#" required maxlength="160" id="ec-title""#),
        summary = field("summary", "One line for the list of services", &get(|v| &v.summary), "text", r#" maxlength="240""#),
        config = esc(&config.to_string()),
        plain = esc(&get(|v| &v.description)),
        sched = card("Scheduling", "calendar", "teal", &scheduling, "bk-sched"),
        media = card("Picture", "image", "pink", &media, "bk-media"),
        place = card("Place", "globe", "blue", &place, "bk-place"),
        qs = card("Questions for the customer", "form", "amber", &questions, "bk-qs"),
        ext = card("Use another scheduler", "link", "slate", &external, "bk-ext"),
        seo = card("Search engine listing", "search", "slate", &seo, "bk-seo"),
    );
    let side = format!(
        "{st}{pr}{org}{actions}",
        st = card("Status", "eye", "green", &format!("{status}{view}"), "bk-status"),
        pr = card("Price & payment", "sell", "amber", &pricing, "bk-price"),
        org = card("Organisation", "folder", "purple", &org, "bk-org"),
        actions = if v.is_some() { format!(r#"<section class="card padded ec-card"><div class="ec-side-actions"><button class="btn text danger" type="submit" form="bk-delete">{} Delete service</button></div></section>"#, svg("delete")) } else { String::new() },
    );
    let heading = v.as_ref().map(|v| v.name.clone()).unwrap_or_else(|| "New service".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{SERVICES}" title="Back to services">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="bk-service">{check} {save}</button></div></header>
{banner}{del}
<form id="bk-service" class="ec-edit" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script><script src="{ASSETS}/react-jsx.js" defer></script><script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script><script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script><script src="{ASSETS}/booking-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = esc(&match &v { Some(v) => format!("{} · {}", rules::duration_label(v.duration_minutes), rules::format_price(v.price_cents, &s.currency)), None => "Something customers can book.".into() }),
        check = svg("check"),
        save = if v.is_none() { "Create" } else { "Save" },
        banner = flash_banner(q),
        del = v.as_ref().map(|v| format!(r#"<form method="post" action="{SERVICES}{}/delete/" id="bk-delete" data-confirm="Delete “{}”|Its past bookings stay in the list.">"#, esc(&v.id), esc(&v.name)) + "</form>").unwrap_or_default(),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

async fn read_form(state: &AppState, pairs: &[(String, String)]) -> Result<rules::ServiceInput, String> {
    let cats = repo::categories(&state.db).await;
    let staff = repo::staff(&state.db, false).await;
    let image = pairs.iter().find(|(k, _)| k == "image").map(|(_, v)| v.clone()).unwrap_or_default();
    let mut all: Vec<(String, String)> = pairs.iter().filter(|(k, _)| k != "image").cloned().collect();
    all.push(("image".into(), image));
    let mut input = rules::ServiceInput::from_form(&all, |kind, id| match kind {
        "category" => cats.iter().any(|c| c.id == id),
        "staff" => staff.iter().any(|p| p.id == id),
        _ => false,
    })?;
    let lexical = pairs.iter().find(|(k, _)| k == "description_lexical").map(|(_, v)| v.clone()).unwrap_or_default();
    if lexical.trim().starts_with('{') {
        let lexical = vuuut_core::url::unbase_assets(&lexical);
        let html = vuuut_render::lexical::to_html(&lexical);
        if html.len() > 400_000 {
            return Err("The description is too long.".into());
        }
        let plain = vuuut_render::markdown::html_to_plaintext(&html);
        input.description = vuuut_core::directory::clean_text(&plain, 20_000);
        let empty = plain.trim().is_empty() && !html.contains("<img") && !html.contains("<iframe");
        input.description_html = if empty { String::new() } else { html };
        input.description_lexical = if empty { String::new() } else { lexical };
    }
    Ok(input)
}

pub(crate) async fn service_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&format!("{SERVICES}new/"), "error", &m),
    };
    match repo::save_service(&state.db, None, &input).await {
        Ok(id) => flash_to(&format!("{SERVICES}{id}/"), "ok", "Service created."),
        Err(_) => flash_to(&format!("{SERVICES}new/"), "error", "The service could not be saved."),
    }
}

pub(crate) async fn service_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{SERVICES}{id}/");
    if repo::find_service(&state.db, &id).await.is_none() {
        return flash_to(SERVICES, "error", "That service could not be found.");
    }
    let input = match read_form(&state, &form).await {
        Ok(i) => i,
        Err(m) => return flash_to(&back, "error", &m),
    };
    match repo::save_service(&state.db, Some(&id), &input).await {
        Ok(_) => flash_to(&back, "ok", "Saved."),
        Err(_) => flash_to(&back, "error", "The service could not be saved."),
    }
}

pub(crate) async fn service_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    match repo::delete_service(&state.db, &id).await {
        Ok(()) => flash_to(SERVICES, "ok", "Service deleted."),
        Err(_) => flash_to(SERVICES, "error", "That could not be deleted."),
    }
}
