//! Webmentions — the open protocol for "someone linked to you".
//!
//! Three halves, despite the arithmetic:
//!
//! * **Receiving.** `POST /webmentions/receive` takes a `source` and a
//!   `target`, checks the target is a page on this site, fetches the source
//!   and confirms the link really is there, and keeps what it found.
//! * **Sending.** When a post is published, every external link in it is a
//!   site that may want to know. Each one's webmention endpoint is discovered
//!   from its `Link` header or its markup, and told.
//! * **Incoming recommendations.** Ghost does not store these separately: a
//!   recommendation is a mention whose source is another Ghost site's
//!   `/.well-known/recommendations.json`. This serves that file too, so other
//!   sites can discover what this one recommends.
//!
//! Verification is the whole of the security model. Anybody can post a
//! webmention claiming any source, so nothing is stored until the source page
//! has been fetched and found to contain a link to the target. Fetching an
//! attacker-supplied URL is itself the risk, so [`fetchable`] refuses
//! anything that is not plain HTTP(S) to a public host.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Mounted at the site root, because the protocol requires an endpoint a
/// stranger can POST to.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/webmentions/receive", post(receive))
        .route("/webmentions/receive/", post(receive))
        .route("/.well-known/recommendations.json", get(recommendations_json))
}

pub fn admin_router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            .route(&format!("/mentions{suffix}"), get(browse))
            .route(&format!("/incoming_recommendations{suffix}"), get(incoming_recommendations));
    }
    router
}

#[derive(Deserialize)]
struct Received {
    source: String,
    target: String,
}

/// How much of a source page is read before giving up. A webmention is a
/// claim from a stranger, and the fetch is the part they control.
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a URL is safe to fetch on a stranger's instruction.
///
/// Only HTTP and HTTPS, and no host that resolves inside the network this
/// server is on: a webmention naming `http://169.254.169.254/` or
/// `http://localhost:5432/` is an attempt to have the server read something
/// on its behalf, not a link to a blog post.
pub fn fetchable(url: &str) -> Option<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".internal") {
        return None;
    }
    // A literal address is checked directly; a name is left to the resolver,
    // which is why the fetch below also refuses a redirect. An IPv6 literal
    // arrives bracketed, and the brackets are not part of the address.
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        let private = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_broadcast() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                // Unique-local (fc00::/7) and link-local (fe80::/10) are the
                // IPv6 equivalents of the ranges refused above; a v4-mapped
                // address is checked as the v4 address it carries.
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
                    || v6.to_ipv4_mapped().is_some_and(|v4| {
                        v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
                    })
            }
        };
        if private {
            return None;
        }
    }
    Some(parsed)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // A redirect is how a name that passed the host check becomes an
        // address that would not have.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Ghost/{} Webmention", ghost_core::GHOST_VERSION))
        .build()
        .unwrap_or_default()
}

/// Read at most `MAX_SOURCE_BYTES` of a response body.
///
/// Chunk at a time rather than all at once: the body is served by whoever
/// the sender named, and "read it then truncate" would already have read it.
async fn read_capped(mut response: reqwest::Response) -> String {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_SOURCE_BYTES {
            body.truncate(MAX_SOURCE_BYTES);
            break;
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

/// `POST /webmentions/receive`.
///
/// Answers 202 as the specification asks — the mention is accepted for
/// processing, and the verification happens after the sender has gone.
async fn receive(State(state): State<AppState>, Form(body): Form<Received>) -> Response {
    let (Some(source), Some(target)) = (fetchable(&body.source), fetchable(&body.target)) else {
        return error(StatusCode::BAD_REQUEST, "source and target must both be HTTP(S) URLs");
    };
    if source == target {
        return error(StatusCode::BAD_REQUEST, "source and target must differ");
    }
    // The target has to be a page here. Otherwise this endpoint is an open
    // relay for having the server fetch arbitrary pages.
    let site = state.site_url();
    let Some(site_host) = fetchable(&site).and_then(|u| u.host_str().map(str::to_ascii_lowercase)) else {
        return error(StatusCode::BAD_REQUEST, "this site has no public URL configured");
    };
    if target.host_str().map(str::to_ascii_lowercase) != Some(site_host) {
        return error(StatusCode::BAD_REQUEST, "target is not a page on this site");
    }

    let background = state.clone();
    let (source, target) = (source.to_string(), target.to_string());
    tokio::spawn(async move {
        if let Err(e) = verify_and_store(&background, &source, &target).await {
            tracing::info!("webmention from {source} to {target} was not accepted: {}", e.message());
        }
    });
    (StatusCode::ACCEPTED, "Webmention accepted").into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]}))).into_response()
}

