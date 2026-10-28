//! `/members/*` — the backend Portal talks to.
//!
//! Ghost mounts this as a separate Express app (`core/server/web/members/`)
//! and Portal is hardcoded to find it at `{siteUrl}/members/api/...`, so the
//! paths here are a contract with the CDN script, not a design choice.
//!
//! Sessions follow Ghost's `members-ssr`: a cookie named `ghost-members-ssr`
//! holding the member's `transient_id`, signed with the site's
//! `theme_session_secret`. Nothing in the cookie is secret — the signature is
//! what makes it unforgeable — and because the payload is the transient id
//! rather than the member id, signing out can invalidate every existing
//! session for a member by rotating that one column.
//!
//! What this port does not have is Stripe, so the paid flows
//! (`create-stripe-*`, subscription updates) answer with an explicit error
//! rather than a silent failure. Free membership — signup, signin, account,
//! newsletter preferences, signout — works end to end.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use ghost_core::ids;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::AppState;
#[path = "member_subscriptions.rs"]
mod subscriptions;

/// Ghost's cookie name — Portal does not read it (it is `httpOnly`), but
/// keeping the name identical means an existing Ghost install's debugging
/// knowledge transfers.
const SESSION_COOKIE: &str = "ghost-members-ssr";
/// Ghost's `cookieMaxAge` default: 184 days.
const SESSION_MAX_AGE: i64 = 184 * 24 * 60 * 60;
/// Ghost's integrity token lifetime (`tokenDuration`), 5 minutes.
const INTEGRITY_TOKEN_MS: i64 = 5 * 60 * 1000;
/// Magic links expire after 24h, as in Ghost's `single-use-token-provider`.
const MAGIC_TOKEN_VALIDITY_SECS: i64 = 24 * 60 * 60;

/// Paths are absolute rather than relative-plus-`nest`, because a nested
/// router's own `/` route loses to the frontend's `/{*path}` catch-all and the
/// magic-link landing at `/members/` would 404.
pub fn members_router() -> Router<AppState> {
    Router::new()
        // The magic-link landing. Ghost applies this to every /members route
        // via middleware; the link itself always points at `/members/`.
        .route("/members/", get(magic_link_landing))
        .route("/members", get(magic_link_landing))
        .route("/members/api/site", get(read_site))
        .route("/members/api/site/", get(read_site))
        .route("/members/api/integrity-token", get(integrity_token))
        .route("/members/api/integrity-token/", get(integrity_token))
        .route("/members/api/offers/{id}", get(read_offer))
        .route("/members/api/offers/{id}/", get(read_offer))
        // Gift redemption, keyed by the token in `/#/portal/gift/redeem/{t}`.
        // Both halves are mounted at the same path, like Ghost's members app.
        .route(
            "/members/api/gifts/{token}/redeem",
            get(gift_get_redeemable).post(gift_redeem),
        )
        .route(
            "/members/api/gifts/{token}/redeem/",
            get(gift_get_redeemable).post(gift_redeem),
        )
        .route("/members/api/member/metafields", get(get_metafields).put(put_metafields))
        .route("/members/api/member/metafields/", get(get_metafields).put(put_metafields))
        .route("/members/api/send-magic-link", post(send_magic_link))
        .route("/members/api/send-magic-link/", post(send_magic_link))
        .route("/members/api/session", get(get_identity).delete(delete_session))
        .route("/members/api/session/", get(get_identity).delete(delete_session))
        // `member/newsletters` must be declared before `member` so the more
        // specific path wins.
        .route("/members/api/member/newsletters", get(get_member_newsletters).put(put_member_newsletters))
        .route("/members/api/member/newsletters/", get(get_member_newsletters).put(put_member_newsletters))
        .route("/members/api/member/suppression", delete(delete_suppression))
        .route("/members/api/member/suppression/", delete(delete_suppression))
        // The retention offers Portal shows a paid member on
        // `#/portal/account/plans` when they move to cancel. Declared before
        // the bare `member` route, like `member/newsletters` above.
        .route("/members/api/member/offers", get(get_member_offers))
        .route("/members/api/member/offers/", get(get_member_offers))
        .route("/members/api/member", get(get_member).put(update_member))
        .route("/members/api/member/", get(get_member).put(update_member))
        // Audience feedback, from the `#/feedback/{post}/{score}` buttons in a
        // newsletter. Authenticated by session or by the signed `uuid`+`key`
        // pair in the link, since it is followed from an email client.
        .route("/members/api/feedback", post(add_feedback))
        .route("/members/api/feedback/", post(add_feedback))
        // `#/portal/recommendations` reports what a visitor did with a
        // recommendation through `navigator.sendBeacon`, which cannot read a
        // response — so these answer 204 and never make the caller wait.
        .route("/members/api/recommendations/{id}/clicked", post(recommendation_clicked))
        .route("/members/api/recommendations/{id}/clicked/", post(recommendation_clicked))
        .route("/members/api/recommendations/{id}/subscribed", post(recommendation_subscribed))
        .route("/members/api/recommendations/{id}/subscribed/", post(recommendation_subscribed))
        // Paid flows need Stripe, which this port has no credentials for.
        .route("/members/api/create-stripe-checkout-session", post(create_checkout_session))
        .route("/members/api/create-stripe-checkout-session/", post(create_checkout_session))
        .route("/members/api/create-stripe-update-session", post(create_billing_portal))
        .route("/members/api/create-stripe-update-session/", post(create_billing_portal))
        .route("/members/api/create-stripe-billing-portal-session", post(create_billing_portal))
        .route("/members/api/create-stripe-billing-portal-session/", post(create_billing_portal))
        // Stripe posts subscription changes here. Not under /api/ because
        // Ghost mounts it at /members/webhooks/stripe.
        .route("/members/webhooks/stripe", post(stripe_webhook))
        .route("/members/webhooks/stripe/", post(stripe_webhook))
        .route("/members/api/subscriptions/{id}", put(subscriptions::update))
        .route("/members/api/subscriptions/{id}/", put(subscriptions::update))
        .route("/members/api/subscriptions/{id}/apply-offer", post(subscriptions::apply_offer))
        .route("/members/api/subscriptions/{id}/apply-offer/", post(subscriptions::apply_offer))
}

// ---------------------------------------------------------------------------
// responses
// ---------------------------------------------------------------------------

/// Members endpoints must never be cached — Ghost sets `private` on the whole
/// app, since every response is scoped to one visitor's session.
fn private_json(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "private, no-cache, no-store")], axum::Json(body)).into_response()
}

fn private_text(status: StatusCode, body: String) -> Response {
    (status, [(header::CACHE_CONTROL, "private, no-cache, no-store")], body).into_response()
}

/// Ghost's JSON error envelope. Portal parses `errors[0].message` and shows it
/// verbatim in the modal, so these strings are visitor-facing.
fn api_error(status: StatusCode, message: &str, error_type: &str) -> Response {
    private_json(
        status,
        json!({ "errors": [{ "message": message, "type": error_type, "errorType": error_type }] }),
    )
}

fn no_content() -> Response {
    (StatusCode::NO_CONTENT, [(header::CACHE_CONTROL, "private, no-cache, no-store")]).into_response()
}

// ---------------------------------------------------------------------------
// Stripe: checkout, billing portal and the webhook that settles them
// ---------------------------------------------------------------------------

fn stripe_cfg(state: &AppState) -> Option<ghost_stripe::StripeConfig> {
    ghost_stripe::StripeConfig::from_settings(&state.settings)
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct CheckoutRequest {
    #[serde(rename = "priceId")]
    price_id: Option<String>,
    #[serde(rename = "tierId")]
    tier_id: Option<String>,
    cadence: Option<String>,
    identity: Option<String>,
    email: Option<String>,
    /// Portal's name for the buyer's address; same field as `email` under
    /// the name Ghost's controller reads (`req.body.customerEmail`).
    #[serde(rename = "customerEmail")]
    customer_email: Option<String>,
    #[serde(rename = "successUrl")]
    success_url: Option<String>,
    #[serde(rename = "cancelUrl")]
    cancel_url: Option<String>,
    metadata: Option<Value>,
    /// Portal sends the offer's id when a visitor arrives on an offer URL.
    #[serde(rename = "offerId")]
    offer_id: Option<String>,
    /// Which flow this is. Ghost's controller names it the same way and
    /// defaults to a subscription when the field is absent.
    #[serde(rename = "type")]
    kind: Option<String>,
    // --- gift purchase fields (Portal's `#/portal/gift` screen) ---
    /// Total months of access: 1, 3, 6 or 12 (a year).
    duration: Option<u32>,
    #[serde(rename = "deliveryMethod")]
    delivery_method: Option<String>,
    #[serde(rename = "deliveryDate")]
    delivery_date: Option<String>,
    #[serde(rename = "recipientEmail")]
    recipient_email: Option<String>,
    #[serde(rename = "recipientName")]
    recipient_name: Option<String>,
    #[serde(rename = "buyerName")]
    buyer_name: Option<String>,
    #[serde(rename = "personalMessage")]
    personal_message: Option<String>,
    /// A note the donor typed into Portal's tips screen; it becomes the
    /// label on Stripe's "personal note" field.
    #[serde(rename = "personalNote")]
    personal_note: Option<String>,
}

/// Start a paid signup. Portal posts the tier and cadence it wants and
/// expects a URL to send the visitor to.
async fn create_checkout_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<CheckoutRequest>,
) -> Response {
    match req.kind.as_deref() {
        // One-time flows. Ghost's controller dispatches on the same field;
        // the default when nothing is said is a subscription.
        Some("gift") => return gift_checkout(&state, &headers, &req).await,
        Some("donation") => return donation_checkout(&state, &headers, &req).await,
        Some("subscription") | None | Some("") => {}
        Some(other) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                &format!("Unsupported checkout type \"{other}\"."),
                "BadRequestError",
            );
        }
    }
    let Some(cfg) = stripe_cfg(&state) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "This site is not accepting payments at the moment.",
            "BadRequestError",
        );
    };

    // Resolve a real configured tier before talking to Stripe. Portal sends
    // a tier id and cadence, including on sites connected before prices existed.
    let yearly = match req.cadence.as_deref() {
        None | Some("month" | "monthly") => false,
        Some("year" | "yearly") => true,
        _ => return api_error(StatusCode::BAD_REQUEST, "Unknown billing cadence.", "BadRequestError"),
    };
    type Tier = (String, String, Option<String>, Option<i64>, Option<i64>, Option<String>, Option<String>, i64);
    let columns = "id, name, currency, monthly_price, yearly_price, monthly_price_id, yearly_price_id, trial_days";
    let mut query = format!("SELECT {columns} FROM products WHERE type = 'paid' AND active = 1 AND visibility = 'public'");
    let selector = if let Some(tier) = &req.tier_id {
        query.push_str(" AND id = ?");
        Some(tier.as_str())
    } else if let Some(price) = &req.price_id {
        query.push_str(" AND (monthly_price_id = ? OR yearly_price_id = ?)");
        Some(price.as_str())
    } else { None };
    query.push_str(" ORDER BY created_at LIMIT 1");
    let mut sql = sqlx::query_as::<_, Tier>(&query);
    if let Some(selector) = selector {
        sql = sql.bind(selector);
        if req.tier_id.is_none() { sql = sql.bind(selector); }
    }
    let tier = match sql.fetch_optional(&state.db).await {
        Ok(Some(tier)) => tier,
        Ok(None) => return api_error(StatusCode::BAD_REQUEST, "That plan is not available for purchase.", "BadRequestError"),
        Err(error) => {
            tracing::error!("members: could not resolve checkout tier: {error}");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not load the selected plan.", "InternalServerError");
        }
    };
    let yearly = if req.tier_id.is_none() && req.price_id.is_some() {
        req.price_id.as_deref() == tier.6.as_deref()
    } else { yearly };
    let amount = if yearly { tier.4 } else { tier.3 };
    if !amount.is_some_and(|amount| amount > 0) {
        return api_error(StatusCode::BAD_REQUEST, "That billing option is not available for purchase.", "BadRequestError");
    }
    let mut price_id = if yearly { tier.6.clone() } else { tier.5.clone() };
    if price_id.as_deref().is_none_or(str::is_empty) {
        if let Err(error) = crate::stripe_tiers::sync_tier(&state, &cfg, &tier.0, &tier.1,
            tier.2.as_deref().unwrap_or("usd"), tier.3, tier.4).await {
            tracing::error!("stripe: tier price setup failed: {}", error.message());
            return api_error(StatusCode::BAD_REQUEST, "This site is not accepting payments at the moment.", "BadRequestError");
        }
        let column = if yearly { "yearly_price_id" } else { "monthly_price_id" };
        price_id = sqlx::query_scalar::<_, Option<String>>(&format!("SELECT {column} FROM products WHERE id = ?"))
            .bind(&tier.0).fetch_one(&state.db).await.ok().flatten();
    }
    let Some(price_id) = price_id.filter(|price| price.starts_with("price_")) else {
        return api_error(StatusCode::BAD_REQUEST, "That plan is not available for purchase yet.", "BadRequestError");
    };

    // An existing session identifies the buyer; otherwise Portal supplies the
    // email typed into the signup form.
    let member = current_member(&state, &headers).await;
    let email = member
        .as_ref()
        .map(|m| m.email.clone())
        .or_else(|| req.email.clone().filter(|e| !e.is_empty()).or_else(|| req.customer_email.clone()))
        .unwrap_or_default();
    let uuid = member.as_ref().map(|m| m.uuid.clone()).unwrap_or_default();

    let site_url = state.site_url();
    let success = req
        .success_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/?action=signup&success=true&stripe=success"));
    let cancel = req
        .cancel_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/?stripe=cancelled"));

    let trial = tier.7;
    let _ = &req.identity;
    let _ = &req.metadata;

    // An offer, if the visitor came in on one. Only an active offer for the
    // tier being bought applies — a code for another tier must not quietly
    // discount this one.
    let offer = match req.offer_id.as_deref().filter(|o| !o.is_empty()) {
        Some(id) => match ghost_db::repos::offers::find_by_id(&state.db, id).await.ok().flatten() {
            Some(o) if o.active => Some(o),
            _ => None,
        },
        None => None,
    };
    let offer = offer.filter(|offer| offer.product_id == tier.0);
    let checkout_offer = offer.as_ref().map(|o| ghost_stripe::CheckoutOffer {
        offer_id: &o.id,
        coupon_id: o.stripe_coupon_id.as_deref(),
        trial_days: (o.discount_type.as_deref() == Some("trial"))
            .then(|| o.discount_amount.unwrap_or(0).max(0) as u32),
    });

    match ghost_stripe::create_checkout_session(
        &cfg,
        &price_id,
        Some(&email),
        &success,
        &cancel,
        &uuid,
        trial.max(0) as u32,
        checkout_offer.as_ref(),
    )
    .await
    {
        Ok(url) => {
            let _ = state.settings.set(&state.db, "stripe_checkout_setup_error", &json!("")).await;
            private_json(StatusCode::OK, json!({ "url": url }))
        },
        Err(e) => {
            tracing::error!("stripe: checkout session failed: {}", e.message());
            let detail = if e.message().contains("account or business name") {
                "Stripe requires an account or business name. Set it in your Stripe dashboard's business details, then retry checkout."
            } else {
                "Stripe could not open checkout. Check the server log for its error and confirm your account setup is complete."
            };
            let _ = state.settings.set(&state.db, "stripe_checkout_setup_error", &json!(detail)).await;
            // Portal translates a fixed set of API messages and otherwise
            // replaces them with its generic signup failure.
            api_error(StatusCode::BAD_REQUEST, "This site is not accepting payments at the moment.", "BadRequestError")
        }
    }
}

