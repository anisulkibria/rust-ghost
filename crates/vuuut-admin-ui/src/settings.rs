//! The settings hub: Ghost's settings navigation, and every group behind it.

use super::*;

// The groups behind the hub, one module each. They are re-exported here
// so the router keeps naming them `settings::<handler>`.
mod general;
pub(crate) mod email;
mod newsletters;
mod features;
mod membership;
mod growth;
mod advanced;
mod backups;

pub(crate) use general::*;
pub(crate) use email::*;
pub(crate) use newsletters::*;
pub(crate) use features::*;
pub(crate) use membership::*;
pub(crate) use growth::*;
pub(crate) use advanced::*;
pub(crate) use backups::*;

// ---------------------------------------------------------------------------
// settings hub (Ghost's settings navigation. Every group has a page; the
// only deferred items are Growth → Network and Growth → Ghost Explore.)
// ---------------------------------------------------------------------------

/// One group in the settings navigation, in Ghost's sidebar order.
pub(crate) struct SettingsGroup {
    pub(crate) slug: &'static str,
    pub(crate) tool: Option<&'static str>,
    pub(crate) title: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) tint: &'static str,
    /// Sub-pages inside the group, in Ghost's order (shown as the row detail).
    pub(crate) items: &'static [&'static str],
}

impl SettingsGroup {
    pub(crate) fn path(&self) -> String {
        match self.tool {
            Some(tool) => format!("/ghost/{tool}/settings/"),
            None => format!("/ghost/settings/{}/", self.slug),
        }
    }
}

/// A–Z by title, with Danger zone last.
pub(crate) const SETTINGS_GROUPS: &[SettingsGroup] = &[
    SettingsGroup { slug: "analytics", tool: Some("analytics"), title: "Analytics", icon: "chart", tint: "green", items: &["Web analytics", "Email opens", "Email clicks", "Member sources", "Outbound link tagging"] },
    SettingsGroup { slug: "cookies", tool: Some("cookies"), title: "Cookies", icon: "cookie", tint: "amber", items: &["On or off", "Banner", "Choices", "Proof of consent"] },
    SettingsGroup { slug: "emails", tool: None, title: "Emails", icon: "mail", tint: "blue", items: &["Transactional email", "SMTP server", "Amazon SES", "Mailgun", "Sender identity", "Send a test"] },
    SettingsGroup { slug: "features", tool: None, title: "Features", icon: "tune", tint: "teal", items: &["Comments", "Automations", "Beta features", "Private features"] },
    SettingsGroup { slug: "forms", tool: Some("forms"), title: "Forms", icon: "form", tint: "amber", items: &["Notifications", "Spam protection", "Privacy & retention"] },
    SettingsGroup { slug: "general", tool: None, title: "General settings", icon: "settings", tint: "pink", items: &["Name, title & description", "Brand", "Timezone", "Meta data", "Social accounts", "Admin address"] },
    SettingsGroup { slug: "images", tool: Some("media"), title: "Images", icon: "image", tint: "green", items: &["Optimisation", "Size", "Quality", "Privacy"] },
    SettingsGroup { slug: "languages", tool: Some("languages"), title: "Languages", icon: "globe", tint: "teal", items: &["Default publication language", "Languages", "Search engines", "Navigation per language"] },
    SettingsGroup { slug: "memberships", tool: Some("memberships"), title: "Membership", icon: "ghost", tint: "teal", items: &["Access", "Spam filters", "Tiers", "Signup portal", "Gift subscriptions", "Tips & donations", "Custom fields"] },
    SettingsGroup { slug: "newsletters", tool: Some("newsletters"), title: "Newsletters", icon: "send", tint: "purple", items: &["Newsletter sending", "Default recipients", "Newsletter provider", "The newsletters themselves"] },
    SettingsGroup { slug: "payments", tool: None, title: "Payments", icon: "card", tint: "green", items: &["Stripe", "Polar", "Gumroad", "Store payment methods", "Tips & donations"] },
    SettingsGroup { slug: "popups", tool: Some("popups"), title: "Popups", icon: "panel-show", tint: "pink", items: &["On or off", "On phones", "Space between popups", "Consent note"] },
    SettingsGroup { slug: "security", tool: Some("security"), title: "Security", icon: "lock", tint: "red", items: &["Sign-in", "Sessions", "Response headers", "Requests this server makes and trusts", "Spam", "Firewall"] },
    SettingsGroup { slug: "seo", tool: Some("seo"), title: "SEO", icon: "trending", tint: "blue", items: &["Search appearance", "Crawling", "robots.txt", "IndexNow"] },
    SettingsGroup { slug: "danger", tool: None, title: "Danger zone", icon: "delete", tint: "red", items: &["Delete all content", "Delete site"] },
];

pub(crate) const LEGACY_SETTINGS_ALIASES: &[(&str, &str, &str)] = &[
    ("/settings/membership/", "memberships", ""),
    ("/settings/checkout/", "memberships", "checkout/"),
    ("/settings/bulkemail/", "newsletters", ""),
];

pub(crate) fn legacy_tool_settings_routes() -> Router<AppState> {
    fn redirect(from: String, to: String) -> axum::routing::MethodRouter<AppState> {
        axum::routing::any(move |uri: axum::http::Uri| async move {
            let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or(uri.path());
            let tail = path.strip_prefix(&from).unwrap_or_default();
            // Preserve submitted forms as well as bookmarked query strings.
            Redirect::permanent(&format!("{to}{tail}"))
        })
    }

    let mut router = Router::new();
    for g in SETTINGS_GROUPS.iter().filter(|g| g.tool.is_some()) {
        let from = format!("/settings/{}/", g.slug);
        let to = g.path();
        router = router.route(&from, redirect(from.clone(), to.clone()));
        if matches!(g.slug, "memberships" | "newsletters") {
            let nested = format!("{from}{{*rest}}");
            router = router.route(&nested, redirect(from, to));
        }
    }
    for (from, group, suffix) in LEGACY_SETTINGS_ALIASES {
        let g = SETTINGS_GROUPS.iter().find(|g| g.slug == *group).expect("known settings group");
        router = router.route(from, redirect(from.to_string(), format!("{}{suffix}", g.path())));
    }
    router
}

