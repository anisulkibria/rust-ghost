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
/// Matches Ghost 6.73's Stripe client and subscription payloads.
pub const STRIPE_API_VERSION: &str = "2020-08-27";

/// A connected Stripe account's credentials.
#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub secret_key: String,
    pub publishable_key: String,
    /// `whsec_…`, set when the webhook endpoint is registered. Without it
    /// incoming webhooks cannot be verified and are refused.
    pub webhook_secret: String,
    pub checkout_design: Option<serde_json::Value>,
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
            checkout_design: settings.get("stripe_checkout_config").and_then(|config|config.get("design").cloned()).filter(|design|design.get("customize").and_then(|value|value.as_bool())==Some(true)),
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
async fn post(cfg: &StripeConfig, path: &str, form: &[(String,String)]) -> Result<serde_json::Value> {
    if path=="/checkout/sessions" {
        if let Some(design)=&cfg.checkout_design {
            let mut decorated=form.to_vec();
            for key in ["button_color","background_color","border_style","font_family"] {if let Some(value)=design[key].as_str(){decorated.push((format!("branding_settings[{key}]"),value.into()));}}
            let response=client().post(format!("{API}{path}")).basic_auth(&cfg.secret_key,None::<&str>).header("Stripe-Version",STRIPE_API_VERSION).form(&decorated).send().await.map_err(|e|GhostError::BadRequest(e.to_string()))?;
            let status=response.status();let body:serde_json::Value=response.json().await.map_err(|_|GhostError::BadRequest("Stripe returned invalid JSON".into()))?;
            if status.is_success(){return Ok(body)}
            if status==reqwest::StatusCode::BAD_REQUEST && body.pointer("/error/param").and_then(|value|value.as_str()).is_some_and(|param|param.starts_with("branding_settings")) {
                // A rejected design must not prevent a buyer from checking out.
                return post_raw(cfg,path,form).await;
            }
            return Err(GhostError::BadRequest(body.pointer("/error/message").and_then(|value|value.as_str()).unwrap_or("Stripe Checkout failed").into()));
        }
    }
    post_raw(cfg,path,form).await
}

async fn post_raw(cfg: &StripeConfig, path: &str, form: &[(String, String)]) -> Result<serde_json::Value> {
    let resp = client()
        .post(format!("{API}{path}"))
        .basic_auth(&cfg.secret_key, None::<&str>)
        .header("Stripe-Version", STRIPE_API_VERSION)
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
        .header("Stripe-Version", STRIPE_API_VERSION)
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
        checkout_design: None,
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

/// The donation price: a one-time price whose amount the donor sets on the
/// checkout page. Ghost creates it once per (currency, preset) through
/// `getPriceForDonations` and reuses the id — `custom_unit_amount` with the
/// suggested amount as the preset is what puts an editable amount box on
/// Stripe's page. A preset under Stripe's minimum charge (100 minor units)
/// is omitted, exactly as Ghost omits it.
pub async fn create_donation_price(
    cfg: &StripeConfig,
    product_id: &str,
    currency: &str,
    preset_cents: Option<i64>,
    nickname: &str,
) -> Result<String> {
    let mut form = vec![
        ("product".to_string(), product_id.to_string()),
        ("currency".to_string(), currency.to_lowercase()),
        ("nickname".to_string(), nickname.to_string()),
        ("custom_unit_amount[enabled]".to_string(), "true".to_string()),
    ];
    if let Some(cents) = preset_cents.filter(|c| *c >= 100) {
        form.push(("custom_unit_amount[preset]".to_string(), cents.to_string()));
    }
    let v = post(cfg, "/prices", &form).await?;
    Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string())
}

/// One-time checkout parameters shared by gifts and donations.
struct OneTimeParams<'a> {
    customer_email: Option<&'a str>,
    success_url: &'a str,
    cancel_url: &'a str,
    /// Extra session metadata, sent back verbatim on the webhook.
    metadata: Vec<(String, String)>,
}

/// Start a gift checkout: one payment of `amount_cents` for a named product
/// ("Gift subscription — {tier} (3 months)"), the shape Ghost's
/// `createGiftCheckoutSession` builds. The price is inline `price_data`
/// because a gift is a bespoke line item, not a reusable plan.
pub async fn create_gift_checkout_session(
    cfg: &StripeConfig,
    amount_cents: i64,
    currency: &str,
    product_name: &str,
    customer_email: Option<&str>,
    success_url: &str,
    cancel_url: &str,
    gift_token: &str,
) -> Result<String> {
    let p = OneTimeParams {
        customer_email,
        success_url,
        cancel_url,
        metadata: vec![("ghost_gift".to_string(), gift_token.to_string())],
    };
    let form = one_time_form(&p, "pay");
    let mut form = form;
    form.push(("line_items[0][quantity]".to_string(), "1".to_string()));
    form.push(("line_items[0][price_data][currency]".to_string(), currency.to_lowercase()));
    form.push(("line_items[0][price_data][unit_amount]".to_string(), amount_cents.to_string()));
    form.push((
        "line_items[0][price_data][product_data][name]".to_string(),
        product_name.to_string(),
    ));
    let v = post(cfg, "/checkout/sessions", &form).await?;
    checkout_url(&v)
}

