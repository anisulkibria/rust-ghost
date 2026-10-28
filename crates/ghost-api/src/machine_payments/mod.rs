//! Per-request paid Markdown access. No membership or browser session is created.
mod admin;
mod config;
mod store;
mod x402;
use crate::AppState;
use axum::{
    http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
pub use config::Config;
use ghost_core::{dates, models::Post, GhostError, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn bad(message: &str) -> GhostError {
    GhostError::BadRequest(message.into())
}
fn db_error(error: sqlx::Error) -> GhostError {
    GhostError::Db(error.to_string())
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn admin_router() -> axum::Router<AppState> {
    admin::router()
}
fn stripe_method(
    state: &AppState,
    config: &Config,
) -> Result<Option<mpp::server::Mpp<mpp::server::StripeChargeMethod>>> {
    if config.stripe_profile_id.is_empty() || config.amount < 50 {
        return Ok(None);
    }
    let Some(stripe) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
        return Ok(None);
    };
    let Some(secret) = config::secret(state) else {
        return Ok(None);
    };
    install_crypto();
    mpp::server::Mpp::create_stripe(
        mpp::server::stripe(mpp::server::StripeConfig {
            secret_key: &stripe.secret_key,
            network_id: &config.stripe_profile_id,
            payment_method_types: &["card", "link"],
            currency: &config.currency.to_lowercase(),
            decimals: 2,
        })
        .realm(&config::realm(state)?)
        .secret_key(&secret),
    )
    .map(Some)
    .map_err(|e| bad(&e.to_string()))
}
fn tempo_method(
    state: &AppState,
    config: &Config,
) -> Result<Option<mpp::server::Mpp<mpp::server::TempoChargeMethod<mpp::server::TempoProvider>>>> {
    if !config.tempo_enabled {
        return Ok(None);
    }
    let Some(recipient) = config::address(state, "tempo") else {
        return Ok(None);
    };
    let Some(secret) = config::secret(state) else {
        return Ok(None);
    };
    let stripe = ghost_stripe::StripeConfig::from_settings(&state.settings)
        .ok_or_else(|| bad("Connect Stripe first"))?;
    if stripe.is_live() == config.tempo_testnet {
        return Ok(None);
    }
    install_crypto();
    let rpc = if config.tempo_testnet {
        "https://rpc.moderato.tempo.xyz"
    } else {
        "https://rpc.tempo.xyz"
    };
    mpp::server::Mpp::create(
        mpp::server::tempo(mpp::server::TempoConfig {
            recipient: &recipient,
        })
        .currencies(["0x20c000000000000000000000b9537d11c60e8b50"])
        .decimals(6)
        .rpc_url(rpc)
        .realm(&config::realm(state)?)
        .secret_key(&secret)
        .store(std::sync::Arc::new(store::SqliteStore(state.db.clone()))),
    )
    .map(Some)
    .map_err(|e| bad(&e.to_string()))
}
fn install_crypto() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
fn stripe_options<'a>(post: &'a Post, scope: &'a Value) -> mpp::server::StripeChargeOptions<'a> {
    mpp::server::StripeChargeOptions {
        external_id: Some(&post.id),
        mppx_scope: Some(scope),
        ..Default::default()
    }
}
fn tempo_options<'a>(post: &'a Post, scope: &'a Value) -> mpp::server::ChargeOptions<'a> {
    mpp::server::ChargeOptions {
        external_id: Some(&post.id),
        mppx_scope: Some(scope),
        ..Default::default()
    }
}
fn preflight(
    credential: &mpp::PaymentCredential,
    secret: &str,
    realm: &str,
    method: &str,
    expected: &mpp::protocol::intents::ChargeRequest,
) -> Result<()> {
    let challenge: mpp::PaymentChallenge = serde_json::from_value(
        serde_json::to_value(&credential.challenge)
            .map_err(|_| bad("Invalid payment challenge"))?,
    )
    .map_err(|_| bad("Invalid payment challenge"))?;
    if !challenge.verify(secret)
        || challenge.expires.is_none()
        || challenge.is_expired()
        || challenge.realm != realm
        || challenge.method.as_str() != method
        || challenge.intent.as_str() != "charge"
        || challenge.header.is_some()
        || challenge.digest.is_some()
    {
        return Err(bad("Invalid or expired payment challenge"));
    }
    let actual: mpp::protocol::intents::ChargeRequest = challenge
        .request
        .decode()
        .map_err(|_| bad("Invalid payment request"))?;
    if actual.amount != expected.amount
        || actual.currency != expected.currency
        || actual.recipient != expected.recipient
        || actual.external_id != expected.external_id
        || actual.method_details != expected.method_details
        || actual.mppx_scope != expected.mppx_scope
    {
        return Err(bad(
            "Payment does not match the article, amount, currency or recipient",
        ));
    }
    Ok(())
}
async fn reserve(
    state: &AppState,
    post: &Post,
    resource: &str,
    config: &Config,
    protocol: &str,
    method: &str,
    key: &str,
) -> Result<()> {
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM machine_payment_attempts WHERE datetime(created_at)>datetime('now','-1 minute')").fetch_one(&state.db).await.map_err(db_error)?;
    if count >= 1000 {
        return Err(bad("Payment verification is temporarily busy"));
    }
    let now = dates::now_db();
    let result=sqlx::query("INSERT OR IGNORE INTO machine_payment_attempts (credential_hash,post_id,resource,protocol,method,amount,currency,status,created_at,updated_at) VALUES (?,?,?,?,?,?,?,'pending',?,?)")
        .bind(key).bind(&post.id).bind(resource).bind(protocol).bind(method).bind(config.amount).bind(&config.currency).bind(&now).bind(&now).execute(&state.db).await.map_err(db_error)?;
    if result.rows_affected() != 1 {
        let retry=sqlx::query("UPDATE machine_payment_attempts SET status='pending',updated_at=? WHERE credential_hash=? AND (status='failed' OR (status='pending' AND datetime(updated_at)<datetime('now','-2 minutes')))").bind(dates::now_db()).bind(key).execute(&state.db).await.map_err(db_error)?;
        if retry.rows_affected() != 1 {
            return Err(bad(
                "This payment credential is already being processed or has been used",
            ));
        }
    }
    Ok(())
}
async fn settled(state: &AppState, key: &str, reference: &str) -> Result<()> {
    if reference.is_empty() || reference.len() > 255 {
        return Err(bad("Missing settlement reference"));
    }
    let result=sqlx::query("UPDATE machine_payment_attempts SET status='settled',reference=?,updated_at=? WHERE credential_hash=? AND status='pending'").bind(reference).bind(dates::now_db()).bind(key).execute(&state.db).await.map_err(|_|bad("Settlement was already used"))?;
    if result.rows_affected() != 1 {
        return Err(bad("Settlement could not be recorded"));
    }
    Ok(())
}
async fn failed(state: &AppState, key: &str) {
    let _=sqlx::query("UPDATE machine_payment_attempts SET status='failed',updated_at=? WHERE credential_hash=? AND status='pending'").bind(dates::now_db()).bind(key).execute(&state.db).await;
}
fn paid_body(post: &Post, receipt_headers: Vec<(&str, String)>) -> Response {
    let raw = post.html.as_deref().unwrap_or("");
    let html = ghost_core::models::segment_html(raw, Some("paid"));
    let body = if html == raw {
        post.markdown
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ghost_render::markdown::html_to_plaintext(&html))
    } else {
        ghost_render::markdown::html_to_plaintext(&html)
    };
    let mut response = (
        [
            (header::CONTENT_TYPE, "text/markdown; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::VARY, "Authorization, Payment-Signature, Cookie"),
        ],
        body,
    )
        .into_response();
    for (key, value) in receipt_headers {
        if let Ok(value) = value.parse() {
            response.headers_mut().insert(
                axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value,
            );
        }
    }
    response
}
/// Only called from the explicit published `.md` branch, after eligibility checks.
pub async fn serve(
    state: &AppState,
    post: &Post,
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<Response> {
    let config = Config::load(state);
    let labs = state
        .settings
        .get("labs")
        .is_some_and(|labs| labs["machinePayments"] == true);
    if !config.enabled
        || !labs
        || !state.settings.get_bool("llms_enabled")
        || post.status != "published"
        || !matches!(post.visibility.as_str(), "paid" | "tiers")
        || ghost_stripe::StripeConfig::from_settings(&state.settings).is_none()
    {
        return None;
    }
    if config.validate().is_err() {
        return None;
    }
    if post.visibility == "tiers"
        && (post.tier_ids.is_empty()
            || ghost_db::repos::tiers::validate_post_tiers(&state.db, "tiers", &post.tier_ids)
                .await
                .is_err())
    {
        return None;
    }
    let stripe = match stripe_method(state, &config) {
        Ok(method) => method,
        Err(_) => return None,
    };
    let tempo = match tempo_method(state, &config) {
        Ok(method) => method,
        Err(_) => return None,
    };
    let x402 = x402::requirement(state, &config);
    if stripe.is_none() && tempo.is_none() && x402.is_none() {
        return None;
    }
    let resource = ghost_core::url::absolute(&state.site_url(), &ghost_core::url::base(uri.path()));
    let scope = json!(uri.path());
    let amount = config.amount_string();
    let mut challenges = Vec::new();
    if let Some(method) = &stripe {
        if let Ok(challenge) =
            method.stripe_charge_with_options(&amount, stripe_options(post, &scope))
        {
            if let Ok(header) = challenge.to_header() {
                challenges.push(header)
            }
        }
    }
    if let Some(method) = &tempo {
        if let Ok(values) = method.charge_with_options(&amount, tempo_options(post, &scope)) {
            for challenge in values {
                if let Ok(header) = challenge.to_header() {
                    challenges.push(header)
                }
            }
        }
    }
    let mut error_message = None;
    if let Some(raw) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            value
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Payment "))
        })
    {
        if raw.len() <= 64 * 1024 {
            match mpp::PaymentCredential::from_header(raw) {
                Ok(credential) => {
                    let secret = config::secret(state).unwrap_or_default();
                    let realm = config::realm(state).unwrap_or_default();
                    let method = credential.challenge.method.as_str();
                    let expected = if method == "stripe" {
                        stripe.as_ref().and_then(|m| {
                            m.stripe_expected_charge_request(&amount, stripe_options(post, &scope))
                                .ok()
                        })
                    } else if method == "tempo" {
                        tempo.as_ref().and_then(|m| {
                            m.expected_charge_request(&amount, tempo_options(post, &scope))
                                .ok()
                        })
                    } else {
                        None
                    };
                    if let Some(expected) = expected {
                        match preflight(&credential, &secret, &realm, method, &expected) {
                            Ok(()) => {
                                let reference = if method == "stripe" {
                                    credential.payload["spt"].as_str().map(str::to_owned)
                                } else {
                                    credential
                                        .charge_payload()
                                        .ok()
                                        .map(|payload| payload.reference().to_string())
                                };
                                if let Some(reference) = reference
                                    .filter(|value| !value.is_empty() && value.len() < 32768)
                                {
                                    let key = hash(&format!(
                                        "mpp:{method}:{}:{reference}",
                                        if method == "stripe" {
                                            credential.challenge.id.as_str()
                                        } else {
                                            ""
                                        }
                                    ));
                                    match reserve(
                                        state, post, &resource, &config, "mpp", method, &key,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            let payment = if method == "stripe" {
                                                settle_stripe(state, &config, post, &credential)
                                                    .await
                                            } else {
                                                match tokio::time::timeout(
                                                    std::time::Duration::from_secs(45),
                                                    tempo
                                                        .as_ref()
                                                        .unwrap()
                                                        .verify_charge_with_options(
                                                            &credential,
                                                            &amount,
                                                            tempo_options(post, &scope),
                                                        ),
                                                )
                                                .await
                                                {
                                                    Ok(Ok(receipt)) => Ok(receipt),
                                                    _ => Err(bad("Payment could not be verified")),
                                                }
                                            };
                                            match payment {
                                                Ok(receipt) if receipt.is_success() => {
                                                    let reference = if method == "tempo" {
                                                        receipt.reference.to_lowercase()
                                                    } else {
                                                        receipt.reference.clone()
                                                    };
                                                    match settled(state, &key, &reference).await {
                                                        Ok(()) => {
                                                            return Some(paid_body(
                                                                post,
                                                                vec![(
                                                                    "payment-receipt",
                                                                    receipt
                                                                        .to_header()
                                                                        .unwrap_or_default(),
                                                                )],
                                                            ))
                                                        }
                                                        Err(error) => {
                                                            error_message = Some(error.message())
                                                        }
                                                    }
                                                }
                                                Ok(_) => {
                                                    error_message =
                                                        Some("Payment was not settled".into())
                                                }
                                                Err(error) => error_message = Some(error.message()),
                                            }
                                            failed(state, &key).await;
                                        }
                                        Err(error) => error_message = Some(error.message()),
                                    }
                                } else {
                                    error_message = Some("Invalid payment payload".into())
                                }
                            }
                            Err(error) => error_message = Some(error.message()),
                        }
                    } else {
                        error_message = Some("Payment method is unavailable".into())
                    }
                }
                Err(_) => error_message = Some("Invalid payment credential".into()),
            }
        } else {
            error_message = Some("Payment credential is too large".into())
        }
    } else if let (Some(raw), Some(required)) = (
        headers
            .get("payment-signature")
            .and_then(|value| value.to_str().ok()),
        x402.as_ref(),
    ) {
        match x402::parse(raw, &resource, required) {
            Ok((payload, key)) => {
                match reserve(state, post, &resource, &config, "x402", "evm", &key).await {
                    Ok(()) => {
                        match x402::settle(&config, &payload, required).await {
                            Ok(receipt) => {
                                let reference =
                                    receipt["transaction"].as_str().unwrap_or("").to_lowercase();
                                match settled(state, &key, &reference).await {
                                    Ok(()) => {
                                        return Some(paid_body(
                                            post,
                                            vec![(
                                                "payment-response",
                                                base64::engine::general_purpose::STANDARD
                                                    .encode(receipt.to_string()),
                                            )],
                                        ))
                                    }
                                    Err(error) => error_message = Some(error.message()),
                                }
                            }
                            Err(error) => error_message = Some(error.message()),
                        }
                        failed(state, &key).await;
                    }
                    Err(error) => error_message = Some(error.message()),
                }
            }
            Err(error) => error_message = Some(error.message()),
        }
    }
    let mut body=x402.as_ref().map(|required|x402::challenge(&resource,required)).unwrap_or_else(||json!({"type":"https://paymentauth.org/problems/payment-required","title":"Payment required","status":402}));
    if let Some(error) = error_message {
        body["error"] = json!(error)
    }
    let mut response = (
        StatusCode::PAYMENT_REQUIRED,
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::VARY, "Authorization, Payment-Signature, Cookie"),
        ],
        Json(body.clone()),
    )
        .into_response();
    for challenge in challenges {
        if let Ok(challenge) = challenge.parse() {
            response
                .headers_mut()
                .append(header::WWW_AUTHENTICATE, challenge);
        }
    }
    if x402.is_some() {
        if let Ok(encoded) = base64::engine::general_purpose::STANDARD
            .encode(body.to_string())
            .parse()
        {
            response.headers_mut().insert("payment-required", encoded);
        }
    }
    Some(response)
}
async fn settle_stripe(
    state: &AppState,
    config: &Config,
    post: &Post,
    credential: &mpp::PaymentCredential,
) -> Result<mpp::Receipt> {
    let token = credential.payload["spt"]
        .as_str()
        .filter(|token| token.starts_with("spt_"))
        .ok_or_else(|| bad("Missing Stripe payment token"))?;
    if credential.payload["externalId"].as_str() != Some(&post.id) {
        return Err(bad("Payment reference does not match this article"));
    }
    let stripe = ghost_stripe::StripeConfig::from_settings(&state.settings)
        .ok_or_else(|| bad("Stripe is unavailable"))?;
    let idempotency = format!("mpp_{}_{}", credential.challenge.id, token);
    let payment = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        ghost_stripe::machine_payment_intent(
            &stripe,
            token,
            config.amount,
            &config.currency,
            &idempotency,
            &post.id,
        ),
    )
    .await
    .map_err(|_| bad("Payment confirmation timed out"))??;
    if payment["status"] != "succeeded"
        || payment["amount"].as_i64() != Some(config.amount)
        || !payment["currency"]
            .as_str()
            .is_some_and(|currency| currency.eq_ignore_ascii_case(&config.currency))
    {
        return Err(bad("Payment was not completed for the required amount"));
    }
    let id = payment["id"]
        .as_str()
        .filter(|id| id.starts_with("pi_"))
        .ok_or_else(|| bad("Missing payment reference"))?;
    Ok(mpp::Receipt::success("stripe", id).with_external_id(&post.id))
}