/// The individual settings behind the hub's search: one row per field, with
/// the section it sits in and the page it lives on. The groups above cover
/// what Ghost's own navigation names; this covers what a user actually types
/// — "port", "webhook", "reply-to" — which none of the page titles contain.
/// The social accounts are folded in from `SOCIAL_ACCOUNTS` at render time
/// rather than repeated here.
pub(crate) const SETTINGS_INDEX: &[(&str, &str, &str)] = &[
    // (page slug, section, field)
    ("general", "Name, title & description", "Name"),
    ("general", "Name, title & description", "Title"),
    ("general", "Name, title & description", "Description"),
    ("general", "Timezone", "Site timezone"),
    // Brand moved here from Customizations on 2026-11-20.
    ("general", "Brand", "Accent color"),
    ("general", "Brand", "Accent colour"),
    ("general", "Brand", "Publication logo"),
    ("general", "Brand", "Publication icon"),
    ("general", "Brand", "Favicon"),
    ("general", "Brand", "Publication cover"),
    ("general", "Brand", "Typography"),
    ("general", "Brand", "Heading font"),
    ("general", "Brand", "Body font"),
    // The section names stay in the haystack: "X card" and "Facebook card"
    // are what these are called, and they are the headings on the page.
    ("general", "Meta data · search", "Meta title"),
    ("general", "Meta data · search", "Meta description"),
    ("general", "Meta data · search", "Structured data for LLMs"),
    ("general", "Meta data · X card", "X title"),
    ("general", "Meta data · X card", "X description"),
    ("general", "Meta data · X card", "X image"),
    ("general", "Meta data · Facebook card", "Facebook title"),
    ("general", "Meta data · Facebook card", "Facebook description"),
    ("general", "Meta data · Facebook card", "Facebook image"),
    ("languages", "Default publication language", "Default publication language"),
    ("languages", "Default publication language", "Publication language"),
    ("languages", "Languages", "Multilingual"),
    ("languages", "Languages", "Translations"),
    ("languages", "Languages", "Language"),
    ("languages", "Search engines", "Publish hreflang alternates"),
    ("languages", "Search engines", "hreflang"),
    ("languages", "Navigation per language", "Primary menu per language"),
    ("languages", "Navigation per language", "Secondary menu per language"),
    // Forms. The module's builder and its responses live under Modules; these
    // are the switches that apply to every form at once, and none of them was
    // in this index until 2026-11-20 — the page was unsearchable.
    ("forms", "Notifications", "Email me about new responses"),
    ("forms", "Notifications", "Default notification addresses"),
    ("forms", "Spam protection", "Single-use signed tokens"),
    ("forms", "Spam protection", "Minimum fill time"),
    ("forms", "Spam protection", "Hidden decoy field"),
    ("forms", "Spam protection", "Honeypot"),
    ("forms", "Spam protection", "Rate limits"),
    ("forms", "Spam protection", "Same-site only"),
    ("forms", "Privacy & retention", "Store the sender's IP address"),
    ("forms", "Privacy & retention", "Keep responses for"),
    ("forms", "Privacy & retention", "Retention"),
    // Security. The field names are what somebody types looking for them —
    // "2FA", "HSTS", "session timeout", "X-Forwarded-For" — not only the
    // labels on the page, because none of those labels contain any of it.
    ("security", "Sign-in", "Require a code emailed at sign-in"),
    ("security", "Sign-in", "Two-factor authentication"),
    ("security", "Sign-in", "2FA"),
    ("security", "Sign-in", "Shortest staff password"),
    ("security", "Sign-in", "Password length"),
    ("security", "Sessions", "Sign staff out after"),
    ("security", "Sessions", "Session timeout"),
    ("security", "Sessions", "Sign staff out when idle for"),
    ("security", "Sessions", "Idle timeout"),
    ("security", "Sessions", "Sign out other sessions when a password changes"),
    ("security", "Response headers", "Send security headers"),
    ("security", "Response headers", "Content Security Policy"),
    ("security", "Response headers", "X-Frame-Options"),
    ("security", "Response headers", "Clickjacking"),
    ("security", "Response headers", "Referrer policy for admin pages"),
    ("security", "Response headers", "Strict transport security (HSTS)"),
    ("security", "Requests this server makes and trusts", "Believe forwarded client addresses"),
    ("security", "Requests this server makes and trusts", "X-Forwarded-For"),
    ("security", "Requests this server makes and trusts", "Trust proxy"),
    ("security", "Requests this server makes and trusts", "Refuse to fetch private addresses"),
    ("security", "Requests this server makes and trusts", "SSRF"),
    ("security", "Spam", "Refuse signups from disposable addresses"),
    ("security", "Spam", "Disposable email"),
    ("security", "Spam", "Signups must come from this site"),
    ("security", "Spam", "Block these user agents"),
    ("security", "Spam", "Bot blocking"),
    ("security", "Spam", "Blocked user agents"),
    ("security", "Firewall", "Firewall"),
    ("security", "Firewall", "Open the firewall"),
    ("security", "Firewall", "Block an IP address"),
    ("security", "Firewall", "IP allowlist"),
    // Danger zone. Like Forms, none of this was indexed until 2026-11-20, so
    // "delete site" found nothing at all.
    // Images. "Compress", "EXIF" and "GPS" are the words people search with;
    // none of them is a label on the page.
    ("images", "Optimisation", "Optimise images when they are uploaded"),
    ("images", "Optimisation", "Compress images"),
    ("images", "Optimisation", "Image optimization"),
    ("images", "Size", "Largest side, in pixels"),
    ("images", "Size", "Resize uploads"),
    ("images", "Size", "Maximum image size"),
    ("images", "Quality", "JPEG quality"),
    ("images", "Privacy", "Remove camera metadata"),
    ("images", "Privacy", "EXIF"),
    ("images", "Privacy", "GPS location"),
    // Cookies. People search for the law, not the feature: "GDPR", "CCPA",
    // "consent banner", "do not sell" — none of which is a label on the page.
    ("cookies", "On or off", "Show the consent banner"),
    ("cookies", "On or off", "Cookie banner"),
    ("cookies", "On or off", "GDPR consent banner"),
    ("cookies", "On or off", "How to ask"),
    ("cookies", "On or off", "Opt-in or opt-out (CCPA)"),
    ("cookies", "Banner", "Where it appears"),
    ("cookies", "Banner", "Heading"),
    ("cookies", "Banner", "Message"),
    ("cookies", "Banner", "Privacy policy link"),
    ("cookies", "Choices", "Ask again after"),
    ("cookies", "Choices", "Honour Global Privacy Control"),
    ("cookies", "Choices", "Do not sell or share"),
    ("cookies", "Choices", "Hold back embedded content"),
    ("cookies", "Choices", "Show a button to change choices"),
    ("cookies", "Proof of consent", "Keep a record of each decision"),
    ("cookies", "Proof of consent", "Consent log"),
    ("cookies", "Proof of consent", "Delete records after"),
    // Popups. "Kill switch" and "GDPR" are what people type looking for the
    // master switch and the consent note; neither phrase is on the page.
    ("popups", "On or off", "Show popups on the site"),
    ("popups", "On or off", "Turn off all popups"),
    ("popups", "On or off", "Kill switch"),
    ("popups", "On phones", "Hide popups on phones"),
    ("popups", "On phones", "Mobile"),
    ("popups", "Space between popups", "Hours between any two popups"),
    ("popups", "Space between popups", "Popup frequency cap"),
    ("popups", "Consent note", "Shown under every address field"),
    ("popups", "Consent note", "GDPR consent"),
    ("popups", "Consent note", "Privacy note"),
    ("danger", "Danger zone", "Delete all content"),
    ("danger", "Danger zone", "Delete site"),
    ("danger", "Danger zone", "Reset all authentication"),
    ("danger", "Danger zone", "Reset all gift links"),
    ("seo", "Search appearance", "Structured data"),
    ("seo", "Search appearance", "Open Graph"),
    ("seo", "Search appearance", "Twitter card"),
    ("seo", "Search appearance", "Schema.org JSON-LD"),
    ("seo", "Search appearance", "Append the site title to page titles"),
    ("seo", "Search appearance", "Referrer policy"),
    ("seo", "Crawling", "List images in the sitemap"),
    ("seo", "Crawling", "Hide tag archives from search"),
    ("seo", "Crawling", "Hide author archives from search"),
    ("seo", "Crawling", "Hide page 2 and beyond from search"),
    ("seo", "Crawling", "noindex"),
    ("seo", "robots.txt", "Custom robots.txt"),
    ("seo", "IndexNow", "IndexNow key"),
    ("analytics", "Analytics", "Web analytics"),
    ("analytics", "Analytics", "Email opens"),
    ("analytics", "Analytics", "Email clicks"),
    ("analytics", "Analytics", "Member sources"),
    ("analytics", "Analytics", "Outbound link tagging"),
    ("emails", "Transactional email", "Transport"),
    ("emails", "SMTP server", "Host"),
    ("emails", "SMTP server", "Port"),
    ("emails", "SMTP server", "Encryption"),
    ("emails", "SMTP server", "Username"),
    ("emails", "SMTP server", "Password"),
    ("emails", "Sender identity", "From address"),
    ("emails", "Sender identity", "From name"),
    ("emails", "Sender identity", "Reply-to"),
    ("emails", "Send a test", "Check the settings work"),
    ("emails", "Send a test", "Send to"),
    ("emails", "Amazon SES", "Newsletter messages per second"),
    ("emails", "Amazon SES", "Rate limit"),
    ("newsletters", "Newsletters", "Newsletter name"),
    // Mailgun is a transport, so its credentials sit with the other
    // transports on Emails; Newsletters only picks which one to use.
    ("emails", "Mailgun", "API key"),
    ("emails", "Mailgun", "Webhook signing key"),
    ("emails", "Mailgun", "Sending domain"),
    ("emails", "Mailgun", "Region"),
    ("emails", "Amazon SES", "AWS region"),
    ("emails", "Amazon SES", "SMTP endpoint"),
    ("emails", "Amazon SES", "SES SMTP username"),
    ("emails", "Amazon SES", "SES SMTP password"),
    ("newsletters", "Sender", "Sender name"),
    ("newsletters", "Sender", "Sender email"),
    ("newsletters", "Header", "Header image URL"),
    ("memberships", "Stripe", "Publishable key"),
    ("memberships", "Stripe", "Secret key"),
    ("memberships", "Stripe", "Webhook signing secret"),
    ("memberships", "Access", "Signup access"),
    ("memberships", "Access", "Default post access"),
    ("memberships", "Access", "Commenting"),
    ("memberships", "Tiers", "Monthly price"),
    ("memberships", "Tiers", "Yearly price"),
    ("memberships", "Tiers", "Currency"),
    ("memberships", "Tiers", "Free trial"),
    ("memberships", "Tiers", "Benefits"),
    ("memberships", "Signup portal", "Show portal button"),
    ("memberships", "Signup portal", "Portal button signup text"),
    ("memberships", "Signup portal", "Portal button icon"),
    ("memberships", "Signup portal", "Portal button style"),
    ("memberships", "Signup portal", "Show portal name"),
    ("memberships", "Signup portal", "Signup terms"),
    ("memberships", "Signup portal", "Require signup terms"),
    ("memberships", "Signup portal", "Plans shown at signup"),
    ("memberships", "Signup portal", "Default price at signup"),
    ("memberships", "Signup portal", "Support email address"),
    ("memberships", "Signup portal", "Promote gifts at signup"),
    ("memberships", "Signup portal", "Promote gifts on the account page"),
    // The welcome settings are flows now, not a single page URL.
    ("memberships", "Welcome emails", "Free member welcome flow"),
    ("memberships", "Welcome emails", "Paid member welcome flow"),
    ("memberships", "Checkout", "Checkout design"),
    ("memberships", "Stripe", "Stripe"),
    ("memberships", "Stripe", "Connect Stripe"),
    ("memberships", "Tiers", "Default Product"),
    ("memberships", "Custom fields", "Type"),
    ("memberships", "Access", "Site access"),
    ("memberships", "Access", "Access code"),
    ("memberships", "Spam filters", "Blocked email domains"),
    ("memberships", "Custom fields", "Field name"),
    ("memberships", "Custom fields", "Member access"),
    ("memberships", "Gift subscriptions", "Shareable link"),
    ("memberships", "Tips & donations", "Currency"),
    ("memberships", "Tips & donations", "Suggested amount"),
    ("newsletters", "Newsletter sending", "Newsletter sending"),
    ("newsletters", "Newsletter sending", "Enabled"),
    ("newsletters", "Newsletters", "Description"),
    ("newsletters", "Newsletter provider", "Provider"),
    ("newsletters", "Newsletter provider", "From address"),
    ("newsletters", "Default recipients", "Who gets a new post by default"),
    ("features", "Comments", "New comments"),
    ("features", "Comments", "Hold for review"),
    ("features", "Automations", "Automations"),
    ("features", "Beta features", "Feature flags"),
    ("features", "Private features", "Developer experiments"),
];

