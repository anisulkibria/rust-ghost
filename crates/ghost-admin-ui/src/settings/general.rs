//! Settings -> General: title, description, timezone, language and the meta data behind them.

use super::*;

// --- general: title & description, timezone, publication language, staff ---

pub(crate) async fn general_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "general").unwrap();
    page_for_role(actor_role,
        "General settings",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/general/" id="general-form" data-meta-previews data-site-title="{title}" data-site-description="{description}">
<div class="section">Name, title &amp; description</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="name" value="{name}" placeholder=" "><span>Name</span></label>
<p class="muted" style="margin:6px 0 0">Shown in the admin panel and available to themes as <code>{{{{@site.name}}}}</code>. Defaults to the title when blank.</p></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="title" value="{title}" placeholder=" " required><span>Title</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{notes}</span>
<div class="body"><label class="field" style="margin:0"><input name="description" value="{description}" placeholder=" "><span>Description</span></label></div></div>
</div>
<div class="section">Timezone</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:center"><span class="ic tint-pink">{globe}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="timezone">{tz_opts}</select><span>Site timezone</span>{car}</label></div></div>
</div>
<div class="section">Publication language</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:center"><span class="ic tint-pink">{lang}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="locale">{locale_opts}</select><span>Publication language</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Used as the site's <code>&lt;html lang&gt;</code> by themes.</p></div></div>
</div>
{metadata}
{social}
</form>"#,
            head = group_head(group, Some("general-form")),
            banner = flash_banner(&q),
            title_ic = svg("title"),
            notes = svg("notes"),
            globe = svg("globe"),
            lang = svg("notes"),
            car = SELECT_CARET,
            title = s("title"),
            name = s("name"),
            description = s("description"),
            tz_opts = select_options(TIMEZONES, &state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into())),
            locale_opts = select_options(LOCALES, &state.settings.get_str("locale").unwrap_or_else(|| "en".into())),
            metadata = metadata_sections(&state),
            social = social_sections(&state),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn general_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let get = |k: &str| form.get(k).cloned().unwrap_or_default();
    let title = get("title").trim().to_string();
    if title.is_empty() {
        return settings_flash("general", "error", "Title can't be empty.");
    }
    let non_empty = |v: String, fallback: &str| {
        let v = v.trim();
        if v.is_empty() { fallback.to_string() } else { v.to_string() }
    };
    let timezone = non_empty(get("timezone"), "Etc/UTC");
    let locale = non_empty(get("locale"), "en");
    let mut ok = state.settings.set(&state.db, "title", &serde_json::json!(title)).await.is_ok()
        && state.settings.set(&state.db, "description", &serde_json::json!(get("description"))).await.is_ok()
        && state.settings.set(&state.db, "timezone", &serde_json::json!(timezone)).await.is_ok()
        && state.settings.set(&state.db, "locale", &serde_json::json!(locale)).await.is_ok();
    if let Some(name) = form.get("name") {
        ok &= state.settings.set(&state.db, "name", &serde_json::json!(name.trim())).await.is_ok();
    }

    // Meta data and the social accounts post with this form now, so one Save
    // covers the page.
    for key in METADATA_KEYS.iter().chain(SOCIAL_ACCOUNTS.iter().map(|(k, _)| k)) {
        ok &= state.settings.set(&state.db, key, &serde_json::json!(get(key))).await.is_ok();
    }
    let llms = form.get("llms_enabled").map(|v| v == "1").unwrap_or(false);
    ok &= state.settings.set(&state.db, "llms_enabled", &serde_json::Value::Bool(llms)).await.is_ok();

    if ok {
        settings_saved(&state, "general", "General settings saved.")
    } else {
        settings_flash("general", "error", "Saving failed — check the logs.")
    }
}

// --- meta data: metadata, twitter card, facebook card ---

/// One meta-data text field row: label + input + recommended-length hint.
pub(crate) fn meta_field_row(icon_svg: &str, tint: &str, key: &str, label: &str, value: &str, hint: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" "><span>{label}</span></label>
<p class="muted" style="margin:6px 0 0">{hint}</p></div></div>"#,
        tint = tint,
        icon = icon_svg,
        key = esc(key),
        label = esc(label),
        value = esc(value),
        hint = esc(hint),
    )
}

