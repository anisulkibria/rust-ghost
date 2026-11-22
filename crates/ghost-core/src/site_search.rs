//! Search and Filtering: the rules for the public site's search.
//!
//! The site has two search engines, and this module is the one place that
//! decides how either of them behaves:
//!
//! * **Sodo Search** — Ghost's own overlay, loaded from a CDN, which builds a
//!   FlexSearch index *in the visitor's browser* out of three feeds this
//!   server publishes (`/ghost/api/content/search-index/*`). It cannot be
//!   configured, so everything that can be changed about it is *what goes
//!   into those feeds*: [`Settings::admits`] and [`sodo_excerpt`].
//! * **Vuuut search** — this port's own engine. Ranking happens on the
//!   server ([`rank`]), so the body of a post can be searched without
//!   shipping every body to every visitor, a synonym list and field weights
//!   apply, results can be filtered by type and tag, and the queries people
//!   type can be counted.
//!
//! Every default reproduces the search as it was before the tool existed:
//! Sodo, posts only, members-only posts listed by title and excerpt, authors
//! and tags shown. Installing the tool changes nothing until somebody moves
//! a switch.
//!
//! This is also a trust boundary in one respect: **a members-only post's body
//! is never searched or quoted.** [`Doc::body`] is empty for anything that is
//! not public, which [`Doc::new`] enforces rather than trusting the caller.

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Ghost's Sodo Search overlay — the default, and what every Ghost theme
    /// is written for.
    Sodo,
    /// The built-in engine.
    Vuuut,
    /// No search at all: neither script is added to the page.
    Off,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Vuuut, Engine::Sodo, Engine::Off];
    pub fn key(self) -> &'static str {
        match self {
            Engine::Sodo => "sodo",
            Engine::Vuuut => "vuuut",
            Engine::Off => "off",
        }
    }
    pub fn from_key(key: &str) -> Engine {
        match key.trim().trim_matches('"') {
            "vuuut" => Engine::Vuuut,
            "off" => Engine::Off,
            _ => Engine::Sodo,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Engine::Sodo => "Sodo search",
            Engine::Vuuut => "Vuuut search",
            Engine::Off => "No search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Best match first; the newer of two equal matches first.
    Relevance,
    /// Every match, newest first.
    Newest,
}

impl Order {
    pub fn key(self) -> &'static str {
        match self {
            Order::Relevance => "relevance",
            Order::Newest => "newest",
        }
    }
    pub fn from_key(key: &str) -> Order {
        if key.trim().trim_matches('"') == "newest" { Order::Newest } else { Order::Relevance }
    }
}

pub const SETTING_ENGINE: &str = "search_engine";
pub const SETTING_INCLUDE_PAGES: &str = "search_include_pages";
pub const SETTING_INCLUDE_MEMBERS: &str = "search_include_members";
pub const SETTING_EXCLUDE_TAGS: &str = "search_exclude_tags";
pub const SETTING_FULL_TEXT: &str = "search_full_text";
pub const SETTING_SHOW_AUTHORS: &str = "search_show_authors";
pub const SETTING_SHOW_TAGS: &str = "search_show_tags";
pub const SETTING_WEIGHT_TITLE: &str = "search_weight_title";
pub const SETTING_WEIGHT_EXCERPT: &str = "search_weight_excerpt";
pub const SETTING_WEIGHT_TAGS: &str = "search_weight_tags";
pub const SETTING_WEIGHT_BODY: &str = "search_weight_body";
pub const SETTING_ORDER: &str = "search_order";
pub const SETTING_LIMIT: &str = "search_limit";
pub const SETTING_SYNONYMS: &str = "search_synonyms";
pub const SETTING_PLACEHOLDER: &str = "search_placeholder";
pub const SETTING_SHORTCUT: &str = "search_shortcut";
pub const SETTING_LOG: &str = "search_log";

