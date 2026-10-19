//! Subdirectory-aware URL construction.
//!
//! Ghost-rust can be installed at the root of a domain (`https://site.com`) or
//! under a path on one (`https://site.com/blog`). The path component of
//! `config.url` is the *base path*, and it is the single source of truth for
//! both where the app mounts and how it writes URLs.
//!
//! # The storage invariant
//!
//! Internal URLs are stored in the database **without** the base path:
//! `/content/images/x.png`, `/my-post/`, `/about/`. Nothing persisted ever
//! carries `/blog`. The base path is added on the way out ([`base`]) and
//! stripped on the way in ([`strip`]).
//!
//! That keeps the database portable: moving a site from `site.com` to
//! `site.com/blog` (or back) is a config change, not a data migration. It is
//! the same split Ghost makes with its `__GHOST_URL__` placeholder, minus the
//! placeholder.
//!
//! # Applying the base path exactly once
//!
//! [`base`] is a deliberate, non-idempotent concatenation: it cannot detect
//! whether a path has already been prefixed, because a site at `/blog` may
//! legitimately hold a post at `/blog/` (making the correct output
//! `/blog/blog/`). Each URL must therefore pass through [`base`] exactly once,
//! at the boundary where it is written out. Callers that re-serialize an
//! already-public URL want [`strip`] first.
use std::sync::OnceLock;

/// The process-wide base path: `""` or `"/blog"` (never a trailing slash).
///
/// Set once during boot from `config.url`, before the server starts serving,
/// and read from render helpers and serializers that have no route to the
/// config. It is immutable for the process lifetime, which is what makes a
/// global sound here: the mount point cannot change without a restart.
static SUBDIR: OnceLock<String> = OnceLock::new();

/// First path segments the app serves itself. A base path may not collide with
/// one, or the mount would shadow the app's own routes.
const RESERVED: &[&str] = &[
    "ghost", "content", "assets", "public", "members", "rss", "feed", "p", "unsubscribe",
    "sitemap.xml", "favicon.ico", "robots.txt",
];

/// Normalize a raw base path into the stored form: `""`, or `/segment` with no
/// trailing slash.
///
/// Rejects anything that could not be mounted or could escape its own prefix.
pub fn normalize(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    // collapse repeated slashes and drop empty segments, so "//blog///" and
    // "blog" both land on "/blog"
    let segments: Vec<&str> = raw.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Ok(String::new());
    }
    for seg in &segments {
        if *seg == "." || *seg == ".." {
            return Err(format!("base path may not contain '{seg}': {raw}"));
        }
        if seg.contains(['?', '#', '\\', ' ']) {
            return Err(format!("base path segment '{seg}' has characters a URL path cannot carry"));
        }
    }
    if RESERVED.contains(&segments[0].to_ascii_lowercase().as_str()) {
        return Err(format!(
            "'/{}' is reserved by Ghost itself and cannot be used as a base path",
            segments[0]
        ));
    }
    Ok(format!("/{}", segments.join("/")))
}

/// Record the process-wide base path. Call once, during boot.
///
/// Returns the normalized value actually stored. A second call is ignored
/// (the first wins) so a stray caller cannot move the mount point underneath
/// a running server.
pub fn init(raw: &str) -> Result<&'static str, String> {
    let normalized = normalize(raw)?;
    let _ = SUBDIR.set(normalized);
    Ok(subdir())
}

/// The base path: `""` at the domain root, `"/blog"` under a subdirectory.
pub fn subdir() -> &'static str {
    SUBDIR.get().map(String::as_str).unwrap_or("")
}

/// True when this install is mounted under a subdirectory.
pub fn has_subdir() -> bool {
    !subdir().is_empty()
}

/// Prefix a stored, root-relative path with the base path, ready to send to a
/// browser. See the module docs: apply exactly once.
///
/// Paths that are not root-relative — absolute URLs, protocol-relative,
/// `mailto:`, `#anchor`, `data:` — are returned untouched, so this is safe to
/// run over user-supplied fields like navigation URLs and image settings.
pub fn base(path: &str) -> String {
    join(subdir(), path)
}

/// [`base`] against an explicit base path (the pure form; `base` is this
/// applied to the global).
pub fn join(subdir: &str, path: &str) -> String {
    if subdir.is_empty() || !is_root_relative(path) {
        return path.to_string();
    }
    format!("{subdir}{path}")
}

