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
    // Settings → SEO. Every tag `ghost_head` emits about this page is decided
    // from one of these, so they have to be on the render root beside the
    // title and the description they modify. `seo_robots_txt` is deliberately
    // *not* here — nothing in a template reads it, and it is the one SEO
    // setting large enough to be worth keeping off every page's context.
    "seo_structured_data", "seo_append_site_title", "seo_referrer_policy",
    "seo_noindex_tag_pages", "seo_noindex_author_pages", "seo_noindex_paged_pages",
    // Settings → Languages. `locales_enabled` is what tells a theme whether
    // to draw a language switcher at all; the two navigation maps are read
    // by `build_site` and replaced with the one menu this request needs, so
    // they never reach a template themselves.
    "locales_enabled", "locale_hreflang",
];

/// Settings read by [`build_site`] and then removed from what it returns.
///
/// The per-language navigation maps hold every language's menu. A template
/// only ever wants *this* request's, and `{{#foreach @site.navigation}}` is
/// how it asks — so the map is resolved into `navigation` and dropped rather
/// than handed over for a theme to index into.
const RESOLVED_AWAY: &[&str] = &["navigation_locales", "secondary_navigation_locales"];

/// Build the `site` object placed at the render root.
///
/// [`build_site_for`] with the site's default language — the shape every
/// caller that is not rendering a page wants (the Content API's `/settings/`,
/// the admin's preview payload).
pub fn build_site(settings: &SettingsCache, site_url: &str, admin_url: &str) -> Value {
    build_site_for(settings, site_url, admin_url, None)
}

/// Build the `site` object for a page being rendered in `locale`.
///
/// `locale` decides two things a template reads without knowing it asked:
/// `@site.locale`/`@site.lang`, which become `<html lang>`, and
/// `@site.navigation`, which is this language's menu rather than the
/// site-wide one. A language with no menu of its own falls back to the
/// site-wide `navigation`, so adding a language never empties its menu.
pub fn build_site_for(
    settings: &SettingsCache,
    site_url: &str,
    admin_url: &str,
    locale: Option<&str>,
) -> Value {
    build_site_for_page(settings, site_url, admin_url, locale, &Value::Null)
}

