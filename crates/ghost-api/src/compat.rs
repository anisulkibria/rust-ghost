//! Ghost-compatible operations shared by integrations and native admin screens.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Multipart, OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use ghost_core::{dates, ids};
use ghost_db::repos;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
pub fn router() -> Router<AppState> {
    let mut r = Router::new();
    for s in ["", "/"] {
        r = r
            .route(&format!("/users/{{id}}{s}"), delete(delete_staff))
            .route(&format!("/invites/{{id}}{s}"), get(read_invite))
            .route(
                &format!("/authentication/invitation{s}"),
                get(invitation).post(accept_invitation),
            )
            .route(
                &format!("/integrations/{{id}}/api_key/{{key}}/refresh{s}"),
                post(refresh_key),
            )
            .route(
                &format!("/settings/access_code/regenerate{s}"),
                post(access_code),
            )
            .route(&format!("/newsletters/verifications{s}"), put(verify_email))
            .route(&format!("/posts{s}"), delete(delete_posts))
            .route(&format!("/pages{s}"), delete(delete_posts))
            .route(&format!("/posts/upload{s}"), post(import_posts))
            .route(
                &format!("/members/upload{s}"),
                get(crate::admin_members::export).post(import_members),
            )
            .route(
                &format!("/members/{{id}}/subscriptions{s}"),
                post(create_subscription),
            )
            .route(
                &format!("/members/{{id}}/subscriptions/{{subscription}}{s}"),
                put(edit_subscription),
            );
    }
    r
}
fn bad(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
    )
        .into_response()
}
async fn access_code(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let value = ids::random_hex(12);
    match state
        .settings
        .set(&state.db, "password", &json!(&value))
        .await
    {
        Ok(()) => Json(json!({"settings":[{"key":"password","value":value}]})).into_response(),
        Err(e) => crate::error_json(&e),
    }
}
async fn refresh_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, key)): Path<(String, String)>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let secret = ids::random_hex(32);
    match sqlx::query("UPDATE api_keys SET secret=?,updated_at=? WHERE id=? AND integration_id=?")
        .bind(&secret)
        .bind(dates::now_db())
        .bind(&key)
        .bind(&id)
        .execute(&state.db)
        .await
    {
        Ok(row) if row.rows_affected() == 1 => {
            *state.frontend_key.write().await = None;
            match repos::integrations::find_by_id(&state.db,&id).await{
                Ok(Some(integration))=>match crate::admin::integration_json(&state,&integration).await{
                    Ok(integration)=>Json(json!({"integrations":[integration],"api_keys":[{"id":key,"secret":secret,"integration_id":id}]})).into_response(),Err(e)=>crate::error_json(&e)
                },_=>StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn delete_staff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match permissions::require(&state, &headers, Role::Administrator).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if headers.contains_key("authorization") || actor.id == id {
        return permissions::denied();
    }
    let Some(user) = repos::users::find_by_id(&state.db, &id)
        .await
        .ok()
        .flatten()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if repos::users::role_name(&state.db, &id)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("Owner")
    {
        return permissions::denied();
    }
    if user.status.as_deref() != Some("inactive")
        && repos::staff::admin_count(&state.db).await.unwrap_or(1) <= 1
        && repos::users::role_name(&state.db, &id)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("Administrator")
    {
        return bad("The last administrator cannot be removed");
    }
    match repos::staff::delete_staff(&state.db, &id).await {
        Ok(()) => {
            let _ = repos::actions::record(&state.db, "user", &id, &actor.id, "deleted").await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => crate::error_json(&e),
    }
}
async fn read_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::staff::list_invites(&state.db).await {Ok(rows)=>match rows.into_iter().find(|i|i.id==id){Some(i)=>Json(json!({"invites":[{"id":i.id,"email":i.email,"status":i.status,"expires":i.expires,"roles":[{"name":i.role}]}]})).into_response(),None=>StatusCode::NOT_FOUND.into_response()},Err(e)=>crate::error_json(&e)}
}
async fn invitation(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let token = q.get("token").map(String::as_str).unwrap_or("");
    match repos::staff::find_invite_by_token(&state.db, token)
        .await
        .ok()
        .flatten()
        .filter(|i| !i.is_expired())
    {
        Some(i) => Json(json!({"invitation":[{"valid":true,"email":i.email}]})).into_response(),
        None => bad("Invitation is invalid or expired"),
    }
}
async fn accept_invitation(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let data = body.pointer("/invitation/0").unwrap_or(&body);
    let Some(invite) =
        repos::staff::find_invite_by_token(&state.db, data["token"].as_str().unwrap_or(""))
            .await
            .ok()
            .flatten()
            .filter(|i| !i.is_expired())
    else {
        return bad("Invitation is invalid or expired");
    };
    let name = data["name"].as_str().unwrap_or("").trim();
    let password = data["password"].as_str().unwrap_or("");
    if name.is_empty() || name.len() > 191 || password.len() < 10 || password.len() > 72 {
        return bad("Name and a password of 10–72 bytes are required");
    }
    let password = password.to_owned();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(password, 12)).await {
        Ok(Ok(h)) => h,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match repos::users::insert_staff(&state.db, name, &invite.email, &hash, &invite.role).await {
        Ok(user) => {
            let _ = repos::staff::revoke_invite(&state.db, &invite.id).await;
            Json(json!({"invitation":[{"id":user.id,"email":user.email}]})).into_response()
        }
        Err(e) => crate::error_json(&e),
    }
}
pub async fn verify_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let token = body["token"]
        .as_str()
        .or_else(|| {
            body.pointer("/verifications/0/token")
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let row=sqlx::query_as::<_,(String,String,String,String)>("DELETE FROM email_verifications WHERE token_hash=? AND datetime(expires_at)>datetime('now') RETURNING resource_type,resource_id,field,value").bind(hash).fetch_optional(&mut *tx).await;
    let Ok(Some((kind, id, field, value))) = row else {
        return bad("Verification token is invalid or expired");
    };
    if !matches!(field.as_str(), "sender_email" | "sender_reply_to") {
        return bad("Invalid verified field");
    }
    let result = if kind == "newsletter" {
        sqlx::query(&format!("UPDATE newsletters SET {field}=? WHERE id=?"))
            .bind(value)
            .bind(id)
            .execute(&mut *tx)
            .await
    } else {
        sqlx::query("UPDATE settings SET value=? WHERE key=?")
            .bind(value)
            .bind(format!("automated_email_{field}"))
            .execute(&mut *tx)
            .await
    };
    if result.is_err() || tx.commit().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let _ = state.settings.reload(&state.db).await;
    Json(json!({"verifications":[{"verified":true}]})).into_response()
}
/// Request confirmation without changing the effective sender until the token is redeemed.
pub async fn request_email_verification(
    state: &AppState,
    kind: &str,
    id: &str,
    field: &str,
    email: &str,
) -> ghost_core::Result<()> {
    if !email.contains('@') || email.contains(['\r', '\n']) || email.len() > 191 {
        return Err(ghost_core::GhostError::BadRequest(
            "Invalid sender email".into(),
        ));
    }
    let token = ids::random_hex(32);
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("INSERT INTO email_verifications(token_hash,resource_type,resource_id,field,value,expires_at) VALUES(?,?,?,?,?,datetime('now','+1 day'))").bind(hash).bind(kind).bind(id).bind(field).bind(email).execute(&state.db).await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.site_name(),
        &state.site_url(),
    );
    ghost_mail::send(
        &cfg,
        &ghost_mail::Message {
            to: email.into(),
            subject: "Verify your publication sender address".into(),
            html: format!(
                "<p>Enter this verification token in publication settings:</p><code>{token}</code>"
            ),
            text: format!("Verification token: {token}"),
        },
    )
    .await
}
async fn delete_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match permissions::require(&state, &headers, Role::Editor).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(filter) = q.get("filter").filter(|s| !s.is_empty()) else {
        return bad("An explicit filter is required");
    };
    let kind = if uri.path().trim_end_matches('/').ends_with("/pages") {
        "page"
    } else {
        "post"
    };
    let query = repos::posts::BrowsePosts {
        filter: Some(match ghost_nql::parse(filter) {
            Ok(e) => e,
            Err(_) => return bad("Invalid post filter"),
        }),
        post_type: Some(kind.into()),
        limit: -1,
        public: false,
        ..Default::default()
    };
    let rows = match repos::posts::browse(&state.db, &query).await {
        Ok(v) => v.posts,
        Err(e) => return crate::error_json(&e),
    };
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for row in &rows {
        if sqlx::query("DELETE FROM posts WHERE id=?")
            .bind(&row.id)
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
    for row in &rows {
        let _ = repos::actions::record(&state.db, kind, &row.id, &actor.id, "deleted").await;
        crate::webhooks::fire_post_change(
            &state,
            &ghost_core::models::post_to_json(row, &[], &[], ""),
            None,
            "deleted",
        );
    }
    Json(json!({"bulk":{"meta":{"stats":{"successful":rows.len()}}}})).into_response()
}
async fn csv_bytes(mut multipart: Multipart) -> Result<Vec<u8>, Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| bad("Invalid upload"))?
    {
        if matches!(field.name(), Some("file" | "postsfile" | "membersfile")) {
            return field
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|_| bad("Could not read upload"));
        }
    }
    Err(bad("A CSV file is required"))
}
async fn import_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    let actor = match permissions::require(&state, &headers, Role::Administrator).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let labs = state.settings.get("labs").unwrap_or(Value::Null);
    if labs["csvContentImporter"] != true {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes = match csv_bytes(multipart).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    let mut posts = Vec::new();
    let mut tags = Vec::new();
    let mut users = Vec::new();
    let mut tag_ids = HashMap::<String, String>::new();
    let mut user_ids = HashMap::<String, String>::new();
    for record in reader.deserialize::<HashMap<String, String>>() {
        let record = match record {
            Ok(row) => row,
            Err(_) => return bad("Invalid CSV record"),
        };
        let mut value = json!(&record);
        if value["title"]
            .as_str()
            .is_none_or(|title| title.trim().is_empty())
        {
            return bad("Every post needs a title");
        }
        value["id"] = json!(ids::new_id());
        if value["status"].as_str().is_none_or(|s| s.is_empty()) {
            value["status"] = json!("draft")
        }
        if !matches!(
            value["status"].as_str(),
            Some("draft" | "published" | "scheduled" | "sent")
        ) {
            return bad("Invalid post status");
        }
        if value["type"].as_str().is_none_or(|s| s.is_empty()) {
            value["type"] = json!("post")
        }
        if !matches!(value["type"].as_str(), Some("post" | "page")) {
            return bad("Invalid content type");
        }
        for field in ["featured", "show_title_and_feature_image"] {
            if let Some(raw) = record.get(field).filter(|s| !s.is_empty()) {
                let boolean = match raw.to_lowercase().as_str() {
                    "true" | "1" => true,
                    "false" | "0" => false,
                    _ => return bad("Invalid boolean in CSV"),
                };
                value[field] = json!(boolean);
            } else {
                value.as_object_mut().unwrap().remove(field);
            }
        }
        if let Some(raw) = record.get("reading_time").filter(|s| !s.is_empty()) {
            let Ok(time) = raw.parse::<i64>() else {
                return bad("Invalid reading time");
            };
            value["reading_time"] = json!(time);
        }
        if value["visibility"].as_str().is_none_or(|s| s.is_empty()) {
            value["visibility"] = json!(record
                .get("post_access")
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("public"));
        }
        if !matches!(
            value["visibility"].as_str(),
            Some("public" | "members" | "paid" | "tiers")
        ) {
            return bad("Invalid post access");
        }
        let mut post_tags = Vec::new();
        for name in record
            .get("tags")
            .into_iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let id = tag_ids
                .entry(name.into())
                .or_insert_with(ids::new_id)
                .clone();
            if !tags.iter().any(|tag: &Value| tag["id"] == id) {
                tags.push(json!({"id":id,"name":name,"slug":ghost_core::slug::safe_string(name),"visibility":if name.starts_with('#'){"internal"}else{"public"}}));
            }
            post_tags.push(json!({"id":id}));
        }
        value["tags"] = json!(post_tags);
        let mut authors = Vec::new();
        for name in record
            .get("authors")
            .or_else(|| record.get("author"))
            .into_iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let existing = sqlx::query_scalar::<_, String>(
                "SELECT id FROM users WHERE name=? OR email=? OR slug=? LIMIT 1",
            )
            .bind(name)
            .bind(name)
            .bind(name)
            .fetch_optional(&state.db)
            .await;
            let id = match existing {
                Ok(Some(id)) => id,
                Ok(None) => {
                    let id = user_ids
                        .entry(name.into())
                        .or_insert_with(ids::new_id)
                        .clone();
                    if !users.iter().any(|u: &Value| u["id"] == id) {
                        users.push(
                            json!({"id":id,"name":name,"slug":ghost_core::slug::safe_string(name)}),
                        );
                    }
                    id
                }
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            authors.push(json!({"id":id}));
        }
        if authors.is_empty() {
            authors.push(json!({"id":actor.id}));
        }
        value["authors"] = json!(authors);
        let mut tiers = Vec::new();
        for name in record
            .get("tiers")
            .into_iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let tier = sqlx::query_scalar::<_, String>(
                "SELECT id FROM products WHERE type='paid' AND (id=? OR name=? OR slug=?) LIMIT 1",
            )
            .bind(name)
            .bind(name)
            .bind(name)
            .fetch_optional(&state.db)
            .await;
            match tier {
                Ok(Some(id)) => tiers.push(json!({"id":id})),
                _ => return bad("Unknown paid tier in CSV"),
            }
        }
        if value["visibility"] == "tiers" && tiers.is_empty() {
            return bad("Tier-restricted content needs paid tiers");
        }
        value["tiers"] = json!(tiers);
        posts.push(value);
    }
    let data = crate::publishing::prepare_content_import(
        &json!({"posts":posts,"tags":tags,"users":users}),
    );
    match ghost_db::transfer::import(&state.db, &data, &actor.id).await {
        Ok(()) => Json(json!({"posts":{"imported":posts.len()}})).into_response(),
        Err(e) => crate::error_json(&e),
    }
}
async fn import_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let bytes = match csv_bytes(multipart).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    let mut records = Vec::new();
    for row in reader.deserialize::<HashMap<String, String>>() {
        match row {
            Ok(row)
                if row
                    .get("email")
                    .is_some_and(|e| e.contains('@') && !e.contains(['\r', '\n', ' '])) =>
            {
                records.push(row)
            }
            _ => return bad("Every CSV row needs a valid email"),
        }
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for row in &records {
        let id = ids::new_id();
        let email = row["email"].to_lowercase();
        let saved=sqlx::query_scalar::<_,String>("INSERT INTO members(id,uuid,transient_id,email,name,note,status,created_at,updated_at) VALUES(?,?,?,?,?,?,'free',?,?) ON CONFLICT(email) DO UPDATE SET name=COALESCE(excluded.name,members.name),note=COALESCE(excluded.note,members.note) RETURNING id")
            .bind(&id).bind(ids::uuid_v4()).bind(ids::uuid_v4()).bind(email).bind(row.get("name")).bind(row.get("note")).bind(row.get("created_at").cloned().unwrap_or_else(dates::now_db)).bind(dates::now_db()).fetch_one(&mut *tx).await;
        let id = match saved {
            Ok(id) => id,
            Err(_) => return bad("Could not import member"),
        };
        for label in row
            .get("labels")
            .into_iter()
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let label_id=match sqlx::query_scalar::<_,String>("INSERT INTO labels(id,name,created_at) VALUES(?,?,?) ON CONFLICT(name) DO UPDATE SET name=excluded.name RETURNING id").bind(ids::new_id()).bind(label).bind(dates::now_db()).fetch_one(&mut *tx).await{Ok(id)=>id,Err(_)=>return bad("Could not import label")};
            if sqlx::query(
                "INSERT OR IGNORE INTO members_labels(id,member_id,label_id) VALUES(?,?,?)",
            )
            .bind(ids::new_id())
            .bind(&id)
            .bind(label_id)
            .execute(&mut *tx)
            .await
            .is_err()
            {
                return bad("Could not attach label");
            }
        }
        if let Some(pref) = row.get("subscribed_to_emails") {
            let subscribed = matches!(pref.as_str(), "true" | "1");
            if sqlx::query("DELETE FROM members_newsletters WHERE member_id=?")
                .bind(&id)
                .execute(&mut *tx)
                .await
                .is_err()
            {
                return bad("Could not update preferences");
            }
            if subscribed {
                let newsletters = match sqlx::query_scalar::<_, String>(
                    "SELECT id FROM newsletters WHERE status='active'",
                )
                .fetch_all(&mut *tx)
                .await
                {
                    Ok(v) => v,
                    Err(_) => return bad("Could not load newsletters"),
                };
                for nid in newsletters {
                    if sqlx::query(
                        "INSERT INTO members_newsletters(id,member_id,newsletter_id) VALUES(?,?,?)",
                    )
                    .bind(ids::new_id())
                    .bind(&id)
                    .bind(nid)
                    .execute(&mut *tx)
                    .await
                    .is_err()
                    {
                        return bad("Could not import newsletter preferences");
                    }
                }
            }
        }
        if let Some(customer) = row.get("stripe_customer_id").filter(|v| !v.is_empty()) {
            if !customer.starts_with("cus_")
                || !customer
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                return bad("Invalid Stripe customer id");
            }
            let other = sqlx::query_scalar::<_, String>(
                "SELECT member_id FROM members_stripe_customers WHERE customer_id=?",
            )
            .bind(customer)
            .fetch_optional(&mut *tx)
            .await;
            if !matches!(other, Ok(None)) && !matches!(&other,Ok(Some(owner))if *owner==id) {
                return bad("Stripe customer already belongs to another member");
            }
            if matches!(other,Ok(None))&&sqlx::query("INSERT INTO members_stripe_customers(id,member_id,customer_id,created_at,updated_at)VALUES(?,?,?,?,?)").bind(ids::new_id()).bind(&id).bind(customer).bind(dates::now_db()).bind(dates::now_db()).execute(&mut *tx).await.is_err(){return bad("Could not import customer reference")}
        }
        if row
            .get("complimentary_plan")
            .is_some_and(|v| matches!(v.as_str(), "true" | "1"))
        {
            let tiers = match sqlx::query_as::<_, (String, String)>(
                "SELECT id,name FROM products WHERE type='paid'",
            )
            .fetch_all(&mut *tx)
            .await
            {
                Ok(v) => v,
                Err(_) => return bad("Could not load complimentary tiers"),
            };
            let names = row
                .get("tiers")
                .map(|v| v.split(',').map(str::trim).collect::<Vec<_>>())
                .unwrap_or_default();
            for (tier, name) in tiers
                .into_iter()
                .filter(|(_, name)| names.is_empty() || names.contains(&name.as_str()))
            {
                let _ = name;
                if sqlx::query("INSERT INTO members_products(id,member_id,product_id) SELECT ?,?,? WHERE NOT EXISTS(SELECT 1 FROM members_products WHERE member_id=? AND product_id=?)").bind(ids::new_id()).bind(&id).bind(&tier).bind(&id).bind(&tier).execute(&mut *tx).await.is_err(){return bad("Could not import complimentary grant")}
            }
            if sqlx::query("UPDATE members SET status='comped' WHERE id=? AND status='free'")
                .bind(&id)
                .execute(&mut *tx)
                .await
                .is_err()
            {
                return bad("Could not update complimentary status");
            }
        }
    }
    if tx.commit().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(json!({"members":{"imported":records.len()}})).into_response()
}
async fn create_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
        return bad("Configure Stripe first");
    };
    let data = body.pointer("/subscriptions/0").unwrap_or(&body);
    let price = data["price_id"]
        .as_str()
        .or_else(|| data["stripe_price_id"].as_str())
        .unwrap_or("");
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE active=1 AND (monthly_price_id=? OR yearly_price_id=?))").bind(price).bind(price).fetch_one(&state.db).await.unwrap_or(false);
    if !valid {
        return bad("Choose an active tier price");
    }
    let customer = sqlx::query_scalar::<_, String>(
        "SELECT customer_id FROM members_stripe_customers WHERE member_id=? LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(customer) = customer else {
        return bad("Member has no Stripe customer");
    };
    match ghost_stripe::create_subscription(&cfg, &customer, price).await {
        Ok(sub) => match crate::members::handle_subscription_change(&state, &sub).await {
            Ok(()) => crate::admin_members::read(State(state), headers, Path(id)).await,
            Err(e) => crate::error_json(&e),
        },
        Err(e) => crate::error_json(&e),
    }
}
async fn edit_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, subscription)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let own:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions s JOIN members_stripe_customers c ON c.customer_id=s.customer_id WHERE c.member_id=? AND s.subscription_id=?)").bind(&id).bind(&subscription).fetch_one(&state.db).await.unwrap_or(false);
    if !own
        || !subscription
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
        return bad("Configure Stripe first");
    };
    let data = body.pointer("/subscriptions/0").unwrap_or(&body);
    let Some(cancel) = data["cancel_at_period_end"].as_bool() else {
        return bad("Choose cancel_at_period_end");
    };
    let result = if data["status"] == "canceled" {
        ghost_stripe::cancel_subscription(&cfg, &subscription).await
    } else {
        ghost_stripe::update_subscription(
            &cfg,
            &subscription,
            &[("cancel_at_period_end".into(), cancel.to_string())],
        )
        .await
    };
    match result {
        Ok(sub) => match crate::members::handle_subscription_change(&state, &sub).await {
            Ok(()) => crate::admin_members::read(State(state), headers, Path(id)).await,
            Err(e) => crate::error_json(&e),
        },
        Err(e) => crate::error_json(&e),
    }
}
