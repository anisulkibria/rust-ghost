//! Private site: the whole frontend behind a shared access code.
//!
//! Ghost's `is_private` mode (core/shared/private-site-access). A visitor
//! without a valid `ghost-private` cookie is bounced to `/private/`, where one
//! access code — not an account — lets them in.
//!
//! The cookie carries a salt and `sha256(code + salt)` rather than the code
//! itself, which is what makes changing the code lock everyone out again: the
//! stored hash no longer matches a hash of the *current* code. (Ghost encodes
//! those two values as base64 JSON; here they are a dotted pair, since nothing
//! outside the server reads the cookie.)
use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use ghost_api::AppState;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::render::{member_context, render_template, RenderCtx};

const COOKIE: &str = "ghost-private";
/// Ghost keeps the private session for 30 days.
const MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;

pub fn is_private(state: &AppState) -> bool {
    state.settings.get_bool("is_private")
}

/// The configured access code. An empty one means nobody can get in, so the
/// gate treats it as "no code set" rather than "everyone matches".
fn access_code(state: &AppState) -> String {
    state.settings.get_str("password").unwrap_or_default()
}

fn hash_code(code: &str, salt: &str) -> String {
    hex::encode(Sha256::digest(format!("{code}{salt}").as_bytes()))
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")))
        .map(str::to_string)
}

/// Whether this request already carries a cookie matching the *current* code.
pub fn has_access(state: &AppState, headers: &HeaderMap) -> bool {
    let code = access_code(state);
    if code.trim().is_empty() {
        return false;
    }
    let Some(raw) = cookie_value(headers) else { return false };
    let Some((salt, token)) = raw.split_once('.') else { return false };
    if salt.is_empty() || token.is_empty() {
        return false;
    }
    hash_code(&code, salt) == token
}

/// Mint the cookie for a correct code. `None` when the code is wrong, so the
/// caller renders the error instead.
fn grant(state: &AppState, submitted: &str) -> Option<String> {
    let code = access_code(state);
    if code.trim().is_empty() || submitted.trim().is_empty() || code != submitted {
        return None;
    }
    let salt = ghost_core::ids::random_hex(8);
    let token = hash_code(&code, &salt);
    Some(format!(
        "{COOKIE}={salt}.{token}; Path={}/; HttpOnly; SameSite=Lax; Max-Age={MAX_AGE_SECS}",
        ghost_core::url::subdir(),
    ))
}

/// Keep a redirect target on this site: an absolute or scheme-relative URL
/// could send someone straight off it after they type the code.
fn safe_redirect(raw: Option<&String>) -> String {
    let target = raw.map(String::as_str).unwrap_or("/");
    let decoded = percent_encoding::percent_decode_str(target).decode_utf8_lossy().to_string();
    if decoded.starts_with('/') && !decoded.starts_with("//") {
        decoded
    } else {
        "/".to_string()
    }
}

/// The robots.txt a private site serves: nothing is indexable.
const PRIVATE_ROBOTS: &str = "User-agent: *\nDisallow: /\n";

/// Paths served straight off disk, which the gate lets through.
const STATIC_PREFIXES: &[&str] = &["/assets/", "/public/", "/content/"];

/// What the gate decided about a request.
pub enum Gate {
    /// Not private, or already signed in — carry on.
    Allow,
    /// Serve this instead (the redirect to `/private/`, or private robots.txt).
    Respond(Response),
    /// Handle it as this path instead — the secret feed URL, which is really
    /// `/rss/` wearing the `public_hash` as a password.
    Rewrite(String),
}

