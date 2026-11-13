//! Settings -> Design: site-wide brand settings plus the active theme's own declared custom settings.

use super::*;

// ---------------------------------------------------------------------------
// design (Ghost-style: site-wide brand settings + the active theme's
// declared custom settings, stored under custom_theme_settings and read by
// templates as @custom)
// ---------------------------------------------------------------------------

/// One `config.custom` declaration from the active theme's package.json.
pub(crate) struct CustomField {
    key: String,
    /// `select`, `color`, `boolean`, `text`, `image`, `collection`, …
    ftype: String,
    group: String,
    description: String,
    options: Vec<String>,
}

impl CustomField {
    /// `site_background_color` → `Site background color` (how Ghost labels
    /// the field in the Customize panel).
    fn label(&self) -> String {
        let mut s = self.key.replace('_', " ");
        if let Some(first) = s.get_mut(0..1) {
            let upper = first.to_uppercase();
            s.replace_range(0..1, &upper);
        }
        s
    }
}

pub(crate) fn custom_fields(decls: &serde_json::Map<String, serde_json::Value>) -> Vec<CustomField> {
    decls
        .iter()
        .map(|(key, def)| CustomField {
            key: key.clone(),
            ftype: def.get("type").and_then(|t| t.as_str()).unwrap_or("text").to_string(),
            group: def.get("group").and_then(|g| g.as_str()).unwrap_or("").to_string(),
            description: def.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            options: def
                .get("options")
                .and_then(|o| o.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// Coerce a posted form into override values for the declared fields,
/// mirroring Ghost's validation: only declared keys are addressable, selects
/// must be one of the theme's options, and an unticked boolean is `false`
/// (marked by the hidden `_<key>` companion input).
pub(crate) fn coerce_custom_values(
    decls: &serde_json::Map<String, serde_json::Value>,
    form: &HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    let defaults = ghost_render::ThemeState::custom_settings_defaults(decls);
    let mut out = serde_json::Map::new();
    for f in custom_fields(decls) {
        let value = match f.ftype.as_str() {
            "boolean" => serde_json::Value::Bool(
                form.contains_key(&f.key) && form.get(&f.key).map(String::as_str) == Some("1"),
            ),
            "select" => match form.get(&f.key) {
                Some(v) if f.options.iter().any(|o| o == v) => serde_json::Value::String(v.clone()),
                // an off-list select falls back to the theme's default
                _ => defaults.get(&f.key).cloned().unwrap_or(serde_json::Value::Null),
            },
            // text, image, color and anything else Ghost-style is a string
            _ => serde_json::Value::String(form.get(&f.key).cloned().unwrap_or_default()),
        };
        out.insert(f.key, value);
    }
    out
}

/// Redirect back to the design page carrying a one-shot banner.
pub(crate) fn design_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/design/?{encoded}")).into_response()
}

pub(crate) fn design_field_row(f: &CustomField, value: &serde_json::Value) -> String {
    let title = esc(&f.label());
    let desc = if f.description.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="detail">{}</span>"#, esc(&f.description))
    };
    let body = format!(r#"<span class="title">{title}</span>{desc}"#);
    match f.ftype.as_str() {
        "boolean" => format!(
            r#"<div class="row"><div class="body">{body}</div>
<div class="trailing"><input type="hidden" name="_{key}" value="1">
<label class="switch"><input type="checkbox" name="{key}" value="1" {checked}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            key = esc(&f.key),
            checked = if value.as_bool().unwrap_or(false) { "checked" } else { "" },
            body = body,
        ),
        "color" => {
            let v = value.as_str().unwrap_or("");
            format!(
                r#"<div class="row"><div class="body">{body}</div>
<div class="trailing color-field"><input type="color" name="{key}" value="{v}" aria-label="{title}"><code>{v}</code></div></div>"#,
                key = esc(&f.key),
                v = esc(v),
                title = title,
                body = body,
            )
        }
        "select" => {
            // A theme's options are plain strings, so each is its own label.
            let options: Vec<(&str, &str)> =
                f.options.iter().map(|o| (o.as_str(), o.as_str())).collect();
            format!(
                r#"<div class="row"><div class="body">{body}</div>
<div class="trailing">{control}</div></div>"#,
                control = selbox(&f.key, &f.label(), &options, value.as_str().unwrap_or("")),
                body = body,
            )
        }
        "image" => {
            let v = value.as_str().unwrap_or("");
            let preview = if v.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<img src="{v}" alt="" style="height:44px;width:88px;object-fit:cover;border-radius:10px;box-shadow:var(--shadow-1)">"#,
                    v = esc(v)
                )
            };
            format!(
                r#"<div class="row" style="align-items:flex-start"><div class="body" style="width:100%">
<div style="display:flex;gap:14px;align-items:center">{preview}
<label class="field" style="margin:0;flex:1"><input data-theme-image name="{key}" value="{v}" placeholder=" "><span>{title} · URL</span></label></div></div></div>"#,
                key = esc(&f.key),
                v = esc(v),
                title = title,
                preview = preview,
            )
        }
        // collection is not editable here (Ghost manages it through a
        // dedicated UI); everything else is a plain text field
        "collection" => format!(
            r#"<div class="row"><div class="body">{body}</div><div class="trailing"><span class="chip tonal">not editable</span></div></div>"#,
            body = body,
        ),
        _ => format!(
            r#"<div class="row"><div class="body">{body}</div>
<div class="trailing" style="flex:1;max-width:260px">{control}</div></div>"#,
            control = textbox(&f.key, &f.label(), value.as_str().unwrap_or("")),
            body = body,
        ),
    }
}

pub(crate) fn brand_image_field(key: &str, label: &str, value: &str) -> String {
    let preview_size = match key {
        "icon" => "max-width:48px;max-height:48px;",
        "logo" => "max-width:min(160px,100%);max-height:48px;",
        _ => "max-width:100%;max-height:120px;",
    };
    format!(
        r#"<div class="row" style="align-items:flex-start" data-brand-image data-upload-url="{upload_url}">
<span class="ic tint-pink">{image}</span>
<div class="body" style="min-width:0;width:100%">
<img alt="{label} preview" hidden style="{preview_size}width:auto;object-fit:contain;border-radius:8px;margin-bottom:12px">
<label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" " data-image-url><span>{label} · URL</span></label>
<div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:12px">
<input type="file" accept="image/jpeg,image/png,image/gif,image/webp,image/svg+xml,image/avif" aria-label="Upload {label}" hidden>
<button class="btn tonal small" type="button" data-image-upload>{upload} Upload image</button>
{unsplash}
<button class="btn text small" type="button" data-image-remove hidden>Remove</button></div>
<p class="muted" role="status" aria-live="polite" style="margin:8px 0 0"></p>
</div></div>"#,
        upload_url = esc(&ghost_core::url::base("/ghost/api/admin/images/upload/")),
        image = svg("image"),
        upload = svg("upload"),
        key = esc(key),
        label = esc(label),
        value = esc(&ghost_core::url::base_asset(value)),
        unsplash = if key == "cover_image" {
            format!(r#"<button class="btn tonal small" type="button" data-image-unsplash>{} Unsplash</button>"#, svg("search"))
        } else { String::new() },
    )
}

pub(crate) async fn design_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let theme = state.theme.read().await.clone();
    let banner = match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    };

    // effective values: theme defaults with the owner's overrides on top
    let overrides = state
        .settings
        .get("custom_theme_settings")
        .and_then(|v| v.get(&theme.name).cloned());
    let effective = theme.custom_settings(overrides.as_ref());

    let fields = custom_fields(&theme.pkg.config.custom);
    // Ghost's design page shows a theme's custom settings on three shelves —
    // Site-wide, Homepage, Post — the convention Casper established with its
    // `group` declarations. A field's declared group picks its shelf;
    // anything ungrouped or novel lands on Site-wide rather than inventing a
    // fourth shelf mid-page.
    let mut shelves: [(&str, Vec<String>); 3] =
        [("Site-wide", vec![]), ("Homepage", vec![]), ("Post", vec![])];
    for f in &fields {
        let group = f.group.to_lowercase();
        let shelf = if group.contains("home") {
            &mut shelves[1]
        } else if group.contains("post") {
            &mut shelves[2]
        } else {
            &mut shelves[0]
        };
        shelf.1.push(f.key.clone());
    }
    let custom_html: String = if fields.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3"><div style="display:flex;gap:14px;align-items:center"><span class="ic tint-purple">{ic}</span>
<p class="muted" style="margin:0">The <b>{name}</b> theme declares no customizable settings.</p></div></div>"#,
            ic = svg("tune"),
            name = esc(&theme.name),
        )
    } else {
        shelves
            .iter()
            .enumerate()
            .filter(|(_, shelf)| !shelf.1.is_empty())
            .map(|(i, (label, keys))| {
                let rows: String = keys
                    .iter()
                    .filter_map(|k| {
                        fields
                            .iter()
                            .find(|f| &f.key == k)
                            .map(|f| design_field_row(f, effective.get(k).unwrap_or(&serde_json::Value::Null)))
                    })
                    .collect();
                format!(
                    r#"<div class="section">{section}</div><div class="card rows rise" style="--i:{i}">{rows}</div>"#,
                    section = esc(label),
                    i = i + 3,
                    rows = rows,
                )
            })
            .collect()
    };

    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    page_for_role(actor_role,
        "Design & branding",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Design &amp; branding</h1><p class="sub">{name} theme</p></div>
<div class="actions"><button class="btn text danger" type="submit" form="reset-form">Reset to defaults</button>
<button class="btn" type="submit" form="design-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/design/" id="design-form" data-unsplash="{unsplash_headers}">
<div class="section">Brand</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-pink">{palette}</span>
<div class="body"><div class="color-field">
<input type="color" name="accent_color" value="{accent}" aria-label="Accent color">
<div><span class="title" style="display:block">Accent color</span><span class="detail">{accent}</span></div>
</div></div></div>
{icon_field}
{logo_field}
{cover_field}
<div class="row" style="align-items:center"><span class="ic tint-pink">{type_ic}</span>
<div class="body"><span class="title">Typography</span><span class="detail">Fonts used across the site</span></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px"><select name="heading_font">{heading_opts}</select><span>Heading font</span>{car}</label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{notes}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px"><select name="body_font">{body_opts}</select><span>Body font</span>{car}</label></div></div>
</div>
{custom_html}
</form>
<form method="post" action="/ghost/design/reset/" id="reset-form" data-confirm="Reset theme settings|Custom settings for {name} go back to the theme's defaults."></form>
<script src="{assets}/unsplash-picker.js" defer></script>"#,
            name = esc(&theme.name),
            assets = ASSETS,
            unsplash_headers = esc(&unsplash_headers(&state).to_string()),
            back = svg("chevron"),
            check = svg("check"),
            title_ic = svg("title"),
            palette = svg("themes"),
            type_ic = svg("tune"),
            notes = svg("notes"),
            car = SELECT_CARET,
            logo_field = brand_image_field("logo", "Publication logo", &state.settings.get_str("logo").unwrap_or_default()),
            icon_field = brand_image_field("icon", "Publication icon", &state.settings.get_str("icon").unwrap_or_default()),
            cover_field = brand_image_field("cover_image", "Publication cover", &state.settings.get_str("cover_image").unwrap_or_default()),
            accent = s("accent_color"),
            heading_opts = select_options(FONT_STACKS, &state.settings.get_str("heading_font").unwrap_or_default()),
            body_opts = select_options(FONT_STACKS, &state.settings.get_str("body_font").unwrap_or_default()),
            banner = banner,
            custom_html = custom_html,
        ),
        true,
    )
    .into_response()
}

