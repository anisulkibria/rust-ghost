//! The admin notification centre.
//!
//! Ghost keeps these in the `notifications` setting rather than a table —
//! there are only ever a handful, they are per-install rather than per-user,
//! and they are written by the system to tell staff something they have to
//! act on: a failed send, an expired Stripe key, a version worth upgrading
//! to. This does the same, so the shape on disk matches.
//!
//! Dismissal is per user. Ghost tracks it by adding the user's id to the
//! notification's `seen_by` list, which is what lets one administrator
//! dismiss a notice without hiding it from the others.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Value};

pub fn admin_router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            .route(&format!("/notifications{suffix}"), get(browse).post(add))
            .route(&format!("/notifications/{{id}}{suffix}"), axum::routing::delete(dismiss));
    }
    router
}

/// How many are kept. Ghost caps the setting too: it is a notice board, not
/// a log, and the setting has a size limit.
const MAX_STORED: usize = 50;

async fn stored(state: &AppState) -> Vec<Value> {
    // `get` hands back the stored JSON; an install that has never had one
    // has no setting at all, which is an empty board rather than an error.
    match state.settings.get("notifications") {
        Some(Value::Array(all)) => all,
        // A string is how the setting reads if it was ever written as one.
        Some(Value::String(raw)) => {
            serde_json::from_str::<Value>(&raw).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn save(state: &AppState, notifications: Vec<Value>) -> Result<()> {
    let trimmed: Vec<Value> = notifications.into_iter().take(MAX_STORED).collect();
    state
        .settings
        .set(&state.db, "notifications", &json!(trimmed))
        .await
        .map_err(|e| GhostError::Internal(e.message()))
}

/// Raise a notification from inside the server.
///
/// `id_hint` makes a notice idempotent: raising "your Stripe key expired"
/// every minute should leave one notification, not a thousand. A repeat
/// refreshes the existing one rather than adding to it.
pub async fn notify(state: &AppState, id_hint: &str, status: &str, message: &str) -> Result<()> {
    let mut all = stored(state).await;
    let now = dates::now_db();
    if let Some(existing) = all.iter_mut().find(|n| n.get("custom_id").and_then(Value::as_str) == Some(id_hint)) {
        existing["message"] = json!(message);
        existing["status"] = json!(status);
        existing["created_at"] = json!(now);
        // A notice that has come back is worth seeing again.
        existing["seen_by"] = json!([]);
        return save(state, all).await;
    }
    all.insert(
        0,
        json!({
            "id": ids::new_id(),
            "custom_id": id_hint,
            "status": status,
            "type": "warn",
            "message": message,
            "dismissible": true,
            "seen_by": [],
            "created_at": now,
        }),
    );
    save(state, all).await
}

/// `GET /notifications` — what this user has not dismissed.
async fn browse(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user = match permissions::require(&state, &headers, Role::Author).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    let visible: Vec<Value> = stored(&state)
        .await
        .into_iter()
        .filter(|n| {
            !n.get("seen_by")
                .and_then(Value::as_array)
                .is_some_and(|seen| seen.iter().any(|id| id.as_str() == Some(user.id.as_str())))
        })
        .collect();
    ([("cache-control", "private, no-store")], Json(json!({"notifications": visible}))).into_response()
}

/// `POST /notifications` — Ghost lets an integration raise one.
async fn add(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let incoming = body.get("notifications").and_then(Value::as_array).cloned().unwrap_or_default();
    if incoming.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors":[{"message":"At least one notification is required.","errorType":"ValidationError"}]})),
        )
            .into_response();
    }
    let mut all = stored(&state).await;
    let mut added = Vec::new();
    for notification in incoming {
        let message = notification.get("message").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if message.is_empty() {
            continue;
        }
        let created = json!({
            "id": ids::new_id(),
            "custom_id": notification.get("custom_id").and_then(Value::as_str),
            "status": notification.get("status").and_then(Value::as_str).unwrap_or("alert"),
            "type": notification.get("type").and_then(Value::as_str).unwrap_or("info"),
            "message": message,
            "dismissible": notification.get("dismissible").and_then(Value::as_bool).unwrap_or(true),
            "seen_by": [],
            "created_at": dates::now_db(),
        });
        all.insert(0, created.clone());
        added.push(created);
    }
    if added.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors":[{"message":"A notification needs a message.","errorType":"ValidationError"}]})),
        )
            .into_response();
    }
    if save(&state, all).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    (StatusCode::CREATED, Json(json!({"notifications": added}))).into_response()
}

/// `DELETE /notifications/{id}` — dismiss, for this user only.
///
/// `all` removes it outright, which is what Ghost's "dismiss all" does.
async fn dismiss(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match permissions::require(&state, &headers, Role::Author).await {
        Ok(user) => user,
        Err(r) => return r,
    };
    let mut all = stored(&state).await;
    if id == "all" {
        for notification in all.iter_mut() {
            mark_seen(notification, &user.id);
        }
    } else {
        let found = all.iter().any(|n| n.get("id").and_then(Value::as_str) == Some(id.as_str()));
        if !found {
            return StatusCode::NOT_FOUND.into_response();
        }
        for notification in all.iter_mut() {
            if notification.get("id").and_then(Value::as_str) == Some(id.as_str()) {
                mark_seen(notification, &user.id);
            }
        }
    }
    if save(&state, all).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Add a user to a notification's `seen_by`, once.
fn mark_seen(notification: &mut Value, user_id: &str) {
    let Some(object) = notification.as_object_mut() else { return };
    let seen = object.entry("seen_by").or_insert_with(|| json!([]));
    if let Some(list) = seen.as_array_mut() {
        if !list.iter().any(|existing| existing.as_str() == Some(user_id)) {
            list.push(json!(user_id));
        }
    }
}