/// Fetch the source, confirm it links to the target, and keep what it says.
pub async fn verify_and_store(state: &AppState, source: &str, target: &str) -> Result<()> {
    let Some(url) = fetchable(source) else {
        return Err(GhostError::BadRequest("source is not fetchable".into()));
    };
    let response = client()
        .get(url)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("source could not be fetched: {e}")))?;
    if !response.status().is_success() {
        // A source that has gone is a mention that has been withdrawn.
        if response.status() == StatusCode::GONE || response.status() == StatusCode::NOT_FOUND {
            let _ = sqlx::query("UPDATE mentions SET deleted=1 WHERE source=? AND target=?")
                .bind(source)
                .bind(target)
                .execute(&state.db)
                .await;
        }
        return Err(GhostError::BadRequest(format!("source returned {}", response.status())));
    }
    let html = read_capped(response).await;
    store_verified(state, source, target, &html).await
}

/// Keep what a fetched source page says, once it has been shown to link here.
///
/// Split from the fetch so that the network guard above can stay strict: a
/// test needs a source it can serve, and every address it could serve one
/// from is an address [`fetchable`] is right to refuse.
pub async fn store_verified(state: &AppState, source: &str, target: &str, html: &str) -> Result<()> {
    if !links_to(html, target) {
        let _ = sqlx::query("UPDATE mentions SET deleted=1 WHERE source=? AND target=?")
            .bind(source)
            .bind(target)
            .execute(&state.db)
            .await;
        return Err(GhostError::BadRequest("source does not link to target".into()));
    }

    let metadata = metadata_of(html);
    let (resource_type, resource_id) = resolve_target(state, target).await;
    let now = dates::now_db();
    // A repeat webmention updates the one already stored rather than adding
    // a second: the source is telling this site the same thing again.
    sqlx::query(
        "INSERT INTO mentions (id, source, source_title, source_site_title, source_excerpt, \
         source_author, source_featured_image, source_favicon, target, resource_id, resource_type, \
         payload, deleted, verified, revalidation_failure_count, created_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,0,1,0,?) \
         ON CONFLICT(source, target) DO UPDATE SET \
           source_title=excluded.source_title, source_site_title=excluded.source_site_title, \
           source_excerpt=excluded.source_excerpt, source_author=excluded.source_author, \
           source_featured_image=excluded.source_featured_image, source_favicon=excluded.source_favicon, \
           resource_id=excluded.resource_id, resource_type=excluded.resource_type, \
           deleted=0, verified=1, revalidation_failure_count=0",
    )
    .bind(ids::new_id())
    .bind(source)
    .bind(&metadata.title)
    .bind(&metadata.site_title)
    .bind(&metadata.excerpt)
    .bind(&metadata.author)
    .bind(&metadata.image)
    .bind(&metadata.favicon)
    .bind(target)
    .bind(&resource_id)
    .bind(&resource_type)
    .bind(Value::Null.to_string())
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| GhostError::Db(e.to_string()))?;

    // Staff who asked to hear about mentions hear about this one. A
    // recommendation is told apart from an ordinary link, because the two
    // read very differently in an inbox.
    let recommendation = is_recommendation(source);
    notify_staff(state, source, target, &metadata, recommendation).await;
    Ok(())
}

