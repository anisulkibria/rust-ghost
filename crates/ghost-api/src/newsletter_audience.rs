//! Independent newsletter targeting, validated before publishing or scheduling.
use crate::AppState;
use ghost_core::{GhostError, Result};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audience {
    pub newsletter_id: Option<String>,
    pub filter: Option<String>,
    pub include_preview: bool,
}
pub async fn load(state: &AppState, id: &str) -> Result<Audience> {
    let row:(Option<String>,Option<String>,bool)=sqlx::query_as("SELECT newsletter_id,newsletter_audience_filter,newsletter_include_preview FROM posts WHERE id=?").bind(id).fetch_one(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(Audience {
        newsletter_id: row.0,
        filter: row.1,
        include_preview: row.2,
    })
}
pub async fn validate(state: &AppState, audience: &Audience) -> Result<()> {
    if let Some(filter) = audience.filter.as_deref().filter(|s| !s.is_empty()) {
        if filter.len() > 2000 {
            return Err(GhostError::BadRequest("Audience filter is too long".into()));
        }
        let expression = ghost_nql::parse(filter)?;
        ghost_nql::compile(&expression, "members")?;
    }
    if let Some(id) = audience.newsletter_id.as_deref().filter(|s| !s.is_empty()) {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM newsletters WHERE id=? AND status='active')",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
        if !active {
            return Err(GhostError::BadRequest("Choose an active newsletter".into()));
        }
    }
    Ok(())
}
pub async fn save(state: &AppState, id: &str, audience: &Audience) -> Result<()> {
    validate(state, audience).await?;
    sqlx::query("UPDATE posts SET newsletter_id=?,newsletter_audience_filter=?,newsletter_include_preview=? WHERE id=?")
        .bind(audience.newsletter_id.as_deref().filter(|s|!s.is_empty())).bind(audience.filter.as_deref().filter(|s|!s.is_empty())).bind(audience.include_preview).bind(id).execute(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    Ok(())
}
