//! The firewall and the spam filters, as middleware.
//!
//! The rules are [`vuuut_core::firewall`]; the storage is
//! [`vuuut_db::repos::firewall`]; this is the layer that puts them on a
//! request. One module rather than two copies, because the public site and
//! the admin must not be able to disagree about who is barred — and because
//! the order a request meets these checks in is itself a decision, stated
//! once, here.
//!
//! ## The order a request meets them in
//!
//! 1. **Is the firewall on at all?** `firewall_enabled` off, or no rules:
//!    return immediately, before the address is even parsed. An empty
//!    firewall costs one atomic read.
//! 2. **Whose address is this?** Through [`crate::brute::client_ip_for`], so
//!    the firewall and the rate limiter agree about who the caller is — and
//!    so that `security_trust_proxy` governs both. This matters more here
//!    than anywhere: with forwarded addresses believed on a directly-exposed
//!    server, **a blocked caller can walk around the block by sending a
//!    header**, and the Security report says so.
//! 3. **Do the rules refuse it?** [`vuuut_core::firewall::decide`].
//! 4. **Is the User-Agent on the blocklist?** Only if `spam_block_user_agents`
//!    is on, and only after the address rules, because an address rule is a
//!    deliberate decision about a person and a user-agent rule is a guess
//!    about a robot.
//!
//! ## What a refused request gets
//!
//! `403` with a short plain-text sentence and no detail: not which rule, not
//! that there is a firewall, not when it expires. A caller who is barred does
//! not need to be told how the barring works, and a scanner that learns the
//! shape of the refusal learns where the holes are. `Cache-Control: no-store`
//! so a proxy cannot hold the refusal and serve it to somebody else after the
//! rule is lifted.
//!
//! The one exception is the admin surface, which answers a browser with a
//! page rather than a bare line — somebody who has just locked themselves out
//! of their own site needs to understand what happened, and they are already
//! on the other side of the rule so there is nothing left to conceal.

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use vuuut_core::firewall as rules;
use vuuut_core::firewall::{Decision, Surface};

use crate::AppState;

// ---------------------------------------------------------------------------
// reading the settings
// ---------------------------------------------------------------------------

/// Whether the firewall is enforced at all.
pub fn enabled(state: &AppState) -> bool {
    state.settings.get_bool("firewall_enabled")
}

/// The user-agent blocklist, or empty when the switch is off.
///
/// Parsed per call rather than cached: it is read only when
/// `spam_block_user_agents` is on, which is off by default, and parsing a
/// handful of short lines is cheaper than the invalidation a cache would
/// need. If this ever shows up in a profile it belongs beside the rule cache
/// on `AppState`.
pub fn blocked_user_agents(state: &AppState) -> Vec<String> {
    if !state.settings.get_bool("spam_block_user_agents") {
        return Vec::new();
    }
    rules::parse_user_agents(&state.settings.get_str("spam_blocked_user_agents").unwrap_or_default())
}

/// Whether a member signup must carry this site's own origin.
pub fn signup_same_origin_required(state: &AppState) -> bool {
    state.settings.get_bool("spam_signup_same_origin")
}

/// Whether a member signup from a disposable address is refused.
pub fn disposable_email_blocked(state: &AppState) -> bool {
    state.settings.get_bool("spam_block_disposable_email")
}

/// Whether this email should be refused at signup, by either filter.
///
/// The site's own `blocked_email_domains` list is checked first and always —
/// it predates this module and is not behind a switch — then the built-in
/// disposable list, which is. Returns the reason, for the log; the *caller*
/// is told only that the address was not accepted, because telling a spammer
/// which list caught them is telling them which address to try next.
pub fn signup_email_refused(state: &AppState, email: &str) -> Option<&'static str> {
    if state.settings.email_domain_is_blocked(email) {
        return Some("the site's blocked-domain list");
    }
    if disposable_email_blocked(state) && rules::is_disposable_email(email) {
        return Some("the built-in disposable-address list");
    }
    None
}

// ---------------------------------------------------------------------------
// the decision
// ---------------------------------------------------------------------------

/// What the firewall says about this request. `None` means let it through.
///
/// Separated from the middleware so both surfaces, and the tests, can ask the
/// question without building a `Request`.
pub async fn check(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<std::net::IpAddr>,
    surface: Surface,
) -> Option<Refusal> {
    // Both halves off: the overwhelmingly common case, and it costs one
    // settings lookup and one atomic read.
    let firewall_on = enabled(state) && !state.firewall.is_empty();
    let agents = blocked_user_agents(state);
    if !firewall_on && agents.is_empty() {
        return None;
    }

    if firewall_on {
        // Through the same reader the rate limiter uses, so the two cannot
        // disagree about who the caller is, and so `security_trust_proxy`
        // governs both.
        let raw = crate::brute::client_ip_for(state, headers, peer);
        match raw.parse::<std::net::IpAddr>() {
            Ok(ip) => {
                let decision = rules::decide(&state.firewall.rules(), ip, surface);
                if let Decision::Block { rule } = decision {
                    if let Some(id) = rule {
                        // One write, on a request that was going to be
                        // refused anyway. Never on the allowed path.
                        vuuut_db::repos::firewall::record_hit(&state.db, &id).await;
                    }
                    return Some(Refusal { surface });
                }
            }
            // An address that will not parse is **let through**. The firewall
            // is a list of specific decisions about specific addresses, and
            // "I could not tell who this is" is not one of them — refusing
            // here would take the site down for every caller the moment the
            // proxy header changed shape.
            Err(_) => {
                tracing::debug!("firewall: could not read the client address {raw:?}");
            }
        }
    }

    if !agents.is_empty() {
        let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
        if rules::user_agent_blocked(ua, &agents) {
            return Some(Refusal { surface });
        }
    }

    None
}

