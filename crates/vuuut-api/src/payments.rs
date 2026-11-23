//! Hosted checkout for one-time digital Ecommerce orders.
use crate::{AppState, ecom};
use axum::{
    Form, Router, body::Bytes, extract::State, http::{HeaderMap, StatusCode}, routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use reqwest::{Client, Url};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{collections::HashMap, sync::LazyLock, time::Duration};
use vuuut_core::{GhostError, Result, ecom::PaymentMethod, ids};
use vuuut_db::repos::{ecom::Order, payments as repo};

pub const POLAR_KEY: &str = "payments_polar_api_key";
pub const POLAR_SECRET: &str = "payments_polar_webhook_secret";
pub const POLAR_SANDBOX: &str = "payments_polar_sandbox";
pub const POLAR_ENABLED: &str = "payments_polar_enabled";
pub const GUMROAD_KEY: &str = "payments_gumroad_api_key";
pub const GUMROAD_ENABLED: &str = "payments_gumroad_enabled";
const ORDER_FIELD: &str = "Vuuut order";

static CLIENT: LazyLock<Client> = LazyLock::new(|| Client::builder()
    .timeout(Duration::from_secs(12))
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .expect("payment HTTP client"));

fn failed(message: &str) -> GhostError {
    GhostError::BadRequest(message.into())
}

fn credential(state: &AppState, key: &str) -> Option<String> {
    state.settings.get_str(key).filter(|value| !value.trim().is_empty())
}

pub fn configured(state: &AppState, method: PaymentMethod) -> bool {
    match method {
        PaymentMethod::Polar => credential(state, POLAR_KEY).is_some() && credential(state, POLAR_SECRET).is_some(),
        PaymentMethod::Gumroad => credential(state, GUMROAD_KEY).is_some(),
        _ => false,
    }
}

pub fn enabled(state: &AppState, method: PaymentMethod) -> bool {
    let key = match method {
        PaymentMethod::Polar => POLAR_ENABLED,
        PaymentMethod::Gumroad => GUMROAD_ENABLED,
        _ => return false,
    };
    state.settings.get_bool(key) && configured(state, method)
}

pub fn polar_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
    })
}

pub fn gumroad_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-=+".contains(&byte))
}

