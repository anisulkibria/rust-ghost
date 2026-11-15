//! What a search engine and a social network read off a page: the meta
//! description, the canonical link, the Open Graph and Twitter card tags, and
//! the schema.org JSON-LD block.
//!
//! This is the port of Ghost's `core/frontend/meta/*` — `get-meta.js` and the
//! twenty small resolvers around it — pulled out of `{{ghost_head}}` into one
//! place that takes plain JSON and returns strings. Nothing here touches the
//! database, the HTTP layer or handlebars, so every rule below is directly
//! testable, and the tests at the bottom of this file are the specification.
//!
//! ## Why it is a separate module
//!
//! Ghost's head output is a long list of small precedence rules — "the og
//! title is the post's `og_title`, or its `meta_title`, or its title; the
//! twitter title is the post's `twitter_title`, or the og title" — and each
//! one is wrong in a way nobody notices for months. They were previously
//! spread through the body of the `ghost_head` helper, interleaved with the
//! script tags for Portal and Sodo Search, which is why several of them were
//! missing entirely (`article:published_time`, the whole JSON-LD block) and
//! two were subtly different from Ghost (the `<title>` the helper emitted on
//! top of the theme's own, and the site name appended to every post title).
//!
//! ## The two rules that are easy to get wrong
//!
//! * **A paged archive gets no social tags and no schema at all.** Ghost
//!   checks `!_.includes(context, 'paged')` before emitting either. Page 2 of
//!   an archive is not a thing anybody shares, and tagging it as the same
//!   `og:url` as page 1 is a duplicate-content signal.
//! * **JSON-LD is JSON, not HTML.** The block is raw text to an HTML parser,
//!   which stops only at the literal `</script`, and a structured-data
//!   consumer parses it as JSON and never HTML-decodes. HTML-escaping the
//!   values would therefore index `Tom &amp; Jerry` literally; see
//!   [`escape_json_ld`].

use serde_json::{json, Map, Value};

/// Ghost's default, and the value its `referrerPolicy` config defaults to.
pub const DEFAULT_REFERRER_POLICY: &str = "no-referrer-when-downgrade";

/// Everything the head of one rendered page is built from.
///
/// `resource` is the post, page, tag or author the route resolved to, or
/// `Null` on a collection; `context` is the render context list (`["post"]`,
/// `["paged", "tag"]`, …). `meta_title` and `meta_description` arrive already
/// resolved because themes read the same two values through `{{meta_title}}`
/// and `{{meta_description}}`, and the two must agree.
pub struct PageMeta {
    pub site: Value,
    pub resource: Value,
    pub context: Vec<String>,
    /// The site URL with no trailing slash.
    pub origin: String,
    /// The request path, base path already stripped.
    pub relative_url: String,
    pub pagination: Value,
    pub meta_title: String,
    pub meta_description: String,
    /// Whether the site wants structured data at all (Settings → SEO).
    pub structured_data: bool,
}