/// [`build_site_for`] for one page, whose `translations` decide where the
/// language switcher's links go.
///
/// `translations` is the `[{locale, url, active}]` list the front end
/// computes for the page (see `ghost_front::routes::translations_for`). A
/// menu item whose URL is [`ghost_core::locales::SWITCHER_URL`] is replaced
/// by one item per language: this page in that language where it exists,
/// and that language's home page where it does not — a switcher that drops
/// a language on an untranslated post leaves its readers no way across.
pub fn build_site_for_page(
    settings: &SettingsCache,
    site_url: &str,
    admin_url: &str,
    locale: Option<&str>,
    translations: &Value,
) -> Value {
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
    m.insert("name".into(), json!(settings.site_name()));
    // `locale` and `lang` are both set further down, from the language this
    // page is actually being rendered in rather than from the setting — on a
    // multilingual site those differ, and `<html lang>` has to follow the
    // page. (Ghost stores no separate `lang`; the publication language is
    // `locale`, and that remains the default.)

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

    // Portal loads FirstPromoter from this derived account field, not the
    // separate toggle and id that the settings screen stores.
    let firstpromoter = settings.get_bool("firstpromoter")
        .then(|| settings.get_str("firstpromoter_id"))
        .flatten().map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
    m.insert("firstpromoter_account".into(), json!(firstpromoter));

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

    // Labs flags that have gone GA are true whatever the database says —
    // Ghost overlays them in `labs.getAll()` rather than writing them, so a
    // site that predates the release still reports them on. Themes branch on
    // these, so the overlay has to happen before `labs` reaches the root.
    let mut labs = match m.get("labs") {
        Some(Value::Object(o)) => o.clone(),
        // Older rows hold the object as a JSON string; both shapes read here.
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
        _ => serde_json::Map::new(),
    };
    for flag in ghost_core::LABS_GA_FLAGS {
        labs.insert((*flag).to_string(), json!(true));
    }
    m.insert("labs".into(), Value::Object(labs));

    // Image settings are stored without the base path; `site` is an output
    // boundary, so put them into browser-ready form here.
    for key in ["logo", "icon", "cover_image", "og_image", "twitter_image"] {
        if let Some(Value::String(path)) = m.get(key) {
            let public = ghost_core::url::base_asset(path);
            m.insert(key.to_string(), json!(public));
        }
    }

    // This request's language, and its menu.
    //
    // Done before the base-path prefixing below, so a swapped-in menu goes
    // through exactly the same treatment the site-wide one does — a
    // per-language menu whose URLs were left unprefixed would link outside
    // the install on a subdirectory site.
    let locales = ghost_core::locales::Locales::from_settings(
        settings.get_str("locale").as_deref(),
        settings.get("locales_enabled").as_ref(),
    );
    let active = locales.resolve(locale);
    for (nav_key, map_key) in
        [("navigation", "navigation_locales"), ("secondary_navigation", "secondary_navigation_locales")]
    {
        let per_locale = settings.get(map_key).and_then(|v| match v {
            // Stored as an object; older rows and form posts hold the JSON
            // as a string.
            Value::Object(o) => Some(Value::Object(o)),
            Value::String(raw) => serde_json::from_str::<Value>(&raw).ok(),
            _ => None,
        });
        if let Some(Value::Array(items)) = per_locale.as_ref().and_then(|v| v.get(&active)).cloned() {
            // An empty list for a language is a deliberate "no menu here",
            // not a missing entry, so it replaces the site-wide one.
            m.insert(nav_key.to_string(), Value::Array(items));
        }
    }
    let switcher = language_switcher(&locales, &active, translations);
    let mut has_switcher = false;
    for key in ["navigation", "secondary_navigation"] {
        if let Some(Value::Array(items)) = m.get(key) {
            let (expanded, found) = place_switcher(items, switcher.as_ref());
            has_switcher |= found;
            m.insert(key.to_string(), Value::Array(expanded));
        }
    }
    // What `{{ghost_head}}` hands the dropdown script. Only when a menu
    // actually carries the switcher: a site that has not added one gets no
    // script at all.
    if let (true, Some(sw)) = (has_switcher, &switcher) {
        m.insert("language_switcher".into(), sw.for_page());
    }
    m.insert("locale".into(), json!(active.clone()));
    m.insert("lang".into(), json!(active.clone()));
    // What a theme branches on to decide whether to draw a switcher.
    m.insert("locales".into(), json!(locales.enabled()));
    m.insert("is_multilingual".into(), json!(locales.multilingual()));
    for key in RESOLVED_AWAY {
        m.remove(*key);
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

/// The language switcher for one page: this page in every language, and
/// the single menu item that stands for it.
///
/// The switcher reaches a theme as **one** ordinary menu item — labelled
/// with the language being read, in that language — so every theme draws
/// it, whether it renders `{{navigation}}` or walks `@site.navigation`
/// itself, and nothing in a theme has to change. `{{ghost_head}}` then
/// injects a script, as it does Portal, that finds that item on the page
/// and turns it into a dropdown of every language (see
/// `assets/language-switcher.js`).
///
/// Without the script the item is still a working link: it goes to the
/// next language, which on the common two-language site is exactly the
/// toggle somebody wants.
struct Switcher {
    active: String,
    /// `(locale, native name, url without base path, active)`.
    links: Vec<(String, String, String, bool)>,
}

impl Switcher {
    /// The language the item links to with scripts off.
    fn fallback(&self) -> &str {
        let at = self.links.iter().position(|l| l.3).unwrap_or(0);
        &self.links[(at + 1) % self.links.len()].2
    }

    fn label(&self) -> String {
        ghost_core::locales::native_name(&self.active)
    }

    /// The menu item. URLs come back *without* the base path: the caller
    /// prefixes every menu URL afterwards.
    fn item(&self) -> Value {
        json!({ "label": self.label(), "url": self.fallback(), "lang_switcher": true })
    }

    /// The payload for the script, with browser-ready URLs. `trigger` is
    /// how it recognises the item in the theme's markup: by where it links
    /// and what it says, since a theme's own menu markup carries no class
    /// this code chose.
    fn for_page(&self) -> Value {
        json!({
            "current": self.active,
            "trigger": { "label": self.label(), "url": ghost_core::url::base(self.fallback()) },
            "languages": self.links.iter().map(|(code, name, url, active)| json!({
                "locale": code,
                "label": name,
                "url": ghost_core::url::base(url),
                "active": active,
            })).collect::<Vec<_>>(),
        })
    }
}

/// `None` on a single-language site: a switcher with one language in it is
/// a link to the page you are on.
///
/// Each language links to this page in that language where it exists, and
/// to its home page where it does not — a switcher that dropped a language
/// on an untranslated post would leave its readers no way across.
fn language_switcher(
    locales: &ghost_core::locales::Locales,
    active: &str,
    translations: &Value,
) -> Option<Switcher> {
    if !locales.multilingual() {
        return None;
    }
    let page_in = |code: &str| -> Option<String> {
        translations.as_array()?.iter().find_map(|t| {
            let c = t.get("locale").and_then(Value::as_str)?;
            let url = t.get("url").and_then(Value::as_str)?;
            // `translations` already carries the base path.
            c.eq_ignore_ascii_case(code).then(|| ghost_core::url::strip(url).to_string())
        })
    };
    let links = locales
        .enabled()
        .iter()
        .map(|code| {
            (
                code.clone(),
                ghost_core::locales::native_name(code),
                page_in(code).unwrap_or_else(|| locales.localise("/", code)),
                code.eq_ignore_ascii_case(active),
            )
        })
        .collect();
    Some(Switcher { active: active.to_string(), links })
}

/// Replace the first switcher marker in a menu with the switcher's item,
/// and drop any others — a menu with the marker twice gets one switcher —
/// or drop them all when there is no switcher to draw. Reports whether the
/// menu had a marker.
fn place_switcher(items: &[Value], switcher: Option<&Switcher>) -> (Vec<Value>, bool) {
    let is_marker = |item: &Value| {
        item.get("url").and_then(Value::as_str).is_some_and(ghost_core::locales::is_switcher)
    };
    if !items.iter().any(is_marker) {
        return (items.to_vec(), false);
    }
    let mut out = Vec::with_capacity(items.len());
    let mut placed = false;
    for item in items {
        if !is_marker(item) {
            out.push(item.clone());
        } else if !placed {
            placed = true;
            if let Some(sw) = switcher {
                out.push(sw.item());
            }
        }
    }
    (out, true)
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
    fn site_name_is_independent_of_title_with_a_title_fallback() {
        let site = site_from(&[("name", json!(" Daily News ")), ("title", json!("News and analysis"))]);
        assert_eq!(site["name"], "Daily News");
        assert_eq!(site["title"], "News and analysis");
        for name in [json!(""), json!("   "), Value::Null] {
            let site = site_from(&[("name", name), ("title", json!("Existing publication"))]);
            assert_eq!(site["name"], "Existing publication");
        }
        assert_eq!(site_from(&[("title", json!("Existing publication"))])["name"], "Existing publication");
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
    fn ga_labs_flags_are_true_whatever_the_database_says() {
        // Ghost overlays these in `labs.getAll()` rather than writing them,
        // so a site that predates the release still reports them on — and a
        // theme branching on one must see the same answer here.
        let site = site_from(&[("labs", json!({ "superEditors": true }))]);
        let labs = site["labs"].as_object().expect("labs reaches the root as an object");
        for flag in ghost_core::LABS_GA_FLAGS {
            assert_eq!(labs[*flag], json!(true), "{flag} is GA and must read true");
        }
        // A stored flag that is not GA survives the overlay untouched.
        assert_eq!(labs["superEditors"], json!(true));

        // A stored `false` for a GA flag loses to the overlay, because in
        // Ghost that flag is no longer switchable at all.
        let stale = site_from(&[("labs", json!({ "editorReact": false }))]);
        assert_eq!(stale["labs"]["editorReact"], json!(true));

        // And a site that has never written a labs object still gets them.
        let fresh = site_from(&[]);
        assert_eq!(fresh["labs"]["billingReact"], json!(true));
    }

    #[test]
    fn firstpromoter_account_is_derived_from_the_enabled_integration() {
        assert_eq!(site_from(&[])["firstpromoter_account"], Value::Null);
        assert_eq!(site_from(&[("firstpromoter_id", json!("account"))])["firstpromoter_account"], Value::Null);
        assert_eq!(site_from(&[("firstpromoter", json!(true)), ("firstpromoter_id", json!(" account "))])["firstpromoter_account"], json!("account"));
        assert_eq!(site_from(&[("firstpromoter", json!(true))])["firstpromoter_account"], Value::Null);
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

    fn page_site(pairs: &[(&str, Value)], locale: &str, translations: Value) -> Value {
        let cache = SettingsCache::from_values(pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())));
        build_site_for_page(&cache, "https://site.test", "https://site.test/ghost", Some(locale), &translations)
    }

    fn menu_with_switcher() -> Value {
        json!([{"label": "Home", "url": "/"}, {"label": "Language", "url": "#language-switcher"}])
    }

    #[test]
    fn the_switcher_is_one_menu_item_named_for_the_language_being_read() {
        let site = page_site(
            &[("locale", json!("en")), ("locales_enabled", json!(["de"])), ("navigation", menu_with_switcher())],
            "de",
            json!([
                {"locale": "en", "url": "/hello/", "active": false},
                {"locale": "de", "url": "/de/hallo/", "active": true},
            ]),
        );
        let nav = site["navigation"].as_array().unwrap();
        assert_eq!(nav.len(), 2, "{nav:?}");
        assert_eq!(nav[1]["label"], "Deutsch");
        // With scripts off it is a toggle to the other language.
        assert_eq!(nav[1]["url"], "/hello/");
        assert_eq!(nav[1]["lang_switcher"], true);

        // The dropdown's data: every language, and how to find the item.
        let sw = &site["language_switcher"];
        assert_eq!(sw["current"], "de");
        assert_eq!(sw["trigger"], json!({"label": "Deutsch", "url": "/hello/"}));
        assert_eq!(
            sw["languages"],
            json!([
                {"locale": "en", "label": "English", "url": "/hello/", "active": false},
                {"locale": "de", "label": "Deutsch", "url": "/de/hallo/", "active": true},
            ])
        );
    }

    #[test]
    fn a_language_without_this_page_links_to_its_home_page() {
        let site = page_site(
            &[("locale", json!("en")), ("locales_enabled", json!(["de", "fr"])), ("navigation", menu_with_switcher())],
            "en",
            json!([{"locale": "en", "url": "/hello/", "active": true}]),
        );
        let urls: Vec<&str> =
            site["language_switcher"]["languages"].as_array().unwrap().iter().map(|l| l["url"].as_str().unwrap()).collect();
        assert_eq!(urls, ["/hello/", "/de/", "/fr/"]);
        assert_eq!(site["navigation"][1]["url"], "/de/", "the fallback is the next language");
    }

    #[test]
    fn a_site_without_a_switcher_gets_no_switcher_data() {
        // Multilingual, but nobody added the switcher to a menu: no script.
        let site = page_site(
            &[("locale", json!("en")), ("locales_enabled", json!(["de"])), ("navigation", json!([{"label": "Home", "url": "/"}]))],
            "en",
            json!([]),
        );
        assert!(site.get("language_switcher").is_none());
        // Single-language with a marker left in: the marker just goes.
        let site = page_site(&[("locale", json!("en")), ("navigation", menu_with_switcher())], "en", Value::Null);
        assert_eq!(site["navigation"], json!([{"label": "Home", "url": "/"}]));
        assert!(site.get("language_switcher").is_none());
    }

    #[test]
    fn a_language_menu_carries_its_own_switcher() {
        let site = page_site(
            &[
                ("locale", json!("en")),
                ("locales_enabled", json!(["de"])),
                ("navigation", json!([{"label": "Home", "url": "/"}])),
                ("navigation_locales", json!({"de": [{"label": "Start", "url": "/de/"}, {"label": "Sprache", "url": "#language-switcher"}]})),
            ],
            "de",
            json!([]),
        );
        let labels: Vec<&str> = site["navigation"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Start", "Deutsch"]);
    }
}
