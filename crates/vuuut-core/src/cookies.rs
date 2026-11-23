//! Cookie consent & privacy compliance: the rules.
//!
//! This is the trust boundary for the module. Everything here is pure — no
//! database, no HTTP — so the admin, the public endpoint and the renderer all
//! ask the same functions, and every one of them is unit-tested below.
//!
//! What lives here, and why each part is a *rule* rather than a preference:
//!
//! * **[`Settings`]** — the banner's configuration, read through one function
//!   that treats a missing row as the default. The default is **off**, so
//!   installing the module changes nothing a visitor sees until somebody
//!   switches it on (the SEO line, not the Security one: a consent banner is
//!   a promise the site owner has to choose to make, and the wording of it is
//!   theirs).
//! * **[`TRACKERS`]** — a signature library of the third-party scripts and
//!   embeds that set cookies. The admin's scan and the renderer's blocking
//!   both read it, so what the report says is blocked is what is blocked.
//! * **[`gate_html`]** — the server-side half of blocking. A recognised
//!   tracker's `<script>` is rewritten to `type="text/plain"` before the page
//!   leaves the server, so it cannot run until the visitor's browser script
//!   re-enables it after consent. Doing this in the browser instead would be
//!   too late: by the time any script of ours runs, a tracker earlier in the
//!   document has already set its cookie.
//! * **[`Choice`]** — a visitor's decision, rebuilt from scratch out of what
//!   their browser sent. Unknown categories are dropped, `necessary` is always
//!   granted, and the action is one of five known words.
//! * **[`audit`]** — the compliance report: what is wrong, in order of how
//!   much it exposes the site.

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// categories
// ---------------------------------------------------------------------------

/// The four purposes a cookie can serve, in the order a banner lists them.
///
/// Four rather than the dozen some consent managers offer: the ePrivacy
/// Directive's line is "strictly necessary or not", and every regulator's
/// guidance groups the rest into roughly these three. More categories means
/// more switches for a reader to refuse, which is a dark pattern of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Necessary,
    Preferences,
    Analytics,
    Marketing,
}

impl Category {
    pub const ALL: [Category; 4] =
        [Category::Necessary, Category::Preferences, Category::Analytics, Category::Marketing];
    /// The three a visitor may refuse.
    pub const OPTIONAL: [Category; 3] = [Category::Preferences, Category::Analytics, Category::Marketing];

    pub fn key(self) -> &'static str {
        match self {
            Category::Necessary => "necessary",
            Category::Preferences => "preferences",
            Category::Analytics => "analytics",
            Category::Marketing => "marketing",
        }
    }
    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.key() == key.trim())
    }
    pub fn label(self) -> &'static str {
        match self {
            Category::Necessary => "Strictly necessary",
            Category::Preferences => "Preferences",
            Category::Analytics => "Analytics",
            Category::Marketing => "Marketing",
        }
    }
    /// What a visitor is told the category is for. Plain, and specific about
    /// what refusing it costs — "improve your experience" says nothing.
    pub fn description(self) -> &'static str {
        match self {
            Category::Necessary => "Keep you signed in, remember this choice, and protect forms from abuse. The site does not work without them, so they cannot be switched off.",
            Category::Preferences => "Remember settings you choose, such as which messages you have already dismissed.",
            Category::Analytics => "Count visits and see which pages are read, so the site can be improved. No advertising.",
            Category::Marketing => "Used by advertising and social networks to show you ads elsewhere, and by embedded videos and posts from other sites.",
        }
    }
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// How the site asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// GDPR / UK GDPR / ePrivacy: nothing optional runs until the visitor
    /// agrees. The only mode that is lawful for marketing cookies in Europe.
    OptIn,
    /// CCPA / CPRA and similar US state laws: everything runs, and the
    /// visitor can opt out of sale and sharing — Global Privacy Control is an
    /// opt-out by law in California.
    OptOut,
    /// Inform only. For a site whose only cookies are necessary or
    /// cookieless; the report says so if that stops being true.
    Notice,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::OptIn, Mode::OptOut, Mode::Notice];
    pub fn key(self) -> &'static str {
        match self {
            Mode::OptIn => "opt_in",
            Mode::OptOut => "opt_out",
            Mode::Notice => "notice",
        }
    }
    pub fn from_key(key: &str) -> Mode {
        Mode::ALL.into_iter().find(|m| m.key() == key.trim()).unwrap_or(Mode::OptIn)
    }
    pub fn label(self) -> &'static str {
        match self {
            Mode::OptIn => "Ask first (opt-in, GDPR)",
            Mode::OptOut => "Allow until refused (opt-out, CCPA)",
            Mode::Notice => "Inform only",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Mode::OptIn => "Opt-in",
            Mode::OptOut => "Opt-out",
            Mode::Notice => "Notice only",
        }
    }
    /// Whether optional scripts are held back by the renderer. In notice mode
    /// they run straight away, so rewriting them would only delay them.
    pub fn gates(self) -> bool {
        !matches!(self, Mode::Notice)
    }
}

/// Where the banner sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// A full-width strip along the bottom.
    Bar,
    /// A card in the bottom-left corner. The least obstructive.
    Box,
    /// A centred dialog that must be answered before reading on.
    Modal,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Box, Layout::Bar, Layout::Modal];
    pub fn key(self) -> &'static str {
        match self {
            Layout::Bar => "bar",
            Layout::Box => "box",
            Layout::Modal => "modal",
        }
    }
    pub fn from_key(key: &str) -> Layout {
        Layout::ALL.into_iter().find(|l| l.key() == key.trim()).unwrap_or(Layout::Box)
    }
    pub fn label(self) -> &'static str {
        match self {
            Layout::Bar => "Bar along the bottom",
            Layout::Box => "Card in the corner",
            Layout::Modal => "Centred dialog",
        }
    }
}

