//! Persistent, bounded Mailgun analytics reconciliation jobs.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}
pub fn router() -> Router<AppState> {
    let mut r = Router::new();
    for s in ["", "/"] {
        r = r
            .route(
                &format!("/emails/{{id}}/analytics{s}"),
                get(status).put(schedule),
            )
            .route(&format!("/emails/analytics{s}"), delete(cancel))
            .route(&format!("/tinybird/token{s}"), get(tinybird_token));
    }
    r
}
async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_id): Path<String>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    match sqlx::query_as::<_,(String,String,String,Option<String>)>("SELECT id,email_id,status,error FROM analytics_jobs ORDER BY updated_at DESC LIMIT 100").fetch_all(&state.db).await{
        Ok(rows)=>Json(json!({"status":rows.iter().any(|r|matches!(r.2.as_str(),"pending"|"running")),"jobs":rows.into_iter().map(|(id,email,status,error)|json!({"id":id,"email_id":email,"status":status,"error":error})).collect::<Vec<_>>()})).into_response(),Err(_)=>StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
async fn schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let created = sqlx::query_scalar::<_, String>("SELECT created_at FROM emails WHERE id=?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(created) = created else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let now = chrono::Utc::now();
    let start = q
        .get("begin")
        .and_then(|v| dates::from_api_string(v))
        .or_else(|| dates::from_db_string(&created))
        .unwrap_or(now - chrono::Duration::days(7));
    let end = q
        .get("end")
        .and_then(|v| dates::from_api_string(v))
        .unwrap_or(now);
    if start >= end || end > now || end - start > chrono::Duration::days(365) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors":[{"message":"Choose a past date range of at most one year"}]})),
        )
            .into_response();
    }
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.site_name(),
        &state.site_url(),
    );
    if ghost_mail::MailgunConfig::from_settings(&state.settings, &cfg).is_none() {
        return (StatusCode::CONFLICT,Json(json!({"errors":[{"message":"Mailgun is required to resync provider events. Local tracking is already recorded."}]}))).into_response();
    }
    match sqlx::query(
        "INSERT INTO analytics_jobs(id,email_id,begin_at,end_at,updated_at) VALUES(?,?,?,?,?)",
    )
    .bind(ids::new_id())
    .bind(&id)
    .bind(start.timestamp().to_string())
    .bind(end.timestamp().to_string())
    .bind(dates::now_db())
    .execute(&state.db)
    .await
    {
        Ok(_) => status(State(state), headers, Path(id)).await,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn cancel(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    match sqlx::query("UPDATE analytics_jobs SET status='cancelled',updated_at=? WHERE status IN ('pending','running')").bind(dates::now_db()).execute(&state.db).await{Ok(_)=>StatusCode::NO_CONTENT.into_response(),Err(_)=>StatusCode::INTERNAL_SERVER_ERROR.into_response()}
}
async fn tinybird_token(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let config = &state.config.host_settings["analytics"]["tinybird"];
    // Only a read-scoped frontend token belongs here, never an ingestion/admin credential.
    let token = config["readToken"].as_str();
    let exp = config["expiresAt"].as_str();
    (
        [("cache-control", "private,no-store")],
        Json(match (token, exp) {
            (Some(token), Some(exp))
                if dates::from_api_string(exp).is_some_and(|t| t > chrono::Utc::now()) =>
            {
                json!({"token":token,"exp":exp})
            }
            _ => Value::Null,
        }),
    )
        .into_response()
}
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = poll(&state).await {
                tracing::warn!("Email analytics: {}", e.message())
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}
pub async fn poll(state: &AppState) -> Result<()> {
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.site_name(),
        &state.site_url(),
    );
    let Some(mg) = ghost_mail::MailgunConfig::from_settings(&state.settings, &cfg) else {
        return Ok(());
    };
    let row=sqlx::query_as::<_,(String,String,String,String,Option<String>)>("UPDATE analytics_jobs SET status='running',updated_at=?,attempts=attempts+1 WHERE id=(SELECT id FROM analytics_jobs WHERE (status='pending' OR (status='running' AND datetime(updated_at)<datetime('now','-5 minutes'))) AND attempts<12 ORDER BY updated_at LIMIT 1) RETURNING id,email_id,begin_at,end_at,cursor").bind(dates::now_db()).fetch_optional(&state.db).await.map_err(db)?;
    let Some((id, email, begin, end, cursor)) = row else {
        return Ok(());
    };
    let result = reconcile(state, &mg, &email, &begin, &end, cursor.as_deref()).await;
    match result {
        Ok(next) => {
            sqlx::query("UPDATE analytics_jobs SET status=?,cursor=?,error=NULL,updated_at=?,attempts=0 WHERE id=? AND status='running'").bind(if next.is_some(){"pending"}else{"completed"}).bind(next).bind(dates::now_db()).bind(id).execute(&state.db).await.map_err(db)?;
        }
        Err(e) => {
            sqlx::query("UPDATE analytics_jobs SET status='failed',error=?,updated_at=? WHERE id=? AND status='running'").bind(e.message()).bind(dates::now_db()).bind(id).execute(&state.db).await.map_err(db)?;
        }
    }
    Ok(())
}
async fn reconcile(
    state: &AppState,
    mg: &ghost_mail::MailgunConfig,
    email: &str,
    begin: &str,
    end: &str,
    cursor: Option<&str>,
) -> Result<Option<String>> {
    let endpoint = format!(
        "{}/{}/events",
        mg.base_url.trim_end_matches('/'),
        percent_encoding::utf8_percent_encode(&mg.domain, percent_encoding::NON_ALPHANUMERIC)
    );
    let base = reqwest::Url::parse(&endpoint).map_err(|e| GhostError::BadRequest(e.to_string()))?;
    let url = cursor
        .and_then(|u| reqwest::Url::parse(u).ok())
        .filter(|u| u.origin() == base.origin() && u.path().starts_with(base.path()))
        .unwrap_or(base);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| GhostError::Internal(e.to_string()))?;
    let mut request = client.get(url).basic_auth("api", Some(&mg.api_key));
    if cursor.is_none() {
        request = request.query(&[
            ("begin", begin),
            ("end", end),
            ("ascending", "yes"),
            ("limit", "300"),
        ]);
    }
    let response = request
        .send()
        .await
        .map_err(|e| GhostError::EmailError(e.to_string()))?;
    if !response.status().is_success() {
        return Err(GhostError::EmailError(format!(
            "Provider analytics returned {}",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|e| GhostError::EmailError(e.to_string()))?;
    let items = value["items"].as_array().cloned().unwrap_or_default();
    for event in &items {
        let provider = event
            .pointer("/message/headers/message-id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_matches(|c| c == '<' || c == '>');
        let belongs:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM email_recipients WHERE email_id=? AND mailgun_message_id=?)").bind(email).bind(provider).fetch_one(&state.db).await.map_err(db)?;
        if belongs {
            let response = crate::newsletter_events::ingest_provider_event(state, event).await;
            if !response.status().is_success() {
                return Err(GhostError::EmailError(
                    "Could not store provider event".into(),
                ));
            }
        }
    }
    Ok(if items.is_empty() {
        None
    } else {
        value
            .pointer("/paging/next")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}
