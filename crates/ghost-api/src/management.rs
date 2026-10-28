//! Administration resources used by Ghost integrations.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use ghost_core::{dates, ids};
use ghost_db::repos;
use serde::Serialize;
use serde_json::{json, Value};
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/roles/", get(roles))
        .route("/labels/", get(labels).post(add_label))
        .route(
            "/labels/{id}/",
            get(read_label).put(edit_label).delete(delete_label),
        )
        .route("/tiers/", get(tiers).post(add_tier))
        .route("/tiers/{id}/", get(read_tier).put(edit_tier))
        .route("/offers/", get(offers).post(add_offer))
        .route("/offers/{id}/", get(read_offer).put(edit_offer))
        .route("/invites/", get(invites).post(invite))
        .route("/invites/{id}/", axum::routing::delete(revoke_invite))
        .route("/users/owner/", axum::routing::put(transfer_owner))
        .route("/actions/", get(actions))
        .route("/stats/realtime/", get(realtime))
        .route("/stats/web/", get(web_stats))
}
fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
    )
        .into_response()
}
fn failed(e: ghost_core::GhostError) -> Response {
    error(StatusCode::UNPROCESSABLE_ENTITY, &e.message())
}
async fn admin(state: &AppState, headers: &HeaderMap) -> std::result::Result<(), Response> {
    permissions::require(state, headers, Role::Administrator)
        .await
        .map(|_| ())
}
async fn roles(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Contributor).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,String,Option<String>)>("SELECT id,name,description FROM roles WHERE name IN ('Owner','Administrator','Editor','Author','Contributor') ORDER BY name").fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"roles":rows.into_iter().map(|(id,name,description)|json!({"id":id,"name":name,"description":description})).collect::<Vec<_>>()})).into_response()
}
async fn label_values(state: &AppState, id: Option<&str>) -> Vec<Value> {
    sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT id,name,slug FROM labels WHERE (? IS NULL OR id=?) ORDER BY name",
    )
    .bind(id)
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, name, slug)| json!({"id":id,"name":name,"slug":slug}))
    .collect()
}
async fn labels(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    Json(json!({"labels":label_values(&state,None).await})).into_response()
}
async fn read_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let rows = label_values(&state, Some(&id)).await;
    if rows.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({"labels":rows})).into_response()
}
async fn write_label(
    state: AppState,
    headers: HeaderMap,
    id: Option<String>,
    body: Value,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let data = body.pointer("/labels/0").unwrap_or(&body);
    let name = data["name"].as_str().unwrap_or("").trim();
    if name.is_empty() || name.len() > 191 {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A label name is required");
    }
    let slug = ghost_core::slug::safe_string(data["slug"].as_str().unwrap_or(name));
    let new = id.is_none();
    let id = id.unwrap_or_else(ids::new_id);
    let now = dates::now_db();
    let result = if new {
        sqlx::query("INSERT INTO labels (id,name,slug,created_at,updated_at) VALUES (?,?,?,?,?)")
            .bind(&id)
            .bind(name)
            .bind(slug)
            .bind(&now)
            .bind(&now)
            .execute(&state.db)
            .await
    } else {
        sqlx::query("UPDATE labels SET name=?,slug=?,updated_at=? WHERE id=?")
            .bind(name)
            .bind(slug)
            .bind(now)
            .bind(&id)
            .execute(&state.db)
            .await
    };
    match result {
        Ok(r) if r.rows_affected() > 0 => (
            if new {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({"labels":label_values(&state,Some(&id)).await})),
        )
            .into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "A label with this name already exists",
        ),
    }
}
async fn add_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    write_label(state, headers, None, body).await
}
async fn edit_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    write_label(state, headers, Some(id), body).await
}
async fn delete_label(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    match sqlx::query("DELETE FROM labels WHERE id=?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
#[derive(Serialize, sqlx::FromRow)]
struct Tier {
    id: String,
    name: String,
    slug: String,
    active: bool,
    visibility: String,
    description: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    currency: Option<String>,
    monthly_price: Option<i64>,
    yearly_price: Option<i64>,
    monthly_price_id: Option<String>,
    yearly_price_id: Option<String>,
    trial_days: i64,
    welcome_page_url: Option<String>,
}
async fn tier_values(state: &AppState, id: Option<&str>) -> Vec<Value> {
    sqlx::query_as::<_,Tier>("SELECT id,name,slug,active,visibility,description,type AS kind,currency,monthly_price,yearly_price,monthly_price_id,yearly_price_id,trial_days,welcome_page_url FROM products WHERE (? IS NULL OR id=?) ORDER BY created_at")
    .bind(id).bind(id).fetch_all(&state.db).await.unwrap_or_default().into_iter().map(|t|{let active=t.active;let mut v=serde_json::to_value(t).unwrap();v["status"]=json!(if active{"active"}else{"archived"});v}).collect()
}
async fn tiers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    Json(json!({"tiers":tier_values(&state,None).await})).into_response()
}
async fn read_tier(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let values = tier_values(&state, Some(&id)).await;
    if values.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({"tiers":values})).into_response()
}
async fn write_tier(
    state: AppState,
    headers: HeaderMap,
    id: Option<String>,
    body: Value,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let new = id.is_none();
    let id = id.unwrap_or_else(ids::new_id);
    let old = if new {
        json!({"type":"paid","currency":"usd","status":"active","visibility":"public","trial_days":0})
    } else {
        let Some(value) = tier_values(&state, Some(&id)).await.into_iter().next() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        value
    };
    let mut data = old;
    let input = body.pointer("/tiers/0").unwrap_or(&body);
    let Some(input) = input.as_object() else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Tier data is required");
    };
    for (key, value) in input {
        data[key] = value.clone()
    }
    let name = data["name"].as_str().unwrap_or("").trim();
    let kind = data["type"].as_str().unwrap_or("paid");
    let visibility = data["visibility"].as_str().unwrap_or("public");
    let status = data["status"].as_str().unwrap_or("active");
    if name.is_empty()
        || name.len() > 191
        || !matches!(kind, "free" | "paid")
        || !matches!(visibility, "public" | "none")
        || !matches!(status, "active" | "archived")
    {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid tier");
    }
    let monthly = data["monthly_price"].as_i64();
    let yearly = data["yearly_price"].as_i64();
    let trial = data["trial_days"].as_i64().unwrap_or(0);
    let currency = data["currency"].as_str().unwrap_or("usd").to_lowercase();
    if monthly.is_some_and(|n| n < 0)
        || yearly.is_some_and(|n| n < 0)
        || !(0..=365).contains(&trial)
        || currency.len() != 3
        || !currency.bytes().all(|c| c.is_ascii_alphabetic())
    {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid tier pricing");
    }
    let slug = ghost_core::slug::safe_string(data["slug"].as_str().unwrap_or(name));
    let now = dates::now_db();
    if new {
        if sqlx::query("INSERT INTO products (id,name,slug,type,created_at) VALUES (?,?,?,?,?)")
            .bind(&id)
            .bind(name)
            .bind(&slug)
            .bind(kind)
            .bind(&now)
            .execute(&state.db)
            .await
            .is_err()
        {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "A tier with this slug already exists",
            );
        }
    }
    if sqlx::query("UPDATE products SET name=?,slug=?,type=?,active=?,visibility=?,description=?,currency=?,monthly_price=?,yearly_price=?,trial_days=?,welcome_page_url=?,updated_at=? WHERE id=?")
        .bind(name).bind(slug).bind(kind).bind(status=="active").bind(visibility).bind(data["description"].as_str()).bind(&currency).bind(monthly).bind(yearly).bind(trial).bind(data["welcome_page_url"].as_str()).bind(now).bind(&id).execute(&state.db).await.is_err(){return error(StatusCode::UNPROCESSABLE_ENTITY,"Could not save tier")}
    if kind == "paid" {
        if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
            if let Err(e) =
                crate::stripe_tiers::sync_tier(&state, &cfg, &id, name, &currency, monthly, yearly)
                    .await
            {
                return failed(e);
            }
        }
    }
    (
        if new {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({"tiers":tier_values(&state,Some(&id)).await})),
    )
        .into_response()
}
async fn add_tier(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    write_tier(state, headers, None, body).await
}
async fn edit_tier(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    write_tier(state, headers, Some(id), body).await
}
fn offer_value(o: repos::offers::Offer) -> Value {
    json!({"id":o.id,"name":o.name,"code":o.code,"status":o.status(),"tier":{"id":o.product_id},"cadence":o.interval,"type":o.discount_type,"amount":o.discount_amount,"duration":o.duration,"duration_in_months":o.duration_in_months,"currency":o.currency,"display_title":o.portal_title,"display_description":o.portal_description,"redemption_type":o.redemption_type})
}
async fn offers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    match repos::offers::list(&state.db).await {
        Ok(rows) => Json(json!({"offers":rows.into_iter().map(offer_value).collect::<Vec<_>>()}))
            .into_response(),
        Err(e) => failed(e),
    }
}
async fn read_offer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    match repos::offers::find_by_id(&state.db, &id).await {
        Ok(Some(row)) => Json(json!({"offers":[offer_value(row)]})).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => failed(e),
    }
}
async fn write_offer(
    state: AppState,
    headers: HeaderMap,
    id: Option<String>,
    body: Value,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let mut data = if let Some(id) = &id {
        match repos::offers::find_by_id(&state.db, id).await {
            Ok(Some(o)) => offer_value(o),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        json!({"status":"active","cadence":"month","type":"percent","duration":"once"})
    };
    let input = body.pointer("/offers/0").unwrap_or(&body);
    let Some(input) = input.as_object() else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Offer data is required");
    };
    for (key, value) in input {
        data[key] = value.clone()
    }
    let s = |key: &str| data[key].as_str().unwrap_or("").to_string();
    let name = s("name");
    let code = s("code");
    let tier = data
        .pointer("/tier/id")
        .and_then(Value::as_str)
        .or_else(|| data["tier_id"].as_str())
        .unwrap_or("");
    let cadence = s("cadence");
    let kind = s("type");
    let duration = s("duration");
    let amount = data["amount"].as_i64().unwrap_or(0);
    if name.trim().is_empty()
        || code.trim().is_empty()
        || !matches!(cadence.as_str(), "month" | "year")
        || !matches!(kind.as_str(), "percent" | "fixed" | "trial")
        || amount <= 0
        || (kind == "percent" && amount > 100)
        || !matches!(
            duration.as_str(),
            "once" | "forever" | "repeating" | "trial"
        )
    {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid offer");
    }
    let tiers = tier_values(&state, Some(tier)).await;
    if tiers.first().is_none_or(|t| t["type"] != "paid") {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Choose a paid tier");
    }
    let currency = tiers[0]["currency"].as_str().unwrap_or("usd").to_string();
    let months = data["duration_in_months"].as_i64();
    if duration == "repeating" && months.is_none_or(|n| n <= 0) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Choose an offer duration");
    }
    let coupon = if kind != "trial" {
        if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
            let discount = if kind == "percent" {
                ghost_stripe::Discount::Percent(amount)
            } else {
                ghost_stripe::Discount::Fixed {
                    amount_cents: amount,
                    currency: &currency,
                }
            };
            match ghost_stripe::create_coupon(&cfg, &name, discount, &duration, months).await {
                Ok(id) => Some(id),
                Err(e) => return failed(e),
            }
        } else {
            None
        }
    } else {
        None
    };
    let input = repos::offers::OfferInput {
        name,
        code,
        product_id: tier.into(),
        interval: cadence,
        discount_type: kind,
        discount_amount: amount,
        duration,
        duration_in_months: months,
        currency: Some(currency),
        portal_title: s("display_title"),
        portal_description: s("display_description"),
        stripe_coupon_id: coupon,
        active: s("status") != "archived",
    };
    let new = id.is_none();
    let result = match id {
        Some(id) => repos::offers::update(&state.db, &id, &input).await,
        None => repos::offers::create(&state.db, &input).await,
    };
    match result {
        Ok(offer) => (
            if new {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({"offers":[offer_value(offer)]})),
        )
            .into_response(),
        Err(e) => failed(e),
    }
}
async fn add_offer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    write_offer(state, headers, None, body).await
}
async fn edit_offer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    write_offer(state, headers, Some(id), body).await
}
async fn invites(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    match repos::staff::list_invites(&state.db).await{Ok(rows)=>Json(json!({"invites":rows.into_iter().map(|i|json!({"id":i.id,"email":i.email,"status":i.status,"expires":i.expires,"roles":[{"name":i.role}]})).collect::<Vec<_>>()})).into_response(),Err(e)=>failed(e)}
}
async fn invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let data = body.pointer("/invites/0").unwrap_or(&body);
    let email = data["email"].as_str().unwrap_or("");
    if !email.contains('@') || email.contains(['\r', '\n']) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Enter a staff email");
    }
    let role = data
        .pointer("/roles/0/name")
        .and_then(Value::as_str)
        .unwrap_or("Author");
    let invite = match repos::staff::create_invite(&state.db, email, role).await {
        Ok(i) => i,
        Err(e) => return failed(e),
    };
    let link = format!("{}/ghost/invite/{}/", state.site_url(), invite.token);
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.site_name(),
        &state.site_url(),
    );
    let message=ghost_mail::Message{to:email.into(),subject:format!("You're invited to {}",state.settings.site_name()),html:format!("<p>You have been invited to join the publication.</p><p><a href=\"{link}\">Accept invitation</a></p>"),text:format!("Accept invitation: {link}")};
    match ghost_mail::send(&cfg, &message).await {
        Ok(()) => {
            let _ = repos::staff::mark_invite_sent(&state.db, &invite.id).await;
            (
                StatusCode::CREATED,
                Json(json!({"invites":[{"id":invite.id,"email":invite.email,"status":"sent"}]})),
            )
                .into_response()
        }
        Err(e) => failed(e),
    }
}
async fn revoke_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    match repos::staff::revoke_invite(&state.db, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failed(e),
    }
}
async fn transfer_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let owner = match permissions::require(&state, &headers, Role::Owner).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if headers.contains_key(header::AUTHORIZATION) {
        return permissions::denied();
    }
    let data = body.pointer("/owner/0").unwrap_or(&body);
    let target = data["id"].as_str().unwrap_or("");
    let password = data["password"].as_str().unwrap_or("");
    if !bcrypt::verify(password, owner.password.as_deref().unwrap_or("")).unwrap_or(false) {
        return permissions::denied();
    }
    let Some(target) = repos::users::find_by_id(&state.db, target)
        .await
        .ok()
        .flatten()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if permissions::role(&state, &target).await.ok() != Some(Role::Administrator) {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The new owner must be an active administrator",
        );
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for (id, role) in [(&owner.id, "Administrator"), (&target.id, "Owner")] {
        if sqlx::query(
            "UPDATE roles_users SET role_id=(SELECT id FROM roles WHERE name=?) WHERE user_id=?",
        )
        .bind(role)
        .bind(id)
        .execute(&mut *tx)
        .await
        .is_err()
        {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if tx.commit().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let _ = repos::actions::record(
        &state.db,
        "user",
        &target.id,
        &owner.id,
        "ownership_transferred",
    )
    .await;
    Json(json!({"owner":[{"id":target.id,"email":target.email}]})).into_response()
}
async fn actions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,Option<String>,String,String,String,String)>("SELECT id,resource_id,resource_type,actor_id,event,created_at FROM actions ORDER BY created_at DESC LIMIT 100").fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"actions":rows.into_iter().map(|(id,resource_id,resource_type,actor_id,event,created_at)|json!({"id":id,"resource_id":resource_id,"resource_type":resource_type,"actor_id":actor_id,"event":event,"created_at":created_at})).collect::<Vec<_>>()})).into_response()
}
async fn realtime(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let active:i64=sqlx::query_scalar("SELECT COUNT(DISTINCT viewer) FROM page_views WHERE datetime(created_at)>=datetime('now','-5 minutes')").fetch_one(&state.db).await.unwrap_or(0);
    (
        [("cache-control", "private, no-store")],
        Json(json!({"stats":[{"active_visitors":active,"window_seconds":300}]})),
    )
        .into_response()
}
async fn web_stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = admin(&state, &headers).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,i64,i64)>("SELECT substr(created_at,1,10),COUNT(*),COUNT(DISTINCT viewer) FROM page_views GROUP BY substr(created_at,1,10) ORDER BY substr(created_at,1,10) DESC LIMIT 90").fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"stats":rows.into_iter().map(|(date,views,visitors)|json!({"date":date,"views":views,"visitors":visitors})).collect::<Vec<_>>()})).into_response()
}
