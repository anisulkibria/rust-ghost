//! Ghost-compatible checkout design settings and real Stripe previews.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
const FONTS: &[&str] = &[
    "default",
    "be_vietnam_pro",
    "bitter",
    "chakra_petch",
    "hahmlet",
    "inconsolata",
    "inter",
    "lato",
    "lora",
    "m_plus_1_code",
    "montserrat",
    "noto_sans",
    "noto_sans_jp",
    "noto_serif",
    "nunito",
    "open_sans",
    "pridi",
    "pt_sans",
    "pt_serif",
    "raleway",
    "roboto",
    "roboto_slab",
    "source_sans_pro",
    "titillium_web",
    "ubuntu_mono",
    "zen_maru_gothic",
];
pub fn router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            .route(
                &format!("/stripe/checkout/config{suffix}"),
                get(read).put(edit),
            )
            .route(&format!("/stripe/checkout/branding{suffix}"), get(branding))
            .route(&format!("/stripe/checkout/preview{suffix}"), post(preview));
    }
    router
}
fn failure(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
    )
        .into_response()
}
fn validated(input: &Value) -> std::result::Result<Value, &'static str> {
    let design = input.get("design").ok_or("Design is required")?;
    if design["customize"] == false {
        return Ok(json!({"design":{"customize":false}}));
    }
    if design["customize"] != true {
        return Err("Choose whether to customize checkout");
    }
    for key in ["button_color", "background_color"] {
        let value = design[key].as_str().ok_or("Enter a checkout color")?;
        if value.len() != 7
            || !value.starts_with('#')
            || !value[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Use six-digit hex colors");
        }
    }
    if !matches!(
        design["border_style"].as_str(),
        Some("rounded" | "rectangular" | "pill")
    ) || !design["font_family"]
        .as_str()
        .is_some_and(|font| FONTS.contains(&font))
    {
        return Err("Choose a supported checkout font and corner style");
    }
    Ok(
        json!({"design":{"customize":true,"button_color":design["button_color"],"background_color":design["background_color"],"border_style":design["border_style"],"font_family":design["font_family"]}}),
    )
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
    Json(json!({"checkout_config":[state.settings.get("stripe_checkout_config").unwrap_or_else(||json!({"design":{"customize":false}}))],"fonts":FONTS})).into_response()
}
async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let user = match permissions::require(&state, &headers, Role::Administrator).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    let input = body.pointer("/checkout_config/0").unwrap_or(&body);
    let config = match validated(input) {
        Ok(value) => value,
        Err(e) => return failure(e),
    };
    if let Err(e) = state
        .settings
        .set(&state.db, "stripe_checkout_config", &config)
        .await
    {
        return failure(&e.message());
    }
    let _ = ghost_db::repos::actions::record(
        &state.db,
        "stripe_checkout_config",
        "site",
        &user.id,
        "edited",
    )
    .await;
    read(State(state), headers).await
}
async fn branding(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let Some(config) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
        return failure("Connect Stripe first");
    };
    match ghost_stripe::checkout_branding(&config,&state.site_url()).await{Ok(branding)=>Json(json!({"checkout_branding":[{"display_name":branding["display_name"],"design":branding}]})).into_response(),Err(e)=>failure(&e.message())}
}
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let input = body.pointer("/checkout_preview/0").unwrap_or(&body);
    let tier_id = input["tier_id"].as_str().unwrap_or("");
    let cadence = input["cadence"].as_str().unwrap_or("month");
    if !matches!(cadence, "month" | "year") {
        return failure("Choose a billing cadence");
    }
    let Some(mut config) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
        return failure("Connect Stripe first");
    };
    if input.get("design").is_some() {
        let value = match validated(input) {
            Ok(value) => value,
            Err(e) => return failure(e),
        };
        config.checkout_design =
            (value["design"]["customize"] == true).then(|| value["design"].clone());
    }
    let tier=sqlx::query_as::<_,(String,Option<String>,Option<i64>,Option<i64>)>("SELECT name,currency,monthly_price,yearly_price FROM products WHERE id=? AND active=1 AND type='paid'").bind(tier_id).fetch_optional(&state.db).await.ok().flatten();
    let Some((name, currency, monthly, yearly)) = tier else {
        return failure("Choose an active paid tier");
    };
    if let Err(e) = crate::stripe_tiers::sync_tier(
        &state,
        &config,
        tier_id,
        &name,
        currency.as_deref().unwrap_or("usd"),
        monthly,
        yearly,
    )
    .await
    {
        return failure(&e.message());
    }
    let column = if cadence == "month" {
        "monthly_price_id"
    } else {
        "yearly_price_id"
    };
    let price = sqlx::query_scalar::<_, Option<String>>(&format!(
        "SELECT {column} FROM products WHERE id=?"
    ))
    .bind(tier_id)
    .fetch_one(&state.db)
    .await
    .ok()
    .flatten();
    let Some(price) = price else {
        return failure("The selected plan has no price");
    };
    match ghost_stripe::checkout_preview(&config, &price, &state.site_url()).await {
        Ok(url) => Json(json!({"checkout_preview":[{"url":url}]})).into_response(),
        Err(e) => failure(&e.message()),
    }
}
