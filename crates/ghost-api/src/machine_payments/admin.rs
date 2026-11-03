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
    let stripe = ghost_stripe::StripeConfig::from_settings(&state.settings);
    // Everything `serve` checks before it will issue a challenge, so the page
    // can name the one thing that is missing instead of leaving the owner to
    // probe a paid `.md` URL and guess.
    let eligible: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts WHERE status = 'published' AND visibility IN ('paid','tiers')",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    ([("cache-control", "private, no-store")], Json(json!({
        "machine_payments": config,
        "methods": method_status(&state, &config),
        "deposit_addresses": {"base": config::address(&state, "base"), "tempo": config::address(&state, "tempo")},
        "stripe_connected": stripe.is_some(),
        "stripe_mode": stripe.as_ref().map(|stripe| if stripe.is_live() { "live" } else { "test" }),
        "is_private": state.settings.get_bool("is_private"),
        "llms_enabled": state.settings.get_bool("llms_enabled"),
        "eligible_posts": eligible,
    }))).into_response()
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
    // The lab flag tracks the switch in both directions: left behind on a
    // disable it would read as on in Settings → Labs while nothing is served.
    let mut labs = state
        .settings
        .get("labs")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if config.enabled {
        labs.insert("machinePayments".into(), json!(true));
    } else {
        labs.remove("machinePayments");
    }
    if let Err(e) = state
        .settings
        .set(&state.db, "labs", &Value::Object(labs))
        .await
    {
        return failure(e);
    }
    // Markdown delivery is what is being sold, so enabling implies the `.md`
    // representation is on. Disabling does not switch it back off: the LLM
    // feed is useful on its own and may have been on first.
    if config.enabled {
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
        // Keep whichever addresses are already stored, reading both shapes
        // `config::address` accepts — a row written before the map existed
        // holds a bare Tempo address as a string, and dropping it here would
        // orphan funds already pointed at it.
        let mut map = serde_json::Map::new();
        for other in ["base", "tempo"] {
            if let Some(existing) = config::address(&state, other) {
                map.insert(other.into(), json!(existing));
            }
        }
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
    // The article's title comes along because a bare 24-character post id is
    // not something an owner can read an activity list by. Left join: an
    // attempt outlives the post it paid for.
    let rows = sqlx::query_as::<_, (String, Option<String>, String, String, i64, String, String, Option<String>, String)>(
        "SELECT a.post_id, p.title, a.protocol, a.method, a.amount, a.currency, a.status, a.reference, a.created_at \
         FROM machine_payment_attempts a LEFT JOIN posts p ON p.id = a.post_id \
         ORDER BY a.created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    ([("cache-control", "private, no-store")], Json(json!({"payments": rows.into_iter().map(
        |(post_id, title, protocol, method, amount, currency, status, reference, created_at)| json!({
            "post_id": post_id, "title": title, "protocol": protocol, "method": method,
            "amount": amount, "currency": currency, "status": status,
            "reference": reference, "created_at": created_at,
        }),
    ).collect::<Vec<_>>()}))).into_response()
}
