//! Portal subscription management. Authenticate the signed identity and check
//! local customer ownership before making any request to Stripe.
use super::*;
use base64::Engine;

async fn actor(state: &AppState, headers: &HeaderMap, body: &Value) -> Option<Member> {
    if let Some(token) = body.get("identity").and_then(Value::as_str) {
        let (payload, signature) = token.rsplit_once('.')?;
        if !secure_eq(&hmac_sha256_hex(&theme_secret(state), payload), signature) { return None; }
        let (header, claims) = payload.split_once('.')?;
        let decode = |part| -> Option<Value> {
            serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part).ok()?).ok()
        };
        let header = decode(header)?;
        let claims = decode(claims)?;
        if header["alg"] != "HS256" || claims["aud"] != state.site_url()
            || claims["exp"].as_i64()? <= chrono::Utc::now().timestamp() { return None; }
        return member_by_email(state, claims["sub"].as_str()?).await;
    }
    current_member(state, headers).await
}

async fn owned(state: &AppState, member: &Member, id: &str) -> bool {
    // Do not interpolate request-controlled Stripe ids into paths.
    if !id.starts_with("sub_") || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') { return false; }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions s JOIN members_stripe_customers c ON c.customer_id = s.customer_id WHERE s.subscription_id = ? AND c.member_id = ?)")
        .bind(id).bind(&member.id).fetch_one(&state.db).await.unwrap_or(false)
}

pub(super) async fn list(state: &AppState, member_id: &str) -> Vec<Value> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<bool>, Option<String>, Option<String>, Option<String>, Option<i64>, Option<String>)>(
        "SELECT s.subscription_id, s.status, s.cancel_at_period_end, s.current_period_end, s.plan_interval, s.plan_currency, s.plan_amount, s.stripe_price_id FROM subscriptions s JOIN members_stripe_customers c ON c.customer_id = s.customer_id WHERE c.member_id = ? ORDER BY s.created_at DESC")
        .bind(member_id).fetch_all(&state.db).await.unwrap_or_default();
    let mut subscriptions = Vec::new();
    for (id, status, cancelled, end, interval, currency, amount, price_id) in rows {
        let tier = sqlx::query_as::<_, (String, String)>("SELECT id, name FROM products WHERE monthly_price_id = ? OR yearly_price_id = ? LIMIT 1")
            .bind(&price_id).bind(&price_id).fetch_optional(&state.db).await.ok().flatten();
        subscriptions.push(json!({"id":id,"subscription_id":id,"status":status,
            "cancel_at_period_end":cancelled.unwrap_or(false),"current_period_end":end,
            "price":{"id":price_id,"amount":amount,"currency":currency,"interval":interval},
            "plan":{"id":price_id,"amount":amount,"currency":currency,"interval":interval},
            "tier":tier.map(|(id,name)| json!({"id":id,"name":name}))}));
    }
    subscriptions
}

