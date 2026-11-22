//! Popups & lead generation: what a popup is, when it is allowed to show,
//! and what a visitor is allowed to send back.
//!
//! The rules half of the module. No database and no HTTP, so every decision
//! that can be wrong — when a popup fires, which pages it belongs on, whether
//! a submission is plausible — is unit-tested here, and the admin screen, the
//! public endpoint and the visitor's script all ask the same functions.
//!
//! ## The shape of a popup
//!
//! A popup is a [`Kind`] (how it appears), a [`Trigger`] (what makes it
//! appear), a [`Frequency`] (how often one person may see it), an
//! [`Audience`] (who it is for) and a [`Targeting`] (which pages it belongs
//! on). Everything else is words.
//!
//! ## Where the decisions are made
//!
//! Targeting, audience and schedule are decided **on the server**: the
//! endpoint only ever hands the browser the popups it is allowed to see on
//! that page, so a reader cannot open devtools and find the copy for a
//! campaign that has not started. Trigger and frequency are decided **in the
//! browser**, because both are about one person's behaviour over time and
//! neither is a secret.

use serde::{Deserialize, Serialize};

/// Hard limits. A popup is written by staff, not by visitors, so these are
/// about keeping one page from being ruined by a paste rather than about
/// trust.
pub const MAX_NAME: usize = 120;
pub const MAX_HEADING: usize = 160;
pub const MAX_BODY: usize = 600;
pub const MAX_LABEL: usize = 60;
pub const MAX_PATHS: usize = 40;
pub const MAX_PATH: usize = 200;

/// A visitor's submission. These *are* a trust boundary.
pub const MAX_EMAIL: usize = 191;
pub const MAX_LEAD_NAME: usize = 120;

/// How the popup appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Centred over a dimmed page. The most attention, and the most
    /// interruption.
    Modal,
    /// A card in a bottom corner. Does not block the page.
    SlideIn,
    /// A full-width strip pinned to the top.
    Bar,
}

impl Kind {
    pub fn from_name(name: &str) -> Kind {
        match name {
            "slide_in" => Kind::SlideIn,
            "bar" => Kind::Bar,
            _ => Kind::Modal,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::Modal => "modal",
            Kind::SlideIn => "slide_in",
            Kind::Bar => "bar",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Modal => "Modal — centred, dims the page",
            Kind::SlideIn => "Slide-in — a corner card",
            Kind::Bar => "Bar — a strip across the top",
        }
    }
    pub const ALL: &'static [Kind] = &[Kind::Modal, Kind::SlideIn, Kind::Bar];
}

/// What makes the popup appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// After `value` seconds on the page.
    Delay,
    /// After the reader has passed `value` percent of the page.
    Scroll,
    /// When the pointer leaves towards the browser chrome. Desktop only —
    /// there is no such gesture on a touch screen, and the script falls back
    /// to a delay there rather than never showing.
    Exit,
}

impl Trigger {
    pub fn from_name(name: &str) -> Trigger {
        match name {
            "scroll" => Trigger::Scroll,
            "exit" => Trigger::Exit,
            _ => Trigger::Delay,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Trigger::Delay => "delay",
            Trigger::Scroll => "scroll",
            Trigger::Exit => "exit",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Delay => "After a few seconds",
            Trigger::Scroll => "After scrolling part of the page",
            Trigger::Exit => "When the pointer leaves the page",
        }
    }
    pub const ALL: &'static [Trigger] = &[Trigger::Delay, Trigger::Scroll, Trigger::Exit];

    /// What `value` means for this trigger, and what it is allowed to be.
    ///
    /// Clamped rather than refused: a popup that fires at 0 seconds is an
    /// advert pasted over the thing somebody came to read, and one at 900%
    /// never fires at all. Both are mistakes to correct, not reasons to
    /// refuse a save.
    pub fn clamp(self, value: i64) -> i64 {
        match self {
            Trigger::Delay => value.clamp(1, 120),
            Trigger::Scroll => value.clamp(5, 100),
            // Exit intent has no number; the stored value is ignored.
            Trigger::Exit => 0,
        }
    }
}

/// How often one person may see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// Once, and never again on that browser.
    Once,
    /// Once, then not again for `days`.
    Days,
    /// Every page view that matches. Rarely what anybody wants.
    Always,
}

