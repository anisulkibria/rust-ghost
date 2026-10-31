use super::*;
use crate::permissions::{self, Role};
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/social/settings/", get(settings).put(configure))
        .route("/social/reader/", get(reader))
        .route("/social/notes/", get(notes).post(add_note))
        .route("/social/notes/{id}/", axum::routing::delete(delete_note))
        .route("/social/following/", get(following_list))
        .route("/social/follow/", post(follow))
        .route("/social/interactions/", post(interact))
        .route(
            "/social/notifications/",
            get(notifications).put(read_notifications),
        )
        .route("/social/blocks/", get(blocks).post(block))
}
async fn authorize(state: &AppState, headers: &HeaderMap) -> std::result::Result<(), Response> {
    permissions::require(state, headers, Role::Editor)
        .await
        .map(|_| ())
}
async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let followers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM federation_followers")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let following: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM federation_following WHERE accepted=1")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
    let failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM federation_deliveries WHERE attempts>=12 AND delivered_at IS NULL",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    let site = reqwest::Url::parse(&state.site_url()).ok();
    let host = site.as_ref().and_then(|url| url.host_str()).unwrap_or("");
    Json(json!({"social":{"enabled":enabled(&state),"actor":actor_id(&state),"handle":format!("@publication@{host}"),"followers":followers,"following":following,"failed_deliveries":failed,"requires_public_https":site.is_none_or(|url|url.scheme()!="https")}})).into_response()
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(enable) = body["enabled"].as_bool() else {
        return failure(bad("Enabled must be a boolean"));
    };
    if enable && state.settings.get_bool("is_private") {
        return failure(bad("A private publication cannot enable public federation"));
    }
    if enable {
        if let Err(e) = crypto::keys(&state).await {
            return failure(e);
        }
    }
    if let Err(e) = state
        .settings
        .set(&state.db, "social_web_enabled", &json!(enable))
        .await
    {
        return failure(e);
    }
    settings(State(state), headers).await
}
async fn reader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    list_objects(state, headers, q, false).await
}
async fn notes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    list_objects(state, headers, q, true).await
}
async fn list_objects(
    state: AppState,
    headers: HeaderMap,
    q: HashMap<String, String>,
    local: bool,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let page = q
        .get("page")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1)
        .clamp(1, 100000);
    let rows=sqlx::query_as::<_,(String,String,String,String,Option<String>,String,bool)>("SELECT o.id,o.actor,o.content,o.url,o.in_reply_to,o.published_at,o.local FROM federation_objects o WHERE o.deleted=0 AND ((?=1 AND o.local=1) OR (?=0 AND (o.local=1 OR EXISTS(SELECT 1 FROM federation_following f WHERE f.actor=o.actor AND f.accepted=1)))) ORDER BY o.published_at DESC,o.id DESC LIMIT 30 OFFSET ?")
        .bind(local).bind(local).bind((page-1)*30).fetch_all(&state.db).await.unwrap_or_default();
    let items=rows.into_iter().map(|(id,actor,content,url,reply,published,local)|json!({"id":id,"actor":actor,"text":ghost_core::comments::to_plaintext(&content),"url":url,"in_reply_to":reply,"published_at":published,"local":local})).collect::<Vec<_>>();
    (
        [("cache-control", "private, no-store")],
        Json(json!({"notes":items,"page":page})),
    )
        .into_response()
}
async fn add_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    match ops::publish_note(
        &state,
        body["text"].as_str().unwrap_or(""),
        body["in_reply_to"].as_str(),
        body["reply_actor"].as_str(),
    )
    .await
    {
        Ok(id) => {
            let url = id.replace("/objects/", "/notes/");
            (StatusCode::CREATED, Json(json!({"notes":[{"id":id,"url":url}]}))).into_response()
        }
        Err(e) => failure(e),
    }
}
async fn delete_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    match ops::delete_note(&state, &id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn following_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (String, bool)>(
        "SELECT actor,accepted FROM federation_following ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    Json(json!({"following":rows.into_iter().map(|(actor,accepted)|json!({"actor":actor,"accepted":accepted})).collect::<Vec<_>>()})).into_response()
}
async fn follow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let handle = body["actor"].as_str().unwrap_or("");
    let result = if body["unfollow"].as_bool().unwrap_or(false) {
        ops::unfollow(&state, handle).await.map(|()| StatusCode::NO_CONTENT)
    } else {
        ops::follow(&state, handle).await.map(|_| StatusCode::CREATED)
    };
    match result {
        Ok(status) => status.into_response(),
        Err(e) => failure(e),
    }
}
async fn interact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    match ops::interact(
        &state,
        body["type"].as_str().unwrap_or(""),
        body["actor"].as_str().unwrap_or(""),
        body["object"].as_str().unwrap_or(""),
    )
    .await
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({"activities":[{"id":id}]}))).into_response(),
        Err(e) => failure(e),
    }
}
async fn notifications(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,String,String,String,Option<String>,String)>("SELECT id,actor,activity_type,body,read_at,created_at FROM federation_received WHERE undone=0 AND activity_type IN ('Follow','Like','Announce','Create','Accept') ORDER BY created_at DESC LIMIT 100").fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"notifications":rows.into_iter().map(|(id,actor,kind,body,read_at,created_at)|{let body=serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);json!({"id":id,"actor":actor,"type":kind,"text":ghost_core::comments::to_plaintext(body.pointer("/object/content").and_then(Value::as_str).unwrap_or("")),"read_at":read_at,"created_at":created_at})}).collect::<Vec<_>>()})).into_response()
}
async fn read_notifications(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    match ops::mark_read(&state).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
async fn blocks(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT actor FROM federation_blocks ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    Json(json!({"blocks":rows})).into_response()
}
async fn block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let blocked = body["unblock"].as_bool() != Some(true);
    match ops::set_block(&state, body["actor"].as_str().unwrap_or(""), blocked).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