/// Whether a page links to a URL.
///
/// Compares the resolved URL rather than the raw text, so that a trailing
/// slash, a different case of host, or a `?utm_source` on the end does not
/// hide a link that is really there.
fn links_to(html: &str, target: &str) -> bool {
    let Some(target) = reqwest::Url::parse(target).ok() else { return false };
    let wanted = (target.host_str().map(str::to_ascii_lowercase), target.path().trim_end_matches('/').to_string());
    for href in hrefs(html) {
        if let Ok(link) = reqwest::Url::parse(&href) {
            if (link.host_str().map(str::to_ascii_lowercase), link.path().trim_end_matches('/').to_string()) == wanted {
                return true;
            }
        }
    }
    false
}

/// Every `href` and `src` value in a document, as written.
fn hrefs(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for attribute in ["href=", "src="] {
        let mut rest = html;
        while let Some(at) = rest.to_ascii_lowercase().find(attribute) {
            rest = &rest[at + attribute.len()..];
            let quote = rest.chars().next();
            let value = match quote {
                Some('"') | Some('\'') => {
                    let quote = quote.unwrap();
                    match rest[1..].find(quote) {
                        Some(end) => &rest[1..1 + end],
                        None => break,
                    }
                }
                _ => rest.split([' ', '>', '\n', '\r', '\t']).next().unwrap_or(""),
            };
            if !value.is_empty() {
                found.push(value.trim().to_string());
            }
        }
    }
    found
}

/// What a source page says about itself.
#[derive(Debug, Default)]
pub struct SourceMetadata {
    pub title: Option<String>,
    pub site_title: Option<String>,
    pub excerpt: Option<String>,
    pub author: Option<String>,
    pub image: Option<String>,
    pub favicon: Option<String>,
}

/// Read a page's own description of itself from its `meta` tags.
///
/// Open Graph first, then the plain equivalents, then the `<title>`. No HTML
/// parser: these are five attributes read off a document that is not trusted,
/// and every value is escaped before it is ever rendered.
pub fn metadata_of(html: &str) -> SourceMetadata {
    let meta = |property: &str| -> Option<String> {
        let lower = html.to_ascii_lowercase();
        let mut from = 0;
        while let Some(at) = lower[from..].find("<meta") {
            let start = from + at;
            let end = lower[start..].find('>').map(|e| start + e + 1)?;
            let tag = &html[start..end];
            let tag_lower = &lower[start..end];
            let names = [format!("\"{property}\""), format!("'{property}'")];
            if names.iter().any(|name| tag_lower.contains(name.as_str())) {
                if let Some(content) = attribute(tag, "content") {
                    if !content.trim().is_empty() {
                        return Some(content);
                    }
                }
            }
            from = end;
        }
        None
    };
    let title = meta("og:title").or_else(|| meta("twitter:title")).or_else(|| {
        let lower = html.to_ascii_lowercase();
        let start = lower.find("<title")?;
        let open = lower[start..].find('>')? + start + 1;
        let close = lower[open..].find("</title>")? + open;
        Some(decode_entities(html[open..close].trim()))
    });
    SourceMetadata {
        title: title.map(|t| truncate(&t, 300)),
        site_title: meta("og:site_name").map(|t| truncate(&t, 300)),
        excerpt: meta("og:description")
            .or_else(|| meta("description"))
            .or_else(|| meta("twitter:description"))
            .map(|t| truncate(&t, 1000)),
        author: meta("author").or_else(|| meta("article:author")).map(|t| truncate(&t, 300)),
        image: meta("og:image").or_else(|| meta("twitter:image")).map(|t| truncate(&t, 1000)),
        favicon: None,
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))?;
    let rest = &tag[at + name.len() + 1..];
    let value = match rest.chars().next()? {
        quote @ ('"' | '\'') => &rest[1..1 + rest[1..].find(quote)?],
        _ => rest.split([' ', '>']).next()?,
    };
    Some(decode_entities(value))
}

/// The five entities an attribute can carry. Enough for a title; this is not
/// a general-purpose decoder, and the result is escaped again on output.
fn decode_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect::<String>().trim().to_string()
}

