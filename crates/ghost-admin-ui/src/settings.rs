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

pub(crate) use general::*;
pub(crate) use email::*;
pub(crate) use newsletters::*;
pub(crate) use features::*;
pub(crate) use membership::*;
pub(crate) use growth::*;
pub(crate) use advanced::*;

// ---------------------------------------------------------------------------
// settings hub (Ghost's settings navigation. Every group has a page; the
// only deferred items are Growth → Network and Growth → Ghost Explore.)
// ---------------------------------------------------------------------------

/// One group in the settings navigation, in Ghost's sidebar order.
pub(crate) struct SettingsGroup {
    pub(crate) slug: &'static str,
    pub(crate) title: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) tint: &'static str,
    /// Sub-pages inside the group, in Ghost's order (shown as the row detail).
    pub(crate) items: &'static [&'static str],
}

impl SettingsGroup {
    /// Where this group's row points.
    ///
    /// Most groups are a page under `/ghost/settings/`. Design and Navigation
    /// keep the top-level pages they already had and are linked from the hub
    /// rather than copied under it, so their URLs — and any bookmark or link
    /// into them — keep working.
    pub(crate) fn path(&self) -> String {
        match self.slug {
            "design" => "/ghost/design/".to_string(),
            "navigation" => "/ghost/navigation/".to_string(),
            other => format!("/ghost/settings/{other}/"),
        }
    }
}

