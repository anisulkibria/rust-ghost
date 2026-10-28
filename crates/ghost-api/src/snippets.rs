//! Reusable editor content, using Ghost's Admin API snippet envelope.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use ghost_db::repos;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/snippets", get(browse).post(add))
        .route("/snippets/", get(browse).post(add))
        .route("/snippets/{id}", get(read).put(edit).delete(delete))
        .route("/snippets/{id}/", get(read).put(edit).delete(delete))
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct Snippet {
    id: String,
    name: String,
    mobiledoc: String,
    lexical: Option<String>,
    created_at: String,
    updated_at: Option<String>,
}

fn error(status: StatusCode, message: &str) -> Response {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "NoPermissionError",
        StatusCode::NOT_FOUND => "NotFoundError",
        StatusCode::UNPROCESSABLE_ENTITY => "ValidationError",
        _ => "InternalServerError",
    };
    (
        status,
        Json(json!({"errors":[{"message":message,"errorType":kind}]})),
    )
        .into_response()
}

fn db_error(err: sqlx::Error) -> Response {
    if err
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Snippet already exists.");
    }
    tracing::error!(error = %err, "Snippet operation failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not save or load snippets.",
    )
}

async fn authorize(state: &AppState, headers: &HeaderMap, write: bool) -> Result<bool, Response> {
    let user = crate::admin::current_user(state, headers)
        .await
        .filter(|user| user.status.as_deref() == Some("active"))
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Authentication required"))?;
    let role = repos::users::role_name(&state.db, &user.id)
        .await
        .unwrap_or_default()
        .unwrap_or_default();
    let can_manage = matches!(role.as_str(), "Owner" | "Administrator" | "Editor");
    if (!can_manage && !matches!(role.as_str(), "Author" | "Contributor")) || (write && !can_manage)
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "You do not have permission to manage snippets.",
        ));
    }
    Ok(can_manage)
}

fn formats(params: &HashMap<String, String>) -> Result<Vec<&str>, Response> {
    let formats: Vec<_> = params
        .get("formats")
        .map(String::as_str)
        .unwrap_or("mobiledoc")
        .split(',')
        .collect();
    if formats
        .iter()
        .any(|format| !matches!(*format, "lexical" | "mobiledoc"))
    {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Formats must be lexical or mobiledoc.",
        ));
    }
    Ok(formats)
}

fn serialized(snippet: Snippet, formats: &[&str]) -> Value {
    let mut value = serde_json::to_value(snippet).expect("snippet JSON");
    for key in ["mobiledoc", "lexical"] {
        if !formats.contains(&key) {
            value.as_object_mut().unwrap().remove(key);
        } else if let Some(text) = value[key].as_str() {
            value[key] = json!(ghost_core::url::base_assets(text));
        }
    }
    value
}

async fn find(state: &AppState, id: &str) -> Result<Snippet, Response> {
    sqlx::query_as::<_, Snippet>("SELECT * FROM snippets WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Snippet not found."))
}

async fn browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let can_manage = match authorize(&state, &headers, false).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let formats = match formats(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .unwrap_or(1)
        .clamp(1, 1_000_000);
    let limit = if params.get("limit").is_some_and(|v| v == "all") {
        -1
    } else {
        params
            .get("limit")
            .and_then(|p| p.parse::<i64>().ok())
            .unwrap_or(15)
            .clamp(1, 1000)
    };
    let total: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM snippets")
        .fetch_one(&state.db)
        .await
    {
        Ok(v) => v,
        Err(e) => return db_error(e),
    };
    let rows = match sqlx::query_as::<_, Snippet>(
        "SELECT * FROM snippets ORDER BY name COLLATE NOCASE, id LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(if limit < 0 { 0 } else { (page - 1) * limit })
    .fetch_all(&state.db)
    .await
    {
        Ok(v) => v,
        Err(e) => return db_error(e),
    };
    let pages = if limit < 0 {
        1
    } else {
        ((total + limit - 1) / limit).max(1)
    };
    Json(json!({"snippets":rows.into_iter().map(|row| serialized(row, &formats)).collect::<Vec<_>>(),
        "meta":{"can_manage":can_manage,"pagination":{"page":page,"pages":pages,"limit":if limit<0 {json!("all")} else {json!(limit)},"total":total,
        "next":if page<pages {Some(page+1)} else {None},"prev":if page>1 {Some(page-1)} else {None}}}})).into_response()
}

async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, false).await {
        return r;
    }
    let formats = match formats(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match find(&state, &id).await {
        Ok(row) => Json(json!({"snippets":[serialized(row, &formats)]})).into_response(),
        Err(r) => r,
    }
}