fn provider_url(value: &str, domain: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?;
    if url.scheme() == "https"
        && url.username().is_empty() && url.password().is_none() && url.port().is_none()
        && (host == domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.')))
    {
        Some(url)
    } else {
        None
    }
}

async fn request(builder: reqwest::RequestBuilder) -> Result<Value> {
    builder.send().await.ok()
        .and_then(|resp| resp.error_for_status().ok())
        .ok_or_else(|| failed("The payment provider did not accept the request. Check the provider settings."))?
        .json().await
        .map_err(|_| failed("The payment provider sent an invalid response."))
}

fn gumroad_api(path: &str, id: &str) -> Url {
    let mut url = Url::parse("https://api.gumroad.com/v2").expect("Gumroad API URL");
    url.path_segments_mut().expect("Gumroad API path").extend([path, id]);
    url
}

fn currency_matches(value: &Value, key: &str, order: &Order) -> bool {
    value[key].as_str().is_some_and(|currency| currency.eq_ignore_ascii_case(&order.currency))
}

fn polar_checkout_response(value: &Value, product: &str, order: &Order) -> Option<(String, String)> {
    let checkout_id = value["id"].as_str().filter(|id| polar_id(id))?;
    let mut prices = value.pointer("/product/prices")?.as_array()?.iter()
        .filter(|price| price["is_archived"] != true);
    let price = prices.next()?;
    if prices.next().is_some() || price["amount_type"] != "fixed"
        || price["price_amount"].as_i64() != Some(order.total_cents)
        || !currency_matches(price, "price_currency", order)
        || value["product_id"] != product || value["amount"].as_i64() != Some(order.total_cents)
        || value["discount_amount"].as_i64() != Some(0) || !currency_matches(value, "currency", order)
        || value.pointer("/product/is_recurring") != Some(&Value::Bool(false))
        || value["is_payment_required"] != true
    {
        return None;
    }
    let url = provider_url(value["url"].as_str()?, "polar.sh")?;
    Some((checkout_id.into(), url.to_string()))
}

async fn create_polar(state: &AppState, order: &Order, product: &str) -> Result<(String, String)> {
    let key = credential(state, POLAR_KEY).ok_or_else(|| failed("Configure Polar in Payments."))?;
    let host = if state.settings.get_bool(POLAR_SANDBOX) { "sandbox-api.polar.sh" } else { "api.polar.sh" };
    let return_url = ecom::order_url(state, order);
    let value = request(CLIENT.post(format!("https://{host}/v1/checkouts/"))
        .bearer_auth(key)
        .json(&json!({
            "products": [product],
            "metadata": {"vuuut_order": order.id},
            "customer_email": order.email,
            "allow_discount_codes": false,
            "allow_trial": false,
            "success_url": return_url,
            "return_url": return_url,
        }))).await?;
    polar_checkout_response(&value, product, order)
        .ok_or_else(|| failed("The Polar product must be a one-time product with the same price and currency."))
}

fn gumroad_checkout_response(value: &Value, product: &str, order: &Order) -> Option<String> {
    let item = &value["product"];
    if value["success"] != true || item["id"] != product || item["published"] != true
        || item["price"].as_i64() != Some(order.total_cents)
        || !currency_matches(item, "currency", order)
        || item["is_recurring_billing"] == true || item["is_tiered_membership"] == true
        || item["is_physical"] == true || item["require_shipping"] == true
        || item["customizable_price"] == true
        || item["variants"].as_array().is_some_and(|variants| !variants.is_empty())
    {
        return None;
    }
    let mut url = provider_url(item["short_url"].as_str()?, "gumroad.com")?;
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut().append_pair("wanted", "true")
        .append_pair("quantity", "1")
        .append_pair("email", &order.email)
        .append_pair("vuuut_order", &order.id)
        .append_pair(ORDER_FIELD, &order.id);
    Some(url.to_string())
}

async fn create_gumroad(state: &AppState, order: &Order, product: &str) -> Result<(String, String)> {
    let key = credential(state, GUMROAD_KEY).ok_or_else(|| failed("Configure Gumroad in Payments."))?;
    let value = request(CLIENT.get(gumroad_api("products", product)).bearer_auth(key)).await?;
    let url = gumroad_checkout_response(&value, product, order)
        .ok_or_else(|| failed("The Gumroad product must be published with the same price and currency."))?;
    Ok((String::new(), url))
}

pub async fn hosted_checkout(state: &AppState, order: &Order) -> Result<String> {
    if order.financial_status != "pending" || order.is_cancelled() {
        return Err(failed("This order no longer accepts payments."));
    }
    let payment = repo::hosted(&state.db, &order.id).await?
        .ok_or_else(|| failed("This order has no hosted payment."))?;
    let method = PaymentMethod::from_key(&payment.provider)
        .filter(|method| enabled(state, *method))
        .ok_or_else(|| failed("This payment method is unavailable."))?;
    if !payment.checkout_url.is_empty() {
        return Ok(payment.checkout_url);
    }
    let (id, url) = match method {
        PaymentMethod::Polar => create_polar(state, order, &payment.product_id).await?,
        PaymentMethod::Gumroad => create_gumroad(state, order, &payment.product_id).await?,
        _ => return Err(failed("This payment method has no hosted checkout.")),
    };
    repo::register_checkout(&state.db, &order.id, &id, &url).await?;
    repo::hosted(&state.db, &order.id).await?.map(|payment| payment.checkout_url)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| failed("Could not save the hosted checkout."))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/members/webhooks/polar/", post(polar_webhook))
        .route("/members/webhooks/polar", post(polar_webhook))
        .route("/members/webhooks/gumroad/", post(gumroad_webhook))
        .route("/members/webhooks/gumroad", post(gumroad_webhook))
}

fn valid_polar_signature(headers: &HeaderMap, body: &[u8], secret: &str, now: i64) -> bool {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let (Some(id), Some(timestamp), Some(signatures)) =
        (header("webhook-id"), header("webhook-timestamp"), header("webhook-signature"))
    else { return false };
    if id.is_empty() || !timestamp.parse::<i64>().is_ok_and(|time| now.abs_diff(time) <= 300) {
        return false;
    }
    let verify = |key: &[u8]| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else { return false };
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        signatures.split_ascii_whitespace()
            .filter_map(|sig| sig.strip_prefix("v1,").and_then(|sig| STANDARD.decode(sig).ok()))
            .any(|sig| mac.clone().verify_slice(&sig).is_ok())
    };
    // Older Polar endpoints and CLI listeners sign with the raw secret.
    verify(secret.as_bytes()) || secret.strip_prefix("polar_whs_")
        .or_else(|| secret.strip_prefix("whsec_"))
        .and_then(|secret| STANDARD.decode(secret).ok())
        .is_some_and(|key| key.len() == 32 && verify(&key))
}

fn polar_receipt_matches(sale: &Value, order: &Order, payment: &repo::HostedPayment) -> bool {
    !payment.checkout_id.is_empty()
        && sale["checkout_id"] == payment.checkout_id && sale["product_id"] == payment.product_id
        && sale["status"] == "paid" && sale["paid"] == true && sale["billing_reason"] == "purchase"
        && sale["subscription_id"].is_null()
        && sale["net_amount"].as_i64() == Some(order.total_cents)
        && currency_matches(sale, "currency", order)
}

