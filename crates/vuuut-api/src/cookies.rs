//! The internet-facing half of Cookie consent & privacy compliance.
//!
//! One injection and one endpoint, in the order a request meets them:
//!
//! 1. **[`inject`]** runs on every rendered page while the banner is on. It
//!    first holds back every recognised tracker in the page
//!    (`vuuut_core::cookies::gate_html` — a `<script>` becomes
//!    `type="text/plain"`, an embed loses its `src`), then adds the stylesheet
//!    and one deferred script carrying the banner's configuration in a
//!    `data-config` attribute. Switched off, it returns the page untouched
//!    without touching the database.
//!
//!    The blocking is done here, on the server, because it is the only place
//!    early enough: by the time any script of ours runs, a tracker earlier in
//!    the document has already set its cookie. Which trackers come back is the
//!    browser script's decision, because it is the visitor's — and it is not
//!    a secret, so nothing is lost by making it there.
//!
//! 2. **`POST /cookies/api/consent/`** records a decision in the
//!    proof-of-consent log. Same-origin only, a 2 KB body, a per-address
//!    budget, and the decision rebuilt by `vuuut_core::cookies::Choice::parse`
//!    so a tampered body cannot log a refusal as a grant. It stores no
//!    address, no user agent and no member id; the id it logs is one the
//!    visitor's own browser generated and can show them.
//!
//! The configuration is not secret — it is the banner's own copy and the
//! cookie declaration — so it travels in the page rather than through a
//! second request. That also means the banner can draw without waiting on
//! the network.

use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use vuuut_core::cookies::{self as rules, Category, Choice, Settings};
use vuuut_db::repos::cookies as repo;
use serde_json::json;
use std::collections::HashMap;

/// An action, a category list, an id and a path. Anything larger is not one.
const MAX_BODY_BYTES: usize = 2 * 1024;

pub static JS: &str = include_str!("../assets/cookies.js");
pub static CSS: &str = include_str!("../assets/cookies.css");

