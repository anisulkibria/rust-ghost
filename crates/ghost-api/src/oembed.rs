//! `/ghost/api/admin/oembed/` — what Koenig's bookmark and embed cards call.
//!
//! Two jobs behind one endpoint, as in Ghost:
//!
//! * `?type=bookmark` — scrape the page's Open Graph/meta tags and answer
//!   with the card's metadata.
//! * no type (or `embed`) — follow the page's oEmbed discovery link and
//!   return the provider's payload, so YouTube, Vimeo, X and the rest render
//!   as real embeds. A page with no oEmbed endpoint degrades to a bookmark,
//!   which is also what Ghost does.
//!
//! Fetching is deliberately conservative: the admin is handing us a URL a
//! signed-in author typed, but the server still only follows http(s), caps
//! the response and gives up quickly.
//!
//! ## This endpoint is a server-side request forgery unless it is guarded
//!
//! Its whole job is to take a URL and have the server read it, and the answer
//! comes partly back to the caller — a bookmark payload carries the fetched
//! page's title, description and icon. The caller only has to be signed in,
//! and the lowest staff role is Contributor, who may write a draft and
//! nothing else.
//!
//! So unless the site has turned it off, every URL — the one the author typed
//! *and* the oEmbed endpoint the fetched page nominates, which is less
//! trustworthy than the first, not more — goes through
//! [`crate::mentions::fetchable`], and redirects are followed through
//! [`crate::mentions::guarded_redirect`] so that a public hostname answering
//! `302 Location: http://169.254.169.254/` does not walk around the check.
//! The switch is the Security module's `security_private_fetch` and it
//! defaults to blocking; see [`fetch_text`].
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

use crate::state::AppState;

/// Mounted (merged, not nested) under the admin API, because a nested router
/// cannot register the bare `/oembed/` the editor actually calls — Koenig
/// builds its URLs with a trailing slash, as Ghost's own API does.
pub fn oembed_router() -> Router<AppState> {
    Router::new().route("/oembed", get(oembed)).route("/oembed/", get(oembed))
}

const MAX_BYTES: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
// Some providers serve a stripped page to unknown agents; Ghost sends a
// browser-shaped UA for the same reason.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; Ghost/6.0; +https://ghost.org)";

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "errors": [{ "message": message, "errorType": "ValidationError" }] }))).into_response()
}

async fn oembed(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if crate::admin::current_user(&state, &headers).await.is_none() {
        return error(StatusCode::UNAUTHORIZED, "Authentication required");
    }
    let Some(url) = q.get("url").map(String::as_str).filter(|u| u.starts_with("http://") || u.starts_with("https://"))
    else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A valid http(s) URL is required");
    };
    let want_bookmark = q.get("type").map(String::as_str) == Some("bookmark");

    // Whether this endpoint may read the server's own network. See
    // [`fetch_text`] for what the two answers mean; the setting is the
    // Security module's `security_private_fetch` and defaults to blocking.
    let guarded = crate::security_settings::private_fetch_blocked(&state);
    if guarded && crate::mentions::fetchable(url).is_none() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "That URL is not reachable from this server. Private and loopback addresses are refused.",
        );
    }

    let Some(page) = fetch_text(url, guarded).await else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Could not fetch that URL");
    };

    if !want_bookmark {
        if let Some(endpoint) = discover_oembed(&page) {
            // The discovered endpoint is a URL *the fetched page chose*, so
            // it is less trustworthy than the one the author typed, not more.
            // It goes through the same guard.
            if guarded && crate::mentions::fetchable(&endpoint).is_none() {
                return Json(bookmark_payload(url, &page)).into_response();
            }
            if let Some(body) = fetch_text(&endpoint, guarded).await {
                if let Ok(mut payload) = serde_json::from_str::<Value>(&body) {
                    // Koenig keys the card off `type`; providers that omit it
                    // still render as a rich embed.
                    if payload.get("type").is_none() {
                        payload["type"] = json!("rich");
                    }
                    payload["url"] = json!(url);
                    return Json(payload).into_response();
                }
            }
        }
    }

    Json(bookmark_payload(url, &page)).into_response()
}