impl Frequency {
    pub fn from_name(name: &str) -> Frequency {
        match name {
            "days" => Frequency::Days,
            "always" => Frequency::Always,
            _ => Frequency::Once,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Frequency::Once => "once",
            Frequency::Days => "days",
            Frequency::Always => "always",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Frequency::Once => "Once per visitor",
            Frequency::Days => "Once, then wait a while",
            Frequency::Always => "Every matching page view",
        }
    }
    pub const ALL: &'static [Frequency] = &[Frequency::Once, Frequency::Days, Frequency::Always];

    pub fn clamp_days(self, days: i64) -> i64 {
        match self {
            Frequency::Days => days.clamp(1, 365),
            _ => 0,
        }
    }
}

/// Who it is for. Decided on the server, from the member cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Everyone,
    /// Nobody who is signed in. The default for a lead-generation popup:
    /// asking an existing member for the email address they signed up with is
    /// the clearest way to look like you are not paying attention.
    LoggedOut,
    /// Signed-in members only — upgrade prompts, surveys.
    Members,
}

impl Audience {
    pub fn from_name(name: &str) -> Audience {
        match name {
            "everyone" => Audience::Everyone,
            "members" => Audience::Members,
            _ => Audience::LoggedOut,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Audience::Everyone => "everyone",
            Audience::LoggedOut => "logged_out",
            Audience::Members => "members",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Audience::Everyone => "Everyone",
            Audience::LoggedOut => "Visitors who are not signed in",
            Audience::Members => "Signed-in members",
        }
    }
    pub const ALL: &'static [Audience] = &[Audience::LoggedOut, Audience::Everyone, Audience::Members];

    /// Whether this audience includes the person looking.
    pub fn includes(self, is_member: bool) -> bool {
        match self {
            Audience::Everyone => true,
            Audience::LoggedOut => !is_member,
            Audience::Members => is_member,
        }
    }
}

/// Which *kind* of page it belongs on, which is a different question from
/// which paths.
///
/// Ghost's default permalink puts a post at `/slug/` — at the root, beside
/// the static pages — so "every post" cannot be written as a path prefix.
/// That is why this exists beside [`Targeting`] rather than inside it: the
/// two filters answer "what kind of page is this" and "which addresses", and
/// a popup may want either, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Anywhere on the site.
    Any,
    /// Single posts only.
    Posts,
    /// Static pages only.
    Pages,
    /// The home page only.
    Home,
    /// Tag and author archives.
    Archives,
}

impl Surface {
    pub fn from_name(name: &str) -> Surface {
        match name {
            "posts" => Surface::Posts,
            "pages" => Surface::Pages,
            "home" => Surface::Home,
            "archives" => Surface::Archives,
            _ => Surface::Any,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Surface::Any => "any",
            Surface::Posts => "posts",
            Surface::Pages => "pages",
            Surface::Home => "home",
            Surface::Archives => "archives",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Surface::Any => "Anywhere on the site",
            Surface::Posts => "Posts only",
            Surface::Pages => "Pages only",
            Surface::Home => "The home page only",
            Surface::Archives => "Tag & author archives",
        }
    }
    pub const ALL: &'static [Surface] =
        &[Surface::Any, Surface::Posts, Surface::Pages, Surface::Home, Surface::Archives];

    /// Whether a page of `kind` is one of these.
    ///
    /// `kind` is what [`ghost_db::repos::popups::kind_of_path`] worked out:
    /// `post`, `page`, `tag`, `author`, `home` or `other`. An unrecognised
    /// kind matches only `Any`, so a page this does not understand never
    /// gets a popup aimed at something specific.
    pub fn includes(self, kind: &str) -> bool {
        match self {
            Surface::Any => true,
            Surface::Posts => kind == "post",
            Surface::Pages => kind == "page",
            Surface::Home => kind == "home",
            Surface::Archives => kind == "tag" || kind == "author",
        }
    }

    /// Whether this filter needs the server to work out what kind of page it
    /// is at all. A site whose popups all say "anywhere" pays nothing.
    pub fn needs_lookup(self) -> bool {
        self != Surface::Any
    }
}

/// Which pages it belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Targeting {
    Everywhere,
    /// Only on paths that begin with one of the listed prefixes.
    Only,
    /// Everywhere except paths that begin with one of them.
    Except,
}

