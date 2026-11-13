//! The built-in apps: Zapier, Slack, Unsplash, FirstPromoter, Pintura and Transistor.

use super::*;

// ---------------------------------------------------------------------------
// built-in app: Zapier
// ---------------------------------------------------------------------------

pub(crate) async fn zapier_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let key = repos::integrations::admin_key_for_slug(&state.db, "zapier")
        .await
        .ok()
        .flatten();
    let key_block = match &key {
        Some(k) => format!(
            r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-amber">{ic}</span>
<div class="body"><span class="title">Admin API key</span><div style="margin-top:6px">{chip}</div></div></div>
<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-blue">{globe}</span>
<div class="body"><span class="title">API URL</span><div style="margin-top:6px">{url}</div></div></div>"#,
            ic = svg("lock"),
            chip = key_chip("admin", k),
            globe = svg("globe"),
            url = key_chip("url", &format!("{}/ghost/api", state.site_url())),
        ),
        None => r#"<div class="row"><div class="body"><span class="detail">The Zapier key is missing from this database. Restart the server to have it recreated.</span></div></div>"#.to_string(),
    };

    page_for_role(
        actor_role,
        "Zapier",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Zapier</h1><p class="sub">Automate Ghost with 6,000+ apps</p></div></header>
{banner}
<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Paste these into Zapier when it asks to connect to Ghost. The Admin API key lets a Zap create and edit content, so treat it like a password.</span></div>
<div class="section">Credentials</div>
<div class="card rows rise" style="--i:2">{key_block}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            bulb = svg("integrations"),
            key_block = key_block,
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// built-in app: Slack
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SlackForm {
    #[serde(default)]
    slack_url: String,
    #[serde(default)]
    slack_username: String,
}

pub(crate) async fn slack_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let url = state.settings.get_str("slack_url").unwrap_or_default();
    let username = state
        .settings
        .get_str("slack_username")
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "Ghost".into());
    let configured = !url.trim().is_empty();
    let test = match configured {
        true => r#"<form method="post" action="/ghost/settings/integrations/slack/test/" style="margin-top:14px"><button class="btn tonal" type="submit">Send test notification</button></form>"#,
        false => "",
    };

    page_for_role(
        actor_role,
        "Slack",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Slack</h1><p class="sub">Post to a channel when you publish</p></div></header>
{banner}
<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Create an <b>Incoming Webhook</b> in Slack (Slack → Apps → Incoming WebHooks), pick a channel, and paste the URL it gives you. The URL is a credential — anyone with it can post to that channel.</span></div>
<div class="card padded rise" style="--i:2"><form method="post" action="/ghost/settings/integrations/slack/">
{url_field}{name_field}
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form>
{test}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            bulb = svg("send"),
            url_field = text_field("slack_url", "Webhook URL", &url),
            name_field = text_field("slack_username", "Username shown in Slack", &username),
            test = test,
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn slack_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<SlackForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if let Err(e) = ghost_api::slack::validate_url(&form.slack_url) {
        return integrations_flash("slack/", "error", &e.message());
    }
    let url = form.slack_url.trim().to_string();
    let username = match form.slack_username.trim() {
        "" => "Ghost".to_string(),
        u => u.to_string(),
    };
    for (key, value) in [("slack_url", url), ("slack_username", username)] {
        if let Err(e) = state.settings.set(&state.db, key, &serde_json::json!(value)).await {
            return integrations_flash("slack/", "error", &e.message());
        }
    }
    integrations_flash("slack/", "ok", "Slack settings saved.")
}

