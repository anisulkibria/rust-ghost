//! Settings -> Analytics, Features and Site: the switches, the lab flags and the announcement bar.

use super::*;

// --- social accounts ---

/// The Social accounts section, for the same form as everything else on
/// General settings.
pub(crate) fn social_sections(state: &AppState) -> String {
    let rows: String = SOCIAL_ACCOUNTS
        .iter()
        .map(|(key, label)| {
            let value = state.settings.get_str(key).unwrap_or_default();
            format!(
                r#"<div class="row" style="align-items:center"><span class="ic tint-blue">{link}</span>
<div class="body"><label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" "><span>{label}</span></label></div></div>"#,
                link = svg("link"),
                key = esc(key),
                value = esc(&value),
                label = esc(label),
            )
        })
        .collect();
    format!(
        r#"<div class="section">Social accounts</div>
<div class="card rows rise" style="--i:7">{rows}</div>
<p class="muted" style="margin:4px 0 0 16px">Full profile URLs, e.g. <code>https://x.com/ghost</code>. Themes expose these as <code>@site.facebook</code> and friends.</p>"#,
        rows = rows,
    )
}

// --- analytics: web analytics, outbound link tagging (Ghost's `analytics`
// settings group is exactly these two keys) ---

pub(crate) async fn analytics_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let toggle = |key: &str, title: &str, detail: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-green">{chart}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="{key}" value="1" {ck}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            chart = svg("chart"),
            title = esc(title),
            detail = esc(detail),
            key = esc(key),
            ck = if state.settings.get_bool(key) { "checked" } else { "" },
        )
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "analytics").unwrap();
    page_for_role(actor_role,
        "Analytics",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/analytics/" id="analytics-form">
<div class="section">Analytics</div>
<p><a class="btn tonal" href="/ghost/analytics/click/">Vuuut Click connection &amp; measurement settings</a></p>
<div class="card rows rise" style="--i:1">
{web}{opens}{clicks}{sources}{outbound}
</div>
<p class="muted" style="margin:10px 2px 0">Email tracking needs clicks and opens to be recorded before a post's
analytics can attribute them. Turning opens off also stops click tracking.</p>
</form>"#,
            head = group_head(group, Some("analytics-form")),
            banner = flash_banner(&q),
            // Same switches, wording and order as Ghost's Analytics panel.
            web = toggle("web_analytics", "Web analytics", "Cookie-free, first party traffic analytics for your site"),
            opens = toggle("email_track_opens", "Email opens", "Record when a member opens an email"),
            clicks = toggle("email_track_clicks", "Email clicks", "Record when a member clicks on any link in an email"),
            sources = toggle("members_track_sources", "Member sources", "Track the traffic sources and posts that drive the most member growth"),
            outbound = toggle("outbound_link_tagging", "Outbound link tagging", "Make it easier for other sites to track the traffic you send them in their analytics"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn analytics_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = |key: &str| form.get(key).map(|v| v == "1").unwrap_or(false);
    // Ghost disables the clicks switch while opens are off, because a click is
    // only attributable to a member whose open was recorded.
    let track_opens = on("email_track_opens");
    let mut ok = true;
    for (key, value) in [
        ("web_analytics", on("web_analytics")),
        ("email_track_opens", track_opens),
        ("email_track_clicks", track_opens && on("email_track_clicks")),
        ("members_track_sources", on("members_track_sources")),
        ("outbound_link_tagging", on("outbound_link_tagging")),
    ] {
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(value)).await.is_ok();
    }
    if ok {
        settings_flash("analytics", "ok", "Analytics settings saved.")
    } else {
        settings_flash("analytics", "error", "Saving failed — check the logs.")
    }
}

// --- features: the system switches for Comments and Automations ---

