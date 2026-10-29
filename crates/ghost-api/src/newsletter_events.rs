//! Signed provider delivery events and persistent newsletter redirect links.
use crate::{permissions::{self, Role}, AppState};
use axum::{extract::{Path, Query, State}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Redirect, Response}, routing::{get,post}, Json, Router};
use ghost_core::{dates, ids, GhostError, Result};
use serde::Serialize;
use serde_json::{json,Value};
use std::collections::HashMap;

pub fn public_router() -> Router<AppState> {
    Router::new().route("/r/{id}",get(redirect)).route("/r/{id}/",get(redirect))
        .route("/r/o/{email_id}",get(open_pixel)).route("/r/o/{email_id}/",get(open_pixel))
        .route("/members/webhooks/mailgun",post(mailgun)).route("/members/webhooks/mailgun/",post(mailgun))
}
pub fn admin_router() -> Router<AppState> {
    let mut router=Router::new().route("/links",get(browse)).route("/links/",get(browse))
        .route("/links/{id}",get(read).put(edit)).route("/links/{id}/",get(read).put(edit))
        .route("/email-suppressions/",get(suppressions))
        .route("/email-suppressions/{email}",axum::routing::delete(unsuppress));
    // Ghost's emails family, minus the retry: sending lives in the admin
    // crate, so `PUT /emails/{id}/retry` is registered alongside this from
    // the binary, where both crates are in scope.
    for suffix in ["","/"] {
        router=router
            .route(&format!("/emails{suffix}"),get(browse_emails))
            .route(&format!("/emails/{{id}}{suffix}"),get(read_email))
            .route(&format!("/emails/{{id}}/batches{suffix}"),get(email_batches))
            .route(&format!("/emails/{{id}}/recipient-failures{suffix}"),get(email_failures))
            .route(&format!("/emails/{{id}}/status{suffix}"),get(email_status));
    }
    router
}

/// Ghost's `emails` resource: one row per newsletter that has gone out.
///
/// Matched by name rather than position: the row is wider than the 16-column
/// tuple sqlx can build, and a positional list this long is one insertion
/// away from silently reading the wrong column into the wrong field.
#[derive(Serialize, sqlx::FromRow)]
struct EmailRow {
    id: String,
    uuid: Option<String>,
    post_id: String,
    status: String,
    recipient_filter: Option<String>,
    error: Option<String>,
    email_count: i64,
    candidate_count: Option<i64>,
    preflight_email_count: Option<i64>,
    preparation_excluded_count: Option<i64>,
    delivered_count: i64,
    opened_count: i64,
    failed_count: i64,
    subject: Option<String>,
    #[sqlx(rename = "from")]
    from: Option<String>,
    reply_to: Option<String>,
    source_type: Option<String>,
    track_opens: bool,
    track_clicks: bool,
    feedback_enabled: bool,
    prepared_at: Option<String>,
    submitted_at: Option<String>,
    newsletter_id: Option<String>,
    created_at: String,
    updated_at: Option<String>,
}

const EMAIL_COLS: &str = "id,uuid,post_id,status,recipient_filter,error,email_count,candidate_count,\
    preflight_email_count,preparation_excluded_count,delivered_count,opened_count,failed_count,\
    subject,`from`,reply_to,source_type,track_opens,track_clicks,feedback_enabled,prepared_at,\
    submitted_at,newsletter_id,created_at,updated_at";