/// Open Stripe's billing portal so a member can change or cancel their plan.
async fn create_billing_portal(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(cfg) = stripe_cfg(&state) else {
        return api_error(StatusCode::BAD_REQUEST, "Payments are not set up on this site.", "BadRequestError");
    };
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
    };
    let customer: Option<String> =
        sqlx::query_scalar("SELECT customer_id FROM members_stripe_customers WHERE member_id = ? LIMIT 1")
            .bind(&member.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some(customer) = customer else {
        return api_error(StatusCode::BAD_REQUEST, "You do not have a billing account yet.", "BadRequestError");
    };
    match ghost_stripe::create_billing_portal_session(&cfg, &customer, &state.site_url()).await {
        Ok(url) => private_json(StatusCode::OK, json!({ "url": url })),
        Err(e) => api_error(StatusCode::BAD_REQUEST, &e.message(), "BadRequestError"),
    }
}

// ---------------------------------------------------------------------------
// gifts — the `/#/portal/gift` flow
// ---------------------------------------------------------------------------

/// Ghost's gift catalogue (`gift-checkout-offer.ts`), which Portal's own
/// picker mirrors: a duration is *total months*, and 12 is sold as a year.
/// (total months, cadence, billing duration, price multiplier)
const GIFT_DURATIONS: &[(u32, &str, u32, u32)] = &[
    (1, "month", 1, 1),
    (3, "month", 3, 3),
    (6, "month", 6, 6),
    (12, "year", 1, 1),
];

/// Resolve what a gift costs: the catalogue entry for the asked duration,
/// checked against the cadence when one was given.
fn resolve_gift_duration(
    duration: Option<u32>,
    cadence: Option<&str>,
) -> Result<&'static (u32, &'static str, u32, u32), String> {
    let total = match duration {
        Some(d) => d,
        // A cadence with no duration is the pre-duration Portal request
        // shape, which Ghost still honours with the shortest matching gift.
        None => match cadence {
            Some("month") => 1,
            Some("year") | Some("yearly") => 12,
            _ => return Err("Expected a supported gift duration or legacy cadence".into()),
        },
    };
    let entry = GIFT_DURATIONS
        .iter()
        .find(|(months, _, _, _)| *months == total)
        .ok_or_else(|| format!("Unsupported gift duration \"{total}\""))?;
    if let Some(c) = cadence.filter(|c| *c == "month" || *c == "year" || *c == "yearly") {
        let c = if c == "yearly" { "year" } else { c };
        if c != entry.1 {
            return Err(format!("Gift duration \"{total}\" conflicts with cadence \"{c}\""));
        }
    }
    Ok(entry)
}

/// A paid tier as a gift needs to see it: its prices and its public face.
struct GiftTier {
    id: String,
    name: String,
    currency: String,
    monthly_price: Option<i64>,
    yearly_price: Option<i64>,
}

async fn gift_tier(state: &AppState, id: &str) -> Option<GiftTier> {
    sqlx::query_as::<_, (String, String, Option<String>, Option<i64>, Option<i64>)>(
        "SELECT id, name, currency, monthly_price, yearly_price \
         FROM products WHERE id = ? AND active = 1 AND visibility = 'public' AND type = 'paid' LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(|(id, name, currency, monthly_price, yearly_price)| GiftTier {
        id,
        name,
        currency: currency.unwrap_or_default(),
        monthly_price,
        yearly_price,
    })
}

/// Ghost's gift token: 12 characters of [A-Za-z0-9] off the CSPRNG — short
/// enough to read out, long enough (~71 bits of entropy) to be unguessable.
fn gift_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..12)
        .map(|_| ALPHABET[ids::random_below(ALPHABET.len() as u32) as usize] as char)
        .collect()
}

/// A purchased-not-yet-redeemed gift, stored as a `tokens` row like a magic
/// link. The `gift: true` marker is what Danger zone's "Reset all gift
/// links" keys on, and the token column is what the redeem URL carries.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Gift {
    gift: bool,
    token: String,
    tier_id: String,
    tier_name: String,
    cadence: String,
    /// Billing duration — 3 for a three-month gift, 1 for a year.
    duration: u32,
    /// Total months of access granted on redemption.
    total_months: u32,
    currency: String,
    amount: i64,
    buyer_email: String,
    #[serde(default)]
    buyer_name: Option<String>,
    #[serde(default)]
    recipient_email: Option<String>,
    #[serde(default)]
    recipient_name: Option<String>,
    #[serde(default)]
    personal_message: Option<String>,
    /// `link` (buyer shares it themselves) or `email` (we deliver it).
    delivery: String,
    #[serde(default)]
    delivery_at: Option<String>,
    /// `payment_pending` → `paid` → `redeemed`.
    status: String,
    #[serde(default)]
    purchased_at: Option<String>,
    #[serde(default)]
    redeemed_by: Option<String>,
    #[serde(default)]
    redeemed_at: Option<String>,
}

async fn insert_gift(state: &AppState, gift: &Gift) -> bool {
    let now = now_rfc3339();
    sqlx::query(
        "INSERT INTO tokens (id, token, uuid, data, created_at, updated_at, used_count, otc_used_count) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0)",
    )
    .bind(ids::new_id())
    .bind(&gift.token)
    .bind(ids::uuid_v4())
    .bind(serde_json::to_string(gift).unwrap_or_default())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .is_ok()
}

async fn find_gift(state: &AppState, token: &str) -> Option<Gift> {
    let data: Option<String> = sqlx::query_scalar("SELECT data FROM tokens WHERE token = ? LIMIT 1")
        .bind(token)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    data.and_then(|d| {
        let g: Gift = serde_json::from_str(&d).ok()?;
        g.gift.then_some(g)
    })
}

async fn update_gift(state: &AppState, gift: &Gift) {
    let _ = sqlx::query("UPDATE tokens SET data = ?, updated_at = ? WHERE token = ?")
        .bind(serde_json::to_string(gift).unwrap_or_default())
        .bind(now_rfc3339())
        .bind(&gift.token)
        .execute(&state.db)
        .await;
}

/// Append query parameters to a URL *before* its `#` fragment — `?a=b#x`, not
/// `#x?a=b` — the way a URL library would. Portal's success URLs are hash
/// routes, so naively appending would hide the params from it.
fn add_query_params(base: &str, params: &[( &str, String )]) -> String {
    let (before_hash, hash) = match base.split_once('#') {
        Some((a, b)) => (a, format!("#{b}")),
        None => (base, String::new()),
    };
    let mut out = before_hash.to_string();
    let mut sep = if before_hash.contains('?') { '&' } else { '?' };
    for (k, v) in params {
        out.push(sep);
        sep = '&';
        out.push_str(k);
        out.push('=');
        out.push_str(&percent_encoding::utf8_percent_encode(v, percent_encoding::NON_ALPHANUMERIC).to_string());
    }
    out.push_str(&hash);
    out
}

/// "{duration_label}" for the checkout line item and emails: Ghost's
/// `'{count} month'` / `'{count} year'` with i18next plurals.
fn gift_duration_label(total_months: u32) -> String {
    if total_months == 12 {
        "1 year".to_string()
    } else if total_months == 1 {
        "1 month".to_string()
    } else {
        format!("{total_months} months")
    }
}