/// One searchable thing in settings: where it is, what to call it, and every
/// word it should answer to.
pub(crate) struct SearchEntry {
    pub(crate) group: &'static str,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) detail: String,
    /// What a query is matched against — the visible text plus the synonyms
    /// and keys that are not on screen.
    pub(crate) haystack: String,
    pub(crate) icon: &'static str,
    pub(crate) tint: &'static str,
}

/// Search entries for the settings hub or a single tool, including fields,
/// social accounts, and feature flags.
///
/// `dev` gates the private feature flags: they are only *rendered* on a
/// development site, so indexing them elsewhere leads to a row that is not
/// there.
pub(crate) fn settings_entries(dev: bool, tool: Option<&str>) -> Vec<SearchEntry> {
    let mut out: Vec<SearchEntry> = Vec::new();
    let mut push = |url: String, title: &str, detail: &str, haystack: &str, g: &SettingsGroup| {
        out.push(SearchEntry {
            group: g.slug,
            url,
            title: title.to_string(),
            detail: detail.to_string(),
            haystack: haystack.to_string(),
            icon: g.icon,
            tint: g.tint,
        });
    };
    // A page's row answers to its own name and its sub-pages' names, but
    // everything below it matches only its own text — so a broad query
    // ("security") lands on the page, and a narrow one ("port", "2FA") lands
    // on the thing itself without flooding the page in.
    let groups = || SETTINGS_GROUPS.iter().filter(|g| g.tool == tool);
    for g in groups() {
        push(
            g.path(),
            g.title,
            &g.items.join(" · "),
            &format!("{} {} {}", g.title, g.items.join(" "), g.slug),
            g,
        );
        for item in g.items {
            push(g.path(), item, g.title, item, g);
        }
    }
    let mut fields: Vec<(&str, &str, &str)> = SETTINGS_INDEX.to_vec();
    for (_, name) in SOCIAL_ACCOUNTS {
        fields.push(("general", "Social accounts", name));
    }
    for (slug, section, field) in &fields {
        if let Some(g) = groups().find(|g| g.slug == *slug) {
            push(
                g.path(),
                field,
                &format!("{} · {}", g.title, section),
                &format!("{section} {field}"),
                g,
            );
        }
    }
    // Each feature flag answers to its own name, so "automatic tax" or
    // "navigation icons" finds the switch rather than only the page it sits
    // on. Folded in here rather than copied into `SETTINGS_INDEX` so the two
    // lists cannot drift, and the flag key joins the haystack because that is
    // what a developer reading a theme has in front of them.
    if let Some(g) = groups().find(|g| g.slug == "features") {
        let private: &[(&str, &str, &str)] = if dev { PRIVATE_FLAGS } else { &[] };
        for (section, set) in [("Beta features", BETA_FLAGS), ("Private features", private)] {
            for (key, title, _) in set {
                push(
                    g.path(),
                    title,
                    &format!("{} · {}", g.title, section),
                    &format!("{section} {title} {key}"),
                    g,
                );
            }
        }
    }
    // A thing named both as a group's sub-page and as a field in
    // `SETTINGS_INDEX` was pushed twice — "Delete site" came back as two
    // identical rows pointing at the same page. The first push wins, so a
    // field keeps the richer `Page · Section` detail it was given there only
    // when no sub-page already claimed the name.
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    out.retain(|e| seen.insert((e.url.clone(), e.title.to_lowercase())));
    out
}

