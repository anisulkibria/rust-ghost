//! Which languages a site publishes in, and how a URL says which one it is.
//!
//! The trust boundary and the single authority for the Multilingual module:
//! the front router, the renderer, the sitemap and the admin all decide what
//! a locale is by calling in here, so none of them can disagree about whether
//! `/de/my-post/` is German or a post whose slug is `my-post` inside a
//! collection called `de`.
//!
//! ## The URL shape
//!
//! The default language is **unprefixed** and every other language carries
//! its code as the first path segment:
//!
//! ```text
//! /my-post/          the default language
//! /de/my-post/       German
//! /de/tag/neues/     a German tag archive
//! ```
//!
//! This is the one shape that costs nothing to adopt: every URL a site
//! already has keeps working untouched, so there is nothing to redirect and
//! no ranking to re-earn. The symmetrical alternative (`/en/my-post/`) is
//! tidier to look at and would 301 every existing link, which is a migration
//! rather than a feature.
//!
//! ## The collision this creates, and why it is still the right trade
//!
//! Once `/de/` means German, a post whose slug is exactly `de` is
//! unreachable — the prefix is read first. That is inherent to prefix
//! routing, not a bug in this module, and it is why [`collides_with_locale`]
//! exists: the admin warns about such a slug at the point somebody creates
//! one, rather than letting the page quietly disappear. Only **enabled**
//! locales are stripped, so the window is as narrow as it can be — a site
//! publishing only in English has no reserved prefixes at all.
//!
//! ## Why an unknown prefix is not a locale
//!
//! [`split_path`] strips a prefix only when the code is one this site has
//! actually enabled. A request for `/xx/my-post/` is therefore a request for
//! a post inside a collection named `xx`, and 404s like any other missing
//! page — it does not become "the default language, somehow". Treating any
//! two-letter segment as a language would make every short first segment
//! ambiguous forever.

use serde_json::Value;

/// The longest a language subtag can be, and the shortest. ISO 639 is two or
/// three letters; the optional region or script suffix is two to four.
const MIN_LANG: usize = 2;
const MAX_LANG: usize = 3;

/// Canonicalise a language code, or reject it.
///
/// Accepts `en`, `de-CH`, `pt-BR`, `zh-Hans`; returns the conventional
/// casing — lowercase language, uppercase two-letter region, title-case
/// longer script subtag — so that two spellings of the same language cannot
/// become two languages. Anything else is `None`: the code reaches a URL
/// prefix, an `hreflang` attribute and a filename under
/// `<theme>/locales/`, and none of those may carry a path separator or a
/// quote.
pub fn normalise(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.split(['-', '_']);
    let lang = parts.next()?;
    if !(MIN_LANG..=MAX_LANG).contains(&lang.chars().count())
        || !lang.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    let mut out = lang.to_ascii_lowercase();
    if let Some(suffix) = parts.next() {
        let n = suffix.chars().count();
        if !(2..=4).contains(&n) || !suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        out.push('-');
        if n == 2 {
            out.push_str(&suffix.to_ascii_uppercase());
        } else {
            // A script subtag is title case: `Hans`, `Latn`.
            let mut chars = suffix.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(&chars.as_str().to_ascii_lowercase());
            }
        }
    }
    // Only one suffix is accepted; `en-GB-oed` is not a code this handles.
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// Every language a site can publish in, as `(code, label)`.
///
/// Codes and coverage come from Ghost's own list, which matters beyond the
/// label: a theme's translations live in `<theme>/locales/<code>.json`, so a
/// code Ghost does not use silently finds no translation file. Labels lead
/// with the language's own name — that is what somebody scans for when
/// picking their own, and what [`native_name`] hands a language switcher.
pub const LANGUAGES: &[(&str, &str)] = &[
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

/// A language's name in that language — `de` → `Deutsch`.
///
/// What a language switcher shows: a German reader looking for German scans
/// for "Deutsch", not "German". Falls back to the upper-cased code for a
/// language the table does not cover, rather than to a blank link.
pub fn native_name(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(value, _)| value.eq_ignore_ascii_case(code))
        .map(|(_, label)| strip_english_name(label).to_string())
        .unwrap_or_else(|| code.to_ascii_uppercase())
}

