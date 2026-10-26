//! A small Stripe client, covering the calls Ghost's membership flow makes.
//!
//! Ghost self-hosted uses "Stripe Direct": you paste your own secret and
//! publishable keys rather than going through Stripe Connect's OAuth, which
//! only Ghost(Pro) can offer. Keys live in settings (`stripe_secret_key`,
//! `stripe_publishable_key`), so connecting and disconnecting is a UI action.
//!
//! The REST API is called directly over `reqwest` instead of pulling in a
//! generated SDK: the surface used here is six endpoints, and form-encoding
//! them is less code than mapping a crate's types onto ours.

use ghost_core::{GhostError, Result};
use ghost_db::SettingsCache;

const API: &str = "https://api.stripe.com/v1";

/// A connected Stripe account's credentials.
#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub secret_key: String,
    pub publishable_key: String,
    /// `whsec_…`, set when the webhook endpoint is registered. Without it
    /// incoming webhooks cannot be verified and are refused.
    pub webhook_secret: String,
}

impl StripeConfig {
    pub fn from_settings(settings: &SettingsCache) -> Option<Self> {
        let s = |k: &str| settings.get_str(k).unwrap_or_default().trim().to_string();
        // Ghost keeps a `stripe_connect_*` pair for Ghost(Pro) OAuth and a
        // plain pair for direct keys; prefer whichever is filled in.
        let secret = match s("stripe_secret_key") {
            v if !v.is_empty() => v,
            _ => s("stripe_connect_secret_key"),
        };
        let publishable = match s("stripe_publishable_key") {
            v if !v.is_empty() => v,
            _ => s("stripe_connect_publishable_key"),
        };
        if secret.is_empty() {
            return None;
        }
        Some(StripeConfig {
            secret_key: secret,
            publishable_key: publishable,
            webhook_secret: s("members_stripe_webhook_secret"),
        })
    }

    /// Stripe test keys are `sk_test_…`/`pk_test_…`; live keys are `sk_live_`.
    /// Worth surfacing, because a site in test mode takes no real money.
    pub fn is_live(&self) -> bool {
        self.secret_key.starts_with("sk_live_")
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// POST a form-encoded request and parse Stripe's JSON reply.
async fn post(cfg: &StripeConfig, path: &str, form: &[(String, String)]) -> Result<serde_json::Value> {
    let resp = client()
        .post(format!("{API}{path}"))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .form(form)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(resp).await
}

async fn get(cfg: &StripeConfig, path: &str) -> Result<serde_json::Value> {
    let resp = client()
        .get(format!("{API}{path}"))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(resp).await
}

async fn parse(resp: reqwest::Response) -> Result<serde_json::Value> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    if status.is_success() {
        return Ok(json);
    }
    // Stripe's own message names the problem precisely ("No such price",
    // "Invalid API Key"), so it is far more useful than the status alone.
    let msg = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(&body)
        .to_string();
    Err(GhostError::BadRequest(format!("Stripe: {msg}")))
}

/// What a connected account looks like to the settings screen.
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub id: String,
    pub display_name: String,
    pub livemode: bool,
}