pub const SETTING_KEYS: [&str; 17] = [
    SETTING_ENGINE,
    SETTING_INCLUDE_PAGES,
    SETTING_INCLUDE_MEMBERS,
    SETTING_EXCLUDE_TAGS,
    SETTING_FULL_TEXT,
    SETTING_SHOW_AUTHORS,
    SETTING_SHOW_TAGS,
    SETTING_WEIGHT_TITLE,
    SETTING_WEIGHT_EXCERPT,
    SETTING_WEIGHT_TAGS,
    SETTING_WEIGHT_BODY,
    SETTING_ORDER,
    SETTING_LIMIT,
    SETTING_SYNONYMS,
    SETTING_PLACEHOLDER,
    SETTING_SHORTCUT,
    SETTING_LOG,
];

/// How many results a search may show. Offered, not free-typed.
pub const LIMITS: [i64; 4] = [10, 20, 30, 50];
pub const MAX_WEIGHT: i64 = 10;
pub const MAX_PLACEHOLDER: usize = 80;
pub const MAX_SYNONYM_LINES: usize = 200;
pub const MAX_QUERY: usize = 100;
pub const MAX_TERMS: usize = 8;
pub const DEFAULT_PLACEHOLDER: &str = "Search posts, tags and authors";
/// How many days of typed searches the insights keep.
pub const LOG_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub engine: Engine,
    pub include_pages: bool,
    /// List members-only and paid posts (title and excerpt — never the body).
    pub include_members: bool,
    /// Tag slugs whose posts are left out of search entirely.
    pub exclude_tags: Vec<String>,
    /// Search the body of public posts, not just title and excerpt.
    pub full_text: bool,
    pub show_authors: bool,
    pub show_tags: bool,
    pub weight_title: i64,
    pub weight_excerpt: i64,
    pub weight_tags: i64,
    pub weight_body: i64,
    pub order: Order,
    pub limit: i64,
    /// Groups of words that mean the same thing here.
    pub synonyms: Vec<Vec<String>>,
    pub placeholder: String,
    /// `/` and ⌘K open the search box.
    pub shortcut: bool,
    /// Count what people search for.
    pub log: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

impl Settings {
    /// `get` is the settings cache's string lookup. A missing or malformed
    /// value is the default; a number out of range is clamped.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Settings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let number = |key: &str| get(key).and_then(|v| v.trim().trim_matches('"').parse::<i64>().ok());
        let weight = |key: &str, default: i64| number(key).unwrap_or(default).clamp(0, MAX_WEIGHT);
        let placeholder = clean(&get(SETTING_PLACEHOLDER).unwrap_or_default(), MAX_PLACEHOLDER);
        Settings {
            engine: Engine::from_key(&get(SETTING_ENGINE).unwrap_or_default()),
            include_pages: flag(SETTING_INCLUDE_PAGES, false),
            include_members: flag(SETTING_INCLUDE_MEMBERS, true),
            exclude_tags: parse_slugs(&get(SETTING_EXCLUDE_TAGS).unwrap_or_default()),
            full_text: flag(SETTING_FULL_TEXT, false),
            show_authors: flag(SETTING_SHOW_AUTHORS, true),
            show_tags: flag(SETTING_SHOW_TAGS, true),
            weight_title: weight(SETTING_WEIGHT_TITLE, 8),
            weight_excerpt: weight(SETTING_WEIGHT_EXCERPT, 4),
            weight_tags: weight(SETTING_WEIGHT_TAGS, 3),
            weight_body: weight(SETTING_WEIGHT_BODY, 1),
            order: Order::from_key(&get(SETTING_ORDER).unwrap_or_default()),
            limit: number(SETTING_LIMIT).filter(|n| LIMITS.contains(n)).unwrap_or(20),
            synonyms: parse_synonyms(&get(SETTING_SYNONYMS).unwrap_or_default()),
            placeholder: if placeholder.is_empty() { DEFAULT_PLACEHOLDER.to_string() } else { placeholder },
            shortcut: flag(SETTING_SHORTCUT, true),
            log: flag(SETTING_LOG, true),
        }
    }

    /// Whether a post may appear in search at all — the one test both
    /// engines share, so Sodo's feed and the built-in engine leave out the
    /// same things.
    pub fn admits(&self, kind: Kind, visibility: &str, tag_slugs: &[String]) -> bool {
        if kind == Kind::Page && !self.include_pages {
            return false;
        }
        if visibility != "public" && !self.include_members {
            return false;
        }
        !tag_slugs.iter().any(|t| self.exclude_tags.iter().any(|x| x == t))
    }
}