/// The editable social accounts, in Ghost's order. Keys are Ghost's settings
/// keys; the seed keeps Ghost's own defaults (facebook `ghost`, x `@ghost`).
pub(crate) const SOCIAL_ACCOUNTS: &[(&str, &str)] = &[
    ("facebook", "Facebook"),
    ("twitter", "X (Twitter)"),
    ("threads", "Threads"),
    ("bluesky", "Bluesky"),
    ("mastodon", "Mastodon"),
    ("tiktok", "TikTok"),
    ("youtube", "YouTube"),
    ("instagram", "Instagram"),
    ("linkedin", "LinkedIn"),
];

/// The Ghost release this port tracks. Reported on the About page and
/// stamped into the export file's meta.
///
/// Re-exported from `vuuut-core` rather than declared here: `/members/api/site`
/// publishes the same version to Portal from another crate, and holding two
/// copies is what let them drift from 5.98 and 6.39 apart.
pub use vuuut_core::GHOST_VERSION;

/// Ghost's beta feature flags, as its Beta features tab lists them
/// (Ghost 6.73's `PUBLIC_BETA_FEATURES`). Flag, title, description.
///
/// Stored the way Ghost stores them: keys on the `labs` settings object.
pub(crate) const BETA_FLAGS: &[(&str, &str, &str)] = &[
    (
        "superEditors",
        "Enhanced Editor role (beta)",
        "Allows newly-assigned editors to manage members and comments in addition to regular roles.",
    ),
    (
        "editorExcerpt",
        "Show post excerpt inline",
        "Adds the excerpt input below the post title in the editor.",
    ),
    (
        "additionalPaymentMethods",
        "Additional payment methods",
        "Enable support for CashApp, iDEAL, Bancontact, and others.",
    ),
    (
        "navigationIcons",
        "Navigation icons & visibility",
        "Add icons and member-visibility controls to navigation menu items. Requires theme support to render icons.",
    ),
];

