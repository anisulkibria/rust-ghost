//! The public half of forms: rendering a form into a page, and accepting a
//! response from anyone on the internet.
//!
//! The editor's form card stores nothing but a form's id — a placeholder
//! `<div>` in the post's HTML. [`expand_html`] swaps each placeholder for the
//! form as it is defined *now*, so editing a form updates every page it is
//! on, and a deleted or closed form stops being offered everywhere at once.
//!
//! A response then has to get through, in this order:
//!
//! 1. **A body limit** of 64 KB, applied before anything is parsed.
//! 2. **An origin check.** A browser says where a request came from; a form
//!    post driven from another site is refused. This is what stops a third
//!    party's page from making its visitors' browsers submit here.
//! 3. **Rate limits**, per address and per form (see the budgets below).
//! 4. **A signed token** minted when the form was rendered. It is bound to
//!    the one form, must be at least a few seconds old (nothing human fills
//!    a form in instantly), expires within a day, and is single-use — the
//!    database refuses a second response carrying the same token.
//! 5. **A decoy field** people cannot see and scripts fill in. A response
//!    that fills it is answered exactly like a success and thrown away, so
//!    whatever sent it learns nothing.
//! 6. **Validation** against the form's own definition, in
//!    [`ghost_core::forms::validate`]: only defined fields are read, each
//!    is checked against its type, and choices must be ones on offer.
//!
//! Once a response is accepted the form does one of three things, chosen in
//! the builder: shows its message, shows the message with a link (to a page
//! or a file), or sends the visitor on to a URL. That address is set by
//! staff and checked when saved; it is never taken from the request, so a
//! response cannot choose where its sender ends up. It is also absent from
//! the page until a response has been accepted.
//!
//! What is stored is plain text. It is escaped wherever it is shown — in the
//! admin, in the notification email — and never interpreted as markup.
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ghost_core::forms::{Field, FieldKind};
use ghost_db::repos::forms::{self as repo, Filed, Form, NewSubmission};
use serde_json::json;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::brute::{self, Budget, ClientAddr};
use crate::AppState;

const CSS: &str = include_str!("../assets/forms.css");
const JS: &str = include_str!("../assets/forms.js");

/// A response is a few short answers. Anything bigger is not a response.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// The youngest a token may be when it is spent.
const MIN_TOKEN_AGE_MS: i64 = 2_500;
/// The oldest. Long enough for a tab left open over lunch; the script on the
/// page renews an expired one without the visitor noticing.
const MAX_TOKEN_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// The decoy's name. Chosen because nothing on a real form is called this,
/// and because a browser has no saved value to autofill it with.
const DECOY: &str = "fax_number";
const TOKEN_FIELD: &str = "_token";

/// One address: ten attempts free, then a wait that doubles from a minute.
/// Attempts, not successes — a script probing the validator is paying too.
const SUBMIT_PER_IP: Budget = Budget { free_retries: 10, min_wait: 60_000, max_wait: 3_600_000, lifetime: 3600 };
/// One form, from everywhere. The address above comes from a header a
/// client behind no proxy could forge, so this is the limit that holds
/// regardless: a flood from a thousand addresses still meets it.
const SUBMIT_PER_FORM: Budget = Budget { free_retries: 300, min_wait: 30_000, max_wait: 600_000, lifetime: 3600 };
/// Notification emails per form. Responses past this are still stored; they
/// just do not each send mail, so a flood cannot be turned into a mail bomb
/// aimed at the site's own inbox.
const NOTIFY_PER_FORM: Budget = Budget { free_retries: 30, min_wait: 600_000, max_wait: 3_600_000, lifetime: 3600 };

/// The exact markup the form card writes into a post. Matched whole, so
/// nothing else in a page can be mistaken for it.
static PLACEHOLDER: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"<div class="kg-card kg-form-card" data-form-id="([0-9a-f]{24})"></div>"#).expect("placeholder regex")
});

/// The placeholder for `form_id`, as the editor and the server renderer
/// write it. `None` if the id is not an id.
pub fn placeholder(form_id: &str) -> Option<String> {
    ghost_render::lexical::form_placeholder(form_id)
}

fn asset_version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(CSS.as_bytes());
        hasher.update(JS.as_bytes());
        hex::encode(hasher.finalize())[..10].to_string()
    });
    &VERSION
}

