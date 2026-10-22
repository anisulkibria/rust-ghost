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

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use ghost_core::ids;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::AppState;

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
        .route("/members/api/member", get(get_member).put(update_member))
        .route("/members/api/member/", get(get_member).put(update_member))
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
        .route("/members/api/subscriptions/{id}", put(stripe_unavailable))
        .route("/members/api/subscriptions/{id}/", put(stripe_unavailable))
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
    #[serde(rename = "successUrl")]
    success_url: Option<String>,
    #[serde(rename = "cancelUrl")]
    cancel_url: Option<String>,
    metadata: Option<Value>,
}

/// Start a paid signup. Portal posts the tier and cadence it wants and
/// expects a URL to send the visitor to.
async fn create_checkout_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<CheckoutRequest>,
) -> Response {
    let Some(cfg) = stripe_cfg(&state) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "This site is not set up to take payments yet.",
            "BadRequestError",
        );
    };

    // Resolve the Stripe price: either given outright, or derived from the
    // tier plus the cadence the visitor picked.
    let price_id = match req.price_id.clone().filter(|p| p.starts_with("price_")) {
        Some(p) => Some(p),
        None => {
            let column = match req.cadence.as_deref() {
                Some("year") | Some("yearly") => "yearly_price_id",
                _ => "monthly_price_id",
            };
            let sql = match &req.tier_id {
                Some(_) => format!("SELECT {column} FROM products WHERE id = ? AND active = 1 LIMIT 1"),
                None => format!(
                    "SELECT {column} FROM products WHERE type = 'paid' AND active = 1 ORDER BY created_at LIMIT 1"
                ),
            };
            let mut q = sqlx::query_scalar::<_, Option<String>>(&sql);
            if let Some(t) = &req.tier_id {
                q = q.bind(t);
            }
            q.fetch_optional(&state.db).await.ok().flatten().flatten()
        }
    };
    let Some(price_id) = price_id.filter(|p| !p.is_empty()) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "That plan is not available for purchase yet.",
            "BadRequestError",
        );
    };

    // An existing session identifies the buyer; otherwise Portal supplies the
    // email typed into the signup form.
    let member = current_member(&state, &headers).await;
    let email = member
        .as_ref()
        .map(|m| m.email.clone())
        .or_else(|| req.email.clone())
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

    let trial: i64 = match &req.tier_id {
        Some(t) => sqlx::query_scalar("SELECT trial_days FROM products WHERE id = ?")
            .bind(t)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .unwrap_or(0),
        None => 0,
    };
    let _ = &req.identity;
    let _ = &req.metadata;

    match ghost_stripe::create_checkout_session(
        &cfg,
        &price_id,
        Some(&email),
        &success,
        &cancel,
        &uuid,
        trial.max(0) as u32,
    )
    .await
    {
        Ok(url) => private_json(StatusCode::OK, json!({ "url": url })),
        Err(e) => {
            tracing::error!("stripe: checkout session failed: {}", e.message());
            api_error(StatusCode::BAD_REQUEST, &e.message(), "BadRequestError")
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

    match kind {
        "checkout.session.completed" => handle_checkout_completed(&state, &cfg, &object).await,
        "customer.subscription.created" | "customer.subscription.updated" | "customer.subscription.deleted" => {
            handle_subscription_change(&state, &object).await
        }
        _ => {}
    }
    // Always 200 for a signed event: a non-2xx makes Stripe retry, and a
    // payload we do not handle is not an error.
    (StatusCode::OK, "ok").into_response()
}

/// A completed checkout: find or create the member, attach the customer.
async fn handle_checkout_completed(state: &AppState, cfg: &ghost_stripe::StripeConfig, obj: &Value) {
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
        return;
    };

    if !customer_id.is_empty() {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM members_stripe_customers WHERE customer_id = ?")
                .bind(&customer_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();
        if existing.is_none() {
            let now = now_rfc3339();
            let _ = sqlx::query(
                "INSERT INTO members_stripe_customers (id, member_id, customer_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(ids::new_id())
            .bind(&member.id)
            .bind(&customer_id)
            .bind(&now)
            .bind(&now)
            .execute(&state.db)
            .await;
        }
    }

    // The session carries only the subscription id, so fetch the real thing
    // rather than guessing its state.
    if let Some(sub_id) = obj.get("subscription").and_then(|s| s.as_str()) {
        if let Ok(sub) = ghost_stripe::get_subscription(cfg, sub_id).await {
            handle_subscription_change(state, &sub).await;
        }
    }
}

/// A subscription was created, changed or cancelled: record it and move the
/// member between free and paid accordingly.
async fn handle_subscription_change(state: &AppState, sub: &Value) {
    let sub_id = sub.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let customer_id = sub.get("customer").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let status = sub.get("status").and_then(|s| s.as_str()).unwrap_or("").to_string();
    if sub_id.is_empty() || customer_id.is_empty() {
        return;
    }
    let member_id: Option<String> =
        sqlx::query_scalar("SELECT member_id FROM members_stripe_customers WHERE customer_id = ? LIMIT 1")
            .bind(&customer_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some(member_id) = member_id else {
        tracing::warn!("stripe: subscription {sub_id} for unknown customer {customer_id}");
        return;
    };

    let price = sub.pointer("/items/data/0/price").cloned().unwrap_or(Value::Null);
    let amount = price.get("unit_amount").and_then(|a| a.as_i64());
    let currency = price.get("currency").and_then(|c| c.as_str()).unwrap_or("usd").to_string();
    let interval = price.pointer("/recurring/interval").and_then(|i| i.as_str()).unwrap_or("month").to_string();
    let stripe_price_id = price.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
    let cancel_at_period_end = sub.get("cancel_at_period_end").and_then(|c| c.as_bool()).unwrap_or(false);
    let period_end = sub.get("current_period_end").and_then(|e| e.as_i64()).unwrap_or(0);
    let period_end = chrono::DateTime::from_timestamp(period_end, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let now = now_rfc3339();

    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM subscriptions WHERE subscription_id = ?")
        .bind(&sub_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        let _ = sqlx::query(
            "UPDATE subscriptions SET status = ?, cancel_at_period_end = ?, current_period_end = ?, \
             stripe_price_id = ?, plan_interval = ?, plan_currency = ?, plan_amount = ?, updated_at = ? \
             WHERE subscription_id = ?",
        )
        .bind(&status).bind(cancel_at_period_end).bind(&period_end)
        .bind(&stripe_price_id).bind(&interval).bind(&currency).bind(amount)
        .bind(&now).bind(&sub_id)
        .execute(&state.db).await;
    } else {
        let _ = sqlx::query(
            "INSERT INTO subscriptions (id, customer_id, subscription_id, status, cancel_at_period_end, \
             current_period_end, start_date, stripe_price_id, plan_interval, plan_currency, plan_amount, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ids::new_id()).bind(&customer_id).bind(&sub_id).bind(&status)
        .bind(cancel_at_period_end).bind(&period_end).bind(&now)
        .bind(&stripe_price_id).bind(&interval).bind(&currency).bind(amount)
        .bind(&now).bind(&now)
        .execute(&state.db).await;
    }

    // `active` and `trialing` are the states that grant access. Anything else
    // — cancelled, unpaid, incomplete — drops the member back to free.
    let paid = matches!(status.as_str(), "active" | "trialing");
    let _ = sqlx::query("UPDATE members SET status = ?, updated_at = ? WHERE id = ?")
        .bind(if paid { "paid" } else { "free" })
        .bind(&now)
        .bind(&member_id)
        .execute(&state.db)
        .await;

    // Mirror the tier membership so content gating can use it.
    if let Some(tier_id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM products WHERE monthly_price_id = ? OR yearly_price_id = ? LIMIT 1",
    )
    .bind(&stripe_price_id)
    .bind(&stripe_price_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    {
        let _ = sqlx::query("DELETE FROM members_products WHERE member_id = ?")
            .bind(&member_id)
            .execute(&state.db)
            .await;
        if paid {
            let _ = sqlx::query(
                "INSERT INTO members_products (id, member_id, product_id, sort_order) VALUES (?, ?, ?, 0)",
            )
            .bind(ids::new_id()).bind(&member_id).bind(&tier_id)
            .execute(&state.db).await;
        }
    }
    tracing::info!("stripe: subscription {sub_id} is {status}; member {member_id} is now {}", if paid { "paid" } else { "free" });

    // Starting a paid subscription is the other event automations trigger on.
    // It runs after the tier mirror above, because a tier-scoped automation
    // matches on exactly those rows. Dropping back to free triggers nothing —
    // there is no "cancelled" trigger — but the next poll will notice the
    // status change and stop any paid chain they were partway through.
    if paid {
        crate::automations_engine::trigger_for_member(state, &member_id).await;
    }
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
fn hmac_sha256_hex(key: &str, msg: &str) -> String {
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
fn secure_eq(a: &str, b: &str) -> bool {
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
        "subscriptions": [],
        "status": m.status,
        "paid": m.status != "free",
        "created_at": m.created_at,
        "enable_comment_notifications": m.enable_comment_notifications,
        "can_comment": can_comment(state, m),
        "commenting": m.commenting.to_json(),
        "newsletters": newsletters,
        "email_suppression": { "suppressed": false, "info": Value::Null },
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
            "url": format!("{}/", state.site_url()),
            "version": "6.39",
            "allow_external_signup": signup_access == "all",
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
    let _ = sqlx::query("UPDATE tokens SET used_count = used_count + 1, first_used_at = ?, updated_at = ? WHERE id = ?")
        .bind(now_rfc3339())
        .bind(now_rfc3339())
        .bind(&token_id)
        .execute(&state.db)
        .await;

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
    if current_member(&state, &headers).await.is_none() {
        return api_error(StatusCode::UNAUTHORIZED, "You must be signed in.", "UnauthorizedError");
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
}