/// Which post or page a mention's target is, if any.
async fn resolve_target(state: &AppState, target: &str) -> (Option<String>, Option<String>) {
    let Ok(url) = reqwest::Url::parse(target) else { return (None, None) };
    let router = state.router.read().await.clone();
    use ghost_routing::tree::RouteMatch;
    let (kind, slug) = match router.match_path(url.path()) {
        Some(RouteMatch::CollectionEntry { slug, .. }) => ("post", slug),
        Some(RouteMatch::StaticPage { slug }) => ("page", slug),
        _ => return (None, None),
    };
    let id: Option<String> =
        sqlx::query_scalar(&format!("SELECT id FROM posts WHERE slug=? AND type='{kind}'"))
            .bind(&slug)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    (Some(kind.to_string()), id)
}

/// A mention whose source is another Ghost site's recommendations file is
/// that site recommending this one.
pub fn is_recommendation(source: &str) -> bool {
    source.trim_end_matches('/').ends_with("/.well-known/recommendations.json")
}

/// Tell the staff who asked to be told.
async fn notify_staff(
    state: &AppState,
    source: &str,
    target: &str,
    metadata: &SourceMetadata,
    recommendation: bool,
) {
    let column = if recommendation { "recommendation_notifications" } else { "mention_notifications" };
    let recipients: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT email FROM users WHERE status='active' AND {column}=1"
    ))
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    if recipients.is_empty() {
        return;
    }
    let site = state.settings.get_str("title").unwrap_or_else(|| "your site".into());
    let who = metadata
        .site_title
        .clone()
        .or_else(|| metadata.title.clone())
        .unwrap_or_else(|| source.to_string());
    let display_source = if recommendation {
        source.trim_end_matches('/').trim_end_matches("/.well-known/recommendations.json").to_string()
    } else {
        source.to_string()
    };
    let (subject, line) = if recommendation {
        (format!("{who} recommended {site}"), format!("{who} now recommends {site} to their readers."))
    } else {
        (format!("{who} mentioned {site}"), format!("{who} linked to {site}."))
    };
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &site, &state.site_url());
    for email in recipients {
        let message = ghost_mail::Message {
            to: email,
            subject: subject.clone(),
            html: format!(
                r#"<p>{line}</p><p><a href="{src}">{src_text}</a> &rarr; <a href="{target}">{target_text}</a></p>"#,
                line = html_escape(&line),
                src = html_escape(&display_source),
                src_text = html_escape(&display_source),
                target = html_escape(target),
                target_text = html_escape(target),
            ),
            text: format!("{line}\n\n{display_source} -> {target}\n"),
        };
        if let Err(e) = ghost_mail::send(&cfg, &message).await {
            tracing::warn!("webmention notification could not be sent: {}", e.message());
        }
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// sending
// ---------------------------------------------------------------------------

/// Tell every site a published post links to.
///
/// Spawned rather than awaited: this is a fetch and a POST per outbound link,
/// against hosts that may be slow or gone, and a publish must not wait on
/// any of them.
pub fn send_for_post(state: &AppState, post_url: String, html: String) {
    if !state.settings.get_bool("webmentions_enabled") {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let own_host = fetchable(&state.site_url()).and_then(|u| u.host_str().map(str::to_ascii_lowercase));
        let mut seen = std::collections::HashSet::new();
        for href in hrefs(&html) {
            let Some(target) = fetchable(&href) else { continue };
            // Links back into this site are navigation, not a mention.
            if target.host_str().map(str::to_ascii_lowercase) == own_host {
                continue;
            }
            let target = target.to_string();
            if !seen.insert(target.clone()) {
                continue;
            }
            if let Err(e) = send_one(&state, &post_url, &target).await {
                tracing::debug!("webmention to {target} was not delivered: {}", e.message());
            }
        }
    });
}

