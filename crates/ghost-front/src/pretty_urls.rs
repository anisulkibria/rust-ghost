//! One address per page.
//!
//! This is the port of Ghost's `prettyUrls` middleware stack
//! (`core/server/web/shared/middleware/pretty-urls.js`), which was missing —
//! and its absence was the largest single SEO defect on the site. Every page
//! answered **200 at two different URLs**:
//!
//! ```text
//! /why-we-rewrote-our-renderer-in-rust/   200, canonical = …-rust/
//! /why-we-rewrote-our-renderer-in-rust    200, canonical = …-rust
//! ```
//!
//! Each one declared *itself* canonical, because the canonical link is built
//! from the request path. So the two were not one page with an alias: they
//! were two pages with identical content, competing for the same ranking and
//! splitting whatever links they were each given. Adding `/About/`, `/ABOUT/`
//! and `/about/amp/` to the same page multiplies that further.
//!
//! Three redirects fix it, in Ghost's order, each a literal 301 so the
//! duplicate is retired rather than merely discouraged:
//!
//! 1. **Trailing slash.** `/x` → `/x/`.
//! 2. **AMP.** `/x/amp/` → `/x/`. AMP support was removed in Ghost 5, and
//!    sites that once had it still receive crawls and inbound links at those
//!    URLs; without this they are 404s where a live page used to be.
//! 3. **Lower case.** `/About/` → `/about/`. Slugs are generated lower-case,
//!    so an upper-case path is always somebody's transcription of one.
//!
//! ## What is deliberately left alone
//!
//! * **Anything whose last segment contains a dot** — `/robots.txt`,
//!   `/sitemap.xml`, `/favicon.ico`, `/assets/built/screen.css`,
//!   `/content/images/2026/11/Hero.JPG`, `/a-post.md`, the IndexNow key file.
//!   Ghost achieves this by mounting prettyUrls *after* its asset routes, so
//!   those requests never reach it; this router has one entry point, so the
//!   rule has to be explicit. It matters in both directions: a trailing slash
//!   would break the asset, and lower-casing would break a file whose name
//!   has a capital in it.
//! * **Anything that is not a GET or HEAD.** A 301 turns a POST into a GET
//!   and silently drops its body, so a form posted to a very slightly wrong
//!   URL would look like it worked and lose the response. Ghost redirects
//!   those too; answering 404 is the safer reading, and no POST target on
//!   this site is reached by a crawler.
//! * **The query string**, which is carried through untouched — it is not
//!   part of the path and lower-casing it would break tokens and UTM tags.
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// A year. The redirect is a fact about this site's URL shape, not about any
/// one page, so it is as cacheable as anything gets. Ghost caches it for
/// `caching:301:maxAge`, which is a year in its default config.
const CACHE: &str = "public, max-age=31536000";

/// Path prefixes this rule must not touch, because they are endpoints rather
/// than pages and their exact spelling is part of a protocol.
///
/// This is the list Ghost does not need, because it mounts `prettyUrls` after
/// the apps that own these paths; one router with one entry point has to say
/// so explicitly. Getting it wrong is not subtle — `/.well-known/webfinger`
/// redirected to `/.well-known/webfinger/` is a 404, and federation stops
/// working — so each entry names what would break.
const RESERVED_PREFIXES: &[&str] = &[
    // WebFinger and the ActivityPub actor, inbox, outbox and collections.
    // Spelled by the protocol, with no trailing slash, and asked for by other
    // servers rather than by people.
    "/.well-known/",
    "/.ghost/",
    // Newsletter click-through and open-pixel URLs, which are already out in
    // sent email and cannot be changed after the fact.
    "/r/",
    // The members API, Portal's session and checkout endpoints, and the
    // comment-count API. Called by scripts, which do not follow redirects
    // with the same forgiveness a browser does.
    "/members/",
    // Theme assets, Koenig card assets, uploaded images, media and files.
    // Mostly caught by the extension rule below, but a file with no extension
    // is still a file.
    "/assets/",
    "/public/",
    "/content/",
    // The form token and submit endpoints, and the form stylesheet.
    "/forms/api/",
    // Webmention receipt, which another site POSTs to and discovers from a
    // `rel=webmention` link that names this exact URL.
    "/webmentions/",
    // The admin, which this router does not serve anyway — listed so that a
    // mistaken fall-through 404s rather than bouncing.
    "/ghost/",
];

