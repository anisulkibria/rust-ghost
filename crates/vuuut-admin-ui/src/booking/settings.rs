//! Booking settings: whether online booking is open and where, the business
//! hours and time zone, policies, notifications — and the categories
//! services are grouped in. Administrator and above.

use super::services::select;
use super::staff::week_editor;
use super::*;
use vuuut_core::booking as b;

const SETTINGS: &str = "/ghost/booking/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div><div class="trailing dr-control">{control}</div></div>"#,
        ic = svg(icon), title = esc(title), detail = esc(detail),
    )
}

pub(crate) async fn booking_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = booking_settings(&state);
    let site_tz = state.settings.get_str("timezone").unwrap_or_default();
    let zones: Vec<(String, String)> = std::iter::once((String::new(), format!("The site's time zone ({})", site_tz.trim_matches('"'))))
        .chain(chrono_tz::TZ_VARIANTS.iter().map(|z| (z.name().to_string(), z.name().replace('_', " "))))
        .collect();
    let cur: Vec<(String, String)> = vuuut_core::directory::CURRENCIES.iter().map(|c| (c.to_string(), c.to_string())).collect();
    let cats = repo::categories(&state.db).await;
    let services = repo::services(&state.db, false).await;
    let tints: Vec<(String, String)> = b::TINTS.iter().map(|t| (t.to_string(), t[..1].to_uppercase() + &t[1..])).collect();
    let cat_rows: String = cats
        .iter()
        .map(|c| {
            format!(
                r#"<details class="dr-cat"><summary><span class="ic tint-{tint}">{ic}</span><b>{name}</b><span class="muted">{n}</span>{chev}</summary>
<form method="post" action="/ghost/booking/categories/{id}/" class="dr-cat-form"><div class="ec-grid2"><label class="field"><input name="name" value="{name}" required maxlength="80" placeholder=" "><span>Name</span></label>{tints}</div>
<label class="field"><input name="description" value="{desc}" maxlength="300" placeholder=" "><span>Description</span></label>
<div class="dr-cat-actions"><button class="btn tonal small danger" type="submit" form="del-{id}">Delete</button><button class="btn small" type="submit">Save</button></div></form>
<form method="post" action="/ghost/booking/categories/{id}/delete/" id="del-{id}" data-confirm="Delete {name}|Its services stay, without a category."></form></details>"#,
                tint = esc(&c.tint),
                ic = svg("folder"),
                name = esc(&c.name),
                n = countable(services.iter().filter(|x| x.category_id.as_deref() == Some(c.id.as_str())).count() as i64, "service"),
                chev = svg("chevron"),
                id = esc(&c.id),
                tints = select("tint", "Colour", &tints, &c.tint),
                desc = esc(&c.description),
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Booking settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="bk-settings" class="bk-settings">
<div class="section">Online booking</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{t_key}" value="{title}" maxlength="80" placeholder="{t_def}"><span>Title</span></label>
<label class="field" style="margin:10px 0 0"><textarea name="{i_key}" rows="2" maxlength="400" placeholder=" ">{intro}</textarea><span>Introduction, above the services</span></label></div></div>
</div>
<div class="section">Address</div>
<div class="card padded rise" style="--i:3"><label class="ec-urlfield"><span class="ec-urlfield-label">Booking page</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{b_key}" value="{base}" placeholder="book" maxlength="80" spellcheck="false"><span class="ec-urlfield-site">/service-name/</span></span></label></div>
<div class="section">Business hours & place</div>
<div class="card padded rise" style="--i:3">
<div class="ec-grid2">{tz}{cur}</div>
<label class="field"><input name="{a_key}" value="{address}" maxlength="300" placeholder=" "><span>Business address (for in-person services)</span></label>
<div class="ec-sublabel">Opening hours</div>{week}
<p class="muted ec-hint">Used for anyone whose own hours leave a day on “Business hours”, and for a business with no staff set up.</p></div>
<div class="section">Policies & email</div>
<div class="card rows rise" style="--i:4">
{cancel}
{remind}
{phone}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{n_key}" value="{notify}" type="email" maxlength="191" placeholder=" "><span>Email new bookings to (besides the person booked)</span></label></div></div>
<div class="row"><div class="body"><label class="field" style="margin:0"><textarea name="{terms_key}" rows="3" maxlength="2000" placeholder=" ">{terms}</textarea><span>Booking terms, shown above the confirm button</span></label></div></div>
</div>
</form>
<div class="section">Categories</div>
<div class="dr-split dr-split-even"><div class="card padded rise" style="--i:5">{cat_list}</div>
<form class="card padded rise" style="--i:5" method="post" action="/ghost/booking/categories/"><div class="ec-grid2"><label class="field"><input name="name" required maxlength="80" placeholder=" "><span>New category</span></label>{new_tint}</div>
<label class="field"><input name="description" maxlength="300" placeholder=" "><span>Description</span></label><div style="display:flex;justify-content:flex-end"><button class="btn" type="submit">{add} Add category</button></div></form></div>
<script src="{ASSETS}/booking-editor.js" defer></script>"#,
            head = crate::tools::tool_head_back("booking", BASE, Some("bk-settings"), ""),
            tabs = tabs_for(&state, "settings", &actor).await,
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Take bookings on the site", &format!("Public address {}", s.paths.public_path("/book/")), &switch(b::SETTING_ENABLED, s.enabled)),
            t_key = b::SETTING_TITLE,
            title = esc(&state.settings.get_str(b::SETTING_TITLE).unwrap_or_default().trim_matches('"').to_string()),
            t_def = b::DEFAULT_TITLE,
            i_key = b::SETTING_INTRO,
            intro = esc(&s.intro),
            site = esc(state.site_url().trim_end_matches('/')),
            b_key = b::SETTING_BASE_PATH,
            base = esc(&s.paths.base),
            tz = select(b::SETTING_TIMEZONE, "Time zone", &zones, &s.timezone),
            cur = select(b::SETTING_CURRENCY, "Currency", &cur, &s.currency),
            a_key = b::SETTING_ADDRESS,
            address = esc(&s.address),
            week = week_editor(&s.hours, "Not set"),
            cancel = row("history", "amber", "Customers can cancel or reschedule until", "Hours before the start. 0 lets them change any time.", &format!(r#"<label class="field" style="margin:0;width:120px"><input name="{k}" type="number" min="0" max="720" value="{v}"><span>Hours</span></label>"#, k = b::SETTING_CANCEL_HOURS, v = s.cancel_hours)),
            remind = row("mail", "blue", "Reminder email", "Hours before each confirmed booking. 0 turns reminders off.", &format!(r#"<label class="field" style="margin:0;width:120px"><input name="{k}" type="number" min="0" max="168" value="{v}"><span>Hours</span></label>"#, k = b::SETTING_REMINDER_HOURS, v = s.reminder_hours)),
            phone = row("person", "purple", "Ask for a phone number", "Required on the booking form.", &switch(b::SETTING_REQUIRE_PHONE, s.require_phone)),
            n_key = b::SETTING_NOTIFY_EMAIL,
            notify = esc(&s.notify_email),
            terms_key = b::SETTING_TERMS,
            terms = esc(&s.terms),
            cat_list = if cats.is_empty() { r#"<p class="muted" style="margin:0">None yet — e.g. Massage, Facials, Consultations.</p>"#.to_string() } else { cat_rows },
            new_tint = select("tint", "Colour", &tints, "blue"),
            add = svg("add"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn booking_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = booking_settings(&state);
    let Some(paths) = b::Paths::new(form.get(b::SETTING_BASE_PATH).map(String::as_str).unwrap_or(&current.paths.base)) else {
        return flash_to(SETTINGS, "error", "Enter an available address of up to three parts, using letters, numbers and dashes.");
    };
    if paths != current.paths && vuuut_core::admin_address::suffix(&paths.base, &state.admin_slug()).is_some() {
        return flash_to(SETTINGS, "error", "Choose an address outside the admin.");
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let tz = get(b::SETTING_TIMEZONE).trim();
    let tz = if tz.parse::<chrono_tz::Tz>().is_ok() { tz.to_string() } else { String::new() };
    let notify = vuuut_core::directory::clean(get(b::SETTING_NOTIFY_EMAIL), 191).to_lowercase();
    if !notify.is_empty() && !vuuut_core::directory::is_email(&notify) {
        return flash_to(SETTINGS, "error", "The notification email does not look right.");
    }
    let hours = vuuut_core::directory::Hours::from_form(|k| form.get(k).cloned().unwrap_or_default());
    let num = |k: &str, hi: i64| get(k).trim().parse::<i64>().ok().map(|n| n.clamp(0, hi)).unwrap_or(0);
    let currency = get(b::SETTING_CURRENCY).trim().to_uppercase();
    let writes = [
        (b::SETTING_ENABLED, serde_json::json!(flag(b::SETTING_ENABLED))),
        (b::SETTING_TITLE, serde_json::json!(vuuut_core::directory::clean(get(b::SETTING_TITLE), 80))),
        (b::SETTING_INTRO, serde_json::json!(vuuut_core::directory::clean(get(b::SETTING_INTRO), 400))),
        (b::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (b::SETTING_TIMEZONE, serde_json::json!(tz)),
        (b::SETTING_CURRENCY, serde_json::json!(if vuuut_core::directory::CURRENCIES.contains(&currency.as_str()) { currency } else { "USD".into() })),
        (b::SETTING_ADDRESS, serde_json::json!(vuuut_core::directory::clean(get(b::SETTING_ADDRESS), 300))),
        (b::SETTING_HOURS, serde_json::json!(hours.to_json())),
        (b::SETTING_CANCEL_HOURS, serde_json::json!(num(b::SETTING_CANCEL_HOURS, 720))),
        (b::SETTING_REMINDER_HOURS, serde_json::json!(num(b::SETTING_REMINDER_HOURS, 168))),
        (b::SETTING_REQUIRE_PHONE, serde_json::json!(flag(b::SETTING_REQUIRE_PHONE))),
        (b::SETTING_NOTIFY_EMAIL, serde_json::json!(notify)),
        (b::SETTING_TERMS, serde_json::json!(vuuut_core::directory::clean_text(get(b::SETTING_TERMS), 2000))),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("booking: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    crate::tools::tool_saved(&state, SETTINGS, "Booking settings saved.")
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct CategoryForm {
    name: String,
    tint: String,
    description: String,
}

async fn save_category(state: &AppState, id: Option<&str>, f: &CategoryForm) -> Response {
    let name = vuuut_core::directory::clean(&f.name, 80);
    if name.is_empty() {
        return flash_to(SETTINGS, "error", "A category needs a name.");
    }
    let slug = vuuut_core::directory::slugify(&name);
    if repo::category_slug_taken(&state.db, &slug, id).await {
        return flash_to(SETTINGS, "error", "Another category already has that name.");
    }
    let tint = if b::TINTS.contains(&f.tint.as_str()) { f.tint.as_str() } else { "blue" };
    let order = repo::categories(&state.db).await.len() as i64;
    match repo::save_category(&state.db, id, &name, &slug, &vuuut_core::directory::clean(&f.description, 300), tint, order).await {
        Ok(_) => flash_to(SETTINGS, "ok", &format!("{name} saved.")),
        Err(_) => flash_to(SETTINGS, "error", "That could not be saved."),
    }
}

pub(crate) async fn category_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<CategoryForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    save_category(&state, None, &f).await
}

pub(crate) async fn category_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<CategoryForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    save_category(&state, Some(&id), &f).await
}

pub(crate) async fn category_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    match repo::delete_category(&state.db, &id).await {
        Ok(()) => flash_to(SETTINGS, "ok", "Category deleted."),
        Err(_) => flash_to(SETTINGS, "error", "That could not be deleted."),
    }
}