/// The `#/portal/gift` purchase: one payment for a fixed term of a tier,
/// minting a token the recipient redeems later. Ghost creates the gift row
/// first and deletes it if Stripe refuses; here the row is written only
/// after Stripe accepts, so a failed session leaves nothing behind.
async fn gift_checkout(state: &AppState, headers: &HeaderMap, req: &CheckoutRequest) -> Response {
    let Some(cfg) = stripe_cfg(state) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "This site is not set up to take payments yet.",
            "BadRequestError",
        );
    };

    let Some(tier_id) = req.tier_id.clone().filter(|t| !t.is_empty()) else {
        return api_error(StatusCode::BAD_REQUEST, "Expected a tier to buy the gift on.", "BadRequestError");
    };
    let entry = match resolve_gift_duration(req.duration, req.cadence.as_deref()) {
        Ok(e) => e,
        Err(context) => return api_error(StatusCode::BAD_REQUEST, &context, "BadRequestError"),
    };
    let (_, cadence, billing_duration, multiplier) = *entry;

    let Some(tier) = gift_tier(state, &tier_id).await else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "The requested tier is not available for gift purchases.",
            "BadRequestError",
        );
    };

    // Portal's plan gate: Ghost refuses a duration whose plan the publisher
    // has turned off in the signup portal. A duration was always sent by the
    // time this runs; an absent list is Ghost's default of both plans on.
    let portal_plans = state
        .settings
        .get("portal_plans")
        .and_then(|p| p.as_array().cloned())
        .unwrap_or_else(|| vec![json!("monthly"), json!("yearly")]);
    let plan_name = if cadence == "year" { "yearly" } else { "monthly" };
    if !portal_plans.iter().any(|p| p == plan_name) {
        return api_error(
            StatusCode::BAD_REQUEST,
            &format!("The {plan_name} Portal plan is not available"),
            "BadRequestError",
        );
    }

    let unit = if cadence == "year" { tier.yearly_price } else { tier.monthly_price };
    let Some(unit) = unit.filter(|p| *p > 0) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "The requested tier does not have a valid gift price.",
            "BadRequestError",
        );
    };
    if tier.currency.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "The requested tier does not have a valid gift price.",
            "BadRequestError",
        );
    }
    let amount = unit * multiplier as i64;

    let member = current_member(state, headers).await;
    let buyer_email = member
        .as_ref()
        .map(|m| m.email.clone())
        .or_else(|| {
            req.email
                .clone()
                .filter(|e| !e.is_empty())
                .or_else(|| req.customer_email.clone())
        })
        .filter(|e| !e.is_empty());
    let Some(buyer_email) = buyer_email else {
        return api_error(StatusCode::BAD_REQUEST, "A buyer email is required.", "BadRequestError");
    };

    let delivery = match req.delivery_method.as_deref() {
        Some("email") => {
            if req.recipient_email.as_deref().unwrap_or("").trim().is_empty() {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "Email delivery needs a recipient address.",
                    "BadRequestError",
                );
            }
            "email"
        }
        _ => "link",
    };

    let delivery_at = match scheduled_gift_date(req.delivery_date.as_deref(), &state.settings.get_str("timezone").unwrap_or_else(||"Etc/UTC".into())) {
        Ok(date)=>date, Err(message)=>return api_error(StatusCode::BAD_REQUEST,&message,"BadRequestError")
    };
    let token = gift_token();
    let site_url = state.site_url();
    let success = req
        .success_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/"));
    let cancel = req
        .cancel_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/"));
    // Portal's success screen reads these to show the buyer their gift link
    // (link delivery) or say the email is on its way.
    let success = add_query_params(
        &success,
        &[
            ("stripe", "gift-purchase-success".to_string()),
            ("gift_token", token.clone()),
            ("gift_tier", tier.id.clone()),
            ("gift_cadence", cadence.to_string()),
            ("gift_delivery", delivery.to_string()),
            ("gift_duration", entry.0.to_string()),
        ],
    );

    let product_name = format!("Gift subscription — {} ({})", tier.name, gift_duration_label(entry.0));
    match ghost_stripe::create_gift_checkout_session(
        &cfg,
        amount,
        &tier.currency,
        &product_name,
        Some(&buyer_email),
        &success,
        &cancel,
        &token,
    )
    .await
    {
        Ok(url) => {
            let gift = Gift {
                gift: true,
                token,
                tier_id: tier.id,
                tier_name: tier.name,
                cadence: cadence.to_string(),
                duration: billing_duration,
                total_months: entry.0,
                currency: tier.currency.to_lowercase(),
                amount,
                buyer_email: buyer_email.clone(),
                buyer_name: req.buyer_name.clone().filter(|n| !n.trim().is_empty()),
                recipient_email: req.recipient_email.clone().filter(|e| !e.trim().is_empty()),
                recipient_name: req.recipient_name.clone().filter(|n| !n.trim().is_empty()),
                personal_message: req.personal_message.clone().filter(|m| !m.trim().is_empty()),
                delivery: delivery.to_string(),
                delivery_at,
                status: "payment_pending".to_string(),
                ..Default::default()
            };
            if !insert_gift(state, &gift).await {
                tracing::error!("members: paid gift checkout for {buyer_email} could not be recorded");
            }
            private_json(StatusCode::OK, json!({ "url": url }))
        }
        Err(e) => {
            tracing::error!("stripe: gift checkout session failed: {}", e.message());
            api_error(StatusCode::BAD_REQUEST, "Unable to start checkout. Please try again.", "BadRequestError")
        }
    }
}

/// Payment landed for a gift: mark it redeemable and deliver it. Email
/// delivery sends the recipient their gift; link delivery confirms to the
/// buyer, repeating the link so it cannot be lost with the browser tab.
async fn gift_payment_completed(state: &AppState, token: &str) -> ghost_core::Result<()> {
    let mut tx=state.db.begin().await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    let data:Option<String>=sqlx::query_scalar("SELECT data FROM tokens WHERE token=? LIMIT 1").bind(token).fetch_optional(&mut *tx).await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    let Some(mut gift)=data.and_then(|data|serde_json::from_str::<Gift>(&data).ok()).filter(|gift|gift.gift) else {
        return Err(ghost_core::GhostError::BadRequest("Gift checkout token was not found".into()));
    };
    if gift.status=="payment_pending" {
        gift.status="paid".into(); gift.purchased_at=Some(now_rfc3339());
        sqlx::query("UPDATE tokens SET data=?,updated_at=? WHERE token=?").bind(serde_json::to_string(&gift).map_err(|e|ghost_core::GhostError::Internal(e.to_string()))?).bind(now_rfc3339()).bind(token).execute(&mut *tx).await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    }
    if matches!(gift.status.as_str(),"paid"|"redeemed") {
        let deliver_at=gift.delivery_at.as_deref().or(gift.purchased_at.as_deref()).unwrap_or("");
        sqlx::query("INSERT OR IGNORE INTO scheduled_gift_deliveries (token,deliver_at) VALUES (?,?)").bind(token).bind(deliver_at).execute(&mut *tx).await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    }
    tx.commit().await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    deliver_due_gifts(state).await;
    Ok(())
}