/// Discover a target's endpoint and tell it, recording the attempt.
async fn send_one(state: &AppState, source: &str, target: &str) -> Result<()> {
    // Already told, and told successfully: a republish should not re-notify
    // every site the post links to.
    let previous: Option<String> =
        sqlx::query_scalar("SELECT status FROM mention_sends WHERE source=? AND target=?")
            .bind(source)
            .bind(target)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if previous.as_deref() == Some("sent") {
        return Ok(());
    }

    let endpoint = discover_endpoint(target).await;
    let result = match &endpoint {
        None => Err(GhostError::BadRequest("no webmention endpoint".into())),
        Some(endpoint) => client()
            .post(endpoint.clone())
            .form(&[("source", source), ("target", target)])
            .send()
            .await
            .map_err(|e| GhostError::BadRequest(e.to_string()))
            .and_then(|response| {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(GhostError::BadRequest(format!("endpoint returned {}", response.status())))
                }
            }),
    };
    let now = dates::now_db();
    let _ = sqlx::query(
        "INSERT INTO mention_sends (id, source, target, endpoint, status, error, created_at, updated_at) \
         VALUES (?,?,?,?,?,?,?,?) \
         ON CONFLICT(source, target) DO UPDATE SET endpoint=excluded.endpoint, status=excluded.status, \
           error=excluded.error, updated_at=excluded.updated_at",
    )
    .bind(ids::new_id())
    .bind(source)
    .bind(target)
    .bind(endpoint.as_ref().map(reqwest::Url::to_string))
    .bind(if result.is_ok() { "sent" } else { "failed" })
    .bind(result.as_ref().err().map(|e| e.message()))
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    result
}

/// A target's webmention endpoint, from its `Link` header or its markup.
///
/// The specification's order: the first `Link` header with `rel="webmention"`,
/// then the first `<link>` or `<a>` in the document with that rel.
pub async fn discover_endpoint(target: &str) -> Option<reqwest::Url> {
    let url = fetchable(target)?;
    let response = client().get(url.clone()).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    for value in response.headers().get_all(axum::http::header::LINK).iter() {
        let Ok(value) = value.to_str() else { continue };
        if let Some(href) = webmention_link_header(value) {
            return url.join(&href).ok().filter(|joined| fetchable(joined.as_str()).is_some());
        }
    }
    let html = read_capped(response).await;
    let href = webmention_rel(&html)?;
    url.join(&href).ok().filter(|joined| fetchable(joined.as_str()).is_some())
}

/// The href of the first `rel="webmention"` entry in a `Link` header.
pub fn webmention_link_header(value: &str) -> Option<String> {
    for part in value.split(',') {
        let lower = part.to_ascii_lowercase();
        // `rel=webmention`, quoted or not, and possibly among other rels.
        let has_rel = lower.split(';').any(|param| {
            let param = param.trim();
            let Some(rels) = param.strip_prefix("rel=") else { return false };
            rels.trim_matches(|c| c == '"' || c == '\'').split_whitespace().any(|rel| rel == "webmention")
        });
        if !has_rel {
            continue;
        }
        let start = part.find('<')?;
        let end = part[start..].find('>')? + start;
        return Some(part[start + 1..end].trim().to_string());
    }
    None
}

/// The href of the first `link` or `a` element with `rel="webmention"`.
pub fn webmention_rel(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find('<') {
        let start = from + at;
        let Some(end) = lower[start..].find('>').map(|e| start + e + 1) else { break };
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        let is_candidate = tag_lower.starts_with("<link") || tag_lower.starts_with("<a ") || tag_lower.starts_with("<a\n");
        if is_candidate {
            let rel_is_webmention = attribute(tag, "rel")
                .is_some_and(|rel| rel.split_whitespace().any(|r| r.eq_ignore_ascii_case("webmention")));
            if rel_is_webmention {
                // An empty href means the endpoint is the page itself, which
                // the specification allows.
                return Some(attribute(tag, "href").unwrap_or_default());
            }
        }
        from = end;
    }
    None
}

// ---------------------------------------------------------------------------
// reading
// ---------------------------------------------------------------------------

