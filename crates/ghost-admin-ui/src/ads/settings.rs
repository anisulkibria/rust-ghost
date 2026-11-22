//! Advertising settings: the master switch, the disclosure label, paying
//! members, how many ads one post may carry, and ads.txt.
//!
//! Saved through `SiteSettings::read`, the same reader the renderer uses, so
//! what is stored is exactly what runs.

use super::*;

const SETTINGS: &str = "/ghost/ads/settings/";

fn switch(name: &str, on: bool) -> String {
    format!(
        r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label>"#,
        on = if on { " checked" } else { "" },
    )
}

fn row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing ad-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

pub(crate) async fn ads_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = ghost_api::ads::settings(&state);
    let problems = rules::ads_txt_problems(&s.ads_txt);
    let warn = if problems.is_empty() {
        String::new()
    } else {
        let lines: String = problems
            .iter()
            .take(8)
            .map(|(n, l)| format!("<li>Line {n}: <code>{}</code></li>", esc(l)))
            .collect();
        format!(
            r#"<div class="ad-warn"><b>{n} doesn't look like a seller record</b> (domain, account id, DIRECT or RESELLER). Saved anyway; check with your network.<ul>{lines}</ul></div>"#,
            n = countable(problems.len() as i64, "line"),
        )
    };
    let max: Vec<(String, String)> = (1..=6).map(|n| (n.to_string(), countable(n, "ad"))).collect();
    let max_opts: String = max
        .iter()
        .map(|(v, l)| format!(r#"<option value="{v}"{s}>{l}</option>"#, s = if *v == s.max_in_content.to_string() { " selected" } else { "" }, l = esc(l)))
        .collect();
    let site = state.site_url();
    page_for_role(
        actor.role,
        "Ad settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="ads-settings">
<div class="section">On or off</div>
<div class="card rows rise" style="--i:2">
{enabled}
{paid}
{max_row}
</div>
<div class="section">Disclosure</div>
<div class="card padded rise" style="--i:3">
<label class="field" style="margin-bottom:0"><input name="{label_key}" value="{label}" maxlength="{label_max}" placeholder="{label_default}"><span>Label shown on every ad</span></label>
<p class="muted ad-hint">Readers have to be able to tell an ad from your writing — “Advertisement”, “Sponsored”, “Partner content”. Blank uses “{label_default}”. Every ad's link is also marked <code>rel="sponsored"</code> for search engines.</p>
</div>
<div class="section">ads.txt</div>
<div class="card padded rise" style="--i:4">
<label class="field" style="margin-bottom:0"><textarea name="{txt_key}" rows="8" class="mono" spellcheck="false" placeholder=" ">{txt}</textarea><span>Authorised sellers</span></label>
<p class="muted ad-hint">Ad networks ask you to publish this so buyers can check who may sell your space. Paste the lines your networks give you, one per line. Served at <a href="{site}/ads.txt" target="_blank" rel="noopener">{site}/ads.txt</a> while ads are on.</p>
{warn}
</div>
</form>
<div class="section">Placing ads in a theme</div>
<div class="card padded rise" style="--i:5">
<p style="margin:0 0 8px">Ads inside posts and the sticky bar need nothing from your theme. For anywhere else — a sidebar, under the header, between posts on the home page — add an empty slot and choose “Wherever a slot is placed” on the ad:</p>
<pre class="ad-code">&lt;div data-ad-slot="sidebar"&gt;&lt;/div&gt;</pre>
<p class="muted ad-hint">It works in a theme template, in Code injection, or in an HTML card inside a post. An empty slot stays empty when no ad is eligible.</p>
</div>"#,
            head = head(&format!(r#"<button class="btn" type="submit" form="ads-settings">{} Save</button>"#, svg("check"))),
            tabs = tabs("settings"),
            banner = flash_banner(&q),
            enabled = row("campaign", if s.enabled { "green" } else { "slate" }, "Show ads on the site",
                if s.enabled { "On: live ads appear where they are placed." } else { "Off: no ad, no script and no ads.txt anywhere." },
                &switch(rules::SETTING_ENABLED, s.enabled)),
            paid = row("heart", "purple", "No ads for paying members",
                "Paid and complimentary members see the site without ads — a common perk of paying.",
                &switch(rules::SETTING_HIDE_PAID, s.hide_paid)),
            max_row = row("posts", "blue", "Ads inside one post",
                "At most this many in a post's body, counting top, middle and end.",
                &format!(r#"<label class="field fixed" style="margin:0"><select name="{k}">{max_opts}</select><span>At most</span>{caret}</label>"#,
                    k = rules::SETTING_MAX_IN_CONTENT, caret = SELECT_CARET)),
            label_key = rules::SETTING_LABEL,
            label = esc(&state.settings.get_str(rules::SETTING_LABEL).unwrap_or_default()),
            label_max = rules::MAX_LABEL,
            label_default = esc(rules::DEFAULT_LABEL),
            txt_key = rules::SETTING_ADS_TXT,
            txt = esc(&s.ads_txt),
            site = esc(site.trim_end_matches('/')),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn ads_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let s = rules::SiteSettings::read(|key| match key {
        rules::SETTING_ENABLED | rules::SETTING_HIDE_PAID => Some(flag(key).to_string()),
        _ => form.get(key).cloned(),
    });
    let label = rules::clean(form.get(rules::SETTING_LABEL).map(String::as_str).unwrap_or(""), rules::MAX_LABEL);
    let writes = [
        (rules::SETTING_ENABLED, serde_json::json!(s.enabled)),
        (rules::SETTING_HIDE_PAID, serde_json::json!(s.hide_paid)),
        (rules::SETTING_MAX_IN_CONTENT, serde_json::json!(s.max_in_content)),
        (rules::SETTING_LABEL, serde_json::json!(label)),
        (rules::SETTING_ADS_TXT, serde_json::json!(s.ads_txt)),
    ];
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("ads: saving {key} failed: {}", e.message());
            return flash_to(SETTINGS, "error", "Saving failed — check the logs.");
        }
    }
    crate::tools::tool_saved(&state, SETTINGS, "Ad settings saved.")
}

/// The Overview's "Switch on" button.
pub(crate) async fn ads_settings_switch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let on = form.get("on").is_some_and(|v| v == "1");
    match state.settings.set(&state.db, rules::SETTING_ENABLED, &serde_json::json!(on)).await {
        Ok(_) => flash_to(PATH, "ok", if on { "Ads are on." } else { "Ads are off." }),
        Err(e) => {
            tracing::error!("ads: switching failed: {}", e.message());
            flash_to(PATH, "error", "That could not be saved.")
        }
    }
}