pub(crate) const SETTINGS_GROUPS: &[SettingsGroup] = &[
    SettingsGroup { slug: "general", title: "General settings", icon: "settings", tint: "pink", items: &["Name, title & description", "Timezone", "Publication language", "Meta data", "Social accounts"] },
    SettingsGroup { slug: "staff", title: "Staff", icon: "ghost", tint: "teal", items: &["People", "Roles", "Invitations"] },
    SettingsGroup { slug: "analytics", title: "Analytics", icon: "chart", tint: "green", items: &["Web analytics", "Email opens", "Email clicks", "Member sources", "Outbound link tagging"] },
    SettingsGroup { slug: "design", title: "Design & branding", icon: "tune", tint: "purple", items: &["Brand", "Site-wide colors", "Theme settings"] },
    SettingsGroup { slug: "navigation", title: "Navigation", icon: "menu", tint: "red", items: &["Primary menu", "Secondary menu"] },
    SettingsGroup { slug: "code-injection", title: "Code injection", icon: "code", tint: "purple", items: &["Site head", "Site foot"] },
    SettingsGroup { slug: "site", title: "Announcement", icon: "flag", tint: "amber", items: &["Announcement bar", "Background", "Visibility"] },
    SettingsGroup { slug: "membership", title: "Membership", icon: "ghost", tint: "teal", items: &["Access", "Spam filters", "Tiers", "Signup portal", "Gift subscriptions", "Tips & donations", "Custom fields"] },
    // This port's own feature, so it has no Ghost sidebar slot to mirror; it
    // sits with Membership because it is the other thing readers (and agents)
    // can be charged for.
    SettingsGroup { slug: "machine-payments", title: "Machine payments", icon: "integrations", tint: "pink", items: &["Access", "Pricing", "Stripe cards & Link", "Stablecoins", "Payment activity"] },
    SettingsGroup { slug: "email", title: "Email", icon: "mail", tint: "blue", items: &["Transactional email", "SMTP server", "Sender identity", "Send a test"] },
    SettingsGroup { slug: "newsletters", title: "Newsletters", icon: "send", tint: "purple", items: &["Newsletter sending", "Default recipients", "Amazon SES", "Mailgun", "Region"] },
    SettingsGroup { slug: "growth", title: "Growth", icon: "globe", tint: "slate", items: &["Network", "Ghost Explore", "Recommendations", "Signup forms", "Offers"] },
    SettingsGroup { slug: "features", title: "Features", icon: "tune", tint: "teal", items: &["Comments", "Automations", "Beta features", "Private features"] },
    // This port's own module. Its builder and responses live under Modules;
    // what is here is what applies to every form at once.
    SettingsGroup { slug: "forms", title: "Forms", icon: "form", tint: "amber", items: &["Notifications", "Spam protection", "Privacy & retention"] },
    SettingsGroup { slug: "integrations", title: "Integrations", icon: "integrations", tint: "red", items: &["Built-in apps", "Custom integrations", "Content API"] },
    SettingsGroup { slug: "migration", title: "Migration tools", icon: "import", tint: "blue", items: &["Import content", "Export all data", "Members", "Post analytics", "Themes"] },
    SettingsGroup { slug: "advanced", title: "Advanced", icon: "code", tint: "red", items: &["Redirects & routes"] },
    SettingsGroup { slug: "history", title: "History", icon: "history", tint: "slate", items: &["Site activity"] },
    SettingsGroup { slug: "danger", title: "Danger zone", icon: "delete", tint: "red", items: &["Delete all content", "Delete site"] },
];

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
    ("general", "Publication language", "Publication language"),
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
    ("analytics", "Analytics", "Web analytics"),
    ("analytics", "Analytics", "Email opens"),
    ("analytics", "Analytics", "Email clicks"),
    ("analytics", "Analytics", "Member sources"),
    ("analytics", "Analytics", "Outbound link tagging"),
    ("site", "Announcement bar", "Announcement"),
    ("site", "Announcement bar", "Background"),
    ("site", "Announcement bar", "Visibility"),
    ("design", "Brand", "Accent color"),
    ("design", "Brand", "Publication icon"),
    ("design", "Brand", "Publication logo"),
    ("design", "Brand", "Publication cover"),
    ("navigation", "Primary menu", "Primary navigation"),
    ("navigation", "Secondary menu", "Secondary navigation"),
    ("email", "Transactional email", "Transport"),
    ("email", "SMTP server", "Host"),
    ("email", "SMTP server", "Port"),
    ("email", "SMTP server", "Encryption"),
    ("email", "SMTP server", "Username"),
    ("email", "SMTP server", "Password"),
    ("email", "Sender identity", "From address"),
    ("email", "Sender identity", "From name"),
    ("email", "Sender identity", "Reply-to"),
    ("email", "Send a test", "Send to"),
    ("newsletters", "Newsletters", "Newsletter name"),
    ("newsletters", "Mailgun", "API key"),
    ("newsletters", "Mailgun", "Sending domain"),
    ("newsletters", "Mailgun", "Region"),
    ("newsletters", "Sender", "Sender name"),
    ("newsletters", "Sender", "Sender email"),
    ("newsletters", "Header", "Header image URL"),
    ("membership", "Stripe", "Publishable key"),
    ("membership", "Stripe", "Secret key"),
    ("membership", "Stripe", "Webhook signing secret"),
    ("membership", "Access", "Signup access"),
    ("membership", "Access", "Default post access"),
    ("membership", "Access", "Commenting"),
    ("membership", "Tiers", "Monthly price"),
    ("membership", "Tiers", "Yearly price"),
    ("membership", "Tiers", "Currency"),
    ("membership", "Tiers", "Free trial"),
    ("membership", "Tiers", "Benefits"),
    ("membership", "Signup portal", "Portal button signup text"),
    ("membership", "Signup portal", "Portal button icon"),
    ("membership", "Signup portal", "Portal button style"),
    ("membership", "Signup portal", "Signup terms"),
    ("membership", "Welcome emails", "Welcome page URL"),
    ("membership", "Access", "Site access"),
    ("membership", "Access", "Access code"),
    ("membership", "Spam filters", "Blocked email domains"),
    ("membership", "Custom fields", "Field name"),
    ("membership", "Custom fields", "Member access"),
    ("machine-payments", "Access", "Paid Markdown access"),
    ("machine-payments", "Pricing", "Price per article"),
    ("machine-payments", "Pricing", "Currency"),
    ("machine-payments", "Stripe cards & Link", "Stripe business network profile ID"),
    ("machine-payments", "Stablecoins", "Tempo USDC"),
    ("machine-payments", "Stablecoins", "Base USDC"),
    ("machine-payments", "Stablecoins", "Settlement network"),
    ("machine-payments", "Stablecoins", "Deposit addresses"),
    ("machine-payments", "Payment activity", "Recent payment attempts"),
    ("growth", "Offers", "Discount code"),
    ("growth", "Offers", "Free trial"),
    ("membership", "Gift subscriptions", "Shareable link"),
    ("membership", "Tips & donations", "Currency"),
    ("membership", "Tips & donations", "Suggested amount"),
    ("newsletters", "Newsletter sending", "Newsletter sending"),
    ("newsletters", "Default recipients", "Who gets a new post by default"),
    ("features", "Comments", "New comments"),
    ("features", "Comments", "Hold for review"),
    ("features", "Automations", "Automations"),
    ("features", "Beta features", "Feature flags"),
    ("features", "Private features", "Developer experiments"),
    ("code-injection", "Site head", "Head"),
    ("code-injection", "Site foot", "Foot"),
    ("advanced", "Redirects & routes", "Redirects"),
    ("advanced", "Redirects & routes", "Routes"),
    // What someone actually types when they are moving a site in or out.
    ("migration", "Import", "Import content"),
    ("migration", "Import", "Universal import"),
    ("migration", "Import", "Ghost export file"),
    ("migration", "Import", "WordPress"),
    ("migration", "Import", "Substack"),
    ("migration", "Import", "CSV"),
    ("migration", "Import", "Migrate from another platform"),
    ("migration", "Export", "Export all data"),
    ("migration", "Export", "Content & settings"),
    ("migration", "Export", "Members"),
    ("migration", "Export", "Post analytics"),
    ("migration", "Export", "Themes"),
    ("migration", "Export", "Media files"),
    ("migration", "Export", "Backup"),
];