/// `GET /mentions` — who has linked here.
async fn browse(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, MentionRow>(
        "SELECT m.id, m.source, m.source_title, m.source_site_title, m.source_excerpt, m.source_author, \
                m.source_featured_image, m.source_favicon, m.target, m.resource_id, m.resource_type, \
                m.verified, m.created_at, p.title AS resource_title \
         FROM mentions m LEFT JOIN posts p ON p.id = m.resource_id \
         WHERE m.deleted = 0 ORDER BY m.created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    Json(json!({"mentions": rows})).into_response()
}

#[derive(serde::Serialize, sqlx::FromRow)]
struct MentionRow {
    id: String,
    source: String,
    source_title: Option<String>,
    source_site_title: Option<String>,
    source_excerpt: Option<String>,
    source_author: Option<String>,
    source_featured_image: Option<String>,
    source_favicon: Option<String>,
    target: String,
    resource_id: Option<String>,
    resource_type: Option<String>,
    verified: bool,
    created_at: String,
    resource_title: Option<String>,
}

/// `GET /incoming_recommendations` — the sites that recommend this one.
///
/// Not a table of its own: these are the mentions whose source is another
/// Ghost site's recommendations file. `recommending_back` says whether this
/// site recommends them too, which is the thing the screen is for.
async fn incoming_recommendations(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<String>, String)>(
        "SELECT id, source, source_site_title, source_title, source_excerpt, source_favicon, created_at \
         FROM mentions WHERE deleted = 0 AND source LIKE '%/.well-known/recommendations.json' \
         ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut recommendations = Vec::new();
    for (id, source, site_title, title, excerpt, favicon, created_at) in rows {
        let url = source.trim_end_matches("/.well-known/recommendations.json").trim_end_matches('/').to_string();
        let recommending_back: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recommendations WHERE url = ? OR url = ? || '/')")
                .bind(&url)
                .bind(&url)
                .fetch_one(&state.db)
                .await
                .unwrap_or(false);
        recommendations.push(json!({"id": id, "url": url,
            "title": site_title.or(title), "excerpt": excerpt, "favicon": favicon,
            "recommending_back": recommending_back, "created_at": created_at}));
    }
    Json(json!({"incoming_recommendations": recommendations})).into_response()
}

/// `/.well-known/recommendations.json` — what this site recommends, so that
/// another Ghost site can discover it and show itself as recommended.
async fn recommendations_json(State(state): State<AppState>) -> Response {
    let rows = sqlx::query_as::<_, (String,)>("SELECT url FROM recommendations ORDER BY created_at DESC LIMIT 100")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    (
        [("content-type", "application/json"), ("cache-control", "public, max-age=300")],
        Json(rows.into_iter().map(|(url,)| json!({"url": url})).collect::<Vec<_>>()),
    )
        .into_response()
}

/// Tell a recommended site that it has been recommended, by sending a
/// webmention from this site's recommendations file to theirs.
///
/// This is how the other end's "who recommends me" screen learns about it,
/// and it is why the file above exists.
pub fn announce_recommendation(state: &AppState, url: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let source = format!("{}/.well-known/recommendations.json", state.site_url().trim_end_matches('/'));
        let target = format!("{}/.well-known/recommendations.json", url.trim_end_matches('/'));
        if let Err(e) = send_one(&state, &source, &target).await {
            tracing::debug!("recommendation announcement to {target} was not delivered: {}", e.message());
        }
    });
}