async fn send_gift_email(
    state: &AppState,
    gift: &Gift,
    link: &str,
) -> ghost_core::Result<()> {
    let site_url = state.site_url();
    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let (to_recipient, to) = if gift.delivery == "email" {
        (true, gift.recipient_email.clone().unwrap_or_default())
    } else {
        (false, gift.buyer_email.clone())
    };
    let sender = if to_recipient {
        gift.buyer_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| gift.buyer_email.clone())
    } else {
        site_title.clone()
    };
    let (subject, text, html) = ghost_mail::templates::gift_subscription(
        to_recipient,
        &site_title,
        &site_url,
        &sender,
        &gift.tier_name,
        &gift_duration_label(gift.total_months),
        gift.personal_message.as_deref(),
        link,
    );
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &site_title, &site_url);
    let msg = ghost_mail::Message { to, subject, html, text };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => Ok(()),
        Err(e) if state.config.env == "development" => {
            tracing::warn!(
                "members: gift email for token {} could not be delivered ({}) — the redeem link is: {link}",
                gift.token,
                e.message()
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// The redemption a gift carries, in Portal's shape. Field names are Ghost's
/// `GiftRedemption`, wrapped the way its API frame wraps every docName.
async fn serialize_redemption(state: &AppState, gift: &Gift) -> Value {
    let tier: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT description, currency FROM products WHERE id = ? LIMIT 1",
    )
    .bind(&gift.tier_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let benefits: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM products_benefits WHERE product_id = ? ORDER BY sort_order ASC",
    )
    .bind(&gift.tier_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Both dates count from payment, Ghost's GIFT_EXPIRY_DAYS apart; the
    // consumption date is when the redeemer's access runs out.
    let purchased = gift
        .purchased_at
        .as_deref()
        .and_then(|p| chrono::NaiveDateTime::parse_from_str(p, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| t.and_utc())
        .unwrap_or_else(chrono::Utc::now);
    let expires_at = purchased + chrono::Duration::days(365);
    let consumes_at = gift
        .redeemed_at
        .as_deref()
        .and_then(|r| chrono::NaiveDateTime::parse_from_str(r, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| months_after(t.and_utc(), gift.total_months));

    json!({ "gifts": [{
        "token": gift.token,
        "cadence": gift.cadence,
        "duration": gift.duration,
        "currency": gift.currency,
        "amount": gift.amount,
        "buyer_name": gift.buyer_name,
        "recipient_name": gift.recipient_name,
        "recipient_email": gift.recipient_email,
        "message": gift.personal_message,
        "expires_at": expires_at.to_rfc3339(),
        "consumes_at": consumes_at.map(|d| d.to_rfc3339()),
        "tier": {
            "id": gift.tier_id,
            "name": gift.tier_name,
            "description": tier.as_ref().and_then(|(d, _)| d.clone()),
            "benefits": benefits,
        }
    }] })
}

/// Ghost adds calendar months (`setMonth(+duration)`; a year cadence is a
/// 1-of-years add), so a Feb 29 present keeps its month rather than clipping
/// to a 28-day calendar.
fn months_after(from: chrono::DateTime<chrono::Utc>, total_months: u32) -> chrono::DateTime<chrono::Utc> {
    from + chrono::Months::new(total_months)
}

/// Ghost's redeemability rules (`Gift.checkRedeemable`), with its exact
/// visitor-facing messages. `member_status` is the signed-in member's, or
/// none for the pre-signin peek Portal makes.
fn check_redeemable(gift: &Gift, member_status: Option<&str>) -> Result<(), (StatusCode, &'static str, &'static str)> {
    match gift.status.as_str() {
        "payment_pending" => {
            return Err((StatusCode::NOT_FOUND, "This gift does not exist.", "NotFoundError"));
        }
        "redeemed" => {
            return Err((StatusCode::BAD_REQUEST, "This gift has already been redeemed.", "BadRequestError"));
        }
        _ => {}
    }
    let expired = gift
        .purchased_at
        .as_deref()
        .and_then(|p| chrono::NaiveDateTime::parse_from_str(p, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| t.and_utc() + chrono::Duration::days(365) < chrono::Utc::now())
        .unwrap_or(true);
    if expired {
        return Err((StatusCode::BAD_REQUEST, "This gift has expired.", "BadRequestError"));
    }
    if member_status.is_some_and(|s| s != "free") {
        return Err((
            StatusCode::BAD_REQUEST,
            "You already have an active subscription.",
            "BadRequestError",
        ));
    }
    Ok(())
}

/// GET /members/api/gifts/{token}/redeem — what Portal shows on the redeem
/// screen. Unauthenticated on purpose: the recipient has not signed in yet.
async fn gift_get_redeemable(State(state): State<AppState>, Path(token): Path<String>, headers: HeaderMap) -> Response {
    let Some(gift) = find_gift(&state, &token).await else {
        return api_error(StatusCode::NOT_FOUND, "This gift does not exist.", "NotFoundError");
    };
    let member_status = current_member(&state, &headers).await.map(|m| m.status);
    if let Err((status, message, kind)) = check_redeemable(&gift, member_status.as_deref()) {
        return api_error(status, message, kind);
    }
    private_json(StatusCode::OK, serialize_redemption(&state, &gift).await)
}

/// POST /members/api/gifts/{token}/redeem — attach the gift's tier and term
/// to the signed-in member. Ghost requires a session here; an unsigned
/// visitor is bounced to signin by Portal.
async fn gift_redeem(State(state): State<AppState>, Path(token): Path<String>, headers: HeaderMap) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "Member authentication required.", "UnauthorizedError");
    };
    let Some(mut gift) = find_gift(&state, &token).await else {
        return api_error(StatusCode::NOT_FOUND, "This gift does not exist.", "NotFoundError");
    };
    if let Err((status, message, kind)) = check_redeemable(&gift, Some(&member.status)) {
        return api_error(status, message, kind);
    }

    let now = now_rfc3339();
    let redeemed_at = chrono::Utc::now();
    let consumes_at = months_after(redeemed_at, gift.total_months)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    // A gift replaces any tier the member had — Ghost writes a single
    // products row with an expiry — and marks the member `gift`, which reads
    // as paid to content gating.
    let _ = sqlx::query("DELETE FROM members_products WHERE member_id = ?")
        .bind(&member.id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query(
        "INSERT INTO members_products (id, member_id, product_id, sort_order, expiry_at) VALUES (?, ?, ?, 0, ?)",
    )
    .bind(ids::new_id())
    .bind(&member.id)
    .bind(&gift.tier_id)
    .bind(&consumes_at)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("UPDATE members SET status = 'gift', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&member.id)
        .execute(&state.db)
        .await;

    gift.status = "redeemed".to_string();
    gift.redeemed_by = Some(member.id);
    gift.redeemed_at = Some(now);
    update_gift(&state, &gift).await;

    private_json(StatusCode::OK, serialize_redemption(&state, &gift).await)
}

// ---------------------------------------------------------------------------
// donations — the `/#/portal/support` flow
// ---------------------------------------------------------------------------

/// The Stripe price a tip is paid against, created on first use and cached
/// in settings (Ghost caches the same in its `stripe_prices` table). The
/// price's amount is donor-set on the checkout page — `custom_unit_amount`
/// — with the site's suggested amount as the preset.
async fn donation_price(state: &AppState, cfg: &ghost_stripe::StripeConfig) -> Result<String, ghost_core::GhostError> {
    let currency = state
        .settings
        .get_str("donations_currency")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "USD".to_string());

    // {"price": "…", "product": "…", "currency": "USD"} — the currency rides
    // along so a change of heart is caught, not silently tipped in the old
    // one. A changed *suggested amount* is tolerated: the preset is only a
    // starting point, the donor can type any amount.
    let cached = state
        .settings
        .get_str("donations_price_id")
        .and_then(|v| serde_json::from_str::<Value>(&v).ok());
    if let Some(price) = cached
        .as_ref()
        .and_then(|c| c.get("price"))
        .and_then(|p| p.as_str())
        .filter(|p| !p.is_empty())
    {
        let cached_currency = cached
            .as_ref()
            .and_then(|c| c.get("currency"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if cached_currency.eq_ignore_ascii_case(&currency) {
            return Ok(price.to_string());
        }
    }

    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let nickname = format!("Support {site_title}");
    let product = cached
        .as_ref()
        .and_then(|c| c.get("product"))
        .and_then(|p| p.as_str())
        .map(str::to_string);
    let product = ghost_stripe::ensure_product(cfg, &nickname, product.as_deref()).await?;
    let preset = state
        .settings
        .get_str("donations_suggested_amount")
        .and_then(|v| v.trim().parse::<i64>().ok());
    let price = ghost_stripe::create_donation_price(cfg, &product, &currency, preset, &nickname).await?;
    let _ = state
        .settings
        .set(
            &state.db,
            "donations_price_id",
            &json!({ "price": price, "product": product, "currency": currency.to_uppercase() }),
        )
        .await;
    Ok(price)
}

/// The `#/portal/support` tip: a one-time payment with no account required
/// (though an email lets the donor be recorded as a free member, the way
/// Ghost records donors).
async fn donation_checkout(state: &AppState, headers: &HeaderMap, req: &CheckoutRequest) -> Response {
    let Some(cfg) = stripe_cfg(state) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "This site is not set up to take payments yet.",
            "BadRequestError",
        );
    };
    if state.settings.get_str("donations_currency").filter(|c| !c.is_empty()).is_none() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "Donations are not configured on this site.",
            "BadRequestError",
        );
    }

    let member = current_member(state, headers).await;
    let email = member
        .as_ref()
        .map(|m| m.email.clone())
        .or_else(|| {
            req.email
                .clone()
                .filter(|e| !e.is_empty())
                .or_else(|| req.customer_email.clone())
        })
        .filter(|e| !e.is_empty());
    let uuid = member.as_ref().map(|m| m.uuid.clone()).unwrap_or_default();

    let price = match donation_price(state, &cfg).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("stripe: donation price could not be prepared: {}", e.message());
            return api_error(
                StatusCode::BAD_REQUEST,
                "Unable to start checkout. Please try again.",
                "BadRequestError",
            );
        }
    };

    let site_url = state.site_url();
    let success = req
        .success_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/?stripe=success"));
    let cancel = req
        .cancel_url
        .clone()
        .filter(|u| u.starts_with(&site_url))
        .unwrap_or_else(|| format!("{site_url}/?stripe=cancelled"));
    let note_label = req
        .personal_note
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Add a personal note".to_string());

    match ghost_stripe::create_donation_checkout_session(
        &cfg,
        &price,
        email.as_deref(),
        &success,
        &cancel,
        &uuid,
        &note_label,
    )
    .await
    {
        Ok(url) => private_json(StatusCode::OK, json!({ "url": url })),
        Err(e) => {
            tracing::error!("stripe: donation checkout session failed: {}", e.message());
            api_error(
                StatusCode::BAD_REQUEST,
                "Unable to start checkout. Please try again.",
                "BadRequestError",
            )
        }
    }
}

/// Stripe's webhook. This is what actually turns a payment into membership —
/// the browser redirect after checkout is a convenience, not proof of payment,
/// so nothing is granted until Stripe tells us here.
async fn stripe_webhook(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let Some(cfg) = stripe_cfg(&state) else {
        return (StatusCode::BAD_REQUEST, "stripe not configured").into_response();
    };
    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // Unsigned or badly-signed payloads are refused before being parsed —
    // otherwise anyone could post themselves a paid subscription.
    if !ghost_stripe::verify_webhook_signature(&body, sig, &cfg.webhook_secret, 300) {
        tracing::warn!("stripe: rejected a webhook with an invalid signature");
        return (StatusCode::BAD_REQUEST, "invalid signature").into_response();
    }
    let event: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad payload").into_response(),
    };
    let kind = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let object = event.pointer("/data/object").cloned().unwrap_or(Value::Null);

    let result=match kind {
        "checkout.session.completed" => handle_checkout_completed(&state, &cfg, &object).await,
        "customer.subscription.created" | "customer.subscription.updated" | "customer.subscription.deleted" => {
            handle_subscription_change(&state, &object).await
        }
        _ => Ok(())
    };
    if let Err(error)=result {
        tracing::error!("Stripe event could not be persisted: {}",error.message());
        return (StatusCode::SERVICE_UNAVAILABLE,"Please retry the event").into_response();
    }
    // Acknowledge only after local persistence succeeds.
    (StatusCode::OK, "ok").into_response()
}