/// Verify a secret key by fetching the account it belongs to. This is what
/// "Connect to Stripe" runs before storing anything — a typo should fail at
/// the point of entry, not at a visitor's first checkout.
pub async fn verify_account(secret_key: &str) -> Result<AccountInfo> {
    let cfg = StripeConfig {
        secret_key: secret_key.trim().to_string(),
        publishable_key: String::new(),
        webhook_secret: String::new(),
    };
    let v = get(&cfg, "/account").await?;
    Ok(AccountInfo {
        id: v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        display_name: v
            .get("settings")
            .and_then(|s| s.get("dashboard"))
            .and_then(|d| d.get("display_name"))
            .and_then(|x| x.as_str())
            .or_else(|| v.get("business_profile").and_then(|b| b.get("name")).and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string(),
        livemode: !secret_key.trim().starts_with("sk_test_"),
    })
}

/// Create (or reuse) the Stripe Product that backs a tier.
pub async fn ensure_product(cfg: &StripeConfig, name: &str, existing: Option<&str>) -> Result<String> {
    if let Some(id) = existing.filter(|i| !i.is_empty()) {
        // Keep the name in step; a missing product means it was deleted in
        // the dashboard, so fall through and make a new one.
        if post(cfg, &format!("/products/{id}"), &[("name".into(), name.into())]).await.is_ok() {
            return Ok(id.to_string());
        }
    }
    let v = post(cfg, "/products", &[("name".into(), name.into())]).await?;
    Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

/// Create a recurring price. Stripe prices are immutable, so a changed amount
/// means a new price — the old one is archived by the caller.
pub async fn create_price(
    cfg: &StripeConfig,
    product_id: &str,
    currency: &str,
    amount_cents: i64,
    interval: &str,
) -> Result<String> {
    let form = vec![
        ("product".to_string(), product_id.to_string()),
        ("currency".to_string(), currency.to_lowercase()),
        ("unit_amount".to_string(), amount_cents.to_string()),
        ("recurring[interval]".to_string(), interval.to_string()),
    ];
    let v = post(cfg, "/prices", &form).await?;
    Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

/// Archive a price so it stops being offered. Stripe does not allow deleting
/// prices that have ever been used.
pub async fn archive_price(cfg: &StripeConfig, price_id: &str) -> Result<()> {
    post(cfg, &format!("/prices/{price_id}"), &[("active".into(), "false".into())]).await?;
    Ok(())
}

/// The offer a checkout is running under: its Stripe coupon, or the trial it
/// grants instead, plus the id to stamp on the session so the webhook can
/// record the redemption.
#[derive(Debug, Clone, Copy)]
pub struct CheckoutOffer<'a> {
    pub offer_id: &'a str,
    pub coupon_id: Option<&'a str>,
    pub trial_days: Option<u32>,
}

/// Start a subscription checkout. Returns the URL to send the visitor to.
#[allow(clippy::too_many_arguments)]
pub async fn create_checkout_session(
    cfg: &StripeConfig,
    price_id: &str,
    customer_email: Option<&str>,
    success_url: &str,
    cancel_url: &str,
    member_uuid: &str,
    trial_days: u32,
    offer: Option<&CheckoutOffer<'_>>,
) -> Result<String> {
    let mut form = vec![
        ("mode".to_string(), "subscription".to_string()),
        ("line_items[0][price]".to_string(), price_id.to_string()),
        ("line_items[0][quantity]".to_string(), "1".to_string()),
        ("success_url".to_string(), success_url.to_string()),
        ("cancel_url".to_string(), cancel_url.to_string()),
        // Carried back on the webhook, which is how a completed payment is
        // matched to the member who started it.
        ("metadata[member_uuid]".to_string(), member_uuid.to_string()),
        ("subscription_data[metadata][member_uuid]".to_string(), member_uuid.to_string()),
    ];
    if let Some(email) = customer_email.filter(|e| !e.is_empty()) {
        form.push(("customer_email".to_string(), email.to_string()));
    }
    // An offer's own trial replaces the tier's, the way Ghost lets a trial
    // offer override the tier default rather than stacking on top of it.
    let trial_days = offer.and_then(|o| o.trial_days).unwrap_or(trial_days);
    if trial_days > 0 {
        form.push(("subscription_data[trial_period_days]".to_string(), trial_days.to_string()));
    }
    if let Some(o) = offer {
        if let Some(coupon) = o.coupon_id.filter(|c| !c.is_empty()) {
            form.push(("discounts[0][coupon]".to_string(), coupon.to_string()));
        }
        // Recorded on both so the webhook can credit the redemption whichever
        // object it reads.
        form.push(("metadata[offer_id]".to_string(), o.offer_id.to_string()));
        form.push(("subscription_data[metadata][offer_id]".to_string(), o.offer_id.to_string()));
    }
    let v = post(cfg, "/checkout/sessions", &form).await?;
    v.get("url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| GhostError::BadRequest("Stripe did not return a checkout URL".into()))
}

/// Billing portal session, so a member can change or cancel their plan.
pub async fn create_billing_portal_session(
    cfg: &StripeConfig,
    customer_id: &str,
    return_url: &str,
) -> Result<String> {
    let form = vec![
        ("customer".to_string(), customer_id.to_string()),
        ("return_url".to_string(), return_url.to_string()),
    ];
    let v = post(cfg, "/billing_portal/sessions", &form).await?;
    v.get("url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| GhostError::BadRequest("Stripe did not return a billing portal URL".into()))
}

/// The events this site acts on. Shared between automatic registration and
/// the `stripe listen --events …` hint shown for local testing, so the two
/// cannot drift apart.
pub const WEBHOOK_EVENTS: &[&str] = &[
    "checkout.session.completed",
    "customer.subscription.created",
    "customer.subscription.updated",
    "customer.subscription.deleted",
    "invoice.payment_failed",
];

/// Register a webhook endpoint and return `(id, signing_secret)`.
pub async fn create_webhook(cfg: &StripeConfig, url: &str) -> Result<(String, String)> {
    let mut form = vec![("url".to_string(), url.to_string())];
    for (i, e) in WEBHOOK_EVENTS.iter().enumerate() {
        form.push((format!("enabled_events[{i}]"), (*e).to_string()));
    }
    let v = post(cfg, "/webhook_endpoints", &form).await?;
    Ok((
        v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        v.get("secret").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
    ))
}

pub async fn delete_webhook(cfg: &StripeConfig, id: &str) -> Result<()> {
    let resp = client()
        .delete(format!("{API}/webhook_endpoints/{id}"))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(resp).await?;
    Ok(())
}

pub async fn get_subscription(cfg: &StripeConfig, id: &str) -> Result<serde_json::Value> {
    get(cfg, &format!("/subscriptions/{id}")).await
}

/// The discount an offer applies, in Stripe's vocabulary.
#[derive(Debug, Clone, Copy)]
pub enum Discount<'a> {
    /// A share off the price, 1–100.
    Percent(i64),
    /// A flat amount off, in the currency's minor unit.
    Fixed { amount_cents: i64, currency: &'a str },
}

/// Create the coupon behind an offer.
///
/// Stripe coupons are immutable in the ways that matter (the discount itself
/// cannot change), so Ghost mints a fresh one whenever an offer's terms are
/// edited rather than trying to update the old one.
///
/// `duration` is Stripe's own `once` / `repeating` / `forever`;
/// `duration_in_months` only applies to `repeating`. A trial offer has no
/// coupon at all — it rides on the subscription's trial period instead.
pub async fn create_coupon(
    cfg: &StripeConfig,
    name: &str,
    discount: Discount<'_>,
    duration: &str,
    duration_in_months: Option<i64>,
) -> Result<String> {
    let mut form = vec![
        ("name".to_string(), name.to_string()),
        ("duration".to_string(), duration.to_string()),
    ];
    match discount {
        Discount::Percent(p) => form.push(("percent_off".to_string(), p.to_string())),
        Discount::Fixed { amount_cents, currency } => {
            form.push(("amount_off".to_string(), amount_cents.to_string()));
            form.push(("currency".to_string(), currency.to_lowercase()));
        }
    }
    if duration == "repeating" {
        // Stripe rejects a repeating coupon without this, so fall back to a
        // single month rather than sending an invalid request.
        form.push((
            "duration_in_months".to_string(),
            duration_in_months.filter(|m| *m > 0).unwrap_or(1).to_string(),
        ));
    }
    let v = post(cfg, "/coupons", &form).await?;
    Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

/// Delete a coupon that is no longer offered. Existing subscriptions already
/// carrying the discount keep it — Stripe applies it from the subscription,
/// not the coupon record.
pub async fn delete_coupon(cfg: &StripeConfig, id: &str) -> Result<()> {
    let resp = client()
        .delete(format!("{API}/coupons/{id}"))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(resp).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// webhook signature verification
// ---------------------------------------------------------------------------

/// Verify a `Stripe-Signature` header against the raw body.
///
/// This is the only thing standing between the webhook endpoint and anyone on
/// the internet marking themselves a paid member, so it is checked before the
/// payload is even parsed. Stripe signs `"{timestamp}.{body}"` with the
/// endpoint's `whsec_…` secret.
pub fn verify_webhook_signature(
    payload: &str,
    sig_header: &str,
    secret: &str,
    tolerance_secs: i64,
) -> bool {
    if secret.is_empty() {
        return false;
    }
    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<&str> = Vec::new();
    for part in sig_header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => timestamp = v.parse().ok(),
            Some(("v1", v)) => signatures.push(v),
            _ => {}
        }
    }
    let Some(ts) = timestamp else { return false };
    // Reject replays of an old, legitimately-signed payload.
    if (chrono::Utc::now().timestamp() - ts).abs() > tolerance_secs {
        return false;
    }
    let expected = hmac_sha256_hex(secret, &format!("{ts}.{payload}"));
    signatures.iter().any(|s| secure_eq(s, &expected))
}

fn hmac_sha256_hex(key: &str, msg: &str) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = key.as_bytes().to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg.as_bytes());
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    hex::encode(outer.finalize())
}

fn secure_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, ts: i64, body: &str) -> String {
        format!("t={ts},v1={}", hmac_sha256_hex(secret, &format!("{ts}.{body}")))
    }

    #[test]
    fn a_genuine_signature_verifies() {
        let body = r#"{"id":"evt_1","type":"checkout.session.completed"}"#;
        let now = chrono::Utc::now().timestamp();
        let header = sign("whsec_test", now, body);
        assert!(verify_webhook_signature(body, &header, "whsec_test", 300));
    }

    #[test]
    fn forged_and_stale_signatures_are_refused() {
        let body = r#"{"id":"evt_1"}"#;
        let now = chrono::Utc::now().timestamp();

        // wrong secret — the whole point of the check
        let header = sign("whsec_attacker", now, body);
        assert!(!verify_webhook_signature(body, &header, "whsec_test", 300));

        // right secret, but the body was changed after signing
        let header = sign("whsec_test", now, body);
        assert!(!verify_webhook_signature(r#"{"id":"evt_2"}"#, &header, "whsec_test", 300));

        // correctly signed, but replayed long after the fact
        let old = sign("whsec_test", now - 10_000, body);
        assert!(!verify_webhook_signature(body, &old, "whsec_test", 300));

        // no secret configured must never pass, whatever is presented
        assert!(!verify_webhook_signature(body, &header, "", 300));

        // malformed headers
        assert!(!verify_webhook_signature(body, "garbage", "whsec_test", 300));
        assert!(!verify_webhook_signature(body, "v1=abc", "whsec_test", 300));
    }

    #[test]
    fn live_keys_are_told_from_test_keys() {
        let mk = |k: &str| StripeConfig {
            secret_key: k.into(),
            publishable_key: String::new(),
            webhook_secret: String::new(),
        };
        assert!(mk("sk_live_abc").is_live());
        assert!(!mk("sk_test_abc").is_live());
    }
}