/// Ghost's private feature flags — its Private features tab, which Ghost shows
/// only when `enableDeveloperExperiments` is configured.
///
/// Titles and descriptions are Ghost's own, so a flag reads here as it does
/// there. `automations` sat with the beta flags until 6.73 moved it here —
/// this port's own Automations system is switched on in Settings → Features
/// and has never been gated on the flag, so the move is cosmetic parity.
pub(crate) const PRIVATE_FLAGS: &[(&str, &str, &str)] = &[
    ("automations", "Automations", "Build flows that react to what members do."),
    ("automationsLifecycleTriggers", "Automations lifecycle triggers", "Triggers for upgrades and downgrades."),
    ("stripeAutomaticTax", "Stripe Automatic Tax (private beta)", "Use Stripe Automatic Tax at Stripe Checkout. Needs to be enabled in Stripe"),
    ("importMemberTier", "Import Member Tier", "Enables tier to be specified when importing members"),
    ("csvContentImporter", "CSV Content Importer", "Enables importing posts from CSV files in the Universal Importer"),
    ("admin7Design", "Admin 7 · Design", "Preview the floating Admin 7 sidebar and its navigation layout."),
    ("admin7CustomizeFromViewSite", "Admin 7 · Customize from View site", "Customize the site design from View site, with a live preview. Needs Admin 7 · Design."),
    ("emailUniqueid", "Email Unique ID", "Enables {uniqueid} variable in emails for unique image URLs to bypass ESP image caching"),
    ("themeTranslation", "Updated theme translation (beta)", "Enable theme translation using i18next instead of the old translation package."),
    ("pictureImageFormats", "Picture Element", "Use the HTML picture element to serve modern image formats (AVIF, WebP) with automatic fallbacks"),
    ("getHelperDeduplication", "Get helper deduplication", "Deduplicate identical {{#get}} helper queries within a single request to avoid redundant database calls"),
    ("membersCustomFields", "Member custom fields", "Let admins create and manage custom field definitions for members, and choose which field each Stripe checkout answer is stored in"),
    ("stripeCheckoutCollection", "Stripe checkout collection", "Let admins turn on shipping address, phone number and tax number collection for a tier, asked by Stripe checkout and stored against the member"),
    ("paywallImprovements", "Paywall improvements", "Enables paywall usability, discoverability and email customization improvements"),
    ("machinePayments", "Machine payments", "Let AI agents pay for access to paid-members markdown (.md) URLs via Stripe Machine Payments Protocol"),
    ("apps", "Apps", "Install and manage third-party apps that run on their own servers. Early and incomplete."),
];

/// Whether structured data for LLMs is published.
///
/// Ghost treats this as on unless explicitly `false`, so a site that has never
/// seen the setting is opted in rather than silently opted out.
pub(crate) fn llms_enabled(state: &AppState) -> bool {
    !matches!(state.settings.get("llms_enabled"), Some(serde_json::Value::Bool(false)))
        && state.settings.get_str("llms_enabled").as_deref() != Some("false")
}

/// Read the `labs` settings object, which is a flat map of flag → bool.
pub(crate) fn labs_flags(state: &AppState) -> serde_json::Map<String, serde_json::Value> {
    match state.settings.get("labs") {
        Some(serde_json::Value::Object(m)) => m,
        // Older rows hold the object as a JSON *string*; both shapes read the
        // same here.
        Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => serde_json::Map::new(),
    }
}

pub(crate) fn labs_flag_on(flags: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    flags.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false)
}