/// A completed checkout: find or create the member, attach the customer.
async fn handle_checkout_completed(state: &AppState, cfg: &ghost_stripe::StripeConfig, obj: &Value) -> ghost_core::Result<()> {
    if obj.pointer("/metadata/ghost_checkout_preview").is_some_and(|value|value=="true"||value==true){return Ok(())}
    // A gift's buyer is not necessarily becoming a member — the gift itself
    // is the product — so it never reaches the member logic below.
    if let Some(token) = obj
        .pointer("/metadata/ghost_gift")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
    {
        return gift_payment_completed(state, token).await;
    }
    // A donation is recorded like any checkout — the donor becomes a free
    // member when an email is known, and the customer is attached — but
    // carries no subscription, so the rest of this handler is a no-op for it.
    let email = obj
        .pointer("/customer_details/email")
        .and_then(|e| e.as_str())
        .or_else(|| obj.get("customer_email").and_then(|e| e.as_str()))
        .unwrap_or("")
        .to_lowercase();
    let customer_id = obj.get("customer").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let uuid = obj.pointer("/metadata/member_uuid").and_then(|u| u.as_str()).unwrap_or("");

    // Prefer the uuid we put in the session's metadata; fall back to the
    // address Stripe collected, which covers a signup that began logged out.
    let member = match member_by_uuid(state, uuid).await {
        Some(m) => Some(m),
        None if !email.is_empty() => match member_by_email(state, &email).await {
            Some(m) => Some(m),
            None => create_member(state, &email, None, &Value::Null).await,
        },
        None => None,
    };
    let Some(member) = member else {
        tracing::error!("stripe: completed checkout for {email} but no member could be resolved");
        return Err(ghost_core::GhostError::Internal("Checkout member could not be resolved".into()));
    };

    if !customer_id.is_empty() {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM members_stripe_customers WHERE customer_id = ?")
                .bind(&customer_id)
                .fetch_optional(&state.db)
                .await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
        if existing.is_none() {
            let now = now_rfc3339();
            sqlx::query(
                "INSERT INTO members_stripe_customers (id, member_id, customer_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(ids::new_id())
            .bind(&member.id)
            .bind(&customer_id)
            .bind(&now)
            .bind(&now)
            .execute(&state.db)
            .await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
        }
    }

    // Credit the offer the checkout ran under. Only the first one sticks:
    // Settings → Growth → Offers counts the offer a member *arrived* on, so a
    // later purchase must not rewrite that history.
    if let Some(offer_id) = obj.pointer("/metadata/offer_id").and_then(|o| o.as_str()) {
        if !offer_id.is_empty() {
            sqlx::query(
                "UPDATE members SET offer_id = ? WHERE id = ? AND (offer_id IS NULL OR offer_id = '')",
            )
            .bind(offer_id)
            .bind(&member.id)
            .execute(&state.db)
            .await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
        }
    }

    // The session carries only the subscription id, so fetch the real thing
    // rather than guessing its state.
    if let Some(sub_id) = obj.get("subscription").and_then(|s| s.as_str()) {
        let sub=ghost_stripe::get_subscription(cfg, sub_id).await?;
        handle_subscription_change(state, &sub).await?;
    }
    Ok(())
}

/// A subscription was created, changed or cancelled: record it and move the
/// member between free and paid accordingly.
async fn handle_subscription_change(state: &AppState, sub: &Value) -> ghost_core::Result<()> {
    if let Some(member)=ghost_db::repos::subscriptions::sync(&state.db,sub).await? {
        if matches!(sub["status"].as_str(),Some("active"|"trialing")) {crate::automations_engine::trigger_for_member(state,&member).await;}
    }
    Ok(())
}

async fn stripe_unavailable() -> Response {
    api_error(
        StatusCode::BAD_REQUEST,
        "Stripe is not configured on this site, so paid plans are unavailable.",
        "BadRequestError",
    )
}

// ---------------------------------------------------------------------------
// signing: HMAC-SHA256 over a payload, keyed by theme_session_secret
// ---------------------------------------------------------------------------

fn theme_secret(state: &AppState) -> String {
    state.settings.get_str("theme_session_secret").unwrap_or_default()
}

/// HMAC-SHA256, hex-encoded. Hand-rolled over `sha2` (block size 64) rather
/// than pulling in an `hmac` dependency for two call sites.
pub(crate) fn hmac_sha256_hex(key: &str, msg: &str) -> String {
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

/// Constant-time compare, so a signature check cannot be narrowed by timing.
pub(crate) fn secure_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn now_rfc3339() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ---------------------------------------------------------------------------
// session cookie
// ---------------------------------------------------------------------------

fn cookie_value(state: &AppState, transient_id: &str) -> String {
    let sig = hmac_sha256_hex(&theme_secret(state), transient_id);
    format!("{transient_id}.{sig}")
}

/// `Path` is scoped to the install's base path, not left at `/`. At a domain
/// root the two are the same thing; under a subdirectory, scoping is what lets
/// two installs on one domain hold separate member sessions instead of
/// overwriting each other's cookie.
fn cookie_path() -> String {
    format!("{}/", ghost_core::url::subdir())
}

fn set_cookie_header(state: &AppState, transient_id: &str) -> String {
    let secure = if state.site_url().starts_with("https://") { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={}; Path={}; HttpOnly; SameSite=Lax; Max-Age={SESSION_MAX_AGE}{secure}",
        cookie_value(state, transient_id),
        cookie_path()
    )
}

fn clear_cookie_header(state: &AppState) -> String {
    let secure = if state.site_url().starts_with("https://") { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; Path={}; HttpOnly; SameSite=Lax; Max-Age=0{secure}", cookie_path())
}

/// Pull the signed transient id out of the request's cookies, verifying the
/// signature. A tampered or unsigned cookie reads as "no session".
fn session_transient_id(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let raw = cookies
        .split(';')
        .filter_map(|c| c.trim().strip_prefix(&format!("{SESSION_COOKIE}=")))
        .next()?;
    let (transient_id, sig) = raw.rsplit_once('.')?;
    let expected = hmac_sha256_hex(&theme_secret(state), transient_id);
    secure_eq(&expected, sig).then(|| transient_id.to_string())
}

// ---------------------------------------------------------------------------
// member loading
// ---------------------------------------------------------------------------

/// The columns every member-shaped response is built from.
const MEMBER_COLUMNS: &str =
    "id, uuid, transient_id, email, name, expertise, status, created_at, enable_comment_notifications, commenting";

type MemberRow = (
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    bool,
    Option<String>,
);

fn row_to_member(r: MemberRow) -> Member {
    Member {
        id: r.0,
        uuid: r.1.unwrap_or_default(),
        transient_id: r.2,
        email: r.3,
        name: r.4,
        expertise: r.5,
        status: r.6,
        created_at: r.7,
        enable_comment_notifications: r.8,
        commenting: crate::comments::Commenting::parse(r.9.as_deref()),
    }
}

pub struct Member {
    pub id: String,
    pub uuid: String,
    pub transient_id: String,
    pub email: String,
    pub name: Option<String>,
    pub expertise: Option<String>,
    pub status: String,
    pub created_at: String,
    pub enable_comment_notifications: bool,
    /// Whether a moderator has suspended this member's ability to comment.
    pub commenting: crate::comments::Commenting,
}

async fn member_by_transient_id(state: &AppState, transient_id: &str) -> Option<Member> {
    sqlx::query_as::<_, MemberRow>(&format!(
        "SELECT {MEMBER_COLUMNS} FROM members WHERE transient_id = ? LIMIT 1"
    ))
    .bind(transient_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(row_to_member)
}

async fn member_by_email(state: &AppState, email: &str) -> Option<Member> {
    sqlx::query_as::<_, MemberRow>(&format!(
        "SELECT {MEMBER_COLUMNS} FROM members WHERE lower(email) = lower(?) LIMIT 1"
    ))
    .bind(email)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(row_to_member)
}

async fn member_by_uuid(state: &AppState, uuid: &str) -> Option<Member> {
    sqlx::query_as::<_, MemberRow>(&format!(
        "SELECT {MEMBER_COLUMNS} FROM members WHERE uuid = ? LIMIT 1"
    ))
    .bind(uuid)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(row_to_member)
}

/// Load the signed-in member, if any. Also used by the frontend renderer to
/// populate `@member`, which is what lets a theme show "Account" instead of
/// "Sign in" without a round trip.
pub async fn current_member(state: &AppState, headers: &HeaderMap) -> Option<Member> {
    let transient_id = session_transient_id(state, headers)?;
    member_by_transient_id(state, &transient_id).await
}

/// Ghost fills this with a Gravatar URL, which is keyed by an md5 of the
/// lowercased address. md5 is not a dependency here (and is not worth adding
/// for an avatar), so report no avatar and let Portal fall back to its
/// initial-letter bubble.
fn avatar_image() -> Value {
    Value::Null
}

/// Whether this member may post comments.
///
/// Two independent gates, both of which Ghost applies here: the site-wide
/// Settings → Comments level (`all` / `paid` / `off`), and whether a
/// moderator has suspended this particular member. The comments widget reads
/// `can_comment` off the session to decide whether to show the form at all,
/// and the write endpoints re-check it — the flag is a UI hint, not the
/// enforcement.
fn can_comment(state: &AppState, m: &Member) -> bool {
    let tier_ok = match state.settings.get_str("comments_enabled").unwrap_or_else(|| "off".into()).as_str() {
        "all" => true,
        "paid" => m.status != "free",
        _ => false,
    };
    tier_ok && m.commenting.can_comment()
}

/// The newsletters a member is subscribed to, in Portal's shape.
async fn member_newsletters(state: &AppState, member_id: &str) -> Vec<Value> {
    sqlx::query_as::<_, (String, Option<String>, String, Option<String>, i64)>(
        "SELECT n.id, n.uuid, n.name, n.description, n.sort_order \
         FROM newsletters n JOIN members_newsletters mn ON mn.newsletter_id = n.id \
         WHERE mn.member_id = ? AND n.status = 'active' ORDER BY n.sort_order ASC",
    )
    .bind(member_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, uuid, name, description, sort_order)| {
        json!({ "id": id, "uuid": uuid, "name": name, "description": description, "sort_order": sort_order })
    })
    .collect()
}

/// Ghost's `formattedMemberResponse`, which Portal's account screen renders
/// directly.
async fn member_json(state: &AppState, m: &Member) -> Value {
    let newsletters = member_newsletters(state, &m.id).await;
    let firstname = m
        .name
        .as_deref()
        .and_then(|n| n.split_whitespace().next())
        .map(|s| s.to_string());
    let site_url = state.site_url();
    json!({
        "uuid": m.uuid,
        "email": m.email,
        "name": m.name,
        "firstname": firstname,
        "expertise": m.expertise,
        "avatar_image": avatar_image(),
        "unsubscribe_url": format!("{site_url}/unsubscribe/?uuid={}&key={}", m.uuid, newsletter_key(state, &m.uuid)),
        "subscribed": !newsletters.is_empty(),
        "subscriptions": subscriptions::list(state, &m.id).await,
        "status": m.status,
        "paid": m.status != "free",
        "created_at": m.created_at,
        "enable_comment_notifications": m.enable_comment_notifications,
        "can_comment": can_comment(state, m),
        "commenting": m.commenting.to_json(),
        "newsletters": newsletters,
        "email_suppression": crate::newsletter_events::suppression(state, &m.email).await,
    })
}

// ---------------------------------------------------------------------------
// GET /members/api/site
// ---------------------------------------------------------------------------

/// Ghost exposes this deliberately-small, unauthenticated slice of site config
/// so that other sites can detect "is this Ghost, and are members on?" without
/// a Content API key or CORS setup. Portal reads it on boot.
async fn read_site(State(state): State<AppState>) -> Response {
    let s = |k: &str| state.settings.get(k).unwrap_or(Value::Null);
    let image = |v: Value| match v.as_str() {
        Some(p) => json!(ghost_core::url::base_asset(p)),
        None => v,
    };
    let signup_access = state
        .settings
        .get_str("members_signup_access")
        .unwrap_or_else(|| "all".into());
    // Ghost withholds external signup when the publisher requires a terms
    // checkbox: a signup form embedded on another site cannot show those terms,
    // so accepting the signup there would collect consent that was never given.
    let terms_gated = state.settings.get_bool("portal_signup_checkbox_required")
        && state
            .settings
            .get_str("portal_signup_terms_html")
            .is_some_and(|t| !t.trim().is_empty());
    private_json(
        StatusCode::OK,
        json!({ "site": {
            "title": s("title"),
            "description": s("description"),
            // Portal renders these straight into <img>, so they go out
            // browser-ready like every other image field
            "logo": image(s("logo")),
            "icon": image(s("icon")),
            "cover_image": image(s("cover_image")),
            "accent_color": s("accent_color"),
            "locale": state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
            // Portal's gift screen reads this to work out the earliest
            // delivery date it may offer.
            "timezone": state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into()),
            "url": format!("{}/", state.site_url()),
            // Ghost publishes `version.safe` here — major.minor, not the
            // patch-level string the export file carries.
            "version": ghost_core::GHOST_VERSION_SAFE,
            "allow_external_signup": signup_access == "all" && !terms_gated,
            "site_uuid": s("site_uuid"),
        }}),
    )
}

// ---------------------------------------------------------------------------
// GET /members/api/integrity-token
// ---------------------------------------------------------------------------

/// `{expiry}:{nonce}:{hmac}` — Ghost's `RequestIntegrityTokenProvider`.
/// Portal fetches one immediately before posting a magic-link request; it
/// makes scripted signup noticeably more effort than a bare POST, since the
/// token is short-lived and must be server-issued.
/// The custom fields a signed-in member may see, with their current values.
///
/// Gated on each field's `member_access`: a `none` field is staff-only and is
/// not merely read-only here but absent, so Portal never shows a member a
/// field the publisher keeps to themselves.
async fn get_metafields(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
    };
    let defs = ghost_db::repos::metafields::readable_definitions(&state.db, true)
        .await
        .unwrap_or_default();
    let values =
        ghost_db::repos::metafields::values_for_member(&state.db, &member.id).await.unwrap_or_default();

    let fields: Vec<Value> = defs
        .iter()
        .map(|d| {
            let held = values.get(&d.key);
            // A record reports its parts; a scalar reports one value.
            let value = match d.parts() {
                Some(parts) => {
                    let mut m = serde_json::Map::new();
                    for (path, _, _) in parts {
                        if let Some(v) = held.and_then(|h| h.get(*path)) {
                            m.insert((*path).to_string(), json!(v));
                        }
                    }
                    Value::Object(m)
                }
                None => json!(held.and_then(|h| h.get("")).cloned().unwrap_or_default()),
            };
            json!({
                "key": d.key,
                "name": d.name,
                "type": d.field_type,
                "identity": d.identity(),
                "editable": ghost_core::metafields::member_can_write(&d.member_access),
                "value": value,
            })
        })
        .collect();
    private_json(StatusCode::OK, json!({ "metafields": fields }))
}

/// A member setting their own values from their account page.
///
/// Only fields at `write` are accepted. A `read` field posted here is refused
/// rather than ignored — a client sending one is wrong about what it may do,
/// and silently dropping it would look like a successful save.
async fn put_metafields(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
    };
    let Some(incoming) = body.get("metafields").and_then(|m| m.as_object()) else {
        return api_error(StatusCode::BAD_REQUEST, "No fields given.", "BadRequestError");
    };

    let defs = ghost_db::repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let mut writes: HashMap<String, HashMap<String, String>> = HashMap::new();
    for (key, value) in incoming {
        let Some(def) = defs.iter().find(|d| d.key == *key) else {
            return api_error(
                StatusCode::BAD_REQUEST,
                &format!("No such field: {key}"),
                "BadRequestError",
            );
        };
        if !def.is_active() || !ghost_core::metafields::member_can_write(&def.member_access) {
            return api_error(
                StatusCode::FORBIDDEN,
                &format!("You cannot change {}.", def.name),
                "NoPermissionError",
            );
        }
        let leaves = writes.entry(key.clone()).or_default();
        match (def.parts(), value) {
            // A record is given as an object of its parts.
            (Some(_), Value::Object(parts)) => {
                for (path, v) in parts {
                    leaves.insert(path.clone(), v.as_str().unwrap_or_default().to_string());
                }
            }
            (None, v) => {
                leaves.insert(String::new(), v.as_str().unwrap_or_default().to_string());
            }
            _ => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    &format!("{} expects an object of its parts.", def.name),
                    "BadRequestError",
                )
            }
        }
    }

    let writer = ghost_db::repos::metafields::Writer::member(&member.id);
    match ghost_db::repos::metafields::write_values(&state.db, &member.id, &writes, &writer).await {
        Ok(_) => get_metafields(State(state), headers).await,
        Err(e) => api_error(StatusCode::BAD_REQUEST, &e.message(), "BadRequestError"),
    }
}