/// The canonical form of `path`, or `None` if it is already canonical.
///
/// Pure, and the whole of the rule: the middleware below only decides whether
/// to answer with what this returns.
pub(crate) fn canonical_path(path: &str) -> Option<String> {
    // Machinery, not a page.
    if RESERVED_PREFIXES.iter().any(|prefix| path.starts_with(prefix)) {
        return None;
    }
    // A dotfile-style path is a protocol's, not a slug's — `/.well-known` and
    // `/.ghost` without their trailing slash reach here.
    if path.starts_with("/.") {
        return None;
    }
    // A file, not a page. Checked on the last segment only, so a directory
    // with a dot in its name does not exempt what is under it.
    let last = path.rsplit('/').next().unwrap_or("");
    if last.contains('.') {
        return None;
    }

    let mut canonical = path.to_string();

    // AMP first, so `/x/amp` becomes `/x/` in one redirect rather than
    // `/x/amp/` and then `/x/`.
    let lower = canonical.to_ascii_lowercase();
    if lower.ends_with("/amp") || lower.ends_with("/amp/") {
        let cut = canonical.len() - if lower.ends_with("/amp/") { 4 } else { 3 };
        canonical.truncate(cut);
    }

    if !canonical.ends_with('/') {
        canonical.push('/');
    }

    // `/page/1/` is an alias for the collection index, and Ghost retires it
    // with a 301 (`handle-page-param.js`). Left alone it serves the home page
    // a second time, at 200, declaring itself canonical — the same duplicate
    // the trailing slash above exists to prevent, one level down. The same
    // goes for `/tag/x/page/1/`.
    if let Some(base) = canonical.strip_suffix("/page/1/") {
        canonical = format!("{base}/");
    }

    if canonical.chars().any(|c| c.is_ascii_uppercase()) {
        canonical = canonical.to_ascii_lowercase();
    }

    // A path that now begins `//` would be read as a protocol-relative URL —
    // i.e. another host — by a browser following the Location header. Ghost
    // guards the same case with `removeOpenRedirectFromUrl`.
    while canonical.starts_with("//") {
        canonical.remove(0);
    }

    (canonical != path).then_some(canonical)
}