/// The mock search result Ghost shows above the metadata fields.
///
/// `title` and `description` carry the saved values already resolved against
/// their site-level fallbacks, so the preview reads the way a result actually
/// would. The elements are tagged with `data-prev` so the page script can
/// retitle them as the fields are typed into.
pub(crate) fn search_preview(site_url: &str, icon: &str, title: &str, description: &str) -> String {
    let trimmed = site_url.trim_end_matches('/');
    let host = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let favicon = if icon.is_empty() {
        String::new()
    } else {
        format!(r#" style="background-image:url('{}')""#, esc(icon))
    };
    format!(
        r#"<div class="seo-prev rise" style="--i:1"><span class="cap">Preview</span>
<div class="sheet">
<div class="site"><span class="favicon"{favicon}></span>
<div><div class="host">{host}</div><div class="addr">{url}</div></div></div>
<span class="g-title" data-prev="search-title">{title}</span>
<span class="g-desc" data-prev="search-description">{description}</span>
</div></div>"#,
        favicon = favicon,
        host = esc(host),
        url = esc(trimmed),
        title = esc(title),
        description = esc(description),
    )
}

/// The mock social post Ghost shows above the X and Facebook card fields.
///
/// `network` is the field-name stem (`twitter` or `og`), which is also what
/// the page script keys its live updates off.
pub(crate) fn social_card_preview(
    network: &str,
    site_url: &str,
    site_title: &str,
    icon: &str,
    image: &str,
    title: &str,
    description: &str,
) -> String {
    let trimmed = site_url.trim_end_matches('/');
    let host = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let avatar = if icon.is_empty() {
        String::new()
    } else {
        format!(r#" style="background-image:url('{}')""#, esc(icon))
    };
    let shot = if image.is_empty() {
        format!(r#"<span class="shot empty" data-prev="{network}-shot">No image set</span>"#)
    } else {
        format!(
            r#"<span class="shot" data-prev="{network}-shot"><img src="{src}" alt=""></span>"#,
            src = esc(image)
        )
    };
    format!(
        r#"<div class="seo-prev social rise" style="--i:1"><span class="cap">Preview</span>
<div class="sheet">
<div class="who"><span class="avatar"{avatar}></span>
<div><div class="name">{site_title}</div><div class="when">2h</div></div></div>
<div class="bar"></div><div class="bar short"></div>
<div class="card-mock">{shot}
<div class="body"><span class="t" data-prev="{network}-title">{title}</span>
<span class="d" data-prev="{network}-description">{description}</span>
<span class="u">{host}</span></div></div>
</div></div>"#,
        avatar = avatar,
        site_title = esc(site_title),
        network = esc(network),
        shot = shot,
        title = esc(title),
        description = esc(description),
        host = esc(host),
    )
}

/// One meta-data image row: preview + URL input (like the design page).
pub(crate) fn meta_image_row(key: &str, label: &str, value: &str) -> String {
    let preview = if value.is_empty() {
        String::new()
    } else {
        format!(
            r#"<img src="{v}" alt="" style="height:44px;width:88px;object-fit:cover;border-radius:10px;box-shadow:var(--shadow-1)">"#,
            v = esc(value)
        )
    };
    format!(
        r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{img}</span>
<div class="body" style="width:100%">
<div style="display:flex;gap:14px;align-items:center">{preview}
<label class="field" style="margin:0;flex:1"><input name="{key}" value="{value}" placeholder=" "><span>{label} · URL</span></label></div></div></div>"#,
        img = svg("image"),
        key = esc(key),
        value = esc(value),
        label = esc(label),
        preview = preview,
    )
}

/// The Meta data sections — search result, X card, Facebook card.
///
/// Built as a fragment rather than a page of its own: these live inside
/// General settings, under the title and description they fall back to, so
/// the thing being overridden is on screen beside the override.
pub(crate) fn metadata_sections(state: &AppState) -> String {
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let raw = |k: &str| state.settings.get_str(k).unwrap_or_default();

    // Every card falls back the way the front end does — the site title and
    // description, then the meta overrides — so the previews show what a
    // crawler would actually read rather than an empty box.
    let site_title = raw("title");
    let site_description = raw("description");
    let icon = raw("icon");
    let site_url = state.site_url();
    let fallback = |specific: &str, meta: &str, site: &str| {
        let specific = raw(specific);
        if !specific.is_empty() {
            return specific;
        }
        let meta = raw(meta);
        if !meta.is_empty() { meta } else { site.to_string() }
    };

    format!(
            r#"<div class="section">Meta data · search</div>
{search_preview}
<div class="card rows rise" style="--i:4">
{meta_title}{meta_description}
{llms_row}
</div>
<div class="section">Meta data · X card</div>
{x_preview}
<div class="card rows rise" style="--i:5">
{twitter_title}{twitter_description}{twitter_image}
</div>
<div class="section">Meta data · Facebook card</div>
{fb_preview}
<div class="card rows rise" style="--i:6">
{og_title}{og_description}{og_image}
</div>"#,
            // Ghost reads this as "on unless explicitly false", so a site that
            // has never touched it is opted in.
            llms_row = format!(
                r#"<div class="row toggle-row"><span class="ic tint-purple">{ic}</span>
<div class="label"><b>Enable structured data for LLMs and AI search engines</b>
<span>Publishes <code>/llms.txt</code>, and lets any post or page URL be fetched as Markdown by appending <code>.md</code>.</span></div>
<label class="switch"><input type="checkbox" name="llms_enabled" value="1"{on} form="general-form"><span class="track"><span class="thumb"></span></span></label></div>"#,
                ic = svg("globe"),
                on = if llms_enabled(state) { " checked" } else { "" },
            ),
            search_preview = search_preview(
                &site_url,
                &icon,
                &fallback("meta_title", "title", &site_title),
                &fallback("meta_description", "description", &site_description),
            ),
            x_preview = social_card_preview(
                "twitter",
                &site_url,
                &site_title,
                &icon,
                &raw("twitter_image"),
                &fallback("twitter_title", "meta_title", &site_title),
                &fallback("twitter_description", "meta_description", &site_description),
            ),
            fb_preview = social_card_preview(
                "og",
                &site_url,
                &site_title,
                &icon,
                &raw("og_image"),
                &fallback("og_title", "meta_title", &site_title),
                &fallback("og_description", "meta_description", &site_description),
            ),
            meta_title = meta_field_row(&svg("title"), "purple", "meta_title", "Meta title", &s("meta_title"), "Recommended: under 70 characters. Falls back to the site title."),
            meta_description = meta_field_row(&svg("notes"), "purple", "meta_description", "Meta description", &s("meta_description"), "Recommended: under 156 characters. Falls back to the site description."),
            // The settings keys stay `twitter_*`: Ghost renamed the label to X
            // but kept the keys, and they are what `twitter:` meta tags read.
            twitter_title = meta_field_row(&svg("title"), "purple", "twitter_title", "X title", &s("twitter_title"), "Recommended: under 70 characters."),
            twitter_description = meta_field_row(&svg("notes"), "purple", "twitter_description", "X description", &s("twitter_description"), "Recommended: under 200 characters."),
            twitter_image = meta_image_row("twitter_image", "X image", &s("twitter_image")),
            og_title = meta_field_row(&svg("title"), "purple", "og_title", "Facebook title", &s("og_title"), "Recommended: under 70 characters."),
            og_description = meta_field_row(&svg("notes"), "purple", "og_description", "Facebook description", &s("og_description"), "Recommended: under 200 characters."),
            og_image = meta_image_row("og_image", "Facebook image", &s("og_image")),
    )
}

/// The text keys the Meta data sections own.
pub(crate) const METADATA_KEYS: &[&str] = &[
    "meta_title", "meta_description",
    "twitter_title", "twitter_description", "twitter_image",
    "og_title", "og_description", "og_image",
];