/// One switch per flag, posted as part of `form_id`.
///
/// Every switch writes one key on the `labs` settings object, which is how
/// Ghost stores them too — so a flag set here is the same flag Ghost would
/// have set, and reaches a theme as `{{#if labs.<flag>}}`. The hidden `shown`
/// field is what tells the save which flags this render actually offered, so
/// an unchecked box can be told apart from a flag that was never on screen.
pub(crate) fn labs_flag_rows(
    flags: &serde_json::Map<String, serde_json::Value>,
    set: &[(&str, &str, &str)],
    form_id: &str,
) -> String {
    set.iter()
        .map(|(key, title, detail)| {
            format!(
                r#"<div class="row toggle-row"><span class="ic tint-teal">{ic}</span>
<div class="label"><b>{title}</b><span>{detail}</span></div>
<input type="hidden" name="shown" value="{key}" form="{form}">
<label class="switch"><input type="checkbox" name="{key}" value="1"{on} form="{form}"><span class="track"><span class="thumb"></span></span></label></div>"#,
                ic = svg("star"),
                key = esc(key),
                form = esc(form_id),
                title = esc(title),
                detail = esc(detail),
                on = if labs_flag_on(flags, key) { " checked" } else { "" },
            )
        })
        .collect()
}

/// The words Ghost builds a private-site access code from
/// (server/services/settings/private-site-access-code.js). Short and
/// sayable, because this code gets shared by hand.
pub(crate) const ACCESS_CODE_WORDS: &[&str] = &[
    "anchor", "aurora", "beacon", "birch", "bright", "cedar", "cloud", "comet", "copper", "coral",
    "ember", "fern", "field", "forest", "golden", "green", "harbor", "hidden", "horizon",
    "juniper", "lagoon", "lunar", "maple", "meadow", "midnight", "north", "ocean", "olive",
    "paper", "pine", "quiet", "river", "sage", "signal", "silver", "solstice", "sparrow", "stone",
    "studio", "summit", "sunrise", "thistle", "valley", "violet", "willow", "window", "winter",
    "wild",
];

/// Ghost's `word###` access code, off the same CSPRNG the rest of the port
/// mints secrets with.
pub(crate) fn generate_access_code() -> String {
    let word = ACCESS_CODE_WORDS
        [vuuut_core::ids::random_below(ACCESS_CODE_WORDS.len() as u32) as usize];
    format!("{word}{:03}", vuuut_core::ids::random_below(1000))
}

/// Currencies offered for tips and donations. Ghost stores
/// `donations_currency` as an upper-case ISO code, unlike a tier's price
/// currency, which it keeps lower-case.
pub(crate) const CURRENCIES: &[(&str, &str)] = &[
    ("USD", "USD $"),
    ("EUR", "EUR €"),
    ("GBP", "GBP £"),
    ("AUD", "AUD $"),
    ("CAD", "CAD $"),
    ("INR", "INR ₹"),
];

/// Ghost's timezone picker (moment-timezone names with GMT offsets), west →
/// east. A stored value outside the list is still shown as a raw option.
pub(crate) const TIMEZONES: &[(&str, &str)] = &[
    ("Etc/GMT+12", "(GMT-12:00) GMT+12"),
    ("Pacific/Honolulu", "(GMT-10:00) Hawaii"),
    ("America/Anchorage", "(GMT-09:00) Alaska"),
    ("America/Los_Angeles", "(GMT-08:00) Pacific Time (US & Canada)"),
    ("America/Vancouver", "(GMT-08:00) Vancouver"),
    ("America/Denver", "(GMT-07:00) Mountain Time (US & Canada)"),
    ("America/Phoenix", "(GMT-07:00) Arizona"),
    ("America/Chicago", "(GMT-06:00) Central Time (US & Canada)"),
    ("America/Mexico_City", "(GMT-06:00) Mexico City"),
    ("America/Bogota", "(GMT-05:00) Bogotá"),
    ("America/Lima", "(GMT-05:00) Lima"),
    ("America/New_York", "(GMT-05:00) Eastern Time (US & Canada)"),
    ("America/Toronto", "(GMT-05:00) Toronto"),
    ("America/Halifax", "(GMT-04:00) Halifax"),
    ("America/Santiago", "(GMT-04:00) Santiago"),
    ("America/Argentina/Buenos_Aires", "(GMT-03:00) Buenos Aires"),
    ("America/Sao_Paulo", "(GMT-03:00) São Paulo"),
    ("Atlantic/Reykjavik", "(GMT+00:00) Reykjavik"),
    ("Europe/London", "(GMT+00:00) London"),
    ("Europe/Lisbon", "(GMT+00:00) Lisbon"),
    ("Etc/UTC", "(GMT+00:00) UTC"),
    ("UTC", "(GMT+00:00) UTC"),
    ("Europe/Dublin", "(GMT+01:00) Dublin"),
    ("Europe/Amsterdam", "(GMT+01:00) Amsterdam"),
    ("Europe/Berlin", "(GMT+01:00) Berlin"),
    ("Europe/Brussels", "(GMT+01:00) Brussels"),
    ("Europe/Madrid", "(GMT+01:00) Madrid"),
    ("Europe/Paris", "(GMT+01:00) Paris"),
    ("Europe/Rome", "(GMT+01:00) Rome"),
    ("Europe/Stockholm", "(GMT+01:00) Stockholm"),
    ("Europe/Vienna", "(GMT+01:00) Vienna"),
    ("Europe/Warsaw", "(GMT+01:00) Warsaw"),
    ("Europe/Zurich", "(GMT+01:00) Zurich"),
    ("Africa/Lagos", "(GMT+01:00) Lagos"),
    ("Europe/Athens", "(GMT+02:00) Athens"),
    ("Europe/Helsinki", "(GMT+02:00) Helsinki"),
    ("Europe/Kiev", "(GMT+02:00) Kyiv"),
    ("Africa/Cairo", "(GMT+02:00) Cairo"),
    ("Africa/Johannesburg", "(GMT+02:00) Johannesburg"),
    ("Europe/Istanbul", "(GMT+03:00) Istanbul"),
    ("Europe/Moscow", "(GMT+03:00) Moscow"),
    ("Africa/Nairobi", "(GMT+03:00) Nairobi"),
    ("Asia/Riyadh", "(GMT+03:00) Riyadh"),
    ("Asia/Tehran", "(GMT+03:30) Tehran"),
    ("Asia/Dubai", "(GMT+04:00) Dubai"),
    ("Asia/Karachi", "(GMT+05:00) Karachi"),
    ("Asia/Kolkata", "(GMT+05:30) Kolkata"),
    ("Asia/Kathmandu", "(GMT+05:45) Kathmandu"),
    ("Asia/Dhaka", "(GMT+06:00) Dhaka"),
    ("Asia/Yangon", "(GMT+06:30) Yangon"),
    ("Asia/Bangkok", "(GMT+07:00) Bangkok"),
    ("Asia/Jakarta", "(GMT+07:00) Jakarta"),
    ("Asia/Shanghai", "(GMT+08:00) Shanghai"),
    ("Asia/Hong_Kong", "(GMT+08:00) Hong Kong"),
    ("Asia/Kuala_Lumpur", "(GMT+08:00) Kuala Lumpur"),
    ("Asia/Manila", "(GMT+08:00) Manila"),
    ("Asia/Singapore", "(GMT+08:00) Singapore"),
    ("Asia/Taipei", "(GMT+08:00) Taipei"),
    ("Asia/Seoul", "(GMT+09:00) Seoul"),
    ("Asia/Tokyo", "(GMT+09:00) Tokyo"),
    ("Australia/Adelaide", "(GMT+09:30) Adelaide"),
    ("Australia/Brisbane", "(GMT+10:00) Brisbane"),
    ("Australia/Melbourne", "(GMT+10:00) Melbourne"),
    ("Australia/Sydney", "(GMT+10:00) Sydney"),
    ("Australia/Perth", "(GMT+08:00) Perth"),
    ("Pacific/Auckland", "(GMT+12:00) Auckland"),
    ("Pacific/Fiji", "(GMT+12:00) Fiji"),
];

