//! Caching & Performance: the rules.
//!
//! Pure. Every decision that could serve the wrong page to the wrong person
//! is made here and unit-tested:
//!
//! * [`request_key`] — whether a request may be answered from the page cache
//!   at all, and under which key. Only anonymous GETs: a member, a staff
//!   session, a private-site visitor, a preview, an API call or an excluded
//!   path is never served a stored page, because what they see differs from
//!   what an anonymous reader sees. Tracking parameters (`utm_*`, `fbclid`…)
//!   are dropped from the key so they do not split the cache; any other
//!   query is kept, sorted.
//! * [`response_storable`] — whether a rendered page may be stored: a 200
//!   HTML page that sets no cookie, is not marked private or no-store, and
//!   carries no form token (a form's anti-spam token is per visitor and
//!   expires; storing it would break the form for everybody after).
//! * [`cache_control`] and [`etag`] — the headers a public page carries.
//! * [`lazy_images`], [`speculation_rules`], [`preconnect`] — front-end
//!   optimisations, each a pure rewrite or a tag.
//! * [`page_facts`] and [`audit`] — what the Overview says about the site.
//!
//! Every setting ships **off**: installing the tool changes no response
//! until somebody switches something on.

use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub const SETTING_PAGE_CACHE: &str = "cache_pages";
pub const SETTING_TTL: &str = "cache_ttl";
pub const SETTING_MAX_PAGES: &str = "cache_max_pages";
pub const SETTING_EXCLUDE: &str = "cache_exclude";
pub const SETTING_BROWSER_TTL: &str = "cache_browser_ttl";
pub const SETTING_CDN_TTL: &str = "cache_cdn_ttl";
pub const SETTING_ETAG: &str = "cache_etag";
pub const SETTING_GZIP: &str = "cache_gzip";
pub const SETTING_LAZY: &str = "perf_lazy_images";
pub const SETTING_SPECULATION: &str = "perf_speculation";
pub const SETTING_PRECONNECT: &str = "perf_preconnect";
pub const SETTING_CDN_AUTO: &str = "cache_cdn_auto_purge";

/// Offered page-cache lifetimes, in seconds.
pub const TTLS: [i64; 6] = [60, 300, 900, 3600, 21_600, 86_400];
/// Offered browser lifetimes for HTML.
pub const BROWSER_TTLS: [i64; 4] = [0, 60, 300, 3600];
/// Offered CDN (`s-maxage`) lifetimes.
pub const CDN_TTLS: [i64; 5] = [0, 60, 300, 3600, 86_400];
pub const MAX_PAGES_RANGE: (i64, i64) = (50, 5000);
/// A page bigger than this is not stored: it is not a page any more.
pub const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speculation {
    Off,
    /// Prerender a link when the reader hovers or presses it.
    Moderate,
    /// Prerender likely links as soon as the page is idle.
    Eager,
}

