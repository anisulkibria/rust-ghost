//! Stripe Connect's OAuth handshake and key adoption.
//!
//! A port of Ghost's `services/members/stripe-connect.js` plus the part of
//! `settings-bread-service` that turns the returned token into the five
//! `stripe_connect_*` settings.
//!
//! The shape of the flow is Stripe's, not Ghost's, and it is worth stating
//! because it explains why a self-hosted install can use Ghost's client ids
//! at all. The site never holds a Connect client secret. It sends the admin
//! to `connect.stripe.com` with Ghost's public client id and Ghost's
//! `redirect_uri`, Stripe hands the authorisation code to that broker
//! (`stripe.ghost.org`), and the broker renders the resulting keys as one
//! base64 blob for the admin to paste back. So the secret that would have to
//! be protected is only ever Ghost's, and the thing crossing the browser is
//! bound to a single admin session by the `state` minted here.
//!
//! That binding is the whole security property, so it lives in one place:
//! [`mint_state`] writes a nonce into the session row, and
//! [`decode_secure_key`] refuses a blob whose `s` does not match it. Both the
//! Admin API endpoint and the native admin's form go through here, because
//! two implementations of a state check is one implementation too many.

use crate::AppState;
use base64::Engine;
use serde::Deserialize;
use ghost_db::Db;

/// Where the pending nonce lives in `sessions.session_data`. Upstream's
/// `STATE_PROP`, spelled the same so a session is legible across both.
pub const STATE_PROP: &str = "stripe-connect-state";

/// When the nonce was minted. Not upstream's — an express session expires on
/// its own cookie, while this port keeps long-lived session rows, so without
/// a stamp a `state` would stay redeemable for as long as the admin stayed
/// signed in.
const STARTED_PROP: &str = "stripe-connect-started-at";

/// How long a started connection may sit unfinished.
const STATE_TTL_SECS: i64 = 30 * 60;

/// Ghost's own Connect application. Public identifiers, as upstream ships
/// them in source; the matching client secret is held by the broker.
const LIVE_CLIENT_ID: &str = "ca_8LBuZWhYshxF0A55KgCXu8PRTquCKC5x";
const TEST_CLIENT_ID: &str = "ca_8LBum4Ctv3mmJ1oD0ZRmxjdAhNrrBUy3";
const REDIRECT_URI: &str = "https://stripe.ghost.org";

/// Upstream's messages, verbatim where a client may be matching on them.
pub const CANNOT_CONNECT: &str = "Cannot connect to stripe unless site is using https://";
pub const BAD_TOKEN: &str = "The Stripe Connect token could not be parsed.";
pub const INCORRECT_STATE: &str = "State did not match.";
pub const HAS_SUBSCRIPTIONS: &str = "Cannot disconnect Stripe whilst you have active subscriptions.";

/// The settings that only this flow may write.
///
/// Upstream filters these out of `PUT /settings` and derives them from the
/// token instead, which is the difference between "an admin key may finish a
/// Connect handshake" and "an admin key may point the site at any Stripe
/// account it likes, with no key verification at all".
pub const DERIVED_SETTINGS: [&str; 6] = [
    "stripe_connect_integration_token",
    "stripe_connect_publishable_key",
    "stripe_connect_secret_key",
    "stripe_connect_livemode",
    "stripe_connect_account_id",
    "stripe_connect_display_name",
];

/// Which Stripe mode a connection is for.
///
/// Upstream validates `mode` against `['live', 'test']`, lowercasing and
/// defaulting an absent or empty value to `live`.
pub fn mode_from(raw: Option<&str>) -> Result<&'static str, &'static str> {
    match raw.map(str::trim).unwrap_or_default().to_ascii_lowercase().as_str() {
        // An empty value is not a wrong value: upstream's allowed-values
        // check returns early on one, so it falls through to the default.
        "" | "live" => Ok("live"),
        "test" => Ok("test"),
        _ => Err("Validation (AllowedValues) failed for mode"),
    }
}