/// This site's languages, as `vuuut_core::locales` understands them.
///
/// One place so that the editor, the tag screen, the Languages module and
/// the settings page cannot disagree about which languages exist. Reads the
/// two settings the Multilingual module owns and nothing else.
pub(crate) fn site_locales(state: &AppState) -> vuuut_core::locales::Locales {
    vuuut_core::locales::Locales::from_settings(
        state.settings.get_str("locale").as_deref(),
        state.settings.get("locales_enabled").as_ref(),
    )
}

/// The human label for a language code — `de` → `Deutsch (German)`.
///
/// Falls back to the code itself, which is what a site using a locale this
/// list does not cover should see rather than a blank.
pub(crate) fn locale_label(code: &str) -> String {
    LOCALES
        .iter()
        .find(|(value, _)| value.eq_ignore_ascii_case(code))
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| code.to_string())
}

/// A `<select>` of this site's languages.
///
/// `blank` labels the "no particular language" option when one is wanted —
/// a tag with no locale belongs to every language — and is omitted entirely
/// when it is `None`, which is how a post's own picker reads: an entry is
/// always in exactly one language.
pub(crate) fn locale_options(
    locales: &vuuut_core::locales::Locales,
    current: &str,
    blank: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(label) = blank {
        out.push_str(&format!(
            r#"<option value=""{sel}>{label}</option>"#,
            sel = if current.trim().is_empty() { " selected" } else { "" },
            label = esc(label),
        ));
    }
    for code in locales.enabled() {
        let selected = code.eq_ignore_ascii_case(current.trim());
        out.push_str(&format!(
            r#"<option value="{code}"{sel}>{label}{default}</option>"#,
            code = esc(code),
            sel = if selected { " selected" } else { "" },
            label = esc(&locale_label(code)),
            default = if locales.is_default(code) { " · default" } else { "" },
        ));
    }
    out
}