pub(super) async fn update(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(member) = actor(&state, &headers, &body).await else { return api_error(StatusCode::UNAUTHORIZED,"You must be signed in.","UnauthorizedError"); };
    if !owned(&state, &member, &id).await { return api_error(StatusCode::NOT_FOUND,"Subscription not found.","NotFoundError"); }
    let reason = body.get("cancellation_reason").and_then(Value::as_str).unwrap_or("");
    let smart = body["smart_cancel"].as_bool().unwrap_or(false);
    let cancel = body.get("cancel_at_period_end").and_then(Value::as_bool);
    if reason.chars().count() > 500 || (!reason.is_empty() && cancel != Some(true) && !smart) {
        return api_error(StatusCode::BAD_REQUEST,"Invalid cancellation reason.","BadRequestError");
    }
    let Some(cfg) = stripe_cfg(&state) else { return stripe_unavailable().await; };
    let mut form = Vec::new();
    if body.get("tierId").is_some() || body.get("priceId").is_some() {
        let cadence = body["cadence"].as_str().unwrap_or("month");
        if !matches!(cadence,"month" | "year") { return api_error(StatusCode::BAD_REQUEST,"Invalid billing cadence.","BadRequestError"); }
        let yearly = cadence == "year";
        let price = if let Some(tier) = body["tierId"].as_str() {
            sqlx::query_scalar::<_, Option<String>>(if yearly { "SELECT yearly_price_id FROM products WHERE id = ? AND active = 1 AND type = 'paid'" } else { "SELECT monthly_price_id FROM products WHERE id = ? AND active = 1 AND type = 'paid'" })
                .bind(tier).fetch_optional(&state.db).await.ok().flatten().flatten()
        } else {
            let supplied = body["priceId"].as_str().unwrap_or("");
            sqlx::query_scalar::<_, String>("SELECT stripe_price_id FROM stripe_prices WHERE id = ? OR stripe_price_id = ?")
                .bind(supplied).bind(supplied).fetch_optional(&state.db).await.ok().flatten().or_else(|| supplied.starts_with("price_").then(||supplied.to_string()))
        };
        let Some(price) = price else { return api_error(StatusCode::NOT_FOUND,"Plan not found.","NotFoundError"); };
        let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE active = 1 AND type = 'paid' AND (monthly_price_id = ? OR yearly_price_id = ?))")
            .bind(&price).bind(&price).fetch_one(&state.db).await.unwrap_or(false);
        if !allowed { return api_error(StatusCode::BAD_REQUEST,"That plan is unavailable.","BadRequestError"); }
        let current = match ghost_stripe::get_subscription(&cfg, &id).await { Ok(v)=>v, Err(e)=>return api_error(StatusCode::BAD_GATEWAY,&e.message(),"StripeError") };
        let Some(item) = current.pointer("/items/data/0/id").and_then(Value::as_str) else { return api_error(StatusCode::BAD_GATEWAY,"Subscription item not found.","StripeError"); };
        form.push(("items[0][id]".into(), item.into()));
        form.push(("items[0][price]".into(), price));
        form.push(("cancel_at_period_end".into(), "false".into()));
        form.push(("proration_behavior".into(), "create_prorations".into()));
    } else if let Some(cancel) = cancel {
        form.push(("cancel_at_period_end".into(), cancel.to_string()));
    } else if smart {
        let current = match ghost_stripe::get_subscription(&cfg, &id).await { Ok(v)=>v, Err(e)=>return api_error(StatusCode::BAD_GATEWAY,&e.message(),"StripeError") };
        if matches!(current["status"].as_str(), Some("past_due"|"unpaid")) {
            return finish(&state, ghost_stripe::cancel_subscription(&cfg, &id).await).await;
        }
        form.push(("cancel_at_period_end".into(), "true".into()));
    } else { return api_error(StatusCode::BAD_REQUEST,"Choose a subscription change.","BadRequestError"); }
    if !reason.is_empty() { form.push(("metadata[cancellation_reason]".into(),reason.into())); }
    finish(&state, ghost_stripe::update_subscription(&cfg,&id,&form).await).await
}

async fn finish(state: &AppState, result: ghost_core::Result<Value>) -> Response {
    match result { Ok(subscription)=> match handle_subscription_change(state, &subscription).await { Ok(())=>no_content(), Err(_)=>api_error(StatusCode::BAD_GATEWAY,"Stripe updated the subscription, but local synchronization is pending. Please refresh your account shortly.","SynchronizationError") },
        Err(error)=>api_error(StatusCode::BAD_GATEWAY,&error.message(),"StripeError") }
}

pub(super) async fn apply_offer(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(member) = actor(&state,&headers,&body).await else { return api_error(StatusCode::UNAUTHORIZED,"You must be signed in.","UnauthorizedError"); };
    if !owned(&state,&member,&id).await { return api_error(StatusCode::NOT_FOUND,"Subscription not found.","NotFoundError"); }
    let offer_id = body["offer_id"].as_str().unwrap_or("");
    let Some(offer) = ghost_db::repos::offers::find_by_id(&state.db,offer_id).await.ok().flatten().filter(|o|o.active && o.redemption_type.as_deref() == Some("retention")) else {
        return api_error(StatusCode::NOT_FOUND,"Retention offer not found.","NotFoundError");
    };
    let applicable: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions s JOIN products p ON (s.stripe_price_id = p.monthly_price_id OR s.stripe_price_id = p.yearly_price_id) WHERE s.subscription_id = ? AND p.id = ? AND s.plan_interval = ? AND s.status IN ('active','trialing'))")
        .bind(&id).bind(&offer.product_id).bind(&offer.interval).fetch_one(&state.db).await.unwrap_or(false);
    if !applicable { return api_error(StatusCode::BAD_REQUEST,"This offer does not apply to your subscription.","BadRequestError"); }
    let Some(coupon) = offer.stripe_coupon_id else { return api_error(StatusCode::BAD_REQUEST,"This offer is unavailable.","BadRequestError"); };
    let Some(cfg) = stripe_cfg(&state) else { return stripe_unavailable().await; };
    finish(&state, ghost_stripe::update_subscription(&cfg,&id,&[("coupon".into(),coupon),("cancel_at_period_end".into(),"false".into()),("metadata[ghost_offer_id]".into(),offer.id)]).await).await
}
