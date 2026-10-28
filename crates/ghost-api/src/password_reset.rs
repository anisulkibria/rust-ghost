//! Expiring, single-use staff password recovery. Stored tokens are hashed.
use crate::AppState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use ghost_core::{dates, ids};
use ghost_db::repos;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/authentication/password_reset/", post(request).put(reset))
        .route("/authentication/password_reset", post(request).put(reset))
}
fn error(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
    )
        .into_response()
}
async fn request(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let data = body.pointer("/password_reset/0").unwrap_or(&body);
    let email = data["email"].as_str().unwrap_or("").trim().to_lowercase();
    let user = repos::users::find_by_email(&state.db, &email)
        .await
        .ok()
        .flatten()
        .filter(|u| u.status.as_deref() == Some("active"));
    if let Some(user) = user {
        let recent:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM staff_password_resets WHERE user_id=? AND datetime(created_at)>datetime('now','-1 minute'))").bind(&user.id).fetch_one(&state.db).await.unwrap_or(true);
        if !recent {
            let token = ids::random_hex(32);
            let hash = hex::encode(Sha256::digest(token.as_bytes()));
            let now = dates::now_db();
            let expires = (chrono::Utc::now() + chrono::Duration::minutes(30))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let saved=sqlx::query("INSERT INTO staff_password_resets (token_hash,user_id,expires_at,created_at) VALUES (?,?,?,?)").bind(hash).bind(&user.id).bind(expires).bind(now).execute(&state.db).await.is_ok();
            if saved {
                let title = state.settings.site_name();
                let site = state.site_url();
                let link = format!("{site}/ghost/forgot-password/?token={token}");
                let html=format!("<p>Reset your password for {}.</p><p><a href=\"{}\">Choose a new password</a></p><p>This link expires in 30 minutes. If you did not request it, you can ignore this message.</p>",title.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;"),link);
                let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site);
                let msg = ghost_mail::Message {
                    to: email,
                    subject: format!("Reset your {title} password"),
                    html,
                    text: format!("Reset your password: {link}\nThis link expires in 30 minutes."),
                };
                if let Err(e) = ghost_mail::send(&cfg, &msg).await {
                    tracing::warn!("Password recovery email failed: {}", e.message());
                }
            }
        }
    }
    // Same response for unknown accounts and throttled requests.
    (StatusCode::CREATED,[(header::CACHE_CONTROL,"private, no-store")],Json(json!({"password_reset":[{"message":"If an active staff account uses this address, a password reset link has been sent."}]}))).into_response()
}
async fn reset(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let data = body.pointer("/password_reset/0").unwrap_or(&body);
    let token = data["token"].as_str().unwrap_or("");
    let password = data["newPassword"]
        .as_str()
        .or_else(|| data["password"].as_str())
        .unwrap_or("");
    if password.chars().count() < 10 || password.len() > 72 {
        return error("Use a password of at least 10 characters and at most 72 bytes.");
    }
    if token.len() != 64 {
        return error("This reset link is invalid or expired.");
    }
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    let password = match bcrypt::hash(password, bcrypt::DEFAULT_COST) {
        Ok(hash) => hash,
        Err(_) => return error("Could not set password"),
    };
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return error("Could not reset password"),
    };
    let user:Option<String>=sqlx::query_scalar("DELETE FROM staff_password_resets WHERE token_hash=? AND datetime(expires_at)>datetime('now') RETURNING user_id").bind(hash).fetch_optional(&mut *tx).await.ok().flatten();
    let Some(user) = user else {
        return error("This reset link is invalid or expired.");
    };
    let updated =
        sqlx::query("UPDATE users SET password=?,updated_at=? WHERE id=? AND status='active'")
            .bind(password)
            .bind(dates::now_db())
            .bind(&user)
            .execute(&mut *tx)
            .await;
    if !updated.is_ok_and(|r| r.rows_affected() == 1) {
        return error("This reset link is invalid or expired.");
    }
    if sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(&user)
        .execute(&mut *tx)
        .await
        .is_err()
        || sqlx::query("DELETE FROM staff_password_resets WHERE user_id=?")
            .bind(&user)
            .execute(&mut *tx)
            .await
            .is_err()
    {
        return error("Could not reset password");
    }
    if tx.commit().await.is_err() {
        return error("Could not reset password");
    }
    StatusCode::NO_CONTENT.into_response()
}
