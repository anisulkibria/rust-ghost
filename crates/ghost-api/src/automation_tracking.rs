//! Per-recipient automation engagement with transactional deduplication.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::json;
use std::collections::HashMap;
fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/r/a/o/{token}", get(open))
        .route("/r/a/c/{link}/{token}", get(click))
}
pub fn admin_router() -> Router<AppState> {
    let mut r = Router::new();
    for s in ["", "/"] {
        r = r.route(
            &format!("/automations/{{automation}}/actions/{{action}}/links{s}"),
            get(links),
        );
    }
    r
}
pub async fn instrument(
    state: &AppState,
    revision: &str,
    uuid: &str,
    html: &str,
) -> Result<String> {
    instrument_for_send(state, revision, uuid, "", html).await
}
pub async fn instrument_for_send(
    state: &AppState,
    revision: &str,
    uuid: &str,
    send_id: &str,
    html: &str,
) -> Result<String> {
    if !state.settings.get_bool("email_track_opens")
        && !state.settings.get_bool("email_track_clicks")
    {
        return Ok(html.into());
    }
    let candidate = ids::random_hex(32);
    sqlx::query("INSERT INTO automation_tracking(token,revision_id,member_uuid,send_id) VALUES(?,?,?,?) ON CONFLICT(revision_id,member_uuid,send_id) DO NOTHING").bind(candidate).bind(revision).bind(uuid).bind(send_id).execute(&state.db).await.map_err(db)?;
    let token: String = sqlx::query_scalar(
        "SELECT token FROM automation_tracking WHERE revision_id=? AND member_uuid=? AND send_id=?",
    )
    .bind(revision)
    .bind(uuid)
    .bind(send_id)
    .fetch_one(&state.db)
    .await
    .map_err(db)?;
    let mut output = String::new();
    let mut previous = 0;
    if state.settings.get_bool("email_track_clicks") {
        let re = regex::Regex::new(r#"(?i)href\s*=\s*["'](https?://[^"'<>]+)["']"#).unwrap();
        for cap in re.captures_iter(html) {
            let m = cap.get(0).unwrap();
            let url = cap[1].replace("&amp;", "&");
            if url.contains("/unsubscribe/") || url.contains("%%{") {
                continue;
            }
            sqlx::query("INSERT INTO automation_links(id,revision_id,url) VALUES(?,?,?) ON CONFLICT(revision_id,url) DO NOTHING").bind(ids::new_id()).bind(revision).bind(&url).execute(&state.db).await.map_err(db)?;
            let link: String =
                sqlx::query_scalar("SELECT id FROM automation_links WHERE revision_id=? AND url=?")
                    .bind(revision)
                    .bind(url)
                    .fetch_one(&state.db)
                    .await
                    .map_err(db)?;
            output.push_str(&html[previous..m.start()]);
            output.push_str(&format!(
                "href=\"{}/r/a/c/{link}/{token}\"",
                state.site_url()
            ));
            previous = m.end();
        }
    }
    output.push_str(&html[previous..]);
    if state.settings.get_bool("email_track_opens") {
        let pixel = format!(
            "<img src=\"{}/r/a/o/{token}\" width=\"1\" height=\"1\" alt=\"\">",
            state.site_url()
        );
        if let Some(at) = output.rfind("</body>") {
            output.insert_str(at, &pixel)
        } else {
            output.push_str(&pixel)
        }
    }
    Ok(output)
}
pub async fn record(state: &AppState, token: &str, link: Option<&str>) -> Result<bool> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let field = if link.is_some() {
        "clicked_at"
    } else {
        "opened_at"
    };
    if let Some(link) = link {
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM automation_links l JOIN automation_tracking t ON t.revision_id=l.revision_id WHERE l.id=? AND t.token=?)").bind(link).bind(token).fetch_one(&mut *tx).await.map_err(db)?;
        if !valid {
            return Ok(false);
        }
        sqlx::query(
            "INSERT OR IGNORE INTO automation_link_clicks(link_id,token,created_at) VALUES(?,?,?)",
        )
        .bind(link)
        .bind(token)
        .bind(dates::now_db())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    let revision=sqlx::query_scalar::<_,String>(&format!("UPDATE automation_tracking SET {field}=? WHERE token=? AND {field} IS NULL RETURNING revision_id")).bind(dates::now_db()).bind(token).fetch_optional(&mut *tx).await.map_err(db)?;
    if let Some(revision) = &revision {
        let counter = if link.is_some() {
            "email_clicked_count"
        } else {
            "email_opened_count"
        };
        sqlx::query(&format!(
            "UPDATE automation_action_revisions SET {counter}=COALESCE({counter},0)+1 WHERE id=?"
        ))
        .bind(revision)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(revision.is_some())
}
async fn open(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    if state.settings.get_bool("email_track_opens") {
        let _ = record(&state, &token, None).await;
    }
    (
        [
            ("content-type", "image/gif"),
            ("cache-control", "private,no-store"),
        ],
        crate::newsletter_events::PIXEL,
    )
        .into_response()
}
async fn click(
    State(state): State<AppState>,
    Path((link, token)): Path<(String, String)>,
) -> Response {
    let url=sqlx::query_scalar::<_,String>("SELECT l.url FROM automation_links l JOIN automation_tracking t ON t.revision_id=l.revision_id WHERE l.id=? AND t.token=?").bind(&link).bind(&token).fetch_optional(&state.db).await.ok().flatten();
    let Some(url) = url
        .filter(|u| reqwest::Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if state.settings.get_bool("email_track_clicks") {
        let _ = record(&state, &token, Some(&link)).await;
    }
    (
        [("cache-control", "private,no-store")],
        Redirect::temporary(&url),
    )
        .into_response()
}
async fn links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((automation, action)): Path<(String, String)>,
    Query(_q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let rows=sqlx::query_as::<_,(String,String,i64)>("SELECT l.id,l.url,COUNT(c.token) FROM automation_links l JOIN automation_action_revisions r ON r.id=l.revision_id JOIN automation_actions a ON a.id=r.action_id LEFT JOIN automation_link_clicks c ON c.link_id=l.id WHERE a.id=? AND a.automation_id=? GROUP BY l.id").bind(action).bind(automation).fetch_all(&state.db).await;
    match rows{Ok(rows)=>Json(json!({"links":rows.into_iter().map(|(id,url,clicks)|json!({"id":id,"to":url,"clicks":clicks})).collect::<Vec<_>>()})).into_response(),Err(_)=>StatusCode::INTERNAL_SERVER_ERROR.into_response()}
}
