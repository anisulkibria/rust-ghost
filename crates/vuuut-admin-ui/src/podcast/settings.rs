//! Podcast settings: whether the pages and feeds are public and where,
//! download counting, the guest/question form and where it is announced,
//! and resetting members' private feed addresses. Administrator and above.

use super::*;
use vuuut_core::podcast as p;

const SETTINGS: &str = "/ghost/podcast/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span><div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div><div class="trailing dr-control">{control}</div></div>"#,
        ic = svg(icon), title = esc(title), detail = esc(detail),
    )
}

pub(crate) async fn podcast_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = podcast_settings(&state);
    page_for_role(
        actor.role,
        "Podcast settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="pc-settings">
<div class="section">Public pages and feeds</div>
<div class="card rows rise" style="--i:2">
{enabled}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{t_key}" value="{title}" maxlength="80" placeholder="{t_def}"><span>Title of the podcasts page (with more than one show)</span></label>
<label class="field" style="margin:10px 0 0"><textarea name="{i_key}" rows="2" maxlength="400" placeholder=" ">{intro}</textarea><span>Introduction</span></label>
<label class="field" style="margin:10px 0 0;max-width:220px"><input name="{pp_key}" type="number" min="4" max="100" value="{per}"><span>Episodes per page</span></label></div></div>
</div>
<div class="section">Address</div>
<div class="card padded rise" style="--i:3"><label class="ec-urlfield"><span class="ec-urlfield-label">Podcast pages</span>
<span class="ec-urlfield-box"><span class="ec-urlfield-site">{site}/</span><input name="{b_key}" value="{base}" placeholder="podcast" maxlength="80" spellcheck="false"><span class="ec-urlfield-site">/show-name/</span></span></label>
<p class="muted ec-hint">Changing this changes every feed address too. Old addresses redirect, but tell Apple and Spotify about the new feed.</p></div>
<div class="section">Listeners</div>
<div class="card rows rise" style="--i:4">
{count}
{requests}
<div class="row"><div class="body"><label class="field" style="margin:0"><input name="{n_key}" value="{notify}" type="email" maxlength="191" placeholder=" "><span>Email new guest pitches and questions to</span></label></div></div>
</div>
</form>
<div class="section">Members' private feeds</div>
<form class="card padded rise" style="--i:5" method="post" action="{SETTINGS}rotate/" data-confirm="Reset every private feed?|Every member's private feed address changes. Members must add the new one to their app.">
<p class="muted" style="margin-top:0">Each signed-in member gets their own feed with every episode their membership opens, on each show's “Members' feed” page. If addresses have been shared around, reset them all.</p>
<button class="btn tonal danger" type="submit">{rot} Reset all private feed addresses</button></form>"#,
            head = crate::tools::tool_head_back("podcast", BASE, Some("pc-settings"), ""),
            tabs = tabs_for(&state, "settings", &actor).await,
            banner = flash_banner(&q),
            enabled = row("globe", if s.enabled { "green" } else { "slate" }, "Publish the podcast pages and RSS feeds", &format!("Public address {}", s.paths.public_path("/podcast/")), &switch(p::SETTING_ENABLED, s.enabled)),
            t_key = p::SETTING_TITLE,
            title = esc(state.settings.get_str(p::SETTING_TITLE).unwrap_or_default().trim_matches('"')),
            t_def = p::DEFAULT_TITLE,
            i_key = p::SETTING_INTRO,
            intro = esc(&s.intro),
            pp_key = p::SETTING_PER_PAGE,
            per = s.per_page,
            site = esc(state.site_url().trim_end_matches('/')),
            b_key = p::SETTING_BASE_PATH,
            base = esc(&s.paths.base),
            count = row("trending", "purple", "Count downloads", "Feeds point at a counting address that hands over the audio. No addresses are stored.", &switch(p::SETTING_COUNT_DOWNLOADS, s.count_downloads)),
            requests = row("mic", "pink", "“Be on the show” form", "Guests can pitch themselves and listeners can send questions from each show's page.", &switch(p::SETTING_REQUESTS, s.requests)),
            n_key = p::SETTING_NOTIFY_EMAIL,
            notify = esc(&s.notify_email),
            rot = svg("repeat"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn podcast_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = podcast_settings(&state);
    let Some(paths) = p::Paths::new(form.get(p::SETTING_BASE_PATH).map(String::as_str).unwrap_or(&current.paths.base)) else {
        return flash_to(SETTINGS, "error", "Enter an available address of up to three parts, using letters, numbers and dashes.");
    };
    if paths != current.paths && vuuut_core::admin_address::suffix(&paths.base, &state.admin_slug()).is_some() {
        return flash_to(SETTINGS, "error", "Choose an address outside the admin.");
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
    let notify = vuuut_core::directory::clean(get(p::SETTING_NOTIFY_EMAIL), 191).to_lowercase();
    if !notify.is_empty() && !vuuut_core::directory::is_email(&notify) {
        return flash_to(SETTINGS, "error", "The notification email does not look right.");
    }
    let per = get(p::SETTING_PER_PAGE).trim().parse::<i64>().ok().map(|n| n.clamp(4, 100)).unwrap_or(12);
    let writes = [
        (p::SETTING_ENABLED, serde_json::json!(flag(p::SETTING_ENABLED))),
        (p::SETTING_TITLE, serde_json::json!(vuuut_core::directory::clean(get(p::SETTING_TITLE), 80))),
        (p::SETTING_INTRO, serde_json::json!(vuuut_core::directory::clean(get(p::SETTING_INTRO), 400))),
        (p::SETTING_BASE_PATH, serde_json::json!(paths.base)),
        (p::SETTING_PER_PAGE, serde_json::json!(per)),
        (p::SETTING_COUNT_DOWNLOADS, serde_json::json!(flag(p::SETTING_COUNT_DOWNLOADS))),
        (p::SETTING_REQUESTS, serde_json::json!(flag(p::SETTING_REQUESTS))),
        (p::SETTING_NOTIFY_EMAIL, serde_json::json!(notify)),
    ];
    if let Err(e) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("podcast: saving settings failed: {}", e.message());
        return flash_to(SETTINGS, "error", "Saving failed. Check the logs.");
    }
    vuuut_db::content::touch();
    crate::tools::tool_saved(&state, SETTINGS, "Podcast settings saved.")
}

pub(crate) async fn rotate_secret(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    match repo::rotate_secret(&state.db).await {
        Ok(()) => flash_to(SETTINGS, "ok", "Every private feed address has changed."),
        Err(_) => flash_to(SETTINGS, "error", "That could not be done."),
    }
}