/// Start a donation checkout against the site's donation price. The donor
/// picks the amount on Stripe's page; the optional personal-note field is
/// Ghost's `donation_message` custom field.
pub async fn create_donation_checkout_session(
    cfg: &StripeConfig,
    price_id: &str,
    customer_email: Option<&str>,
    success_url: &str,
    cancel_url: &str,
    member_uuid: &str,
    note_label: &str,
) -> Result<String> {
    let p = OneTimeParams {
        customer_email,
        success_url,
        cancel_url,
        metadata: vec![
            ("ghost_donation".to_string(), "true".to_string()),
            ("member_uuid".to_string(), member_uuid.to_string()),
        ],
    };
    let form = one_time_form(&p, "pay");
    let mut form = form;
    form.push(("line_items[0][price]".to_string(), price_id.to_string()));
    form.push(("line_items[0][quantity]".to_string(), "1".to_string()));
    // Ghost labels the field "Add a personal note" unless the donor typed a
    // note into Portal, which passes it through as the label's text.
    form.push(("custom_fields[0][key]".to_string(), "donation_message".to_string()));
    form.push(("custom_fields[0][type]".to_string(), "text".to_string()));
    form.push(("custom_fields[0][optional]".to_string(), "true".to_string()));
    form.push(("custom_fields[0][label][type]".to_string(), "custom".to_string()));
    form.push(("custom_fields[0][label][custom]".to_string(), note_label.to_string()));
    let v = post(cfg, "/checkout/sessions", &form).await?;
    checkout_url(&v)
}

fn one_time_form(p: &OneTimeParams<'_>, submit_type: &str) -> Vec<(String, String)> {
    let mut form = vec![
        ("mode".to_string(), "payment".to_string()),
        ("submit_type".to_string(), submit_type.to_string()),
        ("success_url".to_string(), p.success_url.to_string()),
        ("cancel_url".to_string(), p.cancel_url.to_string()),
    ];
    if let Some(email) = p.customer_email.filter(|e| !e.is_empty()) {
        form.push(("customer_email".to_string(), email.to_string()));
    }
    for (k, v) in &p.metadata {
        form.push((format!("metadata[{k}]"), v.clone()));
    }
    form
}

