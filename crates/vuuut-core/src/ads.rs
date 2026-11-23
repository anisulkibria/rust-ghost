//! Advertising & Ad Management: the rules.
//!
//! Pure — no database, no HTTP — so the admin, the renderer and the event
//! endpoint ask the same functions. What lives here:
//!
//! * the vocabulary — [`Kind`], [`Placement`], [`Audience`], [`Device`],
//!   [`Targeting`] — each parsed from a form with a safe default, so a forged
//!   value is the default rather than an error page;
//! * [`AdInput::from_form`], the trust boundary for staff input: every string
//!   bounded, every link checked to be http(s) or a site path, numbers
//!   clamped;
//! * [`eligible`] — whether one ad may be shown to this reader on this page
//!   now: status, schedule, caps, audience and targeting, in that order;
//! * [`pick`] — weighted rotation among the eligible;
//! * [`place_in_content`] — where in a post's body the in-content ads go;
//! * the arithmetic of a report: CTR and revenue, where an unmeasured rate is
//!   `None` rather than 0%.
//!
//! The site-wide settings ship **off**: installing the tool shows no ads and
//! serves no ads.txt until somebody switches it on.
//!
//! Code ads (`Kind::Code`) are HTML that staff paste — an ad network's tag.
//! They are trusted exactly as far as Code injection is (Administrator and
//! above), and no further: the renderer places them inside a labelled
//! container, and the consent module still holds back any recognised
//! tracker inside them until a visitor agrees.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

macro_rules! keyed {
    ($name:ident, $default:ident, $( $variant:ident => $key:literal, $label:literal ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $( $variant ),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];
            pub fn key(self) -> &'static str { match self { $( $name::$variant => $key ),+ } }
            pub fn label(self) -> &'static str { match self { $( $name::$variant => $label ),+ } }
            pub fn from_key(key: &str) -> $name {
                let key = key.trim();
                $( if key == $key { return $name::$variant; } )+
                $name::$default
            }
        }
    };
}

keyed!(Kind, Image,
    Image => "image", "Image banner",
    Text => "text", "Text ad",
    Code => "code", "Ad network code",
);

keyed!(Placement, PostBottom,
    PostTop => "post_top", "Top of every post",
    PostParagraph => "post_paragraph", "Inside posts, after a paragraph",
    PostBottom => "post_bottom", "End of every post",
    Sticky => "sticky", "Sticky bar at the bottom of the screen",
    Slot => "slot", "Wherever a slot is placed",
);

keyed!(Audience, Everyone,
    Everyone => "everyone", "Everyone",
    Visitors => "visitors", "Visitors who are not signed in",
    Free => "free", "Free members only",
    NotPaid => "not_paid", "Everyone except paying members",
);

keyed!(Device, All,
    All => "all", "All devices",
    Desktop => "desktop", "Desktop only",
    Mobile => "mobile", "Phones only",
);

keyed!(Targeting, Everywhere,
    Everywhere => "everywhere", "Every page",
    Only => "only", "Only these pages and tags",
    Except => "except", "Everywhere except these pages and tags",
);

keyed!(Status, Draft,
    Draft => "draft", "Draft",
    Live => "live", "Live",
    Paused => "paused", "Paused",
);

impl Placement {
    /// In-content placements go into the post body before the theme renders
    /// it; the rest are added to the finished page.
    pub fn in_content(self) -> bool {
        matches!(self, Placement::PostTop | Placement::PostParagraph | Placement::PostBottom)
    }
}

// ---------------------------------------------------------------------------
// site settings
// ---------------------------------------------------------------------------

pub const SETTING_ENABLED: &str = "ads_enabled";
pub const SETTING_LABEL: &str = "ads_label";
pub const SETTING_HIDE_PAID: &str = "ads_hide_paid";
pub const SETTING_ADS_TXT: &str = "ads_txt";
pub const SETTING_MAX_IN_CONTENT: &str = "ads_max_in_content";

pub const DEFAULT_LABEL: &str = "Advertisement";
pub const MAX_LABEL: usize = 40;
/// ads.txt is a list of seller lines; big networks publish a few hundred.
pub const MAX_ADS_TXT: usize = 14_000;