/// Ghost refuses to start a connection from a production site served over
/// plain HTTP, because the keys come back through that browser.
pub fn can_connect(env: &str, site_url: &str) -> Result<(), &'static str> {
    if env == "production" && !site_url.starts_with("https://") {
        return Err(CANNOT_CONNECT);
    }
    Ok(())
}

/// The URL to send the admin to, carrying `state`.
pub fn oauth_url(mode: &str, state: &str) -> String {
    let client_id = if mode == "test" { TEST_CLIENT_ID } else { LIVE_CLIENT_ID };
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("scope", "read_write")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("state", state)
        .finish();
    format!("https://connect.stripe.com/oauth/authorize?{query}")
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Mint a nonce for `mode` and record it against this session.
///
/// Returns the encoded state on success. `None` means the session row was not
/// there to write to, and the caller must not proceed: a handshake with no
/// stored state is one that [`decode_secure_key`] could never verify.
pub async fn mint_state(db: &Db, session_id: &str, mode: &str) -> Option<String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        serde_json::json!({ "mode": mode, "randomState": ghost_core::ids::random_hex(16) }).to_string(),
    );
    let written = sqlx::query(&format!(
        "UPDATE sessions SET session_data = json_set(session_data, '$.{STATE_PROP}', ?, '$.{STARTED_PROP}', ?) WHERE session_id = ?"
    ))
    .bind(&encoded)
    .bind(now_unix_seconds())
    .bind(session_id)
    .execute(db)
    .await;
    matches!(written, Ok(result) if result.rows_affected() == 1).then_some(encoded)
}

/// The nonce this session is waiting on, if it is still inside its window.
pub async fn pending_state(db: &Db, session_id: &str) -> Option<String> {
    let row: Option<(Option<String>, Option<i64>)> = sqlx::query_as(&ghost_db::dialect::sql_owned(&format!(
        "SELECT json_extract(session_data, '$.{STATE_PROP}'), json_extract(session_data, '$.{STARTED_PROP}') FROM sessions WHERE session_id = ?"
    )))
    .bind(session_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let (Some(state), Some(started)) = row? else { return None };
    (now_unix_seconds().saturating_sub(started) <= STATE_TTL_SECS).then_some(state)
}

/// Spend the nonce. A `state` is single-use, so this runs before the keys are
/// adopted rather than after — a failure to save must not leave a blob that
/// can be replayed.
pub async fn clear_state(db: &Db, session_id: &str) -> bool {
    sqlx::query(&format!(
        "UPDATE sessions SET session_data = json_remove(session_data, '$.{STATE_PROP}', '$.{STARTED_PROP}') WHERE session_id = ?"
    ))
    .bind(session_id)
    .execute(db)
    .await
    .is_ok()
}

/// The broker's payload. Upstream's single-letter keys.
#[derive(Deserialize)]
struct Payload {
    /// Secret key.
    a: String,
    /// Publishable key.
    p: String,
    /// The `state` this was issued against.
    s: String,
    /// Live mode.
    l: bool,
    /// Account id.
    i: String,
    /// Account display name.
    #[serde(default)]
    n: Option<String>,
}

/// A verified Connect payload.
///
/// No `Debug`: it holds a Stripe secret key, and the point of a type like this
/// is that the key reaches the settings and nowhere else — least of all a log
/// line somebody added while chasing something unrelated.
pub struct SecureKey {
    pub secret_key: String,
    pub publishable_key: String,
    pub account_id: String,
    pub display_name: String,
    pub livemode: bool,
}

/// Decode and verify the blob the admin pasted back.
///
/// `expected` is the nonce from [`pending_state`]. The state check is
/// upstream's; the shape checks after it are this port's, and they are here
/// because the next step spends real time and a real Stripe API call on these
/// strings — a truncated paste should fail at the form, naming what is wrong.
pub fn decode_secure_key(encoded: &str, expected: &str) -> Result<SecureKey, &'static str> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| BAD_TOKEN)?;
    let payload: Payload = serde_json::from_slice(&raw).map_err(|_| BAD_TOKEN)?;
    if expected.is_empty() || payload.s != expected {
        return Err(INCORRECT_STATE);
    }
    if !payload.i.starts_with("acct_") || !payload.a.starts_with("sk_") || !payload.p.starts_with("pk_") {
        return Err("The Stripe secure key is incomplete.");
    }
    // `l` decides which mode the site reports, so it has to agree with the
    // keys themselves. A blob claiming test mode while carrying live keys
    // would take real payments from a site displaying a test badge.
    if payload.l != payload.a.starts_with("sk_live_") || payload.l != payload.p.starts_with("pk_live_") {
        return Err("The Stripe secure key has mismatched test and live modes.");
    }
    Ok(SecureKey {
        secret_key: payload.a,
        publishable_key: payload.p,
        account_id: payload.i,
        display_name: payload.n.unwrap_or_default(),
        livemode: payload.l,
    })
}

