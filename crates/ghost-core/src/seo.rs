//! The SEO module's rules: the settings it owns, and the audit it runs over
//! a site's own content.
//!
//! Two halves, both pure, so the renderer, the public robots.txt handler, the
//! admin dashboard and the tests all answer the same questions the same way:
//!
//! * **[`flag`] and the tables around it** — the `seo_*` settings, their
//!   types and their defaults. The defaults matter more than they look: every
//!   one of them is the behaviour a site had *before* the module existed, so
//!   installing it changes nothing until somebody changes a switch. The one
//!   place that is not true is [`DEFAULT_ROBOTS`], which replaces a 404.
//! * **[`audit`]** — the checks behind the SEO report. Each one is a rule
//!   about a single entry with a severity and a sentence a writer can act on.
//!   Nothing here reads the database or knows what a post is: it takes an
//!   [`Entry`] of already-loaded fields, which is what makes the thresholds
//!   testable and keeps the thresholds in one place rather than scattered
//!   through the page that renders them.
//!
//! ## Why the thresholds are what they are
//!
//! They are the lengths search engines actually render, not a standard:
//! Google truncates a title at roughly 600 CSS pixels (~60 characters in its
//! default face) and a description at ~920px on desktop (~160 characters), and
//! both numbers have moved before. So a title over [`TITLE_MAX`] or a
//! description over [`DESCRIPTION_MAX`] is reported as a *warning* — the page
//! works, the tail is just not shown — while something missing altogether is
//! an *error*. No check here is a reason not to publish.

use serde_json::Value;

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

/// Every `seo_*` switch, with the default that reproduces the behaviour the
/// site had before the SEO module existed.
///
/// `seo_structured_data` is on because Ghost emits Open Graph, Twitter cards
/// and JSON-LD unless `privacy.useStructuredData` is explicitly disabled;
/// everything else is off or neutral.
pub const BOOL_SETTINGS: &[(&str, bool)] = &[
    // Emit the og/twitter/article tags and the schema.org JSON-LD block.
    ("seo_structured_data", true),
    // Append " - {site title}" to a post or page title, which this port used
    // to do unconditionally. Ghost does not, so the default is off.
    ("seo_append_site_title", false),
    // `<meta name="robots" content="noindex,follow">` on archives. Off: a tag
    // or author archive is a legitimate landing page for most sites, and
    // hiding it is a choice, not a fix.
    ("seo_noindex_tag_pages", false),
    ("seo_noindex_author_pages", false),
    ("seo_noindex_paged_pages", false),
    // `<image:image>` entries in the post and page sitemaps.
    ("seo_sitemap_images", true),
];

/// The `Referrer-Policy` the head advertises. Ghost's default is first.
pub const REFERRER_POLICIES: &[(&str, &str)] = &[
    ("no-referrer-when-downgrade", "Send the full URL, except from HTTPS to HTTP (Ghost default)"),
    ("strict-origin-when-cross-origin", "Send only the origin to other sites"),
    ("same-origin", "Send nothing to other sites"),
    ("no-referrer", "Never send a referrer"),
    ("origin", "Always send only the origin"),
    ("unsafe-url", "Always send the full URL"),
];

pub const DEFAULT_REFERRER_POLICY: &str = "no-referrer-when-downgrade";

/// The robots.txt a site serves when nothing custom is saved.
///
/// `{{sitemap}}` is replaced with the absolute sitemap URL by the handler.
/// The disallowed paths are Ghost's own list: the admin, the email and
/// redirect endpoints, the comment-count API and the webmention receiver —
/// none of them are pages, and every one of them costs crawl budget. Note
/// what is *not* here: no content path is blocked, because a site that wants
/// to be read wants to be crawled.
pub const DEFAULT_ROBOTS: &str = "\
User-agent: *
Sitemap: {{sitemap}}
Disallow: /ghost/
Disallow: /p/
Disallow: /email/
Disallow: /members/api/comments/counts/
Disallow: /r/
Disallow: /webmentions/receive/
";

/// The most a custom robots.txt may be. Large enough for any real file;
/// small enough that the setting cannot be used as storage.
pub const ROBOTS_MAX: usize = 8000;

/// The default for one boolean setting, or `false` for a key that is not one.
pub fn bool_default(key: &str) -> bool {
    BOOL_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, d)| *d).unwrap_or(false)
}