/// Read a page, optionally refusing anything inside this server's network.
///
/// ## Why this endpoint is a server-side request forgery if it is not guarded
///
/// It takes a URL and has the server read it, which is the whole feature. The
/// URL comes from a signed-in staff member — but the lowest staff role is
/// Contributor, who may write a draft and nothing else, and "nothing else"
/// should not include reading the server's cloud metadata service or the
/// admin interface of a database on localhost. Worse, part of the answer
/// comes back: a bookmark payload carries the fetched page's title,
/// description and icon, so this is not even blind.
///
/// With `guarded` on, the caller has already run the URL through
/// [`crate::mentions::fetchable`] — only http(s), no credentials in the URL,
/// no host that resolves inside this network — and the redirect policy
/// re-runs that same check on every hop, because a public hostname answering
/// `302 Location: http://169.254.169.254/` defeats a one-shot check entirely.
/// That policy is [`crate::mentions::guarded_redirect`], shared with the
/// webmention metadata fetchers for exactly this reason.
///
/// With it off the old behaviour is restored, redirects and all, which is a
/// choice a site behind its own private registry might legitimately want and
/// which the Security report names as a critical finding.
async fn fetch_text(url: &str, guarded: bool) -> Option<String> {
    let mut builder = reqwest::Client::builder().timeout(TIMEOUT).user_agent(USER_AGENT);
    if guarded {
        // Four hops: `http` → `https` → apex → `www` is three on its own, and
        // a chain longer than that is not a page anybody meant to bookmark.
        builder = builder.redirect(crate::mentions::guarded_redirect(4));
    }
    let client = builder.build().ok()?;
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    let capped = &bytes[..bytes.len().min(MAX_BYTES)];
    Some(String::from_utf8_lossy(capped).into_owned())
}

fn bookmark_payload(url: &str, page: &str) -> Value {
    let title = meta(page, "og:title").or_else(|| meta(page, "twitter:title")).or_else(|| title_tag(page));
    let description = meta(page, "og:description").or_else(|| meta(page, "twitter:description")).or_else(|| meta_name(page, "description"));
    json!({
        "version": "1.0",
        "type": "bookmark",
        "url": url,
        "metadata": {
            "url": meta(page, "og:url").unwrap_or_else(|| url.to_string()),
            "title": title.unwrap_or_else(|| url.to_string()),
            "description": description.unwrap_or_default(),
            "author": meta(page, "article:author").or_else(|| meta_name(page, "author")),
            "publisher": meta(page, "og:site_name"),
            "thumbnail": meta(page, "og:image").or_else(|| meta(page, "twitter:image")).map(|i| absolute(url, &i)),
            "icon": icon(page).map(|i| absolute(url, &i)),
        }
    })
}

// --- a very small amount of HTML scraping -----------------------------------
//
// A full parser would be a heavy dependency for reading six tags out of a
// `<head>`. These scan attribute-by-attribute instead of matching whole tags,
// so attribute order and single/double quotes do not matter.

/// The value of a `<meta>` tag's `content`, found by `property`/`name`.
fn meta_by(page: &str, key: &str, value: &str) -> Option<String> {
    for tag in tags(page, "meta") {
        if attr(&tag, key).as_deref() == Some(value) {
            if let Some(content) = attr(&tag, "content") {
                if !content.trim().is_empty() {
                    return Some(decode_entities(content.trim()));
                }
            }
        }
    }
    None
}

fn meta(page: &str, property: &str) -> Option<String> {
    // Twitter cards use name=, Open Graph uses property=, and plenty of
    // pages mix the two.
    meta_by(page, "property", property).or_else(|| meta_by(page, "name", property))
}

fn meta_name(page: &str, name: &str) -> Option<String> {
    meta_by(page, "name", name)
}