async fn emails_of(state:&AppState,id:Option<&str>)->Vec<EmailRow> {
    sqlx::query_as::<_,EmailRow>(&format!(
        "SELECT {EMAIL_COLS} FROM emails WHERE (? IS NULL OR id=?) ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(id).bind(id).fetch_all(&state.db).await.unwrap_or_default()
}

async fn browse_emails(State(state):State<AppState>,headers:HeaderMap)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    Json(json!({"emails":emails_of(&state,None).await})).into_response()
}

async fn read_email(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let emails=emails_of(&state,Some(&id)).await;
    if emails.is_empty(){return StatusCode::NOT_FOUND.into_response()}
    Json(json!({"emails":emails})).into_response()
}

/// What the admin polls while a send is in flight: the status and the counts,
/// without the body.
async fn email_status(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let row=sqlx::query_as::<_,(String,i64,i64,i64,i64,Option<String>)>(
        "SELECT status,email_count,delivered_count,failed_count,opened_count,error FROM emails WHERE id=?",
    ).bind(&id).fetch_optional(&state.db).await.ok().flatten();
    let Some((status,email_count,delivered,failed,opened,error))=row else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let batches=sqlx::query_as::<_,(i64,i64)>(
        "SELECT COUNT(*),COALESCE(SUM(status='submitted'),0) FROM email_batches WHERE email_id=?",
    ).bind(&id).fetch_one(&state.db).await.unwrap_or((0,0));
    (
        [("cache-control","private, no-store")],
        Json(json!({"emails":[{"id":id,"status":status,"email_count":email_count,
            "delivered_count":delivered,"failed_count":failed,"opened_count":opened,"error":error,
            "batch_count":batches.0,"submitted_batch_count":batches.1}]})),
    ).into_response()
}

/// One row per provider request the send was split into.
async fn email_batches(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let rows=sqlx::query_as::<_,(String,String,Option<String>,Option<String>,i64,i64,Option<i64>,Option<String>,String)>(
        "SELECT id,status,provider_id,member_segment,recipient_count,submitted_count,error_status_code,error_message,created_at \
         FROM email_batches WHERE email_id=? ORDER BY created_at",
    ).bind(&id).fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"batches":rows.into_iter().map(|(id,status,provider_id,segment,recipients,submitted,code,message,created_at)|
        json!({"id":id,"status":status,"provider_id":provider_id,"member_segment":segment,
            "recipient_count":recipients,"submitted_count":submitted,"error_status_code":code,
            "error_message":message,"created_at":created_at})).collect::<Vec<_>>()})).into_response()
}

/// Who the send could not reach, and why. The member is joined in because
/// "which address bounced" is the question this answers.
async fn email_failures(State(state):State<AppState>,headers:HeaderMap,Path(id):Path<String>)->Response {
    if let Err(r)=permissions::require(&state,&headers,Role::Editor).await{return r}
    let rows=sqlx::query_as::<_,(String,Option<String>,Option<String>,Option<String>,Option<i64>,Option<String>,Option<String>,Option<String>,Option<String>)>(
        "SELECT f.id,f.member_id,m.email,m.name,f.code,f.enhanced_code,f.message,f.severity,f.failed_at \
         FROM email_recipient_failures f LEFT JOIN members m ON m.id=f.member_id \
         WHERE f.email_id=? ORDER BY f.failed_at DESC LIMIT 500",
    ).bind(&id).fetch_all(&state.db).await.unwrap_or_default();
    Json(json!({"failures":rows.into_iter().map(|(id,member_id,email,name,code,enhanced,message,severity,failed_at)|
        json!({"id":id,"member_id":member_id,"member_email":email,"member_name":name,"code":code,
            "enhanced_code":enhanced,"message":message,"severity":severity,"failed_at":failed_at})).collect::<Vec<_>>()})).into_response()
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
    ingest_provider_event(&state,&body["event-data"]).await
}