/// Read one boolean `seo_*` setting out of a JSON object (the render root's
/// `site`, or a settings payload), falling back to the key's default.
///
/// Both shapes are accepted because the settings table has held both over its
/// life: a real JSON `true`/`false`, and the string `"true"`/`"false"` that
/// older rows and form posts produce. A key that is absent, null or
/// unrecognised gets the default rather than `false` — otherwise adding the
/// module to an existing site would silently turn structured data off.
pub fn flag(container: &Value, key: &str) -> bool {
    match container.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim() {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => bool_default(key),
        },
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or_else(|| bool_default(key)),
        _ => bool_default(key),
    }
}

/// The referrer policy from a settings container, reduced to a value we offer.
///
/// An unknown string is not passed through: it would reach every page of the
/// site as a header-equivalent meta tag, and a browser that cannot parse it
/// falls back to its own default anyway.
pub fn referrer_policy(container: &Value) -> &'static str {
    let saved = container.get("seo_referrer_policy").and_then(Value::as_str).unwrap_or("").trim();
    REFERRER_POLICIES
        .iter()
        .find(|(value, _)| *value == saved)
        .map(|(value, _)| *value)
        .unwrap_or(DEFAULT_REFERRER_POLICY)
}

// ---------------------------------------------------------------------------
// the audit
// ---------------------------------------------------------------------------

/// Recommended bounds. See the module docs: these are what gets *rendered*,
/// not what is allowed.
pub const TITLE_MIN: usize = 15;
pub const TITLE_MAX: usize = 60;
pub const DESCRIPTION_MIN: usize = 50;
pub const DESCRIPTION_MAX: usize = 160;
/// Under this, a page rarely has enough text to rank for anything.
pub const THIN_CONTENT_WORDS: i64 = 300;
/// A slug longer than this is usually a sentence that wanted to be a title.
pub const SLUG_MAX: usize = 75;

/// How much a finding matters. An `Error` is something a crawler cannot work
/// around; a `Warning` is something it will handle less well than it could.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One finding about one entry.
///
/// `code` is stable and is what the UI filters and groups on; `message` is
/// for the person reading it and may be reworded freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// Every check, with the heading the report groups it under. The order is the
/// order the report lists them in: the ones that cost the most traffic first.
pub const CHECKS: &[(&str, Severity, &str)] = &[
    ("no_description", Severity::Error, "No meta description"),
    ("no_title", Severity::Error, "No title"),
    ("title_long", Severity::Warning, "Title is too long"),
    ("title_short", Severity::Warning, "Title is very short"),
    ("description_long", Severity::Warning, "Meta description is too long"),
    ("description_short", Severity::Warning, "Meta description is very short"),
    ("no_image", Severity::Warning, "No social share image"),
    ("no_image_alt", Severity::Warning, "Feature image has no alt text"),
    ("thin_content", Severity::Warning, "Very little text"),
    ("no_tags", Severity::Warning, "No tags"),
    ("slug_long", Severity::Warning, "Slug is very long"),
    ("duplicate_title", Severity::Error, "Another entry has the same title"),
    ("duplicate_description", Severity::Warning, "Another entry has the same meta description"),
    ("canonical_elsewhere", Severity::Warning, "Points its canonical URL at another address"),
];

pub fn check_label(code: &str) -> &'static str {
    CHECKS.iter().find(|(c, _, _)| *c == code).map(|(_, _, label)| *label).unwrap_or("Issue")
}

/// The fields one entry is judged on. Everything is already resolved — this
/// type is deliberately not a `Post`, so the rules cannot reach past what
/// they are given.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub slug: String,
    /// `"post"` or `"page"`.
    pub kind: String,
    pub status: String,
    pub visibility: String,
    pub published_at: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub custom_excerpt: Option<String>,
    pub feature_image: Option<String>,
    pub feature_image_alt: Option<String>,
    pub og_image: Option<String>,
    pub twitter_image: Option<String>,
    pub canonical_url: Option<String>,
    pub word_count: i64,
    pub tag_count: i64,
    /// Set by [`audit_all`] when a second entry shares this one's title.
    pub duplicate_title: bool,
    /// Likewise for the resolved meta description.
    pub duplicate_description: bool,
}