/// Whether `path` is a root-relative URL — the only shape [`base`] rewrites.
pub fn is_root_relative(path: &str) -> bool {
    // "//host/x" is protocol-relative, i.e. a different origin
    path.starts_with('/') && !path.starts_with("//")
}

/// Remove the base path from an incoming public path, yielding the stored
/// form. The inverse of [`base`].
///
/// A path that does not carry the prefix is returned unchanged, so this is
/// safe to run over input that may already be in stored form — which is what
/// makes it usable on save paths where the editor may submit either shape.
pub fn strip(path: &str) -> &str {
    unjoin(subdir(), path)
}

/// [`strip`] against an explicit base path.
pub fn unjoin<'a>(subdir: &str, path: &'a str) -> &'a str {
    if subdir.is_empty() {
        return path;
    }
    match path.strip_prefix(subdir) {
        // only a full *segment* match counts: "/blogging/" must not be read
        // as "/blog" + "ging/"
        Some("") => "/",
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    }
}

/// Path trees Ghost serves itself. A first segment from this list can never be
/// a base path (see `RESERVED`) or a post slug, which is what makes
/// [`base_asset`] able to tell a prefixed asset path from an unprefixed one.
const ASSET_ROOTS: &[&str] = &["content", "assets", "public"];

/// [`base`], but safe to apply to an asset path that may already carry the
/// base path.
///
/// The general [`base`] cannot detect double application, because `/blog/`
/// under a `/blog` install is an ambiguous post path. Asset paths carry no
/// such ambiguity: `/content`, `/assets` and `/public` are reserved, so
/// `/blog/content/…` is unambiguously already prefixed. That makes this the
/// right tool for `{{img_url}}`, which is handed image fields from serializers
/// (already prefixed) and literal paths from themes (not).
pub fn base_asset(path: &str) -> String {
    base_asset_with(subdir(), path)
}

/// [`base_asset`] against an explicit base path.
pub fn base_asset_with(subdir: &str, path: &str) -> String {
    if subdir.is_empty() || !is_root_relative(path) {
        return path.to_string();
    }
    let already = path
        .strip_prefix(subdir)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(|rest| {
            let seg = rest.split('/').next().unwrap_or("");
            ASSET_ROOTS.contains(&seg)
        })
        .unwrap_or(false);
    if already {
        path.to_string()
    } else {
        join(subdir, path)
    }
}

/// Build an absolute URL from a path that **already carries** the base path.
///
/// `site_url` may be given with or without the base path (`config.url` has it,
/// `Config::site_origin` does not); the base path is removed from the origin
/// before joining, so the result carries it exactly once. This is the one
/// correct way to turn a `{{url}}`-style value into an absolute URL:
/// concatenating `site_url` with a prefixed path yields `/blog/blog/…`.
///
/// A path that is already absolute is returned untouched.
pub fn absolute(site_url: &str, path: &str) -> String {
    absolute_with(subdir(), site_url, path)
}

/// [`absolute`] against an explicit base path.
pub fn absolute_with(subdir: &str, site_url: &str, path: &str) -> String {
    if !is_root_relative(path) {
        return path.to_string();
    }
    let origin = site_url.trim_end_matches('/');
    let origin = match subdir.is_empty() {
        true => origin,
        false => origin.strip_suffix(subdir).unwrap_or(origin),
    };
    format!("{origin}{path}")
}

// ---------------------------------------------------------------------------
// HTML rewriting
// ---------------------------------------------------------------------------

/// Attributes whose value is a single URL.
const URL_ATTRS: &[&str] = &["href", "src", "action", "poster", "formaction", "data-src"];
/// Attributes whose value is a comma-separated candidate list (`url 2x, …`).
const URL_SET_ATTRS: &[&str] = &["srcset", "data-srcset", "imagesrcset"];

/// Prefix every root-relative URL in an HTML fragment's attributes with the
/// base path.
///
/// Used for stored post HTML (which carries `<img src="/content/images/…">`
/// and hand-written internal links) and for the admin UI's own markup. Only
/// attribute values are touched, and only ones that are root-relative, so
/// escaped text — a post body quoting `href=&quot;/x&quot;`, a textarea
/// holding code injection — is left exactly as it was.
pub fn rewrite_html(html: &str) -> String {
    rewrite_html_with(subdir(), html)
}