/// Comma- or line-separated tag slugs, reduced to the characters a slug can
/// hold and de-duplicated.
pub fn parse_slugs(raw: &str) -> Vec<String> {
    let raw = raw.trim().trim_matches('"');
    let mut out: Vec<String> = Vec::new();
    for part in raw.split([',', '\n']) {
        let slug: String = part
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(191)
            .collect();
        if !slug.is_empty() && !out.contains(&slug) {
            out.push(slug);
        }
    }
    out
}

/// One group per line, words separated by commas: `js, javascript`. A group
/// is a set of words any of which finds the others. Normalised the way
/// queries are, so "Café" in the list finds "cafe" typed in the box.
pub fn parse_synonyms(raw: &str) -> Vec<Vec<String>> {
    raw.replace("\\n", "\n")
        .lines()
        .take(MAX_SYNONYM_LINES)
        .filter_map(|line| {
            let mut group: Vec<String> = Vec::new();
            for word in line.split(',') {
                let w = normalize(word).split_whitespace().collect::<Vec<_>>().join(" ");
                if !w.is_empty() && w.chars().count() <= 40 && !group.contains(&w) {
                    group.push(w);
                }
            }
            (group.len() >= 2).then_some(group)
        })
        .collect()
}

/// The stored form of a synonym list — what the admin shows back.
pub fn synonyms_text(groups: &[Vec<String>]) -> String {
    groups.iter().map(|g| g.join(", ")).collect::<Vec<_>>().join("\n")
}

pub fn clean(raw: &str, max: usize) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// Lowercase, with the common Latin accents folded, so "Café" and "cafe"
/// are the same word. Character-for-character: the output has exactly as
/// many `char`s as the input, which is what lets [`snippet`] map a match in
/// the folded text back onto the original.
pub fn normalize(s: &str) -> String {
    s.chars().map(fold).collect()
}

fn fold(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => 'i',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ś' | 'š' | 'ş' => 's',
        'ť' | 'ţ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'ł' => 'l',
        '’' | '‘' => '\'',
        other => other,
    }
}

/// The words of a query: normalised, split on anything that is not a letter
/// or digit, at most [`MAX_TERMS`], each at most 40 characters. Bounded so a
/// pasted essay is still one cheap search.
pub fn terms(query: &str) -> Vec<String> {
    let q: String = query.chars().take(MAX_QUERY).collect();
    let mut out: Vec<String> = Vec::new();
    for w in normalize(&q).split(|c: char| !c.is_alphanumeric()) {
        let w: String = w.chars().take(40).collect();
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
        if out.len() == MAX_TERMS {
            break;
        }
    }
    out
}

/// Each query word with the words that mean the same. A synonym of several
/// words ("search engine optimisation") matches as a phrase.
pub fn expand(terms: &[String], synonyms: &[Vec<String>]) -> Vec<Vec<String>> {
    terms
        .iter()
        .map(|t| {
            let mut alts = vec![t.clone()];
            for group in synonyms {
                if group.iter().any(|w| w == t) {
                    for w in group {
                        if !alts.contains(w) {
                            alts.push(w.clone());
                        }
                    }
                }
            }
            alts
        })
        .collect()
}

// ---------------------------------------------------------------------------
// documents and ranking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Post,
    Page,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Post => "post",
            Kind::Page => "page",
        }
    }
}

/// One post or page, as the engine sees it. Normalised copies of each field
/// are kept beside the originals so a query never re-folds the corpus.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub excerpt: String,
    /// Empty unless the post is public. See the module doc.
    pub body: String,
    pub tags: Vec<(String, String)>,
    pub url: String,
    pub image: Option<String>,
    pub published_at: String,
    pub featured: bool,
    pub visibility: String,
    n_title: String,
    n_excerpt: String,
    n_body: String,
    n_tags: String,
}

