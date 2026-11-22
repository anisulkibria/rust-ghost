//! Caching & Performance: the page cache, response headers, timing, and the
//! CDN purge — the runtime half of the tool.
//!
//! **The page cache** is [`page_cache`], a middleware on the public router,
//! inside the analytics counter (so a cached view is still counted) and the
//! private-site gate. For each request it asks
//! `ghost_core::caching::request_key` whether the request is an anonymous
//! page view; if so and a stored copy from the *current content generation*
//! (`ghost_db::content`) is younger than the configured lifetime, it answers
//! from memory. Otherwise it renders as usual and, if
//! `ghost_core::caching::response_storable` agrees, keeps the result. The
//! generation goes up whenever a post, tag or setting is written and on every
//! site event, so an edit is never hidden behind a stale page — the lifetime
//! is a backstop, not the mechanism.
//!
//! Every HTML response gets an `X-Cache` header saying what happened and,
//! for anonymous pages, the configured `Cache-Control` and (when on) an
//! `ETag`, answering a matching `If-None-Match` with `304 Not Modified`.
//!
//! **Timing**: every HTML response's time is kept in a ring of the last
//! thousand, for the Overview's percentiles and slowest pages. The home page
//! as last rendered is kept too, for the performance check.
//!
//! **Compression** is a `tower-http` gzip layer whose predicate reads a flag
//! this middleware refreshes from settings, so it can be switched without a
//! restart.
//!
//! **The CDN**: with a Cloudflare zone connected and auto-purge on, a
//! content change purges the zone — debounced, so publishing ten posts in a
//! minute is one purge, not ten.

use crate::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ghost_core::caching::{self as rules, Bypass, Settings};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

fn site_key(state: &AppState) -> usize {
    Arc::as_ptr(&state.settings) as *const () as usize
}

// ---------------------------------------------------------------------------
// the store and the numbers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Entry {
    pub body: axum::body::Bytes,
    pub content_type: HeaderValue,
    pub generation: u64,
    pub stored: Instant,
    pub hits: u64,
}

#[derive(Default)]
struct Site {
    pages: HashMap<String, Entry>,
    hits: u64,
    misses: u64,
    bypasses: HashMap<&'static str, u64>,
    samples: VecDeque<Sample>,
    home: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub path: String,
    pub ms: u64,
    pub outcome: &'static str,
}

const SAMPLES: usize = 1000;
static SITES: LazyLock<Mutex<HashMap<usize, Site>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// What the gzip layer's predicate reads.
pub static GZIP: AtomicBool = AtomicBool::new(false);

fn with_site<T>(state: &AppState, f: impl FnOnce(&mut Site) -> T) -> T {
    let mut sites = SITES.lock().unwrap_or_else(|e| e.into_inner());
    f(sites.entry(site_key(state)).or_default())
}

/// What the Overview shows.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub pages: Vec<(String, usize, u64, u64)>,
    pub bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub bypasses: Vec<(&'static str, u64)>,
    pub samples: Vec<Sample>,
    pub home: Option<String>,
}

pub fn snapshot(state: &AppState) -> Snapshot {
    let generation = ghost_db::content::generation();
    let ttl = Duration::from_secs(settings(state).ttl as u64);
    with_site(state, |s| {
        let mut pages: Vec<(String, usize, u64, u64)> = s
            .pages
            .iter()
            .filter(|(_, e)| e.generation == generation && e.stored.elapsed() < ttl)
            .map(|(k, e)| (k.clone(), e.body.len(), e.stored.elapsed().as_secs(), e.hits))
            .collect();
        pages.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
        let mut bypasses: Vec<(&'static str, u64)> = s.bypasses.iter().map(|(k, v)| (*k, *v)).collect();
        bypasses.sort_by(|a, b| b.1.cmp(&a.1));
        Snapshot {
            bytes: pages.iter().map(|p| p.1).sum(),
            pages,
            hits: s.hits,
            misses: s.misses,
            bypasses,
            samples: s.samples.iter().cloned().collect(),
            home: s.home.clone(),
        }
    })
}

/// Empty this site's cache. Returns how many pages went.
pub fn purge_all(state: &AppState) -> usize {
    with_site(state, |s| {
        let n = s.pages.len();
        s.pages.clear();
        n
    })
}

pub fn purge_one(state: &AppState, key: &str) -> bool {
    with_site(state, |s| s.pages.remove(key).is_some())
}

/// Forget the counters, keep the pages.
pub fn reset_stats(state: &AppState) {
    with_site(state, |s| {
        s.hits = 0;
        s.misses = 0;
        s.bypasses.clear();
        s.samples.clear();
    });
}

fn record(state: &AppState, path: &str, started: Instant, outcome: &'static str) {
    let ms = started.elapsed().as_millis() as u64;
    with_site(state, |s| {
        if s.samples.len() == SAMPLES {
            s.samples.pop_front();
        }
        s.samples.push_back(Sample { path: path.chars().take(200).collect(), ms, outcome });
    });
}

// ---------------------------------------------------------------------------
// the middleware
// ---------------------------------------------------------------------------

fn with_header(mut r: Response, name: &'static str, value: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(value) {
        r.headers_mut().insert(name, v);
    }
    r
}