/// The check every public request goes through.
///
/// Mirrors Ghost's `filterPrivateRoutes`: `/private/` and `robots.txt` stay
/// reachable, the `public_hash` RSS URL is the one feed that survives, and
/// everything else needs the cookie.
pub fn gate(state: &AppState, uri: &Uri, headers: &HeaderMap) -> Gate {
    if !is_private(state) {
        return Gate::Allow;
    }
    let path = ghost_core::url::strip(uri.path());

    // The login page itself, or it could never be reached.
    if path == "/private/" || path == "/private" {
        return Gate::Allow;
    }

    // Static files stay open, which is Ghost's own order: favicon, public
    // files, images, media and theme assets are all mounted *before*
    // `filterPrivateRoutes` in its site app. A theme's `private.hbs` needs its
    // stylesheet to render, and none of these leak a post's text.
    if STATIC_PREFIXES.iter().any(|p| path.starts_with(p)) || path == "/favicon.ico" {
        return Gate::Allow;
    }

    // A private site tells crawlers to go away, whoever is asking.
    if path == "/robots.txt" {
        return Gate::Respond(
            ([(header::CONTENT_TYPE, "text/plain; charset=UTF-8")], PRIVATE_ROBOTS).into_response(),
        );
    }

    if has_access(state, headers) {
        // Ghost drops the public feed on a private site — it would hand out
        // the posts the access code is there to protect.
        if path.ends_with("/rss/") || path.ends_with("/rss") {
            return Gate::Respond(StatusCode::NOT_FOUND.into_response());
        }
        return Gate::Allow;
    }

    // The one unauthenticated way to read the feed: the secret URL Ghost
    // builds from `public_hash`, so a reader's RSS app keeps working. Ghost
    // strips the hash and lets the ordinary feed route answer, which is what
    // the rewrite does here.
    let hash = state.settings.get_str("public_hash").unwrap_or_default();
    if !hash.is_empty() {
        let secret = format!("/{hash}/rss");
        if path.trim_end_matches('/') == secret {
            return Gate::Rewrite(ghost_core::url::base("/rss/"));
        }
    }

    let back = percent_encoding::utf8_percent_encode(
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"),
        percent_encoding::NON_ALPHANUMERIC,
    )
    .to_string();
    Gate::Respond(
        Redirect::to(&ghost_core::url::base(&format!("/private/?r={back}"))).into_response(),
    )
}