/// `Português (Brasil) (Portuguese (Brazil))` → `Português (Brasil)`.
///
/// The English name is the label's *last top-level* parenthetical, and it
/// can contain parentheses of its own, so it is found by matching brackets
/// back from the end rather than by searching for the last `" ("`.
fn strip_english_name(label: &str) -> &str {
    if !label.ends_with(')') {
        return label;
    }
    let mut depth = 0usize;
    for (at, c) in label.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let native = label[..at].trim_end();
                    return if native.is_empty() { label } else { native };
                }
            }
            _ => {}
        }
    }
    label
}

/// The URL that marks a menu item as the language switcher.
///
/// A fragment, so it passes every URL check a menu item already goes
/// through and, on a theme or API client that knows nothing about it, is a
/// harmless same-page link rather than a 404. The front end replaces the
/// item with one link per language when it builds `@site.navigation`.
pub const SWITCHER_URL: &str = "#language-switcher";

/// Whether a menu item's URL is the language switcher marker.
pub fn is_switcher(url: &str) -> bool {
    url.trim().eq_ignore_ascii_case(SWITCHER_URL)
}

/// Whether to publish `rel=alternate hreflang` alternates.
///
/// Defaults to **true**, which is why this is not read through
/// `seo::flag`: that function's defaults table only knows the `seo_*` keys
/// and would answer `false` for this one — silently withholding the tags
/// that are the entire point of publishing translations as separate URLs.
///
/// Accepts the string forms older rows and form posts produce, like the SEO
/// flags do.
pub fn hreflang_enabled(container: &Value) -> bool {
    match container.get("locale_hreflang") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.trim(), "false" | "0"),
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or(true),
        _ => true,
    }
}

/// The languages a site publishes in.
///
/// `default` is always present in `enabled` and always first, which several
/// callers rely on when they need "the primary language" without having to
/// special-case an empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locales {
    default: String,
    enabled: Vec<String>,
}

impl Default for Locales {
    fn default() -> Self {
        Locales { default: "en".into(), enabled: vec!["en".into()] }
    }
}

