//! The internet-facing half of Search and Filtering.
//!
//! In the order a request meets it:
//!
//! 1. **[`inject`]** adds the built-in engine's stylesheet and one deferred
//!    script to a rendered page — only while the engine is `vuuut`. The
//!    script binds the same things Sodo Search binds (`data-ghost-search`
//!    buttons, `#/search` links), so every Ghost theme's search button opens
//!    it without a theme change. While the engine is `sodo`, `ghost_head`
//!    adds Sodo as it always has; while it is `off`, neither is added.
//! 2. **`GET /search/api/?q=…&type=…&tag=…`** answers one search. Ranking is
//!    `vuuut_core::site_search::rank`; the corpus is a per-site in-memory
//!    index rebuilt at most every [`TTL`] and whenever staff save the tool,
//!    so a search costs no database query at all. A members-only post's body
//!    is never in the corpus (`Doc::new` enforces it), so no query can fish
//!    paid text out of it.
//! 3. With `log=1` — which the script sends once a visitor stops typing or
//!    picks a result, never per keystroke — the normalised query and its
//!    result count are added to the insights log, if the site keeps one.
//!
//! The search is read-only and public, so it takes no origin check and no
//! session. What bounds its cost is the query (100 characters, 8 words) and
//! the corpus (bodies cut at 20,000 characters), not a rate limit: a search
//! is a scan of memory the size of the site.

use crate::AppState;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use vuuut_core::site_search::{self as rules, Doc, Engine, Kind, Settings};
use vuuut_db::repos;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

pub static JS: &str = include_str!("../assets/search.js");
pub static CSS: &str = include_str!("../assets/search.css");

/// How long a built index is trusted. Publishing a post shows up in search
/// within this, without the save path having to know search exists.
pub const TTL: Duration = Duration::from_secs(60);

fn asset_version() -> &'static str {
    static V: LazyLock<String> = LazyLock::new(|| {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in JS.bytes().chain(CSS.bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:x}")
    });
    &V
}

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/search/api/", get(search))
        .route("/search/api", get(search))
        .route("/search/api/assets/{version}/search.js", get(|| async { asset(JS, "text/javascript; charset=utf-8") }))
        .route("/search/api/assets/{version}/search.css", get(|| async { asset(CSS, "text/css; charset=utf-8") }))
}

fn asset(body: &'static str, mime: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|key| state.settings.get_str(key))
}

// ---------------------------------------------------------------------------
// the index
// ---------------------------------------------------------------------------

/// One built corpus per site. Keyed by the address of the site's settings
/// cache, which is unique per running site (and per test fixture).
type Cache = HashMap<usize, (Instant, Arc<Vec<Doc>>)>;
static CACHE: LazyLock<RwLock<Cache>> = LazyLock::new(|| RwLock::new(HashMap::new()));

fn cache_key(state: &AppState) -> usize {
    Arc::as_ptr(&state.settings) as *const () as usize
}

/// Forget the built index, so the next search sees a change at once. The
/// admin calls it on save.
pub fn invalidate(state: &AppState) {
    if let Ok(mut c) = CACHE.write() {
        c.remove(&cache_key(state));
    }
}

/// The searchable posts and pages, as both engines see them.
pub async fn documents(state: &AppState) -> Arc<Vec<Doc>> {
    let key = cache_key(state);
    if let Ok(c) = CACHE.read() {
        if let Some((at, docs)) = c.get(&key) {
            if at.elapsed() < TTL {
                return docs.clone();
            }
        }
    }
    let docs = Arc::new(build(state, &settings(state)).await);
    if let Ok(mut c) = CACHE.write() {
        c.insert(key, (Instant::now(), docs.clone()));
    }
    docs
}