/// Routes for the site root. Merged inside the site router so a private
/// site's access gate covers them like any other page.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/forms/api/{id}/submit/", post(submit).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)))
        .route("/forms/api/{id}/token/", get(token))
        .route("/forms/api/assets/{version}/forms.css", get(|| async { asset(CSS, "text/css; charset=utf-8") }))
        .route("/forms/api/assets/{version}/forms.js", get(|| async { asset(JS, "text/javascript; charset=utf-8") }))
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

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn sign(secret: &str, form_id: &str, issued_ms: i64, nonce: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    // The purpose tag and the form id are inside the signature, so a token
    // is good for one form and for nothing else this server signs.
    mac.update(format!("vuuut-form-v1|{form_id}|{issued_ms}|{nonce}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn mint_at(secret: &str, form_id: &str, issued_ms: i64) -> String {
    let nonce = ghost_core::ids::random_hex(16);
    format!("{issued_ms}.{nonce}.{}", sign(secret, form_id, issued_ms, &nonce))
}

#[derive(Debug, PartialEq, Eq)]
enum TokenError {
    /// Missing, malformed, or not signed by this server for this form.
    Invalid,
    /// Genuine, but spent too soon after it was issued.
    TooFast,
    Expired,
}

/// Verify a token and return its nonce.
fn verify_at(secret: &str, form_id: &str, token: &str, now: i64) -> Result<String, TokenError> {
    let mut parts = token.split('.');
    let (Some(issued), Some(nonce), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(TokenError::Invalid);
    };
    let issued_ms: i64 = issued.parse().map_err(|_| TokenError::Invalid)?;
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(TokenError::Invalid);
    }
    // Signature first, compared in constant time: the timestamps below are
    // only believed once the token is known to be ours.
    if !crate::members::secure_eq(&sign(secret, form_id, issued_ms, nonce), sig) {
        return Err(TokenError::Invalid);
    }
    let age = now.saturating_sub(issued_ms);
    if age < MIN_TOKEN_AGE_MS {
        return Err(TokenError::TooFast);
    }
    if age > MAX_TOKEN_AGE_MS {
        return Err(TokenError::Expired);
    }
    Ok(nonce.to_string())
}

// ---------------------------------------------------------------------------
// where a request came from
// ---------------------------------------------------------------------------

/// `host[:port]` of a URL or `Origin` value, lower-cased.
fn authority(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, rest)| rest)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    (!authority.is_empty() && !authority.contains('@')).then(|| authority.to_ascii_lowercase())
}

/// Whether a browser-initiated request came from this site's own pages.
///
/// A request with neither header is not from a browser — curl, a script —
/// and is let through to the token and rate-limit checks, which are what
/// deal with those. A browser always sends at least one of the two on a
/// cross-site POST, and that is the case this refuses.
///
/// `Sec-Fetch-Site: same-site` is refused as well as `cross-site`: on a
/// shared parent domain, every other tenant is "same-site".
pub fn same_origin(headers: &HeaderMap, site_url: &str) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty());
    if let Some(site) = text("sec-fetch-site") {
        if !matches!(site, "same-origin" | "none") {
            return false;
        }
    }
    let Some(origin) = text("origin") else { return true };
    let Some(origin) = authority(origin) else {
        // `Origin: null` — a sandboxed frame or a redirect chain — and
        // anything else that is not a real origin.
        return false;
    };
    let ours = [text("host").map(str::to_ascii_lowercase), text("x-forwarded-host").map(str::to_ascii_lowercase), authority(site_url)];
    ours.iter().flatten().any(|host| *host == origin)
}