/// The characters that could end the inline `<script>` early, as JSON `\u`
/// escapes — which `JSON.parse` and every conformant structured-data parser
/// decode back to the original character, so the data round-trips exactly.
///
/// HTML-entity escaping here would be the wrong layer *and* actively harmful:
/// the block is read as JSON and never HTML-decoded, so `&` must stay `&`.
pub fn escape_json_ld(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A non-empty string field, or `None` — the shape every precedence chain
/// below is written in.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn meta_tag(property: &str, content: &str) -> String {
    let attr = if property.starts_with("twitter") { "name" } else { "property" };
    format!(r#"<meta {attr}="{property}" content="{}" />"#, esc(content))
}

impl PageMeta {
    pub fn has(&self, name: &str) -> bool {
        self.context.iter().any(|c| c == name)
    }

    /// A post or a static page — the two contexts that carry an entry.
    pub fn is_entry(&self) -> bool {
        self.has("post") || self.has("page")
    }

    /// A page that is asking not to be indexed: a draft preview, or an error
    /// page. Describing either one to a social network is worse than saying
    /// nothing — a shared 404 would preview as the site's own title and
    /// description, which reads as a real page.
    pub fn unindexed(&self) -> bool {
        self.has("preview") || self.context.iter().any(|c| c.starts_with("error-"))
    }

    fn site_str(&self, key: &str) -> Option<String> {
        str_field(&self.site, key)
    }

    fn own(&self, key: &str) -> Option<String> {
        str_field(&self.resource, key)
    }

    /// An image path from the data, made absolute. Site and resource images
    /// reach here already carrying the install's base path.
    fn image(&self, path: Option<String>) -> Option<String> {
        path.map(|p| ghost_core::url::absolute(&self.origin, &p))
    }

    /// The page's own address, absolute. Not the canonical — a post that was
    /// reposted from elsewhere points its canonical at the original, but its
    /// schema still describes the copy that is being read.
    pub fn url(&self) -> String {
        format!("{}{}", self.origin, self.relative_url)
    }

    /// Ghost's `getCanonicalUrl`: a post, page or tag may override it.
    pub fn canonical(&self) -> String {
        if self.is_entry() || self.has("tag") {
            if let Some(own) = self.own("canonical_url") {
                return own;
            }
        }
        self.url()
    }

    /// `og:type`. Ghost reads it off the context: an author profile is a
    /// `profile`, a post is an `article`, and everything else — including a
    /// static page and a tag archive — is a `website`.
    pub fn og_type(&self) -> &'static str {
        if self.has("author") {
            "profile"
        } else if self.has("post") {
            "article"
        } else {
            "website"
        }
    }

    /// `getTitle(data, root, {property: 'og'|'twitter'})`.
    ///
    /// On a resource the resource's own card title wins; the site-level
    /// `og_title`/`twitter_title` are only consulted on the home page, which
    /// is the one place Ghost reads them.
    fn card_title(&self, property: &str) -> String {
        let key = format!("{property}_title");
        if self.has("home") {
            return self
                .site_str(&key)
                .or_else(|| self.site_str("title"))
                .unwrap_or_default();
        }
        self.own(&key).unwrap_or_else(|| self.meta_title.clone())
    }

    /// `getDescription(data, root, {property})`.
    ///
    /// The card description has a longer fallback chain than
    /// `<meta name="description">`: where the plain one stops at the fields a
    /// human filled in, this one falls through the excerpt to the site
    /// description, because an untagged card is a blank grey box.
    fn card_description(&self, property: &str) -> String {
        let key = format!("{property}_description");
        if self.has("home") {
            return self
                .site_str(&key)
                .or_else(|| self.site_str("meta_description"))
                .or_else(|| self.site_str("description"))
                .unwrap_or_default();
        }
        let own = self.own(&key);
        if self.is_entry() {
            own.or_else(|| self.own("custom_excerpt"))
                .or_else(|| self.own("meta_description"))
                .or_else(|| self.own("excerpt"))
                .or_else(|| self.site_str("description"))
                .unwrap_or_default()
        } else {
            // tag and author archives: their own text, then the site's
            own.or_else(|| self.own("meta_description"))
                .or_else(|| self.own("description"))
                .or_else(|| self.own("bio"))
                .or_else(|| self.site_str("meta_description"))
                .unwrap_or_default()
        }
    }

    pub fn og_title(&self) -> String {
        self.card_title("og")
    }

    /// Note what this does *not* do: fall back to the og title. Ghost runs
    /// the two chains independently, so a post that sets only `og_title`
    /// still tweets under its own title — setting one card's wording is not a
    /// statement about the other's.
    pub fn twitter_title(&self) -> String {
        self.card_title("twitter")
    }

    pub fn og_description(&self) -> String {
        self.card_description("og")
    }

    pub fn twitter_description(&self) -> String {
        self.card_description("twitter")
    }

    /// `getCoverImage`: the image that represents the page. On the home page
    /// and an author archive that is the cover; everywhere else it is the
    /// entry's or the tag's feature image.
    pub fn cover_image(&self) -> Option<String> {
        let path = if self.has("home") {
            self.site_str("cover_image")
        } else if self.has("author") {
            self.own("cover_image")
        } else {
            self.own("feature_image")
        };
        self.image(path)
    }

    /// `getOgImage`.
    pub fn og_image(&self) -> Option<String> {
        let path = if self.has("home") {
            self.site_str("og_image").or_else(|| self.site_str("cover_image"))
        } else if self.is_entry() {
            self.own("og_image")
                .or_else(|| self.own("feature_image"))
                .or_else(|| self.site_str("og_image"))
                .or_else(|| self.site_str("cover_image"))
        } else if self.has("author") {
            self.own("cover_image")
        } else if self.has("tag") {
            self.own("og_image")
                .or_else(|| self.own("feature_image"))
                .or_else(|| self.site_str("cover_image"))
        } else {
            None
        };
        self.image(path)
    }

    /// `getTwitterImage`.
    pub fn twitter_image(&self) -> Option<String> {
        let path = if self.has("home") {
            self.site_str("twitter_image").or_else(|| self.site_str("cover_image"))
        } else if self.is_entry() {
            self.own("twitter_image")
                .or_else(|| self.own("feature_image"))
                .or_else(|| self.site_str("twitter_image"))
                .or_else(|| self.site_str("cover_image"))
        } else if self.has("author") {
            self.own("cover_image")
        } else if self.has("tag") {
            self.own("twitter_image")
                .or_else(|| self.own("feature_image"))
                .or_else(|| self.site_str("cover_image"))
        } else {
            None
        };
        self.image(path)
    }

    /// The publication logo for the schema's publisher, falling back to the
    /// icon — Google wants an Organization to have one
    /// (TryGhost/Ghost#7558), and most sites set only one of the two.
    fn publisher_logo(&self) -> Option<String> {
        self.image(self.site_str("logo").or_else(|| self.site_str("icon")))
    }

    /// `article:tag` and the schema's keywords: the names of the entry's
    /// public tags. Internal tags (`#hash`) are deliberately not published.
    pub fn keywords(&self) -> Vec<String> {
        self.resource
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter(|t| t.get("visibility").and_then(Value::as_str).unwrap_or("public") == "public")
                    .filter_map(|t| str_field(t, "name"))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn published_date(&self) -> Option<String> {
        self.own("published_at")
    }

    pub fn modified_date(&self) -> Option<String> {
        self.own("updated_at")
    }

    /// The entry's primary author, or the author whose archive this is.
    fn author(&self) -> Option<&Value> {
        if self.is_entry() {
            self.resource.get("primary_author").filter(|v| v.is_object())
        } else if self.has("author") {
            Some(&self.resource).filter(|v| v.is_object())
        } else {
            None
        }
    }

    pub fn author_name(&self) -> Option<String> {
        self.author().and_then(|a| str_field(a, "name"))
    }

    pub fn author_url(&self) -> Option<String> {
        self.author()
            .and_then(|a| str_field(a, "url"))
            .map(|u| ghost_core::url::absolute(&self.origin, &u))
    }

    fn author_image(&self) -> Option<String> {
        self.image(self.author().and_then(|a| str_field(a, "profile_image")))
    }

    /// `twitter:creator` — the handle Ghost stores, which it writes out as it
    /// was stored (`@handle`), not as a URL.
    fn creator_twitter(&self) -> Option<String> {
        self.author().and_then(|a| str_field(a, "twitter"))
    }

    /// `article:author` — a Facebook profile URL, built from the handle.
    fn author_facebook(&self) -> Option<String> {
        self.author()
            .and_then(|a| str_field(a, "facebook"))
            .and_then(|h| crate::social::url_for("facebook", &h))
    }

    /// The URL of page `n` of this archive, Ghost's `getPaginatedUrl`.
    fn paginated(&self, page: i64) -> String {
        let base = match self.relative_url.find("/page/") {
            Some(i) => self.relative_url[..i].to_string(),
            None => self.relative_url.trim_end_matches('/').to_string(),
        };
        let tail = if page > 1 { format!("/page/{page}/") } else { "/".to_string() };
        format!("{}{}{}", self.origin, base, tail)
    }

    fn page_link(&self, key: &str) -> Option<String> {
        self.pagination
            .get(key)
            .and_then(Value::as_i64)
            .map(|page| self.paginated(page))
    }

    pub fn prev_url(&self) -> Option<String> {
        self.page_link("prev")
    }

    pub fn next_url(&self) -> Option<String> {
        self.page_link("next")
    }

    /// `twitter:card`: the large image is only honest when there is an image.
    fn twitter_card(&self) -> &'static str {
        if self.twitter_image().is_some() || self.cover_image().is_some() {
            "summary_large_image"
        } else {
            "summary"
        }
    }

    /// Ghost's `finaliseStructuredData` — the og/article/twitter block, in
    /// Ghost's own order, with every empty value dropped rather than emitted
    /// blank.
    ///
    /// Empty on a paged archive: Ghost does not describe page 2 of an archive
    /// to a social network, and tagging it with page 1's `og:url` is a
    /// duplicate-content signal.
    pub fn structured_data_tags(&self) -> Vec<String> {
        if !self.structured_data || self.has("paged") || self.context.is_empty() || self.unindexed() {
            return Vec::new();
        }
        let mut tags = Vec::new();
        let mut push = |property: &str, content: Option<String>| {
            if let Some(c) = content.filter(|c| !c.is_empty()) {
                tags.push(meta_tag(property, &c));
            }
        };
        push("og:site_name", self.site_str("title"));
        push("og:type", Some(self.og_type().to_string()));
        push("og:title", Some(self.og_title()));
        push("og:description", Some(self.og_description()));
        push("og:url", Some(self.canonical()));
        push("og:image", self.og_image().or_else(|| self.cover_image()));
        if self.is_entry() {
            push("article:published_time", self.published_date());
            push("article:modified_time", self.modified_date());
            for keyword in self.keywords() {
                push("article:tag", Some(keyword));
            }
            push(
                "article:publisher",
                self.site_str("facebook").and_then(|h| crate::social::url_for("facebook", &h)),
            );
            push("article:author", self.author_facebook());
        }
        push("twitter:card", Some(self.twitter_card().to_string()));
        push("twitter:title", Some(self.twitter_title()));
        push("twitter:description", Some(self.twitter_description()));
        push("twitter:url", Some(self.canonical()));
        push("twitter:image", self.twitter_image().or_else(|| self.cover_image()));
        let author = self.author_name();
        if author.is_some() {
            push("twitter:label1", Some("Written by".into()));
            push("twitter:data1", author);
        }
        let keywords = self.keywords();
        if !keywords.is_empty() {
            push("twitter:label2", Some("Filed under".into()));
            push("twitter:data2", Some(keywords.join(", ")));
        }
        push(
            "twitter:site",
            self.site_str("twitter").map(|h| format!("@{}", h.trim_start_matches('@'))),
        );
        push(
            "twitter:creator",
            self.creator_twitter().map(|h| format!("@{}", h.trim_start_matches('@'))),
        );
        tags
    }

    fn image_object(&self, url: Option<String>) -> Option<Value> {
        url.map(|url| json!({"@type": "ImageObject", "url": url}))
    }

    fn publisher(&self) -> Value {
        json!({
            "@type": "Organization",
            "name": self.site_str("title"),
            "url": self.origin,
            "logo": self.image_object(self.publisher_logo()),
        })
    }

    /// The `sameAs` array for a schema.org Person: the author's own site plus
    /// every social profile they have filled in.
    fn same_as(&self, person: &Value) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(website) = str_field(person, "website") {
            out.push(website);
        }
        for (platform, key, _) in crate::social::SOCIAL_PLATFORMS {
            // the table's first column is the theme-facing name (`x`); the
            // stored column is the second (`twitter`)
            let _ = platform;
            if let Some(handle) = str_field(person, key) {
                if let Some(url) = crate::social::url_for(key, &handle) {
                    out.push(url);
                }
            }
        }
        out
    }

    fn person(&self, p: &Value, url: Option<String>) -> Value {
        trim(json!({
            "@type": "Person",
            "name": str_field(p, "name"),
            "image": self.image_object(self.image(str_field(p, "profile_image"))),
            "url": url,
            "sameAs": non_empty_list(self.same_as(p)),
            "description": str_field(p, "meta_description"),
        }))
    }

    /// The schema.org JSON-LD object for this page, or `None` where Ghost
    /// emits none (a paged archive, a context it has no schema for, or
    /// structured data switched off).
    pub fn schema(&self) -> Option<Value> {
        if !self.structured_data || self.has("paged") || self.unindexed() {
            return None;
        }
        if self.is_entry() {
            let author = self.resource.get("primary_author").filter(|v| v.is_object());
            // Contributors are the authors after the first; a single-author
            // post has none.
            let contributors: Vec<Value> = self
                .resource
                .get("authors")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .skip(1)
                        .map(|p| {
                            let url = str_field(p, "url")
                                .map(|u| ghost_core::url::absolute(&self.origin, &u));
                            self.person(p, url)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let keywords = self.keywords();
            return Some(trim(json!({
                "@context": "https://schema.org",
                "@type": "Article",
                "publisher": self.publisher(),
                "author": author.map(|a| self.person(a, self.author_url())),
                "contributor": non_empty_list(contributors),
                "headline": self.meta_title,
                "url": self.url(),
                "datePublished": self.published_date(),
                "dateModified": self.modified_date(),
                "image": self.image_object(self.cover_image()),
                "keywords": (!keywords.is_empty()).then(|| keywords.join(", ")),
                // The entry's text, in the order Ghost's `getExcerpt` uses:
                // the custom excerpt, then the meta description, then the
                // automated 50-word excerpt.
                "description": self.own("custom_excerpt")
                    .or_else(|| self.own("meta_description"))
                    .or_else(|| self.own("excerpt")),
                "mainEntityOfPage": self.url(),
            })));
        }
        if self.has("home") {
            return Some(trim(json!({
                "@context": "https://schema.org",
                "@type": "WebSite",
                "publisher": self.publisher(),
                "url": self.url(),
                "name": self.site_str("title"),
                "image": self.image_object(self.cover_image()),
                "mainEntityOfPage": self.url(),
                "description": non_empty(&self.meta_description),
            })));
        }
        if self.has("tag") {
            return Some(trim(json!({
                "@context": "https://schema.org",
                "@type": "Series",
                "publisher": self.publisher(),
                "url": self.url(),
                "image": self.image_object(self.cover_image()),
                "name": self.own("name"),
                "mainEntityOfPage": self.url(),
                "description": non_empty(&self.meta_description),
            })));
        }
        if self.has("author") {
            let url = self.author_url();
            return Some(trim(json!({
                "@context": "https://schema.org",
                "@type": "Person",
                "sameAs": non_empty_list(self.same_as(&self.resource)),
                "name": self.own("name"),
                "url": url,
                "image": self.image_object(self.author_image().or_else(|| self.cover_image())),
                "mainEntityOfPage": self.author_url(),
                "description": non_empty(&self.meta_description),
            })));
        }
        None
    }

    /// `<link rel="alternate" hreflang="…">` for every language this page
    /// exists in, plus `x-default`.
    ///
    /// Read from `translations` on the render root, which the front end fills
    /// in from the entry's translation group — so the switcher a theme draws
    /// and the alternates a crawler reads can never disagree about what
    /// exists.
    ///
    /// This is the whole point of publishing translations as separate URLs:
    /// without it, two URLs carrying the same article in two languages look
    /// like duplicate content, and a search engine picks one and drops the
    /// other. With it, each is served to the audience that reads that
    /// language.
    ///
    /// `x-default` points at the default language's URL — the one to show
    /// somebody whose language this site does not publish in. Emitted only
    /// when the default language is actually among the alternates; naming a
    /// URL that does not exist is worse than omitting the hint.
    pub fn hreflang_tags(&self, translations: &Value, default_locale: &str) -> Vec<String> {
        let Some(items) = translations.as_array().filter(|a| a.len() > 1) else {
            return Vec::new();
        };
        let mut tags = Vec::new();
        for item in items {
            let (Some(code), Some(url)) = (
                item.get("locale").and_then(Value::as_str).filter(|c| !c.is_empty()),
                item.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()),
            ) else {
                continue;
            };
            let absolute = ghost_core::url::absolute(&self.origin, url);
            tags.push(format!(
                r#"<link rel="alternate" hreflang="{}" href="{}" />"#,
                esc(code),
                esc(&absolute)
            ));
            if code.eq_ignore_ascii_case(default_locale) {
                tags.push(format!(
                    r#"<link rel="alternate" hreflang="x-default" href="{}" />"#,
                    esc(&absolute)
                ));
            }
        }
        tags
    }

    /// The JSON-LD `<script>` block, ready to write into the head.
    pub fn schema_tag(&self) -> Option<String> {
        let schema = self.schema()?;
        let body = serde_json::to_string_pretty(&schema).ok()?;
        Some(format!(
            "<script type=\"application/ld+json\">\n{}\n    </script>",
            escape_json_ld(&body)
        ))
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.trim().is_empty()).then(|| s.to_string())
}