pub(crate) async fn ingest_provider_event(state:&AppState,event:&Value)->Response {
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
    // Delivery and engagement events, matched to the recipient row by the id
    // Mailgun returned when the message was accepted. Done after the commit
    // so that recording the event is never what fails the webhook: Mailgun
    // retries a non-2xx, and a retried `delivered` must not re-suppress.
    //
    // `opened` shares the counter with the pixel above, and both are guarded
    // by `opened_at IS NULL`, so a Mailgun site that also loads the pixel
    // counts the open once.
    let provider_id=event.pointer("/message/headers/message-id").and_then(Value::as_str).unwrap_or("")
        .trim_matches(|c|c=='<'||c=='>');
    if !provider_id.is_empty() {
        match kind {
            "delivered" => { record_delivery(&state,provider_id).await; }
            "opened" if state.settings.get_bool("email_track_opens") => {
                if let Some(target)=recipient_of(&state,provider_id).await {
                    if let Some(uuid)=target.member_uuid {
                        let _=record_open(&state,&target.email_id,&uuid).await;
                    }
                }
            }
            // The provider accepted the message and then could not deliver
            // it. The recipient row says `sent`, because it was; this is the
            // separate record of what became of it, and what the admin reads
            // to say who was missed.
            "failed" => {
                if let Some(target)=recipient_of(&state,provider_id).await {
                    let status=&event["delivery-status"];
                    let message=status["message"].as_str()
                        .or_else(||status["description"].as_str())
                        .or_else(||event["reason"].as_str())
                        .unwrap_or("the provider could not deliver this message");
                    let severity=event["severity"].as_str().unwrap_or("temporary");
                    let _=record_failure(&state,Failure{
                        email_id:&target.email_id,batch_id:target.batch_id.as_deref(),
                        member_id:target.member_id.as_deref(),recipient_id:Some(&target.recipient_id),
                        message,severity,
                        code:status["code"].as_i64().or_else(||status["code"].as_str().and_then(|c|c.parse().ok())),
                        enhanced_code:status["enhanced-code"].as_str(),
                        event_id:Some(event_id),
                    }).await;
                    let now=dates::now_db();
                    let _=sqlx::query("UPDATE email_recipients SET status='failed',failed_at=?,error=?,updated_at=? WHERE id=?")
                        .bind(&now).bind(message).bind(&now).bind(&target.recipient_id).execute(&state.db).await;
                    let _=sqlx::query("UPDATE emails SET failed_count=failed_count+1,delivered_count=MAX(delivered_count-1,0),updated_at=? WHERE id=?")
                        .bind(&now).bind(&target.email_id).execute(&state.db).await;
                }
            }
            _ => {}
        }
    }
    StatusCode::OK.into_response()
}

/// What a send knows about one recipient once the transport has answered.
pub struct RecipientRecord<'a> {
    pub email_id:&'a str,
    /// The provider request this address was part of, if the transport sends
    /// in batches.
    pub batch_id:Option<&'a str>,
    pub member_id:&'a str,
    pub member_uuid:&'a str,
    pub email:&'a str,
    /// The provider's id for this recipient's message, which its delivery and
    /// open events are keyed on.
    pub provider_id:Option<&'a str>,
    /// `None` when the message was accepted.
    pub error:Option<&'a str>,
}

/// One row per recipient of a send, which is what the newsletter analytics
/// count and what an open or a delivery event is later matched against.
///
/// `status` is `sent` or `failed`, mirroring the two outcomes the send can
/// see from the transport. A row is written either way: a send that only
/// half-succeeded should show as such rather than vanish.
pub async fn record_recipient(state:&AppState,r:RecipientRecord<'_>)->Result<()> {
    let RecipientRecord{email_id,batch_id,member_id,member_uuid,email,provider_id,error}=r;
    let now=dates::now_db();
    let failed=error.is_some();
    let recipient_id=ids::new_id();
    sqlx::query(
        "INSERT INTO email_recipients \
         (id,email_id,batch_id,member_id,member_uuid,status,delivered_at,failed_at,error,email_address,mailgun_message_id,created_at,updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&recipient_id).bind(email_id).bind(batch_id).bind(member_id).bind(member_uuid)
    .bind(if failed {"failed"} else {"sent"})
    .bind(if failed {None} else {Some(&now)})
    .bind(if failed {Some(&now)} else {None})
    .bind(error)
    .bind(email)
    // Mailgun brackets the id it returns, and reports it unbracketed on its
    // webhooks. Stored unbracketed so the two match.
    .bind(provider_id.map(|id|id.trim_matches(|c|c=='<'||c=='>')))
    .bind(&now).bind(&now)
    .execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    // `email_count` is the denominator of the member's open rate, so it has
    // to move when a message actually goes out — otherwise every member's
    // rate stays null however many opens are recorded against them.
    if !failed {
        sqlx::query("UPDATE members SET email_count=email_count+1,updated_at=? WHERE id=?")
            .bind(&now).bind(member_id).execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    }
    // A refusal from the transport is a failure worth keeping separately from
    // the recipient row: this is the table the admin reads to say who was
    // missed and why, and the retry reads to decide who to try again.
    if let Some(message)=error {
        record_failure(state,Failure{
            email_id,batch_id,member_id:Some(member_id),recipient_id:Some(&recipient_id),
            message,severity:"temporary",code:None,enhanced_code:None,event_id:None,
        }).await?;
    }
    Ok(())
}

/// Why one recipient did not get the email.
pub struct Failure<'a> {
    pub email_id:&'a str,
    pub batch_id:Option<&'a str>,
    pub member_id:Option<&'a str>,
    pub recipient_id:Option<&'a str>,
    pub message:&'a str,
    /// `permanent` or `temporary`. Only a temporary failure is worth retrying.
    pub severity:&'a str,
    /// The provider's numeric SMTP code, when it reported one.
    pub code:Option<i64>,
    pub enhanced_code:Option<&'a str>,
    /// The provider event this came from, so a redelivered webhook does not
    /// record the same failure twice.
    pub event_id:Option<&'a str>,
}

