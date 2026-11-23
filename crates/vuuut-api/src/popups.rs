//! The internet-facing half of Popups & lead generation.
//!
//! Three endpoints and one injection, in the order a request meets them:
//!
//! 1. **`inject`** adds one `<script defer>` to a rendered page, and only
//!    when at least one popup is live. A site with none pays one indexed
//!    `COUNT(*)` per page view and ships no bytes.
//! 2. **`GET /popups/api/active`** answers with the popups this reader is
//!    allowed to see *on this path*. Targeting, audience and schedule are
//!    decided here rather than in the browser, so the copy for a campaign
//!    that has not started is not sitting in the page waiting to be found,
//!    and a members-only prompt is not readable by signing out.
//! 3. **`POST /popups/api/{id}/lead/`** takes an address. Same-origin,
//!    rate-limited per address and per popup, honeypot and minimum fill time
//!    — the same defences `forms` uses, for the same reason.
//! 4. **`POST /popups/api/{id}/seen/`** counts an impression. Deliberately
//!    cheap and deliberately approximate; see the repo.
//!
//! What the browser decides is trigger and frequency, because both are about
//! one person's behaviour over time and neither is a secret.

use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use vuuut_core::popups::{self as rules, Audience, Surface, Targeting};
use vuuut_db::repos::popups as repo;
use serde_json::json;
use std::collections::HashMap;

/// A lead is an address and a name. Anything larger is not one.
const MAX_BODY_BYTES: usize = 4 * 1024;

/// A form filled in faster than this was not filled in by a person. Matches
/// the floor `forms` uses.
const MIN_FILL_MS: i64 = 1_200;

static JS: &str = include_str!("../assets/popups.js");
static CSS: &str = include_str!("../assets/popups.css");

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
        .route("/popups/api/active", get(active))
        .route("/popups/api/active/", get(active))
        .route(
            "/popups/api/{id}/lead/",
            post(lead).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/popups/api/{id}/seen/", post(seen))
        .route(
            "/popups/api/assets/{version}/popups.js",
            get(|| async { asset(JS, "text/javascript; charset=utf-8") }),
        )
        .route(
            "/popups/api/assets/{version}/popups.css",
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

/// Add the script to a rendered page, if there is anything to show.
///
/// Called from the renderer beside `forms::expand_html`. It does not decide
/// *which* popups — see `active` — it only decides whether this page needs
/// the machinery at all.
pub async fn inject(state: &AppState, html: String) -> String {
    // The master switch is checked before the database: switched off, a page
    // view costs nothing at all.
    if !html.contains("</body>")
        || !site_settings(state).enabled
        || !repo::any_live(&state.db).await
    {
        return html;
    }
    let tag = format!(
        r#"<script src="{base}/popups/api/assets/{v}/popups.js" defer data-popups="{base}"></script>"#,
        base = vuuut_core::url::base(""),
        v = asset_version(),
    );
    // Only the last `</body>`: a page that prints the literal string in a
    // code sample should still get its script at the end of the document.
    match html.rfind("</body>") {
        Some(at) => {
            let mut out = String::with_capacity(html.len() + tag.len());
            out.push_str(&html[..at]);
            out.push_str(&tag);
            out.push_str(&html[at..]);
            out
        }
        None => html,
    }
}

/// What this reader may see on this path.
async fn active(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let path = q.get("path").map(String::as_str).unwrap_or("/");
    let site = site_settings(&state);
    if !site.enabled {
        // A page cached from before the switch went off can still load the
        // script; it gets nothing to show.
        return no_store_json(json!({ "popups": [] }));
    }
    let is_member = crate::members::current_member(&state, &headers).await.is_some();
    let now = vuuut_core::dates::now_db();

    let live = repo::live(&state.db).await.unwrap_or_default();
    // What kind of page this is, and only when something actually asks: a
    // site whose popups all say "anywhere" never runs the lookup.
    let kind = if live.iter().any(|p| Surface::from_name(&p.surface).needs_lookup()) {
        repo::kind_of_path(&state.db, path).await
    } else {
        "other"
    };
    let items: Vec<serde_json::Value> = live
        .into_iter()
        .filter(|p| {
            Surface::from_name(&p.surface).includes(kind)
                && Audience::from_name(&p.audience).includes(is_member)
                && rules::in_window(&now, p.starts_at.as_deref(), p.ends_at.as_deref())
                && rules::matches_path(
                    Targeting::from_name(&p.targeting),
                    &p.path_rules(),
                    path,
                )
        })
        .map(|p| {
            // Only what the browser needs to draw and time it. The path
            // rules, the schedule and the audience stay on the server: they
            // have already been applied, and shipping them would hand a
            // reader the shape of every campaign on the site.
            json!({
                "id": p.id,
                "kind": p.kind,
                "heading": p.heading,
                "body": p.body,
                "button_label": p.button_label,
                "button_url": p.button_url,
                "success_message": p.success_message,
                "capture_email": p.captures_email(),
                "capture_name": p.captures_name(),
                "trigger": p.trigger_kind,
                "trigger_value": p.trigger_value,
                "frequency": p.frequency,
                "frequency_days": p.frequency_days,
            })
        })
        .collect();

    no_store_json(json!({
        "popups": items,
        // Applied in the browser because all three are about one person's
        // screen and history, and none of them is a secret.
        "site": {
            "hide_on_mobile": site.hide_on_mobile,
            "cooldown_hours": site.cooldown_hours,
            "consent": site.consent_text,
        },
    }))
}

/// Per-reader (the audience depends on their member cookie) and short-lived,
/// so neither a CDN nor a shared cache may keep it.
fn no_store_json(body: serde_json::Value) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        Json(body).into_response().into_body(),
    )
        .into_response()
}

