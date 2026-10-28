//! Staff permissions shared by Admin API handlers.
use crate::AppState;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use ghost_core::models::{Post, User};
use ghost_db::repos;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Contributor,
    Author,
    Editor,
    Administrator,
    Owner,
}

pub fn denied() -> Response {
    (StatusCode::FORBIDDEN, Json(json!({"errors":[{"message":"You do not have permission to perform this action.","errorType":"NoPermissionError"}]}))).into_response()
}

pub async fn role(state: &AppState, user: &User) -> Result<Role, Response> {
    if user.status.as_deref() != Some("active") {
        return Err(denied());
    }
    match repos::users::role_name(&state.db, &user.id)
        .await
        .ok()
        .flatten()
        .as_deref()
    {
        Some("Owner") => Ok(Role::Owner),
        Some("Administrator") => Ok(Role::Administrator),
        Some("Editor" | "Super Editor") => Ok(Role::Editor),
        Some("Author") => Ok(Role::Author),
        Some("Contributor") => Ok(Role::Contributor),
        _ => Err(denied()),
    }
}

pub async fn require(
    state: &AppState,
    headers: &HeaderMap,
    minimum: Role,
) -> Result<User, Response> {
    let user = crate::admin::current_user(state, headers).await.ok_or_else(||
        (StatusCode::UNAUTHORIZED, Json(json!({"errors":[{"message":"Authentication required","errorType":"NoPermissionError"}]}))).into_response())?;
    if role(state, &user).await? < minimum {
        return Err(denied());
    }
    Ok(user)
}

pub async fn post_access(state: &AppState, user: &User, post: &Post) -> Result<(), Response> {
    if role(state, user).await? >= Role::Editor {
        return Ok(());
    }
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM posts_authors WHERE post_id = ? AND author_id = ?)",
    )
    .bind(&post.id)
    .bind(&user.id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);
    if !owns {
        return Err(denied());
    }
    Ok(())
}

pub async fn post_input(
    state: &AppState,
    user: &User,
    input: &Value,
    existing: Option<&Post>,
) -> Result<(), Response> {
    let role = role(state, user).await?;
    if role == Role::Contributor {
        let status = input
            .get("status")
            .and_then(Value::as_str)
            .or_else(|| existing.map(|p| p.status.as_str()))
            .unwrap_or("draft");
        if status != "draft" || existing.is_some_and(|p| p.status != "draft") {
            return Err(denied());
        }
    }
    if role < Role::Editor {
        if let Some(authors) = input.get("authors") {
            let Some(authors) = authors.as_array() else {
                return Err(denied());
            };
            let requested: Vec<_> = authors
                .iter()
                .filter_map(|a| a.get("id").and_then(Value::as_str))
                .collect();
            let current = if let Some(post) = existing {
                sqlx::query_scalar::<_, String>(
                    "SELECT author_id FROM posts_authors WHERE post_id = ? ORDER BY sort_order",
                )
                .bind(&post.id)
                .fetch_all(&state.db)
                .await
                .unwrap_or_default()
            } else {
                vec![user.id.clone()]
            };
            if requested.len() != authors.len()
                || requested != current.iter().map(String::as_str).collect::<Vec<_>>()
            {
                return Err(denied());
            }
        }
        for key in ["codeinjection_head", "codeinjection_foot"] {
            if let Some(value) = input.get(key) {
                let old = existing.and_then(|p| {
                    if key == "codeinjection_head" {
                        p.codeinjection_head.as_deref()
                    } else {
                        p.codeinjection_foot.as_deref()
                    }
                });
                if value.as_str().filter(|s| !s.is_empty()) != old.filter(|s| !s.is_empty()) {
                    return Err(denied());
                }
            }
        }
    }
    Ok(())
}

pub async fn edit_user(
    state: &AppState,
    actor: &User,
    target: &User,
    input: &Value,
) -> Result<(), Response> {
    let actor_role = role(state, actor).await?;
    let target_role = role(state, target).await?;
    if actor.id != target.id
        && !(actor_role == Role::Owner
            || (actor_role >= Role::Administrator && target_role < Role::Owner)
            || (actor_role == Role::Editor && target_role < Role::Editor))
    {
        return Err(denied());
    }
    if input.get("roles").is_some() || (input.get("status").is_some() && actor_role < Role::Editor)
    {
        return Err(denied());
    }
    Ok(())
}
