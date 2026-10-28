//! Signed provider delivery events and persistent newsletter redirect links.
use crate::{permissions::{self, Role}, AppState};
use axum::{extract::{Path, Query, State}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Redirect, Response}, routing::{get,post}, Json, Router};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json,Value};
use std::collections::HashMap;

pub fn public_router() -> Router<AppState> {
    Router::new().route("/r/{id}",get(redirect)).route("/r/{id}/",get(redirect))
        .route("/members/webhooks/mailgun",post(mailgun)).route("/members/webhooks/mailgun/",post(mailgun))
}
pub fn admin_router() -> Router<AppState> {
    Router::new().route("/links",get(browse)).route("/links/",get(browse))
        .route("/links/{id}",get(read).put(edit)).route("/links/{id}/",get(read).put(edit))
        .route("/email-suppressions/",get(suppressions))
        .route("/email-suppressions/{email}",axum::routing::delete(unsuppress))
}
fn error(status:StatusCode,message:&str)->Response {(status,Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]}))).into_response()}
fn valid_url(url:&str)->bool { reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(),"http"|"https") && u.host_str().is_some() && u.username().is_empty() && u.password().is_none()) }

pub async fn suppression(state:&AppState,email:&str)->Value {
    let found=sqlx::query_as::<_,(String,String)>("SELECT reason, created_at FROM email_suppressions WHERE email = ?")
        .bind(email).fetch_optional(&state.db).await.ok().flatten();
    match found { Some((reason,created_at))=>json!({"suppressed":true,"info":{"reason":reason,"created_at":created_at}}),None=>json!({"suppressed":false,"info":null}) }
}
pub async fn remove_suppression(state:&AppState,email:&str)->Result<()> {
    let mut tx=state.db.begin().await.map_err(|e|GhostError::Db(e.to_string()))?;
    sqlx::query("DELETE FROM email_suppressions WHERE email = ?").bind(email).execute(&mut *tx).await.map_err(|e|GhostError::Db(e.to_string()))?;
    sqlx::query("UPDATE members SET email_disabled = 0 WHERE lower(email) = lower(?)").bind(email).execute(&mut *tx).await.map_err(|e|GhostError::Db(e.to_string()))?;
    tx.commit().await.map_err(|e|GhostError::Db(e.to_string()))?; Ok(())
}
async fn mailgun(State(state):State<AppState>,Json(body):Json<Value>)->Response {
    let key=state.settings.get_str("mailgun_webhook_signing_key").unwrap_or_default();
    let sig=&body["signature"];
    let timestamp=sig["timestamp"].as_str().map(str::to_owned).or_else(||sig["timestamp"].as_i64().map(|v|v.to_string())).unwrap_or_default();
    let token=sig["token"].as_str().unwrap_or("");
    let supplied=sig["signature"].as_str().unwrap_or("");
    let fresh=timestamp.parse::<i64>().is_ok_and(|t|(chrono::Utc::now().timestamp()-t).abs()<=300);
    if key.is_empty() || !fresh || !crate::members::secure_eq(&crate::members::hmac_sha256_hex(&key,&format!("{timestamp}{token}")),supplied) {
        return error(StatusCode::UNAUTHORIZED,"Invalid provider signature.");
    }
    let event=&body["event-data"];
    let kind=event["event"].as_str().unwrap_or("");
    let permanent=kind=="complained" || (kind=="failed" && event["severity"]=="permanent");
    let email=event["recipient"].as_str().unwrap_or("").trim().to_lowercase();
    let event_id=event["id"].as_str().unwrap_or("");
    if email.is_empty() || event_id.is_empty() { return error(StatusCode::BAD_REQUEST,"Event id and recipient are required."); }
    let mut tx=match state.db.begin().await {Ok(tx)=>tx,Err(_)=>return error(StatusCode::SERVICE_UNAVAILABLE,"Could not store delivery event.")};
    let inserted=sqlx::query("INSERT OR IGNORE INTO email_delivery_events (id,event_type,email,created_at) VALUES (?,?,?,?)")
        .bind(event_id).bind(kind).bind(&email).bind(dates::now_db()).execute(&mut *tx).await;
    match inserted {
        Ok(result) if result.rows_affected()==0 => return StatusCode::OK.into_response(),
        Ok(_) => {},
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE,"Could not store delivery event."),
    }
    if permanent {
        if sqlx::query("INSERT INTO email_suppressions (email,reason,provider,created_at) VALUES (?,?,'mailgun',?) ON CONFLICT(email) DO UPDATE SET reason=excluded.reason, created_at=excluded.created_at")
            .bind(&email).bind(if kind=="complained"{"spam complaint"}else{"permanent bounce"}).bind(dates::now_db()).execute(&mut *tx).await.is_err()
            || sqlx::query("UPDATE members SET email_disabled = 1 WHERE lower(email)=?").bind(&email).execute(&mut *tx).await.is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE,"Could not save suppression.");
        }
    }
    if tx.commit().await.is_err() { return error(StatusCode::SERVICE_UNAVAILABLE,"Could not store delivery event."); }
    StatusCode::OK.into_response()
}