/// Settings → Popups, read the way the admin reads it.
fn site_settings(state: &AppState) -> rules::SiteSettings {
    rules::SiteSettings::read(|key| state.settings.get_str(key))
}

/// One more person saw it.
async fn seen(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if !same_site(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if vuuut_core::ids::is_objectid(&id) {
        repo::record_impression(&state.db, &id).await;
    }
    // Nothing to say, and the browser is not waiting to hear it.
    StatusCode::NO_CONTENT.into_response()
}

/// An address, from a popup.
async fn lead(
    State(state): State<AppState>,
    crate::brute::ClientAddr(peer): crate::brute::ClientAddr,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Response {
    if !same_site(&state, &headers) {
        return problem(StatusCode::FORBIDDEN, "This did not come from the site.");
    }
    let mut input: HashMap<String, String> = HashMap::new();
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        input.entry(key.into_owned()).or_insert_with(|| value.into_owned());
    }
    let field = |name: &str| input.get(name).map(String::as_str).unwrap_or("");

    // A bot that fills every field gets a cheerful nothing: telling it which
    // defence caught it is how it learns to pass.
    if !field("website").is_empty() {
        return accepted("Thanks!");
    }
    let elapsed = field("t").parse::<i64>().map(|started| now_ms() - started).unwrap_or(0);
    if !(MIN_FILL_MS..).contains(&elapsed) {
        return accepted("Thanks!");
    }

    let email = rules::clean(field("email"), rules::MAX_EMAIL).to_lowercase();
    if !rules::is_email(&email) {
        return problem(StatusCode::UNPROCESSABLE_ENTITY, "That does not look like an email address.");
    }
    let name = rules::clean(field("name"), rules::MAX_LEAD_NAME);

    // Per address, then per popup: one person cannot be made to receive a
    // hundred welcome emails, and one popup cannot be used as a mailer.
    // `client_ip_for` honours the site's trust-proxy setting, so a budget an
    // address can choose for itself is not a budget — see `brute`.
    let ip = crate::brute::client_ip_for(&state, &headers, peer);
    let ip_key = format!("popup_lead:{ip}");
    if crate::brute::check(&state.db, &ip_key, &LEAD_PER_IP).await.is_err() {
        return problem(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Please wait a little.");
    }

    let Ok(Some(popup)) = repo::find(&state.db, &id).await else {
        return problem(StatusCode::NOT_FOUND, "This is not taking addresses.");
    };
    if !popup.is_live() || !popup.captures_email() || !site_settings(&state).enabled {
        return problem(StatusCode::NOT_FOUND, "This is not taking addresses.");
    }
    if crate::brute::check(&state.db, &format!("popup_lead_all:{}", popup.id), &LEAD_PER_POPUP)
        .await
        .is_err()
    {
        return problem(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Please wait a little.");
    }

    let success = match popup.success_message.trim() {
        "" => "Thanks — check your inbox.".to_string(),
        message => message.to_string(),
    };

    // Already came through this popup: say the same thing rather than
    // counting them twice or telling a stranger who is on the list.
    if repo::lead_exists(&state.db, &popup.id, &email).await {
        return accepted(&success);
    }

    let source = rules::clean(field("url"), 500);
    // The member half is best-effort and comes second. A lead that was
    // recorded but whose signup failed is a person the site can still reach;
    // a signup with no lead row is a conversion nobody can attribute.
    let member_id = crate::members::create_member_from_popup(
        &state,
        &email,
        (!name.is_empty()).then_some(name.as_str()),
        &source,
    )
    .await;

    if let Err(e) = repo::record_lead(
        &state.db,
        &popup.id,
        &email,
        &name,
        &source,
        member_id.as_deref(),
    )
    .await
    {
        tracing::error!("popups: could not record a lead for {}: {}", popup.id, e.message());
        return problem(StatusCode::INTERNAL_SERVER_ERROR, "That could not be saved. Please try again.");
    }
    accepted(&success)
}

/// Budgets. Generous enough that a household behind one address can all
/// subscribe, tight enough that the endpoint is not a mailer.
const LEAD_PER_IP: crate::brute::Budget =
    crate::brute::Budget { free_retries: 10, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };
const LEAD_PER_POPUP: crate::brute::Budget =
    crate::brute::Budget { free_retries: 300, min_wait: 30_000, max_wait: 600_000, lifetime: 3600 };

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Both endpoints that write are same-origin only. The popup is drawn by a
/// script on the site's own page, so there is no case in which a legitimate
/// submission arrives from anywhere else.
fn same_site(state: &AppState, headers: &HeaderMap) -> bool {
    crate::forms::same_origin(headers, &state.site_url())
}

fn accepted(message: &str) -> Response {
    (StatusCode::OK, Json(json!({ "ok": true, "message": message }))).into_response()
}

fn problem(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "message": message }))).into_response()
}