/// The inverse of [`rewrite_html`]: strip the base path back out of an HTML
/// fragment, yielding the stored form.
///
/// Applied on save, so HTML that has been round-tripped through an API
/// response — which serves browser-ready URLs — cannot persist the base path
/// into the database and survive a change of install location.
pub fn unrewrite_html(html: &str) -> String {
    let sub = subdir();
    map_html_urls(sub, html, &|v| unjoin(sub, v).to_string())
}

/// Make every root-relative URL in an HTML fragment absolute against
/// `site_url`.
///
/// Email has no base to resolve a root-relative URL against, so newsletter
/// bodies have to carry full URLs or their images and links simply do not
/// work. Stored HTML is in site terms, so this applies the base path and the
/// origin in one pass.
pub fn absolutize_html(site_url: &str, html: &str) -> String {
    let sub = subdir();
    let site_url = site_url.to_string();
    // force the scan even at a domain root: there is still an origin to add
    map_html_urls("/", html, &|v| match is_root_relative(v) {
        true => absolute_with(sub, &site_url, &join(sub, v)),
        false => v.to_string(),
    })
}

/// [`rewrite_html`] against an explicit base path.
pub fn rewrite_html_with(subdir: &str, html: &str) -> String {
    map_html_urls(subdir, html, &|v| join(subdir, v))
}

/// [`unrewrite_html`] against an explicit base path.
pub fn unrewrite_html_with(subdir: &str, html: &str) -> String {
    map_html_urls(subdir, html, &|v| unjoin(subdir, v).to_string())
}

/// Strip the base path off Ghost's own asset URLs wherever they appear in a
/// text blob, regardless of how they are serialized.
///
/// The editor stores content as Lexical JSON (`{"src":"/content/images/…"}`)
/// and as markdown (`![](…)`), neither of which [`unrewrite_html`]'s
/// attribute scanner can see. Those URLs reach the browser base-path-prefixed
/// — an upload response has to be loadable in the editor — so they come back
/// prefixed on save, and this is what returns them to stored form.
///
/// Two things keep this textual replacement safe. The [`ASSET_ROOTS`]
/// prefixes are reserved, so `/blog/content/…` under a `/blog` install can
/// only be an asset URL; and a match only counts where a URL could actually
/// start, which is what stops `https://elsewhere.com/blog/content/x.png` —
/// another site that merely happens to share the layout — from being rewritten
/// into a path on this one.
pub fn unbase_assets(text: &str) -> String {
    unbase_assets_with(subdir(), text)
}

/// [`unbase_assets`] against an explicit base path.
pub fn unbase_assets_with(subdir: &str, text: &str) -> String {
    if subdir.is_empty() {
        return text.to_string();
    }
    // `/blog/content/…` → `/content/…`
    map_asset_urls(text, subdir, "")
}

/// The inverse of [`unbase_assets`]: put the base path onto Ghost's own asset
/// URLs wherever they appear in a text blob.
///
/// The editor is the case that needs this. Koenig reopens stored content —
/// Lexical JSON, markdown — and renders it live, so an image card has to be
/// handed a `src` it can actually load. Applying it to the blob rather than to
/// parsed attributes is what reaches inside the JSON.
pub fn base_assets(text: &str) -> String {
    base_assets_with(subdir(), text)
}

/// [`base_assets`] against an explicit base path.
pub fn base_assets_with(subdir: &str, text: &str) -> String {
    if subdir.is_empty() {
        return text.to_string();
    }
    // `/content/…` → `/blog/content/…`
    map_asset_urls(text, "", subdir)
}

/// Rewrite `{from}/{asset-root}/…` to `{to}/{asset-root}/…` throughout `text`.
///
/// Shared by [`unbase_assets`] and [`base_assets`], which are the same scan in
/// opposite directions. Two things keep a textual replacement safe here. The
/// [`ASSET_ROOTS`] segments are reserved, so `/blog/content/…` under a `/blog`
/// install can only be an asset URL; and a match only counts where a URL could
/// actually start, which stops `https://elsewhere.com/blog/content/x.png` —
/// another site that merely happens to share the layout — from being rewritten
/// into a path on this one.
fn map_asset_urls(text: &str, from: &str, to: &str) -> String {
    /// Characters a URL can legally follow in HTML, markdown or JSON.
    fn can_precede_url(c: Option<char>) -> bool {
        matches!(
            c,
            None | Some('"' | '\'' | '(' | '=' | ' ' | '\t' | '\n' | '\r' | ',' | '[' | '<')
        )
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        // the anchor is the asset root, which both directions share
        let Some(at) = ASSET_ROOTS
            .iter()
            .filter_map(|root| rest.find(&format!("{from}/{root}/")).map(|i| (i, *root)))
            .min_by_key(|(i, _)| *i)
        else {
            break;
        };
        let (at, root) = at;
        if can_precede_url(rest[..at].chars().next_back()) {
            out.push_str(&rest[..at]);
            out.push_str(&format!("{to}/{root}/"));
        } else {
            out.push_str(&rest[..at]);
            out.push_str(&format!("{from}/{root}/"));
        }
        rest = &rest[at + from.len() + root.len() + 2..];
    }
    out.push_str(rest);
    out
}