impl Locales {
    /// Build from the two settings that describe this: `locale` (the
    /// publication language, which is the default) and `locales_enabled`.
    ///
    /// Deliberately forgiving, because this runs on every request: an
    /// unreadable `locales_enabled` leaves a single-language site rather than
    /// taking the site down, and the default is always added back even if it
    /// has been left out of the list.
    pub fn from_settings(locale: Option<&str>, enabled: Option<&Value>) -> Self {
        let default = locale.and_then(normalise).unwrap_or_else(|| "en".to_string());
        let mut out = vec![default.clone()];
        let listed = match enabled {
            Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str()).map(String::from).collect(),
            // Older rows and form posts hold the array as a JSON string.
            Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
            _ => Vec::new(),
        };
        for code in listed {
            if let Some(code) = normalise(&code) {
                if !out.contains(&code) {
                    out.push(code);
                }
            }
        }
        Locales { default, enabled: out }
    }

    /// For tests and for the admin, which already has validated codes.
    pub fn new(default: &str, others: &[&str]) -> Self {
        Self::from_settings(
            Some(default),
            Some(&Value::Array(others.iter().map(|c| Value::String((*c).into())).collect())),
        )
    }

    pub fn default_locale(&self) -> &str {
        &self.default
    }

    /// Every language, the default first.
    pub fn enabled(&self) -> &[String] {
        &self.enabled
    }

    /// Whether this site publishes in more than one language. Everything the
    /// module adds to a page — the switcher, the `hreflang` block, the
    /// prefixes — is suppressed when this is false, so installing the module
    /// changes nothing until a second language is added.
    pub fn multilingual(&self) -> bool {
        self.enabled.len() > 1
    }

    pub fn is_enabled(&self, code: &str) -> bool {
        normalise(code).is_some_and(|c| self.enabled.contains(&c))
    }

    pub fn is_default(&self, code: &str) -> bool {
        normalise(code).is_some_and(|c| c == self.default)
    }

    /// The canonical form of a code this site has enabled, or the default.
    ///
    /// Used wherever a locale arrives from outside — a form, a query, a
    /// stored row — so an unknown value becomes the default rather than
    /// reaching a query or a URL.
    pub fn resolve(&self, code: Option<&str>) -> String {
        code.and_then(normalise)
            .filter(|c| self.enabled.contains(c))
            .unwrap_or_else(|| self.default.clone())
    }

    /// The path prefix for a language: `""` for the default, `/de` otherwise.
    ///
    /// Lowercased, because a URL path is compared byte for byte and
    /// `/de-CH/` and `/de-ch/` would otherwise be two different pages. The
    /// canonical mixed case is kept for `hreflang`, where it is what the
    /// attribute is specified to carry.
    pub fn prefix(&self, locale: &str) -> String {
        match normalise(locale) {
            Some(code) if code != self.default && self.enabled.contains(&code) => {
                format!("/{}", code.to_ascii_lowercase())
            }
            _ => String::new(),
        }
    }

    /// Split a request path into the language it names and what is left.
    ///
    /// `/de/my-post/` → `("de", "/my-post/")`, and `/de` → `("de", "/")` so
    /// the bare prefix is the language's home page rather than a 404.
    /// Anything that is not an enabled language is left whole and reported as
    /// the default.
    pub fn split_path(&self, path: &str) -> (String, String) {
        let trimmed = path.strip_prefix('/').unwrap_or(path);
        let (first, rest) = match trimmed.find('/') {
            Some(at) => (&trimmed[..at], &trimmed[at..]),
            None => (trimmed, ""),
        };
        if first.is_empty() {
            return (self.default.clone(), path.to_string());
        }
        // Compared lowercased so `/DE/` reaches the same page as `/de/`; the
        // front end redirects the upper-case spelling to the lower, but the
        // match must not depend on that having happened first.
        let candidate = first.to_ascii_lowercase();
        let found = self
            .enabled
            .iter()
            .find(|code| **code != self.default && code.to_ascii_lowercase() == candidate);
        match found {
            Some(code) => {
                let rest = if rest.is_empty() { "/" } else { rest };
                (code.clone(), rest.to_string())
            }
            None => (self.default.clone(), path.to_string()),
        }
    }

    /// Put `path` — a path in the site's default language — into `locale`.
    ///
    /// `path` is expected to carry no prefix already; this is for building a
    /// link out of a stored, language-neutral path.
    pub fn localise(&self, path: &str, locale: &str) -> String {
        let prefix = self.prefix(locale);
        if prefix.is_empty() {
            return path.to_string();
        }
        match path {
            "/" => format!("{prefix}/"),
            p if p.starts_with('/') => format!("{prefix}{p}"),
            p => format!("{prefix}/{p}"),
        }
    }

    /// Whether a slug would be shadowed by a language prefix.
    ///
    /// A post slugged `de` on a site that publishes in German is unreachable:
    /// `/de/` is read as the German home page before anything looks for a
    /// post. The admin uses this to say so while the slug is being chosen,
    /// which is the only moment it is cheap to fix.
    pub fn collides_with_locale(&self, slug: &str) -> bool {
        let slug = slug.trim().trim_matches('/').to_ascii_lowercase();
        !slug.is_empty()
            && self
                .enabled
                .iter()
                .any(|code| **code != self.default && code.to_ascii_lowercase() == slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn site() -> Locales {
        Locales::new("en", &["de", "fr", "pt-BR"])
    }

    #[test]
    fn hreflang_is_on_unless_it_has_been_switched_off() {
        // A key with no row must not read as `false`: that would withhold
        // the alternates on every site that upgraded into this module.
        assert!(hreflang_enabled(&json!({})));
        assert!(hreflang_enabled(&json!({"locale_hreflang": null})));
        assert!(hreflang_enabled(&json!({"locale_hreflang": true})));
        assert!(hreflang_enabled(&json!({"locale_hreflang": "true"})));
        assert!(!hreflang_enabled(&json!({"locale_hreflang": false})));
        assert!(!hreflang_enabled(&json!({"locale_hreflang": "false"})));
        // Nonsense gets the default rather than off.
        assert!(hreflang_enabled(&json!({"locale_hreflang": "maybe"})));
    }

    #[test]
    fn a_switcher_names_each_language_in_that_language() {
        assert_eq!(native_name("de"), "Deutsch");
        assert_eq!(native_name("en"), "English");
        assert_eq!(native_name("FR"), "Français");
        // A label whose own name carries parentheses keeps them.
        assert_eq!(native_name("zh"), "中文(简体)");
        // ...and an English name with parentheses of its own is still
        // taken off whole.
        assert_eq!(native_name("pt-BR"), "Português (Brasil)");
        assert_eq!(native_name("gd"), "Gàidhlig");
        // Unknown codes still produce a visible, non-empty link text.
        assert_eq!(native_name("xx"), "XX");
        // Every listed code is one `normalise` accepts, so none can be
        // enabled under a spelling the switcher would not recognise.
        for (code, _) in LANGUAGES {
            assert_eq!(normalise(code).as_deref(), Some(*code), "{code}");
        }
    }

    #[test]
    fn the_switcher_marker_is_recognised_however_it_was_typed() {
        assert!(is_switcher(SWITCHER_URL));
        assert!(is_switcher(" #Language-Switcher "));
        assert!(!is_switcher("#language"));
        assert!(!is_switcher("/language-switcher/"));
    }

    #[test]
    fn a_code_is_canonicalised_so_one_language_cannot_become_two() {
        assert_eq!(normalise("EN").as_deref(), Some("en"));
        assert_eq!(normalise(" de ").as_deref(), Some("de"));
        assert_eq!(normalise("de_ch").as_deref(), Some("de-CH"));
        assert_eq!(normalise("DE-ch").as_deref(), Some("de-CH"));
        assert_eq!(normalise("pt-br").as_deref(), Some("pt-BR"));
        assert_eq!(normalise("zh-hans").as_deref(), Some("zh-Hans"));
        assert_eq!(normalise("fil").as_deref(), Some("fil"));
    }

    #[test]
    fn a_code_that_could_escape_a_path_or_an_attribute_is_refused() {
        // The code becomes a URL prefix, an hreflang attribute and a filename
        // under <theme>/locales/. None of those may carry a separator, a
        // quote or a dot.
        for bad in [
            "", "   ", "e", "english", "../../etc/passwd", "en/", "en.json", "e n", "en\"", "en-",
            "-en", "en-GB-oed", "4n", "en--GB", "../en", "e/n",
        ] {
            assert_eq!(normalise(bad), None, "{bad:?} must be refused");
        }
    }

    #[test]
    fn the_default_is_always_enabled_and_always_first() {
        // Several callers take `enabled()[0]` as the primary language.
        let listed_without_default = Locales::from_settings(Some("de"), Some(&json!(["fr", "en"])));
        assert_eq!(listed_without_default.default_locale(), "de");
        assert_eq!(listed_without_default.enabled(), ["de", "fr", "en"]);
        // And it is not duplicated when it is in the list too.
        let listed_with = Locales::from_settings(Some("en"), Some(&json!(["en", "de", "en"])));
        assert_eq!(listed_with.enabled(), ["en", "de"]);
    }

    #[test]
    fn unreadable_settings_leave_a_working_single_language_site() {
        // This runs on every request. Bad data must not take the site down.
        for enabled in [None, Some(json!(null)), Some(json!("not json")), Some(json!(42)), Some(json!({}))] {
            let l = Locales::from_settings(Some("en"), enabled.as_ref());
            assert_eq!(l.enabled(), ["en"]);
            assert!(!l.multilingual());
        }
        // A missing publication language falls back to English, not to empty.
        assert_eq!(Locales::from_settings(None, None).default_locale(), "en");
        assert_eq!(Locales::from_settings(Some("nonsense!"), None).default_locale(), "en");
        // Codes in the list that are not codes are dropped, not kept raw.
        let mixed = Locales::from_settings(Some("en"), Some(&json!(["de", "../x", "fr", ""])));
        assert_eq!(mixed.enabled(), ["en", "de", "fr"]);
    }

    #[test]
    fn the_default_language_has_no_prefix_and_the_others_do() {
        let s = site();
        assert_eq!(s.prefix("en"), "");
        assert_eq!(s.prefix("de"), "/de");
        // Lowercased, because a path is compared byte for byte.
        assert_eq!(s.prefix("pt-BR"), "/pt-br");
        // A language this site does not publish in gets no prefix rather
        // than an invented one.
        assert_eq!(s.prefix("es"), "");
        assert_eq!(s.prefix("nonsense"), "");
    }

    #[test]
    fn a_path_is_split_into_the_language_it_names_and_the_rest() {
        let s = site();
        assert_eq!(s.split_path("/de/my-post/"), ("de".into(), "/my-post/".into()));
        assert_eq!(s.split_path("/de/tag/neues/"), ("de".into(), "/tag/neues/".into()));
        assert_eq!(s.split_path("/pt-br/meu-post/"), ("pt-BR".into(), "/meu-post/".into()));
        // The bare prefix is that language's home page, not a 404.
        assert_eq!(s.split_path("/de/"), ("de".into(), "/".into()));
        assert_eq!(s.split_path("/de"), ("de".into(), "/".into()));
        // Case does not decide which page you get.
        assert_eq!(s.split_path("/DE/my-post/"), ("de".into(), "/my-post/".into()));
    }

    #[test]
    fn a_path_in_the_default_language_is_left_exactly_as_it_was() {
        // The whole promise of this URL shape: nothing that works today
        // changes. A regression here silently rewrites every existing URL.
        let s = site();
        for path in ["/", "/my-post/", "/tag/news/", "/author/ada/", "/page/2/", "/about/"] {
            assert_eq!(s.split_path(path), ("en".into(), path.to_string()), "{path}");
        }
    }

    #[test]
    fn an_unknown_first_segment_is_a_slug_and_not_a_language() {
        // `/xx/my-post/` is a post inside a collection called `xx`, and 404s
        // like any other missing page. Treating every short segment as a
        // language would make them ambiguous forever.
        let s = site();
        assert_eq!(s.split_path("/es/mi-post/"), ("en".into(), "/es/mi-post/".into()));
        assert_eq!(s.split_path("/xx/"), ("en".into(), "/xx/".into()));
        // And on a single-language site nothing at all is stripped — so
        // enabling the module changes no URL until a language is added.
        let one = Locales::default();
        for path in ["/de/my-post/", "/fr/", "/pt-br/x/"] {
            assert_eq!(one.split_path(path), ("en".into(), path.to_string()), "{path}");
        }
    }

    #[test]
    fn localising_a_path_round_trips_through_splitting_it() {
        let s = site();
        for locale in s.enabled() {
            for path in ["/", "/my-post/", "/tag/news/"] {
                let localised = s.localise(path, locale);
                let (back, rest) = s.split_path(&localised);
                assert_eq!(&back, locale, "{locale} {path} -> {localised}");
                assert_eq!(rest, path, "{locale} {path} -> {localised}");
            }
        }
        // The default language's URLs are untouched by localising them.
        assert_eq!(s.localise("/my-post/", "en"), "/my-post/");
        assert_eq!(s.localise("/", "en"), "/");
        assert_eq!(s.localise("/", "de"), "/de/");
    }

    #[test]
    fn a_slug_shadowed_by_a_language_prefix_is_reported() {
        // A post slugged `de` on a German-publishing site is unreachable:
        // the prefix is read first. The admin says so while the slug is
        // being chosen, which is the only cheap moment to fix it.
        let s = site();
        assert!(s.collides_with_locale("de"));
        assert!(s.collides_with_locale("DE"));
        assert!(s.collides_with_locale("/de/"));
        assert!(s.collides_with_locale("pt-br"));
        // The default language is not a prefix, so its code is a usable slug.
        assert!(!s.collides_with_locale("en"));
        assert!(!s.collides_with_locale("deutsch"));
        assert!(!s.collides_with_locale(""));
        // And nothing collides on a single-language site.
        assert!(!Locales::default().collides_with_locale("de"));
    }

    #[test]
    fn a_locale_arriving_from_outside_is_reduced_to_one_we_publish_in() {
        let s = site();
        assert_eq!(s.resolve(Some("de")), "de");
        assert_eq!(s.resolve(Some("DE")), "de");
        // Not enabled, nonsense, or absent — all become the default rather
        // than reaching a query or a URL.
        assert_eq!(s.resolve(Some("es")), "en");
        assert_eq!(s.resolve(Some("../../x")), "en");
        assert_eq!(s.resolve(None), "en");
    }
}