pub async fn page_cache(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let s = settings(&state);
    GZIP.store(s.gzip, Ordering::Relaxed);
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);
    let method = req.method().as_str().to_string();
    let cookies = req.headers().get(header::COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
    let private = state.settings.get_bool("is_private");
    let key = rules::request_key(&s, private, &method, &path, query.as_deref(), &cookies);
    let generation = ghost_db::content::generation();

    // ---- a hit ----
    if let Ok(k) = &key {
        let found = with_site(&state, |site| {
            let e = site.pages.get_mut(k)?;
            if e.generation != generation || e.stored.elapsed() >= Duration::from_secs(s.ttl as u64) {
                site.pages.remove(k);
                return None;
            }
            e.hits += 1;
            let hit = e.clone();
            site.hits += 1;
            Some(hit)
        });
        if let Some(e) = found {
            let age = e.stored.elapsed().as_secs().to_string();
            let response = finish_html(&s, true, e.body, e.content_type, if_none_match.as_deref(), "HIT");
            record(&state, &path, started, "hit");
            return with_header(response, "age", &age);
        }
    }

    // ---- render ----
    let response = next.run(req).await;
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    if !is_html {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, rules::MAX_PAGE_BYTES * 4).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "The page could not be read.").into_response();
    };
    let content_type = parts.headers.get(header::CONTENT_TYPE).cloned().unwrap_or(HeaderValue::from_static("text/html; charset=utf-8"));
    let cache_control = parts.headers.get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let sets_cookie = parts.headers.contains_key(header::SET_COOKIE);
    if path == "/" && parts.status == StatusCode::OK {
        let home = String::from_utf8_lossy(&bytes).chars().take(1_000_000).collect::<String>();
        with_site(&state, |site| site.home = Some(home));
    }

    let outcome: Result<(), Bypass> = key.as_ref().map(|_| ()).map_err(|b| *b).and_then(|_| {
        rules::response_storable(parts.status.as_u16(), content_type.to_str().unwrap_or(""), &cache_control, sets_cookie, &bytes)
    });
    match (&key, outcome) {
        (Ok(k), Ok(())) => {
            with_site(&state, |site| {
                site.misses += 1;
                if site.pages.len() as i64 >= s.max_pages && !site.pages.contains_key(k) {
                    // Evict the oldest: the page most likely to be stale anyway.
                    if let Some(oldest) = site.pages.iter().min_by_key(|(_, e)| e.stored).map(|(k, _)| k.clone()) {
                        site.pages.remove(&oldest);
                    }
                }
                site.pages.insert(k.clone(), Entry { body: bytes.clone(), content_type: content_type.clone(), generation, stored: Instant::now(), hits: 0 });
            });
            let mut r = finish_html(&s, true, bytes, content_type, if_none_match.as_deref(), "MISS");
            copy_headers(&parts.headers, r.headers_mut());
            *r.status_mut() = if r.status() == StatusCode::NOT_MODIFIED { StatusCode::NOT_MODIFIED } else { parts.status };
            record(&state, &path, started, "miss");
            r
        }
        (_, Err(reason)) => {
            with_site(&state, |site| *site.bypasses.entry(reason.label()).or_default() += 1);
            record(&state, &path, started, "bypass");
            // With every switch off the response goes out exactly as it was
            // rendered: installing the tool changes nothing.
            let active = s.page_cache || s.etag || s.browser_ttl > 0 || s.cdn_ttl > 0;
            let mut r = Response::from_parts(parts.clone(), Body::from(bytes.clone()));
            if !active {
                return r;
            }
            // An anonymous page that was not stored only because of what it
            // contains (a form) or because the cache is off still gets the
            // public headers; anything personal keeps its own.
            let anonymous = rules::request_key(&Settings { page_cache: true, ..s.clone() }, private, &method, &path, query.as_deref(), &cookies).is_ok();
            let public = anonymous
                && parts.status == StatusCode::OK
                && !sets_cookie
                && !cache_control.contains("private")
                && !cache_control.contains("no-store");
            if public {
                r = finish_html(&s, true, bytes, content_type, if_none_match.as_deref(), "BYPASS");
                copy_headers(&parts.headers, r.headers_mut());
            }
            with_header(r, "x-cache", &format!("BYPASS ({})", reason.label()))
        }
        (Err(_), Ok(())) => unreachable!("a refused key cannot be storable"),
    }
}

/// Headers the renderer set that a stored or rebuilt response must keep —
/// except the ones this middleware decides itself.
fn copy_headers(from: &HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from {
        if matches!(name.as_str(), "content-length" | "cache-control" | "etag" | "content-type") {
            continue;
        }
        if !to.contains_key(name) {
            to.insert(name.clone(), value.clone());
        }
    }
}