fn checkout_url(v: &serde_json::Value) -> Result<String> {
    v.get("url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| GhostError::BadRequest("Stripe did not return a checkout URL".into()))
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
    // `url_history` is where the buyer had been before checkout, for
    // attributing the subscription. It goes on the subscription rather than
    // the session because Stripe does not copy session metadata onto the
    // subscription, and the subscription is what the webhook reads.
    url_history: Option<&str>,
) -> Result<String> {
    let mut form = subscription_checkout_form(price_id, customer_email, success_url, cancel_url, member_uuid, trial_days, offer);
    if let Some(history) = url_history.filter(|h| !h.is_empty()) {
        form.push(("subscription_data[metadata][ghost_url_history]".to_string(), history.to_string()));
    }
    let v = post(cfg, "/checkout/sessions", &form).await?;
    v.get("url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| GhostError::BadRequest("Stripe did not return a checkout URL".into()))
}

#[allow(clippy::too_many_arguments)]
fn subscription_checkout_form(
    price_id: &str, customer_email: Option<&str>, success_url: &str, cancel_url: &str,
    member_uuid: &str, trial_days: u32, offer: Option<&CheckoutOffer<'_>>,
) -> Vec<(String, String)> {
    let mut form = vec![
        ("payment_method_types[0]".to_string(), "card".to_string()),
        ("managed_payments[enabled]".to_string(), "false".to_string()),
        ("automatic_tax[enabled]".to_string(), "false".to_string()),
        ("subscription_data[trial_from_plan]".to_string(), "true".to_string()),
        ("subscription_data[items][0][plan]".to_string(), price_id.to_string()),
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
        form.retain(|(key, _)| key != "subscription_data[trial_from_plan]");
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
    form
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
    let mut form = vec![("url".to_string(), url.to_string()), ("api_version".to_string(), STRIPE_API_VERSION.to_string())];
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
        .header("Stripe-Version", STRIPE_API_VERSION)
        .send()
        .await
        .map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(resp).await?;
    Ok(())
}

pub async fn get_subscription(cfg: &StripeConfig, id: &str) -> Result<serde_json::Value> {
    get(cfg, &format!("/subscriptions/{id}")).await
}

/// Update an existing subscription using Stripe's pinned API contract.
pub async fn update_subscription(cfg: &StripeConfig, id: &str, form: &[(String, String)]) -> Result<serde_json::Value> {
    post(cfg, &format!("/subscriptions/{id}"), form).await
}

pub async fn cancel_subscription(cfg: &StripeConfig, id: &str) -> Result<serde_json::Value> {
    let response = client().delete(format!("{API}/subscriptions/{id}"))
        .basic_auth(&cfg.secret_key, None::<&str>).header("Stripe-Version", STRIPE_API_VERSION)
        .send().await.map_err(|e| GhostError::BadRequest(format!("Stripe request failed: {e}")))?;
    parse(response).await
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
        .header("Stripe-Version", STRIPE_API_VERSION)
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
    fn checkout_parameters_match_the_installed_ghost_673_contract() {
        assert_eq!(STRIPE_API_VERSION, "2020-08-27");
        let form: std::collections::HashMap<_, _> = subscription_checkout_form("price_fixture", Some("member@example.test"),
            "https://site.test/success", "https://site.test/cancel", "member-fixture", 0, None).into_iter().collect();
        assert_eq!(form["subscription_data[items][0][plan]"], "price_fixture");
        assert_eq!(form["subscription_data[trial_from_plan]"], "true");
        assert_eq!(form["managed_payments[enabled]"], "false");
        assert_eq!(form["payment_method_types[0]"], "card");
        assert!(!form.keys().any(|key| key.starts_with("branding_settings")));
        let trial: std::collections::HashMap<_, _> = subscription_checkout_form("price_fixture", None,
            "https://site.test/success", "https://site.test/cancel", "member-fixture", 7, None).into_iter().collect();
        assert_eq!(trial["subscription_data[trial_period_days]"], "7");
        assert!(!trial.contains_key("subscription_data[trial_from_plan]"));
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
        checkout_design: None,
        };
        assert!(mk("sk_live_abc").is_live());
        assert!(!mk("sk_test_abc").is_live());
    }
}

/// Preview endpoints are isolated from the pinned membership API version.
pub async fn machine_deposit_address(cfg:&StripeConfig,network:&str)->Result<serde_json::Value>{
    if !matches!(network,"base"|"tempo"){return Err(GhostError::BadRequest("Unsupported settlement network".into()))}
    let response=client().post(format!("{API}/crypto/deposit_addresses")).basic_auth(&cfg.secret_key,None::<&str>)
        .header("Stripe-Version","2026-05-27.preview").form(&[("network",network)]).send().await.map_err(|e|GhostError::BadRequest(e.to_string()))?;parse(response).await
}
pub async fn machine_payment_intent(cfg:&StripeConfig,token:&str,amount:i64,currency:&str,idempotency_key:&str,post_id:&str)->Result<serde_json::Value>{
    let form=vec![("amount".to_string(),amount.to_string()),("currency".into(),currency.to_lowercase()),("confirm".into(),"true".into()),("automatic_payment_methods[enabled]".into(),"true".into()),("automatic_payment_methods[allow_redirects]".into(),"never".into()),("shared_payment_granted_token".into(),token.into()),("metadata[ghost_machine_payment_post_id]".into(),post_id.into())];
    let response=client().post(format!("{API}/payment_intents")).basic_auth(&cfg.secret_key,None::<&str>).header("Stripe-Version","2026-05-27.preview").header("Idempotency-Key",idempotency_key).form(&form).send().await.map_err(|_|GhostError::BadRequest("Payment processor could not be reached".into()))?;
    // The persistent receipt constraint in the caller permits recovery after a
    // timeout while still granting an individual settlement only once.
    parse(response).await
}

pub async fn checkout_branding(cfg:&StripeConfig,site_url:&str)->Result<serde_json::Value>{
    let form=vec![("mode".into(),"setup".into()),("managed_payments[enabled]".into(),"false".into()),("payment_method_types[0]".into(),"card".into()),("success_url".into(),site_url.into()),("metadata[ghost_checkout_preview]".into(),"true".into())];
    let session=post_raw(cfg,"/checkout/sessions",&form).await?;
    if let Some(id)=session["id"].as_str(){let _=post_raw(cfg,&format!("/checkout/sessions/{id}/expire"),&[]).await;}
    session.get("branding_settings").cloned().ok_or_else(||GhostError::BadRequest("Stripe did not return checkout branding".into()))
}
pub async fn checkout_preview(cfg:&StripeConfig,price_id:&str,site_url:&str)->Result<String>{
    let mut form=subscription_checkout_form(price_id,None,site_url,site_url,"",0,None);
    form.push(("metadata[ghost_checkout_preview]".into(),"true".into()));form.push(("expires_at".into(),(chrono::Utc::now().timestamp()+31*60).to_string()));
    let session=post(cfg,"/checkout/sessions",&form).await?;session["url"].as_str().map(str::to_owned).ok_or_else(||GhostError::BadRequest("Stripe returned no checkout page".into()))
}

pub async fn create_subscription(cfg: &StripeConfig, customer: &str, price: &str) -> Result<serde_json::Value> {
    post(cfg,"/subscriptions",&[("customer".into(),customer.into()),("items[0][price]".into(),price.into()),("payment_behavior".into(),"error_if_incomplete".into())]).await
}