pub(crate) async fn slack_test(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(cfg) = ghost_api::slack::SlackConfig::from_settings(&state.settings) else {
        return integrations_flash("slack/", "error", "Add a webhook URL first.");
    };
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    match ghost_api::slack::send_test(&cfg, &title, &state.site_url()).await {
        Ok(()) => integrations_flash("slack/", "ok", "Test notification sent to Slack."),
        Err(e) => integrations_flash("slack/", "error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// built-in app: Unsplash
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct UnsplashForm {
    #[serde(default)]
    unsplash: Option<String>,
    #[serde(default)]
    unsplash_access_key: String,
}

/// FirstPromoter: referral tracking. Ghost injects their script with the
/// account id, so the id is what makes the toggle do anything.
pub(crate) async fn firstpromoter_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("firstpromoter");
    let id = state.settings.get_str("firstpromoter_id").unwrap_or_default();
    page_for_role(
        actor_role,
        "FirstPromoter",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>FirstPromoter</h1><p class="sub">Referral and affiliate tracking for members</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/firstpromoter/">
{toggle}
<div style="margin-top:14px">{id_field}</div>
<p class="muted" style="margin:10px 0 0">{note}</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row(
                "firstpromoter",
                "Enable FirstPromoter",
                "Adds their tracking script to your public pages",
                on
            ),
            id_field = text_field("firstpromoter_id", "FirstPromoter account ID", &id),
            note = if id.trim().is_empty() {
                "The account ID comes from your FirstPromoter dashboard. Without it the script has nothing to report to, so the toggle does nothing."
            } else {
                "Your pages carry the FirstPromoter script under this account ID."
            },
        ),
        true,
    )
    .into_response()
}

/// Pintura: an image editor in the admin. Ghost loads it from URLs you host,
/// since the library is commercial and not bundled.
pub(crate) async fn pintura_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("pintura");
    let js = state.settings.get_str("pintura_js_url").unwrap_or_default();
    let css = state.settings.get_str("pintura_css_url").unwrap_or_default();
    page_for_role(
        actor_role,
        "Pintura",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Pintura</h1><p class="sub">Edit images without leaving the editor</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/pintura/">
{toggle}
<div class="grid2" style="gap:12px;margin-top:14px">{js_field}{css_field}</div>
<p class="muted" style="margin:10px 0 0">Pintura is a commercial library Ghost does not ship. Buy a licence, host the two files
yourself, and point these at them.</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row("pintura", "Enable Pintura", "Adds an edit button to image cards", on),
            js_field = text_field("pintura_js_url", "Script URL (pintura.umd.js)", &js),
            css_field = text_field("pintura_css_url", "Stylesheet URL (pintura.css)", &css),
        ),
        true,
    )
    .into_response()
}

/// Transistor.fm: private podcast feeds for members. The URL template carries
/// `{uuid}`, which Ghost swaps for the member's own so each gets a feed only
/// they can use.
pub(crate) async fn transistor_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("transistor");
    let g = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    page_for_role(
        actor_role,
        "Transistor.fm",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Transistor.fm</h1><p class="sub">Private podcast feeds for your members</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/transistor/">
{toggle}
<label class="field" style="margin:14px 0 0"><input name="transistor_portal_url_template" value="{url_template}" placeholder="https://partner.transistor.fm/ghost/{{uuid}}"><span>Private feed URL template</span></label>
<p class="muted" style="margin:6px 0 0">Use <code>{{uuid}}</code> where the member's own identifier goes — that is what makes each feed private to them.</p>
<div class="grid2" style="gap:12px;margin-top:14px">
<label class="field" style="margin:0"><input name="transistor_portal_heading" value="{heading}" placeholder=" "><span>Heading in the account page</span></label>
<label class="field" style="margin:0"><input name="transistor_portal_button_text" value="{button}" placeholder=" "><span>Button text</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="transistor_portal_description" rows="2" placeholder=" ">{description}</textarea><span>Description</span></label>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row(
                "transistor",
                "Enable Transistor integration",
                "Offer members their private podcast feed from the account page",
                on
            ),
            url_template = g("transistor_portal_url_template"),
            heading = g("transistor_portal_heading"),
            button = g("transistor_portal_button_text"),
            description = g("transistor_portal_description"),
        ),
        true,
    )
    .into_response()
}