impl Targeting {
    pub fn from_name(name: &str) -> Targeting {
        match name {
            "only" => Targeting::Only,
            "except" => Targeting::Except,
            _ => Targeting::Everywhere,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Targeting::Everywhere => "everywhere",
            Targeting::Only => "only",
            Targeting::Except => "except",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Targeting::Everywhere => "Every page",
            Targeting::Only => "Only these pages",
            Targeting::Except => "Every page except these",
        }
    }
    pub const ALL: &'static [Targeting] =
        &[Targeting::Everywhere, Targeting::Only, Targeting::Except];
}

/// The module's site-wide settings, from Settings → Popups.
///
/// Read through [`SiteSettings::read`] by both the admin screen and the
/// public endpoint, so the two cannot disagree about what a missing row
/// means. **Every default reproduces the module as it behaved before these
/// settings existed**: popups on, on phones too, no spacing between them, no
/// consent note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteSettings {
    /// The master switch. Off means no popup appears anywhere and the visitor
    /// script is not even added to the page — quicker than unpublishing ten.
    pub enabled: bool,
    /// Keep popups off small screens, where a modal covers the whole page.
    pub hide_on_mobile: bool,
    /// Hours a visitor must go between *any* two popups, across every popup on
    /// the site. Each popup's own frequency still applies on top.
    pub cooldown_hours: i64,
    /// A line under every address field — what will be sent and how to stop
    /// it. Blank shows nothing.
    pub consent_text: String,
}

pub const SETTING_ENABLED: &str = "popups_enabled";
pub const SETTING_HIDE_ON_MOBILE: &str = "popups_hide_on_mobile";
pub const SETTING_COOLDOWN_HOURS: &str = "popups_cooldown_hours";
pub const SETTING_CONSENT_TEXT: &str = "popups_consent_text";

/// A week. Long enough to mean "leave people alone", short enough that a
/// typo of 9999 does not switch popups off for good without saying so.
pub const MAX_COOLDOWN_HOURS: i64 = 24 * 7;
pub const MAX_CONSENT: usize = 300;

impl SiteSettings {
    /// `get` is the settings cache's string lookup. A missing row is the
    /// default, never "off": the master switch must not read as disabled on a
    /// site whose migration has not run yet.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> SiteSettings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        SiteSettings {
            enabled: flag(SETTING_ENABLED, true),
            hide_on_mobile: flag(SETTING_HIDE_ON_MOBILE, false),
            cooldown_hours: get(SETTING_COOLDOWN_HOURS)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0)
                .clamp(0, MAX_COOLDOWN_HOURS),
            consent_text: clean(&get(SETTING_CONSENT_TEXT).unwrap_or_default(), MAX_CONSENT),
        }
    }
}

/// Normalise a path prefix the way the matcher expects: one leading slash, no
/// trailing one, no query or fragment, no whitespace.
///
/// `/tag/news/`, `tag/news`, and ` /tag/news?x=1 ` are the same rule, because
/// somebody typing a prefix into a text box types all three.
pub fn normalize_path(raw: &str) -> Option<String> {
    let cut = raw.trim();
    let cut = cut.split(['?', '#']).next().unwrap_or("");
    let trimmed = cut.trim().trim_matches('/');
    if trimmed.is_empty() {
        // The site root is a legitimate rule and survives the trimming above.
        return (cut.trim() == "/").then(|| "/".to_string());
    }
    let mut path = String::with_capacity(trimmed.len() + 1);
    path.push('/');
    path.push_str(&trimmed.chars().take(MAX_PATH).collect::<String>());
    Some(path)
}

/// Parse the admin's textarea of path prefixes: one per line, blanks and
/// `#` comments dropped, deduplicated, capped.
pub fn parse_paths(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(path) = normalize_path(line) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
        if out.len() >= MAX_PATHS {
            break;
        }
    }
    out
}

/// Whether a popup with this targeting belongs on `path`.
///
/// A prefix matches the path itself or anything below it: `/tag/news` matches
/// `/tag/news` and `/tag/news/page/2/` but **not** `/tag/newsletter`, which is
/// the bug a bare `starts_with` would ship. The site root `/` is special — as
/// a prefix it would match everything, so it means the home page alone.
pub fn matches_path(targeting: Targeting, paths: &[String], path: &str) -> bool {
    let path = normalize_path(path).unwrap_or_else(|| "/".into());
    let hit = paths.iter().any(|prefix| {
        if prefix == "/" {
            return path == "/";
        }
        path == *prefix || path.starts_with(&format!("{prefix}/"))
    });
    match targeting {
        Targeting::Everywhere => true,
        Targeting::Only => hit,
        Targeting::Except => !hit,
    }
}