pub async fn record_failure(state:&AppState,f:Failure<'_>)->Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO email_recipient_failures \
         (id,email_id,batch_id,member_id,email_recipient_id,code,enhanced_code,message,severity,event_id,failed_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(ids::new_id()).bind(f.email_id).bind(f.batch_id).bind(f.member_id).bind(f.recipient_id)
    .bind(f.code).bind(f.enhanced_code).bind(f.message).bind(f.severity).bind(f.event_id)
    .bind(dates::now_db())
    .execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(())
}

/// Open a batch record before its provider request goes out, so a send that
/// dies mid-flight still says how far it got.
pub async fn create_batch(state:&AppState,email_id:&str,segment:&str,recipient_count:usize)->Result<String> {
    let id=ids::new_id();
    let now=dates::now_db();
    sqlx::query("INSERT INTO email_batches (id,email_id,status,member_segment,recipient_count,created_at,updated_at) VALUES (?,?,'pending',?,?,?,?)")
        .bind(&id).bind(email_id).bind(segment).bind(recipient_count as i64).bind(&now).bind(&now)
        .execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(id)
}

/// Close a batch record with what the provider said.
///
/// A batch that accepted nobody reads as failed even without a batch-level
/// error: a relay refusing every address one at a time is a failed batch,
/// and `submitted` of zero under a status of `submitted` would say otherwise.
pub async fn settle_batch(
    state:&AppState,batch_id:&str,provider_id:Option<&str>,submitted:usize,error:Option<&str>,
)->Result<()> {
    sqlx::query("UPDATE email_batches SET status=?,provider_id=?,submitted_count=?,error_message=?,updated_at=? WHERE id=?")
        .bind(if error.is_none() && submitted>0 {"submitted"} else {"failed"})
        .bind(provider_id).bind(submitted as i64).bind(error).bind(dates::now_db()).bind(batch_id)
        .execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(())
}

/// Count an open against a recipient, once.
///
/// The first open is the one that counts — a mail client that fetches the
/// pixel on every scroll, or a provider that reports the same open twice,
/// must not move the numbers. `opened_at IS NULL` in the UPDATE is what makes
/// that true even when two requests land at the same moment: whichever writes
/// first is the one whose `rows_affected` is non-zero.
pub async fn record_open(state:&AppState,email_id:&str,member_uuid:&str)->Result<bool> {
    let now=dates::now_db();
    let mut tx=state.db.begin().await.map_err(|e|GhostError::Db(e.to_string()))?;
    let updated=sqlx::query("UPDATE email_recipients SET opened_at=?,updated_at=? WHERE email_id=? AND member_uuid=? AND opened_at IS NULL")
        .bind(&now).bind(&now).bind(email_id).bind(member_uuid).execute(&mut *tx).await
        .map_err(|e|GhostError::Db(e.to_string()))?;
    if updated.rows_affected()==0 {
        tx.commit().await.map_err(|e|GhostError::Db(e.to_string()))?;
        return Ok(false);
    }
    sqlx::query("UPDATE emails SET opened_count=opened_count+1,updated_at=? WHERE id=?")
        .bind(&now).bind(email_id).execute(&mut *tx).await.map_err(|e|GhostError::Db(e.to_string()))?;
    // The member's own counters, which the `email_opened_count` and
    // `email_open_rate` NQL filters read. The rate is only meaningful once
    // something has been sent, so it stays null until then.
    sqlx::query(
        "UPDATE members SET email_opened_count=email_opened_count+1, \
         email_open_rate=CASE WHEN email_count>0 THEN CAST(ROUND((email_opened_count+1)*100.0/email_count) AS INTEGER) ELSE NULL END, \
         updated_at=? WHERE uuid=?",
    )
    .bind(&now).bind(member_uuid).execute(&mut *tx).await.map_err(|e|GhostError::Db(e.to_string()))?;
    tx.commit().await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(true)
}