/// The language table lives in `vuuut_core::locales` so the front end's
/// language switcher and this picker name languages identically.
pub(crate) const LOCALES: &[(&str, &str)] = vuuut_core::locales::LANGUAGES;
/// `<option>` list for a picker, marking `current`; a stored value outside
/// the list (e.g. a locale this list doesn't cover) still shows up raw.
pub(crate) fn select_options(list: &[(&str, &str)], current: &str) -> String {
    let opts: String = list
        .iter()
        .map(|(value, label)| {
            let selected = if *value == current { " selected" } else { "" };
            format!(r#"<option value="{value}"{selected}>{label}</option>"#, value = esc(value), selected = selected, label = esc(label))
        })
        .collect();
    if !current.is_empty() && !list.iter().any(|(v, _)| *v == current) {
        format!(r#"<option value="{c}" selected>{c}</option>{opts}"#, c = esc(current), opts = opts)
    } else {
        opts
    }
}

/// Dropdown caret used inside `.field select` rows.
pub(crate) const SELECT_CARET: &str = r#"<svg class="car" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 10l5 5 5-5z"/></svg>"#;

/// An M3 segmented button: the connected-chips control for a setting with a
/// handful of options, where every choice deserves to be visible at once.
/// Radios post exactly like the select they replace.
pub(crate) fn segmented(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let segs: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                r#"<label><input type="radio" name="{name}" value="{value}"{ck}><span class="ck">{check}</span><span>{label}</span></label>"#,
                name = esc(name),
                value = esc(value),
                ck = if *value == current { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    format!(r#"<div class="seg" role="radiogroup">{segs}</div>"#)
}

/// A compact dropdown for a settings row's trailing position, for option
/// lists too long for a segmented button.
///
/// `label` is the row's own title: the control has no visible label of its
/// own, so without it a screen reader announces the select by its current
/// value — "Bold" rather than "Title weight".
pub(crate) fn selbox(name: &str, label: &str, options: &[(&str, &str)], current: &str) -> String {
    format!(
        r#"<div class="selbox"><select name="{name}" aria-label="{label}">{opts}</select>{car}</div>"#,
        name = esc(name),
        label = esc(label),
        opts = select_options(options, current),
        car = SELECT_CARET,
    )
}

/// The text counterpart of [`selbox`], for a value that is typed rather than
/// chosen.
pub(crate) fn textbox(name: &str, label: &str, value: &str) -> String {
    format!(
        r#"<div class="textbox"><input name="{name}" value="{value}" aria-label="{label}"></div>"#,
        name = esc(name),
        value = esc(value),
        label = esc(label),
    )
}

/// Redirect back to a settings group page carrying a one-shot banner.
/// A settings flash, or the same message as JSON for an API client.
///
pub(crate) fn settings_flash(group: &str, kind: &str, message: &str) -> Response {
    let path = SETTINGS_GROUPS.iter().find(|g| g.slug == group)
        .map_or_else(|| format!("/ghost/settings/{group}/"), SettingsGroup::path);
    crate::tools::tool_flash(&path, kind, message)
}

/// The one-shot ok/error banner carried in the query string.
pub(crate) fn flash_banner(q: &HashMap<String, String>) -> String {
    match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    }
}

/// 404 for a caller that has no role to hand; renders the minimal rail.
pub(crate) fn not_found_page(user: &vuuut_core::models::User, message: &str) -> Response {
    not_found_page_for(Role::Contributor, user, message)
}

/// A 404 rendered inside the admin shell, for a URL that is well-formed but
/// names something that isn't there (or that this user may not see — the two
/// are deliberately indistinguishable, so the page can't be used to probe
/// which staff accounts exist).
pub(crate) fn not_found_page_for(role: Role, user: &vuuut_core::models::User, message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        page_for(
            "Not found",
            "",
            Some(user),
            role,
            &format!(
                r#"<header class="page-head rise"><div><h1>Not found</h1><p class="sub">{msg}</p></div></header>
<div class="card padded rise" style="--i:1;text-align:center">
<p class="muted" style="margin:8px 0 16px">{msg}</p>
<a class="btn tonal" href="/ghost/settings/">Back to settings</a></div>"#,
                msg = esc(message),
            ),
            true,
        ),
    )
        .into_response()
}

/// Shared header for a settings group page with a link back to its tool or
/// the settings index, and an optional Save button
/// bound to the page's form.
pub(crate) fn group_head(g: &SettingsGroup, form_id: Option<&str>) -> String {
    let save = match form_id {
        Some(id) => format!(
            r#"<button class="btn" type="submit" form="{id}">{check} Save</button>"#,
            id = esc(id),
            check = svg("check"),
        ),
        None => String::new(),
    };
    let (back_href, back_title, title) = match g.tool {
        Some(tool) => (
            format!("/ghost/{tool}/"),
            format!("Back to {}", crate::tools::tool(tool).title),
            format!("{} settings", g.title),
        ),
        None => ("/ghost/settings/".into(), "Back to settings".into(), g.title.into()),
    };
    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="{back_href}" title="{back_title}">{back}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{save}</div></header>"#,
        back = svg("chevron"),
        back_title = esc(&back_title),
        title = esc(&title),
        sub = esc(&g.items.join(" · ")),
        save = save,
    )
}

/// A settings row that isn't built yet (kept visible so the hub shows
/// Ghost's full structure while pages are completed one by one).

pub(crate) async fn settings_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Only the groups this viewer may open. The hub itself is reachable when
    // any one of them is; the rows it lists are each checked again when they
    // are followed.
    let grants = crate::permissions::grants_for(&state, actor_role);
    let rows: String = SETTINGS_GROUPS
        .iter()
        .filter(|g| g.tool.is_none() && grants.settings.access(g.slug).may_read())
        .map(|g| {
            let head = format!(
                r#"<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>"#,
                tint = g.tint,
                ic = svg(g.icon),
                title = esc(g.title),
                detail = esc(&g.items.join(" · ")),
            );
            format!(
                r#"<a class="row" href="{path}">{head}
<div class="trailing">{chev}</div></a>"#,
                path = g.path(),
                head = head,
                chev = svg("chevron"),
            )
        })
        .collect();
    // The searchable index, four kinds deep: the pages themselves, the
    // sub-page each group lists, the nested places and headings, and the
    // fields. A page's row answers to its own name and its sub-pages' names,
    // but everything below it matches only its own text — so a broad query
    // ("advanced") lands on the page, and a narrow one ("port", "zapier",
    // "built-in") lands on the thing itself without flooding the page in.
    let hits = settings_entries(state.config.env == "development", None)
        .iter()
        .filter(|e| grants.settings.access(e.group).may_read())
        .map(|e| {
            format!(
                r#"<a class="row" href="{url}" data-search="{haystack}">
<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{chev}</div></a>"#,
                url = esc(&e.url),
                haystack = esc(&e.haystack),
                tint = e.tint,
                ic = svg(e.icon),
                title = esc(&e.title),
                detail = esc(&e.detail),
                chev = svg("chevron"),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    page_for_role(actor_role,
        "Settings",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Settings</h1><p class="sub">Site configuration</p></div></header>
<div class="search rise" style="--i:1">{search}<input id="settings-search" type="search" placeholder="Search all settings" aria-label="Search all settings"></div>
<div class="card rows rise" style="--i:2" id="settings-groups">{rows}</div>
<div class="card rows" id="settings-hits" style="display:none">{hits}
<div class="row" id="settings-empty" style="display:none"><div class="body"><span class="title">Nothing matches</span><span class="detail">Try another word, or clear the search to browse the pages.</span></div></div>
</div>"#,
            search = svg("search"),
            rows = rows,
            hits = hits,
        ),
        true,
    )
    .into_response()
}