/// Walk `html`'s URL-bearing attributes and pass each value through `f`.
fn map_html_urls(subdir: &str, html: &str, f: &dyn Fn(&str) -> String) -> String {
    if subdir.is_empty() || html.is_empty() {
        return html.to_string();
    }
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() + 16);
    let mut i = 0usize;

    while i < bytes.len() {
        // an attribute name is preceded by whitespace inside a tag; scanning
        // for `=` and looking backwards is cheaper than tracking tag state
        let Some(eq) = html[i..].find('=').map(|p| i + p) else {
            out.push_str(&html[i..]);
            break;
        };

        // attribute name: walk back over the identifier before `=`
        let name_end = eq;
        let mut name_start = name_end;
        while name_start > i {
            let c = bytes[name_start - 1];
            if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':' {
                name_start -= 1;
            } else {
                break;
            }
        }
        let name = html[name_start..name_end].to_ascii_lowercase();

        // the value must be quoted and start right after `=`
        let quote = bytes.get(eq + 1).copied();
        let Some(quote) = quote.filter(|q| *q == b'"' || *q == b'\'') else {
            out.push_str(&html[i..eq + 1]);
            i = eq + 1;
            continue;
        };
        let value_start = eq + 2;
        let Some(value_end) = html[value_start..].find(quote as char).map(|p| value_start + p) else {
            out.push_str(&html[i..]);
            break;
        };
        let value = &html[value_start..value_end];

        out.push_str(&html[i..value_start]);
        if URL_ATTRS.contains(&name.as_str()) {
            out.push_str(&f(value));
        } else if URL_SET_ATTRS.contains(&name.as_str()) {
            out.push_str(&map_srcset(value, f));
        } else {
            out.push_str(value);
        }
        out.push(quote as char);
        i = value_end + 1;
    }
    out
}