/// The public view of an offer, which Portal reads to show the discounted
/// price. Ghost's `offers-public` shape: only what a buyer needs.
///
/// Shared with the Content API's `offers/{id}`, which is the route Portal
/// actually calls — the two must not drift, or the offer screen would render
/// differently depending on which one answered.
pub(crate) fn offer_public_json(offer: &ghost_db::repos::offers::Offer) -> Value {
    json!({
        "id": offer.id,
        "display_title": offer.portal_title.clone().unwrap_or_default(),
        "display_description": offer.portal_description.clone().unwrap_or_default(),
        "type": offer.discount_type.clone().unwrap_or_default(),
        "cadence": offer.interval.clone().unwrap_or_default(),
        "amount": offer.discount_amount.unwrap_or(0),
        "duration": offer.duration.clone().unwrap_or_default(),
        "duration_in_months": offer.duration_in_months,
        "currency": offer.currency,
        "status": offer.status(),
        "redemption_type": offer.redemption_type.clone().unwrap_or_else(|| "signup".into()),
        "tier": { "id": offer.product_id },
    })
}

/// Only an offer still being redeemed resolves — an archived code must not.
async fn read_offer(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let offer = ghost_db::repos::offers::find_by_id(&state.db, &id).await.ok().flatten();
    let Some(offer) = offer.filter(|o| o.active) else {
        return api_error(StatusCode::NOT_FOUND, "Offer not found.", "NotFoundError");
    };
    private_json(StatusCode::OK, json!({ "offers": [offer_public_json(&offer)] }))
}

/// The offers Portal may put in front of *this* member.
///
/// Only retention offers, and only for someone paying: Portal asks for these
/// on boot when `member.paid`, then uses them to make a cancelling subscriber
/// a discounted counter-offer on `#/portal/account/plans`. A signup offer has
/// no place here — it is for people who are not members yet.
async fn get_member_offers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return private_json(StatusCode::OK, json!({ "offers": [] }));
    };
    if member.status == "free" {
        return private_json(StatusCode::OK, json!({ "offers": [] }));
    }
    let offers = ghost_db::repos::offers::list(&state.db).await.unwrap_or_default();
    let items: Vec<Value> = offers
        .iter()
        .filter(|o| o.active && o.redemption_type.as_deref() == Some("retention"))
        .map(offer_public_json)
        .collect();
    private_json(StatusCode::OK, json!({ "offers": items }))
}

/// `POST /members/api/feedback` — one score for one post.
///
/// A reader who clicks both buttons in the same email, or changes their mind
/// later, should end up with the score they chose last rather than two rows,
/// so this upserts on the member/post pair.
async fn add_feedback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    // Session first, then the signed link — Ghost runs the same two in that
    // order, so a member reading their own newsletter while signed in is
    // identified by the session even if the link's tokens are stale.
    let member = match current_member(&state, &headers).await {
        Some(m) => Some(m),
        None => auth_member_by_uuid(&state, &params).await,
    };
    let Some(member) = member else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "You must be signed in to leave feedback.",
            "UnauthorizedError",
        );
    };

    let entry = body
        .get("feedback")
        .and_then(|f| f.as_array())
        .and_then(|a| a.first())
        .or(body.get("feedback"));
    let post_id = entry.and_then(|e| e.get("post_id")).and_then(|v| v.as_str()).unwrap_or_default();
    let score = entry.and_then(|e| e.get("score")).and_then(|v| v.as_i64()).unwrap_or(-1);
    if post_id.is_empty() || !(0..=1).contains(&score) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "Feedback needs a post and a score of 0 or 1.",
            "BadRequestError",
        );
    }

    // The post has to exist; a feedback row pointing at nothing would break
    // the foreign key and tell us nothing anyway.
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if exists.is_none() {
        return api_error(StatusCode::NOT_FOUND, "Post not found.", "NotFoundError");
    }

    let now = now_rfc3339();
    let id = ids::new_id();
    let saved = sqlx::query(
        "INSERT INTO members_feedback (id, score, member_id, post_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (member_id, post_id) DO UPDATE SET score = excluded.score, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(score)
    .bind(&member.id)
    .bind(post_id)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    if saved.is_err() {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save your feedback.",
            "InternalServerError",
        );
    }
    // Read the id back: when the upsert hit an existing row, the one minted
    // above was never used, and reporting it would name a record that is not
    // there.
    let stored: Option<(String, String)> =
        sqlx::query_as("SELECT id, created_at FROM members_feedback WHERE member_id = ? AND post_id = ?")
            .bind(&member.id)
            .bind(post_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let (id, created_at) = stored.unwrap_or((id, now));
    private_json(
        StatusCode::CREATED,
        json!({ "feedback": [{
            "id": id,
            "score": score,
            "post_id": post_id,
            "member_id": member.id,
            "created_at": created_at,
        }] }),
    )
}

/// A visitor opened a recommended site from `#/portal/recommendations`.
async fn recommendation_clicked(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    record_recommendation_event(&state, &headers, &id, "recommendation_click_events").await
}

/// A visitor took the one-click subscribe on a recommended site.
async fn recommendation_subscribed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    record_recommendation_event(&state, &headers, &id, "recommendation_subscribe_events").await
}