/// The path of the page a response was sent from, when the browser says so
/// and it is one of ours. The query string is dropped: it can carry tokens
/// and tracking parameters that have no business being stored.
fn page_path(headers: &HeaderMap, site_url: &str) -> Option<String> {
    let referer = headers.get(header::REFERER)?.to_str().ok()?;
    let referer_host = authority(referer)?;
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).map(str::to_ascii_lowercase);
    if Some(&referer_host) != host.as_ref() && Some(referer_host) != authority(site_url) {
        return None;
    }
    let after = referer.split_once("://")?.1;
    let path = after.find('/').map(|i| &after[i..]).unwrap_or("/");
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let cleaned = ghost_core::forms::clean_bounded(path, false, 500);
    cleaned.starts_with('/').then_some(cleaned)
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn field_html(field: &Field, instance: usize) -> String {
    let id = format!("vf{instance}-{}", field.key);
    let key = esc(&field.key);
    let label = esc(&field.label);
    let required = if field.required { " required" } else { "" };
    let star = if field.required { r#"<span class="vf-req" aria-hidden="true">*</span>"# } else { "" };
    let help = if field.help.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="vf-help" id="{id}-help">{}</p>"#, esc(&field.help))
    };
    let described = if field.help.is_empty() { String::new() } else { format!(r#" aria-describedby="{id}-help""#) };
    let placeholder = if field.placeholder.is_empty() { String::new() } else { format!(r#" placeholder="{}""#, esc(&field.placeholder)) };
    let error = r#"<p class="vf-error" role="alert" hidden></p>"#;

    let typed = |kind: &str, extra: &str, max: usize| {
        format!(
            r#"<div class="vf-field" data-vf-field="{key}"><label class="vf-label" for="{id}">{label}{star}</label>
<input class="vf-input" type="{kind}" id="{id}" name="{key}" maxlength="{max}"{extra}{placeholder}{required}{described}>{help}{error}</div>"#
        )
    };

    match field.kind {
        FieldKind::Text => typed("text", "", ghost_core::forms::MAX_SHORT),
        FieldKind::Email => typed("email", r#" autocomplete="email" inputmode="email""#, ghost_core::forms::MAX_EMAIL),
        FieldKind::Phone => typed("tel", r#" autocomplete="tel" inputmode="tel""#, 32),
        FieldKind::Url => typed("url", r#" inputmode="url""#, ghost_core::forms::MAX_URL),
        FieldKind::Number => typed("text", r#" inputmode="decimal""#, 32),
        FieldKind::Date => typed("date", "", 10),
        FieldKind::Textarea => format!(
            r#"<div class="vf-field" data-vf-field="{key}"><label class="vf-label" for="{id}">{label}{star}</label>
<textarea class="vf-input" id="{id}" name="{key}" rows="5" maxlength="{max}"{placeholder}{required}{described}></textarea>{help}{error}</div>"#,
            max = ghost_core::forms::MAX_LONG,
        ),
        FieldKind::Select => {
            let options: String =
                field.options.iter().map(|o| format!(r#"<option value="{v}">{v}</option>"#, v = esc(o))).collect();
            format!(
                r#"<div class="vf-field" data-vf-field="{key}"><label class="vf-label" for="{id}">{label}{star}</label>
<select class="vf-input" id="{id}" name="{key}"{required}{described}><option value="">Choose…</option>{options}</select>{help}{error}</div>"#
            )
        }
        FieldKind::Radio | FieldKind::Checkboxes => {
            let kind = if field.kind == FieldKind::Radio { "radio" } else { "checkbox" };
            // `required` on a radio group means "one of these"; on a set of
            // checkboxes it would mean "all of these", so it is left to the
            // server there.
            let each_required = if field.kind == FieldKind::Radio { required } else { "" };
            let choices: String = field
                .options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    format!(
                        r#"<label class="vf-choice" for="{id}-{i}"><input type="{kind}" id="{id}-{i}" name="{key}" value="{v}"{each_required}><span>{v}</span></label>"#,
                        v = esc(o),
                    )
                })
                .collect();
            format!(
                r#"<fieldset class="vf-field" data-vf-field="{key}"{described}><legend class="vf-label">{label}{star}</legend>
<div class="vf-choices">{choices}</div>{help}{error}</fieldset>"#
            )
        }
        FieldKind::Consent => format!(
            r#"<div class="vf-field" data-vf-field="{key}"><label class="vf-choice" for="{id}"><input type="checkbox" id="{id}" name="{key}" value="1"{required}{described}><span>{label}{star}</span></label>{help}{error}</div>"#
        ),
    }
}

fn submit_url(form_id: &str) -> String {
    ghost_core::url::base(&format!("/forms/api/{form_id}/submit/"))
}

fn token_url(form_id: &str) -> String {
    ghost_core::url::base(&format!("/forms/api/{form_id}/token/"))
}

/// One form as markup. `token` is the freshly minted token it carries.
fn form_html(form: &Form, token: &str, instance: usize) -> String {
    if !form.is_active() || form.fields.is_empty() {
        return r#"<div class="kg-card kg-form-card"><p class="vf-closed">This form is no longer accepting responses.</p></div>"#.to_string();
    }
    let fields: String = form.fields.iter().map(|f| field_html(f, instance)).collect();
    let label = if form.submit_label.trim().is_empty() { ghost_core::forms::DEFAULT_SUBMIT_LABEL } else { form.submit_label.as_str() };
    format!(
        r#"<div class="kg-card kg-form-card"><form class="vf-form" method="post" action="{action}" accept-charset="utf-8" data-vf-form="{id}" data-vf-token-url="{token_url}">
<input type="hidden" name="{TOKEN_FIELD}" value="{token}">
<div class="vf-hp" aria-hidden="true" style="position:absolute;left:-10000px;width:1px;height:1px;overflow:hidden"><label for="vf{instance}-hp">Leave this field empty</label><input type="text" id="vf{instance}-hp" name="{DECOY}" tabindex="-1" autocomplete="off" value=""></div>
{fields}
<div class="vf-actions"><button class="vf-submit" type="submit">{label}</button></div>
<p class="vf-status" role="status" aria-live="polite" hidden></p>
</form></div>"#,
        action = esc(&submit_url(&form.id)),
        id = esc(&form.id),
        token_url = esc(&token_url(&form.id)),
        token = esc(token),
        label = esc(label),
    )
}

/// Replace every form-card placeholder in a rendered page with its form.
///
/// A page with no placeholder is returned untouched and costs one substring
/// search. A placeholder naming a form that no longer exists renders as
/// nothing at all.
pub async fn expand_html(state: &AppState, html: String) -> String {
    if !html.contains("kg-form-card") || !PLACEHOLDER.is_match(&html) {
        return html;
    }
    let Ok(secret) = repo::signing_secret(&state.db).await else {
        tracing::error!("forms: no signing secret; forms are not rendered");
        return PLACEHOLDER.replace_all(&html, "").into_owned();
    };

    let mut forms: HashMap<String, Option<Form>> = HashMap::new();
    for caps in PLACEHOLDER.captures_iter(&html) {
        let id = caps[1].to_string();
        if !forms.contains_key(&id) {
            let form = repo::find(&state.db, &id).await.ok().flatten();
            forms.insert(id, form);
        }
    }

    let issued = now_ms();
    let mut instance = 0usize;
    let mut assets_added = false;
    PLACEHOLDER
        .replace_all(&html, |caps: &regex::Captures<'_>| {
            let Some(Some(form)) = forms.get(&caps[1]) else { return String::new() };
            instance += 1;
            // Each copy of a form gets a token of its own, so two copies on
            // one page can both be sent.
            let mut out = form_html(form, &mint_at(&secret, &form.id, issued), instance);
            if !assets_added {
                assets_added = true;
                let version = asset_version();
                out.push_str(&format!(
                    r#"<link rel="stylesheet" href="{css}"><script defer src="{js}"></script>"#,
                    css = esc(&ghost_core::url::base(&format!("/forms/api/assets/{version}/forms.css"))),
                    js = esc(&ghost_core::url::base(&format!("/forms/api/assets/{version}/forms.js"))),
                ));
            }
            out
        })
        .into_owned()
}

// ---------------------------------------------------------------------------
// answering
// ---------------------------------------------------------------------------

fn wants_json(headers: &HeaderMap) -> bool {
    headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|accept| accept.contains("application/json"))
}

/// A complete page for a browser that posted the form without the script.
/// It loads nothing and runs nothing, and says so in its own policy.
fn plain_page(status: StatusCode, title: &str, message: &str, details: &[String], link: Option<(&String, &String)>) -> Response {
    let list = if details.is_empty() {
        String::new()
    } else {
        format!("<ul>{}</ul>", details.iter().map(|d| format!("<li>{}</li>", esc(d))).collect::<String>())
    };
    let link = link
        .map(|(url, label)| format!(r#"<p><a class="go" href="{}" rel="noopener">{}</a></p>"#, esc(url), esc(label)))
        .unwrap_or_default();
    let back = if status.is_success() {
        format!(r#"<p><a href="{}">Return to the site</a></p>"#, esc(&ghost_core::url::base("/")))
    } else {
        r#"<p>Use your browser’s back button to return to the form — what you typed will still be there.</p>"#.to_string()
    };
    let body = format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title>
<style>body{{font-family:system-ui,-apple-system,"Segoe UI",sans-serif;line-height:1.55;color:#15171a;background:#fff;margin:0;padding:12vh 24px;}}main{{max-width:520px;margin:0 auto;}}h1{{font-size:1.6rem;margin:0 0 .5em;}}a{{color:inherit;}}a.go{{display:inline-block;padding:.6em 1.2em;border-radius:8px;background:#15171a;color:#fff;text-decoration:none;font-weight:600;}}@media(prefers-color-scheme:dark){{body{{color:#f4f5f6;background:#15171a;}}}}</style></head>
<body><main><h1>{title}</h1><p>{message}</p>{link}{list}{back}</main></body></html>"#,
        title = esc(title),
        message = esc(message),
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}

/// A refusal, in whichever shape the caller asked for.
fn refuse(headers: &HeaderMap, status: StatusCode, code: &str, message: &str) -> Response {
    if wants_json(headers) {
        return (
            status,
            [(header::CACHE_CONTROL, "no-store"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")],
            Json(json!({ "ok": false, "code": code, "message": message })),
        )
            .into_response();
    }
    plain_page(status, "The form was not sent", message, &[], None)
}

/// What the visitor is given once a response is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Message(String),
    Link { message: String, url: String, label: String },
    Redirect { message: String, url: String },
}

/// The outcome a form is configured for.
///
/// The stored address is checked again here rather than trusted: a row
/// written by an older build, or edited by hand, must not be able to turn
/// this endpoint into a redirect to somewhere the check would have refused.
/// Anything that fails falls back to the plain message.
fn outcome(form: &Form) -> Outcome {
    let message = success_message(form).to_string();
    let url = match ghost_core::forms::normalize_success_url(&form.success_url) {
        Ok(url) if !url.is_empty() => ghost_core::url::base(&url),
        _ => return Outcome::Message(message),
    };
    match ghost_core::forms::SuccessAction::parse(&form.success_action) {
        ghost_core::forms::SuccessAction::Message => Outcome::Message(message),
        ghost_core::forms::SuccessAction::Redirect => Outcome::Redirect { message, url },
        ghost_core::forms::SuccessAction::Link => {
            let label = match ghost_core::forms::clean_bounded(&form.success_link_label, false, ghost_core::forms::MAX_LINK_LABEL) {
                label if label.is_empty() => ghost_core::forms::DEFAULT_LINK_LABEL.to_string(),
                label => label,
            };
            Outcome::Link { message, url, label }
        }
    }
}

fn accepted(headers: &HeaderMap, outcome: &Outcome) -> Response {
    if wants_json(headers) {
        let body = match outcome {
            Outcome::Message(message) => json!({ "ok": true, "message": message }),
            Outcome::Link { message, url, label } => json!({ "ok": true, "message": message, "link": { "url": url, "label": label } }),
            Outcome::Redirect { message, url } => json!({ "ok": true, "message": message, "redirect": url }),
        };
        return (StatusCode::OK, [(header::CACHE_CONTROL, "no-store"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], Json(body))
            .into_response();
    }
    // Without the script the browser has navigated here, so a redirect is a
    // real one and a link is a link on the page.
    match outcome {
        Outcome::Message(message) => plain_page(StatusCode::OK, "Sent", message, &[], None),
        Outcome::Link { message, url, label } => plain_page(StatusCode::OK, "Sent", message, &[], Some((url, label))),
        Outcome::Redirect { url, .. } => match url.parse::<axum::http::HeaderValue>() {
            Ok(location) => {
                (StatusCode::SEE_OTHER, [(header::LOCATION, location), (header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-store"))])
                    .into_response()
            }
            Err(_) => plain_page(StatusCode::OK, "Sent", success_message_text(outcome), &[], None),
        },
    }
}

fn success_message_text(outcome: &Outcome) -> &str {
    match outcome {
        Outcome::Message(message) | Outcome::Link { message, .. } | Outcome::Redirect { message, .. } => message,
    }
}

fn success_message(form: &Form) -> &str {
    if form.success_message.trim().is_empty() {
        ghost_core::forms::DEFAULT_SUCCESS_MESSAGE
    } else {
        form.success_message.as_str()
    }
}

/// The caller's address, if it really is one. The proxy headers are text a
/// client can set, so anything that does not parse is discarded rather than
/// stored or used as a key.
fn caller_ip(headers: &HeaderMap, peer: Option<std::net::IpAddr>) -> Option<String> {
    brute::client_ip(headers, peer).parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
}

/// `GET /forms/api/{id}/token/` — a fresh token for a form already on the
/// page, for the script to use when the one it was rendered with has expired
/// or been spent.
async fn token(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if !same_origin(&headers, &state.site_url()) {
        return refuse(&headers, StatusCode::FORBIDDEN, "origin", "This request did not come from this site.");
    }
    let form = match repo::find(&state.db, &id).await {
        Ok(Some(form)) if form.is_active() => form,
        _ => return refuse(&headers, StatusCode::NOT_FOUND, "closed", "This form is not accepting responses."),
    };
    let Ok(secret) = repo::signing_secret(&state.db).await else {
        return refuse(&headers, StatusCode::INTERNAL_SERVER_ERROR, "server", "The form is unavailable right now.");
    };
    (
        [(header::CACHE_CONTROL, "no-store"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")],
        Json(json!({ "token": mint_at(&secret, &form.id, now_ms()), "wait_ms": MIN_TOKEN_AGE_MS })),
    )
        .into_response()
}

/// `POST /forms/api/{id}/submit/`
async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ClientAddr(peer): ClientAddr,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let site_url = state.site_url();
    if !same_origin(&headers, &site_url) {
        return refuse(&headers, StatusCode::FORBIDDEN, "origin", "This form can only be sent from this site.");
    }
    // Only the encoding a form itself produces. Refusing the rest also
    // refuses the "simple" cross-site content types a browser will send
    // without asking first.
    let is_form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.trim().to_ascii_lowercase().starts_with("application/x-www-form-urlencoded"));
    if !is_form {
        return refuse(&headers, StatusCode::UNSUPPORTED_MEDIA_TYPE, "type", "The form was sent in a format this site does not accept.");
    }

    let ip = caller_ip(&headers, peer);
    let ip_key = format!("form_submit:{}", ip.as_deref().unwrap_or("unknown"));
    if let Err(blocked) = brute::check(&state.db, &ip_key, &SUBMIT_PER_IP).await {
        return too_many(&headers, &blocked);
    }

    let form = match repo::find(&state.db, &id).await {
        Ok(Some(form)) if form.is_active() && !form.fields.is_empty() => form,
        Ok(_) => return refuse(&headers, StatusCode::NOT_FOUND, "closed", "This form is not accepting responses."),
        Err(e) => {
            tracing::error!("forms: loading {id} failed: {}", e.message());
            return refuse(&headers, StatusCode::INTERNAL_SERVER_ERROR, "server", "The form could not be sent. Please try again.");
        }
    };
    if let Err(blocked) = brute::check(&state.db, &format!("form_submit_all:{}", form.id), &SUBMIT_PER_FORM).await {
        return too_many(&headers, &blocked);
    }

    let mut input: HashMap<String, Vec<String>> = HashMap::new();
    for (key, value) in form_urlencoded::parse(&body) {
        input.entry(key.into_owned()).or_default().push(value.into_owned());
    }
    let first = |name: &str| input.get(name).and_then(|v| v.first()).map(String::as_str).unwrap_or("");

    let secret = match repo::signing_secret(&state.db).await {
        Ok(secret) => secret,
        Err(e) => {
            tracing::error!("forms: {}", e.message());
            return refuse(&headers, StatusCode::INTERNAL_SERVER_ERROR, "server", "The form could not be sent. Please try again.");
        }
    };
    let nonce = match verify_at(&secret, &form.id, first(TOKEN_FIELD), now_ms()) {
        Ok(nonce) => nonce,
        // Sent within a couple of seconds of the page loading. Told to try
        // again rather than told why.
        Err(TokenError::TooFast) => {
            return refuse(&headers, StatusCode::BAD_REQUEST, "fast", "That was quick — please wait a moment and send the form again.")
        }
        Err(TokenError::Expired | TokenError::Invalid) => {
            return refuse(&headers, StatusCode::BAD_REQUEST, "token", "This form has expired. Reload the page and try again.")
        }
    };

    // Something filled in the field nobody can see. Answer as though it
    // worked — a refusal would tell the script which field gave it away.
    if !first(DECOY).is_empty() {
        tracing::info!("forms: discarded a response to {} that filled the decoy field", form.id);
        return accepted(&headers, &outcome(&form));
    }

    let answers = match ghost_core::forms::validate(&form.fields, &input) {
        Ok(answers) => answers,
        Err(errors) => {
            if wants_json(&headers) {
                let by_field: serde_json::Map<String, serde_json::Value> =
                    errors.into_iter().map(|(key, message)| (key, serde_json::Value::String(message))).collect();
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    [(header::CACHE_CONTROL, "no-store"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")],
                    Json(json!({ "ok": false, "code": "invalid", "message": "Please check the highlighted fields.", "errors": by_field })),
                )
                    .into_response();
            }
            let details: Vec<String> = errors
                .iter()
                .map(|(key, message)| {
                    let label = form.fields.iter().find(|f| f.key == *key).map(|f| f.label.as_str()).unwrap_or("Form");
                    format!("{label}: {message}")
                })
                .collect();
            return plain_page(StatusCode::UNPROCESSABLE_ENTITY, "Please check the form", "Some answers need another look:", &details, None);
        }
    };

    let config = repo::config(&state.db).await.unwrap_or(repo::Config {
        notify_enabled: false,
        notify_emails: String::new(),
        // If the settings cannot be read, err towards keeping less.
        store_ip: false,
        retention_days: 0,
    });
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ghost_core::forms::clean_bounded(ua, false, 300))
        .filter(|ua| !ua.is_empty());
    let page = page_path(&headers, &site_url);
    let member = crate::members::current_member(&state, &headers).await;

    let filed = repo::file(
        &state.db,
        &NewSubmission {
            form_id: &form.id,
            form_name: &form.name,
            answers: &answers,
            nonce: &nonce,
            ip: ip.as_deref().filter(|_| config.store_ip),
            user_agent: user_agent.as_deref(),
            page_path: page.as_deref(),
            member_id: member.as_ref().map(|m| m.id.as_str()),
        },
    )
    .await;
    let submission_id = match filed {
        Ok(Filed::Stored(id)) => id,
        // The same token twice: a double-click, a resent request, or a
        // replay. The first one was filed; this one changes nothing.
        Ok(Filed::Replayed) => {
            return refuse(&headers, StatusCode::CONFLICT, "token", "This form was already sent. Reload the page to send another.")
        }
        Err(e) => {
            tracing::error!("forms: storing a response to {} failed: {}", form.id, e.message());
            return refuse(&headers, StatusCode::INTERNAL_SERVER_ERROR, "server", "The form could not be sent. Please try again.");
        }
    };

    // Neither of these is the visitor's concern, so neither delays or can
    // fail the answer they get.
    let background = state.clone();
    let result = outcome(&form);
    tokio::spawn(async move {
        if config.retention_days > 0 {
            let _ = repo::purge_older_than(&background.db, config.retention_days).await;
        }
        notify(&background, &form, &config, &submission_id, &answers).await;
    });

    accepted(&headers, &result)
}

fn too_many(headers: &HeaderMap, blocked: &brute::Blocked) -> Response {
    let mut response = refuse(headers, StatusCode::TOO_MANY_REQUESTS, "rate", "Too many attempts. Please wait a little and try again.");
    if let Ok(value) = blocked.retry_after_secs().to_string().parse() {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

// ---------------------------------------------------------------------------
// telling staff
// ---------------------------------------------------------------------------

/// Who hears about a response: the form's own list, else the site-wide one,
/// else the owner.
async fn recipients(state: &AppState, form: &Form, config: &repo::Config) -> Vec<String> {
    for list in [&form.notify_emails, &config.notify_emails] {
        let (good, _) = ghost_core::forms::parse_recipients(list);
        if !good.is_empty() {
            return good;
        }
    }
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT u.email FROM users u JOIN roles_users ru ON ru.user_id = u.id JOIN roles r ON r.id = ru.role_id \
         WHERE r.name = 'Owner' LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    owner.filter(|email| ghost_core::forms::is_email(&email.to_lowercase())).into_iter().collect()
}

/// The notification, as `(subject, html, text)`. Every value a visitor
/// supplied is escaped in the HTML part, and the subject is built only from
/// the form's name — never from an answer — with control characters removed,
/// so a response cannot add headers or recipients to the message.
fn notification(site_title: &str, admin_url: &str, form: &Form, submission_id: &str, answers: &[ghost_core::forms::Answer]) -> (String, String, String) {
    let name = ghost_core::forms::clean_bounded(&form.name, false, 100);
    let subject = format!("New response: {name}");
    let link = format!("{admin_url}/forms/responses/{submission_id}/");

    let mut text = format!("A new response was sent through “{name}”.\n\n");
    let mut rows = String::new();
    for answer in answers {
        text.push_str(&format!("{}:\n{}\n\n", answer.label, answer.value));
        rows.push_str(&format!(
            r#"<tr><td style="padding:10px 0;border-top:1px solid #e6e8eb;vertical-align:top"><div style="font-size:12px;color:#738a94;text-transform:uppercase;letter-spacing:.4px">{label}</div><div style="font-size:15px;color:#15171a;white-space:pre-wrap;word-break:break-word">{value}</div></td></tr>"#,
            label = esc(&answer.label),
            value = esc(&answer.value),
        ));
    }
    text.push_str(&format!("Open it in the admin: {link}\n"));

    let html = format!(
        r#"<!DOCTYPE html><html><body style="margin:0;padding:24px;background:#f4f5f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:560px;margin:0 auto;background:#ffffff;border-radius:10px;padding:28px">
<tr><td><div style="font-size:13px;color:#738a94">{site}</div><h1 style="font-size:20px;margin:6px 0 16px;color:#15171a">New response: {name}</h1></td></tr>
{rows}
<tr><td style="padding-top:20px;border-top:1px solid #e6e8eb"><a href="{link}" style="display:inline-block;padding:10px 18px;background:#15171a;color:#ffffff;text-decoration:none;border-radius:6px;font-size:14px">Open in admin</a>
<p style="font-size:12px;color:#738a94;margin:16px 0 0">Sent by a visitor through a form on your site. Treat links and requests in it with the same care as any message from a stranger.</p></td></tr>
</table></body></html>"#,
        site = esc(site_title),
        name = esc(&name),
        link = esc(&link),
    );
    (subject, html, text)
}

async fn notify(state: &AppState, form: &Form, config: &repo::Config, submission_id: &str, answers: &[ghost_core::forms::Answer]) {
    if !config.notify_enabled {
        return;
    }
    if brute::check(&state.db, &format!("form_notify:{}", form.id), &NOTIFY_PER_FORM).await.is_err() {
        tracing::warn!("forms: notification for {} skipped — too many in the last hour", form.id);
        return;
    }
    let to = recipients(state, form, config).await;
    if to.is_empty() {
        return;
    }
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &site_title, &state.site_url());
    let (subject, html, text) = notification(&site_title, &state.admin_url(), form, submission_id, answers);
    for address in to {
        let msg = ghost_mail::Message { to: address.clone(), subject: subject.clone(), html: html.clone(), text: text.clone() };
        if let Err(e) = ghost_mail::send(&cfg, &msg).await {
            tracing::warn!("forms: notifying {address} failed: {}", e.message());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghost_core::forms::Answer;

    const FORM: &str = "0123456789abcdef01234567";
    const OTHER: &str = "76543210fedcba9876543210";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
        }
        map
    }

    fn form(fields: Vec<Field>) -> Form {
        Form {
            id: FORM.into(),
            name: "Contact".into(),
            status: "active".into(),
            fields,
            submit_label: "Send".into(),
            success_message: String::new(),
            notify_emails: String::new(),
            success_action: "message".into(),
            success_url: String::new(),
            success_link_label: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn what_follows_a_response_is_what_the_form_is_set_to_and_nothing_unsafe() {
        let mut f = form(vec![]);
        f.success_message = "Thanks".into();
        assert_eq!(outcome(&f), Outcome::Message("Thanks".into()));

        // an action with no address is just the message
        f.success_action = "redirect".into();
        assert_eq!(outcome(&f), Outcome::Message("Thanks".into()));

        f.success_url = "/thanks/".into();
        assert_eq!(outcome(&f), Outcome::Redirect { message: "Thanks".into(), url: "/thanks/".into() });

        f.success_action = "link".into();
        f.success_url = "https://example.com/guide.pdf".into();
        assert_eq!(
            outcome(&f),
            Outcome::Link { message: "Thanks".into(), url: "https://example.com/guide.pdf".into(), label: "Open link".into() }
        );
        f.success_link_label = "Download\r\n the guide".into();
        assert!(matches!(outcome(&f), Outcome::Link { label, .. } if label == "Download the guide"));

        // a stored address the check refuses is never acted on, whatever the action
        for bad in ["javascript:alert(1)", "//evil.test/", "data:text/html,x", "https://ok.test/\r\nSet-Cookie: a=b"] {
            f.success_url = bad.into();
            for action in ["link", "redirect"] {
                f.success_action = action.into();
                assert_eq!(outcome(&f), Outcome::Message("Thanks".into()), "{action} {bad:?}");
            }
        }
    }

    #[test]
    fn a_browser_without_the_script_is_redirected_or_shown_the_link() {
        let plain = HeaderMap::new();
        let redirect = accepted(&plain, &Outcome::Redirect { message: "Thanks".into(), url: "/thanks/".into() });
        assert_eq!(redirect.status(), StatusCode::SEE_OTHER);
        assert_eq!(redirect.headers().get(header::LOCATION).unwrap(), "/thanks/");

        let json = headers(&[("accept", "application/json")]);
        // the script is told where to go rather than being redirected itself
        assert_eq!(accepted(&json, &Outcome::Redirect { message: "Thanks".into(), url: "/thanks/".into() }).status(), StatusCode::OK);
    }

    #[test]
    fn a_token_is_good_for_its_own_form_inside_its_window_and_nowhere_else() {
        let token = mint_at("secret", FORM, 1_000_000);
        let good = 1_000_000 + MIN_TOKEN_AGE_MS;
        assert!(verify_at("secret", FORM, &token, good).is_ok());
        // another form, another key
        assert_eq!(verify_at("secret", OTHER, &token, good), Err(TokenError::Invalid));
        assert_eq!(verify_at("other-secret", FORM, &token, good), Err(TokenError::Invalid));
        // too soon, too late
        assert_eq!(verify_at("secret", FORM, &token, 1_000_000 + MIN_TOKEN_AGE_MS - 1), Err(TokenError::TooFast));
        assert_eq!(verify_at("secret", FORM, &token, 1_000_000 + MAX_TOKEN_AGE_MS + 1), Err(TokenError::Expired));
    }

    #[test]
    fn a_tampered_or_malformed_token_is_refused() {
        let token = mint_at("secret", FORM, 1_000_000);
        let now = 1_000_000 + MIN_TOKEN_AGE_MS;
        let parts: Vec<&str> = token.split('.').collect();
        // back-dating the timestamp to dodge the minimum age breaks the signature
        let backdated = format!("{}.{}.{}", 1, parts[1], parts[2]);
        assert_eq!(verify_at("secret", FORM, &backdated, now), Err(TokenError::Invalid));
        // so does swapping the nonce to dodge the single-use rule
        let renonced = format!("{}.{}.{}", parts[0], "0".repeat(32), parts[2]);
        assert_eq!(verify_at("secret", FORM, &renonced, now), Err(TokenError::Invalid));
        for bad in ["", "a.b", "1.2.3.4", "x.y.z", &format!("{}.short.{}", parts[0], parts[2]), &format!("{token}.extra")] {
            assert_eq!(verify_at("secret", FORM, bad, now), Err(TokenError::Invalid), "{bad:?}");
        }
    }

    #[test]
    fn every_token_has_its_own_nonce() {
        let a = mint_at("secret", FORM, 5);
        let b = mint_at("secret", FORM, 5);
        assert_ne!(a, b);
    }

    #[test]
    fn requests_from_other_sites_are_refused() {
        let site = "https://example.com";
        // not a browser: nothing to check here
        assert!(same_origin(&headers(&[]), site));
        // this site's own page
        assert!(same_origin(&headers(&[("origin", "https://example.com"), ("host", "example.com"), ("sec-fetch-site", "same-origin")]), site));
        // behind a proxy that rewrites Host, the configured URL still matches
        assert!(same_origin(&headers(&[("origin", "https://example.com"), ("host", "127.0.0.1:2368")]), site));
        // another site, however it describes itself
        assert!(!same_origin(&headers(&[("origin", "https://evil.test"), ("host", "example.com")]), site));
        assert!(!same_origin(&headers(&[("sec-fetch-site", "cross-site"), ("host", "example.com")]), site));
        // a sibling subdomain is another tenant, not this site
        assert!(!same_origin(&headers(&[("sec-fetch-site", "same-site"), ("origin", "https://example.com"), ("host", "example.com")]), site));
        assert!(!same_origin(&headers(&[("origin", "https://other.example.com"), ("host", "example.com")]), site));
        // a look-alike that merely contains the host
        assert!(!same_origin(&headers(&[("origin", "https://example.com.evil.test"), ("host", "example.com")]), site));
        assert!(!same_origin(&headers(&[("origin", "https://evil.test/?example.com"), ("host", "example.com")]), site));
        // an opaque origin
        assert!(!same_origin(&headers(&[("origin", "null"), ("host", "example.com")]), site));
    }

    #[test]
    fn the_stored_page_is_a_path_on_this_site_without_its_query() {
        let site = "https://example.com";
        let from = |referer: &str| page_path(&headers(&[("referer", referer), ("host", "example.com")]), site);
        assert_eq!(from("https://example.com/contact/?token=abc#top").as_deref(), Some("/contact/"));
        assert_eq!(from("https://example.com").as_deref(), Some("/"));
        assert_eq!(from("https://evil.test/contact/"), None);
        assert_eq!(page_path(&headers(&[]), site), None);
    }

    #[test]
    fn everything_a_form_definition_contributes_to_the_page_is_escaped() {
        let hostile = r#""><script>alert(1)</script><img src=x onerror='x'>"#;
        let fields = vec![
            Field { key: "f_00000001".into(), kind: FieldKind::Text, label: hostile.into(), required: true, placeholder: hostile.into(), help: hostile.into(), options: vec![] },
            Field { key: "f_00000002".into(), kind: FieldKind::Select, label: hostile.into(), required: false, placeholder: String::new(), help: String::new(), options: vec![hostile.into()] },
            Field { key: "f_00000003".into(), kind: FieldKind::Checkboxes, label: hostile.into(), required: false, placeholder: String::new(), help: String::new(), options: vec![hostile.into()] },
            Field { key: "f_00000004".into(), kind: FieldKind::Consent, label: hostile.into(), required: true, placeholder: String::new(), help: String::new(), options: vec![] },
            Field { key: "f_00000005".into(), kind: FieldKind::Textarea, label: hostile.into(), required: false, placeholder: hostile.into(), help: String::new(), options: vec![] },
        ];
        let mut f = form(fields);
        f.submit_label = hostile.into();
        let html = form_html(&f, "1.abc.def", 1);
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("onerror='"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        // and it carries the pieces the endpoint depends on
        assert!(html.contains(r#"name="_token" value="1.abc.def""#));
        assert!(html.contains(r#"name="fax_number""#));
        assert!(html.contains(&format!("/forms/api/{FORM}/submit/")));
    }

    #[test]
    fn a_closed_form_offers_no_inputs() {
        let mut f = form(ghost_core::forms::default_fields());
        f.status = "closed".into();
        let html = form_html(&f, "t", 1);
        assert!(!html.contains("<form"));
        assert!(!html.contains("<input"));
        assert!(html.contains("no longer accepting"));
    }

    #[test]
    fn only_the_exact_placeholder_is_a_placeholder() {
        assert_eq!(placeholder(FORM).unwrap(), format!(r#"<div class="kg-card kg-form-card" data-form-id="{FORM}"></div>"#));
        assert!(PLACEHOLDER.is_match(&placeholder(FORM).unwrap()));
        assert!(placeholder("nope").is_none());
        // upper-case hex is not what the editor writes, and would not match
        assert!(placeholder("0123456789ABCDEF01234567").is_none());
        assert!(placeholder(r#"0123456789abcdef0123456"><script>"#).is_none());
        for not in [
            r#"<div class="kg-card kg-form-card" data-form-id="short"></div>"#,
            r#"<div class="kg-card kg-form-card" data-form-id="0123456789abcdef01234567" onclick="x"></div>"#,
            r#"<div class="kg-form-card" data-form-id="0123456789abcdef01234567"></div>"#,
        ] {
            assert!(!PLACEHOLDER.is_match(not), "{not}");
        }
    }

    #[test]
    fn the_notification_escapes_answers_and_keeps_them_out_of_the_subject() {
        let mut f = form(vec![]);
        f.name = "Contact\r\nBcc: victim@example.com".into();
        let answers = vec![Answer {
            key: "f_00000001".into(),
            label: "Message <b>".into(),
            kind: "textarea".into(),
            value: "<script>alert(1)</script>\nSubject: hijack".into(),
        }];
        let (subject, html, text) = notification("Site", "https://example.com/ghost", &f, "abc", &answers);
        assert!(!subject.contains('\r') && !subject.contains('\n'), "{subject:?}");
        assert!(!subject.contains("hijack"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("Message &lt;b&gt;"));
        assert!(text.contains("https://example.com/ghost/forms/responses/abc/"));
    }
}