/// Bodies are cut here. Past it a match adds nothing a reader would notice,
/// and a site of long posts keeps a bounded index.
pub const MAX_BODY_CHARS: usize = 20_000;

impl Doc {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: Kind,
        title: String,
        excerpt: String,
        body: &str,
        tags: Vec<(String, String)>,
        url: String,
        image: Option<String>,
        published_at: String,
        featured: bool,
        visibility: String,
    ) -> Doc {
        let body: String = if visibility == "public" { body.chars().take(MAX_BODY_CHARS).collect() } else { String::new() };
        let excerpt = excerpt.trim().to_string();
        Doc {
            n_title: normalize(&title),
            n_excerpt: normalize(&excerpt),
            n_body: normalize(&body),
            n_tags: normalize(&tags.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(" · ")),
            id,
            kind,
            title,
            excerpt,
            body,
            tags,
            url,
            image,
            published_at,
            featured,
            visibility,
        }
    }
}

/// How one word matched one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Hit {
    /// Starts a word: "java" in "javascript". Only for the last word typed,
    /// which is the one still being typed.
    Prefix,
    Whole,
}

/// Where `needle` occurs in `hay` as a whole word (or the start of one).
fn find_word(hay: &str, needle: &str, prefix_ok: bool) -> Option<Hit> {
    if needle.is_empty() {
        return None;
    }
    let mut best = None;
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let at = from + i;
        let before_ok = hay[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = hay[at + needle.len()..].chars().next();
        let after_ok = after.is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(Hit::Whole);
        }
        if before_ok && prefix_ok {
            best = Some(Hit::Prefix);
        }
        from = at + needle.len().max(1);
        if from >= hay.len() {
            break;
        }
    }
    best
}

/// A ranked result, with the reasons for the tester to show.
#[derive(Debug, Clone)]
pub struct Scored<'a> {
    pub doc: &'a Doc,
    pub score: f64,
    pub why: Vec<String>,
}

/// Rank every admissible document for a query.
///
/// Every word typed has to match somewhere (in the word itself or one of its
/// synonyms) — an AND across words, an OR within a synonym group — because a
/// search that answers "javascript tutorials" with every tutorial on the site
/// has stopped listening. Each match scores its field's weight, twice for a
/// whole word, once for the start of one; the whole query appearing in the
/// title as a phrase adds the title weight again. A featured post breaks a
/// tie, and then the newer one.
///
/// `kind` and `tag` are the visitor's filters; `None` means all.
pub fn rank<'a>(
    docs: &'a [Doc],
    query: &str,
    s: &Settings,
    kind: Option<Kind>,
    tag: Option<&str>,
) -> Vec<Scored<'a>> {
    let words = terms(query);
    if words.is_empty() {
        return Vec::new();
    }
    let groups = expand(&words, &s.synonyms);
    let phrase = words.join(" ");
    let last = words.len() - 1;
    let mut out: Vec<Scored> = Vec::new();
    for doc in docs {
        if kind.is_some_and(|k| k != doc.kind) {
            continue;
        }
        if let Some(t) = tag {
            if !doc.tags.iter().any(|(_, slug)| slug == t) {
                continue;
            }
        }
        let fields: [(&str, &str, i64); 4] = [
            ("title", &doc.n_title, s.weight_title),
            ("excerpt", &doc.n_excerpt, s.weight_excerpt),
            ("tags", &doc.n_tags, s.weight_tags),
            ("body", if s.full_text { &doc.n_body } else { "" }, s.weight_body),
        ];
        let mut score = 0.0;
        let mut why = Vec::new();
        let mut all = true;
        for (i, alts) in groups.iter().enumerate() {
            let mut best: f64 = 0.0;
            let mut best_why = String::new();
            for alt in alts {
                for (name, hay, weight) in fields {
                    if weight == 0 {
                        continue;
                    }
                    if let Some(hit) = find_word(hay, alt, i == last) {
                        let points = weight as f64 * if hit == Hit::Whole { 2.0 } else { 1.0 };
                        if points > best {
                            best = points;
                            best_why = format!(
                                "{name}: {kind} “{alt}”{via} +{points}",
                                kind = if hit == Hit::Whole { "word" } else { "starts" },
                                via = if *alt != words[i] { format!(" (synonym of “{}”)", words[i]) } else { String::new() },
                            );
                        }
                    }
                }
            }
            if best == 0.0 {
                all = false;
                break;
            }
            score += best;
            why.push(best_why);
        }
        if !all {
            continue;
        }
        if words.len() > 1 && s.weight_title > 0 && doc.n_title.contains(&phrase) {
            score += s.weight_title as f64;
            why.push(format!("title: the whole phrase +{}", s.weight_title));
        }
        if doc.featured {
            score += 0.5;
            why.push("featured +0.5".into());
        }
        out.push(Scored { doc, score, why });
    }
    match s.order {
        Order::Relevance => out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.doc.published_at.cmp(&a.doc.published_at))
        }),
        Order::Newest => out.sort_by(|a, b| b.doc.published_at.cmp(&a.doc.published_at)),
    }
    out
}

