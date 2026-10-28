//! Native publication ActivityPub identity, inbox, outbox and delivery queue.
mod admin;
mod crypto;
mod network;
use crate::AppState;
use axum::{
    extract::{OriginalUri, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
const PUBLIC: &str = "https://www.w3.org/ns/activitystreams#Public";
pub fn enabled(state: &AppState) -> bool {
    state.settings.get_bool("social_web_enabled") && !state.settings.get_bool("is_private")
}
fn bad(message: &str) -> GhostError {
    GhostError::BadRequest(message.into())
}
fn db_error(error: sqlx::Error) -> GhostError {
    GhostError::Db(error.to_string())
}
fn failure(error: GhostError) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":error.message(),"errorType":"ValidationError"}]})),
    )
        .into_response()
}
fn actor_id(state: &AppState) -> String {
    format!("{}/.ghost/activitypub/actor", state.site_url())
}
fn endpoint(state: &AppState, path: &str) -> String {
    format!("{}/.ghost/activitypub/{path}", state.site_url())
}
fn ap(value: Value) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/activity+json; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        value.to_string(),
    )
        .into_response()
}
fn object_id(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value["id"].as_str())
}
fn same_origin(a: &str, b: &str) -> bool {
    reqwest::Url::parse(a)
        .ok()
        .zip(reqwest::Url::parse(b).ok())
        .is_some_and(|(a, b)| a.origin() == b.origin())
}
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/.well-known/webfinger", get(webfinger))
        .route("/.ghost/activitypub/notes/{id}", get(note_page))
        .route("/.ghost/activitypub/actor", get(actor))
        .route("/.ghost/activitypub/inbox", post(inbox))
        .route("/.ghost/activitypub/outbox", get(outbox))
        .route("/.ghost/activitypub/followers", get(followers))
        .route("/.ghost/activitypub/following", get(following))
        .route("/.ghost/activitypub/objects/{id}", get(object))
        .route("/.ghost/activitypub/activities/{id}", get(activity))
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
}
pub fn admin_router() -> Router<AppState> {
    admin::router()
}
async fn actor(State(state): State<AppState>) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (_, public) = match crypto::keys(&state).await {
        Ok(keys) => keys,
        Err(e) => return failure(e),
    };
    let id = actor_id(&state);
    ap(
        json!({"@context":["https://www.w3.org/ns/activitystreams","https://w3id.org/security/v1"],"id":id,"type":"Service","preferredUsername":"publication","name":state.settings.site_name(),"summary":state.settings.get_str("description").unwrap_or_default(),"url":state.site_url(),"inbox":endpoint(&state,"inbox"),"outbox":endpoint(&state,"outbox"),"followers":endpoint(&state,"followers"),"following":endpoint(&state,"following"),"manuallyApprovesFollowers":false,"discoverable":true,"published":state.settings.get_str("site_created_at"),"icon":state.settings.get_str("icon").filter(|v|!v.is_empty()).map(|url|json!({"type":"Image","url":ghost_core::url::absolute(&state.site_url(),&url)})),"image":state.settings.get_str("cover_image").filter(|v|!v.is_empty()).map(|url|json!({"type":"Image","url":ghost_core::url::absolute(&state.site_url(),&url)})),"publicKey":{"id":format!("{id}#main-key"),"owner":id,"publicKeyPem":public}}),
    )
}
async fn webfinger(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let site = match reqwest::Url::parse(&state.site_url()) {
        Ok(site) => site,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let host = match site.port() {
        Some(port) => format!("{}:{port}", site.host_str().unwrap_or("")),
        None => site.host_str().unwrap_or("").to_owned(),
    };
    let account = format!("acct:publication@{host}");
    if !q
        .get("resource")
        .is_some_and(|resource| resource == &account || resource == &actor_id(&state))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    ([(header::CONTENT_TYPE,"application/jrd+json")],json!({"subject":account,"aliases":[actor_id(&state),state.site_url()],"links":[{"rel":"self","type":"application/activity+json","href":actor_id(&state)},{"rel":"http://webfinger.net/rel/profile-page","type":"text/html","href":state.site_url()}]}).to_string()).into_response()
}
async fn followers(State(state): State<AppState>) -> Response {
    collection(&state, "followers").await
}
async fn following(State(state): State<AppState>) -> Response {
    collection(&state, "following").await
}
async fn collection(state: &AppState, kind: &str) -> Response {
    if !enabled(state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let table = if kind == "followers" {
        "federation_followers"
    } else {
        "federation_following"
    };
    let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    // Counts are public; follower actor lists remain private to prevent harvesting.
    ap(
        json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(state,kind),"type":"OrderedCollection","totalItems":count}),
    )
}
async fn outbox(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM federation_activities WHERE undone=0 AND activity_type IN ('Create','Update','Delete','Announce')").fetch_one(&state.db).await.unwrap_or(0);
    if !q.contains_key("page") {
        return ap(
            json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(&state,"outbox"),"type":"OrderedCollection","totalItems":count,"first":format!("{}?page=1",endpoint(&state,"outbox"))}),
        );
    }
    let page = q
        .get("page")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1)
        .clamp(1, 100000);
    let bodies=sqlx::query_scalar::<_,String>("SELECT body FROM federation_activities WHERE undone=0 AND activity_type IN ('Create','Update','Delete','Announce') ORDER BY created_at DESC,id DESC LIMIT 20 OFFSET ?").bind((page-1)*20).fetch_all(&state.db).await.unwrap_or_default();
    let mut items = Vec::new();
    for body in bodies {
        if let Ok(body) = serde_json::from_str::<Value>(&body) {
            items.push(current_activity(&state, body).await)
        }
    }
    let mut result = json!({"@context":"https://www.w3.org/ns/activitystreams","id":format!("{}?page={page}",endpoint(&state,"outbox")),"type":"OrderedCollectionPage","partOf":endpoint(&state,"outbox"),"orderedItems":items});
    if page * 20 < count {
        result["next"] = json!(format!("{}?page={}", endpoint(&state, "outbox"), page + 1));
    }
    ap(result)
}
async fn object(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let url = endpoint(&state, &format!("objects/{id}"));
    let row=sqlx::query_as::<_,(String,String,Option<String>,String,bool)>("SELECT content,url,in_reply_to,published_at,deleted FROM federation_objects WHERE id=? AND local=1").bind(&url).fetch_optional(&state.db).await.ok().flatten();
    if let Some((content, url, reply, published, deleted)) = row {
        if deleted {
            return (StatusCode::GONE,ap(json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(&state,&format!("objects/{id}")),"type":"Tombstone"}))).into_response();
        }
        return ap(
            json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(&state,&format!("objects/{id}")),"type":"Note","attributedTo":actor_id(&state),"url":url,"content":content,"inReplyTo":reply,"published":published,"to":[PUBLIC],"cc":[endpoint(&state,"followers")]}),
        );
    }
    match ghost_db::repos::posts::find_by_id(&state.db, &id).await {
        Ok(Some(post)) if post.status == "published" => ap(post_object(&state, &post).await),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn activity(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let body=sqlx::query_scalar::<_,String>("SELECT body FROM federation_activities WHERE id=? AND undone=0 AND activity_type IN ('Create','Update','Delete','Announce')").bind(endpoint(&state,&format!("activities/{id}"))).fetch_optional(&state.db).await.ok().flatten();
    match body.and_then(|s| serde_json::from_str(&s).ok()) {
        Some(body) => ap(current_activity(&state, body).await),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn post_object(state: &AppState, post: &ghost_core::models::Post) -> Value {
    let mut data = ghost_core::models::post_to_json(post, &[], &[], "");
    let resolver = crate::content::resolve(state).await;
    data["url"] = json!(resolver(&data));
    let html =
        ghost_core::url::absolutize_html(&state.site_url(), &ghost_core::models::public_html(post));
    json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(state,&format!("objects/{}",post.id)),"type":"Article","attributedTo":actor_id(state),"name":post.title,"summary":post.custom_excerpt,"content":html,"url":data["url"],"published":post.published_at.as_deref().and_then(dates::from_db_string).map(|d|d.to_rfc3339()),"updated":post.updated_at.as_deref().and_then(dates::from_db_string).map(|d|d.to_rfc3339()),"to":[PUBLIC],"cc":[endpoint(state,"followers")],"sensitive":false})
}
async fn enqueue_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    body: &Value,
    inboxes: &[String],
) -> Result<()> {
    let id = body["id"]
        .as_str()
        .ok_or_else(|| bad("Missing activity id"))?;
    let now = dates::now_db();
    sqlx::query("INSERT OR IGNORE INTO federation_activities (id,activity_type,object_id,body,created_at) VALUES (?,?,?,?,?)").bind(id).bind(body["type"].as_str().unwrap_or("")).bind(object_id(&body["object"])).bind(body.to_string()).bind(&now).execute(&mut **tx).await.map_err(db_error)?;
    for inbox in inboxes {
        sqlx::query("INSERT OR IGNORE INTO federation_deliveries (id,activity_id,inbox,next_attempt_at) VALUES (?,?,?,?)").bind(ids::new_id()).bind(id).bind(inbox).bind(&now).execute(&mut **tx).await.map_err(db_error)?;
    }
    Ok(())
}
async fn enqueue(state: &AppState, body: &Value, inboxes: &[String]) -> Result<()> {
    let mut tx = state.db.begin().await.map_err(db_error)?;
    enqueue_tx(&mut tx, body, inboxes).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}