fn trimmed(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl Entry {
    /// The title a search result would show: the SEO title if one was
    /// written, otherwise the entry's own.
    pub fn effective_title(&self) -> &str {
        trimmed(&self.meta_title).unwrap_or_else(|| self.title.trim())
    }

    /// The description a search result would show — and `None` where the page
    /// genuinely has none, which is the single most common finding. Mirrors
    /// what `{{ghost_head}}` emits: the meta description, then the custom
    /// excerpt, and nothing else. The automated excerpt is *not* in this
    /// chain, because Ghost does not publish it as a description either.
    pub fn effective_description(&self) -> Option<&str> {
        trimmed(&self.meta_description).or_else(|| trimmed(&self.custom_excerpt))
    }

    /// Whether anything would appear as the share image.
    pub fn has_image(&self) -> bool {
        trimmed(&self.og_image).is_some()
            || trimmed(&self.twitter_image).is_some()
            || trimmed(&self.feature_image).is_some()
    }

    pub fn is_published(&self) -> bool {
        self.status == "published" || self.status == "sent"
    }
}

/// Count the characters a search engine counts — Unicode scalar values, not
/// bytes. An emoji in a title is one glyph, and a byte length would report a
/// four-times-too-long title for a site written in Bengali.
fn len(s: &str) -> usize {
    s.chars().count()
}

/// Run every check against one entry.
pub fn audit(entry: &Entry) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |code: &'static str, message: String| {
        let severity = CHECKS
            .iter()
            .find(|(c, _, _)| *c == code)
            .map(|(_, s, _)| *s)
            .unwrap_or(Severity::Warning);
        out.push(Finding { code, severity, message });
    };

    let title = entry.effective_title();
    if title.is_empty() {
        add("no_title", "This entry has no title at all.".into());
    } else {
        let n = len(title);
        if n > TITLE_MAX {
            add("title_long", format!("The title is {n} characters; search results cut off around {TITLE_MAX}."));
        } else if n < TITLE_MIN {
            add("title_short", format!("The title is only {n} characters — there is room to say more."));
        }
    }

    match entry.effective_description() {
        None => add(
            "no_description",
            "No meta description or excerpt, so the search result snippet is whatever the crawler picks.".into(),
        ),
        Some(description) => {
            let n = len(description);
            if n > DESCRIPTION_MAX {
                add("description_long", format!("The description is {n} characters; around {DESCRIPTION_MAX} is shown."));
            } else if n < DESCRIPTION_MIN {
                add("description_short", format!("The description is only {n} characters — most results show about {DESCRIPTION_MAX}."));
            }
        }
    }

    if !entry.has_image() {
        add("no_image", "Nothing to show when this is shared: no feature, social or site image.".into());
    } else if trimmed(&entry.feature_image).is_some() && trimmed(&entry.feature_image_alt).is_none() {
        add("no_image_alt", "The feature image has no alt text.".into());
    }

    if entry.word_count > 0 && entry.word_count < THIN_CONTENT_WORDS {
        add("thin_content", format!("About {} words; under {THIN_CONTENT_WORDS} rarely ranks for anything.", entry.word_count));
    }

    // A page is not expected to be tagged — tags are the post taxonomy.
    if entry.kind == "post" && entry.tag_count == 0 {
        add("no_tags", "No tags, so this post is in no archive and has no related posts.".into());
    }

    if len(&entry.slug) > SLUG_MAX {
        add("slug_long", format!("The slug is {} characters long.", len(&entry.slug)));
    }

    if entry.duplicate_title {
        add("duplicate_title", "Another entry has the same title, so the two compete for the same result.".into());
    }
    if entry.duplicate_description {
        add("duplicate_description", "Another entry has the same meta description.".into());
    }
    if trimmed(&entry.canonical_url).is_some() {
        add(
            "canonical_elsewhere",
            "The canonical URL points somewhere else, so this copy is deliberately not indexed.".into(),
        );
    }

    out
}

/// A score out of 100 for one entry's findings.
///
/// An error costs more than a warning, and the floor is 0 — a page with eight
/// problems is not more broken than one with six, it is just unfinished.
pub fn score(findings: &[Finding]) -> u8 {
    let cost: i32 = findings
        .iter()
        .map(|f| match f.severity {
            Severity::Error => 20,
            Severity::Warning => 8,
        })
        .sum();
    (100 - cost).clamp(0, 100) as u8
}

