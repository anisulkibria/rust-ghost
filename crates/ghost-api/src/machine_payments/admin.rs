use super::*;
use crate::permissions::{self, Role};
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Router,
};
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/machine-payments/settings/", get(read).put(save))
        .route("/machine-payments/deposit-addresses/", post(deposit))
        .route("/machine-payments/payments/", get(payments))
}
fn failure(error: GhostError) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":error.message(),"errorType":"ValidationError"}]})),
    )
        .into_response()
}
async fn authorize(state: &AppState, headers: &HeaderMap) -> std::result::Result<(), Response> {
    permissions::require(state, headers, Role::Administrator)
        .await
        .map(|_| ())
}
async fn read(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let config = Config::load(&state);
    let methods = json!({"stripe":!config.stripe_profile_id.is_empty()&&config.amount>=50,"tempo":config.tempo_enabled&&config::address(&state,"tempo").is_some(),"x402":x402::requirement(&state,&config).is_some()});
    ([("cache-control","private, no-store")],Json(json!({"machine_payments":config,"methods":methods,"deposit_addresses":{"base":config::address(&state,"base"),"tempo":config::address(&state,"tempo")},"stripe_connected":ghost_stripe::StripeConfig::from_settings(&state.settings).is_some()}))).into_response()
}
async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(config): Json<Config>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    if let Err(e) = config.validate() {
        return failure(e);
    }
    if config.enabled && ghost_stripe::StripeConfig::from_settings(&state.settings).is_none() {
        return failure(bad("Connect Stripe before enabling machine payments"));
    }
    if config.enabled && state.settings.get_bool("is_private") {
        return failure(bad("Machine payments require a public publication"));
    }
    // Disable acceptance while updating economic terms; enable only after every field is saved.
    if let Err(e) = state
        .settings
        .set(&state.db, "machine_payments_enabled", &json!(false))
        .await
    {
        return failure(e);
    }
    if config::secret(&state).is_none() {
        if let Err(e) = state
            .settings
            .set(
                &state.db,
                "machine_payments_secret",
                &json!(ghost_core::ids::random_hex(32)),
            )
            .await
        {
            return failure(e);
        }
    }
    for (key, value) in [
        ("machine_payments_amount", json!(config.amount)),
        (
            "machine_payments_currency",
            json!(config.currency.to_uppercase()),
        ),
        (
            "machine_payments_stripe_profile_id",
            json!(config.stripe_profile_id),
        ),
        (
            "machine_payments_tempo_enabled",
            json!(config.tempo_enabled),
        ),
        (
            "machine_payments_tempo_testnet",
            json!(config.tempo_testnet),
        ),
        ("machine_payments_x402_enabled", json!(config.x402_enabled)),
        ("machine_payments_x402_network", json!(config.x402_network)),
        (
            "machine_payments_x402_facilitator",
            json!(config.x402_facilitator),
        ),
    ] {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            return failure(e);
        }
    }
    if config.enabled {
        let mut labs = state
            .settings
            .get("labs")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        labs.insert("machinePayments".into(), json!(true));
        if let Err(e) = state
            .settings
            .set(&state.db, "labs", &Value::Object(labs))
            .await
        {
            return failure(e);
        }
        if let Err(e) = state
            .settings
            .set(&state.db, "llms_enabled", &json!(true))
            .await
        {
            return failure(e);
        }
    }
    if let Err(e) = state
        .settings
        .set(
            &state.db,
            "machine_payments_enabled",
            &json!(config.enabled),
        )
        .await
    {
        return failure(e);
    }
    read(State(state), headers).await
}
async fn deposit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let network = body["network"].as_str().unwrap_or("");
    if !matches!(network, "base" | "tempo") {
        return failure(bad("Choose Base or Tempo"));
    }
    if config::address(&state, network).is_none() {
        let Some(stripe) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
            return failure(bad("Connect Stripe first"));
        };
        let address = match ghost_stripe::machine_deposit_address(&stripe, network).await {
            Ok(value) => value,
            Err(e) => return failure(e),
        };
        let Some(address) = address["address"]
            .as_str()
            .filter(|value| config::valid_address(value))
        else {
            return failure(bad("Stripe returned no valid deposit address"));
        };
        let mut map = state
            .settings
            .get("machine_payments_deposit_address")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        map.insert(network.into(), json!(address));
        if let Err(e) = state
            .settings
            .set(
                &state.db,
                "machine_payments_deposit_address",
                &Value::Object(map),
            )
            .await
        {
            return failure(e);
        }
    }
    read(State(state), headers).await
}
async fn payments(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,String,String,i64,String,String,Option<String>,String)>("SELECT post_id,protocol,method,amount,currency,status,reference,created_at FROM machine_payment_attempts ORDER BY created_at DESC LIMIT 100").fetch_all(&state.db).await.unwrap_or_default();
    ([("cache-control","private, no-store")],Json(json!({"payments":rows.into_iter().map(|(post_id,protocol,method,amount,currency,status,reference,created_at)|json!({"post_id":post_id,"protocol":protocol,"method":method,"amount":amount,"currency":currency,"status":status,"reference":reference,"created_at":created_at})).collect::<Vec<_>>()}))).into_response()
}