/// `GET /mentions/{id}` is not part of Ghost's API; this is the delete a
/// moderator needs when a mention is spam.
pub async fn hide(state: &AppState, id: &str) -> Result<()> {
    sqlx::query("UPDATE mentions SET deleted=1 WHERE id=?")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_http_urls_are_ever_fetched() {
        assert!(fetchable("https://example.com/post/").is_some());
        assert!(fetchable("http://example.com").is_some());
        // Anything that would have the server read something on the sender's
        // behalf rather than fetch a web page.
        for refused in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "http://localhost:5432/",
            "http://x.localhost/",
            "http://127.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.1.2.3/",
            "http://192.168.0.1/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://user:pw@example.com/",
            "https://db.internal/",
            "not a url",
            "",
        ] {
            assert!(fetchable(refused).is_none(), "{refused} should not be fetchable");
        }
    }

    #[test]
    fn a_link_is_found_however_it_is_written() {
        let target = "https://example.com/my-post/";
        assert!(links_to(r#"<a href="https://example.com/my-post/">x</a>"#, target));
        // Trailing slash, host case and a tracking parameter do not hide it.
        assert!(links_to(r#"<a href="https://EXAMPLE.com/my-post">x</a>"#, target));
        assert!(links_to(r#"<a href='https://example.com/my-post/?utm_source=x'>x</a>"#, target));
        // A different page on the same site is not the target.
        assert!(!links_to(r#"<a href="https://example.com/other/">x</a>"#, target));
        assert!(!links_to("<p>no links here</p>", target));
        // The claim being in the text rather than a link is not a link.
        assert!(!links_to("<p>https://example.com/my-post/</p>", target));
    }

    #[test]
    fn an_endpoint_is_discovered_the_way_the_specification_orders_it() {
        // Quoted, unquoted, and among other rels.
        assert_eq!(webmention_link_header(r#"<https://a.example/wm>; rel="webmention""#).as_deref(), Some("https://a.example/wm"));
        assert_eq!(webmention_link_header("<https://a.example/wm>; rel=webmention").as_deref(), Some("https://a.example/wm"));
        assert_eq!(
            webmention_link_header(r#"<https://a.example/hub>; rel="hub", <https://a.example/wm>; rel="webmention""#).as_deref(),
            Some("https://a.example/wm")
        );
        assert_eq!(webmention_link_header(r#"<https://a.example/x>; rel="canonical""#), None);

        // The first matching element wins, whether link or anchor.
        assert_eq!(webmention_rel(r#"<link rel="webmention" href="/wm">"#).as_deref(), Some("/wm"));
        assert_eq!(webmention_rel(r#"<html><a rel="webmention" href="/endpoint">x</a>"#).as_deref(), Some("/endpoint"));
        assert_eq!(
            webmention_rel(r#"<link rel="stylesheet" href="/a.css"><link rel="webmention" href="/first"><link rel="webmention" href="/second">"#).as_deref(),
            Some("/first")
        );
        // An empty href means the page itself, which is allowed.
        assert_eq!(webmention_rel(r#"<link rel="webmention" href="">"#).as_deref(), Some(""));
        assert_eq!(webmention_rel("<p>nothing</p>"), None);
    }

    #[test]
    fn a_sources_own_description_is_read_from_its_meta_tags() {
        let html = r#"<html><head><title>Fallback &amp; title</title>
            <meta property="og:title" content="A post about Ghost">
            <meta property="og:site_name" content="Someone&#39;s Blog">
            <meta name="description" content="Why we did the thing.">
            <meta property="og:image" content="https://a.example/card.png">
            <meta name="author" content="A Writer"></head></html>"#;
        let meta = metadata_of(html);
        assert_eq!(meta.title.as_deref(), Some("A post about Ghost"));
        assert_eq!(meta.site_title.as_deref(), Some("Someone's Blog"));
        assert_eq!(meta.excerpt.as_deref(), Some("Why we did the thing."));
        assert_eq!(meta.image.as_deref(), Some("https://a.example/card.png"));
        assert_eq!(meta.author.as_deref(), Some("A Writer"));

        // No Open Graph: the document title stands in, entities decoded.
        let bare = metadata_of("<html><head><title>Fallback &amp; title</title></head></html>");
        assert_eq!(bare.title.as_deref(), Some("Fallback & title"));
        assert_eq!(bare.excerpt, None);
        assert_eq!(metadata_of("<p>no head</p>").title, None);
    }

    #[test]
    fn a_recommendation_is_told_apart_from_an_ordinary_link() {
        assert!(is_recommendation("https://other.example/.well-known/recommendations.json"));
        assert!(is_recommendation("https://other.example/.well-known/recommendations.json/"));
        assert!(!is_recommendation("https://other.example/blog/post/"));
        assert!(!is_recommendation("https://other.example/recommendations.json"));
    }
}