fn title_tag(page: &str) -> Option<String> {
    let lower = page.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = page[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    let text = decode_entities(page[open_end..end].trim());
    Some(text).filter(|t| !t.is_empty())
}

fn icon(page: &str) -> Option<String> {
    let mut fallback = None;
    for tag in tags(page, "link") {
        let rel = attr(&tag, "rel").unwrap_or_default().to_lowercase();
        if !rel.split_whitespace().any(|r| r == "icon" || r == "shortcut" || r == "apple-touch-icon") {
            continue;
        }
        let Some(href) = attr(&tag, "href") else { continue };
        // Prefer a dedicated touch icon; it is the one that is reliably big
        // enough for the bookmark card.
        if rel.contains("apple-touch-icon") {
            return Some(href);
        }
        fallback.get_or_insert(href);
    }
    fallback
}

fn discover_oembed(page: &str) -> Option<String> {
    for tag in tags(page, "link") {
        let kind = attr(&tag, "type").unwrap_or_default();
        if kind.contains("json+oembed") || kind.contains("oembed+json") {
            return attr(&tag, "href").map(|h| decode_entities(&h));
        }
    }
    None
}

/// Every `<name …>` tag body in the document.
fn tags(page: &str, name: &str) -> Vec<String> {
    let lower = page.to_lowercase();
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(&needle) {
        let start = from + found;
        let after = start + needle.len();
        // `<linked>` is not `<link>`: the name has to end at the tag.
        match page[after..].chars().next() {
            Some(c) if c.is_whitespace() || c == '>' || c == '/' => {}
            _ => {
                from = after;
                continue;
            }
        }
        let Some(end) = page[start..].find('>') else { break };
        out.push(page[start..start + end].to_string());
        from = start + end;
    }
    out
}

/// One attribute's value out of a tag body.
fn attr(tag: &str, key: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(key) {
        let start = from + found;
        from = start + key.len();
        // Must be a whole attribute name, not the tail of another one.
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let eq = lower[from..].find('=')? + from + 1;
        let value = tag[eq..].trim_start();
        let quote = value.chars().next()?;
        return if quote == '"' || quote == '\'' {
            value[1..].find(quote).map(|end| value[1..1 + end].to_string())
        } else {
            Some(value.split_whitespace().next()?.trim_end_matches('/').to_string())
        };
    }
    None
}

/// Resolve a page-relative URL against the page it came from.
fn absolute(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let origin = match base.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or(rest)),
        None => return url.to_string(),
    };
    if let Some(stripped) = url.strip_prefix("//") {
        let scheme = base.split("://").next().unwrap_or("https");
        return format!("{scheme}://{stripped}");
    }
    if url.starts_with('/') {
        return format!("{origin}{url}");
    }
    format!("{origin}/{url}")
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><head>
        <title>Fallback &amp; Title</title>
        <meta property="og:title" content="Real Title">
        <meta name="twitter:description" content="A description">
        <meta property="og:site_name" content="Example">
        <meta property="og:image" content="/img/cover.png">
        <link rel="apple-touch-icon" href="https://cdn.example.com/icon.png">
        <link rel="alternate" type="application/json+oembed" href="https://example.com/oembed?url=x">
        </head><body></body></html>"#;

    #[test]
    fn reads_open_graph_over_the_title_tag() {
        assert_eq!(meta(PAGE, "og:title").as_deref(), Some("Real Title"));
        assert_eq!(title_tag(PAGE).as_deref(), Some("Fallback & Title"));
    }

    #[test]
    fn falls_back_from_property_to_name() {
        assert_eq!(meta(PAGE, "twitter:description").as_deref(), Some("A description"));
    }

    #[test]
    fn finds_the_oembed_endpoint_and_icon() {
        assert_eq!(discover_oembed(PAGE).as_deref(), Some("https://example.com/oembed?url=x"));
        assert_eq!(icon(PAGE).as_deref(), Some("https://cdn.example.com/icon.png"));
    }

    #[test]
    fn bookmark_payload_resolves_relative_images() {
        let payload = bookmark_payload("https://example.com/post/one", PAGE);
        assert_eq!(payload["metadata"]["thumbnail"], json!("https://example.com/img/cover.png"));
        assert_eq!(payload["metadata"]["publisher"], json!("Example"));
        assert_eq!(payload["type"], json!("bookmark"));
    }

    #[test]
    fn tag_names_must_match_whole() {
        let page = r#"<linkedin href="no"><link rel="icon" href="yes.png">"#;
        assert_eq!(icon(page).as_deref(), Some("yes.png"));
    }

    #[test]
    fn absolute_handles_each_url_shape() {
        assert_eq!(absolute("https://e.com/a/b", "https://cdn.e.com/x.png"), "https://cdn.e.com/x.png");
        assert_eq!(absolute("https://e.com/a/b", "//cdn.e.com/x.png"), "https://cdn.e.com/x.png");
        assert_eq!(absolute("https://e.com/a/b", "/x.png"), "https://e.com/x.png");
        assert_eq!(absolute("https://e.com/a/b", "x.png"), "https://e.com/x.png");
    }
}