#[derive(Debug, Clone, PartialEq)]
pub struct SiteSettings {
    /// The master switch. Off: no ad anywhere, no script, no ads.txt.
    pub enabled: bool,
    /// The disclosure every ad carries. Advertising law in most places wants
    /// an ad to be recognisable as one; blank falls back to the default
    /// rather than to nothing.
    pub label: String,
    /// Paying members see no ads at all — the most common thing a membership
    /// site promises its subscribers.
    pub hide_paid: bool,
    pub ads_txt: String,
    /// At most this many ads inside one post's body.
    pub max_in_content: i64,
}

impl Default for SiteSettings {
    fn default() -> Self {
        SiteSettings::read(|_| None)
    }
}

impl SiteSettings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> SiteSettings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let label = clean(&get(SETTING_LABEL).unwrap_or_default(), MAX_LABEL);
        SiteSettings {
            enabled: flag(SETTING_ENABLED, false),
            label: if label.is_empty() { DEFAULT_LABEL.into() } else { label },
            hide_paid: flag(SETTING_HIDE_PAID, true),
            ads_txt: ads_txt(&get(SETTING_ADS_TXT).unwrap_or_default()),
            max_in_content: get(SETTING_MAX_IN_CONTENT)
                .and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok())
                .unwrap_or(3)
                .clamp(1, 6),
        }
    }
}