/// Whether a popup is within its scheduled window. Either end may be absent.
///
/// `now`, `starts_at` and `ends_at` are database timestamp strings, which sort
/// lexicographically because they are all `YYYY-MM-DD HH:MM:SS` in UTC.
pub fn in_window(now: &str, starts_at: Option<&str>, ends_at: Option<&str>) -> bool {
    let started = starts_at.map(|s| s.trim()).filter(|s| !s.is_empty()).is_none_or(|s| s <= now);
    let not_ended = ends_at.map(|s| s.trim()).filter(|s| !s.is_empty()).is_none_or(|s| s > now);
    started && not_ended
}

/// Cut a string to a limit without splitting a character, and strip the
/// control characters that would otherwise travel into the page.
pub fn clean(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A plausible email address. Deliberately loose — the authority on whether
/// an address works is whether mail to it arrives — but it refuses the shapes
/// that are never addresses, which is most of what a bot sends.
pub fn is_email(raw: &str) -> bool {
    let value = raw.trim();
    if value.len() < 3 || value.len() > MAX_EMAIL || value.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else { return false };
    !local.is_empty()
        && domain.len() >= 3
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && value.matches('@').count() == 1
}

/// The conversion rate, as a percentage, or `None` when nothing has been
/// shown yet.
///
/// `None` rather than `0.0`: a popup nobody has seen has not converted
/// nobody — it has no rate at all, and a screen that prints "0%" for it is
/// reporting a measurement that was never taken.
pub fn conversion_rate(impressions: i64, submissions: i64) -> Option<f64> {
    (impressions > 0).then(|| submissions as f64 * 100.0 / impressions as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_matches_a_segment_not_a_substring() {
        let paths = vec!["/tag/news".to_string()];
        assert!(matches_path(Targeting::Only, &paths, "/tag/news"));
        assert!(matches_path(Targeting::Only, &paths, "/tag/news/"));
        assert!(matches_path(Targeting::Only, &paths, "/tag/news/page/2/"));
        // The bug a bare starts_with would ship.
        assert!(!matches_path(Targeting::Only, &paths, "/tag/newsletter/"));
        assert!(!matches_path(Targeting::Only, &paths, "/about/"));
    }

    #[test]
    fn the_site_root_as_a_rule_means_the_home_page_only() {
        let paths = vec!["/".to_string()];
        assert!(matches_path(Targeting::Only, &paths, "/"));
        assert!(!matches_path(Targeting::Only, &paths, "/about/"));
        // And as an exception it excludes only the home page.
        assert!(!matches_path(Targeting::Except, &paths, "/"));
        assert!(matches_path(Targeting::Except, &paths, "/about/"));
    }

    #[test]
    fn everywhere_ignores_the_list_entirely() {
        let paths = vec!["/about".to_string()];
        assert!(matches_path(Targeting::Everywhere, &paths, "/about/"));
        assert!(matches_path(Targeting::Everywhere, &paths, "/anything/"));
        assert!(matches_path(Targeting::Everywhere, &[], "/"));
    }

    #[test]
    fn paths_are_parsed_the_way_people_type_them() {
        let parsed = parse_paths("  tag/news/ \n/about\n\n# a comment\n/about\nhttps-ish?x=1\n");
        assert_eq!(parsed, vec!["/tag/news", "/about", "/https-ish"]);
        assert!(parse_paths("").is_empty());
        // and the cap holds
        let many = (0..100).map(|i| format!("/p{i}")).collect::<Vec<_>>().join("\n");
        assert_eq!(parse_paths(&many).len(), MAX_PATHS);
    }

    #[test]
    fn a_surface_matches_only_its_own_kind_of_page() {
        assert!(Surface::Any.includes("post") && Surface::Any.includes("anything"));
        assert!(Surface::Posts.includes("post") && !Surface::Posts.includes("page"));
        assert!(Surface::Pages.includes("page") && !Surface::Pages.includes("post"));
        assert!(Surface::Home.includes("home") && !Surface::Home.includes("post"));
        assert!(Surface::Archives.includes("tag") && Surface::Archives.includes("author"));
        assert!(!Surface::Archives.includes("home"));
        // A kind nothing recognises gets only the popups aimed at everything.
        assert!(!Surface::Posts.includes("gallery"));
        assert!(Surface::Any.includes("gallery"));
        assert!(!Surface::Any.needs_lookup() && Surface::Posts.needs_lookup());
    }

    #[test]
    fn missing_site_settings_read_as_the_defaults_not_as_off() {
        let none = SiteSettings::read(|_| None);
        assert!(none.enabled, "a missing master switch must not disable popups");
        assert!(!none.hide_on_mobile);
        assert_eq!(none.cooldown_hours, 0);
        assert_eq!(none.consent_text, "");

        let set = SiteSettings::read(|k| match k {
            SETTING_ENABLED => Some("false".into()),
            SETTING_HIDE_ON_MOBILE => Some("true".into()),
            SETTING_COOLDOWN_HOURS => Some("99999".into()),
            SETTING_CONSENT_TEXT => Some("  We send one email a week.\u{0}  ".into()),
            _ => None,
        });
        assert!(!set.enabled && set.hide_on_mobile);
        assert_eq!(set.cooldown_hours, MAX_COOLDOWN_HOURS, "clamped, not refused");
        assert_eq!(set.consent_text, "We send one email a week.");
        // A garbage number is the default, not an error.
        assert_eq!(SiteSettings::read(|k| (k == SETTING_COOLDOWN_HOURS).then(|| "soon".into())).cooldown_hours, 0);
    }

    #[test]
    fn an_audience_includes_the_right_people() {
        assert!(Audience::Everyone.includes(true) && Audience::Everyone.includes(false));
        assert!(Audience::LoggedOut.includes(false) && !Audience::LoggedOut.includes(true));
        assert!(Audience::Members.includes(true) && !Audience::Members.includes(false));
    }

    #[test]
    fn a_window_with_no_ends_is_always_open() {
        let now = "2026-11-20 12:00:00";
        assert!(in_window(now, None, None));
        assert!(in_window(now, Some(""), Some("   ")));
        assert!(in_window(now, Some("2026-11-01 00:00:00"), Some("2026-12-01 00:00:00")));
        assert!(!in_window(now, Some("2026-12-01 00:00:00"), None), "not started");
        assert!(!in_window(now, None, Some("2026-11-01 00:00:00")), "already ended");
    }

    #[test]
    fn trigger_values_are_clamped_to_something_usable() {
        assert_eq!(Trigger::Delay.clamp(0), 1);
        assert_eq!(Trigger::Delay.clamp(5), 5);
        assert_eq!(Trigger::Delay.clamp(9999), 120);
        assert_eq!(Trigger::Scroll.clamp(1), 5);
        assert_eq!(Trigger::Scroll.clamp(900), 100);
        assert_eq!(Trigger::Exit.clamp(42), 0, "exit intent carries no number");
        assert_eq!(Frequency::Days.clamp_days(0), 1);
        assert_eq!(Frequency::Once.clamp_days(30), 0);
    }

    #[test]
    fn emails_that_are_never_addresses_are_refused() {
        assert!(is_email("reader@example.com"));
        assert!(is_email("a+b@sub.example.co.uk"));
        for bad in ["", "reader", "@example.com", "reader@", "reader@com",
                    "reader@.com", "reader@example..com", "a b@example.com",
                    "two@@example.com"] {
            assert!(!is_email(bad), "{bad} should not be an address");
        }
        assert!(!is_email(&format!("{}@example.com", "a".repeat(MAX_EMAIL))));
    }

    #[test]
    fn clean_bounds_and_strips_control_characters() {
        assert_eq!(clean("  hello  ", 50), "hello");
        assert_eq!(clean("a\u{0}b\u{7}c", 50), "abc");
        assert_eq!(clean("line\nline", 50), "line\nline", "newlines survive");
        assert_eq!(clean(&"x".repeat(500), 10), "x".repeat(10));
        // a multi-byte character is not split
        let cut = clean("ééééé", 3);
        assert_eq!(cut.chars().count(), 3);
    }

    #[test]
    fn a_popup_nobody_saw_has_no_rate_at_all() {
        assert_eq!(conversion_rate(0, 0), None);
        assert_eq!(conversion_rate(100, 7), Some(7.0));
        assert_eq!(conversion_rate(3, 1).map(|r| r.round()), Some(33.0));
    }
}