/// Mark a recipient delivered, from a provider that reports deliveries.
async fn record_delivery(state:&AppState,provider_id:&str)->bool {
    let now=dates::now_db();
    let updated=sqlx::query("UPDATE email_recipients SET delivered_at=?,updated_at=? WHERE mailgun_message_id=? AND delivered_at IS NULL")
        .bind(&now).bind(&now).bind(provider_id).execute(&state.db).await;
    matches!(updated,Ok(result) if result.rows_affected()>0)
}

/// The recipient a provider event is about.
struct EventTarget {
    recipient_id:String,
    email_id:String,
    batch_id:Option<String>,
    member_id:Option<String>,
    member_uuid:Option<String>,
}

async fn recipient_of(state:&AppState,provider_id:&str)->Option<EventTarget> {
    sqlx::query_as::<_,(String,String,Option<String>,Option<String>,Option<String>)>(
        "SELECT id,email_id,batch_id,member_id,member_uuid FROM email_recipients WHERE mailgun_message_id=?",
    )
    .bind(provider_id).fetch_optional(&state.db).await.ok().flatten()
    .map(|(recipient_id,email_id,batch_id,member_id,member_uuid)|EventTarget{recipient_id,email_id,batch_id,member_id,member_uuid})
}

/// A 1×1 transparent GIF — the smallest thing that can be an `<img>`.
pub(crate) const PIXEL:&[u8]=&[
    0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,0x00,0x00,0x00,
    0xff,0xff,0xff,0x21,0xf9,0x04,0x01,0x00,0x00,0x00,0x00,0x2c,0x00,0x00,0x00,0x00,
    0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,0x3b,
];

/// The open-tracking pixel.
///
/// Ghost gets opens from Mailgun, which injects its own pixel and reports the
/// result. This port also sends over SMTP and direct-to-MX, where there is no
/// provider to ask, so it serves the pixel itself — and the Mailgun webhook
/// below feeds the same counter, so a Mailgun site gets provider-grade data
/// without double-counting.
///
/// The image is always returned, whatever the request turns out to be about.
/// A broken-image icon in the middle of a newsletter would be a worse bug
/// than a missing statistic.
async fn open_pixel(State(state):State<AppState>,Path(email_id):Path<String>,Query(params):Query<HashMap<String,String>>)->Response {
    let pixel=|| ([("content-type","image/gif"),("cache-control","no-store, private, max-age=0")],PIXEL).into_response();
    if !state.settings.get_bool("email_track_opens") { return pixel(); }
    let (Some(uuid),Some(key))=(params.get("m"),params.get("k")) else { return pixel() };
    // The same uuid + HMAC pair the unsubscribe link carries, so an open
    // cannot be fabricated for a member whose uuid someone guessed.
    if !crate::members::secure_eq(&crate::members::newsletter_key(&state,uuid),key) { return pixel(); }
    let _=record_open(&state,&email_id,uuid).await;
    pixel()
}

/// Add the open pixel to a rendered body. `%%{uuid}%%` and `%%{key}%%` are
/// filled in per recipient by `personalize`, the same as the links above.
pub fn track_opens(state:&AppState,email_id:&str,html:&str)->String {
    if !state.settings.get_bool("email_track_opens") { return html.into(); }
    let pixel=format!(
        r#"<img src="{site}/r/o/{email_id}/?m=%%{{uuid}}%%&amp;k=%%{{key}}%%" width="1" height="1" alt="" border="0" style="display:block;width:1px;height:1px;border:0;outline:none">"#,
        site=state.site_url(),
    );
    // Inside `</body>`, so the pixel is part of the document a mail client
    // renders rather than trailing markup it may discard.
    match html.rfind("</body>") {
        Some(at)=>format!("{}{pixel}{}",&html[..at],&html[at..]),
        None=>format!("{html}{pixel}"),
    }
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