fn input(body: &Value, existing: Option<&Snippet>) -> Result<(String, String, String), Response> {
    let item = body
        .get("snippets")
        .and_then(Value::as_array)
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .filter(|item| item.is_object())
        .ok_or_else(|| {
            error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "One snippet is required in snippets[].",
            )
        })?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| existing.map(|s| s.name.as_str()))
        .unwrap_or("")
        .trim();
    if name.is_empty() || name.chars().count() > 191 {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Snippet name must be between 1 and 191 characters.",
        ));
    }
    let lexical = item
        .get("lexical")
        .and_then(Value::as_str)
        .or_else(|| existing.and_then(|s| s.lexical.as_deref()))
        .unwrap_or("");
    let document: Value = serde_json::from_str(lexical).map_err(|_| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Snippet content must be valid Lexical JSON.",
        )
    })?;
    if !document
        .get("nodes")
        .and_then(Value::as_array)
        .is_some_and(|nodes| {
            !nodes.is_empty()
                && nodes
                    .iter()
                    .all(|node| node.get("type").and_then(Value::as_str).is_some())
        })
    {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Select some text or a card to save as a snippet.",
        ));
    }
    let mobiledoc = item
        .get("mobiledoc")
        .and_then(Value::as_str)
        .or_else(|| existing.map(|s| s.mobiledoc.as_str()))
        .unwrap_or("{}");
    if serde_json::from_str::<Value>(mobiledoc).is_err() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Mobiledoc must be valid JSON.",
        ));
    }
    Ok((
        name.to_string(),
        ghost_core::url::unbase_assets(lexical),
        ghost_core::url::unbase_assets(mobiledoc),
    ))
}

async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, true).await {
        return r;
    }
    let formats = match formats(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (name, lexical, mobiledoc) = match input(&body, None) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    if let Err(e) = sqlx::query("INSERT INTO snippets (id, name, lexical, mobiledoc, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(name).bind(lexical).bind(mobiledoc).bind(&now).bind(&now).execute(&state.db).await { return db_error(e); }
    match find(&state, &id).await {
        Ok(row) => (
            StatusCode::CREATED,
            Json(json!({"snippets":[serialized(row, &formats)]})),
        )
            .into_response(),
        Err(r) => r,
    }
}

async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, true).await {
        return r;
    }
    let formats = match formats(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let existing = match find(&state, &id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (name, lexical, mobiledoc) = match input(&body, Some(&existing)) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(e) = sqlx::query(
        "UPDATE snippets SET name = ?, lexical = ?, mobiledoc = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(lexical)
    .bind(mobiledoc)
    .bind(ghost_core::dates::now_db())
    .bind(&id)
    .execute(&state.db)
    .await
    {
        return db_error(e);
    }
    match find(&state, &id).await {
        Ok(row) => Json(json!({"snippets":[serialized(row, &formats)]})).into_response(),
        Err(r) => r,
    }
}

async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, true).await {
        return r;
    }
    match sqlx::query("DELETE FROM snippets WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => error(StatusCode::NOT_FOUND, "Snippet not found."),
        Err(e) => db_error(e),
    }
}