/// Whether any member still holds a paid subscription.
///
/// Upstream's disconnect refuses while one does, and it is right to: the keys
/// are what reconciles those subscriptions, so dropping them leaves members
/// paying Stripe for access this site can no longer confirm.
pub async fn has_paid_members(db: &Db) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM members WHERE status = 'paid'")
        .fetch_one(db)
        .await
        .unwrap_or(0)
        > 0
}

/// What adopting a key did, for the caller to report however it reports.
pub struct Adopted {
    /// The account's display name, or its id when it has none.
    pub account: String,
    pub livemode: bool,
    /// Non-fatal trouble: tier prices or the webhook could not be set up.
    /// Both are recoverable later, and neither should fail a connection that
    /// otherwise works — a site on localhost is simply unreachable by Stripe.
    pub notes: Vec<String>,
}

/// Verify a secret key against Stripe and make it this site's key.
///
/// The single implementation behind both the Admin API's Connect token and
/// the native admin's paste-your-own-keys form. It is deliberately written to
/// return facts rather than an HTTP response, because those two callers
/// report failure very differently — one as a JSON error, one as a redirect
/// with a flash message — and that difference is all they should differ by.
pub async fn adopt_keys(
    state: &AppState,
    secret_key: &str,
    publishable_key: &str,
) -> Result<Adopted, String> {
    let secret = secret_key.trim().to_string();
    let publishable = publishable_key.trim().to_string();
    if !secret.starts_with("sk_") && !secret.starts_with("rk_") {
        return Err("That does not look like a Stripe secret key — it should start with sk_ or rk_.".into());
    }
    if !publishable.is_empty() && !publishable.starts_with("pk_") {
        return Err("The publishable key should start with pk_.".into());
    }
    let account = ghost_stripe::verify_account(&secret).await.map_err(|e| e.message())?;

    let old_account = state.settings.get_str("stripe_connect_account_id").unwrap_or_default();
    let old_live = ghost_stripe::StripeConfig::from_settings(&state.settings).map(|cfg| cfg.is_live());
    let account_changed = old_account != account.id || old_live != Some(account.livemode);

    if account_changed {
        // Price and product ids belong to the old account and mean nothing to
        // the new one, so they go before anything can try to charge against
        // them. Transactional, because a half-cleared set is worse than both.
        let reset = async {
            let mut tx = state.db.begin().await?;
            sqlx::query("UPDATE products SET monthly_price_id = NULL, yearly_price_id = NULL WHERE type = 'paid'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM stripe_prices").execute(&mut *tx).await?;
            sqlx::query("DELETE FROM stripe_products").execute(&mut *tx).await?;
            tx.commit().await
        }
        .await;
        if let Err(error) = reset {
            return Err(format!("Could not reset Stripe price references: {error}"));
        }

        // Keep a working endpoint or local Stripe CLI secret when reconnecting
        // the same account and mode. A different account must not inherit it.
        if let Some(old_cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
            if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|id| !id.is_empty()) {
                let _ = ghost_stripe::delete_webhook(&old_cfg, &id).await;
            }
        }
        for key in ["members_stripe_webhook_id", "members_stripe_webhook_secret", "stripe_checkout_setup_error"] {
            let _ = state.settings.set(&state.db, key, &serde_json::json!("")).await;
        }
    }

    let set = |k: &'static str, v: String| {
        let state = state.clone();
        async move { state.settings.set(&state.db, k, &serde_json::json!(v)).await.is_ok() }
    };
    let mut ok = set("stripe_secret_key", secret.clone()).await;
    ok &= set("stripe_publishable_key", publishable).await;
    ok &= set("stripe_connect_account_id", account.id.clone()).await;
    ok &= set("stripe_connect_display_name", account.display_name.clone()).await;
    ok &= set("stripe_connect_livemode", account.livemode.to_string()).await;
    // Paid tiers only become offerable once Stripe is attached, and Portal
    // reads this to decide whether to show them at all.
    ok &= state.settings.set(&state.db, "paid_members_enabled", &serde_json::json!(true)).await.is_ok();
    if !ok {
        return Err("Could not save the Stripe settings — check the logs.".into());
    }

    let cfg = ghost_stripe::StripeConfig {
        secret_key: secret,
        publishable_key: String::new(),
        webhook_secret: String::new(),
        checkout_design: None,
    };
    let mut notes = Vec::new();
    if let Err(error) = crate::stripe_tiers::sync_active_tiers(state, &cfg).await {
        notes.push(format!(
            "Tier price setup failed: {}. Checkout will retry setup when needed.",
            error.message()
        ));
    }
    let hook_url = format!("{}/members/webhooks/stripe/", state.site_url());
    let have_hook = state
        .settings
        .get_str("members_stripe_webhook_secret")
        .is_some_and(|secret| !secret.is_empty());
    if !have_hook {
        match ghost_stripe::create_webhook(&cfg, &hook_url).await {
            Ok((id, whsec)) => {
                let _ = state.settings.set(&state.db, "members_stripe_webhook_id", &serde_json::json!(id)).await;
                let _ = state.settings.set(&state.db, "members_stripe_webhook_secret", &serde_json::json!(whsec)).await;
            }
            Err(e) => notes.push(format!(
                "Webhooks could not be registered ({}) — subscriptions will not update automatically until {hook_url} is reachable from the internet.",
                e.message()
            )),
        }
    }

    Ok(Adopted {
        account: if account.display_name.is_empty() { account.id } else { account.display_name },
        livemode: account.livemode,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(value: serde_json::Value) -> String {
        base64::engine::general_purpose::STANDARD.encode(value.to_string())
    }

    fn live_blob(state: &str) -> String {
        blob(serde_json::json!({
            "a": "sk_live_secret", "p": "pk_live_publishable", "s": state,
            "l": true, "i": "acct_123", "n": "The Publication",
        }))
    }

    #[test]
    fn mode_follows_upstreams_allowed_values() {
        assert_eq!(mode_from(None), Ok("live"));
        assert_eq!(mode_from(Some("")), Ok("live"));
        assert_eq!(mode_from(Some("live")), Ok("live"));
        assert_eq!(mode_from(Some("test")), Ok("test"));
        // Upstream lowercases before comparing, so this is a real accept.
        assert_eq!(mode_from(Some("TEST")), Ok("test"));
        assert!(mode_from(Some("sandbox")).is_err());
    }

    #[test]
    fn the_client_id_and_redirect_are_ghosts_own() {
        let params = |url: &str| {
            let parsed = reqwest::Url::parse(url).expect("a valid url");
            assert_eq!(parsed.host_str(), Some("connect.stripe.com"));
            assert_eq!(parsed.path(), "/oauth/authorize");
            parsed.query_pairs().into_owned().collect::<std::collections::HashMap<_, _>>()
        };
        // A base64 state carries `+`, `/` and `=`, all of which mean something
        // else in a query string — so it has to come back out unchanged.
        let test = params(&oauth_url("test", "abc+/=def"));
        assert_eq!(test["client_id"], TEST_CLIENT_ID);
        assert_eq!(test["redirect_uri"], "https://stripe.ghost.org");
        assert_eq!(test["response_type"], "code");
        assert_eq!(test["scope"], "read_write");
        assert_eq!(test["state"], "abc+/=def");
        assert_eq!(params(&oauth_url("live", "s"))["client_id"], LIVE_CLIENT_ID);
    }

    #[test]
    fn a_production_site_on_plain_http_cannot_connect() {
        assert_eq!(can_connect("production", "http://example.com"), Err(CANNOT_CONNECT));
        assert!(can_connect("production", "https://example.com").is_ok());
        // Development is how anybody tests the flow at all.
        assert!(can_connect("development", "http://localhost:1971").is_ok());
    }

    #[test]
    fn a_key_issued_for_another_session_is_refused() {
        assert_eq!(decode_secure_key(&live_blob("mine"), "theirs").err(), Some(INCORRECT_STATE));
        // And a session with nothing pending can never match.
        assert_eq!(decode_secure_key(&live_blob(""), "").err(), Some(INCORRECT_STATE));
        let key = decode_secure_key(&live_blob("mine"), "mine").expect("matching state");
        assert_eq!(key.account_id, "acct_123");
        assert_eq!(key.display_name, "The Publication");
        assert!(key.livemode);
    }

    #[test]
    fn a_blob_that_is_not_a_connect_payload_reports_upstreams_message() {
        assert_eq!(decode_secure_key("not base64 at all!", "s").err(), Some(BAD_TOKEN));
        assert_eq!(decode_secure_key(&blob(serde_json::json!({"hello": "world"})), "s").err(), Some(BAD_TOKEN));
    }

    #[test]
    fn mode_must_agree_with_the_keys_it_arrives_with() {
        let lying = blob(serde_json::json!({
            "a": "sk_live_secret", "p": "pk_live_publishable", "s": "s", "l": false, "i": "acct_1",
        }));
        assert!(decode_secure_key(&lying, "s").err().is_some_and(|e| e.contains("mismatched")));
        let test_keys = blob(serde_json::json!({
            "a": "sk_test_secret", "p": "pk_test_publishable", "s": "s", "l": false, "i": "acct_1",
        }));
        let key = decode_secure_key(&test_keys, "s").expect("consistent test keys");
        assert!(!key.livemode);
        // A display name is optional; the account lookup supplies the real one.
        assert_eq!(key.display_name, "");
    }

    #[test]
    fn an_incomplete_paste_is_named_as_such_rather_than_sent_to_stripe() {
        for partial in [
            serde_json::json!({"a": "sk_test_x", "p": "pk_test_x", "s": "s", "l": false, "i": "cus_1"}),
            serde_json::json!({"a": "whsec_x", "p": "pk_test_x", "s": "s", "l": false, "i": "acct_1"}),
            serde_json::json!({"a": "sk_test_x", "p": "sk_test_x", "s": "s", "l": false, "i": "acct_1"}),
        ] {
            assert_eq!(
                decode_secure_key(&blob(partial), "s").err(),
                Some("The Stripe secure key is incomplete.")
            );
        }
    }

    #[test]
    fn the_settings_this_flow_owns_are_the_ones_upstream_filters() {
        // Upstream's list, so a direct write cannot point the site at an
        // unverified account.
        assert!(DERIVED_SETTINGS.contains(&"stripe_connect_integration_token"));
        assert!(DERIVED_SETTINGS.contains(&"stripe_connect_secret_key"));
        assert!(DERIVED_SETTINGS.contains(&"stripe_connect_account_id"));
        assert_eq!(DERIVED_SETTINGS.len(), 6);
    }
}
