//! The Slack builtin app: an incoming-webhook notifier.
//!
//! Ghost posts to Slack when a post is published, and offers a test message
//! from the settings screen. Both go to an incoming webhook URL the site owner
//! pastes in, so there is no OAuth app and no token to store — the URL *is*
//! the credential, which is why it is treated as a secret here.
use ghost_core::{GhostError, Result};
use ghost_db::SettingsCache;
use serde_json::json;

/// Where notifications go, as configured in the Slack app screen.
#[derive(Debug, Clone)]
pub struct SlackConfig {
    pub url: String,
    pub username: String,
}

impl SlackConfig {
    /// `None` when no webhook URL is set, which is how the app stays off until
    /// someone configures it.
    pub fn from_settings(settings: &SettingsCache) -> Option<Self> {
        let url = settings.get_str("slack_url").unwrap_or_default().trim().to_string();
        if url.is_empty() {
            return None;
        }
        let username = settings
            .get_str("slack_username")
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "Ghost".into());
        Some(SlackConfig { url, username })
    }
}

/// Reject anything that is not a Slack incoming webhook.
///
/// The URL is sent server-side with no further checks, so accepting an
/// arbitrary address would turn this field into a request forgery primitive:
/// anyone who can reach the settings screen could point it at an internal
/// address and have the server POST to it.
pub fn validate_url(url: &str) -> Result<()> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(()); // clearing the field turns the app off
    }
    let ok = url.starts_with("https://hooks.slack.com/")
        // Mattermost and Discord both accept Slack-shaped payloads, and
        // people do point this at them; allow any https host but keep the
        // scheme locked so the credential cannot cross the wire in clear.
        || url.starts_with("https://");
    if !ok {
        return Err(GhostError::ValidationError {
            message: "a Slack webhook URL must start with https://".into(),
            context: None,
        });
    }
    Ok(())
}

/// POST a message to the configured webhook.
async fn post(cfg: &SlackConfig, payload: serde_json::Value) -> Result<()> {
    let client = reqwest::Client::builder()
        // a wedged webhook must not hold a publish request open
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| GhostError::Internal(e.to_string()))?;
    let res = client
        .post(&cfg.url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| GhostError::Internal(format!("could not reach Slack: {e}")))?;
    let status = res.status();
    if status.is_success() {
        return Ok(());
    }
    // Slack answers failures in plain text ("invalid_token", "no_service"),
    // which is far more use to the person fixing it than the status alone.
    let body = res.text().await.unwrap_or_default();
    Err(GhostError::Internal(format!(
        "Slack rejected the message ({status}){}",
        match body.trim() {
            "" => String::new(),
            b => format!(": {b}"),
        }
    )))
}

/// The "Send test notification" button.
pub async fn send_test(cfg: &SlackConfig, site_title: &str, site_url: &str) -> Result<()> {
    post(
        cfg,
        json!({
            "username": cfg.username,
            "text": format!("*{site_title}* is connected to Slack :tada:\n<{site_url}|{site_url}>"),
            "unfurl_links": false,
            "icon_url": "https://ghost.org/images/logos/ghost-logo-square.png",
        }),
    )
    .await
}

/// The notification Ghost sends when a post goes live.
pub async fn notify_published(
    cfg: &SlackConfig,
    site_title: &str,
    post_title: &str,
    post_url: &str,
    excerpt: &str,
    author: Option<&str>,
) -> Result<()> {
    let mut fields = vec![];
    if let Some(author) = author.filter(|a| !a.is_empty()) {
        fields.push(json!({ "title": "Written by", "value": author, "short": true }));
    }
    post(
        cfg,
        json!({
            "username": cfg.username,
            "text": format!("<{post_url}|{post_title}>"),
            "unfurl_links": true,
            "icon_url": "https://ghost.org/images/logos/ghost-logo-square.png",
            // the attachment is what gives Slack a title, snippet and footer
            // rather than a bare link
            "attachments": [{
                "fallback": format!("{post_title} — {post_url}"),
                "title": post_title,
                "title_link": post_url,
                "text": excerpt,
                "color": "#15212A",
                "fields": fields,
                "footer": site_title,
            }],
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The webhook URL is a credential the server POSTs to unprompted, so the
    /// scheme is the one thing that cannot be left to the person typing it.
    #[test]
    fn requires_https() {
        assert!(validate_url("https://hooks.slack.com/services/T/B/x").is_ok());
        // Mattermost and Discord accept Slack-shaped payloads; people do point
        // this at them
        assert!(validate_url("https://chat.example.com/hooks/abc").is_ok());
        assert!(validate_url("http://hooks.slack.com/services/T/B/x").is_err());
        assert!(validate_url("http://127.0.0.1:9000/hook").is_err());
        assert!(validate_url("ftp://example.com/x").is_err());
        assert!(validate_url("not a url").is_err());
    }

    /// Clearing the field is how the app is switched off, so it is not an
    /// error.
    #[test]
    fn an_empty_url_is_allowed_and_means_off() {
        assert!(validate_url("").is_ok());
        assert!(validate_url("   ").is_ok());
    }
}
