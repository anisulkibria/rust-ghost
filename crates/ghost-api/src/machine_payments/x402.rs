//! Native x402 v2 verification/settlement using the configured public facilitator.
use super::*;
use base64::Engine;
pub fn requirement(state: &AppState, config: &Config) -> Option<Value> {
    if !config.x402_enabled {
        return None;
    }
    let stripe = ghost_stripe::StripeConfig::from_settings(&state.settings)?;
    if stripe.is_live() != (config.x402_network == "eip155:8453") {
        return None;
    }
    let recipient = config::address(state, "base")?;
    let asset = if config.x402_network == "eip155:8453" {
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
    } else {
        "0x036CbD53842c5426634e7929541eC2318f3dCF7e"
    };
    Some(
        json!({"scheme":"exact","network":config.x402_network,"amount":(config.amount.checked_mul(10000)?).to_string(),"asset":asset,"payTo":recipient,"maxTimeoutSeconds":300,"extra":{"name":"USDC","version":"2"}}),
    )
}
pub fn challenge(resource: &str, required: &Value) -> Value {
    json!({"x402Version":2,"resource":{"url":resource,"description":"Paid article Markdown","mimeType":"text/markdown"},"accepts":[required]})
}
pub fn parse(header: &str, resource: &str, required: &Value) -> Result<(Value, String)> {
    if header.len() > 64 * 1024 {
        return Err(bad("Payment credential is too large"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(header)
        .map_err(|_| bad("Invalid x402 credential"))?;
    let payload: Value =
        serde_json::from_slice(&bytes).map_err(|_| bad("Invalid x402 credential"))?;
    if payload["x402Version"] != 2
        || payload["accepted"] != *required
        || payload
            .pointer("/resource/url")
            .and_then(Value::as_str)
            .is_some_and(|url| url != resource)
    {
        return Err(bad("Payment credential does not match this resource"));
    }
    let signature = payload
        .pointer("/payload/signature")
        .and_then(Value::as_str)
        .filter(|s| s.len() >= 128 && s.len() <= 1024)
        .ok_or_else(|| bad("Missing payment signature"))?;
    let authorization = &payload["payload"]["authorization"];
    if !authorization["to"].as_str().is_some_and(|to| {
        required["payTo"]
            .as_str()
            .is_some_and(|payee| to.eq_ignore_ascii_case(payee))
    }) || authorization["value"] != required["amount"]
    {
        return Err(bad("Payment recipient or amount mismatch"));
    }
    let from = authorization["from"]
        .as_str()
        .filter(|s| config::valid_address(s))
        .ok_or_else(|| bad("Invalid payer"))?;
    let nonce = authorization["nonce"]
        .as_str()
        .filter(|s| s.len() == 66 && s.starts_with("0x"))
        .ok_or_else(|| bad("Invalid payment nonce"))?;
    let now = chrono::Utc::now().timestamp();
    let expiry = authorization["validBefore"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| authorization["validBefore"].as_i64())
        .ok_or_else(|| bad("Missing payment expiry"))?;
    if expiry <= now || expiry > now + 3600 {
        return Err(bad(
            "Payment credential is expired or its expiry is too far away",
        ));
    }
    let key = hash(&format!(
        "x402:{}:{}:{}:{}",
        required["network"],
        required["asset"],
        from.to_lowercase(),
        nonce.to_lowercase()
    ));
    let _ = signature;
    Ok((payload, key))
}
pub async fn settle(config: &Config, payload: &Value, required: &Value) -> Result<Value> {
    let endpoint = config.x402_facilitator.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| bad(&e.to_string()))?;
    let body = json!({"x402Version":2,"paymentPayload":payload,"paymentRequirements":required});
    let verification = client
        .post(format!("{endpoint}/verify"))
        .json(&body)
        .send()
        .await
        .map_err(|_| bad("Payment verifier could not be reached"))?;
    if !verification.status().is_success() {
        return Err(bad("Payment verification failed"));
    }
    let verification: Value = verification
        .json()
        .await
        .map_err(|_| bad("Invalid verifier response"))?;
    if verification["isValid"] != true {
        return Err(bad("Payment credential rejected"));
    }
    let settled = client
        .post(format!("{endpoint}/settle"))
        .json(&body)
        .send()
        .await
        .map_err(|_| bad("Payment settlement could not be confirmed"))?;
    if !settled.status().is_success() {
        return Err(bad("Payment settlement failed"));
    }
    let settled: Value = settled
        .json()
        .await
        .map_err(|_| bad("Invalid settlement response"))?;
    if settled["success"] != true
        || settled["network"] != required["network"]
        || settled["transaction"]
            .as_str()
            .is_none_or(|s| s.len() != 66 || !s.starts_with("0x"))
    {
        return Err(bad("Payment was not settled on the required network"));
    }
    Ok(settled)
}
