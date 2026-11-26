//! Property settings: whether the catalogue is public and where, how it
//! lists, the area unit, enquiries and the mortgage calculator.
//! Administrator and above. Saved through `Settings::read`'s keys, the
//! reader the public catalogue uses.

use super::*;
use vuuut_core::properties as p;

const SETTINGS: &str = "/ghost/properties/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing dr-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

fn small(name: &str, value: &str, label: &str, extra: &str) -> String {
    format!(r#"<label class="field" style="margin:0;max-width:130px"><input name="{name}" value="{v}" placeholder=" "{extra} aria-label="{l}"><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

pub(crate) async fn property_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = property_settings(&state);
    let site = state.site_url();
    let units = format!(
        r#"<label class="field fixed" style="margin:0"><select name="{k}"><option value="sqft"{a}>Square feet</option><option value="m2"{b}>Square metres</option></select><span>Unit</span>{SELECT_CARET}</label>"#,
        k = p::SETTING_AREA_UNIT,
        a = if s.area_unit == p::AreaUnit::SqFt { " selected" } else { "" },
        b = if s.area_unit == p::AreaUnit::SqM { " selected" } else { "" },
    );
    page_for_role(
        actor.role,
        "Property settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="pr-settings">
<div class="section">The catalogue</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{t_key}" value="{title}" maxlength="60" placeholder="{t_default}"><span>Title</span></label>
<label class="field" style="margin:10px 0 0"><textarea name="{i_key}" rows="2" maxlength="400" placeholder=" ">{intro}</textarea><span>Introduction, above the search</span></label></div></div>
{per}
{unit}
{sold}
{currency}
</div>
<div class="section">Address</div>
<div class="card padded rise" style="--i:3">
<label class="ec-urlfield"><span class="ec-urlfield-label">Catalogue</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{b_key}" value="{base}" placeholder="properties" maxlength="80" spellcheck="false" autocomplete="off"><span class="ec-urlfield-site">/property-name/</span></span></label>
<p class="muted" style="margin:8px 0 0">Properties, types (…/type/house/), agents (…/agent/name/), saved properties and the compare page all live under this address.</p>
</div>
<div class="section">Enquiries</div>
<div class="card rows rise" style="--i:3">
{enquiries}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{n_key}" type="email" value="{notify}" maxlength="191" placeholder=" "><span>Also send every enquiry to (besides the property's agent)</span></label></div></div>
{contact}
</div>
<div class="section">Mortgage calculator</div>
<div class="card rows rise" style="--i:4">
{mortgage}
{rates}
</div>
</form>"#,
            head = crate::tools::tool_head_back("properties", BASE, Some("pr-settings"), ""),
            tabs = tabs_for(&state, "settings", &actor).await,
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Show the catalogue on the site", &format!("Public address {}", s.paths.public_path("/properties/")), &switch(p::SETTING_ENABLED, s.enabled)),
            t_key = p::SETTING_TITLE,
            title = esc(state.settings.get_str(p::SETTING_TITLE).unwrap_or_default().trim_matches('"')),
            t_default = p::DEFAULT_TITLE,
            i_key = p::SETTING_INTRO,
            intro = esc(&s.intro),
            per = row("layout", "blue", "Properties per page", "How many the search shows at once.", &small(p::SETTING_PER_PAGE, &s.per_page.to_string(), "Show", r#" type="number" min="1" max="96""#)),
            unit = row("house", "teal", "Area unit", "Floor and plot sizes are entered and shown in this unit.", &units),
            sold = row("check", "green", "Keep sold and let properties on the site", "Marked Sold or Let, after the available ones. Off: they leave the site when sold.", &switch(p::SETTING_SHOW_SOLD, s.show_sold)),
            currency = row("sell", "amber", "Currency", "Set in Payments.", &format!(r#"<a href="/ghost/settings/payments/#currency">{}</a>"#, esc(&s.currency))),
            b_key = p::SETTING_BASE_PATH,
            base = esc(&s.paths.base),
            site = esc(site.trim_end_matches('/')),
            enquiries = row("mail", "amber", "Enquiry and viewing forms", "“Contact the agent” and “Request a viewing” on every property page.", &switch(p::SETTING_ENQUIRIES, s.enquiries)),
            n_key = p::SETTING_NOTIFY_EMAIL,
            notify = esc(&s.notify_email),
            contact = row("person", "blue", "Show agents' email and phone", "On property and agent pages. Off: visitors use the form only.", &switch(p::SETTING_SHOW_AGENT_CONTACT, s.show_agent_contact)),
            mortgage = row("percent", "purple", "Mortgage calculator", "On properties for sale: monthly repayment from price, deposit, rate and term.", &switch(p::SETTING_MORTGAGE, s.mortgage)),
            rates = format!(
                r#"<div class="row"><div class="body"><span class="title">Starting values</span><span class="detail">Visitors can change them; these are what the calculator opens with.</span>
<div class="pr-grid3" style="margin-top:10px">{}{}{}</div></div></div>"#,
                small(p::SETTING_RATE, &p::rate_label(s.rate_bp), "Interest %", r#" inputmode="decimal""#),
                small(p::SETTING_TERM, &s.term_years.to_string(), "Term (years)", r#" type="number" min="1" max="50""#),
                small(p::SETTING_DEPOSIT, &s.deposit_pct.to_string(), "Deposit %", r#" type="number" min="0" max="100""#),
            ),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn property_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = property_settings(&state);
    let Some(paths) = p::Paths::new(form.get(p::SETTING_BASE_PATH).map(String::as_str).unwrap_or(&current.paths.base)) else {
        return flash_to(SETTINGS, "error", "Enter an available address of up to three parts, using letters, numbers and dashes.");
    };
    if paths != current.paths {
        let admin = state.admin_slug();
        if vuuut_core::admin_address::suffix(&paths.base, &admin).is_some() {
            return flash_to(SETTINGS, "error", "Choose an address outside the admin.");
        }
    }
    let Some(rate) = form.get(p::SETTING_RATE).and_then(|v| p::parse_rate(v)) else {
        return flash_to(SETTINGS, "error", "Enter an interest rate between 0 and 30, like 6.5.");
    };
    let notify = vuuut_core::directory::clean(form.get(p::SETTING_NOTIFY_EMAIL).map(String::as_str).unwrap_or(""), 191);
    if !notify.is_empty() && !vuuut_core::directory::is_email(&notify) {
        return flash_to(SETTINGS, "error", "That notification address does not look right.");
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let int = |k: &str, d: i64, lo: i64, hi: i64| form.get(k).and_then(|v| v.trim().parse::<i64>().ok()).filter(|n| (lo..=hi).contains(n)).unwrap_or(d);
    let unit = p::AreaUnit::from_key(form.get(p::SETTING_AREA_UNIT).map(String::as_str).unwrap_or("sqft"));
    let writes = [
        (p::SETTING_ENABLED, serde_json::json!(flag(p::SETTING_ENABLED))),
        (p::SETTING_TITLE, serde_json::json!(vuuut_core::directory::clean(form.get(p::SETTING_TITLE).map(String::as_str).unwrap_or(""), 60))),
        (p::SETTING_INTRO, serde_json::json!(vuuut_core::directory::clean(form.get(p::SETTING_INTRO).map(String::as_str).unwrap_or(""), 400))),
        (p::SETTING_PER_PAGE, serde_json::json!(int(p::SETTING_PER_PAGE, p::DEFAULT_PER_PAGE, 1, 96))),
        (p::SETTING_AREA_UNIT, serde_json::json!(unit.key())),
        (p::SETTING_SHOW_SOLD, serde_json::json!(flag(p::SETTING_SHOW_SOLD))),
        (p::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (p::SETTING_ENQUIRIES, serde_json::json!(flag(p::SETTING_ENQUIRIES))),
        (p::SETTING_NOTIFY_EMAIL, serde_json::json!(notify)),
        (p::SETTING_SHOW_AGENT_CONTACT, serde_json::json!(flag(p::SETTING_SHOW_AGENT_CONTACT))),
        (p::SETTING_MORTGAGE, serde_json::json!(flag(p::SETTING_MORTGAGE))),
        (p::SETTING_RATE, serde_json::json!(p::rate_label(rate))),
        (p::SETTING_TERM, serde_json::json!(int(p::SETTING_TERM, 25, 1, 50))),
        (p::SETTING_DEPOSIT, serde_json::json!(int(p::SETTING_DEPOSIT, 20, 0, 100))),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("properties: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    crate::tools::tool_saved(&state, SETTINGS, "Property settings saved.")
}