/// Fill in the two cross-entry flags, then audit each entry.
///
/// Duplicates can only be found by looking at the whole set, which is why
/// they are flags on [`Entry`] rather than checks inside [`audit`]: the rule
/// stays a pure function of one entry, and the one thing it cannot know is
/// computed once here instead of once per entry.
///
/// Only published entries are compared. Two drafts with the title "Untitled"
/// are not competing for anything.
pub fn audit_all(entries: &mut [Entry]) -> Vec<(usize, Vec<Finding>)> {
    use std::collections::HashMap;
    let mut titles: HashMap<String, usize> = HashMap::new();
    let mut descriptions: HashMap<String, usize> = HashMap::new();
    for entry in entries.iter() {
        if !entry.is_published() {
            continue;
        }
        let title = entry.effective_title().to_lowercase();
        if !title.is_empty() {
            *titles.entry(title).or_insert(0) += 1;
        }
        if let Some(description) = entry.effective_description() {
            *descriptions.entry(description.to_lowercase()).or_insert(0) += 1;
        }
    }
    for entry in entries.iter_mut() {
        if !entry.is_published() {
            entry.duplicate_title = false;
            entry.duplicate_description = false;
            continue;
        }
        entry.duplicate_title = titles
            .get(&entry.effective_title().to_lowercase())
            .is_some_and(|n| *n > 1);
        entry.duplicate_description = entry
            .effective_description()
            .and_then(|d| descriptions.get(&d.to_lowercase()))
            .is_some_and(|n| *n > 1);
    }
    entries.iter().enumerate().map(|(i, e)| (i, audit(e))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good() -> Entry {
        Entry {
            id: "1".into(),
            title: "A perfectly reasonable title for a post".into(),
            slug: "a-perfectly-reasonable-title".into(),
            kind: "post".into(),
            status: "published".into(),
            visibility: "public".into(),
            meta_description: Some(
                "A description of exactly the sort of length that a search engine will show in full.".into(),
            ),
            feature_image: Some("/content/images/x.jpg".into()),
            feature_image_alt: Some("A photograph".into()),
            word_count: 900,
            tag_count: 2,
            ..Entry::default()
        }
    }

    fn codes(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn a_complete_entry_has_nothing_to_report_and_scores_full_marks() {
        let findings = audit(&good());
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(score(&findings), 100);
    }

    #[test]
    fn a_missing_description_is_the_headline_finding() {
        let mut e = good();
        e.meta_description = None;
        assert_eq!(codes(&audit(&e)), ["no_description"]);
        // the custom excerpt counts, because ghost_head publishes it
        e.custom_excerpt = Some("A sentence that is long enough to be a useful search result snippet.".into());
        assert!(audit(&e).is_empty());
    }

    #[test]
    fn the_automated_excerpt_does_not_count_as_a_description() {
        // `{{ghost_head}}` does not publish it, so neither does the audit:
        // reporting a page as described when nothing is emitted for it would
        // make the report lie in the one direction that matters.
        let mut e = good();
        e.meta_description = None;
        e.custom_excerpt = None;
        assert_eq!(e.effective_description(), None);
    }

    #[test]
    fn lengths_are_counted_in_characters_not_bytes() {
        // A byte count would report a four-times-too-long title for any site
        // not written in ASCII.
        let mut e = good();
        e.title = "বাংলায় একটি শিরোনাম যা যথেষ্ট দীর্ঘ".into();
        assert!(!codes(&audit(&e)).contains(&"title_long"), "{:?}", audit(&e));
    }

    #[test]
    fn an_over_long_title_or_description_is_a_warning_not_an_error() {
        let mut e = good();
        e.title = "x".repeat(TITLE_MAX + 1);
        e.meta_description = Some("y".repeat(DESCRIPTION_MAX + 1));
        let findings = audit(&e);
        assert_eq!(codes(&findings), ["title_long", "description_long"]);
        assert!(findings.iter().all(|f| f.severity == Severity::Warning), "{findings:?}");
    }

    #[test]
    fn a_page_is_not_asked_for_tags_but_a_post_is() {
        let mut post = good();
        post.tag_count = 0;
        assert!(codes(&audit(&post)).contains(&"no_tags"));
        let mut page = post.clone();
        page.kind = "page".into();
        assert!(!codes(&audit(&page)).contains(&"no_tags"));
    }

    #[test]
    fn any_of_the_three_images_answers_the_share_image_check() {
        let mut e = good();
        e.feature_image = None;
        e.feature_image_alt = None;
        assert!(codes(&audit(&e)).contains(&"no_image"));
        for key in ["og", "twitter"] {
            let mut with = e.clone();
            match key {
                "og" => with.og_image = Some("/content/images/og.png".into()),
                _ => with.twitter_image = Some("/content/images/tw.png".into()),
            }
            assert!(audit(&with).is_empty(), "{key}: {:?}", audit(&with));
        }
    }

    #[test]
    fn a_feature_image_without_alt_text_is_reported_once_the_image_exists() {
        let mut e = good();
        e.feature_image_alt = None;
        assert_eq!(codes(&audit(&e)), ["no_image_alt"]);
    }

    #[test]
    fn duplicates_are_only_counted_between_published_entries() {
        let mut entries = vec![good(), good(), good()];
        entries[0].id = "a".into();
        entries[1].id = "b".into();
        entries[2].id = "c".into();
        entries[2].status = "draft".into();
        let reports = audit_all(&mut entries);
        assert!(codes(&reports[0].1).contains(&"duplicate_title"));
        assert!(codes(&reports[1].1).contains(&"duplicate_title"));
        // the draft shares the title but competes with nothing
        assert!(!codes(&reports[2].1).contains(&"duplicate_title"));

        // and a single published entry is not its own duplicate
        let mut one = vec![good()];
        assert!(audit_all(&mut one)[0].1.is_empty());
    }

    #[test]
    fn a_title_differing_only_in_case_is_still_a_duplicate() {
        let mut entries = vec![good(), good()];
        entries[1].title = entries[0].title.to_uppercase();
        let reports = audit_all(&mut entries);
        assert!(codes(&reports[0].1).contains(&"duplicate_title"));
    }

    #[test]
    fn the_score_falls_faster_for_an_error_than_for_a_warning() {
        let error = vec![Finding { code: "no_description", severity: Severity::Error, message: String::new() }];
        let warning = vec![Finding { code: "no_tags", severity: Severity::Warning, message: String::new() }];
        assert_eq!(score(&error), 80);
        assert_eq!(score(&warning), 92);
        // and it never goes below zero however long the list is
        let many: Vec<Finding> = (0..20)
            .map(|_| Finding { code: "no_title", severity: Severity::Error, message: String::new() })
            .collect();
        assert_eq!(score(&many), 0);
    }

    #[test]
    fn an_empty_body_is_not_reported_as_thin_content() {
        // A word count of zero means nothing was measured — a page that is
        // all cards, say. Reporting it as thin would cry wolf.
        let mut e = good();
        e.word_count = 0;
        assert!(!codes(&audit(&e)).contains(&"thin_content"));
        e.word_count = 10;
        assert!(codes(&audit(&e)).contains(&"thin_content"));
    }

    #[test]
    fn a_setting_that_was_never_written_reads_as_its_default() {
        // This is the whole safety property of installing the module: an
        // absent key must not read as `false` and silently remove the
        // structured data a site already had.
        let empty = json!({});
        assert!(flag(&empty, "seo_structured_data"));
        assert!(flag(&empty, "seo_sitemap_images"));
        assert!(!flag(&empty, "seo_append_site_title"));
        assert!(flag(&json!({"seo_structured_data": null}), "seo_structured_data"));
        assert!(!flag(&json!({"seo_structured_data": false}), "seo_structured_data"));
        // the string forms older rows and form posts produce
        assert!(!flag(&json!({"seo_structured_data": "false"}), "seo_structured_data"));
        assert!(flag(&json!({"seo_append_site_title": "true"}), "seo_append_site_title"));
        // and nonsense gets the default, not `false`
        assert!(flag(&json!({"seo_structured_data": "yes please"}), "seo_structured_data"));
    }

    #[test]
    fn an_unoffered_referrer_policy_is_refused() {
        assert_eq!(referrer_policy(&json!({})), DEFAULT_REFERRER_POLICY);
        assert_eq!(referrer_policy(&json!({"seo_referrer_policy": "same-origin"})), "same-origin");
        assert_eq!(
            referrer_policy(&json!({"seo_referrer_policy": "\" onload=\"alert(1)"})),
            DEFAULT_REFERRER_POLICY
        );
    }
}