async fn polar_webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(secret) = credential(&state, POLAR_SECRET) else { return StatusCode::NOT_FOUND };
    if !valid_polar_signature(&headers, &body, &secret, chrono::Utc::now().timestamp()) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else { return StatusCode::BAD_REQUEST };
    if event["type"] != "order.paid" {
        return StatusCode::NO_CONTENT;
    }
    let sale = &event["data"];
    let (Some(id), Some(receipt)) = (
        sale.pointer("/metadata/vuuut_order").and_then(Value::as_str).filter(|id| ids::is_objectid(id)),
        sale["id"].as_str().filter(|id| polar_id(id)),
    ) else { return StatusCode::NO_CONTENT };
    let Some(order) = vuuut_db::repos::ecom::find_order(&state.db, id).await else { return StatusCode::NO_CONTENT };
    let payment = match repo::hosted(&state.db, id).await {
        Ok(Some(payment)) if payment.provider == "polar" => payment,
        Ok(_) => return StatusCode::NO_CONTENT,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };
    if !polar_receipt_matches(sale, &order, &payment) {
        return StatusCode::NO_CONTENT;
    }
    settle(&state, &order, receipt, "Polar").await
}

fn gumroad_order(sale: &Value) -> Option<&str> {
    sale.pointer("/custom_fields/Vuuut order").and_then(Value::as_str)
        .or_else(|| sale.pointer("/url_params/vuuut_order").and_then(Value::as_str))
        .filter(|id| ids::is_objectid(id))
}

fn gumroad_receipt_matches(sale: &Value, receipt: &str, order: &Order, payment: &repo::HostedPayment) -> bool {
    !payment.checkout_url.is_empty()
        && sale["id"] == receipt && gumroad_order(sale) == Some(order.id.as_str())
        && sale["product_id"] == payment.product_id
        && sale["email"].as_str().is_some_and(|email| email.eq_ignore_ascii_case(&order.email))
        && sale["paid"] == true && sale["refunded"] == false && sale["chargedback"] == false
        && sale["partially_refunded"] != true && sale["is_recurring_billing"] == false
        && sale["is_product_physical"] == false && sale["is_gift_sender_purchase"] != true
        && sale["is_gift_receiver_purchase"] != true && sale["test"] != true && sale["is_test_purchase"] != true
        && sale["quantity"].as_i64().unwrap_or(1) == 1
        && sale["price"].as_i64() == Some(order.total_cents)
        && currency_matches(sale, "currency", order)
}

async fn gumroad_webhook(State(state): State<AppState>, Form(form): Form<HashMap<String, String>>) -> StatusCode {
    let Some(key) = credential(&state, GUMROAD_KEY) else { return StatusCode::NOT_FOUND };
    // Ping is unsigned. Use its sale ID only to retrieve the seller's authenticated
    // sale. Require a local order reference before spending an API request.
    let (Some(id), Some(receipt)) = (
        form.get(ORDER_FIELD).or_else(|| form.get("custom_fields[Vuuut order]"))
            .or_else(|| form.get("url_params[vuuut_order]")).filter(|id| ids::is_objectid(id)),
        form.get("sale_id").filter(|id| gumroad_id(id)),
    ) else { return StatusCode::NO_CONTENT };
    let Some(order) = vuuut_db::repos::ecom::find_order(&state.db, id).await else { return StatusCode::NO_CONTENT };
    if order.financial_status != "pending" || order.is_cancelled() {
        return StatusCode::NO_CONTENT;
    }
    let payment = match repo::hosted(&state.db, id).await {
        Ok(Some(payment)) if payment.provider == "gumroad" => payment,
        Ok(_) => return StatusCode::NO_CONTENT,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };
    let Ok(data) = request(CLIENT.get(gumroad_api("sales", receipt)).bearer_auth(&key)).await else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let sale = &data["sale"];
    let Some(sale_order) = sale["order_id"].as_u64() else { return StatusCode::NO_CONTENT };
    if data["success"] != true || !gumroad_receipt_matches(sale, receipt, &order, &payment) {
        return StatusCode::NO_CONTENT;
    }
    // The single-sale endpoint can include test or unfinished sales and its
    // "paid" flag only describes the price. The filtered sales collection
    // includes successful purchases, so require the same sale there as well.
    let Ok(confirmed) = request(CLIENT.get("https://api.gumroad.com/v2/sales")
        .bearer_auth(key)
        .query(&[("order_id", sale_order.to_string()), ("product_id", payment.product_id.clone())])).await
    else { return StatusCode::SERVICE_UNAVAILABLE };
    let verified = confirmed["success"] == true && confirmed["sales"].as_array().is_some_and(|sales| {
        sales.iter().any(|sale| gumroad_receipt_matches(sale, receipt, &order, &payment))
    });
    if !verified {
        return StatusCode::NO_CONTENT;
    }
    settle(&state, &order, receipt, "Gumroad").await
}

async fn settle(state: &AppState, order: &Order, receipt: &str, provider: &str) -> StatusCode {
    match repo::settle(&state.db, order, receipt, &format!("Payment confirmed by {provider}.")).await {
        Ok(true) => {
            if let Some(order) = vuuut_db::repos::ecom::find_order(&state.db, &order.id).await {
                ecom::on_paid(state, &order).await;
                ecom::receipt(state, &order).await;
                ecom::notify_staff(state, &order).await;
            }
            StatusCode::NO_CONTENT
        }
        Ok(false) => StatusCode::NO_CONTENT,
        Err(error) => {
            tracing::warn!("could not record hosted payment for order {}: {error}", order.id);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}