pub const SETTING_ENABLED: &str = "cookies_enabled";
pub const SETTING_MODE: &str = "cookies_mode";
pub const SETTING_LAYOUT: &str = "cookies_layout";
pub const SETTING_TITLE: &str = "cookies_title";
pub const SETTING_MESSAGE: &str = "cookies_message";
pub const SETTING_POLICY_URL: &str = "cookies_policy_url";
pub const SETTING_REVISION: &str = "cookies_revision";
pub const SETTING_EXPIRY_MONTHS: &str = "cookies_expiry_months";
pub const SETTING_HONOUR_GPC: &str = "cookies_honour_gpc";
pub const SETTING_BLOCK_EMBEDS: &str = "cookies_block_embeds";
pub const SETTING_REOPEN: &str = "cookies_reopen";
pub const SETTING_LOG: &str = "cookies_log";
pub const SETTING_LOG_DAYS: &str = "cookies_log_days";

/// Every key, for the migration test and the settings save.
pub const SETTING_KEYS: [&str; 13] = [
    SETTING_ENABLED,
    SETTING_MODE,
    SETTING_LAYOUT,
    SETTING_TITLE,
    SETTING_MESSAGE,
    SETTING_POLICY_URL,
    SETTING_REVISION,
    SETTING_EXPIRY_MONTHS,
    SETTING_HONOUR_GPC,
    SETTING_BLOCK_EMBEDS,
    SETTING_REOPEN,
    SETTING_LOG,
    SETTING_LOG_DAYS,
];

pub const DEFAULT_TITLE: &str = "Your privacy";
pub const DEFAULT_MESSAGE: &str = "We use cookies to keep this site working. With your permission we would also like to count visits and load content from other sites. You can change your mind at any time.";

pub const MAX_TITLE: usize = 80;
pub const MAX_MESSAGE: usize = 600;
pub const MAX_URL: usize = 500;

/// How long a decision is remembered before the visitor is asked again.
/// Thirteen months is the ceiling the CNIL and the ICO both name; six is
/// the CNIL's recommendation. Nothing longer is offered.
pub const EXPIRY_MONTHS: [i64; 4] = [3, 6, 12, 13];
/// How long the consent log keeps a record. GDPR asks the site to be able to
/// *demonstrate* consent (Art. 7(1)) for as long as it relies on it, which is
/// at most the expiry plus the time a complaint might take.
pub const LOG_DAYS: [i64; 5] = [90, 180, 365, 730, 1825];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    pub layout: Layout,
    pub title: String,
    pub message: String,
    /// The site's privacy or cookie policy. Blank shows no link.
    pub policy_url: String,
    /// Bumped by "Ask everyone again". A stored decision for an older
    /// revision is not a decision about what the site does now.
    pub revision: i64,
    pub expiry_months: i64,
    pub honour_gpc: bool,
    /// Hold back embedded videos and posts until marketing is allowed.
    pub block_embeds: bool,
    /// A small button that reopens the choices once the banner is gone.
    pub reopen: bool,
    /// Keep a record of each decision.
    pub log: bool,
    pub log_days: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    /// `get` is the settings cache's string lookup. A missing or malformed row
    /// is the default; a value that was never offered is the default too.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let number = |key: &str| get(key).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let text = |key: &str, max: usize, default: &str| {
            let raw = get(key).unwrap_or_default();
            let cleaned = clean(&raw, max);
            if cleaned.is_empty() { default.to_string() } else { cleaned }
        };
        Settings {
            enabled: flag(SETTING_ENABLED, false),
            mode: Mode::from_key(&get(SETTING_MODE).unwrap_or_default()),
            layout: Layout::from_key(&get(SETTING_LAYOUT).unwrap_or_default()),
            title: text(SETTING_TITLE, MAX_TITLE, DEFAULT_TITLE),
            message: text(SETTING_MESSAGE, MAX_MESSAGE, DEFAULT_MESSAGE),
            policy_url: policy_url(&get(SETTING_POLICY_URL).unwrap_or_default()).unwrap_or_default(),
            revision: number(SETTING_REVISION).unwrap_or(1).max(1),
            expiry_months: number(SETTING_EXPIRY_MONTHS)
                .filter(|m| EXPIRY_MONTHS.contains(m))
                .unwrap_or(12),
            honour_gpc: flag(SETTING_HONOUR_GPC, true),
            block_embeds: flag(SETTING_BLOCK_EMBEDS, false),
            reopen: flag(SETTING_REOPEN, true),
            log: flag(SETTING_LOG, true),
            log_days: number(SETTING_LOG_DAYS).filter(|d| LOG_DAYS.contains(d)).unwrap_or(365),
        }
    }

    pub fn expiry_days(&self) -> i64 {
        // A month as the regulators count it: 13 months is 395 days, not 390.
        self.expiry_months * 365 / 12
    }
}