/// The font choices the Brand card offers, when the theme declares none of
/// its own. The stored value is the CSS `font-family` stack itself, so a
/// theme can drop `@site.heading_font` straight into its stylesheet, and the
/// empty string is Ghost's seeded default — "let the theme decide".
pub(crate) const FONT_STACKS: &[(&str, &str)] = &[
    ("", "Theme default"),
    ("-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif", "Modern sans-serif"),
    ("'Helvetica Neue', Helvetica, Arial, sans-serif", "Clean sans-serif"),
    ("Georgia, 'Times New Roman', Times, serif", "Classic serif"),
    ("'Iowan Old Style', 'Palatino Linotype', 'Book Antiqua', Palatino, serif", "Elegant serif"),
    ("'SF Mono', 'Cascadia Code', Consolas, 'Courier New', monospace", "Monospace"),
];

pub(crate) async fn design_preview(State(state): State<AppState>, headers: HeaderMap, axum::Json(form): axum::Json<HashMap<String,String>>) -> Response {
    if let Err(r)=require_role(&state,&headers,Role::Administrator).await{return r}
    let theme=state.theme.read().await.clone();
    let mut values=Vec::new();
    for key in ["logo","icon","cover_image","accent_color","heading_font","body_font"] {
        if let Some(value)=form.get(key){values.push((key.into(),serde_json::json!(value)));}
    }
    let mut custom=state.settings.get("custom_theme_settings").and_then(|value|value.as_object().cloned()).unwrap_or_default();
    custom.insert(theme.name.clone(),serde_json::Value::Object(coerce_custom_values(&theme.pkg.config.custom,&form)));
    values.push(("custom_theme_settings".into(),serde_json::Value::Object(custom)));
    values.push(("web_analytics".into(),serde_json::json!(false)));
    values.push(("is_private".into(),serde_json::json!(false)));
    let mut preview=state.clone();preview.settings=std::sync::Arc::new(state.settings.with_overrides(values));
    use tower::ServiceExt;
    match ghost_front::site_router(preview).oneshot(axum::http::Request::builder().uri("/").body(axum::body::Body::empty()).unwrap()).await {
        Ok(mut response)=>{response.headers_mut().insert(header::CACHE_CONTROL,"private, no-store".parse().unwrap());response},
        Err(_)=>(StatusCode::INTERNAL_SERVER_ERROR,"Could not render preview").into_response()
    }
}

