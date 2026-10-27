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
    // The two gift promotions and the Transistor block are settings Portal
    // reads off this payload: without them the `#/portal/gift` upsells never
    // appear and the account page drops the private-feed row, even though
    // Settings → Membership and Settings → Integrations both offer them.
    "portal_signup_gift_promotion", "portal_account_gift_promotion",
    "transistor_portal_heading", "transistor_portal_description",
    "transistor_portal_button_text", "transistor_portal_url_template",
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
    // Paid membership and tips both mean one thing to Ghost: is Stripe
    // connected. It calculates both keys from the credentials rather than
    // storing them, and so does this — a stored copy can only drift from the
    // keys it is meant to describe, and a stale `false` here is what hides
    // paid signup, `#/portal/account/plans`, `#/portal/gift` and
    // `#/portal/support` on a site that is perfectly able to take money.
    let stripe_connected = ["stripe_secret_key", "stripe_connect_secret_key"]
        .iter()
        .any(|k| settings.get_str(k).is_some_and(|v| !v.trim().is_empty()));
    m.insert("paid_members_enabled".into(), json!(stripe_connected));
    m.insert("donations_enabled".into(), json!(stripe_connected));
    m.entry("comments_enabled".to_string()).or_insert(json!("off"));
    m.entry("recommendations_enabled".to_string()).or_insert(json!(false));

    // Portal shows the private-podcast row when `transistor_portal_enabled` is
    // true, which Ghost computes rather than stores: the integration has to be
    // switched on *and* a feed template has to exist, or the button would lead
    // nowhere. The two settings live on separate admin screens, so neither one
    // alone is enough.
    let transistor = settings.get_bool("transistor");
    let has_template = m
        .get("transistor_portal_url_template")
        .and_then(|v| v.as_str())
        .is_some_and(|t| !t.trim().is_empty());
    m.insert("transistor_portal_enabled".into(), json!(transistor && has_template));

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

#[cfg(test)]
mod tests {
    use super::*;

    fn site_from(pairs: &[(&str, Value)]) -> Value {
        let cache = SettingsCache::from_values(
            pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())),
        );
        build_site(&cache, "https://site.test", "https://site.test/ghost")
    }

    #[test]
    fn portal_settings_reach_the_content_api() {
        // Every one of these is editable in Settings → Membership, and Portal
        // reads them off this object. A key missing here is a switch in the
        // admin that does nothing on the site.
        let site = site_from(&[
            ("portal_signup_gift_promotion", json!(true)),
            ("portal_account_gift_promotion", json!(true)),
            ("portal_plans", json!(["free", "monthly"])),
            ("portal_default_plan", json!("yearly")),
            ("portal_button_signup_text", json!("Join us")),
        ]);
        assert_eq!(site["portal_signup_gift_promotion"], json!(true));
        assert_eq!(site["portal_account_gift_promotion"], json!(true));
        assert_eq!(site["portal_plans"], json!(["free", "monthly"]));
        assert_eq!(site["portal_default_plan"], json!("yearly"));
        assert_eq!(site["portal_button_signup_text"], json!("Join us"));
    }

    #[test]
    fn stripe_credentials_decide_paid_membership_and_tips() {
        // Both keys are calculated, not stored: Portal hides paid signup,
        // gifts and `#/portal/support` whenever they are false.
        let without = site_from(&[]);
        assert_eq!(without["paid_members_enabled"], json!(false));
        assert_eq!(without["donations_enabled"], json!(false));

        let direct = site_from(&[("stripe_secret_key", json!("sk_test_123"))]);
        assert_eq!(direct["paid_members_enabled"], json!(true));
        assert_eq!(direct["donations_enabled"], json!(true));

        // Ghost(Pro) fills the Connect pair instead, and it counts the same.
        let connect = site_from(&[("stripe_connect_secret_key", json!("sk_test_456"))]);
        assert_eq!(connect["paid_members_enabled"], json!(true));

        // A stale stored `true` must not outrank missing credentials.
        let stale = site_from(&[("paid_members_enabled", json!(true))]);
        assert_eq!(stale["paid_members_enabled"], json!(false));
    }

    #[test]
    fn the_transistor_block_needs_both_the_switch_and_a_template() {
        let neither = site_from(&[]);
        assert_eq!(neither["transistor_portal_enabled"], json!(false));

        let no_template = site_from(&[("transistor", json!(true))]);
        assert_eq!(no_template["transistor_portal_enabled"], json!(false));

        // A template with the integration switched off is just as useless.
        let off = site_from(&[("transistor_portal_url_template", json!("https://x.test/{uuid}"))]);
        assert_eq!(off["transistor_portal_enabled"], json!(false));

        let both = site_from(&[
            ("transistor", json!(true)),
            ("transistor_portal_url_template", json!("https://x.test/{uuid}")),
        ]);
        assert_eq!(both["transistor_portal_enabled"], json!(true));
        assert_eq!(both["transistor_portal_url_template"], json!("https://x.test/{uuid}"));
    }
}
