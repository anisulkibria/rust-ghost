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
    if !enabled(&state) {
        return failure(bad("Enable the social web first"));
    }
    let markdown = body["text"].as_str().unwrap_or("").trim();
    if markdown.is_empty() || markdown.chars().count() > 10000 {
        return failure(bad("Write a note of 1–10000 characters"));
    }
    let reply = body["in_reply_to"].as_str();
    if reply.is_some_and(|url| {
        url.len() > 2000 || !reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https")
    }) {
        return failure(bad("Invalid reply target"));
    }
    let content = ghost_render::markdown::to_html(markdown);
    let uuid = ids::uuid_v4();
    let id = endpoint(&state, &format!("objects/{uuid}"));
    let url = endpoint(&state, &format!("notes/{uuid}"));
    let now = dates::now_db();
    let mut inboxes = follower_inboxes(&state).await;
    let mut mentions = Vec::new();
    if let Some(actor) = body["reply_actor"].as_str() {
        match network::actor(actor).await {
            Ok(actor) => {
                inboxes.push(actor.inbox);
                mentions.push(actor.id)
            }
            Err(e) => return failure(e),
        }
    }
    let object = json!({"id":id,"type":"Note","attributedTo":actor_id(&state),"content":content,"inReplyTo":reply,"url":url,"published":chrono::Utc::now().to_rfc3339(),"to":[PUBLIC],"cc":mentions});
    let mut activity = activity_body(&state, "Create", object);
    activity["cc"] = json!(mentions);
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return failure(db_error(e)),
    };
    if let Err(e)=sqlx::query("INSERT INTO federation_objects (id,actor,object_type,content,url,in_reply_to,published_at,updated_at,local) VALUES (?,?,'Note',?,?,?,?,?,1)").bind(&id).bind(actor_id(&state)).bind(content).bind(&url).bind(reply).bind(&now).bind(&now).execute(&mut *tx).await{return failure(db_error(e))}
    if let Err(e) = enqueue_tx(&mut tx, &activity, &inboxes).await {
        return failure(e);
    }
    if let Err(e) = tx.commit().await {
        return failure(db_error(e));
    }
    (
        StatusCode::CREATED,
        Json(json!({"notes":[{"id":id,"url":url}]})),
    )
        .into_response()
}
async fn delete_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let object = endpoint(&state, &format!("objects/{id}"));
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return failure(db_error(e)),
    };
    let changed=sqlx::query("UPDATE federation_objects SET deleted=1,content='',updated_at=? WHERE id=? AND local=1 AND deleted=0").bind(dates::now_db()).bind(&object).execute(&mut *tx).await;
    match changed {
        Ok(changed) if changed.rows_affected() == 1 => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return failure(db_error(e)),
    }
    let activity = activity_body(&state, "Delete", json!({"id":object,"type":"Tombstone"}));
    // Read recipients on this transaction so single-connection installations work too.
    let inboxes =
        sqlx::query_scalar::<_, String>("SELECT DISTINCT inbox FROM federation_followers")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();
    if let Err(e) = enqueue_tx(&mut tx, &activity, &inboxes).await {
        return failure(e);
    }
    if let Err(e) = tx.commit().await {
        return failure(db_error(e));
    }
    StatusCode::NO_CONTENT.into_response()
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
    if !enabled(&state) {
        return failure(bad("Enable the social web first"));
    }
    let handle = body["actor"].as_str().unwrap_or("");
    if handle.len() > 2000 {
        return failure(bad("Invalid actor"));
    }
    let unfollow = body["unfollow"].as_bool().unwrap_or(false);
    if unfollow {
        let existing = sqlx::query_as::<_, (String, String)>(
            "SELECT inbox,follow_id FROM federation_following WHERE actor=?",
        )
        .bind(handle)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        let Some((inbox, id)) = existing else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mut activity = activity_body(
            &state,
            "Undo",
            json!({"id":id,"type":"Follow","actor":actor_id(&state),"object":handle}),
        );
        activity["to"] = json!([handle]);
        activity["cc"] = json!([]);
        let mut tx = match state.db.begin().await {
            Ok(tx) => tx,
            Err(e) => return failure(db_error(e)),
        };
        if let Err(e) = enqueue_tx(&mut tx, &activity, &[inbox]).await {
            return failure(e);
        }
        if let Err(e) = sqlx::query("DELETE FROM federation_following WHERE actor=?")
            .bind(handle)
            .execute(&mut *tx)
            .await
        {
            return failure(db_error(e));
        }
        if let Err(e) = tx.commit().await {
            return failure(db_error(e));
        }
        return StatusCode::NO_CONTENT.into_response();
    }
    let actor = match network::discover(handle).await {
        Ok(actor) => actor,
        Err(e) => return failure(e),
    };
    if actor.id == actor_id(&state) {
        return failure(bad("Cannot follow the publication itself"));
    }
    let blocked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_blocks WHERE actor=?)")
            .bind(&actor.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(true);
    if blocked {
        return failure(bad("Unblock this account before following it"));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_following WHERE actor=?)")
            .bind(&actor.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
    if exists {
        return StatusCode::NO_CONTENT.into_response();
    }
    let mut activity = activity_body(&state, "Follow", json!(actor.id));
    activity["to"] = json!([actor.id]);
    activity["cc"] = json!([]);
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return failure(db_error(e)),
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO federation_following (actor,inbox,follow_id,created_at) VALUES (?,?,?,?)",
    )
    .bind(&actor.id)
    .bind(&actor.inbox)
    .bind(activity["id"].as_str().unwrap())
    .bind(dates::now_db())
    .execute(&mut *tx)
    .await
    {
        return failure(db_error(e));
    }
    if let Err(e) = enqueue_tx(&mut tx, &activity, &[actor.inbox]).await {
        return failure(e);
    }
    if let Err(e) = tx.commit().await {
        return failure(db_error(e));
    }
    StatusCode::CREATED.into_response()
}
async fn interact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    if !enabled(&state) {
        return failure(bad("Enable the social web first"));
    }
    let kind = body["type"].as_str().unwrap_or("");
    if !matches!(kind, "Like" | "Announce" | "Undo") {
        return failure(bad("Choose Like, Announce or Undo"));
    }
    let target = body["object"].as_str().unwrap_or("");
    if target.len() > 2000 || !reqwest::Url::parse(target).is_ok_and(|url| url.scheme() == "https")
    {
        return failure(bad("Invalid interaction target"));
    }
    let actor = match network::actor(body["actor"].as_str().unwrap_or("")).await {
        Ok(actor) => actor,
        Err(e) => return failure(e),
    };
    let object = if kind == "Undo" {
        let original=sqlx::query_scalar::<_,String>("SELECT body FROM federation_activities WHERE id=? AND activity_type IN ('Like','Announce') AND undone=0").bind(target).fetch_optional(&state.db).await.ok().flatten();
        let Some(original) = original.and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        original
    } else {
        json!(target)
    };
    let mut activity = activity_body(&state, kind, object);
    activity["cc"] = json!([actor.id]);
    let mut inboxes = vec![actor.inbox];
    if kind == "Announce" || kind == "Undo" {
        inboxes.extend(follower_inboxes(&state).await)
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return failure(db_error(e)),
    };
    if kind == "Undo" {
        if let Err(e) = sqlx::query("UPDATE federation_activities SET undone=1 WHERE id=?")
            .bind(target)
            .execute(&mut *tx)
            .await
        {
            return failure(db_error(e));
        }
    }
    if let Err(e) = enqueue_tx(&mut tx, &activity, &inboxes).await {
        return failure(e);
    }
    if let Err(e) = tx.commit().await {
        return failure(db_error(e));
    }
    (
        StatusCode::CREATED,
        Json(json!({"activities":[{"id":activity["id"]}]})),
    )
        .into_response()
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
    match sqlx::query("UPDATE federation_received SET read_at=? WHERE read_at IS NULL")
        .bind(dates::now_db())
        .execute(&state.db)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(db_error(e)),
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
    let actor = body["actor"].as_str().unwrap_or("");
    if !reqwest::Url::parse(actor).is_ok_and(|url| url.scheme() == "https") {
        return failure(bad("Invalid actor URL"));
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return failure(db_error(e)),
    };
    let result = if body["unblock"].as_bool() == Some(true) {
        sqlx::query("DELETE FROM federation_blocks WHERE actor=?")
            .bind(actor)
            .execute(&mut *tx)
            .await
    } else {
        sqlx::query("INSERT OR IGNORE INTO federation_blocks (actor,created_at) VALUES (?,?)")
            .bind(actor)
            .bind(dates::now_db())
            .execute(&mut *tx)
            .await
    };
    if let Err(e) = result {
        return failure(db_error(e));
    }
    if body["unblock"].as_bool() != Some(true) {
        for table in ["federation_followers", "federation_following"] {
            if let Err(e) = sqlx::query(&format!("DELETE FROM {table} WHERE actor=?"))
                .bind(actor)
                .execute(&mut *tx)
                .await
            {
                return failure(db_error(e));
            }
        }
    }
    if let Err(e) = tx.commit().await {
        return failure(db_error(e));
    }
    StatusCode::NO_CONTENT.into_response()
}