pub(crate) async fn features_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "features").unwrap();
    let moderation = state.settings.get_str("comments_moderation").unwrap_or_else(|| "immediate".into());
    // get_str + "false" rather than get_bool: a missing row must read as on,
    // and get_bool calls missing false
    let automations_on = state.settings.get_str("automations_enabled").as_deref() != Some("false");

    // The feature flags live here rather than under Advanced: they are
    // features, and Advanced is where a site's plumbing lives — import,
    // redirects, routes. One page, one Save, so the flags post as part of the
    // same form as the switches above them.
    let flags = labs_flags(&state);
    let beta_rows = labs_flag_rows(&flags, BETA_FLAGS, "features-form");

    // Said plainly, once: a switch here sets the same flag Ghost would set,
    // and your theme can read it — but none of this port's own features are
    // gated on one yet, so turning a flag on will not by itself change how
    // the admin or the site behaves. Better stated than implied.
    let honoured_note = "Each switch sets the flag Ghost stores under the same name, and your theme can read any of \
         them as <code>{{#if labs.flagName}}</code>. None of this port's own features are gated on a \
         flag yet, so turning one on changes what your theme sees rather than how the admin behaves.";

    // Ghost keeps these behind `enableDeveloperExperiments`; the equivalent
    // here is running in development, so a production site is not offered
    // switches whose features are unfinished by Ghost's own account.
    let private_section = if state.config.env == "development" {
        format!(
            r#"<div class="section">Private features</div>
<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted" style="margin:10px 2px 0">Ghost hides these unless developer experiments are switched on, and shows them here
because this site is running in development.</p>"#,
            rows = labs_flag_rows(&flags, PRIVATE_FLAGS, "features-form"),
        )
    } else {
        String::new()
    };

    page_for_role(actor_role,
        "Features",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/features/" id="features-form">
<div class="section">Comments</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-purple">{comment}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px">
<select name="comments_moderation">{mod_opts}</select><span>New comments</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Held comments arrive hidden and wait in the moderation screen's Hidden filter — restoring one publishes it.</p></div></div>
</div>
<div class="section">Automations</div>
<div class="card rows rise" style="--i:2">
{automations_toggle}
</div>
</form>
<div class="section">Beta features</div>
<div class="card rows rise" style="--i:3">{beta_rows}</div>
<p class="muted" style="margin:10px 2px 0">A testing ground for new or experimental features. They may change, break or
inexplicably disappear at any time. {honoured}</p>
{private_section}"#,
            head = group_head(group, Some("features-form")),
            banner = flash_banner(&q),
            comment = svg("comment"),
            car = SELECT_CARET,
            mod_opts = crate::select_options(
                &[("immediate", "Publish immediately"), ("review", "Hold for review")],
                &moderation,
            ),
            automations_toggle = toggle_row(
                "automations_enabled",
                "Automations",
                "Runs the email sequences that start when members join. Stopping the system pauses every automation — nothing is enrolled, nothing is sent, and runs in flight wait exactly where they are.",
                automations_on,
            ),
            beta_rows = beta_rows,
            honoured = honoured_note,
            private_section = private_section,
        ),
        true,
    )
    .into_response()
}

/// `shown` repeats once per feature flag on the page, so the body is read as
/// pairs; the map is only for the fields that appear once.
pub(crate) async fn features_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let form: HashMap<String, String> = pairs.iter().cloned().collect();
    let moderation = form.get("comments_moderation").map(String::as_str).unwrap_or("immediate");
    if !matches!(moderation, "immediate" | "review") {
        return settings_flash("features", "error", "New comments must publish immediately or be held for review.");
    }
    let automations_on = form.get("automations_enabled").map(|v| v == "1").unwrap_or(false);
    let saved_moderation = state
        .settings
        .set(&state.db, "comments_moderation", &serde_json::json!(moderation))
        .await;
    let saved_automations = state
        .settings
        .set(&state.db, "automations_enabled", &serde_json::Value::Bool(automations_on))
        .await;

    // Only the flags this page actually rendered are touched: on a production
    // site the private ones are never shown, and reading their absence as
    // "off" would clear flags the publisher never saw.
    let shown: Vec<&str> =
        pairs.iter().filter(|(k, _)| k == "shown").map(|(_, v)| v.as_str()).collect();
    let mut flags = labs_flags(&state);
    for key in shown {
        // A flag that is off is removed rather than stored as `false`, which
        // is the shape Ghost's own object takes.
        if pairs.iter().any(|(k, v)| k == key && v == "1") {
            flags.insert(key.to_string(), serde_json::Value::Bool(true));
        } else {
            flags.remove(key);
        }
    }
    let saved_flags =
        state.settings.set(&state.db, "labs", &serde_json::Value::Object(flags)).await;

    // Turning the system back on should not wait for the next tick to notice
    if automations_on {
        ghost_api::automations_engine::request_poll();
    }
    if saved_moderation.is_ok() && saved_automations.is_ok() && saved_flags.is_ok() {
        settings_flash("features", "ok", "Feature settings saved.")
    } else {
        settings_flash("features", "error", "Saving failed — check the logs.")
    }
}

// --- site: theme (the existing design page) + announcement bar ---

