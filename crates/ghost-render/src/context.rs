//! @site / @config render context construction (public settings allowlist).
use ghost_db::SettingsCache;
use serde_json::{json, Map, Value};

/// Ghost's public settings allowlist (shared/settings-cache/public.js).
/// Themes branch on many of these — `members_enabled` and the portal keys in
/// particular gate whole sections of Source and Casper.
const PUBLIC_KEYS: &[&str] = &[
    "title", "description", "logo", "icon", "accent_color", "cover_image",
    "facebook", "twitter", "threads", "bluesky", "mastodon", "tiktok", "youtube",
    "instagram", "linkedin", "lang", "locale", "timezone", "codeinjection_head",
    "codeinjection_foot", "navigation", "secondary_navigation", "meta_title",
    "meta_description", "og_image", "og_title", "og_description",
    "announcement_content", "announcement_background", "announcement_visibility",
    "twitter_image", "twitter_title", "twitter_description",
    "members_support_address", "members_enabled", "donations_enabled",
    "allow_self_signup", "members_invite_only", "members_signup_access",
    "paid_members_enabled", "firstpromoter_account",
    "portal_button_style", "portal_button_signup_text", "portal_button_icon",
    "portal_signup_terms_html", "portal_signup_checkbox_required", "portal_plans",
    "portal_default_plan", "portal_name", "portal_button",
    "comments_enabled", "recommendations_enabled", "outbound_link_tagging",
    "default_email_address", "support_email_address",
    "editor_default_email_recipients", "labs", "site_uuid",
    // `ghost_head` decides whether to advertise the Markdown source from
    // these two, so both have to reach the render root.
    "llms_enabled", "is_private",
];

/// Build the `site` object placed at the render root.
pub fn build_site(settings: &SettingsCache, site_url: &str, admin_url: &str) -> Value {
    let mut m = Map::new();
    for key in PUBLIC_KEYS {
        if let Some(v) = settings.get(key) {
            if !v.is_null() {
                m.insert((*key).to_string(), v);
            }
        }
    }
    // ensure common defaults so templates never explode
    let mut d = |k: &str, default: Value| m.entry(k.to_string()).or_insert(default).clone();
    d("title", json!("Ghost"));
    d("description", json!(""));
    d("locale", json!("en"));
    d("timezone", json!("Etc/UTC"));
    d("navigation", json!([{"label": "Home", "url": "/"}]));
    d("secondary_navigation", Value::Array(vec![]));
    d("codeinjection_head", json!(""));
    d("codeinjection_foot", json!(""));
    // Ghost stores no separate `lang`; publication language is `locale`
    let locale = m.get("locale").cloned().unwrap_or(json!("en"));
    m.entry("lang".to_string()).or_insert(locale);

    // Ghost derives the members flags from members_signup_access rather than
    // storing them, and themes branch on them heavily.
    let signup_access = m
        .get("members_signup_access")
        .and_then(|v| v.as_str())
        .unwrap_or("all")
        .to_string();
    let members_enabled = signup_access != "none";
    m.insert("members_enabled".into(), json!(members_enabled));
    m.insert("members_invite_only".into(), json!(signup_access == "invite"));
    m.insert("allow_self_signup".into(), json!(signup_access == "all"));
    // paid membership also needs Stripe, which this port does not wire up
    m.entry("paid_members_enabled".to_string()).or_insert(json!(false));
    m.entry("donations_enabled".to_string()).or_insert(json!(false));
    m.entry("comments_enabled".to_string()).or_insert(json!("off"));
    m.entry("recommendations_enabled".to_string()).or_insert(json!(false));

    // Image settings are stored without the base path; `site` is an output
    // boundary, so put them into browser-ready form here.
    for key in ["logo", "icon", "cover_image", "og_image", "twitter_image"] {
        if let Some(Value::String(path)) = m.get(key) {
            let public = ghost_core::url::base_asset(path);
            m.insert(key.to_string(), json!(public));
        }
    }

    // Menu URLs too. Themes reach these both ways — `{{navigation}}`, which
    // renders the `navigation` partial, and a bare `{{#foreach
    // @site.navigation}}` (Flux's footer does exactly that) — so the prefix
    // has to be on the data rather than applied by the helper, or the second
    // form links outside the install.
    for key in ["navigation", "secondary_navigation"] {
        if let Some(Value::Array(items)) = m.get(key) {
            let prefixed: Vec<Value> = items
                .iter()
                .map(|item| match item {
                    Value::Object(o) => {
                        let mut o = o.clone();
                        if let Some(Value::String(url)) = o.get("url") {
                            let public = ghost_core::url::base(url);
                            o.insert("url".into(), json!(public));
                        }
                        Value::Object(o)
                    }
                    other => other.clone(),
                })
                .collect();
            m.insert(key.to_string(), Value::Array(prefixed));
        }
    }

    // computed
    m.insert("url".into(), json!(site_url));
    m.insert("admin_url".into(), json!(admin_url));
    Value::Object(m)
}

/// Build the `config` object (theme package.json config).
///
/// `card_assets` says whether the Koenig card stylesheet is linked, and
/// `card_assets_version` busts its cache; `ghost_head` reads both.
pub fn build_config(posts_per_page: i64, image_sizes: Value, card_assets: bool, card_assets_version: &str) -> Value {
    json!({
        "posts_per_page": posts_per_page,
        "image_sizes": image_sizes,
        "card_assets": card_assets,
        "card_assets_version": card_assets_version,
    })
}