/// `/a.png 1x, /b.png 2x` — map each candidate's URL, keep its descriptor.
fn map_srcset(value: &str, f: &dyn Fn(&str) -> String) -> String {
    value
        .split(',')
        .map(|candidate| {
            let trimmed = candidate.trim_start();
            let lead = &candidate[..candidate.len() - trimmed.len()];
            match trimmed.split_once(char::is_whitespace) {
                Some((url, rest)) => format!("{lead}{}{}{rest}", f(url), " "),
                None => format!("{lead}{}", f(trimmed)),
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_shapes_onto_one_form() {
        assert_eq!(normalize("").unwrap(), "");
        assert_eq!(normalize("/").unwrap(), "");
        assert_eq!(normalize("blog").unwrap(), "/blog");
        assert_eq!(normalize("/blog").unwrap(), "/blog");
        assert_eq!(normalize("/blog/").unwrap(), "/blog");
        assert_eq!(normalize("//blog///").unwrap(), "/blog");
        assert_eq!(normalize("/a/b/").unwrap(), "/a/b");
    }

    #[test]
    fn rejects_unmountable_base_paths() {
        assert!(normalize("/..").is_err());
        assert!(normalize("/a/../b").is_err());
        assert!(normalize("/ghost").is_err());
        assert!(normalize("/Ghost").is_err());
        assert!(normalize("/content").is_err());
        assert!(normalize("/a b").is_err());
        assert!(normalize("/a?b").is_err());
    }

    #[test]
    fn joins_only_root_relative_paths() {
        assert_eq!(join("/blog", "/about/"), "/blog/about/");
        assert_eq!(join("/blog", "/"), "/blog/");
        assert_eq!(join("", "/about/"), "/about/");
        // left alone: other origins and non-path targets
        assert_eq!(join("/blog", "https://x.com/a"), "https://x.com/a");
        assert_eq!(join("/blog", "//cdn.x.com/a"), "//cdn.x.com/a");
        assert_eq!(join("/blog", "mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(join("/blog", "#anchor"), "#anchor");
        assert_eq!(join("/blog", "about/"), "about/");
    }

    #[test]
    fn unjoin_inverts_join_on_segment_boundaries() {
        assert_eq!(unjoin("/blog", "/blog/about/"), "/about/");
        assert_eq!(unjoin("/blog", "/blog"), "/");
        assert_eq!(unjoin("/blog", "/blog/"), "/");
        assert_eq!(unjoin("", "/about/"), "/about/");
        // not a segment boundary — a sibling path that merely shares a prefix
        assert_eq!(unjoin("/blog", "/blogging/"), "/blogging/");
        // already stored form
        assert_eq!(unjoin("/blog", "/about/"), "/about/");
    }

    /// A post legitimately slugged "blog" under a base path of "/blog" is the
    /// case that rules out making `join` idempotent.
    #[test]
    fn join_does_not_second_guess_a_colliding_path() {
        assert_eq!(join("/blog", "/blog/"), "/blog/blog/");
    }

    #[test]
    fn base_asset_applies_once_whichever_form_it_is_given() {
        // unprefixed (stored form) gets the prefix
        assert_eq!(base_asset_with("/blog", "/content/images/a.png"), "/blog/content/images/a.png");
        assert_eq!(base_asset_with("/blog", "/assets/css/x.css"), "/blog/assets/css/x.css");
        // already prefixed is left alone rather than doubled
        assert_eq!(
            base_asset_with("/blog", "/blog/content/images/a.png"),
            "/blog/content/images/a.png"
        );
        assert_eq!(base_asset_with("/blog", "/blog/public/cards.min.js"), "/blog/public/cards.min.js");
        // a non-asset path is not an asset path, so it is prefixed plainly —
        // including the slug that collides with the base path
        assert_eq!(base_asset_with("/blog", "/blog/"), "/blog/blog/");
        // untouched shapes
        assert_eq!(base_asset_with("", "/content/images/a.png"), "/content/images/a.png");
        assert_eq!(base_asset_with("/blog", "https://x.com/a.png"), "https://x.com/a.png");
    }

    /// The trap this function exists to close: a prefixed path concatenated
    /// onto a site url that also carries the prefix gives `/blog/blog/…`.
    #[test]
    fn absolute_carries_the_base_path_exactly_once() {
        assert_eq!(
            absolute_with("/blog", "https://h.com/blog", "/blog/post/"),
            "https://h.com/blog/post/"
        );
        // site url given without the base path works the same
        assert_eq!(
            absolute_with("/blog", "https://h.com", "/blog/post/"),
            "https://h.com/blog/post/"
        );
        // trailing slash on the site url
        assert_eq!(
            absolute_with("/blog", "https://h.com/blog/", "/blog/post/"),
            "https://h.com/blog/post/"
        );
        // at the domain root
        assert_eq!(absolute_with("", "https://h.com", "/post/"), "https://h.com/post/");
        // already absolute
        assert_eq!(
            absolute_with("/blog", "https://h.com/blog", "https://other/x"),
            "https://other/x"
        );
    }

    #[test]
    fn rewrites_url_attributes_only() {
        let html = r#"<a href="/about/">x</a><img src="/content/images/a.png" alt="/not/a/url">"#;
        assert_eq!(
            rewrite_html_with("/blog", html),
            r#"<a href="/blog/about/">x</a><img src="/blog/content/images/a.png" alt="/not/a/url">"#
        );
    }

    #[test]
    fn leaves_external_and_escaped_urls_alone() {
        let html = r#"<a href="https://x.com/a">x</a><a href="//cdn/a">y</a><textarea>href=&quot;/x&quot;</textarea>"#;
        assert_eq!(rewrite_html_with("/blog", html), html);
    }

    #[test]
    fn rewrites_srcset_candidates() {
        let html = r#"<img srcset="/a.png 1x, /b.png 2x" src="/a.png">"#;
        assert_eq!(
            rewrite_html_with("/blog", html),
            r#"<img srcset="/blog/a.png 1x, /blog/b.png 2x" src="/blog/a.png">"#
        );
    }

    /// Email has no base for a root-relative URL, so a newsletter body has to
    /// come out with full URLs — at a domain root as much as under a base path.
    #[test]
    fn absolutize_html_makes_email_bodies_resolvable() {
        let stored = r#"<p><img src="/content/images/a.png"><a href="/about/">x</a><a href="https://x.com/">e</a></p>"#;
        // the global base path is empty in tests, which is the domain-root case
        assert_eq!(
            absolutize_html("https://h.com", stored),
            r#"<p><img src="https://h.com/content/images/a.png"><a href="https://h.com/about/">x</a><a href="https://x.com/">e</a></p>"#
        );
    }

    /// Asset URLs come back from the editor prefixed whatever the content
    /// format, so the save-path normalizer has to reach into Lexical JSON and
    /// markdown as well as HTML.
    #[test]
    fn unbase_assets_reaches_every_content_format() {
        assert_eq!(
            unbase_assets_with("/blog", r#"{"type":"image","src":"/blog/content/images/a.png"}"#),
            r#"{"type":"image","src":"/content/images/a.png"}"#
        );
        assert_eq!(
            unbase_assets_with("/blog", "![alt](/blog/content/images/a.png)"),
            "![alt](/content/images/a.png)"
        );
        assert_eq!(
            unbase_assets_with("/blog", r#"<img src="/blog/content/images/a.png">"#),
            r#"<img src="/content/images/a.png">"#
        );
        // already stored form is left alone
        assert_eq!(
            unbase_assets_with("/blog", "/content/images/a.png"),
            "/content/images/a.png"
        );
        // another site that merely shares the path layout is not ours to rewrite
        assert_eq!(
            unbase_assets_with("/blog", r#"<img src="https://x.com/blog/content/images/b.png">"#),
            r#"<img src="https://x.com/blog/content/images/b.png">"#
        );
        // a post slugged the same as the base path is not an asset path
        assert_eq!(unbase_assets_with("/blog", r#"<a href="/blog/blog/">x</a>"#), r#"<a href="/blog/blog/">x</a>"#);
        // prose mentioning the base path survives
        assert_eq!(unbase_assets_with("/blog", "see /blog/about/ for more"), "see /blog/about/ for more");
        assert_eq!(unbase_assets_with("", "/blog/content/images/a.png"), "/blog/content/images/a.png");
    }

    /// The editor needs the forward direction: stored content rendered live
    /// has to carry loadable `src` values.
    #[test]
    fn base_assets_is_the_inverse_of_unbase_assets() {
        let stored = r#"{"src":"/content/images/a.png","href":"/about/"} ![x](/content/images/b.png)"#;
        let public = base_assets_with("/blog", stored);
        assert_eq!(
            public,
            r#"{"src":"/blog/content/images/a.png","href":"/about/"} ![x](/blog/content/images/b.png)"#
        );
        assert_eq!(unbase_assets_with("/blog", &public), stored);
        // other origins are left alone in this direction too
        let ext = r#"<img src="https://x.com/content/images/b.png">"#;
        assert_eq!(base_assets_with("/blog", ext), ext);
        assert_eq!(base_assets_with("", stored), stored);
    }

    /// Content that has been through an API response and back must not carry
    /// the base path into storage.
    #[test]
    fn unrewrite_returns_html_to_stored_form() {
        let stored = r#"<p><a href="/about/"><img src="/content/images/a.png" srcset="/content/images/a.png 1x"></a><a href="https://x.com/">e</a></p>"#;
        let public = rewrite_html_with("/blog", stored);
        assert_eq!(unrewrite_html_with("/blog", &public), stored);
        // idempotent on content that is already stored form
        assert_eq!(unrewrite_html_with("/blog", stored), stored);
    }

    #[test]
    fn rewrite_is_a_no_op_at_the_domain_root() {
        let html = r#"<a href="/about/"><img srcset="/a.png 1x"></a>"#;
        assert_eq!(rewrite_html_with("", html), html);
    }

    /// Single-quoted values, unquoted values and stray `=` must not derail the
    /// scanner or lose content.
    #[test]
    fn survives_irregular_markup() {
        let html = r#"<a href='/a/' data-x=1 title="a = b">t</a>"#;
        assert_eq!(
            rewrite_html_with("/blog", html),
            r#"<a href='/blog/a/' data-x=1 title="a = b">t</a>"#
        );
    }

    #[test]
    fn preserves_multibyte_content() {
        let html = r#"<a href="/日本/">日本語 ✦</a>"#;
        assert_eq!(rewrite_html_with("/blog", html), r#"<a href="/blog/日本/">日本語 ✦</a>"#);
    }
}