/// A public page: the configured `Cache-Control`, and an `ETag` with
/// `304 Not Modified` when switched on.
fn finish_html(s: &Settings, public: bool, body: axum::body::Bytes, content_type: HeaderValue, if_none_match: Option<&str>, xcache: &str) -> Response {
    let tag = s.etag.then(|| rules::etag(&body));
    let not_modified = matches!((&tag, if_none_match), (Some(t), Some(inm)) if rules::etag_matches(inm, t));
    let mut r = if not_modified { Response::new(Body::empty()) } else { Response::new(Body::from(body)) };
    if not_modified {
        *r.status_mut() = StatusCode::NOT_MODIFIED;
    }
    r.headers_mut().insert(header::CONTENT_TYPE, content_type);
    if public {
        if let Ok(v) = HeaderValue::from_str(&rules::cache_control(s)) {
            r.headers_mut().insert(header::CACHE_CONTROL, v);
        }
    }
    if let Some(t) = tag.and_then(|t| HeaderValue::from_str(&t).ok()) {
        r.headers_mut().insert(header::ETAG, t);
    }
    with_header(r, "x-cache", xcache)
}

/// The gzip layer's switch: compress when the tool says so and the default
/// rules agree (not images, not tiny, not already encoded).
pub fn gzip_enabled() -> bool {
    GZIP.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// rendering hooks
// ---------------------------------------------------------------------------

/// Lazy-load the images in a post body, when switched on.
pub fn apply_to_post(state: &AppState, data: &mut Value) {
    if !settings(state).lazy_images {
        return;
    }
    if let Some(html) = data.pointer("/post/html").and_then(Value::as_str) {
        let lazy = rules::lazy_images(html);
        data["post"]["html"] = Value::String(lazy);
    }
}

/// Preconnect hints and speculation rules, before `</head>`.
pub fn head_tags(state: &AppState, html: String) -> String {
    let s = settings(state);
    let mut tags = rules::preconnect(&s.preconnect);
    if let Some(rules_tag) = rules::speculation_rules(s.speculation) {
        tags.push_str(&rules_tag);
    }
    if tags.is_empty() {
        return html;
    }
    match html.find("</head>") {
        Some(at) => format!("{}{tags}{}", &html[..at], &html[at..]),
        None => html,
    }
}

// ---------------------------------------------------------------------------
// the CDN
// ---------------------------------------------------------------------------

static PURGE_PENDING: AtomicBool = AtomicBool::new(false);
static PURGES: AtomicU64 = AtomicU64::new(0);

/// Content changed: the in-memory cache already keys on the generation; this
/// bumps it for the events that write nothing to the database (theme files,
/// routes), and purges the CDN when one is connected and auto-purge is on.
pub fn content_changed(state: &AppState) {
    ghost_db::content::touch();
    if !settings(state).cdn_auto_purge || PURGE_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        // One purge for a burst of changes.
        tokio::time::sleep(Duration::from_secs(15)).await;
        PURGE_PENDING.store(false, Ordering::SeqCst);
        if let Some(cdn) = ghost_db::repos::caching::cdn(&state.db).await.filter(|c| c.connected()) {
            let result = purge_cdn(&cdn.zone, &cdn.token, None).await;
            ghost_db::repos::caching::record_purge(&state.db, result.err().as_deref().unwrap_or("")).await;
        }
    });
}

pub fn purge_count() -> u64 {
    PURGES.load(Ordering::Relaxed)
}

fn cf() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

fn cf_error(v: &Value, status: u16) -> String {
    let message = v.pointer("/errors/0/message").and_then(Value::as_str).unwrap_or("no reason given");
    format!("Cloudflare answered {status}: {message}")
}

/// A zone id is 32 hex characters; a token is a long URL-safe string.
pub fn valid_zone(zone: &str) -> bool {
    zone.len() == 32 && zone.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn valid_token(token: &str) -> bool {
    (20..=200).contains(&token.len()) && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Check the zone and token, returning the zone's name.
pub async fn test_cdn(zone: &str, token: &str) -> Result<String, String> {
    let r = cf()
        .get(format!("https://api.cloudflare.com/client/v4/zones/{zone}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "Cloudflare could not be reached.".to_string())?;
    let status = r.status().as_u16();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status != 200 || v.get("success") != Some(&Value::Bool(true)) {
        return Err(cf_error(&v, status));
    }
    Ok(v.pointer("/result/name").and_then(Value::as_str).unwrap_or("zone").to_string())
}

/// Purge everything, or just the given URLs.
pub async fn purge_cdn(zone: &str, token: &str, urls: Option<Vec<String>>) -> Result<(), String> {
    let body = match urls {
        Some(files) => json!({ "files": files }),
        None => json!({ "purge_everything": true }),
    };
    let r = cf()
        .post(format!("https://api.cloudflare.com/client/v4/zones/{zone}/purge_cache"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|_| "Cloudflare could not be reached.".to_string())?;
    let status = r.status().as_u16();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if status != 200 || v.get("success") != Some(&Value::Bool(true)) {
        return Err(cf_error(&v, status));
    }
    PURGES.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_and_token_shapes() {
        assert!(valid_zone("0123456789abcdef0123456789abcdef"));
        assert!(!valid_zone("0123456789abcdef0123456789abcde/"));
        assert!(valid_token("abcdefghij_klmnopqrst-UVW"));
        assert!(!valid_token("short"));
        assert!(!valid_token("has space in it and is long enough"));
    }
}
