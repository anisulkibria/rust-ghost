//! Portfolio settings: personal or studio, the profile (name, headline,
//! about, portrait, location, availability, contact, résumé file, social
//! profiles), where the pages live, and the contact form. Administrator
//! and above.

use super::*;
use crate::podcast::{field, media_box, select, textarea};
use vuuut_core::portfolio as p;

const SETTINGS: &str = "/ghost/portfolio/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div><div class="trailing dr-control">{control}</div></div>"#,
        ic = svg(icon), title = esc(title), detail = esc(detail),
    )
}

pub(crate) async fn portfolio_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = portfolio_settings(&state);
    let raw = |k: &str| state.settings.get_str(k).unwrap_or_default().trim_matches('"').to_string();
    let socials: String = p::SOCIALS.iter().map(|(k, l)| field(&format!("social_{k}"), l, s.socials.iter().find(|(x, _)| x == k).map(|(_, u)| u.as_str()).unwrap_or(""), "url", r#" maxlength="500""#)).collect();
    let cur: Vec<(String, String)> = vuuut_core::directory::CURRENCIES.iter().map(|c| (c.to_string(), c.to_string())).collect();
    page_for_role(
        actor.role,
        "Portfolio settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="pf-settings">
<div class="section">The portfolio</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><div class="pf-modes">{personal}{studio}</div></div></div>
<div class="row"><div class="body">{title}{intro}</div></div>
</div>
<div class="section">Address</div>
<div class="card padded rise" style="--i:3"><label class="ec-urlfield"><span class="ec-urlfield-label">Portfolio</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{b_key}" value="{base}" placeholder="portfolio" maxlength="80" spellcheck="false"><span class="ec-urlfield-site">/project-name/</span></span></label>
<p class="muted ec-hint">Use “work” or “projects” if you prefer. To make the portfolio your home page, point the theme's home link at it, or add a redirect from / in Tools → Redirects.</p></div>
<div class="section">Profile</div>
<div class="dr-split rise" style="--i:4"><div class="card padded">
<div class="ec-grid2">{name}{headline}</div>
{about}
<div class="ec-grid3">{location}{founded}{currency}</div>
<div class="ec-grid2">{email}{phone}</div>
{resume}
</div><div class="card padded"><div class="ec-sublabel">Portrait or logo</div>{portrait}<p class="muted ec-hint">A square photo of you, or the studio's mark. Shown in the hero and on About.</p>
<div class="ec-sublabel">Availability</div>{avail}{avail_note}</div></div>
<div class="section">Profiles elsewhere</div>
<div class="card padded rise" style="--i:5"><div class="ec-grid3">{socials}</div></div>
<div class="section">Contact form</div>
<div class="card rows rise" style="--i:6">
{enquiries}
<div class="row"><div class="body"><div class="ec-grid2">{notify}<span></span></div>{budgets}</div></div>
</div>
</form>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
            head = crate::tools::tool_head_back("portfolio", BASE, Some("pf-settings"), ""),
            tabs = tabs_for(&state, "settings", &actor).await,
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Publish the portfolio", &format!("Public address {}", s.paths.public_path("/portfolio/")), &switch(p::SETTING_ENABLED, s.enabled)),
            personal = format!(r#"<label class="pf-mode"><input type="radio" name="{k}" value="personal"{c}><span><b>Personal</b><small>A designer, developer, photographer, writer, architect… Leads with you, your résumé and skills; “Hire me”.</small></span></label>"#, k = p::SETTING_MODE, c = if s.studio() { "" } else { " checked" }),
            studio = format!(r#"<label class="pf-mode"><input type="radio" name="{k}" value="studio"{c}><span><b>Studio or company</b><small>An agency, practice or production company. Leads with services, clients and the team; “Start a project”.</small></span></label>"#, k = p::SETTING_MODE, c = if s.studio() { " checked" } else { "" }),
            title = field(p::SETTING_TITLE, "Title of the work page", &raw(p::SETTING_TITLE), "text", r#" maxlength="80" placeholder="Work""#),
            intro = textarea(p::SETTING_INTRO, "Introduction under the headline", &s.intro, 2, r#" maxlength="600""#),
            site = esc(state.site_url().trim_end_matches('/')),
            b_key = p::SETTING_BASE_PATH,
            base = esc(&s.paths.base),
            name = field(p::SETTING_NAME, "Your name or the studio's", &s.name, "text", r#" maxlength="120""#),
            headline = field(p::SETTING_HEADLINE, "Headline", &s.headline, "text", r#" maxlength="200" placeholder="Product designer making calm software""#),
            about = textarea(p::SETTING_ABOUT, "About — a few paragraphs", &s.about, 6, r#" maxlength="6000""#),
            location = field(p::SETTING_LOCATION, "Based in", &s.location, "text", r#" maxlength="120""#),
            founded = field(p::SETTING_FOUNDED, if s.studio() { "Founded (year)" } else { "Working since (year)" }, &if s.founded > 0 { s.founded.to_string() } else { String::new() }, "number", r#" min="1800" max="2200""#),
            currency = select(p::SETTING_CURRENCY, "Currency for prices", &cur, &s.currency),
            email = field(p::SETTING_EMAIL, "Contact email (shown)", &s.email, "email", r#" maxlength="191""#),
            phone = field(p::SETTING_PHONE, "Phone (shown)", &s.phone, "tel", r#" maxlength="40""#),
            resume = field(p::SETTING_RESUME_URL, "Résumé PDF address (upload it in Media, then paste its address)", &s.resume_url, "text", r#" maxlength="500""#),
            portrait = media_box(&s.portrait, "Portrait"),
            avail = select(p::SETTING_AVAILABILITY, "Status", &p::Availability::options(), s.availability.key()),
            avail_note = field(p::SETTING_AVAILABILITY_NOTE, "Note (e.g. “from March”, “for freelance projects”)", &s.availability_note, "text", r#" maxlength="160""#),
            socials = socials,
            enquiries = row("mail", "amber", "Contact form", &format!("At {}", s.paths.public_path("/portfolio/contact/")), &switch(p::SETTING_ENQUIRIES, s.enquiries)),
            notify = field(p::SETTING_NOTIFY_EMAIL, "Email new enquiries to (default: the contact email)", &s.notify_email, "email", r#" maxlength="191""#),
            budgets = textarea(p::SETTING_BUDGETS, "Budget choices on the form — one per line", &s.budgets.join("\n"), 5, r#" maxlength="800""#),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn portfolio_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("");
    let flag = |k: &str| get(k) == "1";
    let current = portfolio_settings(&state);
    let Some(paths) = p::Paths::new(if form.iter().any(|(n, _)| n == p::SETTING_BASE_PATH) { get(p::SETTING_BASE_PATH) } else { &current.paths.base }) else {
        return flash_to(SETTINGS, "error", "Enter an available address of up to three parts, using letters, numbers and dashes.");
    };
    if paths != current.paths && vuuut_core::admin_address::suffix(&paths.base, &state.admin_slug()).is_some() {
        return flash_to(SETTINGS, "error", "Choose an address outside the admin.");
    }
    let clean = vuuut_core::directory::clean;
    let email = clean(get(p::SETTING_EMAIL), 191).to_lowercase();
    let notify = clean(get(p::SETTING_NOTIFY_EMAIL), 191).to_lowercase();
    for e in [&email, &notify] {
        if !e.is_empty() && !vuuut_core::directory::is_email(e) {
            return flash_to(SETTINGS, "error", "An email address does not look right.");
        }
    }
    let portrait = form.iter().find(|(k, _)| k == "image").map(|(_, v)| v.as_str()).unwrap_or("");
    let Some(portrait) = vuuut_core::directory::safe_image(portrait) else { return flash_to(SETTINGS, "error", "The portrait must be an upload or an https:// address.") };
    let resume = clean(get(p::SETTING_RESUME_URL), 500);
    let resume = if resume.is_empty() { resume } else {
        match vuuut_core::directory::safe_url(&resume).filter(|u| !u.is_empty()).or_else(|| (resume.starts_with("/content/") && !resume.contains("..") && !resume.contains(['"', '<', '>', ' '])).then(|| resume.clone())) {
            Some(u) => u,
            None => return flash_to(SETTINGS, "error", "The résumé address must start with https:// or /content/."),
        }
    };
    let socials = match p::socials_from_form(&form) {
        Ok(v) => v,
        Err(m) => return flash_to(SETTINGS, "error", &m),
    };
    let founded = get(p::SETTING_FOUNDED).trim().parse::<i64>().ok().filter(|y| (1800..=2200).contains(y)).map(|y| y.to_string()).unwrap_or_default();
    let currency = get(p::SETTING_CURRENCY).trim().to_uppercase();
    let budgets: Vec<String> = get(p::SETTING_BUDGETS).lines().map(|l| clean(l, 60)).filter(|l| !l.is_empty()).take(12).collect();
    let writes = [
        (p::SETTING_ENABLED, serde_json::json!(flag(p::SETTING_ENABLED))),
        (p::SETTING_MODE, serde_json::json!(p::Mode::from_key(get(p::SETTING_MODE)).key())),
        (p::SETTING_TITLE, serde_json::json!(clean(get(p::SETTING_TITLE), 80))),
        (p::SETTING_INTRO, serde_json::json!(vuuut_core::directory::clean_text(get(p::SETTING_INTRO), 600))),
        (p::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (p::SETTING_NAME, serde_json::json!(clean(get(p::SETTING_NAME), 120))),
        (p::SETTING_HEADLINE, serde_json::json!(clean(get(p::SETTING_HEADLINE), 200))),
        (p::SETTING_ABOUT, serde_json::json!(vuuut_core::directory::clean_text(get(p::SETTING_ABOUT), 6000))),
        (p::SETTING_PORTRAIT, serde_json::json!(portrait)),
        (p::SETTING_LOCATION, serde_json::json!(clean(get(p::SETTING_LOCATION), 120))),
        (p::SETTING_FOUNDED, serde_json::json!(founded)),
        (p::SETTING_CURRENCY, serde_json::json!(if vuuut_core::directory::CURRENCIES.contains(&currency.as_str()) { currency } else { "USD".into() })),
        (p::SETTING_EMAIL, serde_json::json!(email)),
        (p::SETTING_PHONE, serde_json::json!(vuuut_core::directory::clean_phone(get(p::SETTING_PHONE)).unwrap_or_default())),
        (p::SETTING_RESUME_URL, serde_json::json!(resume)),
        (p::SETTING_AVAILABILITY, serde_json::json!(p::Availability::from_key(get(p::SETTING_AVAILABILITY)).key())),
        (p::SETTING_AVAILABILITY_NOTE, serde_json::json!(clean(get(p::SETTING_AVAILABILITY_NOTE), 160))),
        (p::SETTING_SOCIALS, serde_json::json!(if socials.is_empty() { String::new() } else { p::socials_json(&socials) })),
        (p::SETTING_ENQUIRIES, serde_json::json!(flag(p::SETTING_ENQUIRIES))),
        (p::SETTING_NOTIFY_EMAIL, serde_json::json!(notify)),
        (p::SETTING_BUDGETS, serde_json::json!(budgets.join("\n"))),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("portfolio: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    vuuut_db::content::touch();
    crate::tools::tool_saved(&state, SETTINGS, "Portfolio settings saved.")
}
