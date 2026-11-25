//! Directory settings: whether the directory is public, the submission form
//! and its moderation, how long classifieds run, and how listings are shown.
//! Administrator and above. Saved through `Settings::read`, the same reader
//! the public directory uses.

use super::*;
use vuuut_core::directory as d;

const SETTINGS: &str = "/ghost/directories/settings/";

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

fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if v == current { " selected" } else { "" }, l = esc(l))).collect();
    format!(r#"<label class="field fixed" style="margin:0"><select name="{name}">{o}</select><span>{label}</span>{caret}</label>"#, label = esc(label), caret = SELECT_CARET)
}

fn address_field(name: &str, label: &str, site: &str, value: &str, placeholder: &str, suffix: &str) -> String {
    format!(
        r#"<label class="ec-urlfield"><span class="ec-urlfield-label">{label}</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{name}" value="{value}" placeholder="{placeholder}" maxlength="80" spellcheck="false" autocomplete="off"><span class="ec-urlfield-site">{suffix}</span></span></label>"#,
        site = esc(site),
        value = esc(value),
        placeholder = esc(placeholder),
    )
}

pub(crate) async fn dir_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = dir_settings(&state);
    let site = state.site_url();
    let home = s.paths.public_path("/directory/");
    let submit = s.paths.public_path("/directory/submit/");
    let days: Vec<(String, String)> = d::DAYS.iter().map(|n| (n.to_string(), format!("{n} days"))).collect();
    page_for_role(
        actor.role,
        "Directory settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="dr-settings">
<div class="section">The public directory</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{t_key}" value="{title}" maxlength="60" placeholder="{t_default}"><span>Title</span></label>
<label class="field" style="margin:10px 0 0"><textarea name="{i_key}" rows="2" maxlength="400" placeholder=" ">{intro}</textarea><span>Introduction, above the listings</span></label></div></div>
{per}
{contact}
{currency}
</div>
<div class="section">Directory addresses</div>
<div class="card padded rise" style="--i:3">
{address}
{listings}
<p class="muted" style="margin:8px 0 0">Leave the listing path empty to put listings directly inside the directory. Enter {listing_example} to group them under a separate path.</p>
</div>
<div class="section">Listings from visitors</div>
<div class="card rows rise" style="--i:3">
{subs}
{members}
{moderate}
{days}
</div>
<p class="muted rise" style="--i:4;margin:12px 4px 0">The form has a hidden decoy field, a minimum fill time and a per-visitor limit, and allows five listings per email address a day.</p>
</form>"#,
            head = crate::tools::tool_head_back("directories", BASE, Some("dr-settings"), ""),
            tabs = tabs("settings", true),
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Show the directory on the site",
                &format!("Public address {home}"), &switch(d::SETTING_ENABLED, s.enabled)),
            address = address_field(d::SETTING_BASE_PATH, "Directory homepage", &site, &s.paths.base, d::DEFAULT_BASE, "/"),
            listings = address_field(d::SETTING_LISTING_PATH, "Listing path", &site, &s.paths.listing, &s.paths.base, "/listing-name/"),
            listing_example = format!("<code>{}/listings</code>", esc(&s.paths.base)),
            t_key = d::SETTING_TITLE,
            title = esc(&state.settings.get_str(d::SETTING_TITLE).unwrap_or_default()),
            t_default = esc(d::DEFAULT_TITLE),
            i_key = d::SETTING_INTRO,
            intro = esc(&s.intro),
            per = row("layout", "blue", "Listings per page", "How many the directory shows at once.", &format!(
                r#"<label class="field" style="margin:0"><input name="{key}" type="number" min="1" step="1" value="{size}" placeholder="{default}" aria-label="Listings per page"><span>Show</span></label>"#,
                key = d::SETTING_PER_PAGE,
                size = s.per_page,
                default = d::DEFAULT_PER_PAGE,
            )),
            contact = row("mail", "teal", "Show email and phone", "Off shows only each listing's website.", &switch(d::SETTING_SHOW_CONTACT, s.show_contact)),
            currency = row("tags", "amber", "Currency", "Set in Payments.", &format!(r#"<a href="/ghost/settings/payments/#currency">{}</a>"#, esc(&s.currency))),
            subs = row("add", "pink", "Let visitors add listings", &format!("Submission form at {submit}"), &switch(d::SETTING_SUBMISSIONS, s.submissions)),
            members = row("person", "purple", "Members only", "Only signed-in members can use the form.", &switch(d::SETTING_MEMBERS_ONLY, s.members_only)),
            moderate = row("check", "green", "Review before publishing", "New listings wait in Overview until an editor approves them.", &switch(d::SETTING_MODERATE, s.moderate)),
            days = row("history", "amber", "Classifieds run for", "After this, a classified disappears from the site until it is renewed.", &select(d::SETTING_CLASSIFIED_DAYS, "Days", &days, &s.classified_days.to_string())),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn dir_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = dir_settings(&state);
    let per_page = match form.get(d::SETTING_PER_PAGE).map(|value| value.trim()) {
        None => current.per_page,
        Some("") => d::DEFAULT_PER_PAGE,
        Some(value) => match value.parse::<i64>() {
            Ok(count) if count > 0 => count,
            _ => return flash_to(SETTINGS, "error", "Enter a positive whole number for listings per page."),
        },
    };
    let base = form.get(d::SETTING_BASE_PATH).unwrap_or(&current.paths.base);
    let listing = form.get(d::SETTING_LISTING_PATH).unwrap_or(&current.paths.listing);
    let Some(paths) = d::Paths::new(base, listing) else {
        return flash_to(SETTINGS, "error", "Enter an available directory and listing path with up to three parts, using letters, numbers and dashes.");
    };
    let store = vuuut_core::ecom::Settings::read(|key| state.settings.get_str(key));
    if paths != current.paths {
        let admin = state.admin_slug();
        if [&paths.base, &paths.listing].iter().any(|path| vuuut_core::admin_address::suffix(path, &admin).is_some()) {
            return flash_to(SETTINGS, "error", "Choose paths outside the admin address.");
        }
        if paths.overlaps_store(&store.base_path, &store.product_path) {
            return flash_to(SETTINGS, "error", "Choose paths outside the store and its product addresses.");
        }
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let s = d::Settings::read(|key| match key {
        d::SETTING_ENABLED | d::SETTING_SUBMISSIONS | d::SETTING_MEMBERS_ONLY | d::SETTING_MODERATE | d::SETTING_SHOW_CONTACT => Some(flag(key).to_string()),
        _ => form.get(key).cloned(),
    });
    let title = d::clean(form.get(d::SETTING_TITLE).map(String::as_str).unwrap_or(""), 60);
    let writes = [
        (d::SETTING_ENABLED, serde_json::json!(s.enabled)),
        (d::SETTING_TITLE, serde_json::json!(title)),
        (d::SETTING_INTRO, serde_json::json!(s.intro)),
        (d::SETTING_SUBMISSIONS, serde_json::json!(s.submissions)),
        (d::SETTING_MEMBERS_ONLY, serde_json::json!(s.members_only)),
        (d::SETTING_MODERATE, serde_json::json!(s.moderate)),
        (d::SETTING_CLASSIFIED_DAYS, serde_json::json!(s.classified_days)),
        (d::SETTING_PER_PAGE, serde_json::json!(per_page)),
        (d::SETTING_SHOW_CONTACT, serde_json::json!(s.show_contact)),
        (d::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (d::SETTING_LISTING_PATH, serde_json::json!(paths.listing)),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("directory: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    crate::tools::tool_saved(&state, SETTINGS, "Directory settings saved.")
}