/// Content hash, so a changed script is a new URL by construction and the old
/// one can be cached for a year.
fn asset_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
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
        .route(
            "/cookies/api/consent/",
            post(consent).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route(
            "/cookies/api/assets/{version}/cookies.js",
            get(|| async { asset(JS, "text/javascript; charset=utf-8") }),
        )
        .route(
            "/cookies/api/assets/{version}/cookies.css",
            get(|| async { asset(CSS, "text/css; charset=utf-8") }),
        )
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

/// Settings → Cookies, read the way the admin reads it.
pub fn settings(state: &AppState) -> Settings {
    Settings::read(|key| state.settings.get_str(key))
}

/// Hold back the trackers and add the banner, if the banner is on.
///
/// Called from the renderer last, after forms and popups have added their
/// own markup, so a popup's script is seen too (it is necessary, and left
/// alone).
pub async fn inject(state: &AppState, html: String) -> String {
    let site = settings(state);
    if !site.enabled || !html.contains("</body>") {
        return html;
    }
    let html = if site.mode.gates() {
        rules::gate_html(&html, site.block_embeds).0
    } else {
        html
    };
    let declarations = repo::declarations(&state.db).await.unwrap_or_default();
    let config = config_json(&site, &declarations);
    let base = vuuut_core::url::base("");
    let v = asset_version();
    let css = format!(r#"<link rel="stylesheet" href="{base}/cookies/api/assets/{v}/cookies.css">"#);
    let script = format!(
        r#"<script src="{base}/cookies/api/assets/{v}/cookies.js" defer data-cookie-ignore data-config="{config}"></script>"#,
        config = attr_escape(&config.to_string()),
    );
    // In the head when there is one, so the stylesheet is there before the
    // banner draws; otherwise at the end of the body, which always exists.
    let tags = format!("{css}{script}");
    let at = html.find("</head>").or_else(|| html.rfind("</body>"));
    match at {
        Some(at) => {
            let mut out = String::with_capacity(html.len() + tags.len());
            out.push_str(&html[..at]);
            out.push_str(&tags);
            out.push_str(&html[at..]);
            out
        }
        None => html,
    }
}

/// What the browser script needs: the banner's copy and behaviour, and the
/// declaration grouped by category, built-in cookies first.
pub fn config_json(site: &Settings, declared: &[repo::Declaration]) -> serde_json::Value {
    let mut by_category: HashMap<Category, Vec<serde_json::Value>> = HashMap::new();
    for c in rules::builtin_cookies() {
        by_category.entry(c.category).or_default().push(json!({
            "name": c.name, "provider": c.provider, "duration": c.duration, "purpose": c.purpose,
        }));
    }
    for d in declared {
        let Some(category) = Category::from_key(&d.category) else { continue };
        by_category.entry(category).or_default().push(json!({
            "name": d.name, "provider": d.provider, "duration": d.duration, "purpose": d.purpose,
        }));
    }
    let categories: Vec<serde_json::Value> = Category::ALL
        .into_iter()
        .map(|c| {
            json!({
                "key": c.key(),
                "label": c.label(),
                "description": c.description(),
                "required": c == Category::Necessary,
                "cookies": by_category.remove(&c).unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "mode": site.mode.key(),
        "layout": site.layout.key(),
        "title": site.title,
        "message": site.message,
        "policy": site.policy_url,
        "revision": site.revision,
        "days": site.expiry_days(),
        "gpc": site.honour_gpc,
        "reopen": site.reopen,
        "log": site.log,
        "endpoint": format!("{}/cookies/api/consent/", vuuut_core::url::base("")),
        "categories": categories,
    })
}

/// Escape for a double-quoted attribute. The JSON is built from settings and
/// declarations staff typed, so every one of these characters can occur.
fn attr_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// One visitor's decision, for the log.
async fn consent(
    State(state): State<AppState>,
    crate::brute::ClientAddr(peer): crate::brute::ClientAddr,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !crate::forms::same_origin(&headers, &state.site_url()) {
        return problem(StatusCode::FORBIDDEN, "This did not come from the site.");
    }
    let site = settings(&state);
    // A page cached from before the switch went off can still post; and a
    // site that keeps no log keeps no log, whatever the browser thinks.
    if !site.enabled || !site.log {
        return StatusCode::NO_CONTENT.into_response();
    }
    let mut input: HashMap<String, String> = HashMap::new();
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        input.entry(key.into_owned()).or_insert_with(|| value.into_owned());
    }
    let field = |name: &str| input.get(name).map(String::as_str).unwrap_or("");
    let Some(choice) = Choice::parse(field("action"), field("categories")) else {
        return problem(StatusCode::UNPROCESSABLE_ENTITY, "That is not a choice this banner offers.");
    };
    let consent_id = field("id").to_ascii_lowercase();
    if !rules::is_consent_id(&consent_id) {
        return problem(StatusCode::UNPROCESSABLE_ENTITY, "Missing consent id.");
    }
    // Generous: one person changing their mind a few times, a household
    // behind one address. Tight enough that the log is not a free table.
    let ip = crate::brute::client_ip_for(&state, &headers, peer);
    if crate::brute::check(&state.db, &format!("cookie_consent:{ip}"), &PER_IP).await.is_err() {
        return problem(StatusCode::TOO_MANY_REQUESTS, "Too many changes. Please wait a little.");
    }
    let gpc = headers.get("sec-gpc").is_some_and(|v| v.as_bytes() == b"1");
    // A path on this site, bounded; anything else is recorded as nothing.
    let page = rules::clean(field("page"), 300);
    let page = if page.starts_with('/') && !page.starts_with("//") { page } else { String::new() };
    // The revision is the server's, not the browser's: a decision is about
    // what the site does now, whatever an old cached page believed.
    if let Err(e) = repo::record(&state.db, &consent_id, &choice, site.revision, gpc, &page).await {
        tracing::error!("cookies: could not record a consent: {}", e.message());
        return problem(StatusCode::INTERNAL_SERVER_ERROR, "That could not be saved.");
    }
    // Retention, swept now and then by the requests that grow the table, so
    // a site whose staff never open the log still forgets on schedule.
    if rand::random::<u8>() < 3 {
        let _ = repo::purge_older_than(&state.db, site.log_days).await;
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

const PER_IP: crate::brute::Budget =
    crate::brute::Budget { free_retries: 30, min_wait: 30_000, max_wait: 600_000, lifetime: 3600 };

fn problem(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "message": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configuration_survives_an_attribute_whatever_staff_typed() {
        let site = Settings::read(|k| match k {
            rules::SETTING_TITLE => Some(r#""><script>alert(1)</script>"#.into()),
            _ => None,
        });
        let escaped = attr_escape(&config_json(&site, &[]).to_string());
        assert!(!escaped.contains('"') && !escaped.contains('<') && !escaped.contains('\''));
    }

    #[test]
    fn the_declaration_lists_built_in_cookies_first_and_skips_unknown_categories() {
        let declared = vec![
            repo::Declaration {
                id: "a".into(), name: "_ga".into(), provider: "Google".into(), category: "analytics".into(),
                duration: "2 years".into(), purpose: "x".into(), tracker_key: "".into(),
                created_at: "".into(), updated_at: "".into(),
            },
            repo::Declaration {
                id: "b".into(), name: "odd".into(), provider: "".into(), category: "bogus".into(),
                duration: "".into(), purpose: "".into(), tracker_key: "".into(),
                created_at: "".into(), updated_at: "".into(),
            },
        ];
        let config = config_json(&Settings::default(), &declared);
        let cats = config["categories"].as_array().unwrap();
        assert_eq!(cats[0]["key"], "necessary");
        assert_eq!(cats[0]["required"], true);
        assert_eq!(cats[0]["cookies"][0]["name"], "vuuut_consent");
        assert_eq!(cats[2]["cookies"][0]["name"], "_ga");
        assert!(!config.to_string().contains("odd"));
    }
}