/// Everything published, filtered by the tool's rules. The tag lookup is one
/// query for the whole site rather than one per post.
pub async fn build(state: &AppState, s: &Settings) -> Vec<Doc> {
    let mut tags_of: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (post, name, slug) in repos::site_search::post_tags(&state.db).await.unwrap_or_default() {
        tags_of.entry(post).or_default().push((name, slug));
    }
    let resolver = crate::content::resolve(state).await;
    let mut out = Vec::new();
    for kind in [Kind::Post, Kind::Page] {
        if kind == Kind::Page && !s.include_pages {
            continue;
        }
        let page = repos::posts::browse(
            &state.db,
            &repos::posts::BrowsePosts {
                filter: None,
                order: Some("published_at DESC".into()),
                limit: 10_000,
                page: 1,
                post_type: Some(kind.key().into()),
                public: true,
                include_authors: false,
                include_tags: false,
                locale: None,
                default_locale: None,
            },
        )
        .await;
        let Ok(page) = page else { continue };
        for p in &page.posts {
            let tags = tags_of.remove(&p.id).unwrap_or_default();
            let slugs: Vec<String> = tags.iter().map(|(_, s)| s.clone()).collect();
            if !s.admits(kind, &p.visibility, &slugs) {
                continue;
            }
            let mut j = vuuut_core::models::post_to_json(p, &[], &[], "");
            let url = resolver(&j);
            j["url"] = json!(url);
            // A generated excerpt is the start of the body, so for a post that
            // is not public only the excerpt somebody wrote is kept — the
            // body rule in `Doc::new` would otherwise leak through this one.
            let excerpt = if p.visibility == "public" {
                j.get("excerpt").and_then(Value::as_str).unwrap_or("").to_string()
            } else {
                p.custom_excerpt.clone().unwrap_or_default()
            };
            out.push(Doc::new(
                p.id.clone(),
                kind,
                p.title.clone().unwrap_or_default(),
                excerpt,
                p.plaintext.as_deref().unwrap_or(""),
                tags,
                url,
                p.feature_image.clone().map(|i| vuuut_core::url::absolute(&state.site_url(), &i)),
                p.published_at.clone().unwrap_or_default(),
                p.featured,
                p.visibility.clone(),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the endpoint
// ---------------------------------------------------------------------------

async fn search(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = settings(&state);
    if s.engine != Engine::Vuuut {
        return no_store(json!({ "results": [], "tags": [], "authors": [], "facets": [] }));
    }
    let query = q.get("q").map(String::as_str).unwrap_or("");
    let kind = match q.get("type").map(String::as_str) {
        Some("post") => Some(Kind::Post),
        Some("page") if s.include_pages => Some(Kind::Page),
        _ => None,
    };
    // A tag filter is a slug: reduced to slug characters before it is
    // compared with anything.
    let tag = q.get("tag").map(|t| rules::parse_slugs(t)).and_then(|v| v.into_iter().next());
    let body = answer(&state, &s, query, kind, tag.as_deref()).await;
    if q.get("log").is_some_and(|v| v == "1") && s.log {
        if let Some(form) = rules::log_form(query) {
            let n = body["total"].as_i64().unwrap_or(0);
            let _ = repos::site_search::record(&state.db, &form, n).await;
            if rand::random::<u8>() < 3 {
                let _ = repos::site_search::purge_older_than(&state.db, rules::LOG_DAYS).await;
            }
        }
    }
    no_store(body)
}

/// One search, answered. Public so the admin's tester and the endpoint
/// cannot disagree.
pub async fn answer(state: &AppState, s: &Settings, query: &str, kind: Option<Kind>, tag: Option<&str>) -> Value {
    let docs = documents(state).await;
    let ranked = rules::rank(&docs, query, s, kind, tag);
    let groups = rules::expand(&rules::terms(query), &s.synonyms);
    // Facets come from the matches before the tag filter, so picking a tag
    // does not make the other chips vanish.
    let facets = if tag.is_some() { rules::facets(&rules::rank(&docs, query, s, kind, None), 8) } else { rules::facets(&ranked, 8) };
    let results: Vec<Value> = ranked
        .iter()
        .take(s.limit as usize)
        .map(|r| {
            let d = r.doc;
            // The passage comes from whichever field matched: the excerpt
            // when it holds a word, else the body (public posts only).
            let source = if !d.excerpt.is_empty()
                && groups.iter().flatten().any(|w| rules::normalize(&d.excerpt).contains(w.as_str()))
            {
                d.excerpt.as_str()
            } else if s.full_text && !d.body.is_empty() {
                d.body.as_str()
            } else {
                d.excerpt.as_str()
            };
            let (excerpt, marks) = rules::snippet(source, &groups, 180);
            let (_, title_marks) = rules::snippet(&d.title, &groups, 400);
            json!({
                "kind": d.kind.key(),
                "title": d.title,
                "titleMarks": title_marks,
                "url": d.url,
                "excerpt": excerpt,
                "marks": marks,
                "date": d.published_at.get(..10).unwrap_or(""),
                "tags": d.tags.iter().take(3).map(|(n, _)| n.clone()).collect::<Vec<_>>(),
                "image": d.image,
                "members": d.visibility != "public",
            })
        })
        .collect();
    let words = rules::terms(query);
    let (tags, authors) = if words.is_empty() || kind.is_some() || tag.is_some() {
        (Vec::new(), Vec::new())
    } else {
        names(state, s, &words).await
    };
    json!({
        "total": ranked.len(),
        "results": results,
        "tags": tags,
        "authors": authors,
        "facets": facets.iter().map(|(n, slug, c)| json!({"name": n, "slug": slug, "count": c})).collect::<Vec<_>>(),
    })
}

/// Tags and authors whose names contain every word — the two short lists
/// above the posts, as in Sodo.
async fn names(state: &AppState, s: &Settings, words: &[String]) -> (Vec<Value>, Vec<Value>) {
    let matches = |name: &str| {
        let n = rules::normalize(name);
        words.iter().all(|w| n.contains(w.as_str()))
    };
    // Linked through the same resolver the Content API uses, so a site whose
    // routes move tags or authors gets the right address.
    let resolver = crate::content::resolve(state).await;
    let mut tags = Vec::new();
    if s.show_tags {
        if let Ok((all, _)) = repos::tags::browse(&state.db, None, Some("name ASC"), 10_000, 1, true, false).await {
            for t in all {
                let name = t.name.clone().unwrap_or_default();
                let slug = t.slug.clone().unwrap_or_default();
                if matches(&name) && !s.exclude_tags.contains(&slug) {
                    let j = vuuut_core::models::tag_to_json(&t, None);
                    tags.push(json!({ "name": name, "url": resolver(&j) }));
                }
                if tags.len() == 5 {
                    break;
                }
            }
        }
    }
    let mut authors = Vec::new();
    if s.show_authors {
        if let Ok(users) = repos::users::list(&state.db, true).await {
            for u in users {
                let name = u.name.clone().unwrap_or_default();
                if matches(&name) {
                    let j = vuuut_core::models::user_public_json(&u, None);
                    authors.push(json!({ "name": name, "url": resolver(&j) }));
                }
                if authors.len() == 5 {
                    break;
                }
            }
        }
    }
    (tags, authors)
}

fn no_store(body: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(body),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// the injection
// ---------------------------------------------------------------------------

/// Add the built-in engine to a page, when it is the site's engine.
pub fn inject(state: &AppState, html: String) -> String {
    let s = settings(state);
    if s.engine != Engine::Vuuut || !html.contains("</body>") {
        return html;
    }
    let base = vuuut_core::url::base("");
    let v = asset_version();
    let config = json!({
        "endpoint": format!("{base}/search/api/"),
        "placeholder": s.placeholder,
        "pages": s.include_pages,
        "shortcut": s.shortcut,
        "log": s.log,
    });
    let tags = format!(
        r#"<link rel="stylesheet" href="{base}/search/api/assets/{v}/search.css"><script src="{base}/search/api/assets/{v}/search.js" defer data-cookie-ignore data-config="{c}"></script>"#,
        c = attr_escape(&config.to_string()),
    );
    let at = html.find("</head>").or_else(|| html.rfind("</body>"));
    match at {
        Some(at) => format!("{}{tags}{}", &html[..at], &html[at..]),
        None => html,
    }
}

fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('\'', "&#39;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_cannot_break_out_of_its_attribute() {
        let out = attr_escape(r#"{"placeholder":"\"><script>x</script>'"}"#);
        assert!(!out.contains('"') && !out.contains('<') && !out.contains('\''));
    }
}