/// Both beacons record the same three columns into their own table.
///
/// `member_id` stays null for a visitor who is not signed in — the event is
/// still worth having, and Ghost's column is nullable for exactly that. An
/// unknown recommendation id is dropped rather than refused: a beacon cannot
/// read the answer, so an error would only be noise.
async fn record_recommendation_event(
    state: &AppState,
    headers: &HeaderMap,
    recommendation_id: &str,
    table: &str,
) -> Response {
    let known: Option<String> = sqlx::query_scalar("SELECT id FROM recommendations WHERE id = ?")
        .bind(recommendation_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if known.is_some() {
        let member_id = current_member(state, headers).await.map(|m| m.id);
        let _ = sqlx::query(&format!(
            "INSERT INTO {table} (id, recommendation_id, member_id, created_at) VALUES (?, ?, ?, ?)"
        ))
        .bind(ids::new_id())
        .bind(recommendation_id)
        .bind(member_id)
        .bind(now_rfc3339())
        .execute(&state.db)
        .await;
    }
    no_content()
}

async fn integrity_token(State(state): State<AppState>) -> Response {
    let expiry = chrono::Utc::now().timestamp_millis() + INTEGRITY_TOKEN_MS;
    let nonce = ids::random_hex(16);
    let payload = format!("{expiry}:{nonce}");
    let sig = hmac_sha256_hex(&theme_secret(&state), &payload);
    private_text(StatusCode::OK, format!("{payload}:{sig}"))
}

fn integrity_token_valid(state: &AppState, token: &str) -> bool {
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() != 3 {
        return false;
    }
    let Ok(expiry) = parts[0].parse::<i64>() else {
        return false;
    };
    let expected = hmac_sha256_hex(&theme_secret(state), &format!("{}:{}", parts[0], parts[1]));
    secure_eq(&expected, parts[2]) && chrono::Utc::now().timestamp_millis() <= expiry
}

// ---------------------------------------------------------------------------
// POST /members/api/send-magic-link
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct MagicLinkRequest {
    email: String,
    #[serde(rename = "emailType")]
    email_type: Option<String>,
    name: Option<String>,
    newsletters: Option<Vec<Value>>,
    redirect: Option<String>,
    #[serde(rename = "integrityToken")]
    integrity_token: Option<String>,
    /// Portal's honeypot field; a real browser leaves it empty.
    honeypot: Option<String>,
}

/// Issue a magic link for signup or signin.
///
/// Ghost always answers 201 here regardless of whether the address is already
/// a member, because a different answer for known and unknown emails turns
/// this endpoint into a membership oracle. We keep that behaviour, and so the
/// member row is only created when the link is *used*, not when it is sent.
async fn send_magic_link(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<MagicLinkRequest>,
) -> Response {
    if req.honeypot.as_deref().is_some_and(|h| !h.is_empty()) {
        // Silently succeed: a bot gets no signal either way.
        return private_json(StatusCode::CREATED, json!({}));
    }
    match &req.integrity_token {
        Some(t) if integrity_token_valid(&state, t) => {}
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "Your request has expired, please reload the page and try again.",
                "BadRequestError",
            )
        }
    }

    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return api_error(StatusCode::BAD_REQUEST, "Please enter a valid email address.", "BadRequestError");
    }

    let existing = member_by_email(&state, &email).await;
    let signup_access = state
        .settings
        .get_str("members_signup_access")
        .unwrap_or_else(|| "all".into());
    let requested_type = req.email_type.as_deref().unwrap_or("signin");

    // An unknown email asking to sign in is really a signup, and vice versa —
    // Ghost resolves the action from the member's existence, not the client's
    // claim, so a stale Portal tab cannot create duplicates.
    let action = if existing.is_some() { "signin" } else { "signup" };

    if action == "signup" && signup_access != "all" {
        // Invite-only or members-off: don't create, don't leak. Ghost returns
        // an error here only for the explicit signup case.
        return api_error(
            StatusCode::BAD_REQUEST,
            "This site is invite-only, contact the owner for access.",
            "BadRequestError",
        );
    }

    // Settings → Membership → Spam filters. Only signups are checked: a member
    // who already has an account keeps it (and their ability to sign in) if
    // their domain is blocked later, which is how Ghost treats the list.
    if action == "signup" && state.settings.email_domain_is_blocked(&email) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "Signups from that email domain are not allowed.",
            "BadRequestError",
        );
    }

    let token = ids::random_hex(32);
    let data = json!({
        "email": email,
        "action": action,
        "name": req.name,
        "newsletters": req.newsletters,
        "redirect": req.redirect,
        "requested_type": requested_type,
    });
    let now = now_rfc3339();
    if let Err(e) = sqlx::query(
        "INSERT INTO tokens (id, token, uuid, data, created_at, updated_at, used_count, otc_used_count) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0)",
    )
    .bind(ids::new_id())
    .bind(&token)
    .bind(ids::uuid_v4())
    .bind(data.to_string())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    {
        tracing::error!("members: could not store magic link token: {e}");
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to send magic link email",
            "InternalServerError",
        );
    }

    let mut link = format!("{}/members/?token={token}&action={action}", state.site_url());
    if let Some(r) = req.redirect.as_deref().filter(|r| !r.is_empty()) {
        link.push_str(&format!(
            "&r={}",
            percent_encoding::utf8_percent_encode(r, percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    // Deliver off the request path. An SMTP handshake can take seconds, and
    // Portal holds its spinner until this responds; worse, letting delivery
    // time leak into the response would undo the uniform answer above, since
    // a send to a real mailbox and a send that bounces take different times.
    // Failures are logged — Settings → Email has a test button that reports
    // transport errors synchronously, which is the right place to debug them.
    let bg = state.clone();
    let (email_bg, action_bg, link_bg) = (email.clone(), action.to_string(), link.clone());
    tokio::spawn(async move {
        if let Err(e) = send_magic_link_email(&bg, &email_bg, &action_bg, &link_bg).await {
            tracing::error!(
                "members: could not send {action_bg} email to {email_bg}: {}",
                e.message()
            );
        }
    });

    private_json(StatusCode::CREATED, json!({}))
}

/// Render and deliver the magic-link email for `action`.
///
/// When delivery fails in development the link is written to the log, so a
/// blocked outbound port 25 or an unconfigured relay does not make signin
/// impossible to exercise locally. That never happens outside development: a
/// magic link grants access to the account it names, and logs get shipped
/// and retained far more widely than anyone intends.
async fn send_magic_link_email(
    state: &AppState,
    email: &str,
    action: &str,
    link: &str,
) -> ghost_core::Result<()> {
    let site_url = state.site_url();
    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let accent = state.settings.get_str("accent_color").unwrap_or_default();
    let domain = ghost_mail::site_domain(&site_url);

    let kind = ghost_mail::templates::LinkType::from_action(action);
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &site_title, &site_url);
    let msg = ghost_mail::Message {
        to: email.to_string(),
        subject: kind.subject(&site_title),
        html: ghost_mail::templates::magic_link_html(kind, &site_title, &domain, email, link, &accent),
        text: ghost_mail::templates::magic_link_text(kind, &site_title, &domain, email, link),
    };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => Ok(()),
        Err(e) if state.config.env == "development" => {
            tracing::warn!(
                "members: {action} email to {email} could not be delivered ({}) — use this link to continue: {link}",
                e.message()
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// GET /members/?token=...  — the magic-link landing
// ---------------------------------------------------------------------------

/// Exchange a magic-link token for a session cookie, then bounce to the site.
///
/// Ghost appends `success=true` and the original `action` to the redirect, and
/// Portal reads those on load to decide whether to pop the "you're in" notice
/// — which is why the query string matters as much as the cookie.
async fn magic_link_landing(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let site_url = state.site_url();
    let Some(token) = params.get("token") else {
        return Redirect::to(&format!("{site_url}/")).into_response();
    };

    let row = sqlx::query_as::<_, (String, String, i64, String)>(
        "SELECT id, data, used_count, created_at FROM tokens WHERE token = ? LIMIT 1",
    )
    .bind(token)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let fail = |code: &str| {
        Redirect::to(&format!("{site_url}/?success=false&errorCode={code}")).into_response()
    };

    let Some((token_id, data, used_count, created_at)) = row else {
        return fail("InvalidLink");
    };
    // Single use, like Ghost's single-use-token-provider: a link that already
    // created a session must not create another if it leaks from an inbox.
    if used_count > 0 {
        return fail("TokenExpired");
    }
    let expired = chrono::NaiveDateTime::parse_from_str(&created_at, "%Y-%m-%d %H:%M:%S")
        .map(|t| chrono::Utc::now().naive_utc().signed_duration_since(t).num_seconds() > MAGIC_TOKEN_VALIDITY_SECS)
        .unwrap_or(false);
    if expired {
        return fail("TokenExpired");
    }

    let data: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
    let email = data.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string();
    if email.is_empty() {
        return fail("InvalidLink");
    }
    let action = data.get("action").and_then(|a| a.as_str()).unwrap_or("signin").to_string();
    let name = data.get("name").and_then(|n| n.as_str()).map(|s| s.to_string());
    let requested_newsletters = data.get("newsletters").cloned().unwrap_or(Value::Null);

    // Burn the token before issuing the session, so a double-click cannot
    // race two signups through.
    let consumed = sqlx::query("UPDATE tokens SET used_count = used_count + 1, first_used_at = ?, updated_at = ? WHERE id = ? AND used_count = 0")
        .bind(now_rfc3339())
        .bind(now_rfc3339())
        .bind(&token_id)
        .execute(&state.db)
        .await;
    if !consumed.is_ok_and(|result|result.rows_affected()==1) { return fail("TokenExpired"); }

    let member = match member_by_email(&state, &email).await {
        Some(m) => m,
        None => match create_member(&state, &email, name.as_deref(), &requested_newsletters).await {
            Some(m) => m,
            None => return fail("InvalidLink"),
        },
    };

    let _ = sqlx::query("UPDATE members SET last_seen_at = ?, updated_at = ? WHERE id = ?")
        .bind(now_rfc3339())
        .bind(now_rfc3339())
        .bind(&member.id)
        .execute(&state.db)
        .await;

    // `r` is attacker-influencable (it rides in the emailed URL), so only
    // honour it when it points back at this site.
    let referrer = params
        .get("r")
        .filter(|r| r.starts_with(&site_url))
        .cloned()
        .unwrap_or_else(|| format!("{site_url}/"));
    let sep = if referrer.contains('?') { '&' } else { '?' };
    let target = format!("{referrer}{sep}action={action}&success=true");

    (
        StatusCode::FOUND,
        [
            (header::SET_COOKIE, set_cookie_header(&state, &member.transient_id)),
            (header::LOCATION, target),
        ],
    )
        .into_response()
}

/// Create a free member, subscribing them to the newsletters Portal asked for
/// — or to every `subscribe_on_signup` newsletter when it asked for none,
/// which is Ghost's default for a bare signup.
async fn create_member(
    state: &AppState,
    email: &str,
    name: Option<&str>,
    requested_newsletters: &Value,
) -> Option<Member> {
    let id = ids::new_id();
    let now = now_rfc3339();
    let transient_id = ids::uuid_v4();
    sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, name, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'free', ?, ?)",
    )
    .bind(&id)
    .bind(ids::uuid_v4())
    .bind(&transient_id)
    .bind(email)
    .bind(name)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await
    .ok()?;

    let _ = sqlx::query("INSERT INTO members_created_events (id, member_id, source, created_at) VALUES (?, ?, 'member', ?)")
        .bind(ids::new_id())
        .bind(&id)
        .bind(&now)
        .execute(&state.db)
        .await;

    let ids_requested: Vec<String> = requested_newsletters
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|n| n.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let newsletter_ids: Vec<String> = if ids_requested.is_empty() {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM newsletters WHERE status = 'active' AND subscribe_on_signup = 1 ORDER BY sort_order ASC",
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    } else {
        ids_requested
    };
    for nid in newsletter_ids {
        let _ = sqlx::query(
            "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, 'confirmed')",
        )
        .bind(ids::new_id())
        .bind(&id)
        .bind(&nid)
        .execute(&state.db)
        .await;
    }

    // Automations are triggered by membership starting, so this is the one
    // place a signup can enrol someone. It runs after the newsletter
    // subscriptions are written, because whether the member wants updates at
    // all is read from them.
    crate::automations_engine::trigger_for_member(state, &id).await;

    // Tell the integrations. This is the signup every "new member" trigger
    // is really waiting on — the admin's own form fired one and this path,
    // the one real readers come through, did not.
    crate::webhooks::fire_member(
        state,
        "member.added",
        &id,
        email,
        name.unwrap_or_default(),
        json!({}),
    );

    member_by_email(state, email).await
}

// ---------------------------------------------------------------------------
// session
// ---------------------------------------------------------------------------

/// Portal calls this to learn whether anyone is signed in, and forwards the
/// body to Stripe endpoints as proof of identity. Ghost returns a JWT signed
/// with the members keypair; this port signs with the theme secret, since
/// nothing but this server verifies it.
async fn get_identity(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return no_content();
    };
    let header = json!({ "alg": "HS256", "typ": "JWT" });
    let claims = json!({
        "sub": member.email,
        "aud": state.site_url(),
        "iat": chrono::Utc::now().timestamp(),
        "exp": chrono::Utc::now().timestamp() + 600,
    });
    let b64 = |v: &Value| base64_url_nopad::encode(v.to_string().as_bytes());
    let payload = format!("{}.{}", b64(&header), b64(&claims));
    let sig = hmac_sha256_hex(&theme_secret(&state), &payload);
    private_text(StatusCode::OK, format!("{payload}.{sig}"))
}

/// Sign out. Rotating `transient_id` is what makes "sign out of all devices"
/// work: every other signed cookie still names the old id, which now matches
/// no row.
///
/// The body is read as a raw string because Portal sends signout both with and
/// without a JSON body, and a typed extractor would reject the empty case.
async fn delete_session(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let all = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("all").and_then(|a| a.as_bool()))
        .unwrap_or(false);
    if all {
        if let Some(member) = current_member(&state, &headers).await {
            let _ = sqlx::query("UPDATE members SET transient_id = ?, updated_at = ? WHERE id = ?")
                .bind(ids::uuid_v4())
                .bind(now_rfc3339())
                .bind(&member.id)
                .execute(&state.db)
                .await;
        }
    }
    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, clear_cookie_header(&state)),
            (header::CACHE_CONTROL, "private, no-cache, no-store".to_string()),
        ],
        "",
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// member data
// ---------------------------------------------------------------------------

async fn get_member(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match current_member(&state, &headers).await {
        Some(m) => {
            let body = member_json(&state, &m).await;
            private_json(StatusCode::OK, body)
        }
        None => no_content(),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct UpdateMemberRequest {
    name: Option<String>,
    newsletters: Option<Vec<Value>>,
    #[serde(rename = "enable_comment_notifications")]
    enable_comment_notifications: Option<bool>,
    subscribed: Option<bool>,
}

/// Portal's account screen posts the whole form back here.
async fn update_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<UpdateMemberRequest>,
) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
    };
    if let Some(name) = &req.name {
        let _ = sqlx::query("UPDATE members SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(now_rfc3339())
            .bind(&member.id)
            .execute(&state.db)
            .await;
    }
    if let Some(flag) = req.enable_comment_notifications {
        let _ = sqlx::query("UPDATE members SET enable_comment_notifications = ?, updated_at = ? WHERE id = ?")
            .bind(flag)
            .bind(now_rfc3339())
            .bind(&member.id)
            .execute(&state.db)
            .await;
    }
    if let Some(list) = &req.newsletters {
        set_member_newsletters(&state, &member.id, list).await;
    } else if req.subscribed == Some(false) {
        set_member_newsletters(&state, &member.id, &[]).await;
    }

    // Re-read so the response reflects what was actually stored.
    match member_by_transient_id(&state, &member.transient_id).await {
        Some(m) => {
            let body = member_json(&state, &m).await;
            private_json(StatusCode::OK, body)
        }
        None => no_content(),
    }
}

/// Replace a member's subscriptions with exactly `list`.
async fn set_member_newsletters(state: &AppState, member_id: &str, list: &[Value]) {
    let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
        .bind(member_id)
        .execute(&state.db)
        .await;
    for n in list {
        let Some(nid) = n.get("id").and_then(|i| i.as_str()) else {
            continue;
        };
        let _ = sqlx::query(
            "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, 'confirmed')",
        )
        .bind(ids::new_id())
        .bind(member_id)
        .bind(nid)
        .execute(&state.db)
        .await;
    }
}