async fn follower_inboxes(state: &AppState) -> Vec<String> {
    sqlx::query_scalar("SELECT DISTINCT inbox FROM federation_followers")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
}
fn activity_body(state: &AppState, kind: &str, object: Value) -> Value {
    json!({"@context":"https://www.w3.org/ns/activitystreams","id":endpoint(state,&format!("activities/{}",ids::uuid_v4())),"type":kind,"actor":actor_id(state),"object":object,"to":[PUBLIC],"cc":[endpoint(state,"followers")],"published":chrono::Utc::now().to_rfc3339()})
}
static INBOX_WORKERS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
async fn inbox(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let pool = INBOX_WORKERS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(8)))
        .clone();
    let Ok(_permit) = pool.try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let activity: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Some(actor) = activity["actor"].as_str() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(id) = activity["id"]
        .as_str()
        .filter(|id| id.len() < 2000 && same_origin(id, actor))
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let blocked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_blocks WHERE actor=?)")
            .bind(actor)
            .fetch_one(&state.db)
            .await
            .unwrap_or(true);
    if blocked {
        return StatusCode::ACCEPTED.into_response();
    }
    let Some(key) = crypto::key_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let actor = match network::actor(actor).await {
        Ok(actor) => actor,
        Err(e) => return failure(e),
    };
    if actor.key_id != key {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let base = ghost_core::url::subdir();
    let path = uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or(uri.path());
    let path = if !base.is_empty() && !path.starts_with(&format!("{base}/")) {
        format!("{base}{path}")
    } else {
        path.into()
    };
    if !crypto::verify(&headers, &path, &body, &key, &actor.public_key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM federation_received WHERE actor=? AND datetime(created_at)>datetime('now','-1 hour')").bind(&actor.id).fetch_one(&state.db).await.unwrap_or(1001);
    if count >= 1000 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    match receive(&state.db, &actor_id(&state), &actor, id, &activity).await {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => failure(e),
    }
}
async fn receive(
    pool: &sqlx::SqlitePool,
    local_actor: &str,
    actor: &network::Actor,
    id: &str,
    activity: &Value,
) -> Result<()> {
    let kind = activity["type"].as_str().unwrap_or("");
    let now = dates::now_db();
    let mut tx = pool.begin().await.map_err(db_error)?;
    let stored=sqlx::query("INSERT OR IGNORE INTO federation_received (id,actor,activity_type,body,created_at) VALUES (?,?,?,?,?)").bind(id).bind(&actor.id).bind(kind).bind(activity.to_string()).bind(&now).execute(&mut *tx).await.map_err(db_error)?;
    if stored.rows_affected() == 0 {
        return Ok(());
    }
    let object = &activity["object"];
    match kind {
        "Follow" => {
            if object_id(object) != Some(local_actor) {
                return Err(bad("Follow addressed to another actor"));
            }
            sqlx::query("INSERT INTO federation_followers (actor,inbox,follow_id,created_at) VALUES (?,?,?,?) ON CONFLICT(actor) DO UPDATE SET inbox=excluded.inbox,follow_id=excluded.follow_id").bind(&actor.id).bind(&actor.inbox).bind(id).bind(&now).execute(&mut *tx).await.map_err(db_error)?;
            let mut accept = json!({"@context":"https://www.w3.org/ns/activitystreams","id":format!("{}/activities/{}",local_actor.trim_end_matches("/actor"),ids::uuid_v4()),"type":"Accept","actor":local_actor,"object":activity});
            accept["to"] = json!([actor.id]);
            accept["cc"] = json!([]);
            enqueue_tx(&mut tx, &accept, &[actor.inbox.clone()]).await?;
        }
        "Accept" | "Reject" => {
            let follow = object_id(object).unwrap_or("");
            let result = if kind == "Accept" {
                sqlx::query(
                    "UPDATE federation_following SET accepted=1 WHERE actor=? AND follow_id=?",
                )
                .bind(&actor.id)
                .bind(follow)
                .execute(&mut *tx)
                .await
            } else {
                sqlx::query("DELETE FROM federation_following WHERE actor=? AND follow_id=?")
                    .bind(&actor.id)
                    .bind(follow)
                    .execute(&mut *tx)
                    .await
            };
            result.map_err(db_error)?;
        }
        "Undo" => {
            let original = object_id(object).unwrap_or("");
            sqlx::query("DELETE FROM federation_followers WHERE actor=? AND follow_id=?")
                .bind(&actor.id)
                .bind(original)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            sqlx::query("UPDATE federation_received SET undone=1 WHERE actor=? AND id=?")
                .bind(&actor.id)
                .bind(original)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
        "Create" | "Update" => {
            if matches!(object["type"].as_str(), Some("Note" | "Article")) {
                if object["attributedTo"].as_str() != Some(&actor.id) {
                    return Err(bad(
                        "Object attribution does not match its authenticated actor",
                    ));
                }
                let object_id = object["id"]
                    .as_str()
                    .filter(|id| id.len() < 2000 && same_origin(id, &actor.id))
                    .ok_or_else(|| bad("Invalid object id"))?;
                let existing: Option<String> =
                    sqlx::query_scalar("SELECT actor FROM federation_objects WHERE id=?")
                        .bind(object_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(db_error)?;
                if existing.as_ref().is_some_and(|id| id != &actor.id) {
                    return Err(bad("Object belongs to another actor"));
                }
                let content = object["content"].as_str().unwrap_or("");
                if content.len() > 128 * 1024 {
                    return Err(bad("Note exceeds the content limit"));
                }
                let public = addresses_public(activity) || addresses_public(object);
                // Reader displays only public-addressed posts. Private inbox data stays private.
                if public {
                    sqlx::query("INSERT INTO federation_objects (id,actor,object_type,content,url,in_reply_to,published_at,updated_at) VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET content=excluded.content,url=excluded.url,updated_at=excluded.updated_at WHERE federation_objects.actor=excluded.actor AND federation_objects.deleted=0")
                    .bind(object_id).bind(&actor.id).bind(object["type"].as_str().unwrap_or("Note")).bind(content).bind(object["url"].as_str().unwrap_or(object_id)).bind(object_id_ref(&object["inReplyTo"]))
                    .bind(object["published"].as_str().and_then(|d|chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d|d.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(||now.clone()))
                    .bind(&now).execute(&mut *tx).await.map_err(db_error)?;
                }
            }
        }
        "Delete" => {
            if let Some(object) = object_id(object) {
                sqlx::query("UPDATE federation_objects SET deleted=1,content='',updated_at=? WHERE id=? AND actor=? AND local=0").bind(&now).bind(object).bind(&actor.id).execute(&mut *tx).await.map_err(db_error)?;
            }
        }
        "Like" | "Announce" => {}
        _ => {}
    }
    tx.commit().await.map_err(db_error)?;
    Ok(())
}
fn object_id_ref(value: &Value) -> Option<&str> {
    object_id(value)
}
fn addresses_public(value: &Value) -> bool {
    ["to", "cc"].iter().any(|key| {
        value[*key]
            .as_array()
            .is_some_and(|list| list.iter().any(|id| id == PUBLIC))
            || value[*key] == PUBLIC
    })
}

/// Called by both publication paths. Only public excerpts are exported for gated posts.
pub fn post_changed(state: &AppState, id: &str, deleted: bool) {
    if !enabled(state) {
        return;
    }
    let state = state.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
        if let Err(e) = publish_post(&state, &id, deleted).await {
            tracing::warn!("Social publication failed: {}", e.message());
        }
    });
}
async fn publish_post(state: &AppState, id: &str, deleted: bool) -> Result<()> {
    if !enabled(state) {
        return Ok(());
    }
    let object_uri = endpoint(state, &format!("objects/{id}"));
    let existed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM federation_activities WHERE object_id=? AND activity_type IN ('Create','Update'))").bind(&object_uri).fetch_one(&state.db).await.map_err(db_error)?;
    let post = ghost_db::repos::posts::find_by_id(&state.db, id).await?;
    let published = post.as_ref().is_some_and(|post| post.status == "published");
    if (!published || deleted) && !existed {
        return Ok(());
    }
    let (kind, object) = if deleted || !published {
        ("Delete", json!({"id":object_uri,"type":"Tombstone"}))
    } else {
        (
            if existed { "Update" } else { "Create" },
            post_object(state, post.as_ref().unwrap()).await,
        )
    };
    let mut activity = activity_body(state, kind, object);
    use sha2::{Digest, Sha256};
    let revision = post
        .as_ref()
        .and_then(|p| p.updated_at.as_deref())
        .unwrap_or("deleted");
    activity["id"] = json!(endpoint(
        state,
        &format!(
            "activities/{}",
            hex::encode(Sha256::digest(
                format!("{id}:{revision}:{kind}:{}", activity["object"]).as_bytes()
            ))
        )
    ));
    enqueue(state, &activity, &follower_inboxes(state).await).await
}
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            if enabled(&state) {
                if let Err(e) = deliver(&state).await {
                    tracing::warn!("Social delivery worker failed: {}", e.message());
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}
async fn deliver(state: &AppState) -> Result<()> {
    let pending=sqlx::query_as::<_,(String,String,String,i64)>("SELECT d.id,d.inbox,a.body,d.attempts FROM federation_deliveries d JOIN federation_activities a ON a.id=d.activity_id WHERE d.delivered_at IS NULL AND d.attempts<12 AND datetime(d.next_attempt_at)<=datetime('now') ORDER BY d.next_attempt_at LIMIT 50").fetch_all(&state.db).await.map_err(db_error)?;
    if pending.is_empty() {
        return Ok(());
    }
    let (private, _) = crypto::keys(state).await?;
    for (id, inbox, body, attempts) in pending {
        let retry = (chrono::Utc::now()
            + chrono::Duration::seconds((30i64 * 2i64.pow(attempts.min(10) as u32)).min(86400)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
        let claimed=sqlx::query("UPDATE federation_deliveries SET attempts=attempts+1,next_attempt_at=? WHERE id=? AND attempts=? AND delivered_at IS NULL").bind(retry).bind(&id).bind(attempts).execute(&state.db).await.map_err(db_error)?.rows_affected()==1;
        if !claimed {
            continue;
        }
        let body = current_activity(
            state,
            serde_json::from_str(&body).map_err(|_| bad("Invalid queued activity"))?,
        )
        .await
        .to_string();
        match send(state, &private, &inbox, &body).await {
            Ok(()) => {
                sqlx::query(
                    "UPDATE federation_deliveries SET delivered_at=?,last_error=NULL WHERE id=?",
                )
                .bind(dates::now_db())
                .bind(id)
                .execute(&state.db)
                .await
                .map_err(db_error)?;
            }
            Err(error) => {
                sqlx::query("UPDATE federation_deliveries SET last_error=? WHERE id=?")
                    .bind(error.message())
                    .bind(id)
                    .execute(&state.db)
                    .await
                    .map_err(db_error)?;
            }
        }
    }
    Ok(())
}
async fn send(state: &AppState, private: &str, inbox: &str, body: &str) -> Result<()> {
    let (client, url) = network::client(inbox).await?;
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or("")),
        None => url.host_str().unwrap_or("").to_owned(),
    };
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let digest = crypto::digest(body);
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    };
    let canonical =
        format!("(request-target): post {path}\nhost: {host}\ndate: {date}\ndigest: {digest}");
    let signature = crypto::sign(private, &canonical)?;
    let signature=format!("keyId=\"{}#main-key\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date digest\",signature=\"{signature}\"",actor_id(state));
    let response = client
        .post(url)
        .header("host", host)
        .header("date", date)
        .header("digest", digest)
        .header("signature", signature)
        .header("content-type", "application/activity+json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|_| bad("Remote inbox could not be reached"))?;
    if !response.status().is_success() {
        return Err(bad(&format!("Remote inbox returned {}", response.status())));
    }
    Ok(())
}

/// Historical outbox and queued bodies must respect a post's current access policy.
async fn current_activity(state: &AppState, mut activity: Value) -> Value {
    if !matches!(activity["type"].as_str(), Some("Create" | "Update")) {
        return activity;
    }
    let Some(object) = object_id(&activity["object"]).map(str::to_owned) else {
        return activity;
    };
    let prefix = endpoint(state, "objects/");
    let Some(id) = object.strip_prefix(&prefix) else {
        return activity;
    };
    if ghost_core::ids::is_objectid(id) {
        match ghost_db::repos::posts::find_by_id(&state.db, id).await {
            Ok(Some(post)) if post.status == "published" => {
                activity["object"] = post_object(state, &post).await
            }
            _ => {
                activity["type"] = json!("Delete");
                activity["object"] = json!({"id":object,"type":"Tombstone"});
            }
        }
    } else {
        let deleted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM federation_objects WHERE id=? AND local=1 AND deleted=1)",
        )
        .bind(&object)
        .fetch_one(&state.db)
        .await
        .unwrap_or(true);
        if deleted {
            activity["type"] = json!("Delete");
            activity["object"] = json!({"id":object,"type":"Tombstone"});
        }
    }
    activity
}
async fn note_page(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT content,published_at FROM federation_objects WHERE id=? AND local=1 AND deleted=0",
    )
    .bind(endpoint(&state, &format!("objects/{id}")))
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((content, date)) = row else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    axum::response::Html(format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Note — {}</title><style>body{{max-width:700px;margin:48px auto;padding:0 24px;font:18px/1.7 system-ui;color:#242127}}a{{color:inherit}}img,video{{max-width:100%}}header{{margin-bottom:48px}}time{{font-size:14px;color:#666}}</style></head><body><header><a href=\"{}\">{}</a></header><article>{content}</article><time>{}</time><script src=\"{}\" defer></script></body></html>",escape(&state.settings.site_name()),escape(&state.site_url()),escape(&state.settings.site_name()),escape(&date),ghost_core::url::base("/public/staff-toolbar.js"))).into_response()
}

#[cfg(test)]
mod receive_tests {
    use super::*;
    #[tokio::test]
    async fn follows_undos_and_object_ownership_are_atomic_and_replay_safe() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&db).await.unwrap();
        let local = "https://publication.example/.ghost/activitypub/actor";
        let actor = network::Actor {
            id: "https://remote.example/actor".into(),
            inbox: "https://remote.example/inbox".into(),
            key_id: String::new(),
            public_key: String::new(),
        };
        let follow = json!({"id":"https://remote.example/follow/1","type":"Follow","actor":actor.id,"object":local});
        receive(&db, local, &actor, follow["id"].as_str().unwrap(), &follow)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM federation_followers")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM federation_deliveries")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let undo = json!({"id":"https://remote.example/undo/1","type":"Undo","actor":actor.id,"object":follow});
        receive(&db, local, &actor, undo["id"].as_str().unwrap(), &undo)
            .await
            .unwrap();
        receive(&db, local, &actor, follow["id"].as_str().unwrap(), &follow)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM federation_followers")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let spoof = json!({"id":"https://remote.example/create/spoof","type":"Create","actor":actor.id,"object":{"id":"https://remote.example/note/1","type":"Note","attributedTo":"https://victim.example/actor","content":"Spoofed","to":[PUBLIC]}});
        assert!(
            receive(&db, local, &actor, spoof["id"].as_str().unwrap(), &spoof)
                .await
                .is_err()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM federation_received WHERE id=?")
            .bind(spoof["id"].as_str())
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let create = json!({"id":"https://remote.example/create/1","type":"Create","actor":actor.id,"object":{"id":"https://remote.example/note/1","type":"Note","attributedTo":actor.id,"content":"Original","to":[PUBLIC]}});
        receive(&db, local, &actor, create["id"].as_str().unwrap(), &create)
            .await
            .unwrap();
        let delete = json!({"id":"https://remote.example/delete/1","type":"Delete","actor":actor.id,"object":"https://remote.example/note/1"});
        receive(&db, local, &actor, delete["id"].as_str().unwrap(), &delete)
            .await
            .unwrap();
        let mut stale = create.clone();
        stale["id"] = json!("https://remote.example/create/stale");
        receive(&db, local, &actor, stale["id"].as_str().unwrap(), &stale)
            .await
            .unwrap();
        let(deleted,content):(bool,String)=sqlx::query_as("SELECT deleted,content FROM federation_objects WHERE id='https://remote.example/note/1'").fetch_one(&db).await.unwrap();
        assert!(deleted);
        assert!(content.is_empty());
    }
}