/// Ghost's announcement visibility audiences (AnnouncementVisibilityValues).
pub(crate) const ANNOUNCEMENT_VISIBILITIES: &[(&str, &str)] = &[
    ("visitors", "Visitors"),
    ("free_members", "Free members"),
    ("paid_members", "Paid members"),
];

/// Read `announcement_visibility` (a JSON array) into the checked set.
pub(crate) fn announcement_visibility_set(v: Option<serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(a)) => a
            .into_iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str::<Vec<String>>(&s).unwrap_or_default()
        }
        _ => vec![],
    }
}

pub(crate) async fn site_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let checked: Vec<String> = announcement_visibility_set(state.settings.get("announcement_visibility"));
    let vis_chips: String = ANNOUNCEMENT_VISIBILITIES
        .iter()
        .map(|(value, label)| {
            let ck = if checked.iter().any(|c| c == value) { "checked" } else { "" };
            format!(
                r#"<label class="fchip"><input type="checkbox" name="visibility" value="{value}" {ck}><span class="ck">{check}</span>{label}</label>"#,
                value = esc(value),
                ck = ck,
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "site").unwrap();
    page_for_role(actor_role,
        "Announcement",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/site/" id="site-form">
<div class="section">Announcement bar</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{megaphone}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="announcement_content" rows="3" placeholder=" ">{content}</textarea><span>Announcement · Markdown supported</span></label>
<p class="muted" style="margin:8px 0 0">Leave empty to hide the bar. Shown at the top of every page.</p></div></div>
<div class="row"><span class="ic tint-amber">{palette}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px"><select name="announcement_background">{bg_opts}</select><span>Background</span>{car}</label></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{visibility}</span>
<div class="body"><span class="title" style="display:block;margin-bottom:8px">Visibility</span>
<div class="chips">{vis_chips}</div>
<p class="muted" style="margin:8px 0 0">At least one audience must be selected for the bar to appear. Member audiences take effect once members are available.</p></div></div>
</div>
</form>"#,
            head = group_head(group, Some("site-form")),
            banner = flash_banner(&q),
            megaphone = svg("flag"),
            palette = svg("themes"),
            visibility = svg("globe"),
            car = SELECT_CARET,
            content = s("announcement_content"),
            bg_opts = select_options(&[("dark", "Dark"), ("light", "Light"), ("accent", "Accent color")], &state.settings.get_str("announcement_background").unwrap_or_else(|| "dark".into())),
            vis_chips = vis_chips,
        ),
        true,
    )
    .into_response()
}

/// The announcement-bar form: the visibility checkboxes all post under
/// `visibility`, so this needs the same hand-rolled parsing as the editor's
/// tag checkboxes.
#[derive(Default)]
pub(crate) struct SiteForm {
    announcement_content: String,
    announcement_background: String,
    visibility: Vec<String>,
}

impl SiteForm {
    pub(crate) fn parse(body: &str) -> Self {
        let mut form = SiteForm::default();
        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match key.as_ref() {
                "announcement_content" => form.announcement_content = value.into_owned(),
                "announcement_background" => form.announcement_background = value.into_owned(),
                "visibility" => form.visibility.push(value.into_owned()),
                _ => {}
            }
        }
        form
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for SiteForm {
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body = String::from_utf8_lossy(&bytes);
        Ok(SiteForm::parse(&body))
    }
}

pub(crate) async fn site_save(State(state): State<AppState>, headers: HeaderMap, form: SiteForm) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // only Ghost's backgrounds and audiences are addressable; anything else
    // falls back to the stored value
    let background = if ["dark", "light", "accent"].contains(&form.announcement_background.as_str()) {
        form.announcement_background
    } else {
        state.settings.get_str("announcement_background").unwrap_or_else(|| "dark".into())
    };
    let visibility: Vec<String> = form
        .visibility
        .into_iter()
        .filter(|v| ANNOUNCEMENT_VISIBILITIES.iter().any(|(val, _)| val == v))
        .collect();
    let ok = state
        .settings
        .set(&state.db, "announcement_content", &serde_json::json!(form.announcement_content))
        .await
        .is_ok()
        && state.settings.set(&state.db, "announcement_background", &serde_json::json!(background)).await.is_ok()
        && state
            .settings
            .set(&state.db, "announcement_visibility", &serde_json::json!(visibility))
            .await
            .is_ok();
    if ok {
        settings_saved(&state, "site", "Site settings saved.")
    } else {
        settings_flash("site", "error", "Saving failed — check the logs.")
    }
}