/// ads.txt, normalised: lines kept as typed but bounded, control characters
/// dropped, `\r\n` folded. It is served as `text/plain`, so there is nothing
/// to escape — only something to bound.
pub fn ads_txt(raw: &str) -> String {
    let text: String = raw
        .replace("\r\n", "\n")
        .replace("\\n", "\n")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(MAX_ADS_TXT)
        .collect();
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Lines of ads.txt that do not look like a record (`domain, id, DIRECT|RESELLER[, cert]`),
/// a comment, or a variable (`contact=`, `subdomain=`…) — shown as warnings,
/// never rejected, because the format is loose in practice.
pub fn ads_txt_problems(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let l = line.split('#').next().unwrap_or("").trim();
            if l.is_empty() || l.contains('=') {
                return None;
            }
            let fields: Vec<&str> = l.split(',').map(str::trim).collect();
            let ok = fields.len() >= 3
                && fields[0].contains('.')
                && !fields[1].is_empty()
                && matches!(fields[2].to_ascii_uppercase().as_str(), "DIRECT" | "RESELLER");
            (!ok).then(|| (i + 1, line.to_string()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// staff input
// ---------------------------------------------------------------------------

pub const MAX_NAME: usize = 120;
pub const MAX_URL: usize = 1_000;
pub const MAX_ALT: usize = 200;
pub const MAX_HEADLINE: usize = 90;
pub const MAX_BODY: usize = 280;
pub const MAX_CTA: usize = 30;
pub const MAX_CODE: usize = 6_000;
pub const MAX_PATHS: usize = 2_000;
pub const MAX_PARAGRAPH: i64 = 50;
pub const MAX_WEIGHT: i64 = 100;
/// Prices are in cents of the site's currency.
pub const MAX_PRICE_CENTS: i64 = 1_000_000;

pub fn clean(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A link an ad may point at, or show: an absolute http(s) URL or a path on
/// this site. Never `javascript:`, `data:` or `//host`.
pub fn safe_url(raw: &str) -> Option<String> {
    let url = clean(raw, MAX_URL);
    if url.is_empty() {
        return Some(String::new());
    }
    if url.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '\\' | '`')) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if (url.starts_with('/') && !url.starts_with("//")) || lower.starts_with("https://") || lower.starts_with("http://") {
        Some(url)
    } else {
        None
    }
}

/// One path prefix or `tag:slug` per line, reduced to what can match.
pub fn parse_rules(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            if let Some(tag) = l.strip_prefix("tag:") {
                let slug: String = tag.trim().to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                (!slug.is_empty()).then(|| format!("tag:{slug}"))
            } else if l.starts_with('/') && !l.starts_with("//") {
                Some(l.chars().filter(|c| !c.is_whitespace() && !matches!(c, '"' | '<' | '>')).take(300).collect())
            } else {
                None
            }
        })
        .take(100)
        .collect()
}

/// What a save writes, rebuilt from a form.
#[derive(Debug, Clone, PartialEq)]
pub struct AdInput {
    pub name: String,
    pub advertiser_id: Option<String>,
    pub status: Status,
    pub kind: Kind,
    pub placement: Placement,
    pub paragraph: i64,
    pub slot: String,
    pub image: String,
    pub link: String,
    pub alt: String,
    pub headline: String,
    pub body: String,
    pub cta: String,
    pub code: String,
    pub audience: Audience,
    pub device: Device,
    pub targeting: Targeting,
    pub rules: Vec<String>,
    pub weight: i64,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub max_impressions: i64,
    pub max_clicks: i64,
    pub cpm_cents: i64,
    pub cpc_cents: i64,
}

/// A slot name: what an author writes in `data-ad-slot="…"`.
pub fn slot_name(raw: &str) -> String {
    raw.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect()
}

impl AdInput {
    /// Every field from the form, validated. `Err` names the first problem a
    /// person can fix; nothing is ever silently dropped that they typed.
    /// `advertiser_exists` says whether an advertiser id is real.
    pub fn from_form(form: &HashMap<String, String>, advertiser_exists: impl Fn(&str) -> bool) -> Result<AdInput, String> {
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let num = |k: &str, max: i64| get(k).trim().parse::<i64>().unwrap_or(0).clamp(0, max);
        let money = |k: &str| -> i64 {
            // Typed as an amount ("2.50"), stored in cents.
            let v = get(k).trim().replace(',', ".");
            v.parse::<f64>().map(|f| (f * 100.0).round() as i64).unwrap_or(0).clamp(0, MAX_PRICE_CENTS)
        };
        let name = clean(get("name"), MAX_NAME);
        if name.is_empty() {
            return Err("Give the ad a name — it is how you will find it in reports.".into());
        }
        let kind = Kind::from_key(get("kind"));
        let placement = Placement::from_key(get("placement"));
        let link = safe_url(get("link")).ok_or("The ad's link must be an http(s) address or a path on this site.")?;
        let image = safe_url(get("image")).ok_or("The image must be an http(s) address or a path on this site.")?;
        let code = get("code").trim().chars().take(MAX_CODE).collect::<String>();
        match kind {
            Kind::Image if image.is_empty() => return Err("An image banner needs an image.".into()),
            Kind::Image | Kind::Text if link.is_empty() => return Err("Where should the ad lead? Add its link.".into()),
            Kind::Text if clean(get("headline"), MAX_HEADLINE).is_empty() => return Err("A text ad needs a headline.".into()),
            Kind::Code if code.is_empty() => return Err("Paste the ad network's code.".into()),
            _ => {}
        }
        let slot = slot_name(get("slot"));
        if placement == Placement::Slot && slot.is_empty() {
            return Err("Name the slot this ad fills, for example “sidebar”.".into());
        }
        let advertiser_id = Some(get("advertiser_id").trim().to_string()).filter(|id| !id.is_empty() && advertiser_exists(id));
        let date = |k: &str| {
            let v = get(k).trim();
            crate::dates::from_db_string(&format!("{} 00:00:00", v.get(..10).unwrap_or("")))
                .map(|d| crate::dates::to_db_string(&d))
        };
        let starts_at = date("starts_at");
        let ends_at = date("ends_at").map(|e| e.replace(" 00:00:00", " 23:59:59"));
        if let (Some(s), Some(e)) = (&starts_at, &ends_at) {
            if e < s {
                return Err("The end date is before the start date.".into());
            }
        }
        let weight = num("weight", MAX_WEIGHT);
        Ok(AdInput {
            name,
            advertiser_id,
            status: Status::from_key(get("status")),
            kind,
            placement,
            paragraph: num("paragraph", MAX_PARAGRAPH).max(1),
            slot,
            image,
            link,
            alt: clean(get("alt"), MAX_ALT),
            headline: clean(get("headline"), MAX_HEADLINE),
            body: clean(get("body"), MAX_BODY),
            cta: clean(get("cta"), MAX_CTA),
            code,
            audience: Audience::from_key(get("audience")),
            device: Device::from_key(get("device")),
            targeting: Targeting::from_key(get("targeting")),
            rules: parse_rules(&get("rules").chars().take(MAX_PATHS).collect::<String>()),
            weight: if weight == 0 { 1 } else { weight },
            starts_at,
            ends_at,
            max_impressions: num("max_impressions", i64::MAX / 4),
            max_clicks: num("max_clicks", i64::MAX / 4),
            cpm_cents: money("cpm"),
            cpc_cents: money("cpc"),
        })
    }
}

// ---------------------------------------------------------------------------
// who sees what
// ---------------------------------------------------------------------------

/// The reader and the page, as the renderer knows them.
#[derive(Debug, Clone, Default)]
pub struct Viewer<'a> {
    /// `None` for a visitor; otherwise the member status: free, paid, comped.
    pub member: Option<&'a str>,
    pub path: &'a str,
    /// Slugs of the post's tags; empty on anything that is not a post.
    pub tags: &'a [String],
    /// `"post"`, `"page"`, or `""` for archives and the home page.
    pub kind: &'a str,
    pub now: &'a str,
}