/// Redirect to the canonical form of the URL, or carry on.
pub async fn canonicalize(request: Request, next: Next) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return next.run(request).await;
    }
    let uri = request.uri().clone();
    let Some(path) = canonical_path(uri.path()) else {
        return next.run(request).await;
    };
    let target = match uri.query() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    };
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, target), (header::CACHE_CONTROL, CACHE.to_string())],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_gets_exactly_one_address() {
        for (from, to) in [
            ("/a-post", "/a-post/"),
            ("/tag/engineering", "/tag/engineering/"),
            ("/author/ada", "/author/ada/"),
            // AMP, which Ghost 5 removed and crawlers still ask for
            ("/a-post/amp/", "/a-post/"),
            ("/a-post/amp", "/a-post/"),
            ("/a-post/AMP/", "/a-post/"),
            // case
            ("/About/", "/about/"),
            ("/TAG/Engineering/", "/tag/engineering/"),
            // and all three at once, in one redirect
            ("/About/AMP", "/about/"),
            // page 1 is the collection index under another name
            ("/page/1/", "/"),
            ("/page/1", "/"),
            ("/tag/engineering/page/1/", "/tag/engineering/"),
            ("/author/ada/page/1", "/author/ada/"),
        ] {
            assert_eq!(canonical_path(from).as_deref(), Some(to), "{from}");
        }
        // Page 2 onwards is a real page and stays where it is.
        assert_eq!(canonical_path("/page/2/"), None);
        assert_eq!(canonical_path("/tag/engineering/page/11/"), None);
        // `/page/10/` must not be mistaken for page 1 by a prefix match.
        assert_eq!(canonical_path("/page/10/"), None);
    }

    #[test]
    fn an_already_canonical_path_is_left_alone() {
        // No redirect at all, rather than a redirect to itself — that would
        // be an infinite loop for every page on the site.
        for path in ["/", "/a-post/", "/tag/engineering/", "/author/ada/", "/page/2/"] {
            assert_eq!(canonical_path(path), None, "{path}");
        }
    }

    #[test]
    fn a_file_is_never_touched() {
        // Both halves matter: a trailing slash breaks the asset, and
        // lower-casing breaks a filename with a capital in it. These are the
        // requests Ghost keeps away from this rule by mounting it late.
        for path in [
            "/robots.txt",
            "/sitemap.xml",
            "/sitemap-posts.xml",
            "/favicon.ico",
            "/llms.txt",
            "/assets/built/screen.css",
            "/content/images/2026/11/Hero.JPG",
            "/content/media/A-Video.mp4",
            "/a-post.md",
            "/A-Post.md",
            "/abcdef0123456789.txt",
        ] {
            assert_eq!(canonical_path(path), None, "{path}");
        }
        // A dot in a *directory* does not exempt what is inside it.
        assert_eq!(canonical_path("/v1.0/notes").as_deref(), Some("/v1.0/notes/"));
    }

    #[test]
    fn a_redirect_target_can_never_name_another_host() {
        // `//evil.example/x` in a Location header is a protocol-relative URL:
        // a browser following it leaves this site entirely.
        assert_eq!(canonical_path("//evil.example/x").as_deref(), Some("/evil.example/x/"));
        assert_eq!(canonical_path("///evil/x").as_deref(), Some("/evil/x/"));
        // `//host.tld` on its own never even gets that far: the dot in the
        // last segment makes it a file to this rule, so there is no redirect
        // to build. Asserted so that the safety does not silently depend on
        // which of the two rules fires first.
        assert_eq!(canonical_path("///evil.example"), None);
        // And whatever the input, the result can never begin with two slashes.
        for hostile in ["//x", "////x", "//a/b"] {
            if let Some(target) = canonical_path(hostile) {
                assert!(!target.starts_with("//"), "{hostile} -> {target}");
            }
        }
    }

    /// The test this rule most needs. Every one of these is an endpoint whose
    /// spelling belongs to a protocol or is already out in the world in sent
    /// email, and a 301 to a slashed variant is a 404 — federation stops
    /// working, newsletter links stop resolving, Portal stops signing people
    /// in. Ghost keeps them away from this rule by mounting it late; here the
    /// list is the only thing standing between it and them.
    #[test]
    fn the_machinery_is_never_redirected() {
        for path in [
            "/.well-known/webfinger",
            "/.well-known/jwks.json",
            "/.ghost/activitypub/actor",
            "/.ghost/activitypub/outbox",
            "/.ghost/activitypub/notes/abc",
            "/r/abcdef",
            "/r/o/abcdef",
            "/members/api/member",
            "/members/api/comments/counts",
            "/members/webhooks/mailgun",
            "/forms/api/abc/token",
            "/webmentions/receive",
            "/assets/built/main",
            "/public/cards",
            "/content/images/x",
            "/ghost/api/content/posts",
            // and the two dotfile roots without their slash
            "/.well-known",
            "/.ghost",
        ] {
            assert_eq!(canonical_path(path), None, "{path} must not be redirected");
        }
    }

    #[test]
    fn amp_is_only_stripped_as_a_whole_segment() {
        // `/lamp/` and `/amplifier/` are ordinary slugs.
        assert_eq!(canonical_path("/lamp/"), None);
        assert_eq!(canonical_path("/amplifier/"), None);
        assert_eq!(canonical_path("/amp-hour/"), None);
        // and the bare `/amp/` at the site root resolves to the home page
        assert_eq!(canonical_path("/amp/").as_deref(), Some("/"));
    }
}