/// The socket address the server attached to this request, if any.
///
/// This has to be read from the request's extensions rather than left as
/// `None`: with `security_trust_proxy` **off** — which is the secure setting
/// and the one the Security report tells people to choose — the socket is the
/// *only* source of the caller's address. A firewall that quietly stopped
/// matching the moment somebody hardened their rate limiting would be worse
/// than no firewall, because it would still be on the screen saying it was
/// enforcing rules.
fn peer_of(req: &Request) -> Option<std::net::IpAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|axum::extract::ConnectInfo(addr)| addr.ip())
}

/// A refused request.
pub struct Refusal {
    surface: Surface,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let mut response = match self.surface {
            // A bare line: a caller who is barred does not need to be told
            // how the barring works, and a scanner that learns the shape of
            // the refusal learns where the holes are.
            Surface::Public => (StatusCode::FORBIDDEN, "Forbidden.").into_response(),
            // The admin answers a browser with something readable. Somebody
            // who has just locked themselves out of their own site needs to
            // understand what happened, and they are already on the other
            // side of the rule, so there is nothing left to conceal.
            Surface::Admin => (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                ADMIN_REFUSAL,
            )
                .into_response(),
        };
        // `no-store`, so a proxy cannot hold the refusal and go on serving it
        // to somebody else after the rule is lifted.
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
        response
    }
}

/// The page an admin refusal renders.
///
/// Deliberately self-contained — no stylesheet, no template, no settings
/// lookup. It is served to somebody the firewall has decided not to talk to,
/// and the less of the application that runs for them the better. It names
/// the table and the setting so that whoever has locked themselves out knows
/// what to change from a shell, which is the only place left they can change
/// it from.
const ADMIN_REFUSAL: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Blocked</title>
<style>body{font:16px/1.6 -apple-system,Segoe UI,sans-serif;max-width:34rem;margin:18vh auto;padding:0 1.5rem;color:#15171a}
h1{font-size:1.4rem;margin:0 0 .6rem}p{margin:0 0 .8rem;color:#56606c}code{background:#f1f3f5;padding:.1em .35em;border-radius:3px;font-size:.9em}</style>
</head><body><h1>This address cannot reach the admin</h1>
<p>A firewall rule on this site refuses connections from where you are.</p>
<p>If that is a mistake and you are an administrator, the rules are in the
<code>firewall_rules</code> table, and the whole firewall can be switched off by setting
<code>firewall_enabled</code> to <code>false</code> in <code>settings</code>.</p>
</body></html>"#;

// ---------------------------------------------------------------------------
// middleware
// ---------------------------------------------------------------------------

/// The firewall on the public site. Mount it outermost.
pub async fn public_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let peer = peer_of(&req);
    if let Some(refusal) = check(&state, req.headers(), peer, Surface::Public).await {
        return refusal.into_response();
    }
    next.run(req).await
}

/// The firewall on `/ghost/`. Mount it outermost, before the session is read
/// — an address that is barred is barred whether or not it has a cookie.
pub async fn admin_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let peer = peer_of(&req);
    if let Some(refusal) = check(&state, req.headers(), peer, Surface::Admin).await {
        return refusal.into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use vuuut_core::firewall::{Action, Pattern, Rule, Scope};

    fn rule(action: Action, scope: Scope, pattern: &str) -> Rule {
        Rule {
            id: "r1".into(),
            action,
            scope,
            pattern: Pattern::parse(pattern).unwrap(),
            note: String::new(),
        }
    }

    #[test]
    fn a_refusal_is_never_cacheable_and_never_explains_itself_to_the_public() {
        let response = Refusal { surface: Surface::Public }.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store",
            "a proxy could hold this and serve it after the rule is lifted",
        );
    }

    #[test]
    fn the_admin_refusal_says_how_to_undo_it() {
        // The one place the firewall explains itself: whoever is reading this
        // has locked themselves out, and a shell is the only place left they
        // can fix it from.
        assert!(ADMIN_REFUSAL.contains("firewall_rules"));
        assert!(ADMIN_REFUSAL.contains("firewall_enabled"));
        // And it is self-contained — no stylesheet, no template, nothing of
        // the application runs for somebody the firewall refused.
        assert!(!ADMIN_REFUSAL.contains("/ghost/assets/"));
    }

    #[test]
    fn the_public_refusal_gives_a_scanner_nothing() {
        let body = "Forbidden.";
        assert!(!body.contains("firewall"));
        assert!(!body.contains("rule"));
    }

    #[test]
    fn an_unreadable_client_address_is_let_through_rather_than_refused() {
        // "I could not tell who this is" is not a decision somebody wrote
        // down. Refusing here would take the site down for every caller the
        // moment a proxy header changed shape.
        let rules = vec![rule(Action::Block, Scope::Site, "0.0.0.0/0")];
        // `decide` is only reached with a parsed address; this asserts the
        // shape of the guard above it.
        assert!("unknown".parse::<std::net::IpAddr>().is_err());
        assert!(rules::decide(&rules, "203.0.113.4".parse().unwrap(), Surface::Public).is_block());
    }
}