impl Viewer<'_> {
    pub fn pays(&self) -> bool {
        matches!(self.member, Some("paid") | Some("comped"))
    }
}

/// The parts of a stored ad that decide whether it shows.
#[derive(Debug, Clone)]
pub struct Rule<'a> {
    pub status: Status,
    pub placement: Placement,
    pub audience: Audience,
    pub targeting: Targeting,
    pub rules: &'a [String],
    pub starts_at: Option<&'a str>,
    pub ends_at: Option<&'a str>,
    pub max_impressions: i64,
    pub max_clicks: i64,
    pub impressions: i64,
    pub clicks: i64,
}

/// Why an ad is not running — the admin's list says it, so "Live" never
/// sits beside an ad nobody is seeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    NotLive,
    NotStarted,
    Ended,
    ImpressionsUsed,
    ClicksUsed,
}

impl Blocked {
    pub fn label(self) -> &'static str {
        match self {
            Blocked::NotLive => "Not live",
            Blocked::NotStarted => "Scheduled",
            Blocked::Ended => "Ended",
            Blocked::ImpressionsUsed => "Impressions used up",
            Blocked::ClicksUsed => "Clicks used up",
        }
    }
}

/// Whether an ad is running at all at `now`, regardless of who is looking.
pub fn running(r: &Rule, now: &str) -> Result<(), Blocked> {
    if r.status != Status::Live {
        return Err(Blocked::NotLive);
    }
    if r.starts_at.is_some_and(|s| now < s) {
        return Err(Blocked::NotStarted);
    }
    if r.ends_at.is_some_and(|e| now > e) {
        return Err(Blocked::Ended);
    }
    if r.max_impressions > 0 && r.impressions >= r.max_impressions {
        return Err(Blocked::ImpressionsUsed);
    }
    if r.max_clicks > 0 && r.clicks >= r.max_clicks {
        return Err(Blocked::ClicksUsed);
    }
    Ok(())
}

/// Whether this reader may be shown this ad on this page.
pub fn eligible(r: &Rule, v: &Viewer, site: &SiteSettings) -> bool {
    if !site.enabled || running(r, v.now).is_err() {
        return false;
    }
    if site.hide_paid && v.pays() {
        return false;
    }
    let audience_ok = match r.audience {
        Audience::Everyone => true,
        Audience::Visitors => v.member.is_none(),
        Audience::Free => v.member == Some("free"),
        Audience::NotPaid => !v.pays(),
    };
    if !audience_ok {
        return false;
    }
    // In-content ads only exist where there is a post body to put them in.
    if r.placement.in_content() && v.kind != "post" {
        return false;
    }
    let hit = r.rules.iter().any(|rule| match rule.strip_prefix("tag:") {
        Some(tag) => v.tags.iter().any(|t| t == tag),
        None => v.path.starts_with(rule.as_str()),
    });
    match r.targeting {
        Targeting::Everywhere => true,
        Targeting::Only => hit,
        Targeting::Except => !hit,
    }
}