/// A short passage of `text` around the first match, and the character
/// ranges inside it to highlight. The browser places the passage with
/// `textContent` and wraps the ranges itself — the server never sends markup.
pub fn snippet(text: &str, groups: &[Vec<String>], max_chars: usize) -> (String, Vec<(usize, usize)>) {
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars.iter().map(|c| fold(*c)).collect();
    let folded_s: String = folded.iter().collect();
    // Character index of the first match of any word.
    let first = groups
        .iter()
        .flatten()
        .filter_map(|w| folded_s.find(w.as_str()).map(|b| folded_s[..b].chars().count()))
        .min();
    let start = match first {
        Some(at) if at > max_chars / 3 => {
            // Back up to a word boundary a little before the match.
            let mut s = at - max_chars / 4;
            while s > 0 && !chars[s - 1].is_whitespace() {
                s -= 1;
            }
            s
        }
        _ => 0,
    };
    let end = (start + max_chars).min(chars.len());
    let mut passage: String = chars[start..end].iter().collect();
    let lead = if start > 0 { 1 } else { 0 };
    if start > 0 {
        passage = format!("…{}", passage.trim_start());
    }
    if end < chars.len() {
        passage = format!("{}…", passage.trim_end());
    }
    let shown: Vec<char> = passage.chars().collect();
    let shown_folded: String = shown.iter().map(|c| fold(*c)).collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for w in groups.iter().flatten() {
        let mut from = 0;
        while let Some(b) = shown_folded[from..].find(w.as_str()) {
            let byte = from + b;
            let s = shown_folded[..byte].chars().count();
            let e = s + w.chars().count();
            // Whole-word starts only, so "art" does not light up "start".
            let ok = s == 0 || !shown[s - 1].is_alphanumeric();
            if ok {
                ranges.push((s, e));
            }
            from = byte + w.len();
        }
    }
    let _ = lead;
    ranges.sort();
    // Merge overlaps.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }
    (passage, merged)
}

/// What Sodo Search indexes as a post's excerpt. Sodo searches title and
/// excerpt only, so with full text on, the start of the body is appended —
/// after the excerpt, which is what Sodo shows first in a result. Never for
/// a post that is not public.
pub fn sodo_excerpt(excerpt: &str, body: &str, visibility: &str, full_text: bool) -> String {
    if !full_text || visibility != "public" {
        return excerpt.to_string();
    }
    let body: String = body.split_whitespace().take(400).collect::<Vec<_>>().join(" ");
    if body.is_empty() {
        return excerpt.to_string();
    }
    if excerpt.trim().is_empty() { body } else { format!("{} {}", excerpt.trim(), body) }
}

/// Insights: one query as stored — normalised and bounded, so "SEO", "seo "
/// and "Séo" count as one search.
pub fn log_form(query: &str) -> Option<String> {
    let t = terms(query);
    (!t.is_empty()).then(|| t.join(" "))
}

