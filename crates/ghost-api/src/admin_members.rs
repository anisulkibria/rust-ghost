//! Members CRUD, saved-filter exports, bulk actions and member impersonation.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use ghost_core::{dates, ids};
use ghost_db::repos;
use serde_json::{json, Value};
use std::collections::HashMap;
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/members/", get(browse).post(add).delete(delete_filtered))
        .route("/members", get(browse).post(add).delete(delete_filtered))
        .route("/members/export/", get(export))
        .route("/members/bulk/", post(bulk).put(bulk_ghost))
        .route("/members/bulk", axum::routing::put(bulk_ghost))
        .route("/members/{id}/", get(read).put(edit).delete(delete))
        .route("/members/{id}", get(read).put(edit).delete(delete))
        .route("/members/{id}/signin_urls/", get(signin_url))
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
async fn authorize(state: &AppState, headers: &HeaderMap) -> std::result::Result<(), Response> {
    permissions::require(state, headers, Role::Administrator)
        .await
        .map(|_| ())
}
async fn value(state: &AppState, member: repos::members::Member) -> Value {
    let id = member.id.clone();
    let mut value = serde_json::to_value(member).unwrap();
    let labels=sqlx::query_as::<_,(String,String,String)>("SELECT l.id,l.name,l.slug FROM labels l JOIN members_labels ml ON ml.label_id=l.id WHERE ml.member_id=? ORDER BY l.name").bind(&id).fetch_all(&state.db).await.unwrap_or_default();
    let tiers=sqlx::query_as::<_,(String,String)>("SELECT p.id,p.name FROM products p JOIN members_products mp ON mp.product_id=p.id WHERE mp.member_id=? AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at)>datetime('now'))").bind(&id).fetch_all(&state.db).await.unwrap_or_default();
    let newsletters=sqlx::query_as::<_,(String,String)>("SELECT n.id,n.name FROM newsletters n JOIN members_newsletters mn ON mn.newsletter_id=n.id WHERE mn.member_id=?").bind(&id).fetch_all(&state.db).await.unwrap_or_default();
    value["labels"] = json!(labels
        .into_iter()
        .map(|(id, name, slug)| json!({"id":id,"name":name,"slug":slug}))
        .collect::<Vec<_>>());
    value["tiers"] = json!(tiers
        .into_iter()
        .map(|(id, name)| json!({"id":id,"name":name}))
        .collect::<Vec<_>>());
    value["newsletters"] = json!(newsletters
        .into_iter()
        .map(|(id, name)| json!({"id":id,"name":name}))
        .collect::<Vec<_>>());
    value["subscriptions"]=json!(crate::members::subscriptions_for(&state,&id).await);
    value
}
async fn browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let limit = q
        .get("limit")
        .map(|s| {
            if s == "all" {
                -1
            } else {
                s.parse::<i64>().unwrap_or(15).clamp(1, 1000)
            }
        })
        .unwrap_or(15);
    let page = q
        .get("page")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    match repos::members::browse(&state.db, q.get("filter").map(String::as_str), limit, page).await
    {
        Ok((rows, total)) => {
            let mut members = Vec::new();
            for row in rows {
                members.push(value(&state, row).await)
            }
            Json(json!({"members":members,"meta":{"pagination":ghost_core::models::pagination_json(page,if limit<0{total.max(1)}else{limit},total)}})).into_response()
        }
        Err(e) => failed(e),
    }
}
pub(crate) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    match repos::members::find(&state.db, &id).await {
        Ok(Some(row)) => Json(json!({"members":[value(&state,row).await]})).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => failed(e),
    }
}
fn input(body: &Value) -> &Value {
    body.pointer("/members/0").unwrap_or(body)
}
fn valid_email(email: &str) -> bool {
    email.len() <= 191
        && !email.contains(['\r', '\n', ' '])
        && email
            .split_once('@')
            .is_some_and(|(l, r)| !l.is_empty() && r.contains('.') && !r.starts_with('.'))
}
async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let data = input(&body);
    let email = data["email"].as_str().unwrap_or("").trim().to_lowercase();
    if !valid_email(&email) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Enter a valid email");
    }
    let id = ids::new_id();
    let now = dates::now_db();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save member"),
    };
    if sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,note,status,created_at,updated_at) VALUES (?,?,?,?,?,?,'free',?,?)")
        .bind(&id).bind(ids::uuid_v4()).bind(ids::uuid_v4()).bind(email).bind(data["name"].as_str()).bind(data["note"].as_str()).bind(&now).bind(&now).execute(&mut *tx).await.is_err(){return error(StatusCode::UNPROCESSABLE_ENTITY,"A member with this email already exists")}
    if let Err(e) = relations(&mut tx, &id, data).await {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e);
    }
    if tx.commit().await.is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save member");
    }
    // `admin` is Ghost's source for a member staff added by hand. There is
    // no page history to attribute it to, and inventing one would put a
    // staff action into the "which post brought them in" figures.
    let _ = crate::member_events::record_signup(&state, &id, "admin", &Default::default()).await;
    match repos::members::find(&state.db, &id).await {
        Ok(Some(row)) => (
            StatusCode::CREATED,
            Json(json!({"members":[value(&state,row).await]})),
        )
            .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn relations(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    data: &Value,
) -> std::result::Result<(), String> {
    for (key, table, column, parent) in [
        ("labels", "members_labels", "label_id", "labels"),
        (
            "newsletters",
            "members_newsletters",
            "newsletter_id",
            "newsletters",
        ),
    ] {
        if let Some(list) = data.get(key) {
            let Some(list) = list.as_array() else {
                return Err(format!("{key} must be an array"));
            };
            sqlx::query(&format!("DELETE FROM {table} WHERE member_id=?"))
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
            for item in list {
                let target = item
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .ok_or_else(|| format!("Invalid {key} id"))?;
                let found: bool = sqlx::query_scalar(&format!(
                    "SELECT EXISTS(SELECT 1 FROM {parent} WHERE id=?)"
                ))
                .bind(target)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
                if !found {
                    return Err(format!("Unknown {key} id"));
                }
                sqlx::query(&format!(
                    "INSERT OR IGNORE INTO {table} (id,member_id,{column}) VALUES (?,?,?)"
                ))
                .bind(ids::new_id())
                .bind(id)
                .bind(target)
                .execute(&mut **tx)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let Some(existing) = repos::members::find(&state.db, &id).await.ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let data = input(&body);
    let email = data["email"]
        .as_str()
        .unwrap_or(&existing.email)
        .trim()
        .to_lowercase();
    if !valid_email(&email) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Enter a valid email");
    }
    // Stripe is authoritative for paid status. Grants are managed through tiers.
    if data.get("status").is_some() && data["status"].as_str() != Some(existing.status.as_str()) {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Use billing or complimentary tier grants to change membership status",
        );
    }
    let name = if data.get("name").is_some() {
        data["name"].as_str()
    } else {
        existing.name.as_deref()
    };
    let note = if data.get("note").is_some() {
        data["note"].as_str()
    } else {
        existing.note.as_deref()
    };
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if sqlx::query("UPDATE members SET email=?,name=?,note=?,updated_at=?,transient_id=CASE WHEN email!=? THEN ? ELSE transient_id END WHERE id=?")
        .bind(&email).bind(name).bind(note).bind(dates::now_db()).bind(&email).bind(ids::uuid_v4()).bind(&id).execute(&mut *tx).await.is_err(){return error(StatusCode::UNPROCESSABLE_ENTITY,"Email is already in use")}
    if let Err(e) = relations(&mut tx, &id, data).await {
        return error(StatusCode::UNPROCESSABLE_ENTITY, &e);
    }
    if tx.commit().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // A staff member moving somebody's address is as much a part of their
    // history as the member doing it themselves.
    let _ = crate::member_events::record_email_change(&state, &id, &existing.email, &email).await;
    match repos::members::find(&state.db, &id).await {
        Ok(Some(row)) => {
            // Tier edits above can move a member between free, comped and
            // paid, and the status timeline should say so.
            if row.status != existing.status {
                let _ =
                    crate::member_events::record_status_change(&state, &id, Some(&existing.status), &row.status).await;
            }
            Json(json!({"members":[value(&state,row).await]})).into_response()
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    // Deleting a local member must never silently leave a paid Stripe subscription.
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM members_stripe_customers c JOIN subscriptions s ON s.customer_id=c.customer_id WHERE c.member_id=? AND s.status IN ('active','trialing','past_due','unpaid'))")
        .bind(&id).fetch_one(&state.db).await.unwrap_or(true);
    if active {
        return error(
            StatusCode::CONFLICT,
            "Cancel active billing subscriptions before deleting this member",
        );
    }
    match sqlx::query("DELETE FROM members WHERE id=?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn signin_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match permissions::require(&state, &headers, Role::Administrator).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    // Impersonation is a staff session action, never delegated to integration keys.
    if headers.contains_key(header::AUTHORIZATION) {
        return permissions::denied();
    }
    let Some(member) = repos::members::find(&state.db, &id).await.ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let token = ids::random_hex(32);
    let now = dates::now_db();
    let data =
        json!({"email":member.email,"action":"signin","impersonation":true,"actor_id":actor.id})
            .to_string();
    if sqlx::query("INSERT INTO tokens (id,token,data,created_at,updated_at) VALUES (?,?,?,?,?)")
        .bind(ids::new_id())
        .bind(&token)
        .bind(data)
        .bind(&now)
        .bind(&now)
        .execute(&state.db)
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let _ = repos::actions::record(&state.db, "member", &id, &actor.id, "impersonated").await;
    ([(header::CACHE_CONTROL,"private, no-store")],Json(json!({"member_signin_urls":[{"url":format!("{}/members/?token={token}&action=signin",state.site_url())}]}))).into_response()
}
pub(crate) async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows =
        match repos::members::browse(&state.db, q.get("filter").map(String::as_str), -1, 1).await {
            Ok((rows, _)) => rows,
            Err(e) => return failed(e),
        };
    let mut writer=csv::Writer::from_writer(Vec::new());
    if writer.write_record(["id","email","name","note","subscribed_to_emails","complimentary_plan","stripe_customer_id","created_at","deleted_at","labels","tiers"]).is_err(){return StatusCode::INTERNAL_SERVER_ERROR.into_response()}
    for row in rows {
        let labels:Vec<String>=sqlx::query_scalar("SELECT l.name FROM labels l JOIN members_labels ml ON ml.label_id=l.id WHERE ml.member_id=? ORDER BY l.name").bind(&row.id).fetch_all(&state.db).await.unwrap_or_default();
        let tiers:Vec<String>=sqlx::query_scalar("SELECT p.name FROM products p JOIN members_products mp ON mp.product_id=p.id WHERE mp.member_id=? AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at)>datetime('now')) ORDER BY p.name").bind(&row.id).fetch_all(&state.db).await.unwrap_or_default();
        let customer:Option<String>=sqlx::query_scalar("SELECT customer_id FROM members_stripe_customers WHERE member_id=? LIMIT 1").bind(&row.id).fetch_optional(&state.db).await.ok().flatten();
        let subscribed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM members_newsletters WHERE member_id=?)").bind(&row.id).fetch_one(&state.db).await.unwrap_or(false);
        if writer.write_record([row.id,row.email,row.name.unwrap_or_default(),row.note.unwrap_or_default(),subscribed.to_string(),(row.status=="comped").to_string(),customer.unwrap_or_default(),row.created_at,String::new(),labels.join(","),tiers.join(",")]).is_err(){return StatusCode::INTERNAL_SERVER_ERROR.into_response()}
    }
    let csv=match writer.into_inner(){Ok(bytes)=>bytes,Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response()};
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=members.csv",
            ),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        csv,
    )
        .into_response()
}
async fn bulk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let action = body["action"].as_str().unwrap_or("");
    if !matches!(action, "label" | "unlabel" | "unsubscribe" | "delete") {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid member action");
    }
    let selected = if let Some(list) = body["ids"].as_array() {
        list.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        let filter=body["filter"].as_str().filter(|s|!s.is_empty());
        if filter.is_none()&&body["all"]!=true{return error(StatusCode::UNPROCESSABLE_ENTITY,"Select members or provide a filter")}
        match repos::members::browse(&state.db,filter,-1,1).await{
            Ok((rows,_))=>rows.into_iter().map(|m|m.id).collect(),Err(e)=>return failed(e)
        }
    };
    if selected.is_empty() || selected.len() > 10000 {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Choose between 1 and 10000 members",
        );
    }
    let label = body["label_id"].as_str().unwrap_or("");
    if matches!(action, "label" | "unlabel") {
        let found: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM labels WHERE id=?)")
            .bind(label)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
        if !found {
            return error(StatusCode::UNPROCESSABLE_ENTITY, "Label not found");
        }
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for id in &selected {
        let result=match action{
            "label"=>sqlx::query("INSERT OR IGNORE INTO members_labels (id,member_id,label_id) SELECT ?,id,? FROM members WHERE id=?").bind(ids::new_id()).bind(label).bind(id).execute(&mut *tx).await,
            "unlabel"=>sqlx::query("DELETE FROM members_labels WHERE member_id=? AND label_id=?").bind(id).bind(label).execute(&mut *tx).await,
            "unsubscribe"=>sqlx::query("DELETE FROM members_newsletters WHERE member_id=?").bind(id).execute(&mut *tx).await,
            _=>{let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM members_stripe_customers c JOIN subscriptions s ON s.customer_id=c.customer_id WHERE c.member_id=? AND s.status IN ('active','trialing','past_due','unpaid'))").bind(id).fetch_one(&mut *tx).await.unwrap_or(true);
                if active{return error(StatusCode::CONFLICT,"Cancel active billing subscriptions before deleting members")}
                sqlx::query("DELETE FROM members WHERE id=?").bind(id).execute(&mut *tx).await}
        };
        if result.is_err() {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Bulk operation failed; no changes were saved",
            );
        }
    }
    if tx.commit().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(json!({"bulk":{"action":action,"successful":selected,"meta":{"stats":{"successful":selected.len(),"unsuccessful":0}}}})).into_response()
}

async fn bulk_ghost(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>,Json(body):Json<Value>)->Response {
    let source=body.get("bulk").unwrap_or(&body);
    let action=match source["action"].as_str().unwrap_or(""){"addLabel"=>"label","removeLabel"=>"unlabel",a=>a};
    let mut data=source.clone();data["action"]=json!(action);data["all"]=json!(q.get("all").is_some_and(|v|v=="true"));
    if data.get("filter").is_none(){data["filter"]=json!(q.get("filter"));}
    if data.get("label_id").is_none(){data["label_id"]=source.pointer("/meta/label/id").or_else(||source.pointer("/meta/label_id")).cloned().unwrap_or(Value::Null);}
    bulk(State(state),headers,Json(data)).await
}
async fn delete_filtered(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>)->Response {
    bulk(State(state),headers,Json(json!({"action":"delete","filter":q.get("filter"),"all":q.get("all").is_some_and(|v|v=="true")}))).await
}