impl Speculation {
    pub fn key(self) -> &'static str {
        match self {
            Speculation::Off => "off",
            Speculation::Moderate => "moderate",
            Speculation::Eager => "eager",
        }
    }
    pub fn from_key(k: &str) -> Speculation {
        match k.trim().trim_matches('"') {
            "moderate" => Speculation::Moderate,
            "eager" => Speculation::Eager,
            _ => Speculation::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub page_cache: bool,
    pub ttl: i64,
    pub max_pages: i64,
    /// Path prefixes never cached, one per line.
    pub exclude: Vec<String>,
    pub browser_ttl: i64,
    pub cdn_ttl: i64,
    pub etag: bool,
    pub gzip: bool,
    pub lazy_images: bool,
    pub speculation: Speculation,
    /// `https://origin` values to preconnect to.
    pub preconnect: Vec<String>,
    /// Purge the CDN whenever content changes.
    pub cdn_auto_purge: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |k: &str| matches!(get(k).as_deref().map(str::trim), Some("true") | Some("1"));
        let num = |k: &str| get(k).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let offered = |k: &str, list: &[i64], default: i64| num(k).filter(|n| list.contains(n)).unwrap_or(default);
        Settings {
            page_cache: flag(SETTING_PAGE_CACHE),
            ttl: offered(SETTING_TTL, &TTLS, 300),
            max_pages: num(SETTING_MAX_PAGES).unwrap_or(500).clamp(MAX_PAGES_RANGE.0, MAX_PAGES_RANGE.1),
            exclude: parse_paths(&get(SETTING_EXCLUDE).unwrap_or_default()),
            browser_ttl: offered(SETTING_BROWSER_TTL, &BROWSER_TTLS, 0),
            cdn_ttl: offered(SETTING_CDN_TTL, &CDN_TTLS, 0),
            etag: flag(SETTING_ETAG),
            gzip: flag(SETTING_GZIP),
            lazy_images: flag(SETTING_LAZY),
            speculation: Speculation::from_key(&get(SETTING_SPECULATION).unwrap_or_default()),
            preconnect: parse_origins(&get(SETTING_PRECONNECT).unwrap_or_default()),
            cdn_auto_purge: flag(SETTING_CDN_AUTO),
        }
    }
}

/// Path prefixes, one per line: each starts with `/`, no spaces, bounded.
pub fn parse_paths(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in raw.replace("\\n", "\n").lines().map(str::trim) {
        if l.starts_with('/') && !l.starts_with("//") && l.len() <= 200 && !l.contains(char::is_whitespace) && !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
        if out.len() == 50 {
            break;
        }
    }
    out
}

/// `https://host[:port]` origins, one per line — nothing else, because each
/// becomes a `<link rel="preconnect">` on every page.
pub fn parse_origins(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in raw.replace("\\n", "\n").lines().map(|l| l.trim().trim_end_matches('/')) {
        let Some(host) = l.strip_prefix("https://") else { continue };
        let ok = !host.is_empty()
            && host.contains('.')
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
        if ok && !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
        if out.len() == 8 {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// what may be cached
// ---------------------------------------------------------------------------

/// Why a request or a response was not served from or stored in the cache —
/// shown in the `X-Cache` header and counted on the Overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bypass {
    Off,
    Method,
    SignedIn,
    Private,
    System,
    Excluded,
    Query,
    Status,
    NotHtml,
    NoStore,
    SetsCookie,
    Form,
    TooBig,
}

impl Bypass {
    pub fn label(self) -> &'static str {
        match self {
            Bypass::Off => "cache off",
            Bypass::Method => "not a GET",
            Bypass::SignedIn => "signed in",
            Bypass::Private => "private site",
            Bypass::System => "not a page",
            Bypass::Excluded => "excluded path",
            Bypass::Query => "query string",
            Bypass::Status => "not a 200",
            Bypass::NotHtml => "not HTML",
            Bypass::NoStore => "marked private",
            Bypass::SetsCookie => "sets a cookie",
            Bypass::Form => "has a form",
            Bypass::TooBig => "too large",
        }
    }
}

/// Cookies that mean the visitor sees something an anonymous reader does
/// not: a member, staff, a private-site pass.
pub const PERSONAL_COOKIES: [&str; 5] = ["vuuut-members-ssr", "vuuut-admin-session", "ghost-private", "ghost-members-ssr", "ghost-admin-api-session"];

/// Parameters that only say where a reader came from.
fn tracking(param: &str) -> bool {
    param.starts_with("utm_") || matches!(param, "fbclid" | "gclid" | "dclid" | "msclkid" | "mc_cid" | "mc_eid" | "ref" | "_ga")
}

/// Paths that are never pages: the admin, APIs, previews, members, feeds of
/// per-request data.
fn system(path: &str) -> bool {
    const PREFIXES: [&str; 12] = [
        "/ghost", "/members/", "/p/", "/.ghost/", "/.well-known/", "/webmentions/",
        "/cookies/api/", "/search/api/", "/ads/api/", "/popups/api/", "/forms/", "/unsubscribe",
    ];
    PREFIXES.iter().any(|p| path.starts_with(p))
}

/// The key a request is cached under, or why it is not cacheable.
pub fn request_key(
    s: &Settings,
    private_site: bool,
    method: &str,
    path: &str,
    query: Option<&str>,
    cookie_header: &str,
) -> Result<String, Bypass> {
    if !s.page_cache {
        return Err(Bypass::Off);
    }
    if !matches!(method, "GET" | "HEAD") {
        return Err(Bypass::Method);
    }
    if private_site {
        return Err(Bypass::Private);
    }
    if system(path) {
        return Err(Bypass::System);
    }
    if s.exclude.iter().any(|e| path.starts_with(e.as_str())) {
        return Err(Bypass::Excluded);
    }
    let personal = cookie_header.split(';').filter_map(|c| c.split_once('=')).any(|(name, _)| {
        let n = name.trim();
        PERSONAL_COOKIES.iter().any(|p| n == *p || n.starts_with(&format!("{p}.")))
    });
    if personal {
        return Err(Bypass::SignedIn);
    }
    let mut kept: BTreeMap<String, String> = BTreeMap::new();
    for pair in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if !tracking(k) {
            kept.insert(k.to_string(), v.to_string());
        }
    }
    let q = kept.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    if q.len() > 200 {
        return Err(Bypass::Query);
    }
    Ok(if q.is_empty() { path.to_string() } else { format!("{path}?{q}") })
}

