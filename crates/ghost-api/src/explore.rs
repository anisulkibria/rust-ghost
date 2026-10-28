//! Explore synchronization uses Ghost's public registration contract directly.
use crate::AppState;
use serde_json::json;
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = ping(&state).await {
                tracing::warn!("Explore synchronization failed: {}", e.message());
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}
pub async fn ping(state: &AppState) -> ghost_core::Result<()> {
    if !state.settings.get_bool("explore_ping") {
        return Ok(());
    }
    let url = state.site_url();
    let parsed =
        reqwest::Url::parse(&url).map_err(|e| ghost_core::GhostError::BadRequest(e.to_string()))?;
    // Local/private installations cannot be discovered from the public internet.
    if state.settings.get_bool("is_private")
        || parsed.host_str().is_none_or(|h| {
            h == "localhost"
                || h.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
                    std::net::IpAddr::V4(ip) => {
                        ip.is_private()
                            || ip.is_loopback()
                            || ip.is_link_local()
                            || ip.is_unspecified()
                    }
                    std::net::IpAddr::V6(ip) => {
                        ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local()
                    }
                })
        })
    {
        return Ok(());
    }
    let (total,first,last):(i64,Option<String>,Option<String>)=sqlx::query_as("SELECT COUNT(*),MIN(published_at),MAX(published_at) FROM posts WHERE status='published' AND type='post'").fetch_one(&state.db).await.map_err(|e|ghost_core::GhostError::Db(e.to_string()))?;
    let mut payload = json!({"ghost":ghost_core::GHOST_VERSION,"site_uuid":state.settings.get_str("site_uuid"),"url":url,"theme":state.settings.get_str("active_theme"),"facebook":state.settings.get_str("facebook"),"twitter":state.settings.get_str("twitter"),"posts_total":total,"posts_first":first,"posts_last":last});
    if state.settings.get_bool("explore_ping_growth") {
        let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
            .fetch_one(&state.db)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;
        payload["members_total"] = json!(members);
        let live =
            ghost_stripe::StripeConfig::from_settings(&state.settings).is_some_and(|c| c.is_live());
        let mrr = if live {
            sqlx::query_as::<_,(Option<String>,f64)>("SELECT plan_currency,COALESCE(SUM(CASE WHEN plan_interval='year' THEN plan_amount/12.0 ELSE plan_amount END),0) FROM subscriptions WHERE status='active' GROUP BY plan_currency").fetch_all(&state.db).await.unwrap_or_default().into_iter().map(|(currency,mrr)|json!({"currency":currency,"mrr":mrr})).collect::<Vec<_>>()
        } else {
            vec![]
        };
        payload["mrr"] = json!(mrr);
    }
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ghost_core::GhostError::BadRequest(e.to_string()))?
        .post("https://explore.ghost.org/api/update")
        .json(&payload)
        .send()
        .await
        .map_err(|e| ghost_core::GhostError::BadRequest(e.to_string()))?;
    if !response.status().is_success() {
        return Err(ghost_core::GhostError::BadRequest(format!(
            "Explore returned {}",
            response.status()
        )));
    }
    Ok(())
}
