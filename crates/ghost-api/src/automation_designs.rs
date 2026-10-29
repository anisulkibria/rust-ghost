//! Immutable per-email designs: runs retain their original revision's design.
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
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/automations/{automation}/actions/{action}/design/",
            get(read).put(edit),
        )
        .route("/email-design-settings/{id}/", get(read_setting))
}
fn error(error: GhostError) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":error.message(),"errorType":"ValidationError"}]})),
    )
        .into_response()
}
fn bad(message: &str) -> GhostError {
    GhostError::BadRequest(message.into())
}
fn db(error: sqlx::Error) -> GhostError {
    GhostError::Db(error.to_string())
}
const COLORS: &[&str] = &[
    "background_color",
    "header_background_color",
    "post_title_color",
    "section_title_color",
    "divider_color",
    "link_color",
    "button_color",
];
const TEXT: &[&str] = &[
    "title_font_category",
    "body_font_category",
    "title_alignment",
    "title_font_weight",
    "link_style",
    "button_style",
    "button_corners",
    "image_corners",
    "footer_content",
    "sender_name",
    "sender_email",
    "sender_reply_to",
];
const FLAGS: &[&str] = &[
    "show_header_icon",
    "show_header_title",
    "show_header_name",
    "show_badge",
    "show_subscription_details",
];
pub(crate) fn validate(settings: &Value) -> Result<()> {
    let Some(settings) = settings.as_object() else {
        return Err(bad("Email design must be an object"));
    };
    for (key, value) in settings {
        if COLORS.contains(&key.as_str()) {
            let Some(value) = value.as_str() else {
                return Err(bad("Invalid email color"));
            };
            if !ghost_mail::newsletter::is_hex_color(value)
                && !matches!(value, "light" | "dark" | "transparent" | "accent")
            {
                return Err(bad("Use a hex color or a supported theme color"));
            }
        } else if FLAGS.contains(&key.as_str()) {
            if !value.is_boolean() {
                return Err(bad("Invalid email design toggle"));
            }
        } else if TEXT.contains(&key.as_str()) {
            let Some(value) = value.as_str().filter(|s| s.len() <= 5000) else {
                return Err(bad("Invalid email design text"));
            };
            let valid = match key.as_str() {
                "title_font_category" | "body_font_category" => {
                    matches!(value, "serif" | "sans_serif")
                }
                "title_alignment" => matches!(value, "left" | "center" | "right"),
                "title_font_weight" => matches!(value, "normal" | "bold"),
                "link_style" => matches!(value, "regular" | "underline"),
                "button_style" => matches!(value, "fill" | "outline" | "solid"),
                "button_corners" | "image_corners" => matches!(value, "square" | "rounded"),
                _ => true,
            };
            if !valid {
                return Err(bad("Unsupported email design option"));
            }
        } else {
            return Err(bad(&format!("Unknown email design field: {key}")));
        }
    }
    Ok(())
}
pub async fn apply(
    state: &AppState,
    newsletter: ghost_db::repos::newsletters::Newsletter,
    id: Option<&str>,
) -> Result<ghost_db::repos::newsletters::Newsletter> {
    let Some(id) = id else {
        if let Some(settings)=state.settings.get("automated_email_design") {
            validate(&settings)?;
            let mut value=serde_json::to_value(newsletter).map_err(|e|GhostError::Internal(e.to_string()))?;
            for (key,val) in settings.as_object().unwrap(){value[key]=val.clone();}
            return serde_json::from_value(value).map_err(|e|GhostError::Internal(e.to_string()));
        }
        return Ok(newsletter)
    };
    let settings: Option<String> =
        sqlx::query_scalar("SELECT settings FROM email_design_settings WHERE id=?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(db)?;
    let Some(settings) = settings else {
        return Err(bad("Email design no longer exists"));
    };
    let settings: Value =
        serde_json::from_str(&settings).map_err(|_| bad("Invalid saved email design"))?;
    validate(&settings)?;
    let mut value =
        serde_json::to_value(newsletter).map_err(|e| GhostError::Internal(e.to_string()))?;
    for (key, val) in settings.as_object().unwrap() {
        value[key] = val.clone()
    }
    serde_json::from_value(value).map_err(|e| GhostError::Internal(e.to_string()))
}
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    automation: &str,
    action: &str,
) -> std::result::Result<(), Response> {
    permissions::require(state, headers, Role::Administrator).await?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM automation_actions WHERE id=? AND automation_id=? AND deleted_at IS NULL AND type='send_email')").bind(action).bind(automation).fetch_one(&state.db).await.unwrap_or(false);
    if exists {
        Ok(())
    } else {
        Err(StatusCode::NOT_FOUND.into_response())
    }
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((automation, action)): Path<(String, String)>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, &automation, &action).await {
        return r;
    }
    let id:Option<String>=sqlx::query_scalar("SELECT email_design_setting_id FROM automation_action_revisions WHERE action_id=? ORDER BY created_at DESC LIMIT 1").bind(&action).fetch_one(&state.db).await.ok().flatten();
    let default = match ghost_db::repos::newsletters::default_newsletter(&state.db).await {
        Ok(Some(value)) => value,
        _ => return error(bad("Create a newsletter first")),
    };
    match apply(&state, default, id.as_deref()).await {
        Ok(newsletter) => {
            let value = serde_json::to_value(newsletter).unwrap();
            let mut settings = serde_json::Map::new();
            for key in COLORS.iter().chain(TEXT).chain(FLAGS) {
                if let Some(value) = value.get(*key) {
                    if !value.is_null() {
                        settings.insert((*key).into(), value.clone());
                    }
                }
            }
            Json(json!({"design":{"id":id,"settings":settings}})).into_response()
        }
        Err(e) => error(e),
    }
}
async fn read_setting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Administrator).await {
        return r;
    }
    match sqlx::query_scalar::<_,String>("SELECT settings FROM email_design_settings WHERE id=?").bind(&id).fetch_optional(&state.db).await{Ok(Some(settings))=>Json(json!({"email_design_settings":[{"id":id,"settings":serde_json::from_str::<Value>(&settings).unwrap_or(Value::Null)}]})).into_response(),Ok(None)=>StatusCode::NOT_FOUND.into_response(),Err(e)=>error(db(e))}
}
async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((automation, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, &automation, &action).await {
        return r;
    }
    let settings = body.get("settings").unwrap_or(&body);
    if let Err(e) = validate(settings) {
        return error(e);
    }
    let base = match ghost_db::repos::newsletters::default_newsletter(&state.db).await {
        Ok(Some(base)) => base,
        _ => return error(bad("Create a newsletter first")),
    };
    let existing:Option<String>=sqlx::query_scalar("SELECT email_design_setting_id FROM automation_action_revisions WHERE action_id=? ORDER BY created_at DESC LIMIT 1").bind(&action).fetch_one(&state.db).await.ok().flatten();
    let base = match apply(&state, base, existing.as_deref()).await {
        Ok(base) => base,
        Err(e) => return error(e),
    };
    let base = serde_json::to_value(base).unwrap();
    let mut values = serde_json::Map::new();
    for key in COLORS.iter().chain(TEXT).chain(FLAGS) {
        if let Some(value) = base.get(*key).filter(|v| !v.is_null()) {
            values.insert((*key).to_owned(), value.clone());
        }
    }
    for (key, value) in settings.as_object().unwrap() {
        values.insert(key.clone(), value.clone());
    }
    let settings = Value::Object(values);
    if let Err(e) = validate(&settings) {
        return error(e);
    }
    let id = ids::new_id();
    let now = dates::now_db();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return error(db(e)),
    };
    if let Err(e) = sqlx::query("UPDATE automation_actions SET updated_at=? WHERE id=?")
        .bind(&now)
        .bind(&action)
        .execute(&mut *tx)
        .await
    {
        return error(db(e));
    }
    let latest:Option<(String,String)>=sqlx::query_as("SELECT id,created_at FROM automation_action_revisions WHERE action_id=? ORDER BY created_at DESC LIMIT 1").bind(&action).fetch_optional(&mut *tx).await.ok().flatten();
    let Some((revision, latest_at)) = latest else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let at = if now > latest_at {
        now.clone()
    } else {
        dates::from_db_string(&latest_at)
            .map(|time| dates::to_db_string(&(time + chrono::Duration::seconds(1))))
            .unwrap_or(now)
    };
    if let Err(e) =
        sqlx::query("INSERT INTO email_design_settings (id,settings,created_at) VALUES (?,?,?)")
            .bind(&id)
            .bind(settings.to_string())
            .bind(&at)
            .execute(&mut *tx)
            .await
    {
        return error(db(e));
    }
    if let Err(e)=sqlx::query("INSERT INTO automation_action_revisions (id,created_at,action_id,wait_hours,email_subject,email_lexical,email_design_setting_id,email_sent_count,email_opened_count,email_clicked_count) SELECT ?,?,action_id,wait_hours,email_subject,email_lexical,?,0,0,0 FROM automation_action_revisions WHERE id=?")
        .bind(ids::new_id()).bind(at).bind(&id).bind(revision).execute(&mut *tx).await{return error(db(e))}
    if let Err(e) = tx.commit().await {
        return error(db(e));
    }
    Json(json!({"design":{"id":id,"settings":settings}})).into_response()
}

pub(crate) fn design_values(newsletter:&ghost_db::repos::newsletters::Newsletter)->Value{
    let source=serde_json::to_value(newsletter).unwrap_or(Value::Null);
    let mut fields=serde_json::Map::new();
    for key in COLORS.iter().chain(TEXT).chain(FLAGS){if let Some(value)=source.get(*key).filter(|v|!v.is_null()){fields.insert((*key).into(),value.clone());}}
    Value::Object(fields)
}