/// Sub-pages and on-page headings the group rows do not already carry:
/// things with their own URL nested inside a group (the built-in apps
/// Integrations lists), and headings that are neither the page's own
/// name nor one of its listed sub-pages. Each row links straight to where
/// the thing actually lives.
pub(crate) const SETTINGS_PLACES: &[(&str, &str, &str)] = &[
    // (path under /ghost/settings/, what you are looking for, parent group slug)
    ("integrations/zapier/", "Zapier", "integrations"),
    ("integrations/slack/", "Slack", "integrations"),
    ("integrations/unsplash/", "Unsplash", "integrations"),
    ("membership/", "Stripe", "membership"),
    ("growth/", "Coming later", "growth"),
];

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
/// Re-exported from `ghost-core` rather than declared here: `/members/api/site`
/// publishes the same version to Portal from another crate, and holding two
/// copies is what let them drift from 5.98 and 6.39 apart.
pub use ghost_core::GHOST_VERSION;

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
        [ghost_core::ids::random_below(ACCESS_CODE_WORDS.len() as u32) as usize];
    format!("{word}{:03}", ghost_core::ids::random_below(1000))
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

/// Ghost's publication language picker: the setting stores the `locale`
/// code (Ghost has no separate `lang` setting).
///
/// Codes and coverage come from Ghost's own list, which matters beyond the
/// label: a theme's translations live in `<theme>/locales/<code>.json`, so a
/// locale saved under a code Ghost does not use (`zh-cn` rather than `zh`)
/// silently finds no translation file. Labels lead with the language's own
/// name, because that is what someone scans for when picking their own.
pub(crate) const LOCALES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("ar", "العربية (Arabic)"),
    ("az", "Azərbaycan (Azerbaijani)"),
    ("bg", "Български (Bulgarian)"),
    ("bn", "বাংলা (Bengali)"),
    ("bs", "Bosanski (Bosnian)"),
    ("ca", "Català (Catalan)"),
    ("cs", "Čeština (Czech)"),
    ("da", "Dansk (Danish)"),
    ("de", "Deutsch (German)"),
    ("de-CH", "Schweizerdeutsch (Swiss German)"),
    ("el", "Ελληνικά (Greek)"),
    ("en", "English"),
    ("eo", "Esperanto"),
    ("es", "Español (Spanish)"),
    ("et", "Eesti (Estonian)"),
    ("eu", "Euskara (Basque)"),
    ("fa", "فارسی (Persian/Farsi)"),
    ("fi", "Suomi (Finnish)"),
    ("fr", "Français (French)"),
    ("gd", "Gàidhlig (Gaelic (Scottish))"),
    ("he", "עברית (Hebrew)"),
    ("hi", "हिन्दी (Hindi)"),
    ("hr", "Hrvatski (Croatian)"),
    ("hu", "Magyar (Hungarian)"),
    ("id", "Bahasa Indonesia (Indonesian)"),
    ("is", "Íslenska (Icelandic)"),
    ("it", "Italiano (Italian)"),
    ("ja", "日本語 (Japanese)"),
    ("ko", "한국어 (Korean)"),
    ("kz", "Қазақша (Kazakh)"),
    ("lt", "Lietuvių (Lithuanian)"),
    ("lv", "Latviešu (Latvian)"),
    ("mk", "Македонски (Macedonian)"),
    ("mn", "Монгол (Mongolian)"),
    ("ms", "Bahasa Melayu (Malay)"),
    ("nb", "Norsk bokmål (Norwegian Bokmål)"),
    ("ne", "नेपाली (Nepali)"),
    ("nl", "Nederlands (Dutch)"),
    ("nn", "Norsk nynorsk (Norwegian Nynorsk)"),
    ("pa", "ਪੰਜਾਬੀ (Punjabi)"),
    ("pl", "Polski (Polish)"),
    ("pt", "Português (Portuguese)"),
    ("pt-BR", "Português (Brasil) (Portuguese (Brazil))"),
    ("ro", "Română (Romanian)"),
    ("ru", "Русский (Russian)"),
    ("si", "සිංහල (Sinhala)"),
    ("sk", "Slovenčina (Slovak)"),
    ("sl", "Slovenščina (Slovenian)"),
    ("sq", "Shqip (Albanian)"),
    ("sr", "Srpski (Serbian)"),
    ("sr-Cyrl", "Српски (Serbian (Cyrillic))"),
    ("sv", "Svenska (Swedish)"),
    ("sw", "Kiswahili (Swahili)"),
    ("ta", "தமிழ் (Tamil)"),
    ("th", "ไทย (Thai)"),
    ("tr", "Türkçe (Turkish)"),
    ("uk", "Українська (Ukrainian)"),
    ("ur", "اردو (Urdu)"),
    ("uz", "Oʻzbekcha (Uzbek)"),
    ("vi", "Tiếng Việt (Vietnamese)"),
    ("zh", "中文(简体) (Chinese)"),
    ("zh-Hant", "中文(繁體) (Traditional Chinese)"),
];
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
/// The four file-handling handlers below are reached two ways: from the
/// admin's own forms, which want a redirect and a banner, and from the Admin
/// API, where a redirect to an HTML page is useless. One handler either way
/// means the validation and the writing cannot drift between them.
pub(crate) fn flash(headers: &HeaderMap, group: &str, kind: &str, message: &str) -> Response {
    if !wants_json(headers) {
        return settings_flash(group, kind, message);
    }
    let status = if kind == "error" { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK };
    if kind == "error" {
        (
            status,
            axum::Json(serde_json::json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
        )
            .into_response()
    } else {
        (status, axum::Json(serde_json::json!({"meta":{"message":message}}))).into_response()
    }
}

pub(crate) fn settings_flash(group: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/{group}/?{encoded}")).into_response()
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
pub(crate) fn not_found_page(user: &ghost_core::models::User, message: &str) -> Response {
    not_found_page_for(Role::Contributor, user, message)
}

/// A 404 rendered inside the admin shell, for a URL that is well-formed but
/// names something that isn't there (or that this user may not see — the two
/// are deliberately indistinguishable, so the page can't be used to probe
/// which staff accounts exist).
pub(crate) fn not_found_page_for(role: Role, user: &ghost_core::models::User, message: &str) -> Response {
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

/// Shared header for a settings group page: back to the settings index, the
/// group title with its sub-pages as subtitle, and an optional Save button
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
    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{save}</div></header>"#,
        back = svg("chevron"),
        title = esc(g.title),
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
    let rows: String = SETTINGS_GROUPS
        .iter()
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
    let mut hit_rows: Vec<String> = Vec::new();
    let mut push_hit = |url: String, title: &str, detail: &str, haystack: &str, group: &SettingsGroup| {
        hit_rows.push(format!(
            r#"<a class="row" href="{url}" data-search="{haystack}">
<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{chev}</div></a>"#,
            url = esc(&url),
            haystack = esc(haystack),
            tint = group.tint,
            ic = svg(group.icon),
            title = esc(title),
            detail = esc(detail),
            chev = svg("chevron"),
        ));
    };
    for g in SETTINGS_GROUPS {
        push_hit(
            g.path(),
            g.title,
            &g.items.join(" · "),
            &format!("{} {} {}", g.title, g.items.join(" "), g.slug),
            g,
        );
        for item in g.items {
            push_hit(g.path(), item, g.title, item, g);
        }
    }
    for (path, name, parent) in SETTINGS_PLACES {
        if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == *parent) {
            push_hit(format!("/ghost/settings/{path}"), name, g.title, name, g);
        }
    }
    let mut fields: Vec<(&str, &str, &str)> = SETTINGS_INDEX.to_vec();
    for (_, name) in SOCIAL_ACCOUNTS {
        fields.push(("general", "Social accounts", name));
    }
    for (slug, section, field) in &fields {
        if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == *slug) {
            push_hit(
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
    //
    // The private flags are indexed only where the Features page renders
    // them: on a production site, a hit would lead to a row that is not
    // there.
    if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == "features") {
        let private: &[(&str, &str, &str)] =
            if state.config.env == "development" { PRIVATE_FLAGS } else { &[] };
        for (section, set) in [("Beta features", BETA_FLAGS), ("Private features", private)] {
            for (key, title, _) in set {
                push_hit(
                    g.path(),
                    title,
                    &format!("{} · {}", g.title, section),
                    &format!("{section} {title} {key}"),
                    g,
                );
            }
        }
    }
    let hits = hit_rows.join("\n");
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