/// Tag slugs present on documents, with counts, most common first — the
/// filter chips under the search box.
pub fn facets(results: &[Scored], max: usize) -> Vec<(String, String, usize)> {
    let mut counts: Vec<(String, String, usize)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for r in results {
        for (name, slug) in &r.doc.tags {
            if seen.insert(format!("{}|{slug}", r.doc.id)) {
                match counts.iter_mut().find(|(_, s, _)| s == slug) {
                    Some(c) => c.2 += 1,
                    None => counts.push((name.clone(), slug.clone(), 1)),
                }
            }
        }
    }
    counts.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    counts.truncate(max);
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings::read(|k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string()))
    }

    fn doc(id: &str, title: &str, excerpt: &str, body: &str, tags: &[&str], date: &str, visibility: &str) -> Doc {
        Doc::new(
            id.into(),
            Kind::Post,
            title.into(),
            excerpt.into(),
            body,
            tags.iter().map(|t| (t.to_string(), t.to_lowercase())).collect(),
            format!("/{id}/"),
            None,
            date.into(),
            false,
            visibility.into(),
        )
    }

    #[test]
    fn the_defaults_reproduce_the_search_as_it_was() {
        let s = Settings::default();
        assert_eq!(s.engine, Engine::Sodo);
        assert!(!s.include_pages && s.include_members && !s.full_text && s.show_authors && s.show_tags);
        assert!(s.admits(Kind::Post, "paid", &[]));
        assert!(!s.admits(Kind::Page, "public", &[]));
        assert_eq!(s.limit, 20);
    }

    #[test]
    fn out_of_range_values_are_clamped_or_defaulted() {
        let s = settings(&[(SETTING_WEIGHT_TITLE, "99"), (SETTING_WEIGHT_BODY, "-3"), (SETTING_LIMIT, "5000"), (SETTING_ENGINE, "\"vuuut\"")]);
        assert_eq!((s.weight_title, s.weight_body, s.limit), (10, 0, 20));
        assert_eq!(s.engine, Engine::Vuuut);
    }

    #[test]
    fn excluded_tags_and_members_only_posts_are_left_out_of_both_engines() {
        let s = settings(&[(SETTING_EXCLUDE_TAGS, "Internal, news ,,<b>"), (SETTING_INCLUDE_MEMBERS, "false")]);
        assert_eq!(s.exclude_tags, ["internal", "news", "b"]);
        assert!(!s.admits(Kind::Post, "public", &["news".into()]));
        assert!(!s.admits(Kind::Post, "members", &[]));
        assert!(s.admits(Kind::Post, "public", &["other".into()]));
    }

    #[test]
    fn a_members_only_body_is_never_kept_whatever_the_caller_passes() {
        let d = doc("a", "Paid", "teaser", "secret body text", &[], "2026-01-01", "paid");
        assert!(d.body.is_empty());
        let s = settings(&[(SETTING_FULL_TEXT, "true")]);
        let docs = vec![d];
        assert!(rank(&docs, "secret", &s, None, None).is_empty());
        assert_eq!(sodo_excerpt("teaser", "secret body", "paid", true), "teaser");
    }

    #[test]
    fn every_word_must_match_and_a_title_outranks_a_body() {
        let s = settings(&[(SETTING_FULL_TEXT, "true")]);
        let docs = vec![
            doc("body", "Notes", "", "a guide to rust macros", &[], "2026-03-01", "public"),
            doc("title", "Rust macros", "", "", &[], "2026-01-01", "public"),
            doc("half", "Rust", "", "", &[], "2026-05-01", "public"),
        ];
        let ids: Vec<&str> = rank(&docs, "rust macros", &s, None, None).iter().map(|r| r.doc.id.as_str()).collect();
        assert_eq!(ids, ["title", "body"]);
    }

    #[test]
    fn the_body_is_only_searched_when_full_text_is_on() {
        let docs = vec![doc("b", "Notes", "", "about kubernetes", &[], "2026-01-01", "public")];
        assert!(rank(&docs, "kubernetes", &Settings::default(), None, None).is_empty());
        assert_eq!(rank(&docs, "kubernetes", &settings(&[(SETTING_FULL_TEXT, "1")]), None, None).len(), 1);
    }

    #[test]
    fn the_last_word_matches_as_a_prefix_and_earlier_ones_do_not() {
        let docs = vec![doc("j", "JavaScript patterns", "", "", &[], "2026-01-01", "public")];
        let s = Settings::default();
        assert_eq!(rank(&docs, "javas", &s, None, None).len(), 1, "still typing");
        assert!(rank(&docs, "javas patterns", &s, None, None).is_empty(), "a finished word is a whole word");
        assert!(rank(&docs, "script", &s, None, None).is_empty(), "not the middle of a word");
    }

    #[test]
    fn synonyms_and_accents_find_the_same_post() {
        let s = settings(&[(SETTING_SYNONYMS, "js, javascript\nseo, search engine optimisation\nnope")]);
        assert_eq!(s.synonyms.len(), 2, "a line with one word is not a group");
        let docs = vec![
            doc("a", "Learning JavaScript", "", "", &[], "2026-01-01", "public"),
            doc("b", "Café culture", "", "", &[], "2026-01-01", "public"),
            doc("c", "A search engine optimisation primer", "", "", &[], "2026-01-01", "public"),
        ];
        let hits = rank(&docs, "JS", &s, None, None);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].why[0].contains("synonym"), "{:?}", hits[0].why);
        assert_eq!(rank(&docs, "cafe", &s, None, None).len(), 1);
        assert_eq!(rank(&docs, "seo", &s, None, None)[0].doc.id, "c");
    }

    #[test]
    fn filters_narrow_by_type_and_tag() {
        let mut page = doc("p", "Rust page", "", "", &["Guides"], "2026-01-01", "public");
        page.kind = Kind::Page;
        let docs = vec![page, doc("q", "Rust post", "", "", &["News"], "2026-01-01", "public")];
        let s = Settings::default();
        assert_eq!(rank(&docs, "rust", &s, Some(Kind::Page), None)[0].doc.id, "p");
        assert_eq!(rank(&docs, "rust", &s, None, Some("news"))[0].doc.id, "q");
        let all = rank(&docs, "rust", &s, None, None);
        assert_eq!(facets(&all, 10).len(), 2);
    }

    #[test]
    fn newest_order_ignores_the_score() {
        let s = settings(&[(SETTING_ORDER, "newest")]);
        let docs = vec![
            doc("old", "Rust rust", "rust", "", &["rust"], "2020-01-01", "public"),
            doc("new", "Notes", "rust", "", &[], "2026-01-01", "public"),
        ];
        assert_eq!(rank(&docs, "rust", &s, None, None)[0].doc.id, "new");
    }

    #[test]
    fn a_zero_weight_field_is_not_searched() {
        let s = settings(&[(SETTING_WEIGHT_EXCERPT, "0")]);
        let docs = vec![doc("e", "Title", "hidden word", "", &[], "2026-01-01", "public")];
        assert!(rank(&docs, "hidden", &s, None, None).is_empty());
    }

    #[test]
    fn a_snippet_centres_on_the_match_and_marks_it_in_characters() {
        let text = format!("{} Café latte is the thing", "word ".repeat(60));
        let (passage, ranges) = snippet(&text, &[vec!["cafe".into()]], 80);
        assert!(passage.starts_with('…'));
        let marked: String = passage.chars().skip(ranges[0].0).take(ranges[0].1 - ranges[0].0).collect();
        assert_eq!(marked, "Café");
        let (_, none) = snippet("restart the art", &[vec!["art".into()]], 80);
        assert_eq!(none, vec![(12, 15)], "the start of a word only");
    }

    #[test]
    fn queries_are_bounded_and_logged_in_one_form() {
        assert_eq!(terms(&"a ".repeat(50)).len(), 1);
        assert_eq!(terms("one two three four five six seven eight nine").len(), MAX_TERMS);
        assert_eq!(log_form("  SÉO, Tips! "), Some("seo tips".into()));
        assert_eq!(log_form("  !!! "), None);
    }

    #[test]
    fn sodo_gets_the_body_after_the_excerpt_only_when_asked() {
        assert_eq!(sodo_excerpt("Short.", "the body", "public", false), "Short.");
        assert_eq!(sodo_excerpt("Short.", "the  body", "public", true), "Short. the body");
    }
}