/// The middleware wrapper around [`gate`].
pub async fn guard(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    match gate(&state, &req.uri().clone(), req.headers()) {
        Gate::Allow => next.run(req).await,
        Gate::Respond(r) => r,
        Gate::Rewrite(path) => {
            match path.parse::<Uri>() {
                Ok(uri) => *req.uri_mut() = uri,
                // An unparseable rewrite must not fall through to the
                // original, secret-bearing URL; refuse instead.
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            }
            next.run(req).await
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct PasswordForm {
    password: String,
}

/// `GET /private/` — the access-code form.
pub async fn private_page(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    // Nothing to protect, or already in: Ghost sends both cases home.
    if !is_private(&state) || has_access(&state, &headers) {
        return Redirect::to(&ghost_core::url::base("/")).into_response();
    }
    let _ = uri;
    render_private(&state, headers, &safe_redirect(q.get("r")), false).await
}

/// `POST /private/` — check the code and let them in.
pub async fn private_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Form(form): Form<PasswordForm>,
) -> Response {
    if !is_private(&state) {
        return Redirect::to(&ghost_core::url::base("/")).into_response();
    }
    let forward = safe_redirect(q.get("r"));
    match grant(&state, &form.password) {
        Some(cookie) => {
            let mut resp = Redirect::to(&ghost_core::url::base(&forward)).into_response();
            if let Ok(v) = cookie.parse() {
                resp.headers_mut().insert(header::SET_COOKIE, v);
            }
            resp
        }
        None => render_private(&state, headers, &forward, true).await,
    }
}

/// Render the theme's `private.hbs` if it has one, else the built-in form —
/// the same theme-then-fallback shape the error pages use.
async fn render_private(
    state: &AppState,
    headers: HeaderMap,
    forward: &str,
    wrong_code: bool,
) -> Response {
    let theme = state.theme.read().await.clone();
    let error = if wrong_code { "Incorrect access code." } else { "" };

    let mut root = Map::new();
    root.insert("error".into(), Value::String(error.to_string()));
    root.insert("forward".into(), Value::String(forward.to_string()));

    let rctx = RenderCtx {
        context: vec!["private".to_string()],
        relative_url: "/private/".to_string(),
        member: member_context(state, &headers).await,
    };
    let candidates = vec!["private".to_string()];
    if theme.has_template("private") {
        if let Ok(html) =
            render_template(state, &theme, &candidates, Value::Object(root), &rctx).await
        {
            if !html.trim().is_empty() {
                return private_response(wrong_code, Html(html).into_response());
            }
        }
    }

    let title = state.settings.get_str("title").unwrap_or_else(|| "This site".into());
    let action = ghost_core::url::base(&format!(
        "/private/?r={}",
        percent_encoding::utf8_percent_encode(forward, percent_encoding::NON_ALPHANUMERIC)
    ));
    let html = format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow">
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 0;
  min-height: 100vh; display: grid; place-items: center; background: #f6f6f7; color: #15171a; }}
@media (prefers-color-scheme: dark) {{ body {{ background: #15171a; color: #f6f6f7; }} }}
.box {{ width: min(92vw, 420px); text-align: center; padding: 8vh 0; }}
h1 {{ font-size: 1.4rem; font-weight: 600; margin: 0 0 .4rem; }}
p {{ color: #626d79; margin: 0 0 1.6rem; }}
input {{ width: 100%; box-sizing: border-box; font: inherit; padding: .8rem 1rem;
  border: 1px solid #c5c9cd; border-radius: 8px; background: transparent; color: inherit; }}
input:focus {{ outline: 2px solid #3b82f6; outline-offset: 1px; border-color: transparent; }}
button {{ width: 100%; margin-top: .8rem; font: inherit; font-weight: 600; padding: .8rem 1rem;
  border: 0; border-radius: 8px; background: #15171a; color: #fff; cursor: pointer; }}
@media (prefers-color-scheme: dark) {{ button {{ background: #f6f6f7; color: #15171a; }} }}
.err {{ color: #d7373f; margin: 0 0 1rem; font-size: .92rem; }}
</style></head><body><div class="box">
<h1>{title}</h1>
<p>This site is private. Enter the access code to continue.</p>
{err}
<form method="post" action="{action}">
<label for="pw" style="position:absolute;left:-9999px">Access code</label>
<input id="pw" name="password" type="password" autocomplete="current-password" autofocus required>
<button type="submit">Continue</button>
</form>
</div></body></html>"#,
        title = html_escape(&title),
        action = html_escape(&action),
        err = if wrong_code { r#"<p class="err">Incorrect access code.</p>"# } else { "" },
    );
    private_response(wrong_code, Html(html).into_response())
}

/// A wrong code is a 403, and neither variant may ever be cached — a cached
/// copy of the form would be served to someone who has already signed in, and
/// a cached copy of the site to someone who has not.
fn private_response(wrong_code: bool, mut resp: Response) -> Response {
    if wrong_code {
        *resp.status_mut() = StatusCode::FORBIDDEN;
    }
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store, private"),
    );
    resp
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cookie_hash_is_salted_with_the_code() {
        let a = hash_code("letmein", "salt1");
        let b = hash_code("letmein", "salt2");
        // same code, different salt → different token, so a stolen cookie
        // cannot be replayed against a session minted elsewhere
        assert_ne!(a, b);
        assert_eq!(a, hash_code("letmein", "salt1"));
        // changing the code invalidates a cookie minted under the old one
        assert_ne!(hash_code("newcode", "salt1"), a);
    }

    #[test]
    fn redirects_stay_on_this_site() {
        assert_eq!(safe_redirect(Some(&"/about/".to_string())), "/about/");
        assert_eq!(safe_redirect(Some(&"%2Fabout%2F".to_string())), "/about/");
        // protocol-relative and absolute targets are off-site
        assert_eq!(safe_redirect(Some(&"//evil.test/".to_string())), "/");
        assert_eq!(safe_redirect(Some(&"https://evil.test/".to_string())), "/");
        assert_eq!(safe_redirect(None), "/");
    }
}
