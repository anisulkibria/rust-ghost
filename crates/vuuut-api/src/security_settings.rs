//! The Security module's settings, read off [`AppState`].
//!
//! [`vuuut_core::security`] holds the rules and takes a `serde_json::Value`,
//! so that it can be unit-tested without a server and called from anywhere.
//! This is the one adapter between that and the running application: it pulls
//! the `security_*` keys out of the settings cache and hands them over.
//!
//! It exists as a module of its own, rather than as a helper beside each
//! caller, because four crates ask these questions on hot paths — the rate
//! limiter on every authentication attempt, the admin middleware on every
//! admin response, the session guard on every admin request, and the link
//! previewer on every bookmark card. Four copies of "read these keys into a
//! JSON object" would be four chances for them to disagree about what a
//! missing row means, and a missing row meaning `false` is a defence that
//! quietly turns itself off. One function, one answer. This is the same
//! argument `vuuut_core::seo::flag` makes for the SEO flags.
//!
//! ## Cost
//!
//! [`snapshot`] builds a small `serde_json::Map` per call. The settings cache
//! itself is already in memory, so there is no query; what this costs is a
//! handful of map lookups and clones of short values. The two questions asked
//! most often — [`trust_proxy`] and [`session_policy`] — read only the keys
//! they need and skip the snapshot for that reason.

use crate::AppState;
use vuuut_core::security as rules;
use serde_json::Value;

/// Every `security_*` key the rules read, as one JSON object.
///
/// Keys that have no row are simply absent, which is what the rules want:
/// `vuuut_core::security::flag` falls back to the key's own default, so a
/// database that predates this module reads as "the defences are on" rather
/// than as "everything is off".
pub fn snapshot(state: &AppState) -> Value {
    let mut map = serde_json::Map::new();
    let mut take = |key: &str| {
        if let Some(value) = state.settings.get(key) {
            map.insert(key.to_string(), value);
        }
    };
    for (key, _) in rules::BOOL_SETTINGS {
        take(key);
    }
    for key in [
        "security_admin_referrer_policy",
        "security_session_days",
        "security_session_idle_minutes",
        "security_min_password_length",
    ] {
        take(key);
    }
    Value::Object(map)
}

/// One key as a JSON object, for the questions asked on a hot path.
fn only(state: &AppState, key: &str) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(value) = state.settings.get(key) {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}

/// Whether `X-Forwarded-For` is believed. See
/// [`crate::brute::client_ip_for`], which is the only thing that should be
/// asking.
pub fn trust_proxy(state: &AppState) -> bool {
    rules::trust_proxy(&only(state, "security_trust_proxy"))
}

/// Whether the server refuses to fetch URLs that resolve inside its own
/// network. See [`crate::mentions::is_public_host`], which is the predicate
/// this switches on and off.
pub fn private_fetch_blocked(state: &AppState) -> bool {
    rules::private_fetch_blocked(&only(state, "security_private_fetch"))
}

/// Whether a password change signs the account out of its other sessions.
pub fn revoke_on_password_change(state: &AppState) -> bool {
    rules::flag(&only(state, "security_revoke_on_password_change"), "security_revoke_on_password_change")
}

/// The session policy as `(maximum age, idle timeout)`, both in seconds. An
/// idle timeout of zero means there is none.
pub fn session_policy(state: &AppState) -> (i64, i64) {
    let mut map = serde_json::Map::new();
    for key in ["security_session_days", "security_session_idle_minutes"] {
        if let Some(value) = state.settings.get(key) {
            map.insert(key.to_string(), value);
        }
    }
    let settings = Value::Object(map);
    (rules::session_max_age_secs(&settings), rules::session_idle_secs(&settings))
}

/// The shortest staff password this site accepts. Every password path —
/// setup, the staff screen, the Admin API, the reset link, a redeemed invite
/// — asks this rather than spelling out a number, so that raising it raises
/// it everywhere.
pub fn min_password_length(state: &AppState) -> usize {
    rules::min_password_length(&only(state, "security_min_password_length"))
}

/// The sentence a too-short password is refused with. One literal, because
/// five screens say it and a site that has raised the minimum should not have
/// one of them still claiming ten.
pub fn password_too_short(state: &AppState) -> String {
    format!("Password must be at least {} characters.", min_password_length(state))
}

/// The security headers a response on `surface` should carry.
///
/// `https` is read from the site's own URL rather than from the request,
/// because that is the question HSTS turns on: a site whose canonical URL is
/// http must never send a promise that would make it unreachable.
pub fn response_headers(state: &AppState, surface: rules::Surface) -> Vec<(&'static str, String)> {
    let https = state.site_url().starts_with("https://");
    rules::response_headers(&snapshot(state), surface, https)
}

/// The `spam_*` and `firewall_*` keys, as one JSON object.
///
/// A second snapshot rather than more keys in [`snapshot`], because the two
/// sets have opposite defaults — a missing `security_*` row means the defence
/// is on, a missing `spam_*` row means the filter is off — and they are read
/// back through different functions for exactly that reason. Mixing them in
/// one object would make it possible to read a key through the wrong one and
/// get a plausible, wrong answer. See [`vuuut_core::firewall::flag_spam`].
pub fn spam_snapshot(state: &AppState) -> Value {
    let mut map = serde_json::Map::new();
    for (key, _) in vuuut_core::firewall::BOOL_SETTINGS {
        if let Some(value) = state.settings.get(key) {
            map.insert(key.to_string(), value);
        }
    }
    if let Some(value) = state.settings.get("spam_blocked_user_agents") {
        map.insert("spam_blocked_user_agents".into(), value);
    }
    Value::Object(map)
}