/// A policy link is either a site path (`/privacy/`) or an absolute http(s)
/// URL. Anything else — `javascript:`, `data:`, a protocol-relative `//host`
/// — is refused, because it ends up as an `href` on every page of the site.
pub fn policy_url(raw: &str) -> Option<String> {
    let url = clean(raw, MAX_URL);
    if url.is_empty() {
        return Some(String::new());
    }
    if url.chars().any(|c| c.is_whitespace() || c == '"' || c == '<' || c == '>' || c == '\\') {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if (url.starts_with('/') && !url.starts_with("//"))
        || lower.starts_with("https://")
        || lower.starts_with("http://")
    {
        Some(url)
    } else {
        None
    }
}

/// Trim, drop control characters, and bound by characters (not bytes, so a
/// multi-byte character is never cut in half).
pub fn clean(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// trackers
// ---------------------------------------------------------------------------

/// A cookie a tracker is known to set: name, how long, what for. A name
/// ending in `*` is a prefix (`_ga_*` is one cookie per GA4 property).
pub type KnownCookie = (&'static str, &'static str, &'static str);

/// A third-party script or embed this module can recognise.
#[derive(Debug, Clone, Copy)]
pub struct Tracker {
    pub key: &'static str,
    pub name: &'static str,
    /// Who runs it — the "provider" column of a cookie declaration.
    pub vendor: &'static str,
    pub category: Category,
    /// What gives it away in markup. Matched case-insensitively against a
    /// script's `src` and body, or an iframe's `src`.
    pub needles: &'static [&'static str],
    pub cookies: &'static [KnownCookie],
    /// Embeds are iframes and only blocked when `block_embeds` is on.
    pub embed: bool,
    /// Sets nothing on the visitor's device, so it needs no consent and is
    /// never held back. Listed so the scan can say "found, and fine".
    pub cookieless: bool,
}

impl Tracker {
    /// Whether the renderer holds this one back.
    pub fn gated(&self) -> bool {
        !self.cookieless && self.category != Category::Necessary
    }
}

/// The library. Order is the order the scan reports in when two match the
/// same tag; put the specific before the general (Google Ads before the tag
/// manager that usually loads it).
pub const TRACKERS: &[Tracker] = &[
    Tracker {
        key: "google_analytics",
        name: "Google Analytics",
        vendor: "Google",
        category: Category::Analytics,
        needles: &["googletagmanager.com/gtag/js", "google-analytics.com/analytics.js", "google-analytics.com/ga.js", "gtag('config'", "gtag(\"config\""],
        cookies: &[
            ("_ga", "2 years", "Tells one visitor from another for Google Analytics."),
            ("_ga_*", "2 years", "Keeps the state of a visit for one Google Analytics property."),
            ("_gid", "24 hours", "Tells one visitor from another for a day."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "google_ads",
        name: "Google Ads",
        vendor: "Google",
        category: Category::Marketing,
        needles: &["googleadservices.com", "googlesyndication.com", "doubleclick.net", "adsbygoogle"],
        cookies: &[
            ("_gcl_au", "3 months", "Measures which ads led to a visit."),
            ("IDE", "13 months", "Shows and measures Google ads on other sites."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "google_tag_manager",
        name: "Google Tag Manager",
        vendor: "Google",
        category: Category::Marketing,
        needles: &["googletagmanager.com/gtm.js", "googletagmanager.com/ns.html"],
        cookies: &[],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "meta_pixel",
        name: "Meta Pixel",
        vendor: "Meta",
        category: Category::Marketing,
        needles: &["connect.facebook.net", "fbq('init'", "fbq(\"init\""],
        cookies: &[
            ("_fbp", "3 months", "Shows Facebook and Instagram ads to people who visited."),
            ("fr", "3 months", "Delivers and measures Meta ads."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "linkedin_insight",
        name: "LinkedIn Insight Tag",
        vendor: "LinkedIn",
        category: Category::Marketing,
        needles: &["snap.licdn.com", "_linkedin_partner_id"],
        cookies: &[
            ("li_sugr", "3 months", "Matches visitors to LinkedIn members for ads."),
            ("bcookie", "1 year", "Identifies the browser to LinkedIn."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "tiktok_pixel",
        name: "TikTok Pixel",
        vendor: "TikTok",
        category: Category::Marketing,
        needles: &["analytics.tiktok.com"],
        cookies: &[("_ttp", "13 months", "Measures and targets TikTok ads.")],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "x_pixel",
        name: "X (Twitter) Pixel",
        vendor: "X Corp.",
        category: Category::Marketing,
        needles: &["static.ads-twitter.com"],
        cookies: &[("muc_ads", "2 years", "Measures and targets X ads.")],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "hotjar",
        name: "Hotjar",
        vendor: "Hotjar",
        category: Category::Analytics,
        needles: &["static.hotjar.com", "hotjar.com/c/hotjar-"],
        cookies: &[
            ("_hjSessionUser_*", "1 year", "Tells one Hotjar visitor from another."),
            ("_hjSession_*", "30 minutes", "Holds the current Hotjar session."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "clarity",
        name: "Microsoft Clarity",
        vendor: "Microsoft",
        category: Category::Analytics,
        needles: &["clarity.ms/tag"],
        cookies: &[
            ("_clck", "1 year", "Tells one Clarity visitor from another."),
            ("_clsk", "1 day", "Joins page views into one Clarity recording."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "matomo",
        name: "Matomo",
        vendor: "Matomo (self-hosted)",
        category: Category::Analytics,
        needles: &["matomo.js", "piwik.js", "_paq.push"],
        cookies: &[
            ("_pk_id.*", "13 months", "Tells one Matomo visitor from another."),
            ("_pk_ses.*", "30 minutes", "Holds the current Matomo visit."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "plausible",
        name: "Plausible",
        vendor: "Plausible Analytics",
        category: Category::Analytics,
        needles: &["plausible.io/js"],
        cookies: &[],
        embed: false,
        cookieless: true,
    },
    Tracker {
        key: "fathom",
        name: "Fathom",
        vendor: "Fathom Analytics",
        category: Category::Analytics,
        needles: &["cdn.usefathom.com"],
        cookies: &[],
        embed: false,
        cookieless: true,
    },
    Tracker {
        key: "stripe",
        name: "Stripe",
        vendor: "Stripe",
        category: Category::Necessary,
        needles: &["js.stripe.com"],
        cookies: &[
            ("__stripe_mid", "1 year", "Prevents card fraud on payments."),
            ("__stripe_sid", "30 minutes", "Prevents card fraud on payments."),
        ],
        embed: false,
        cookieless: false,
    },
    Tracker {
        key: "youtube",
        name: "YouTube embeds",
        vendor: "Google",
        category: Category::Marketing,
        needles: &["youtube.com/embed/", "youtu.be/"],
        cookies: &[
            ("VISITOR_INFO1_LIVE", "6 months", "Estimates bandwidth and tracks viewing for YouTube."),
            ("YSC", "Session", "Counts YouTube views and remembers player settings."),
        ],
        embed: true,
        cookieless: false,
    },
    Tracker {
        key: "vimeo",
        name: "Vimeo embeds",
        vendor: "Vimeo",
        category: Category::Marketing,
        needles: &["player.vimeo.com/video/"],
        cookies: &[("vuid", "2 years", "Counts Vimeo views and remembers the viewer.")],
        embed: true,
        cookieless: false,
    },
    Tracker {
        key: "x_embed",
        name: "X (Twitter) posts",
        vendor: "X Corp.",
        category: Category::Marketing,
        needles: &["platform.twitter.com/widgets.js", "platform.x.com/widgets.js"],
        cookies: &[],
        embed: true,
        cookieless: false,
    },
    Tracker {
        key: "instagram_embed",
        name: "Instagram posts",
        vendor: "Meta",
        category: Category::Marketing,
        needles: &["instagram.com/embed.js", "instagram.com/p/"],
        cookies: &[],
        embed: true,
        cookieless: false,
    },
    Tracker {
        key: "spotify_embed",
        name: "Spotify players",
        vendor: "Spotify",
        category: Category::Marketing,
        needles: &["open.spotify.com/embed"],
        cookies: &[("sp_t", "1 year", "Identifies the listener to Spotify.")],
        embed: true,
        cookieless: false,
    },
];

pub fn tracker(key: &str) -> Option<&'static Tracker> {
    TRACKERS.iter().find(|t| t.key == key)
}

/// The first tracker a piece of markup belongs to, if any.
pub fn identify(haystack: &str) -> Option<&'static Tracker> {
    let lower = haystack.to_ascii_lowercase();
    TRACKERS
        .iter()
        .find(|t| t.needles.iter().any(|n| lower.contains(&n.to_ascii_lowercase())))
}

/// Every tracker that appears anywhere in a document, each once, in library
/// order. The scan runs this over code injection, post bodies and theme
/// templates.
pub fn detect(source: &str) -> Vec<&'static Tracker> {
    let lower = source.to_ascii_lowercase();
    TRACKERS
        .iter()
        .filter(|t| t.needles.iter().any(|n| lower.contains(&n.to_ascii_lowercase())))
        .collect()
}

/// The cookies the site itself sets. Always declared, never editable: they
/// are what the CMS does, not what somebody pasted in.
pub fn builtin_cookies() -> Vec<Declared> {
    let own = |name: &str, duration: &str, purpose: &str, category: Category| Declared {
        name: name.into(),
        provider: "This site".into(),
        category,
        duration: duration.into(),
        purpose: purpose.into(),
    };
    vec![
        own("vuuut_consent", "Up to 13 months", "Remembers the choices made in this banner.", Category::Necessary),
        own("vuuut-members-ssr", "6 months", "Keeps a member signed in.", Category::Necessary),
        own("vuuut-members-ssr.sig", "6 months", "Proves the sign-in cookie was issued by this site.", Category::Necessary),
        own("ghost-private", "Until the browser closes", "Lets a visitor through a password-protected site.", Category::Necessary),
        own("vuuut-popups (local storage)", "Until cleared", "Remembers which popups have been closed so they are not shown again.", Category::Preferences),
    ]
}

/// A row in the cookie declaration, built-in or declared by staff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub name: String,
    pub provider: String,
    pub category: Category,
    pub duration: String,
    pub purpose: String,
}

pub const MAX_COOKIE_NAME: usize = 100;
pub const MAX_PROVIDER: usize = 100;
pub const MAX_DURATION: usize = 40;
pub const MAX_PURPOSE: usize = 300;

/// A declaration from the admin's form, rebuilt rather than trusted. A name is
/// a cookie name: printable, no spaces, no `;` or `=` (which would end it).
pub fn declaration(name: &str, provider: &str, category: &str, duration: &str, purpose: &str) -> Result<Declared, &'static str> {
    let name = clean(name, MAX_COOKIE_NAME);
    if name.is_empty() {
        return Err("A cookie needs a name.");
    }
    if name.chars().any(|c| c.is_whitespace() || matches!(c, ';' | '=' | ',' | '"' | '<' | '>')) {
        return Err("A cookie name cannot contain spaces or ; = , \" < >.");
    }
    let category = Category::from_key(category).ok_or("Choose what the cookie is for.")?;
    let provider = clean(provider, MAX_PROVIDER);
    Ok(Declared {
        name,
        provider: if provider.is_empty() { "This site".into() } else { provider },
        category,
        duration: clean(duration, MAX_DURATION),
        purpose: clean(purpose, MAX_PURPOSE),
    })
}

// ---------------------------------------------------------------------------
// gating the rendered page
// ---------------------------------------------------------------------------

/// The attribute a held-back script or iframe carries, and the one an author
/// can write by hand to hold back a script the library does not know.
pub const CATEGORY_ATTR: &str = "data-cookie-category";

/// What [`gate_html`] did, for tests and for the scan's "blocked" chip.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Gated {
    pub scripts: usize,
    pub embeds: usize,
}

/// Rewrite every recognised tracker in a rendered page so it cannot run.
///
/// A `<script>` becomes `<script type="text/plain" data-cookie-category="…">`
/// — the browser parses it and never executes it, and the visitor script
/// swaps it for a live copy once the category is allowed. An embed's `src`
/// moves to `data-cookie-src`, so the frame never loads.
///
/// Left alone, deliberately:
/// * a script that already has a non-JavaScript `type` (JSON-LD, templates);
/// * a script that already carries `data-cookie-category` (the author chose);
/// * a script marked `data-cookie-ignore`;
/// * anything necessary or cookieless.
///
/// This is a scanner over tags rather than an HTML parser, and it is written
/// to fail closed in the safe direction: anything it cannot read as a tag is
/// copied through unchanged, so the worst case is a tracker not held back —
/// which the scan in the admin would still report — never a broken page.
pub fn gate_html(html: &str, block_embeds: bool) -> (String, Gated) {
    let mut out = String::with_capacity(html.len() + 256);
    let mut gated = Gated::default();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(rel) = next_tag(&lower[at..]) {
        let start = at + rel;
        let Some(end_rel) = lower[start..].find('>') else { break };
        let open_end = start + end_rel + 1;
        let tag = &html[start..open_end];
        let tag_lower = &lower[start..open_end];
        out.push_str(&html[at..start]);
        if tag_lower.starts_with("<script") {
            // The body runs to the matching close tag. A script cannot
            // contain `</script` (the HTML tokenizer ends it there), so the
            // first one is the end.
            let body_end = lower[open_end..].find("</script").map(|i| open_end + i).unwrap_or(open_end);
            let src = attr(tag, "src").unwrap_or_default();
            let body = &html[open_end..body_end];
            let hit = if gateable_script(tag) { identify(&format!("{src} {body}")) } else { None };
            match hit.filter(|t| t.gated()) {
                Some(t) => {
                    gated.scripts += 1;
                    out.push_str(&hold_script(tag, t.category));
                }
                None => out.push_str(tag),
            }
            out.push_str(body);
            at = body_end;
        } else {
            // `<iframe`
            let src = attr(tag, "src").unwrap_or_default();
            let hit = if block_embeds && attr(tag, CATEGORY_ATTR).is_none() { identify(&src) } else { None };
            match hit.filter(|t| t.gated() && t.embed) {
                Some(t) => {
                    gated.embeds += 1;
                    out.push_str(&hold_iframe(tag, t));
                }
                None => out.push_str(tag),
            }
            at = open_end;
        }
    }
    out.push_str(&html[at..]);
    (out, gated)
}

/// The next `<script` or `<iframe` that is really a tag (followed by
/// whitespace, `>` or `/`), not `<scripted` in some text.
fn next_tag(lower: &str) -> Option<usize> {
    let mut from = 0;
    loop {
        let s = lower[from..].find("<script").map(|i| (from + i, 7));
        let f = lower[from..].find("<iframe").map(|i| (from + i, 7));
        let (pos, len) = match (s, f) {
            (Some(a), Some(b)) => if a.0 < b.0 { a } else { b },
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return None,
        };
        match lower.as_bytes().get(pos + len) {
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') => return Some(pos),
            _ => from = pos + len,
        }
    }
}

fn gateable_script(tag: &str) -> bool {
    if attr(tag, CATEGORY_ATTR).is_some() || attr(tag, "data-cookie-ignore").is_some() {
        return false;
    }
    match attr(tag, "type").map(|t| t.trim().to_ascii_lowercase()) {
        None => true,
        Some(t) => t.is_empty() || t == "module" || t.contains("javascript") || t.contains("ecmascript"),
    }
}

fn hold_script(tag: &str, category: Category) -> String {
    // Keep the original type (a module must come back as a module), drop it
    // from the tag, and put ours first so it is the one the browser reads.
    let original = attr(tag, "type").unwrap_or_default();
    let without = remove_attr(tag, "type");
    let rest = &without["<script".len()..];
    let keep = if original.trim().eq_ignore_ascii_case("module") { r#" data-cookie-type="module""# } else { "" };
    format!(r#"<script type="text/plain" {CATEGORY_ATTR}="{}"{keep}{rest}"#, category.key())
}

fn hold_iframe(tag: &str, tracker: &Tracker) -> String {
    let src = attr(tag, "src").unwrap_or_default();
    let without = remove_attr(tag, "src");
    let rest = &without["<iframe".len()..];
    format!(
        r#"<iframe data-cookie-src="{src}" {CATEGORY_ATTR}="{}" data-cookie-vendor="{}"{rest}"#,
        tracker.category.key(),
        tracker.name,
        src = attr_escape(&src),
    )
}

/// Escape for a double-quoted attribute. `src` came out of an attribute that
/// was already HTML, so only the quote and the angle brackets matter here.
fn attr_escape(s: &str) -> String {
    s.replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Where an attribute's name starts and its value, if any. Attribute names are
/// matched whole: `src` does not match `data-src`.
fn find_attr(tag: &str, name: &str) -> Option<(usize, usize, Option<(usize, usize)>)> {
    let bytes = tag.as_bytes();
    let lower = tag.to_ascii_lowercase();
    // Skip the tag name.
    let mut i = tag.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(tag.len());
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' {
            return None;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let this = &lower[name_start..i];
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = None;
        let mut end = i;
        if j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                let quote = bytes[j];
                let v_start = j + 1;
                let v_end = tag[v_start..].find(quote as char).map(|k| v_start + k).unwrap_or(tag.len());
                value = Some((v_start, v_end));
                end = (v_end + 1).min(tag.len());
            } else {
                let v_start = j;
                while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b'>' {
                    j += 1;
                }
                value = Some((v_start, j));
                end = j;
            }
        }
        if this == name {
            return Some((name_start, end, value));
        }
        i = end.max(i + 1);
    }
    None
}

/// An attribute's value; `Some("")` for a bare attribute.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    find_attr(tag, name).map(|(_, _, v)| v.map(|(s, e)| tag[s..e].to_string()).unwrap_or_default())
}

fn remove_attr(tag: &str, name: &str) -> String {
    match find_attr(tag, name) {
        Some((s, e, _)) => {
            // Take the whitespace before it too, so no double space is left.
            let s = tag[..s].trim_end().len();
            format!("{}{}", &tag[..s], &tag[e..])
        }
        None => tag.to_string(),
    }
}

// ---------------------------------------------------------------------------
// a visitor's decision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    AcceptAll,
    RejectAll,
    Custom,
    /// Global Privacy Control answered for them.
    Gpc,
    /// Inform-only mode: the visitor acknowledged the notice.
    Acknowledge,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::AcceptAll, Action::RejectAll, Action::Custom, Action::Gpc, Action::Acknowledge];
    pub fn key(self) -> &'static str {
        match self {
            Action::AcceptAll => "accept_all",
            Action::RejectAll => "reject_all",
            Action::Custom => "custom",
            Action::Gpc => "gpc",
            Action::Acknowledge => "acknowledge",
        }
    }
    pub fn from_key(key: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.key() == key.trim())
    }
    pub fn label(self) -> &'static str {
        match self {
            Action::AcceptAll => "Accepted all",
            Action::RejectAll => "Rejected all",
            Action::Custom => "Chose some",
            Action::Gpc => "Global Privacy Control",
            Action::Acknowledge => "Acknowledged",
        }
    }
}

/// One decision, normalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub action: Action,
    /// Always contains `Necessary`.
    pub granted: BTreeSet<Category>,
}

impl Choice {
    /// Rebuild a decision from what a browser sent. The action decides
    /// whenever it can — "accept all" grants all whatever the list says, and
    /// "reject all" grants nothing optional — so a tampered list cannot make
    /// the log say somebody refused while recording a grant, or the reverse.
    pub fn parse(action: &str, categories: &str) -> Option<Choice> {
        let action = Action::from_key(action)?;
        let mut granted: BTreeSet<Category> = match action {
            Action::AcceptAll | Action::Acknowledge => Category::ALL.into_iter().collect(),
            Action::RejectAll => BTreeSet::new(),
            Action::Custom | Action::Gpc => categories.split(',').filter_map(Category::from_key).collect(),
        };
        if action == Action::Gpc {
            // GPC is a refusal of sale and sharing whatever else was sent.
            granted.remove(&Category::Marketing);
        }
        granted.insert(Category::Necessary);
        Some(Choice { action, granted })
    }

    /// `necessary,analytics` — the stored and logged form, in a fixed order.
    pub fn categories(&self) -> String {
        Category::ALL
            .into_iter()
            .filter(|c| self.granted.contains(c))
            .map(Category::key)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A consent id is what the visitor's browser generated and shows them in
/// the preferences panel, so they can quote it in a request. A UUID, or
/// nothing — it is looked up in the log and printed in the admin.
pub fn is_consent_id(id: &str) -> bool {
    id.len() == 36
        && id.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// A search for the log: lowercase hex and dashes only, so it can be echoed
/// back into a link and a `LIKE` without either being able to misread it.
pub fn log_query(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_ascii_hexdigit() || *c == '-')
        .take(36)
        .collect::<String>()
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// the figures
// ---------------------------------------------------------------------------

/// The log over a window, counted. Built from `(action, categories, n)` rows
/// so the arithmetic is here and testable rather than in SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub total: i64,
    pub accept_all: i64,
    pub reject_all: i64,
    pub custom: i64,
    pub gpc: i64,
    pub acknowledge: i64,
    /// For each optional category, how many decisions granted it.
    pub granted: [(Category, i64); 3],
}

impl Default for Summary {
    fn default() -> Self {
        Summary {
            total: 0,
            accept_all: 0,
            reject_all: 0,
            custom: 0,
            gpc: 0,
            acknowledge: 0,
            granted: [(Category::Preferences, 0), (Category::Analytics, 0), (Category::Marketing, 0)],
        }
    }
}

impl Summary {
    pub fn from_rows(rows: &[(String, String, i64)]) -> Summary {
        let mut s = Summary::default();
        for (action, categories, n) in rows {
            let n = (*n).max(0);
            let Some(action) = Action::from_key(action) else { continue };
            s.total += n;
            match action {
                Action::AcceptAll => s.accept_all += n,
                Action::RejectAll => s.reject_all += n,
                Action::Custom => s.custom += n,
                Action::Gpc => s.gpc += n,
                Action::Acknowledge => s.acknowledge += n,
            }
            let set: BTreeSet<Category> = categories.split(',').filter_map(Category::from_key).collect();
            for slot in s.granted.iter_mut() {
                if set.contains(&slot.0) {
                    slot.1 += n;
                }
            }
        }
        s
    }

    /// A share of all decisions, or `None` when nobody has decided — a rate
    /// over nothing is unmeasured, not 0%.
    pub fn rate(&self, n: i64) -> Option<f64> {
        (self.total > 0).then(|| n as f64 * 100.0 / self.total as f64)
    }
}

// ---------------------------------------------------------------------------
// the audit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The site is setting cookies it has no permission to set.
    Critical,
    /// A gap a regulator or a complaint would find.
    Warning,
    /// Worth knowing; not wrong.
    Note,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Critical => "Critical",
            Level::Warning => "Warning",
            Level::Note => "Note",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub detail: String,
    /// Where to fix it, inside the admin.
    pub fix: Option<(&'static str, &'static str)>,
}

/// What the audit judges: the settings plus what the scan found.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub settings: Settings,
    /// Keys of every tracker found anywhere on the site.
    pub found: Vec<&'static str>,
    /// Keys of found trackers whose cookies are not in the declaration.
    pub undeclared: Vec<&'static str>,
    /// `Some(true)` when Vuuut Click is connected and waits for consent.
    pub click_needs_consent: Option<bool>,
}

pub const FIX_SETTINGS: (&str, &str) = ("/ghost/cookies/settings/", "Cookie settings");
pub const FIX_DECLARATION: (&str, &str) = ("/ghost/cookies/declarations/", "Cookie declaration");

/// The report, most serious first. An empty list is a clean bill.
pub fn audit(input: &Inputs) -> Vec<Finding> {
    let s = &input.settings;
    let found: Vec<&Tracker> = input.found.iter().filter_map(|k| tracker(k)).collect();
    let needs_consent: Vec<&&Tracker> = found.iter().filter(|t| t.gated() && (!t.embed || s.block_embeds || !s.enabled)).collect();
    let embeds: Vec<&&Tracker> = found.iter().filter(|t| t.embed && t.gated()).collect();
    let names = |ts: &[&&Tracker]| ts.iter().map(|t| t.name).collect::<Vec<_>>().join(", ");
    let mut out = Vec::new();

    if !s.enabled && !needs_consent.is_empty() {
        out.push(Finding {
            level: Level::Critical,
            title: format!("{} set cookies before anyone agrees", plural(needs_consent.len(), "tracker", "trackers")),
            detail: format!("{} run on every page and the consent banner is off. In the EU, the UK and several US states that needs a visitor's permission first.", names(&needs_consent)),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && s.mode == Mode::Notice && !needs_consent.is_empty() {
        out.push(Finding {
            level: Level::Critical,
            title: "An inform-only banner is not consent".into(),
            detail: format!("{} set cookies that need permission, and a notice does not ask for it. Switch to “Ask first”.", names(&needs_consent)),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && s.mode == Mode::OptOut && needs_consent.iter().any(|t| t.category == Category::Marketing) {
        out.push(Finding {
            level: Level::Warning,
            title: "Marketing cookies load before European visitors agree".into(),
            detail: "Opt-out is how US state laws work. Under the GDPR and the UK's PECR, advertising cookies need a yes first — use “Ask first” if any readers are in Europe.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && !s.block_embeds && !embeds.is_empty() {
        out.push(Finding {
            level: Level::Warning,
            title: format!("{} load before anyone agrees", names(&embeds)),
            detail: "Embedded players and posts set their own cookies the moment the page loads. Hold them back until marketing is allowed; readers get a one-click placeholder instead.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if input.click_needs_consent == Some(true) && !s.enabled {
        out.push(Finding {
            level: Level::Warning,
            title: "Analytics is waiting for consent nobody can give".into(),
            detail: "Vuuut Click is connected in consent-required mode, but there is no banner to ask, so it counts no visits at all. Switch the banner on and analytics follows each visitor's answer.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if !input.undeclared.is_empty() {
        let undeclared: Vec<&Tracker> = input.undeclared.iter().filter_map(|k| tracker(k)).collect();
        out.push(Finding {
            level: Level::Warning,
            title: format!("{} not in the cookie declaration", plural(undeclared.len(), "tracker is", "trackers are")),
            detail: format!("Visitors cannot see what {} stores or for how long. Add them in one click.", undeclared.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")),
            fix: Some(FIX_DECLARATION),
        });
    }
    if s.enabled && s.policy_url.is_empty() {
        out.push(Finding {
            level: Level::Warning,
            title: "The banner links to no privacy policy".into(),
            detail: "Consent has to be informed. Link the page that says who you share data with and how to reach you.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && !s.log && s.mode != Mode::Notice {
        out.push(Finding {
            level: Level::Warning,
            title: "No record of consent is kept".into(),
            detail: "The GDPR asks the site to be able to show that somebody agreed (Article 7). The log stores no names or addresses — only the choice, a random id and when.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && !s.honour_gpc {
        out.push(Finding {
            level: if s.mode == Mode::OptOut { Level::Warning } else { Level::Note },
            title: "Global Privacy Control is ignored".into(),
            detail: "Browsers such as Firefox, Brave and DuckDuckGo send it on the visitor's behalf. California and Colorado treat it as a legally binding opt-out.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    if s.enabled && s.expiry_months > 12 {
        out.push(Finding {
            level: Level::Note,
            title: "Choices are kept for 13 months".into(),
            detail: "That is the most any regulator allows. France's CNIL recommends asking again after 6.".into(),
            fix: Some(FIX_SETTINGS),
        });
    }
    out.sort_by_key(|f| f.level);
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings::read(|k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn a_missing_row_reads_as_off_so_installing_changes_nothing() {
        let s = Settings::default();
        assert!(!s.enabled);
        assert_eq!(s.mode, Mode::OptIn);
        assert_eq!(s.revision, 1);
        assert_eq!(s.expiry_months, 12);
        assert!(s.honour_gpc && s.log && s.reopen && !s.block_embeds);
        assert_eq!(s.title, DEFAULT_TITLE);
    }

    #[test]
    fn values_that_were_never_offered_fall_back_to_the_default() {
        let s = settings(&[
            (SETTING_MODE, "anything"),
            (SETTING_EXPIRY_MONTHS, "120"),
            (SETTING_LOG_DAYS, "3"),
            (SETTING_REVISION, "-4"),
            (SETTING_LAYOUT, "fullscreen"),
        ]);
        assert_eq!(s.mode, Mode::OptIn);
        assert_eq!(s.expiry_months, 12);
        assert_eq!(s.log_days, 365);
        assert_eq!(s.revision, 1);
        assert_eq!(s.layout, Layout::Box);
    }

    #[test]
    fn a_policy_link_is_a_path_or_http_and_nothing_that_can_run() {
        assert_eq!(policy_url("/privacy/").as_deref(), Some("/privacy/"));
        assert_eq!(policy_url("https://example.com/p").as_deref(), Some("https://example.com/p"));
        assert_eq!(policy_url("").as_deref(), Some(""));
        for bad in ["javascript:alert(1)", "//evil.example/", "data:text/html,x", "/a\"onmouseover=x", "JaVaScRiPt:x"] {
            assert_eq!(policy_url(bad), None, "{bad}");
        }
        // And a stored bad value reads as no link at all.
        assert_eq!(settings(&[(SETTING_POLICY_URL, "javascript:alert(1)")]).policy_url, "");
    }

    #[test]
    fn thirteen_months_is_395_days() {
        assert_eq!(settings(&[(SETTING_EXPIRY_MONTHS, "13")]).expiry_days(), 395);
        assert_eq!(settings(&[(SETTING_EXPIRY_MONTHS, "6")]).expiry_days(), 182);
    }

    #[test]
    fn the_scan_finds_each_tracker_once_and_names_cookieless_ones_as_such() {
        let html = r#"<script async src="https://www.googletagmanager.com/gtag/js?id=G-1"></script>
<script>gtag('config', 'G-1');</script><script defer data-domain="x" src="https://plausible.io/js/script.js"></script>"#;
        let found: Vec<&str> = detect(html).iter().map(|t| t.key).collect();
        assert_eq!(found, ["google_analytics", "plausible"]);
        assert!(tracker("plausible").unwrap().cookieless);
        assert!(!tracker("plausible").unwrap().gated());
        assert!(!tracker("stripe").unwrap().gated(), "necessary is never held back");
    }

    #[test]
    fn a_tracker_script_is_held_back_and_everything_else_is_left_alone() {
        let html = r#"<head><script type="application/ld+json">{"googletagmanager.com/gtag/js":1}</script>
<script async src="https://www.googletagmanager.com/gtag/js?id=G-1"></script>
<script>!function(f,b,e,v,n,t,s){}(window,document,'script','https://connect.facebook.net/en_US/fbevents.js');fbq('init','1');</script>
<script src="/assets/theme.js"></script>
<script src="https://js.stripe.com/v3/"></script>
<script data-cookie-category="analytics" type="text/plain">custom()</script>
</head><body><p>&lt;script src="https://connect.facebook.net"&gt;</p></body>"#;
        let (out, gated) = gate_html(html, false);
        assert_eq!(gated, Gated { scripts: 2, embeds: 0 });
        assert!(out.contains(r#"<script type="text/plain" data-cookie-category="analytics" async src="https://www.googletagmanager.com/gtag/js?id=G-1">"#), "{out}");
        assert!(out.contains(r#"<script type="text/plain" data-cookie-category="marketing">!function"#));
        assert!(out.contains(r#"<script type="application/ld+json">"#), "JSON-LD untouched");
        assert!(out.contains(r#"<script src="/assets/theme.js">"#));
        assert!(out.contains(r#"<script src="https://js.stripe.com/v3/">"#), "necessary untouched");
        assert!(out.contains(r#"<script data-cookie-category="analytics" type="text/plain">custom()"#), "author's own untouched");
        assert!(out.contains("&lt;script src=\"https://connect.facebook.net\"&gt;"), "escaped text untouched");
    }

    #[test]
    fn a_module_script_comes_back_as_a_module() {
        let (out, _) = gate_html(r#"<script type="module" src="https://static.hotjar.com/c/x.js"></script>"#, false);
        assert_eq!(out, r#"<script type="text/plain" data-cookie-category="analytics" data-cookie-type="module" src="https://static.hotjar.com/c/x.js"></script>"#);
    }

    #[test]
    fn an_embed_is_held_back_only_when_asked() {
        let html = r#"<iframe width="560" src="https://www.youtube.com/embed/abc?x=1&amp;y=2" allowfullscreen></iframe>"#;
        let (same, g) = gate_html(html, false);
        assert_eq!((same.as_str(), g.embeds), (html, 0));
        let (out, g) = gate_html(html, true);
        assert_eq!(g.embeds, 1);
        assert_eq!(
            out,
            r#"<iframe data-cookie-src="https://www.youtube.com/embed/abc?x=1&amp;y=2" data-cookie-category="marketing" data-cookie-vendor="YouTube embeds" width="560" allowfullscreen></iframe>"#
        );
    }

    #[test]
    fn a_tag_name_must_be_whole_and_a_broken_tag_is_copied_through() {
        let html = "<scripted>no</scripted><iframes src=\"https://www.youtube.com/embed/a\"><script src=\"https://connect.facebook.net/x.js\"";
        let (out, g) = gate_html(html, true);
        assert_eq!(out, html);
        assert_eq!(g, Gated::default());
    }

    #[test]
    fn data_src_is_not_src() {
        assert_eq!(attr(r#"<script data-src="a" src='b'>"#, "src").as_deref(), Some("b"));
        assert_eq!(attr("<script async src=x>", "async").as_deref(), Some(""));
        assert_eq!(attr("<script async src=x>", "type"), None);
    }

    #[test]
    fn the_action_decides_when_the_list_disagrees() {
        let c = Choice::parse("reject_all", "analytics,marketing").unwrap();
        assert_eq!(c.categories(), "necessary");
        let c = Choice::parse("accept_all", "").unwrap();
        assert_eq!(c.categories(), "necessary,preferences,analytics,marketing");
        let c = Choice::parse("custom", "marketing,bogus,analytics").unwrap();
        assert_eq!(c.categories(), "necessary,analytics,marketing");
        let c = Choice::parse("gpc", "preferences,analytics,marketing").unwrap();
        assert_eq!(c.categories(), "necessary,preferences,analytics", "GPC always refuses marketing");
        assert_eq!(Choice::parse("sell_everything", ""), None);
    }

    #[test]
    fn consent_ids_are_uuids_and_searches_are_reduced_to_hex() {
        assert!(is_consent_id("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(!is_consent_id("0f8fad5b-d9cb-469f-a165-70867728950"));
        assert!(!is_consent_id("0f8fad5b-d9cb-469f-a165-70867728950g"));
        assert_eq!(log_query(" 0F8F'%\"<b> "), "0f8fb");
    }

    #[test]
    fn a_declaration_name_cannot_smuggle_a_cookie_delimiter() {
        assert!(declaration("a;b", "", "analytics", "", "").is_err());
        assert!(declaration("a b", "", "analytics", "", "").is_err());
        assert!(declaration("_ga", "", "nonsense", "", "").is_err());
        let d = declaration(" _ga ", "", "analytics", "2 years", "x").unwrap();
        assert_eq!((d.name.as_str(), d.provider.as_str()), ("_ga", "This site"));
    }

    #[test]
    fn the_summary_counts_actions_and_grants_and_a_rate_over_nothing_is_none() {
        let s = Summary::from_rows(&[
            ("accept_all".into(), "necessary,preferences,analytics,marketing".into(), 6),
            ("reject_all".into(), "necessary".into(), 3),
            ("custom".into(), "necessary,analytics".into(), 1),
            ("forged".into(), "necessary,marketing".into(), 50),
        ]);
        assert_eq!((s.total, s.accept_all, s.reject_all, s.custom), (10, 6, 3, 1));
        assert_eq!(s.granted[1], (Category::Analytics, 7));
        assert_eq!(s.granted[2], (Category::Marketing, 6));
        assert_eq!(s.rate(s.accept_all), Some(60.0));
        assert_eq!(Summary::default().rate(0), None);
    }

    #[test]
    fn trackers_with_the_banner_off_are_critical_and_come_first() {
        let input = Inputs {
            settings: Settings::default(),
            found: vec!["plausible", "google_analytics", "youtube"],
            undeclared: vec!["google_analytics"],
            click_needs_consent: Some(true),
        };
        let report = audit(&input);
        assert_eq!(report[0].level, Level::Critical);
        assert!(report[0].title.starts_with("2 trackers"), "{}", report[0].title);
        assert!(report[0].detail.contains("Google Analytics") && !report[0].detail.contains("Plausible"));
        assert!(report.iter().any(|f| f.title.contains("waiting for consent")));
        assert!(report.windows(2).all(|w| w[0].level <= w[1].level));
    }

    #[test]
    fn a_well_configured_site_has_a_clean_report() {
        let input = Inputs {
            settings: settings(&[
                (SETTING_ENABLED, "true"),
                (SETTING_POLICY_URL, "/privacy/"),
                (SETTING_BLOCK_EMBEDS, "true"),
            ]),
            found: vec!["google_analytics", "youtube"],
            undeclared: vec![],
            click_needs_consent: Some(true),
        };
        assert_eq!(audit(&input), vec![]);
    }

    #[test]
    fn inform_only_with_real_trackers_is_critical() {
        let input = Inputs {
            settings: settings(&[(SETTING_ENABLED, "true"), (SETTING_MODE, "notice"), (SETTING_POLICY_URL, "/p/")]),
            found: vec!["meta_pixel"],
            ..Inputs::default()
        };
        assert_eq!(audit(&input)[0].title, "An inform-only banner is not consent");
    }
}