/// Whether a rendered response may be stored.
pub fn response_storable(status: u16, content_type: &str, cache_control: &str, sets_cookie: bool, body: &[u8]) -> Result<(), Bypass> {
    if status != 200 {
        return Err(Bypass::Status);
    }
    if !content_type.starts_with("text/html") {
        return Err(Bypass::NotHtml);
    }
    let cc = cache_control.to_ascii_lowercase();
    if cc.contains("no-store") || cc.contains("private") {
        return Err(Bypass::NoStore);
    }
    if sets_cookie {
        return Err(Bypass::SetsCookie);
    }
    if body.len() > MAX_PAGE_BYTES {
        return Err(Bypass::TooBig);
    }
    // A form card's anti-spam token is issued per request and expires.
    if contains(body, b"name=\"_token\"") || contains(body, b"data-vf-form") {
        return Err(Bypass::Form);
    }
    Ok(())
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// `Cache-Control` for a public, anonymous HTML page. With nothing set it is
/// what browsers assume anyway: check every time.
pub fn cache_control(s: &Settings) -> String {
    let mut parts = vec!["public".to_string(), format!("max-age={}", s.browser_ttl)];
    if s.browser_ttl == 0 {
        parts.push("must-revalidate".into());
    }
    if s.cdn_ttl > 0 {
        parts.push(format!("s-maxage={}", s.cdn_ttl));
        // While the CDN fetches a fresh copy it may keep serving the stale
        // one for as long again, so readers never wait on the origin.
        parts.push(format!("stale-while-revalidate={}", s.cdn_ttl));
    }
    parts.join(", ")
}

/// A weak validator from the body: FNV-1a, 64 bits. Weak, because the same
/// page compressed and uncompressed carries the same tag.
pub fn etag(body: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in body {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("W/\"{h:016x}\"")
}

/// Whether an `If-None-Match` header matches a tag (any of a list, `*`, and
/// weak and strong forms compared weakly, as RFC 9110 says for GET).
pub fn etag_matches(if_none_match: &str, tag: &str) -> bool {
    let bare = |t: &str| t.trim().trim_start_matches("W/").to_string();
    if_none_match.trim() == "*" || if_none_match.split(',').any(|t| bare(t) == bare(tag))
}

// ---------------------------------------------------------------------------
// front-end optimisations
// ---------------------------------------------------------------------------

/// Add `loading="lazy"` and `decoding="async"` to images in a post body that
/// do not say how to load. The first image is left alone: it is usually the
/// largest thing above the fold, and lazy-loading it delays the moment the
/// page looks loaded.
pub fn lazy_images(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 256);
    let mut at = 0;
    let mut seen = 0;
    while let Some(rel) = lower[at..].find("<img") {
        let start = at + rel;
        let after = lower.as_bytes().get(start + 4).copied();
        if !matches!(after, Some(b' ' | b'\t' | b'\n' | b'/' | b'>')) {
            out.push_str(&html[at..start + 4]);
            at = start + 4;
            continue;
        }
        let Some(end_rel) = lower[start..].find('>') else { break };
        let end = start + end_rel;
        let tag_lower = &lower[start..end];
        out.push_str(&html[at..start]);
        seen += 1;
        let mut tag = html[start..end].to_string();
        if seen > 1 && !tag_lower.contains("loading=") {
            let self_closing = tag.ends_with('/');
            if self_closing {
                tag.pop();
                tag = tag.trim_end().to_string();
            }
            tag.push_str(" loading=\"lazy\"");
            if !tag_lower.contains("decoding=") {
                tag.push_str(" decoding=\"async\"");
            }
            if self_closing {
                tag.push_str(" /");
            }
        }
        out.push_str(&tag);
        at = end;
    }
    out.push_str(&html[at..]);
    out
}

/// Speculation rules: the browser prerenders a same-site page the reader is
/// about to open, so it appears instantly. Never the admin, members pages,
/// anything with a query, or a link marked `rel=nofollow`.
pub fn speculation_rules(mode: Speculation) -> Option<String> {
    let eagerness = match mode {
        Speculation::Off => return None,
        Speculation::Moderate => "moderate",
        Speculation::Eager => "eager",
    };
    Some(format!(
        r#"<script type="speculationrules">{{"prerender":[{{"source":"document","where":{{"and":[{{"href_matches":"/*"}},{{"not":{{"href_matches":["/ghost/*","/members/*","/p/*","/*\\?*"]}}}},{{"not":{{"selector_matches":"[rel~=nofollow]"}}}}]}},"eagerness":"{eagerness}"}}]}}</script>"#
    ))
}

pub fn preconnect(origins: &[String]) -> String {
    origins
        .iter()
        .map(|o| format!(r#"<link rel="preconnect" href="{o}" crossorigin>"#))
        .collect()
}

// ---------------------------------------------------------------------------
// what the Overview says
// ---------------------------------------------------------------------------

/// What can be read off a rendered page without loading it in a browser.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageFacts {
    pub bytes: usize,
    /// `<script src>` in `<head>` without `defer`, `async` or `type=module`.
    pub blocking_scripts: usize,
    pub stylesheets: usize,
    pub images: usize,
    pub images_without_size: usize,
    pub images_not_lazy: usize,
    /// Hosts other than this site that the page loads from.
    pub third_party: Vec<String>,
}

pub fn page_facts(html: &str, own_host: &str) -> PageFacts {
    let lower = html.to_ascii_lowercase();
    let head_end = lower.find("</head>").unwrap_or(0);
    let mut f = PageFacts { bytes: html.len(), ..PageFacts::default() };
    let tags = |name: &str| -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut at = 0;
        let open = format!("<{name}");
        while let Some(rel) = lower[at..].find(&open) {
            let s = at + rel;
            let e = lower[s..].find('>').map(|i| s + i).unwrap_or(lower.len());
            out.push((s, lower[s..e].to_string()));
            at = e;
        }
        out
    };
    let host_of = |tag: &str, attr: &str| -> Option<String> {
        let key = format!("{attr}=\"");
        let i = tag.find(&key)? + key.len();
        let v = &tag[i..tag[i..].find('"').map(|j| i + j).unwrap_or(tag.len())];
        let rest = v.strip_prefix("https://").or_else(|| v.strip_prefix("http://")).or_else(|| v.strip_prefix("//"))?;
        Some(rest.split(['/', '?', '#']).next().unwrap_or("").to_string())
    };
    let mut third: Vec<String> = Vec::new();
    for (pos, t) in tags("script") {
        if t.contains("src=") {
            if pos < head_end && !t.contains("defer") && !t.contains("async") && !t.contains("type=\"module\"") {
                f.blocking_scripts += 1;
            }
            if let Some(h) = host_of(&t, "src") {
                if !h.is_empty() && h != own_host && !third.contains(&h) {
                    third.push(h);
                }
            }
        }
    }
    for (_, t) in tags("link") {
        if t.contains("stylesheet") {
            f.stylesheets += 1;
            if let Some(h) = host_of(&t, "href") {
                if !h.is_empty() && h != own_host && !third.contains(&h) {
                    third.push(h);
                }
            }
        }
    }
    for (_, t) in tags("img") {
        f.images += 1;
        if !(t.contains("width=") && t.contains("height=")) {
            f.images_without_size += 1;
        }
        if !t.contains("loading=") {
            f.images_not_lazy += 1;
        }
    }
    third.sort();
    f.third_party = third;
    f
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Important,
    Suggestion,
    Good,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub settings: Settings,
    /// The home page as last rendered, if there is one to read.
    pub home: Option<PageFacts>,
    /// Median and 95th-percentile render time in milliseconds, if measured.
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub hit_rate: Option<f64>,
    pub cdn_connected: bool,
    pub images_optimized: bool,
}

/// The Overview's list: what to change, most valuable first, then what is
/// already right.
pub fn audit(i: &Inputs) -> Vec<Finding> {
    let s = &i.settings;
    let mut out = Vec::new();
    let mut add = |level, title: &str, detail: String| out.push(Finding { level, title: title.into(), detail });
    match (s.page_cache, i.p95_ms) {
        (false, Some(ms)) if ms > 150 => add(Level::Important, "Turn on the page cache", format!("The slowest pages take {ms} ms to render. Served from the cache, an anonymous reader's page takes about a millisecond.")),
        (false, _) => add(Level::Suggestion, "Turn on the page cache", "Pages for readers who are not signed in can be served from memory instead of rendered every time.".into()),
        (true, _) => add(Level::Good, "Page cache is on", match i.hit_rate {
            Some(r) => format!("{r:.0}% of cacheable requests were served from memory."),
            None => "No requests measured yet.".into(),
        }),
    }
    if !s.gzip {
        add(Level::Important, "Compress responses", "HTML, CSS and JavaScript shrink to a quarter of their size with gzip. Most hosts do this in front of the site; if yours does not, switch it on here.".into());
    } else {
        add(Level::Good, "Responses are compressed", "gzip is on for text responses.".into());
    }
    if let Some(h) = &i.home {
        if h.blocking_scripts > 0 {
            add(Level::Important, "Scripts block the first paint", format!("{} script(s) in the home page's <head> load before anything is drawn. Add defer or async in the theme or Code injection.", h.blocking_scripts));
        }
        if h.images_without_size > 0 {
            add(Level::Suggestion, "Images without a size", format!("{} of {} images on the home page have no width and height, so the page jumps as they load.", h.images_without_size, h.images));
        }
        if h.bytes > 400_000 {
            add(Level::Suggestion, "A heavy home page", format!("The home page's HTML is {} KB. Fewer posts per page or shorter excerpts would help.", h.bytes / 1024));
        }
        if h.third_party.len() > 3 {
            add(Level::Suggestion, "Many outside hosts", format!("The home page loads from {} other hosts ({}). Each costs a connection; preconnect to the ones you keep.", h.third_party.len(), h.third_party.join(", ")));
        }
    }
    if !s.lazy_images {
        add(Level::Suggestion, "Lazy-load images in posts", "Images further down a post load only as the reader scrolls to them.".into());
    }
    if s.speculation == Speculation::Off {
        add(Level::Suggestion, "Open the next page before the click", "Speculation rules let Chrome and Edge prerender a link the reader is about to open, so it appears instantly.".into());
    }
    if !i.images_optimized {
        add(Level::Suggestion, "Optimise uploaded images", "Settings → Images can resize and recompress images as they are uploaded.".into());
    }
    if s.cdn_ttl > 0 && !i.cdn_connected {
        add(Level::Important, "A CDN caches pages it is never told to forget", format!("Pages are marked cacheable by a CDN for {} seconds, but no CDN is connected for purging, so an edit can take that long to show.", s.cdn_ttl));
    }
    out.sort_by_key(|f| f.level);
    out
}

/// Median and 95th percentile of a set of durations.
pub fn percentiles(mut ms: Vec<u64>) -> Option<(u64, u64)> {
    if ms.is_empty() {
        return None;
    }
    ms.sort_unstable();
    let at = |p: f64| ms[((ms.len() as f64 - 1.0) * p).round() as usize];
    Some((at(0.5), at(0.95)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Settings {
        Settings::read(|k| (k == SETTING_PAGE_CACHE).then(|| "true".into()))
    }

    #[test]
    fn everything_ships_off() {
        let s = Settings::default();
        assert!(!s.page_cache && !s.etag && !s.gzip && !s.lazy_images && !s.cdn_auto_purge);
        assert_eq!((s.ttl, s.browser_ttl, s.cdn_ttl, s.max_pages), (300, 0, 0, 500));
        assert_eq!(s.speculation, Speculation::Off);
        assert_eq!(request_key(&s, false, "GET", "/", None, ""), Err(Bypass::Off));
    }

    #[test]
    fn only_anonymous_page_requests_are_cached() {
        let s = on();
        assert_eq!(request_key(&s, false, "GET", "/hello/", None, ""), Ok("/hello/".into()));
        assert_eq!(request_key(&s, false, "POST", "/hello/", None, ""), Err(Bypass::Method));
        assert_eq!(request_key(&s, true, "GET", "/hello/", None, ""), Err(Bypass::Private));
        assert_eq!(request_key(&s, false, "GET", "/hello/", None, "theme=dark; vuuut-members-ssr=abc"), Err(Bypass::SignedIn));
        assert_eq!(request_key(&s, false, "GET", "/hello/", None, "vuuut-members-ssr.sig=x"), Err(Bypass::SignedIn));
        assert_eq!(request_key(&s, false, "GET", "/hello/", None, "vuuut_consent=x"), Ok("/hello/".into()), "the consent cookie is not personal");
        for path in ["/ghost/", "/members/api/member/", "/p/uuid/", "/search/api/", "/ads/api/events/"] {
            assert_eq!(request_key(&s, false, "GET", path, None, ""), Err(Bypass::System), "{path}");
        }
    }

    #[test]
    fn tracking_parameters_do_not_split_the_cache_and_other_queries_are_sorted() {
        let s = on();
        assert_eq!(request_key(&s, false, "GET", "/a/", Some("utm_source=x&fbclid=1"), ""), Ok("/a/".into()));
        assert_eq!(request_key(&s, false, "GET", "/a/", Some("b=2&a=1&utm_medium=y"), ""), Ok("/a/?a=1&b=2".into()));
        let long = format!("q={}", "x".repeat(300));
        assert_eq!(request_key(&s, false, "GET", "/a/", Some(&long), ""), Err(Bypass::Query));
        let ex = Settings::read(|k| match k {
            SETTING_PAGE_CACHE => Some("true".into()),
            SETTING_EXCLUDE => Some("/live/\nnot-a-path\n//x".into()),
            _ => None,
        });
        assert_eq!(ex.exclude, ["/live/"]);
        assert_eq!(request_key(&ex, false, "GET", "/live/now/", None, ""), Err(Bypass::Excluded));
    }

    #[test]
    fn a_page_with_a_form_token_or_a_cookie_is_never_stored() {
        assert_eq!(response_storable(200, "text/html; charset=utf-8", "", false, b"<p>ok</p>"), Ok(()));
        assert_eq!(response_storable(404, "text/html", "", false, b""), Err(Bypass::Status));
        assert_eq!(response_storable(200, "application/json", "", false, b"{}"), Err(Bypass::NotHtml));
        assert_eq!(response_storable(200, "text/html", "private, no-store", false, b""), Err(Bypass::NoStore));
        assert_eq!(response_storable(200, "text/html", "", true, b""), Err(Bypass::SetsCookie));
        assert_eq!(response_storable(200, "text/html", "", false, br#"<input name="_token" value="1.a.b">"#), Err(Bypass::Form));
    }

    #[test]
    fn headers_follow_the_settings() {
        assert_eq!(cache_control(&Settings::default()), "public, max-age=0, must-revalidate");
        let s = Settings::read(|k| match k {
            SETTING_BROWSER_TTL => Some("60".into()),
            SETTING_CDN_TTL => Some("3600".into()),
            _ => None,
        });
        assert_eq!(cache_control(&s), "public, max-age=60, s-maxage=3600, stale-while-revalidate=3600");
        let t = etag(b"hello");
        assert!(t.starts_with("W/\""));
        assert!(etag_matches(&t, &t) && etag_matches(&t.replace("W/", ""), &t) && etag_matches("\"x\", *", &t) == false);
        assert!(etag_matches("*", &t));
        assert_ne!(etag(b"hello"), etag(b"hello!"));
    }

    #[test]
    fn lazy_loading_spares_the_first_image_and_existing_choices() {
        let html = r#"<img src="a.jpg"><p>x</p><img src="b.jpg"><img loading="eager" src="c.jpg"><img src="d.jpg" /><imgx>"#;
        assert_eq!(
            lazy_images(html),
            r#"<img src="a.jpg"><p>x</p><img src="b.jpg" loading="lazy" decoding="async"><img loading="eager" src="c.jpg"><img src="d.jpg" loading="lazy" decoding="async" /><imgx>"#
        );
    }

    #[test]
    fn only_https_origins_are_preconnected() {
        let o = parse_origins("https://fonts.gstatic.com/\nhttp://x.com\njavascript:alert(1)\nhttps://a\"b.com");
        assert_eq!(o, ["https://fonts.gstatic.com"]);
        assert!(preconnect(&o).contains(r#"href="https://fonts.gstatic.com""#));
        assert!(speculation_rules(Speculation::Off).is_none());
        assert!(speculation_rules(Speculation::Moderate).unwrap().contains(r#""eagerness":"moderate""#));
    }

    #[test]
    fn page_facts_find_blocking_scripts_and_unsized_images() {
        let html = r#"<head><script src="https://cdn.x.com/a.js"></script><script defer src="/b.js"></script>
<link rel="stylesheet" href="https://fonts.example.com/c.css"></head><body><img src="a" width="1" height="1"><img src="b" loading="lazy"></body>"#;
        let f = page_facts(html, "site.example");
        assert_eq!((f.blocking_scripts, f.stylesheets, f.images, f.images_without_size, f.images_not_lazy), (1, 1, 2, 1, 1));
        assert_eq!(f.third_party, ["cdn.x.com", "fonts.example.com"]);
    }

    #[test]
    fn the_audit_ranks_what_matters_first() {
        let i = Inputs {
            p95_ms: Some(400),
            home: Some(PageFacts { blocking_scripts: 2, ..PageFacts::default() }),
            ..Inputs::default()
        };
        let a = audit(&i);
        assert_eq!(a[0].level, Level::Important);
        assert!(a.iter().any(|f| f.title == "Turn on the page cache" && f.detail.contains("400 ms")));
        assert!(a.windows(2).all(|w| w[0].level <= w[1].level));
        assert_eq!(percentiles(vec![5, 1, 9, 3, 7]), Some((5, 9)));
        assert_eq!(percentiles(vec![]), None);
    }
}