/// A list, or nothing where it is empty — `trim` then drops the key, which is
/// what Ghost does with an empty `sameAs` or `contributor`.
fn non_empty_list<T>(items: Vec<T>) -> Option<Vec<T>> {
    (!items.is_empty()).then_some(items)
}

/// Drop every null from an object, recursively — Ghost's `trimSchema`. A
/// `"image": null` in JSON-LD is not neutral: it is a declared-empty property.
fn trim(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let kept: Map<String, Value> = map
                .into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, trim(v)))
                .collect();
            Value::Object(kept)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(trim).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> Value {
        json!({
            "title": "My Site",
            "description": "Words about things",
            "url": "https://example.com",
            "logo": "/content/images/logo.png",
            "icon": "/content/images/icon.png",
            "cover_image": "/content/images/cover.jpg",
            "facebook": "mysite",
            "twitter": "@mysite",
        })
    }

    fn post() -> Value {
        json!({
            "id": "1",
            "title": "A Post",
            "slug": "a-post",
            "excerpt": "Fifty words of the body",
            "custom_excerpt": "What this post is about",
            "feature_image": "/content/images/feature.jpg",
            "published_at": "2026-01-02T03:04:05.000Z",
            "updated_at": "2026-02-03T04:05:06.000Z",
            "tags": [
                {"name": "Engineering", "visibility": "public"},
                {"name": "internal", "visibility": "internal"},
            ],
            "primary_author": {
                "name": "Ada Lovelace",
                "slug": "ada",
                "url": "/author/ada/",
                "profile_image": "/content/images/ada.jpg",
                "website": "https://ada.example",
                "twitter": "@ada",
                "facebook": "ada",
            },
            "authors": [
                {"name": "Ada Lovelace", "slug": "ada", "url": "/author/ada/"},
                {"name": "Grace Hopper", "slug": "grace", "url": "/author/grace/"},
            ],
        })
    }

    fn meta(context: &[&str], resource: Value, relative_url: &str) -> PageMeta {
        PageMeta {
            site: site(),
            resource,
            context: context.iter().map(|c| c.to_string()).collect(),
            origin: "https://example.com".into(),
            relative_url: relative_url.into(),
            pagination: Value::Null,
            meta_title: "A Post".into(),
            meta_description: "What this post is about".into(),
            structured_data: true,
        }
    }

    fn post_meta() -> PageMeta {
        meta(&["post"], post(), "/a-post/")
    }

    fn tags_of(meta: &PageMeta) -> String {
        meta.structured_data_tags().join("\n")
    }

    #[test]
    fn a_post_is_described_to_a_social_network_in_full() {
        // Every one of these was missing before this module existed, and each
        // is a field a crawler or a share preview reads.
        let out = tags_of(&post_meta());
        for expected in [
            r#"<meta property="og:site_name" content="My Site" />"#,
            r#"<meta property="og:type" content="article" />"#,
            r#"<meta property="og:title" content="A Post" />"#,
            r#"<meta property="og:description" content="What this post is about" />"#,
            r#"<meta property="og:url" content="https://example.com/a-post/" />"#,
            r#"<meta property="og:image" content="https://example.com/content/images/feature.jpg" />"#,
            r#"<meta property="article:published_time" content="2026-01-02T03:04:05.000Z" />"#,
            r#"<meta property="article:modified_time" content="2026-02-03T04:05:06.000Z" />"#,
            r#"<meta property="article:tag" content="Engineering" />"#,
            r#"<meta property="article:publisher" content="https://www.facebook.com/mysite" />"#,
            r#"<meta property="article:author" content="https://www.facebook.com/ada" />"#,
            r#"<meta name="twitter:card" content="summary_large_image" />"#,
            r#"<meta name="twitter:title" content="A Post" />"#,
            r#"<meta name="twitter:url" content="https://example.com/a-post/" />"#,
            r#"<meta name="twitter:label1" content="Written by" />"#,
            r#"<meta name="twitter:data1" content="Ada Lovelace" />"#,
            r#"<meta name="twitter:label2" content="Filed under" />"#,
            r#"<meta name="twitter:data2" content="Engineering" />"#,
            r#"<meta name="twitter:site" content="@mysite" />"#,
            r#"<meta name="twitter:creator" content="@ada" />"#,
        ] {
            assert!(out.contains(expected), "missing {expected}\n{out}");
        }
        // An internal tag is not published anywhere.
        assert!(!out.contains("internal"), "{out}");
    }

    #[test]
    fn a_paged_archive_is_not_described_at_all() {
        // Ghost checks `!_.includes(context, 'paged')` before emitting either
        // the social tags or the schema: page 2 of an archive is nobody's
        // share target, and giving it page 1's og:url is duplicate content.
        let mut m = meta(&["paged", "tag"], json!({"id": "t", "name": "Engineering"}), "/tag/engineering/page/2/");
        m.pagination = json!({"page": 2, "pages": 3, "prev": 1, "next": 3});
        assert!(m.structured_data_tags().is_empty());
        assert!(m.schema().is_none());
        // The pagination links are still emitted — those are what tell a
        // crawler the pages belong to one sequence.
        assert_eq!(m.prev_url().as_deref(), Some("https://example.com/tag/engineering/"));
        assert_eq!(m.next_url().as_deref(), Some("https://example.com/tag/engineering/page/3/"));
    }

    #[test]
    fn the_first_page_of_an_archive_links_only_forward() {
        let mut m = meta(&["tag"], json!({"id": "t", "name": "Engineering"}), "/tag/engineering/");
        m.pagination = json!({"page": 1, "pages": 2, "prev": null, "next": 2});
        assert_eq!(m.prev_url(), None);
        assert_eq!(m.next_url().as_deref(), Some("https://example.com/tag/engineering/page/2/"));
    }

    #[test]
    fn og_type_follows_the_context_the_way_ghost_reads_it() {
        assert_eq!(post_meta().og_type(), "article");
        // A static page is a website to Ghost, not an article.
        assert_eq!(meta(&["page"], post(), "/about/").og_type(), "website");
        assert_eq!(meta(&["author"], json!({"id": "a"}), "/author/ada/").og_type(), "profile");
        assert_eq!(meta(&["tag"], json!({"id": "t"}), "/tag/x/").og_type(), "website");
        assert_eq!(meta(&["home", "index"], Value::Null, "/").og_type(), "website");
    }

    #[test]
    fn a_resources_own_card_fields_outrank_the_sites() {
        let mut p = post();
        p["og_title"] = json!("Shared as this");
        p["twitter_description"] = json!("Tweeted as this");
        p["og_image"] = json!("/content/images/social.png");
        let m = meta(&["post"], p, "/a-post/");
        assert_eq!(m.og_title(), "Shared as this");
        assert_eq!(m.twitter_description(), "Tweeted as this");
        assert_eq!(m.og_image().as_deref(), Some("https://example.com/content/images/social.png"));
        // The two card chains are independent in Ghost: setting the og title
        // says nothing about the twitter one, which keeps the post's title.
        assert_eq!(m.twitter_title(), "A Post");
        // and the twitter image still falls through to the feature image
        assert_eq!(m.twitter_image().as_deref(), Some("https://example.com/content/images/feature.jpg"));
    }

    #[test]
    fn the_home_page_reads_the_sites_own_card_fields() {
        let mut m = meta(&["home", "index"], Value::Null, "/");
        m.meta_title = "My Site".into();
        m.meta_description = "Words about things".into();
        m.site["og_title"] = json!("Follow my site");
        assert_eq!(m.og_title(), "Follow my site");
        assert_eq!(m.og_description(), "Words about things");
        assert_eq!(m.og_image().as_deref(), Some("https://example.com/content/images/cover.jpg"));
    }

    #[test]
    fn a_canonical_override_moves_the_social_url_with_it() {
        // A reposted entry names the original as canonical; the share target
        // has to agree, or the two addresses compete.
        let mut p = post();
        p["canonical_url"] = json!("https://elsewhere.example/original/");
        let m = meta(&["post"], p, "/a-post/");
        assert_eq!(m.canonical(), "https://elsewhere.example/original/");
        assert!(tags_of(&m).contains(r#"<meta property="og:url" content="https://elsewhere.example/original/" />"#));
        // the schema still describes the copy being read
        assert_eq!(m.schema().unwrap()["url"], json!("https://example.com/a-post/"));
    }

    #[test]
    fn a_post_carries_an_article_schema_with_its_author_and_publisher() {
        let schema = post_meta().schema().expect("a post has a schema");
        assert_eq!(schema["@type"], json!("Article"));
        assert_eq!(schema["headline"], json!("A Post"));
        assert_eq!(schema["datePublished"], json!("2026-01-02T03:04:05.000Z"));
        assert_eq!(schema["keywords"], json!("Engineering"));
        assert_eq!(schema["description"], json!("What this post is about"));
        assert_eq!(schema["image"]["url"], json!("https://example.com/content/images/feature.jpg"));
        assert_eq!(schema["publisher"]["@type"], json!("Organization"));
        assert_eq!(schema["publisher"]["logo"]["url"], json!("https://example.com/content/images/logo.png"));
        assert_eq!(schema["author"]["name"], json!("Ada Lovelace"));
        assert_eq!(schema["author"]["url"], json!("https://example.com/author/ada/"));
        assert_eq!(
            schema["author"]["sameAs"],
            json!(["https://ada.example", "https://x.com/ada", "https://www.facebook.com/ada"])
        );
        // the second author is a contributor, not a co-author
        assert_eq!(schema["contributor"][0]["name"], json!("Grace Hopper"));
    }

    #[test]
    fn the_home_tag_and_author_pages_each_get_their_own_schema_type() {
        let home = meta(&["home", "index"], Value::Null, "/").schema().unwrap();
        assert_eq!(home["@type"], json!("WebSite"));
        assert_eq!(home["url"], json!("https://example.com/"));

        let tag = meta(&["tag"], json!({"id": "t", "name": "Engineering"}), "/tag/engineering/")
            .schema()
            .unwrap();
        assert_eq!(tag["@type"], json!("Series"));
        assert_eq!(tag["name"], json!("Engineering"));

        let author = meta(
            &["author"],
            json!({"id": "a", "name": "Ada Lovelace", "url": "/author/ada/", "twitter": "ada"}),
            "/author/ada/",
        )
        .schema()
        .unwrap();
        assert_eq!(author["@type"], json!("Person"));
        assert_eq!(author["url"], json!("https://example.com/author/ada/"));
        assert_eq!(author["sameAs"], json!(["https://x.com/ada"]));
    }

    #[test]
    fn nothing_null_survives_into_the_schema() {
        // A declared-empty property is worse than an absent one: it tells a
        // consumer the value is known to be nothing.
        let bare = meta(&["post"], json!({"id": "1", "title": "Bare"}), "/bare/");
        let schema = bare.schema().unwrap();
        let printed = serde_json::to_string(&schema).unwrap();
        assert!(!printed.contains("null"), "{printed}");
        assert!(schema.get("image").is_none(), "{printed}");
        assert!(schema.get("keywords").is_none(), "{printed}");
    }

    #[test]
    fn the_json_ld_block_cannot_break_out_of_its_script_tag() {
        // `</script` is the only thing an HTML parser looks for inside a
        // script element, and it never decodes entities — so the escape has
        // to be a JSON one, and `&` has to survive as `&`.
        let mut p = post();
        p["title"] = json!("Tom & Jerry </script><img src=x onerror=alert(1)>");
        p["custom_excerpt"] = json!("a < b");
        let mut m = meta(&["post"], p, "/tj/");
        m.meta_title = "Tom & Jerry </script><img src=x onerror=alert(1)>".into();
        let tag = m.schema_tag().unwrap();
        assert!(!tag.contains("</script><img"), "{tag}");
        assert!(tag.contains("\\u003c/script\\u003e"), "{tag}");
        // the ampersand is data, and is not HTML-escaped into the JSON
        assert!(tag.contains("Tom & Jerry"), "{tag}");
        assert!(!tag.contains("&amp;"), "{tag}");
    }

    #[test]
    fn switching_structured_data_off_removes_both_the_tags_and_the_schema() {
        let mut m = post_meta();
        m.structured_data = false;
        assert!(m.structured_data_tags().is_empty());
        assert!(m.schema().is_none());
        assert!(m.schema_tag().is_none());
    }

    #[test]
    fn a_hostile_title_is_escaped_in_the_meta_tags() {
        let mut m = post_meta();
        m.meta_title = r#"x" onload="alert(1)"#.into();
        let out = tags_of(&m);
        assert!(out.contains(r#"content="x&quot; onload=&quot;alert(1)""#), "{out}");
    }

    #[test]
    fn an_entry_with_no_image_anywhere_gets_the_small_card() {
        let mut m = meta(&["post"], json!({"id": "1", "title": "Bare"}), "/bare/");
        m.site = json!({"title": "My Site"});
        assert!(tags_of(&m).contains(r#"<meta name="twitter:card" content="summary" />"#));
    }
}