/// The unsubscribe-link flow: authenticated by `uuid` plus an HMAC `key`
/// rather than a session, because it is followed from an email client where no
/// cookie exists. Ghost derives the key from `members_email_auth_secret`.
pub(crate) fn newsletter_key(state: &AppState, uuid: &str) -> String {
    let secret = state
        .settings
        .get_str("members_email_auth_secret")
        .unwrap_or_else(|| theme_secret(state));
    hmac_sha256_hex(&secret, uuid)
}

async fn auth_member_by_uuid(state: &AppState, params: &HashMap<String, String>) -> Option<Member> {
    let uuid = params.get("uuid")?;
    let key = params.get("key")?;
    if !secure_eq(&newsletter_key(state, uuid), key) {
        return None;
    }
    member_by_uuid(state, uuid).await
}

async fn get_member_newsletters(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(member) = auth_member_by_uuid(&state, &params).await else {
        return api_error(StatusCode::UNAUTHORIZED, "Invalid unsubscribe link.", "UnauthorizedError");
    };
    let newsletters = member_newsletters(&state, &member.id).await;
    private_json(
        StatusCode::OK,
        json!({
            "email": member.email,
            "name": member.name,
            "newsletters": newsletters,
            "enable_comment_notifications": member.enable_comment_notifications,
            "status": member.status,
        }),
    )
}

async fn put_member_newsletters(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    axum::Json(req): axum::Json<UpdateMemberRequest>,
) -> Response {
    let Some(member) = auth_member_by_uuid(&state, &params).await else {
        return api_error(StatusCode::UNAUTHORIZED, "Invalid unsubscribe link.", "UnauthorizedError");
    };
    if let Some(list) = &req.newsletters {
        set_member_newsletters(&state, &member.id, list).await;
    }
    if let Some(flag) = req.enable_comment_notifications {
        let _ = sqlx::query("UPDATE members SET enable_comment_notifications = ?, updated_at = ? WHERE id = ?")
            .bind(flag)
            .bind(now_rfc3339())
            .bind(&member.id)
            .execute(&state.db)
            .await;
    }
    let newsletters = member_newsletters(&state, &member.id).await;
    private_json(
        StatusCode::OK,
        json!({
            "email": member.email,
            "name": member.name,
            "newsletters": newsletters,
            "enable_comment_notifications": member.enable_comment_notifications,
            "status": member.status,
        }),
    )
}

/// Ghost removes the address from its bulk-mail suppression list here. There
/// is no suppression list in this port, so report success — the member's
/// subscriptions are already the only thing gating delivery.
async fn delete_suppression(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(member) = current_member(&state, &headers).await else {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
    };
    if crate::newsletter_events::remove_suppression(&state, &member.email).await.is_err() {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not remove suppression.", "InternalServerError");
    }
    private_json(StatusCode::OK, json!({}))
}

/// Minimal unpadded base64url, for the identity JWT.
mod base64_url_nopad {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
        for chunk in input.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
            let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
            for (i, v) in idx.iter().enumerate() {
                if i <= chunk.len() {
                    out.push(ALPHABET[*v as usize] as char);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_known_vector() {
        // RFC 4231 test case 2, so the hand-rolled HMAC is pinned to the spec
        // rather than to itself.
        assert_eq!(
            hmac_sha256_hex("Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signed_cookies_reject_tampering() {
        let sig = hmac_sha256_hex("secret", "transient-abc");
        assert!(secure_eq(&sig, &hmac_sha256_hex("secret", "transient-abc")));
        // a different member id must not validate under the same signature
        assert!(!secure_eq(&sig, &hmac_sha256_hex("secret", "transient-abd")));
        // nor the same id under a different site secret
        assert!(!secure_eq(&sig, &hmac_sha256_hex("other", "transient-abc")));
    }

    #[test]
    fn secure_eq_is_length_safe() {
        assert!(!secure_eq("abc", "abcd"));
        assert!(secure_eq("", ""));
    }

    #[test]
    fn base64url_is_unpadded_and_url_safe() {
        assert_eq!(base64_url_nopad::encode(b"ab"), "YWI");
        assert_eq!(base64_url_nopad::encode(b"abc"), "YWJj");
        assert_eq!(base64_url_nopad::encode(b"a"), "YQ");
        assert!(!base64_url_nopad::encode(&[251, 255, 190]).contains('+'));
    }
    // --- gifts -------------------------------------------------------------

    #[test]
    fn gift_durations_resolve_from_ghosts_catalogue() {
        // explicit durations
        assert_eq!(resolve_gift_duration(Some(1), None).unwrap().1, "month");
        assert_eq!(resolve_gift_duration(Some(3), None).unwrap().1, "month");
        assert_eq!(resolve_gift_duration(Some(6), None).unwrap().1, "month");
        assert_eq!(resolve_gift_duration(Some(12), None).unwrap().1, "year");
        // a year is billed once, a multi-month gift once per month
        assert_eq!(resolve_gift_duration(Some(12), None).unwrap().2, 1);
        assert_eq!(resolve_gift_duration(Some(3), None).unwrap().2, 3);
        // cadence must agree with the duration
        assert!(resolve_gift_duration(Some(12), Some("month")).is_err());
        assert!(resolve_gift_duration(Some(3), Some("year")).is_err());
        assert!(resolve_gift_duration(Some(3), Some("month")).is_ok());
        // off-catalogue durations are refused
        assert!(resolve_gift_duration(Some(5), None).is_err());
        // the legacy cadence-only shape still resolves
        assert_eq!(resolve_gift_duration(None, Some("year")).unwrap().0, 12);
        assert_eq!(resolve_gift_duration(None, Some("month")).unwrap().0, 1);
    }

    #[test]
    fn gift_tokens_are_twelve_url_safe_characters() {
        let token = gift_token();
        assert_eq!(token.len(), 12);
        assert!(token.bytes().all(|b| b.is_ascii_alphanumeric()));
        // off the CSPRNG, not a counter: two in a row differ
        assert_ne!(token, gift_token());
    }

    #[test]
    fn query_params_go_before_the_hash_fragment() {
        // Portal's success URLs are hash routes; params appended after the
        // hash would never reach the success screen.
        assert_eq!(
            add_query_params("https://x.test/#/portal/", &[("a", "1".into())]),
            "https://x.test/?a=1#/portal/"
        );
        assert_eq!(
            add_query_params("https://x.test/?b=2#/portal/gift", &[("a", "1 b".into())]),
            "https://x.test/?b=2&a=1%20b#/portal/gift"
        );
        assert_eq!(
            add_query_params("https://x.test/gift", &[("a", "1".into())]),
            "https://x.test/gift?a=1"
        );
    }

    #[test]
    fn gift_duration_labels_read_like_ghosts_i18n() {
        assert_eq!(gift_duration_label(1), "1 month");
        assert_eq!(gift_duration_label(3), "3 months");
        assert_eq!(gift_duration_label(6), "6 months");
        assert_eq!(gift_duration_label(12), "1 year");
    }

    #[test]
    fn gifts_round_trip_through_the_tokens_row() {
        let gift = Gift {
            gift: true,
            token: "AbC123dEf456".into(),
            tier_id: "tier1".into(),
            tier_name: "Supporter".into(),
            cadence: "month".into(),
            duration: 3,
            total_months: 3,
            currency: "usd".into(),
            amount: 1500,
            buyer_email: "buyer@test".into(),
            delivery: "link".into(),
            delivery_at: None,
            status: "payment_pending".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&gift).unwrap();
        let back: Gift = serde_json::from_str(&json).unwrap();
        assert_eq!(back.token, gift.token);
        assert_eq!(back.total_months, 3);
    }

    #[test]
    fn unredeemable_gifts_get_ghosts_visitor_messages() {
        let pending = Gift { status: "payment_pending".into(), ..Default::default() };
        let (code, message, _) = check_redeemable(&pending, None).unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(message, "This gift does not exist.");

        let redeemed = Gift { status: "redeemed".into(), ..Default::default() };
        let (code, message, _) = check_redeemable(&redeemed, None).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(message, "This gift has already been redeemed.");

        // a paid member cannot take a gift — Ghost keeps gifts for free
        // members and new signups
        let fresh = Gift {
            status: "paid".into(),
            purchased_at: Some(now_rfc3339()),
            ..Default::default()
        };
        let (code, message, _) = check_redeemable(&fresh, Some("paid")).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(message, "You already have an active subscription.");
        // the same gift is fine for a signed-out visitor
        assert!(check_redeemable(&fresh, None).is_ok());
    }

}

fn scheduled_gift_date(date:Option<&str>,timezone:&str)->std::result::Result<Option<String>,String>{
    use chrono::TimeZone;
    let Some(date)=date.filter(|d|!d.is_empty())else{return Ok(None)};
    let zone:chrono_tz::Tz=timezone.parse().unwrap_or(chrono_tz::UTC);
    let selected=chrono::NaiveDate::parse_from_str(date,"%Y-%m-%d").map_err(|_|"Choose a valid gift delivery date".to_string())?;
    let today=chrono::Utc::now().with_timezone(&zone).date_naive();
    let days=selected.signed_duration_since(today).num_days();
    if !(0..=365).contains(&days){return Err("Choose today or a date within the next 365 days".into())}
    if days==0{return Ok(None)}
    let at=zone.from_local_datetime(&selected.and_hms_opt(9,0,0).unwrap()).earliest().ok_or_else(||"Invalid local delivery time".to_string())?;
    Ok(Some(at.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string()))
}
/// Native worker with persistent retry state. A process restart does not lose a delivery.
pub fn spawn_gift_delivery(state:AppState){tokio::spawn(async move{loop{deliver_due_gifts(&state).await;tokio::time::sleep(std::time::Duration::from_secs(30)).await;}});}
async fn deliver_due_gifts(state:&AppState){
    let due=sqlx::query_scalar::<_,String>("SELECT token FROM scheduled_gift_deliveries WHERE sent_at IS NULL AND attempts<12 AND datetime(deliver_at)<=datetime('now') AND (last_attempt_at IS NULL OR datetime(last_attempt_at)<datetime('now','-5 minutes')) LIMIT 50")
        .fetch_all(&state.db).await.unwrap_or_default();
    for token in due{
        let claimed=sqlx::query("UPDATE scheduled_gift_deliveries SET attempts=attempts+1,last_attempt_at=? WHERE token=? AND sent_at IS NULL AND (last_attempt_at IS NULL OR datetime(last_attempt_at)<datetime('now','-5 minutes'))")
            .bind(now_rfc3339()).bind(&token).execute(&state.db).await.is_ok_and(|r|r.rows_affected()==1);
        if !claimed{continue}
        let Some(gift)=find_gift(state,&token).await.filter(|g|g.status=="paid"||g.status=="redeemed")else{continue};
        let link=format!("{}/#/portal/gift/redeem/{}",state.site_url(),gift.token);
        match send_gift_email(state,&gift,&link).await{
            Ok(())=>{let _=sqlx::query("UPDATE scheduled_gift_deliveries SET sent_at=?,last_error=NULL WHERE token=?").bind(now_rfc3339()).bind(&token).execute(&state.db).await;},
            Err(error)=>{let _=sqlx::query("UPDATE scheduled_gift_deliveries SET last_error=? WHERE token=?").bind(error.message()).bind(&token).execute(&state.db).await;}
        }
    }
}