/// One save for all three: a toggle plus its text settings.
///
/// The app is named by the caller rather than taken from the path — these are
/// three fixed routes, so there is no segment to extract.
pub(crate) async fn builtin_app_save(
    state: &AppState,
    headers: &HeaderMap,
    app: &str,
    form: &HashMap<String, String>,
) -> Response {
    if let Err(r) = require_role(state, headers, Role::Administrator).await {
        return r;
    }
    let fields: &[&str] = match app {
        "firstpromoter" => &["firstpromoter_id"],
        "pintura" => &["pintura_js_url", "pintura_css_url"],
        "transistor" => &[
            "transistor_portal_url_template",
            "transistor_portal_heading",
            "transistor_portal_button_text",
            "transistor_portal_description",
        ],
        _ => return integrations_flash("", "error", "Unknown app."),
    };
    if form.get(app).is_some_and(|value| value == "1") {
        let nonempty = |key: &str| form.get(key).is_some_and(|value| !value.trim().is_empty());
        let valid_url = |key: &str| form.get(key).and_then(|value| reqwest::Url::parse(value.trim()).ok())
            .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
        let message = match app {
            "firstpromoter" if !nonempty("firstpromoter_id") => Some("Enter your FirstPromoter account ID before enabling the integration."),
            "pintura" if !valid_url("pintura_js_url") || !valid_url("pintura_css_url") => Some("Enter valid HTTP or HTTPS URLs for both Pintura assets before enabling the integration."),
            "transistor" if !valid_url("transistor_portal_url_template") || !form.get("transistor_portal_url_template").is_some_and(|value| value.contains("{uuid}")) => Some("Enter a valid private feed URL containing {uuid} before enabling Transistor."),
            _ => None,
        };
        if let Some(message) = message { return integrations_flash(&format!("{app}/"), "error", message); }
    }
    let mut ok = state
        .settings
        .set(&state.db, app, &serde_json::Value::Bool(form.get(app).map(|v| v == "1").unwrap_or(false)))
        .await
        .is_ok();
    for key in fields {
        let value = form.get(*key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    if ok {
        integrations_flash(&format!("{app}/"), "ok", "Saved.")
    } else {
        integrations_flash(&format!("{app}/"), "error", "Saving failed — check the logs.")
    }
}

pub(crate) async fn firstpromoter_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "firstpromoter", &form).await
}

pub(crate) async fn pintura_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "pintura", &form).await
}

pub(crate) async fn transistor_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "transistor", &form).await
}

pub(crate) async fn unsplash_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("unsplash");
    let key = state.settings.get_str("unsplash_access_key").unwrap_or_default();
    let note = match key.trim().is_empty() {
        true => "Without an access key the picker shows a built-in sample gallery. Add a key from unsplash.com/developers to search the whole library.",
        false => "Searching the full Unsplash library with your access key.",
    };

    page_for_role(
        actor_role,
        "Unsplash",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Unsplash</h1><p class="sub">Free photography inside the editor</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/unsplash/">
{toggle}
<div style="margin-top:14px">{key_field}</div>
<p class="muted" style="margin:10px 0 0">{note}</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row("unsplash", "Unsplash in the editor", "Adds an Unsplash button to image cards", on),
            key_field = text_field("unsplash_access_key", "Unsplash access key (optional)", &key),
            note = esc(note),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn unsplash_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<UnsplashForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.unsplash.as_deref() == Some("1");
    if let Err(e) = state.settings.set(&state.db, "unsplash", &serde_json::json!(on)).await {
        return integrations_flash("unsplash/", "error", &e.message());
    }
    let key = form.unsplash_access_key.trim().to_string();
    if let Err(e) = state
        .settings
        .set(&state.db, "unsplash_access_key", &serde_json::json!(key))
        .await
    {
        return integrations_flash("unsplash/", "error", &e.message());
    }
    integrations_flash("unsplash/", "ok", "Unsplash settings saved.")
}
