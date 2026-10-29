//! Compatibility facade for Ghost's single welcome-email resources.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use ghost_core::{
    automations::{Action, ActionKind, Status, TriggerScope},
    ids,
};
use ghost_db::repos::automations as repo;
use serde_json::{json, Value};
pub fn router() -> Router<AppState> {
    let mut r = Router::new();
    for s in ["", "/"] {
        r = r
            .route(&format!("/automated_emails{s}"), get(browse).post(add))
            .route(
                &format!("/automated_emails/design{s}"),
                get(design).put(save_design),
            )
            .route(&format!("/automated_emails/senders{s}"), put(senders))
            .route(
                &format!("/automated_emails/verifications{s}"),
                put(crate::compat::verify_email),
            )
            .route(&format!("/automated_emails/{{id}}{s}"), get(read).put(edit))
            .route(
                &format!("/automated_emails/{{id}}/preview{s}"),
                post(preview),
            )
            .route(&format!("/automated_emails/{{id}}/test{s}"), post(test))
            .route(
                &format!("/automations/{{id}}/email_preview{s}"),
                post(preview),
            );
    }
    r
}
fn bad(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
    )
        .into_response()
}
async fn value(state: &AppState, id: &str) -> Option<Value> {
    let a = repo::find(&state.db, id).await.ok().flatten()?;
    let email = a
        .actions
        .iter()
        .find(|a| matches!(a.kind, ActionKind::SendEmail { .. }));
    let (subject, lexical) = email
        .map(|a| match &a.kind {
            ActionKind::SendEmail { subject, lexical } => (subject.as_str(), lexical.as_str()),
            _ => unreachable!(),
        })
        .unwrap_or(("", ""));
    Some(
        json!({"id":a.summary.id,"name":a.summary.name,"slug":a.summary.slug,"status":a.summary.status,"subject":subject,"lexical":lexical,"email_design_setting_id":email.and_then(|a|a.email_design_setting_id.as_deref()),"sender_name":state.settings.get_str("automated_email_sender_name"),"sender_email":state.settings.get_str("automated_email_sender_email"),"sender_reply_to":state.settings.get_str("automated_email_sender_reply_to"),"created_at":a.summary.created_at,"updated_at":a.summary.updated_at}),
    )
}
async fn browse(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let rows = match repo::browse(&state.db, &["active", "inactive", "archived"]).await {
        Ok(rows) => rows,
        Err(e) => return crate::error_json(&e),
    };
    let mut emails = Vec::new();
    for row in rows
        .into_iter()
        .filter(|a| ghost_core::automations::is_welcome_email_slug(a.slug.as_deref()))
    {
        if let Some(v) = value(&state, &row.id).await {
            emails.push(v)
        }
    }
    Json(json!({"automated_emails":emails})).into_response()
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match value(&state, &id).await {
        Some(v) => Json(json!({"automated_emails":[v]})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let input = body.pointer("/automated_emails/0").unwrap_or(&body);
    let slug = input["slug"].as_str().unwrap_or("");
    if !ghost_core::automations::is_welcome_email_slug(Some(slug)) {
        return bad("Choose a free or paid welcome email slug");
    }
    let id = sqlx::query_scalar::<_, String>("SELECT id FROM automations WHERE slug=?")
        .bind(slug)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(id) = id else {
        return bad("The welcome automation has not been seeded");
    };
    edit(State(state), headers, Path(id), Json(body)).await
}
async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(a) = repo::find(&state.db, &id).await.ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let data = body.pointer("/automated_emails/0").unwrap_or(&body);
    if data.get("slug").is_some() && data["slug"].as_str() != a.summary.slug.as_deref() {
        return bad("The welcome email slug cannot be changed");
    }
    let existing = a
        .actions
        .iter()
        .find(|a| matches!(a.kind, ActionKind::SendEmail { .. }));
    let (subject, lexical) = existing
        .map(|a| match &a.kind {
            ActionKind::SendEmail { subject, lexical } => (subject.as_str(), lexical.as_str()),
            _ => unreachable!(),
        })
        .unwrap_or(("", ""));
    let subject = data["subject"].as_str().unwrap_or(subject);
    let lexical = data["lexical"].as_str().unwrap_or(lexical);
    if subject.len() > 300 || serde_json::from_str::<Value>(lexical).is_err() {
        return bad("A valid email subject and Lexical body are required");
    }
    let status = match Status::parse(data["status"].as_str().unwrap_or(&a.summary.status)) {
        Some(s) => s,
        None => return bad("Invalid status"),
    };
    let action = Action {
        id: existing.map(|a| a.id.clone()).unwrap_or_else(ids::new_id),
        kind: ActionKind::SendEmail {
            subject: subject.into(),
            lexical: lexical.into(),
        },
    };
    let input = repo::AutomationInput {
        name: data["name"].as_str().unwrap_or(&a.summary.name),
        description: &a.summary.description,
        status,
        trigger_tier_scope: a
            .summary
            .trigger_tier_scope
            .as_deref()
            .and_then(TriggerScope::parse),
        trigger_tier_ids: &a.trigger_tier_ids,
        actions: &[action],
        edges: &[],
    };
    if let Err(e) = repo::update(&state.db, &id, &input).await {
        return crate::error_json(&e);
    }
    read(State(state), headers, Path(id)).await
}
async fn design_data(state: &AppState) -> ghost_core::Result<Value> {
    let newsletter = ghost_db::repos::newsletters::default_newsletter(&state.db)
        .await?
        .ok_or_else(|| ghost_core::GhostError::BadRequest("Create a newsletter first".into()))?;
    let mut data = crate::automation_designs::design_values(&newsletter);
    if let Some(Value::Object(saved)) = state.settings.get("automated_email_design") {
        for (key, value) in saved {
            data[key] = value;
        }
    }
    data["id"] = json!("native-automated-email-design");
    Ok(data)
}
async fn design(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match design_data(&state).await {
        Ok(data) => Json(json!({"automated_email_design":[data]})).into_response(),
        Err(e) => crate::error_json(&e),
    }
}
async fn save_design(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let input = body.pointer("/automated_email_design/0").unwrap_or(&body);
    let Some(fields) = input.as_object() else {
        return bad("Expected a design object");
    };
    let mut data = match design_data(&state).await {
        Ok(data) => data,
        Err(e) => return crate::error_json(&e),
    };
    data.as_object_mut().unwrap().remove("id");
    for (key, value) in fields {
        if key != "id" {
            data[key] = value.clone();
        }
    }
    if let Err(e) = crate::automation_designs::validate(&data) {
        return crate::error_json(&e);
    }
    match state
        .settings
        .set(&state.db, "automated_email_design", &data)
        .await
    {
        Ok(()) => design(State(state), headers).await,
        Err(e) => crate::error_json(&e),
    }
}
async fn senders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let data = body.pointer("/automated_emails/0").unwrap_or(&body);
    let mut pending = false;
    for field in ["sender_name", "sender_email", "sender_reply_to"] {
        if let Some(value) = data[field].as_str() {
            if field == "sender_name" {
                if value.len() > 191 || value.contains(['\r', '\n']) {
                    return bad("Invalid sender name");
                }
                if let Err(e) = state
                    .settings
                    .set(
                        &state.db,
                        &format!("automated_email_{field}"),
                        &json!(value),
                    )
                    .await
                {
                    return crate::error_json(&e);
                }
            } else {
                if let Err(e) = crate::compat::request_email_verification(
                    &state,
                    "automated",
                    "default",
                    field,
                    value,
                )
                .await
                {
                    return crate::error_json(&e);
                }
                pending = true;
            }
        }
    }
    Json(json!({"automated_emails":[{"verification_pending":pending}]})).into_response()
}
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(a) = repo::find(&state.db, &id).await.ok().flatten() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let email = a.actions.iter().find(|a| {
        matches!(a.kind, ActionKind::SendEmail { .. })
            && body["action_id"].as_str().is_none_or(|id| a.id == id)
    });
    let (saved_subject, saved_lexical) = email
        .map(|a| match &a.kind {
            ActionKind::SendEmail { subject, lexical } => (subject.as_str(), lexical.as_str()),
            _ => unreachable!(),
        })
        .unwrap_or(("", ""));
    let subject = body["subject"].as_str().unwrap_or(saved_subject);
    let lexical = body["lexical"].as_str().unwrap_or(saved_lexical);
    if subject.len() > 300 || serde_json::from_str::<Value>(lexical).is_err() {
        return bad("A valid subject and Lexical body are required");
    }
    let design = email.and_then(|a| a.email_design_setting_id.as_deref());
    match crate::automations_engine::preview_email(&state, subject, lexical, design).await {
        Ok(html) => {
            let text = ghost_mail::newsletter::plaintext(&html);
            Json(json!({"html":html,"text":text,"plaintext":text,"subject":subject,"email_preview":[{"html":html,"text":text,"plaintext":text,"subject":subject}]})).into_response()
        }
        Err(e) => bad(&e),
    }
}
async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let response=crate::automations::send_test_email(State(state), Path(id), headers, Json(body)).await;
    if response.status().is_success(){StatusCode::NO_CONTENT.into_response()}else{response}
}