/// Register each HTTP link once per post; placeholders personalize after rendering.
pub async fn track_links(state:&AppState,post_id:&str,html:&str)->Result<String> {
    if !state.settings.get_bool("email_track_clicks") { return Ok(html.into()); }
    let re=regex::Regex::new(r#"(?i)href\s*=\s*(["'])(https?://[^"'<>]+)["']"#).unwrap();
    let mut output=String::with_capacity(html.len()); let mut previous=0;
    for cap in re.captures_iter(html) {
        let matched=cap.get(0).unwrap(); let url=cap[2].replace("&amp;","&");
        if url.contains("%%{") || url.contains("/unsubscribe/") || !valid_url(&url) { continue; }
        let id=ids::random_hex(16); let now=dates::now_db();
        sqlx::query("INSERT INTO newsletter_links (id,post_id,from_url,to_url,created_at,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(post_id,from_url) DO NOTHING")
            .bind(&id).bind(post_id).bind(&url).bind(&url).bind(&now).bind(&now).execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
        let id:String=sqlx::query_scalar("SELECT id FROM newsletter_links WHERE post_id=? AND from_url=?")
            .bind(post_id).bind(&url).fetch_one(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
        output.push_str(&html[previous..matched.start()]);
        output.push_str(&format!("href=\"{}/r/{id}/?m=%%{{uuid}}%%\"",state.site_url())); previous=matched.end();
    }
    output.push_str(&html[previous..]); Ok(output)
}
async fn redirect(State(state):State<AppState>,Path(id):Path<String>,Query(params):Query<HashMap<String,String>>)->Response {
    let url=sqlx::query_scalar::<_,String>("SELECT to_url FROM newsletter_links WHERE id=?").bind(&id).fetch_optional(&state.db).await.ok().flatten();
    let Some(url)=url.filter(|u|valid_url(u)) else{return StatusCode::NOT_FOUND.into_response()};
    if let Some(member)=params.get("m") {
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM members WHERE uuid=?)").bind(member).fetch_one(&state.db).await.unwrap_or(false);
        if exists { let _=sqlx::query("INSERT INTO newsletter_clicks (id,link_id,member_uuid,created_at) VALUES (?,?,?,?)")
            .bind(ids::new_id()).bind(id).bind(member).bind(dates::now_db()).execute(&state.db).await; }
    }
    ([("cache-control","private, no-store")],Redirect::temporary(&url)).into_response()
}
async fn values(state:&AppState,id:Option<&str>,post:Option<&str>)->Vec<Value> {
    sqlx::query_as::<_,(String,String,String,String,i64,i64)>("SELECT l.id,l.post_id,l.from_url,l.to_url,COUNT(c.id),COUNT(DISTINCT c.member_uuid) FROM newsletter_links l LEFT JOIN newsletter_clicks c ON c.link_id=l.id WHERE (? IS NULL OR l.id=?) AND (? IS NULL OR l.post_id=?) GROUP BY l.id ORDER BY l.created_at DESC")
        .bind(id).bind(id).bind(post).bind(post).fetch_all(&state.db).await.unwrap_or_default().into_iter()
        .map(|(id,post_id,from,to,clicks,unique_clicks)|json!({"id":id,"post_id":post_id,"from":from,"to":to,"clicks":clicks,"unique_clicks":unique_clicks})).collect()
}
async fn browse(State(state):State<AppState>,headers:HeaderMap,Query(q):Query<HashMap<String,String>>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    Json(json!({"links":values(&state,None,q.get("post_id").map(String::as_str)).await})).into_response()
}
async fn read(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let links=values(&state,Some(&id),None).await;
    if links.is_empty(){return StatusCode::NOT_FOUND.into_response()}
    Json(json!({"links":links})).into_response()
}
async fn edit(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>,Json(body):Json<Value>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let url=body.pointer("/links/0/to").and_then(Value::as_str).unwrap_or("");
    if !valid_url(url){return error(StatusCode::UNPROCESSABLE_ENTITY,"An HTTP or HTTPS destination is required.")}
    match sqlx::query("UPDATE newsletter_links SET to_url=?,updated_at=? WHERE id=?").bind(url).bind(dates::now_db()).bind(&id).execute(&state.db).await {
        Ok(result) if result.rows_affected()>0=>Json(json!({"links":values(&state,Some(&id),None).await})).into_response(),
        Ok(_)=>StatusCode::NOT_FOUND.into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not update link.")
    }
}
async fn suppressions(State(state):State<AppState>,headers:HeaderMap)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Administrator).await{return r}
    let rows=sqlx::query_as::<_,(String,String,String)>("SELECT email,reason,created_at FROM email_suppressions ORDER BY created_at DESC").fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"suppressions":rows.into_iter().map(|(email,reason,created_at)|json!({"email":email,"reason":reason,"created_at":created_at})).collect::<Vec<_>>()})).into_response()
}
async fn unsuppress(State(state):State<AppState>,headers:HeaderMap,Path(email):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Administrator).await{return r}
    match remove_suppression(&state,&email).await{Ok(())=>StatusCode::NO_CONTENT.into_response(),Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not remove suppression.")}
}
