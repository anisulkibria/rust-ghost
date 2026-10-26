//! Firing the webhooks a custom integration has registered.
//!
//! Ghost's webhook service: when something happens on the site, every
//! registered listener for that event gets the payload POSTed to it.
//!
//! Delivery happens off the request. A publisher pressing Publish must not
//! wait on somebody else's server, and must not see an error because that
//! server is down — the outcome is recorded on the webhook row instead, which
//! is what the integration's page reports.
use ghost_db::repos;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AppState;

/// Ghost's webhook catalog, grouped as its event picker groups them.
/// Group label, then the events in it as (value, label).
pub const EVENT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Site", &[("site.changed", "Site changed (rebuild)")]),
    (
        "Posts",
        &[
            ("post.added", "Post created"),
            ("post.deleted", "Post deleted"),
            ("post.edited", "Post updated"),
            ("post.published", "Post published"),
            ("post.published.edited", "Published post updated"),
            ("post.unpublished", "Post unpublished"),
            ("post.scheduled", "Post scheduled"),
            ("post.unscheduled", "Post unscheduled"),
            ("post.tag.attached", "Tag added to post"),
            ("post.tag.detached", "Tag removed from post"),
        ],
    ),
    (
        "Pages",
        &[
            ("page.added", "Page created"),
            ("page.deleted", "Page deleted"),
            ("page.edited", "Page updated"),
            ("page.published", "Page published"),
            ("page.published.edited", "Published page updated"),
            ("page.unpublished", "Page unpublished"),
            ("page.tag.attached", "Tag added to page"),
            ("page.tag.detached", "Tag removed from page"),
        ],
    ),
    (
        "Tags",
        &[
            ("tag.added", "Tag created"),
            ("tag.edited", "Tag updated"),
            ("tag.deleted", "Tag deleted"),
        ],
    ),
    (
        "Members",
        &[
            ("member.added", "Member added"),
            ("member.edited", "Member updated"),
            ("member.deleted", "Member deleted"),
        ],
    ),
];

/// Whether this build knows the event, so a hand-typed one is refused rather
/// than silently never firing.
pub fn is_known_event(event: &str) -> bool {
    EVENT_GROUPS.iter().any(|(_, evs)| evs.iter().any(|(v, _)| *v == event))
}

/// The label for an event, for the admin list.
pub fn event_label(event: &str) -> &str {
    EVENT_GROUPS
        .iter()
        .flat_map(|(_, evs)| evs.iter())
        .find(|(v, _)| *v == event)
        .map(|(_, l)| *l)
        .unwrap_or(event)
}

/// HMAC-SHA256 of the body under the webhook's secret, hex encoded.
///
/// Ghost signs deliveries so a receiver can tell a real one from anybody who
/// learned the URL. Hand-rolled over `sha2` (block size 64) rather than adding
/// an `hmac` dependency, matching how the member session cookie is signed.
fn sign(secret: &str, body: &str) -> String {
    const BLOCK: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(BLOCK, 0);
    let ipad: Vec<u8> = key.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = key.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::digest([&ipad[..], body.as_bytes()].concat());
    hex::encode(Sha256::digest([&opad[..], &inner[..]].concat()))
}

/// Fire every webhook listening for `event`.
///
/// Returns immediately: the deliveries run on their own task, so nothing on
/// the request path waits for a third-party server.
pub fn fire(state: &AppState, event: &'static str, payload: Value) {
    let state = state.clone();
    tokio::spawn(async move {
        let hooks = match repos::webhooks::list_for_event(&state.db, event).await {
            Ok(h) if !h.is_empty() => h,
            _ => return,
        };
        let body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        for hook in hooks {
            let mut req = client
                .post(&hook.target_url)
                .header("Content-Type", "application/json")
                .header("User-Agent", "Ghost/5.98 (https://github.com/TryGhost/Ghost)")
                .body(body.clone());
            if let Some(secret) = hook.secret.as_deref().filter(|s| !s.is_empty()) {
                req = req.header("X-Ghost-Signature", sign(secret, &body));
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let error = if status.is_success() {
                        None
                    } else {
                        Some(status.canonical_reason().unwrap_or("request failed").to_string())
                    };
                    repos::webhooks::record_delivery(
                        &state.db,
                        &hook.id,
                        status.as_str(),
                        error.as_deref(),
                    )
                    .await;
                }
                Err(e) => {
                    // A target that cannot be reached is the common case and
                    // not this site's fault, so it is recorded rather than
                    // logged as an error.
                    tracing::debug!("webhook {} to {} failed: {e}", hook.event, hook.target_url);
                    repos::webhooks::record_delivery(&state.db, &hook.id, "error", Some(&e.to_string()))
                        .await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_matches_ghosts() {
        let all: Vec<&str> =
            EVENT_GROUPS.iter().flat_map(|(_, e)| e.iter().map(|(v, _)| *v)).collect();
        assert_eq!(all.len(), 25);
        assert!(is_known_event("post.published"));
        assert!(is_known_event("member.deleted"));
        assert!(!is_known_event("post.exploded"));
        assert_eq!(event_label("post.published"), "Post published");
        // an event this build does not know still prints as itself
        assert_eq!(event_label("future.event"), "future.event");
    }

    /// Signing must be real HMAC-SHA256, so a receiver verifying with a
    /// standard library agrees with us.
    #[test]
    fn signatures_match_known_hmac_vectors() {
        // RFC 4231 test case 1
        let key = "\x0b".repeat(20);
        assert_eq!(
            sign(&key, "Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // same body, different secret → different signature
        assert_ne!(sign("a", "body"), sign("b", "body"));
    }
}