pub(crate) async fn design_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let theme = state.theme.read().await.clone();

    for key in ["logo", "icon", "cover_image", "accent_color", "heading_font", "body_font"] {
        let v = form.get(key).cloned().unwrap_or_default();
        let _ = state.settings.set(&state.db, key, &serde_json::json!(v)).await;
    }

    let values = coerce_custom_values(&theme.pkg.config.custom, &form);
    match state
        .settings
        .set(&state.db, "custom_theme_settings", &{
            let mut all = match state.settings.get("custom_theme_settings") {
                Some(serde_json::Value::Object(m)) => m,
                _ => serde_json::Map::new(),
            };
            all.insert(theme.name.clone(), serde_json::Value::Object(values));
            serde_json::Value::Object(all)
        })
        .await
    {
        Ok(()) => design_flash("ok", "Design settings saved."),
        Err(e) => design_flash("error", &e.message()),
    }
}

pub(crate) async fn design_reset(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let theme = state.theme.read().await.clone();
    let mut all = match state.settings.get("custom_theme_settings") {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    all.remove(&theme.name);
    match state
        .settings
        .set(&state.db, "custom_theme_settings", &serde_json::Value::Object(all))
        .await
    {
        Ok(()) => design_flash("ok", "Theme settings reset to defaults."),
        Err(e) => design_flash("error", &e.message()),
    }
}