/// Weighted rotation: an ad of weight 3 shows three times as often as one of
/// weight 1. `roll` is a number in `0..total` the caller draws, so the
/// choice is testable.
pub fn pick<T>(candidates: &[(T, i64)], roll: u64) -> Option<&T> {
    let total: u64 = candidates.iter().map(|(_, w)| (*w).max(1) as u64).sum();
    if total == 0 {
        return None;
    }
    let mut at = roll % total;
    for (item, w) in candidates {
        let w = (*w).max(1) as u64;
        if at < w {
            return Some(item);
        }
        at -= w;
    }
    None
}

/// Insert ads into a post body. `top` goes first, `bottom` last, and each
/// `(n, markup)` after the n-th top-level paragraph — or at the end when the
/// post is shorter, so a short post still carries its ad rather than losing
/// it. Paragraphs are counted by `</p>` outside any `<figure>`, `<blockquote>`
/// or list, which is where an ad between paragraphs belongs.
pub fn place_in_content(html: &str, top: Option<&str>, after: &[(i64, String)], bottom: Option<&str>) -> String {
    let lower = html.to_ascii_lowercase();
    let mut ends: Vec<usize> = Vec::new();
    let mut depth: i32 = 0;
    let mut i = 0;
    while let Some(rel) = lower[i..].find('<') {
        let at = i + rel;
        let rest = &lower[at..];
        let opens = ["<figure", "<blockquote", "<ul", "<ol", "<table", "<aside", "<div class=\"kg-"];
        let closes = ["</figure>", "</blockquote>", "</ul>", "</ol>", "</table>", "</aside>"];
        if opens.iter().any(|o| rest.starts_with(o)) {
            depth += 1;
        } else if closes.iter().any(|c| rest.starts_with(c)) {
            depth = (depth - 1).max(0);
        } else if rest.starts_with("</p>") && depth == 0 {
            ends.push(at + 4);
        }
        i = at + 1;
    }
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for (n, markup) in after {
        let idx = (*n as usize).max(1) - 1;
        let pos = ends.get(idx).copied().unwrap_or(html.len());
        inserts.push((pos, markup.clone()));
    }
    inserts.sort_by_key(|(p, _)| *p);
    let mut out = String::with_capacity(html.len() + 512);
    if let Some(t) = top {
        out.push_str(t);
    }
    let mut last = 0;
    for (pos, markup) in inserts {
        out.push_str(&html[last..pos]);
        out.push_str(&markup);
        last = pos;
    }
    out.push_str(&html[last..]);
    if let Some(b) = bottom {
        out.push_str(b);
    }
    out
}

// ---------------------------------------------------------------------------
// reports
// ---------------------------------------------------------------------------

/// Clicks per impression, or `None` when nobody has seen it.
pub fn ctr(impressions: i64, clicks: i64) -> Option<f64> {
    (impressions > 0).then(|| clicks as f64 * 100.0 / impressions as f64)
}

/// What an ad earned, in cents: CPM per thousand impressions plus CPC per
/// click. Rounded down — a report should never promise more than was earned.
pub fn revenue_cents(impressions: i64, clicks: i64, cpm_cents: i64, cpc_cents: i64) -> i64 {
    impressions.max(0) * cpm_cents.max(0) / 1000 + clicks.max(0) * cpc_cents.max(0)
}

/// `1234567` cents → `12,345.67`.
pub fn money(cents: i64) -> String {
    let whole = cents / 100;
    let frac = (cents % 100).abs();
    let digits = whole.abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{grouped}.{frac:02}", if cents < 0 { "-" } else { "" })
}

/// Event ids from the visitor script: at most this many per request.
pub const MAX_EVENTS: usize = 12;

#[cfg(test)]
mod tests {
    use super::*;

    fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn rule<'a>(rules: &'a [String]) -> Rule<'a> {
        Rule {
            status: Status::Live,
            placement: Placement::PostBottom,
            audience: Audience::Everyone,
            targeting: Targeting::Everywhere,
            rules,
            starts_at: None,
            ends_at: None,
            max_impressions: 0,
            max_clicks: 0,
            impressions: 0,
            clicks: 0,
        }
    }

    fn on() -> SiteSettings {
        SiteSettings::read(|k| (k == SETTING_ENABLED).then(|| "true".into()))
    }

    fn viewer<'a>(member: Option<&'a str>, path: &'a str, tags: &'a [String]) -> Viewer<'a> {
        Viewer { member, path, tags, kind: "post", now: "2026-11-21 12:00:00" }
    }

    #[test]
    fn installing_shows_nothing_and_paid_members_are_spared_by_default() {
        let s = SiteSettings::default();
        assert!(!s.enabled && s.hide_paid);
        assert_eq!(s.label, DEFAULT_LABEL);
        let none: Vec<String> = vec![];
        assert!(!eligible(&rule(&none), &viewer(None, "/a/", &none), &s));
        assert!(eligible(&rule(&none), &viewer(None, "/a/", &none), &on()));
        assert!(!eligible(&rule(&none), &viewer(Some("paid"), "/a/", &none), &on()));
        assert!(!eligible(&rule(&none), &viewer(Some("comped"), "/a/", &none), &on()));
    }

    #[test]
    fn a_link_that_could_run_is_refused() {
        for bad in ["javascript:alert(1)", "//evil.example", "data:text/html,x", "https://a.b/\"onerror", "JAVASCRIPT:x"] {
            assert_eq!(safe_url(bad), None, "{bad}");
        }
        assert_eq!(safe_url("https://shop.example/x?y=1").as_deref(), Some("https://shop.example/x?y=1"));
        assert_eq!(safe_url("/sponsor/").as_deref(), Some("/sponsor/"));
    }

    #[test]
    fn the_form_is_rebuilt_and_bounded() {
        let f = form(&[
            ("name", "  Spring sale  "),
            ("kind", "image"),
            ("image", "https://cdn.example/a.png"),
            ("link", "https://shop.example/"),
            ("placement", "bogus"),
            ("audience", "<script>"),
            ("paragraph", "999"),
            ("weight", "0"),
            ("cpm", "2.50"),
            ("cpc", "0,4"),
            ("rules", "/blog/\njavascript:x\ntag:News\n//evil"),
            ("advertiser_id", "nope"),
        ]);
        let ad = AdInput::from_form(&f, |_| false).unwrap();
        assert_eq!(ad.name, "Spring sale");
        assert_eq!(ad.placement, Placement::PostBottom);
        assert_eq!(ad.audience, Audience::Everyone);
        assert_eq!((ad.paragraph, ad.weight), (MAX_PARAGRAPH, 1));
        assert_eq!((ad.cpm_cents, ad.cpc_cents), (250, 40));
        assert_eq!(ad.rules, ["/blog/", "tag:news"]);
        assert_eq!(ad.advertiser_id, None, "an unknown advertiser is not linked");
    }

    #[test]
    fn each_kind_says_what_it_is_missing() {
        assert!(AdInput::from_form(&form(&[("name", "x"), ("kind", "image"), ("link", "/a/")]), |_| true).unwrap_err().contains("image"));
        assert!(AdInput::from_form(&form(&[("name", "x"), ("kind", "text"), ("link", "/a/")]), |_| true).unwrap_err().contains("headline"));
        assert!(AdInput::from_form(&form(&[("name", "x"), ("kind", "code")]), |_| true).unwrap_err().contains("code"));
        assert!(AdInput::from_form(&form(&[("name", "x"), ("kind", "code"), ("code", "<b>"), ("placement", "slot")]), |_| true).unwrap_err().contains("slot"));
        assert!(AdInput::from_form(&form(&[("name", "x"), ("kind", "code"), ("code", "<b>"), ("starts_at", "2026-12-01"), ("ends_at", "2026-11-01")]), |_| true).unwrap_err().contains("before"));
        assert!(AdInput::from_form(&form(&[("kind", "code"), ("code", "<b>")]), |_| true).unwrap_err().contains("name"));
    }

    #[test]
    fn schedule_and_caps_stop_an_ad_and_say_why() {
        let none: Vec<String> = vec![];
        let mut r = rule(&none);
        r.starts_at = Some("2026-12-01 00:00:00");
        assert_eq!(running(&r, "2026-11-21 12:00:00"), Err(Blocked::NotStarted));
        r.starts_at = None;
        r.ends_at = Some("2026-11-01 23:59:59");
        assert_eq!(running(&r, "2026-11-21 12:00:00"), Err(Blocked::Ended));
        r.ends_at = None;
        r.max_impressions = 100;
        r.impressions = 100;
        assert_eq!(running(&r, "2026-11-21 12:00:00"), Err(Blocked::ImpressionsUsed));
        r.max_impressions = 0;
        r.max_clicks = 5;
        r.clicks = 5;
        assert_eq!(running(&r, "2026-11-21 12:00:00"), Err(Blocked::ClicksUsed));
    }

    #[test]
    fn audiences_and_targeting_narrow_who_sees_it() {
        let none: Vec<String> = vec![];
        let mut r = rule(&none);
        let site = SiteSettings::read(|k| match k {
            SETTING_ENABLED => Some("true".into()),
            SETTING_HIDE_PAID => Some("false".into()),
            _ => None,
        });
        r.audience = Audience::Visitors;
        assert!(eligible(&r, &viewer(None, "/", &none), &site));
        assert!(!eligible(&r, &viewer(Some("free"), "/", &none), &site));
        r.audience = Audience::NotPaid;
        assert!(eligible(&r, &viewer(Some("free"), "/", &none), &site));
        assert!(!eligible(&r, &viewer(Some("paid"), "/", &none), &site));

        let rules = vec!["/guides/".to_string(), "tag:tech".to_string()];
        let mut t = rule(&rules);
        t.targeting = Targeting::Only;
        let tech = vec!["tech".to_string()];
        assert!(eligible(&t, &viewer(None, "/guides/a/", &none), &on()));
        assert!(eligible(&t, &viewer(None, "/x/", &tech), &on()));
        assert!(!eligible(&t, &viewer(None, "/x/", &none), &on()));
        t.targeting = Targeting::Except;
        assert!(!eligible(&t, &viewer(None, "/guides/a/", &none), &on()));
        // In-content ads need a post.
        let mut page = viewer(None, "/about/", &none);
        page.kind = "page";
        assert!(!eligible(&rule(&none), &page, &on()));
    }

    #[test]
    fn rotation_follows_weight() {
        let c = vec![("a", 1), ("b", 3)];
        let counts = (0..400u64).fold((0, 0), |(a, b), roll| match pick(&c, roll) {
            Some(&"a") => (a + 1, b),
            _ => (a, b + 1),
        });
        assert_eq!(counts, (100, 300));
        assert_eq!(pick::<&str>(&[], 3), None);
    }

    #[test]
    fn in_content_ads_go_between_top_level_paragraphs() {
        let html = "<p>one</p><figure><p>caption</p></figure><p>two</p><blockquote><p>q</p></blockquote><p>three</p>";
        let out = place_in_content(html, Some("[T]"), &[(2, "[A2]".into()), (9, "[A9]".into())], Some("[B]"));
        assert_eq!(out, "[T]<p>one</p><figure><p>caption</p></figure><p>two</p>[A2]<blockquote><p>q</p></blockquote><p>three</p>[A9][B]");
    }

    #[test]
    fn reports_never_invent_a_rate_or_round_revenue_up() {
        assert_eq!(ctr(0, 0), None);
        assert_eq!(ctr(200, 3), Some(1.5));
        assert_eq!(revenue_cents(1999, 2, 250, 40), 499 + 80);
        assert_eq!(money(123456789), "1,234,567.89");
        assert_eq!(money(5), "0.05");
    }

    #[test]
    fn ads_txt_is_bounded_and_odd_lines_are_flagged_not_dropped() {
        let t = ads_txt("google.com, pub-1, DIRECT, f08c47fec0942fa0\r\n# comment\ncontact=ads@x.com\nnot a record\u{0007}");
        assert_eq!(t.lines().count(), 4);
        assert_eq!(ads_txt_problems(&t), vec![(4, "not a record".to_string())]);
    }
}
