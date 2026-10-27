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
            ("post.rescheduled", "Post rescheduled"),
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
            ("page.scheduled", "Page scheduled"),
            ("page.unscheduled", "Page unscheduled"),
            ("page.rescheduled", "Page rescheduled"),
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

/// HMAC-SHA256, hex encoded.
///
/// Hand-rolled over `sha2` (block size 64) rather than adding an `hmac`
/// dependency, matching how the member session cookie is signed.
fn hmac_hex(secret: &str, message: &str) -> String {
    const BLOCK: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(BLOCK, 0);
    let ipad: Vec<u8> = key.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = key.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::digest([&ipad[..], message.as_bytes()].concat());
    hex::encode(Sha256::digest([&opad[..], &inner[..]].concat()))
}

/// Ghost retries a failed delivery five times before giving up.
const MAX_DELIVERY_RETRIES: u32 = 5;
/// First backoff step; doubled on each further attempt.
const RETRY_BASE_DELAY_MS: u64 = 300;

/// Ghost's `X-Ghost-Signature` header value: `sha256=<hex>, t=<ms>`.
///
/// Two details here are the whole reason a receiver can verify a delivery,
/// and both were wrong: the signed message is the body **with the timestamp
/// appended**, not the body alone, and the header carries the scheme and that
/// timestamp alongside the digest. A receiver following Ghost's documented
/// verification — recompute `hmac(secret, body + t)` and compare — rejected
/// every delivery this port sent.
///
/// The timestamp is also what makes the signature replay-resistant: it is
/// inside the MAC, so it cannot be changed without invalidating it.
fn signature_header(secret: &str, body: &str, timestamp_ms: i64) -> String {
    let digest = hmac_hex(secret, &format!("{body}{timestamp_ms}"));
    format!("sha256={digest}, t={timestamp_ms}")
}

/// Ghost's member webhook payload: the row under `current`, and what it
/// looked like before under `previous`.
///
/// Lives here rather than in the admin crate because members are created on
/// two paths — the admin's own form and Portal signup — and only the first
/// used to fire anything. An integration listening for new members therefore
/// never heard about the ones who actually signed up, which is the case that
/// matters most.
pub fn fire_member(
    state: &AppState,
    event: &'static str,
    id: &str,
    email: &str,
    name: &str,
    previous: Value,
) {
    fire(
        state,
        event,
        serde_json::json!({
            "member": {
                "current": { "id": id, "email": email, "name": name },
                "previous": previous,
            }
        }),
    );
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        for hook in hooks {
            // Ghost puts the event name into the payload it sends, so a
            // receiver listening on one URL for several events can tell them
            // apart without parsing the body for clues. Built per hook
            // because the name comes off the row.
            let mut hook_payload = payload.clone();
            if let Value::Object(m) = &mut hook_payload {
                // `event` first, as Ghost spreads it.
                let rest = std::mem::take(m);
                m.insert("event".into(), Value::String(hook.event.clone()));
                m.extend(rest);
            }
            let body = serde_json::to_string(&hook_payload).unwrap_or_else(|_| "{}".into());

            let mut req_builder = client
                .post(&hook.target_url)
                .header("Content-Type", "application/json")
                // Ghost sends the API version it is speaking, and receivers
                // branch on it.
                .header("Content-Version", format!("v{}", ghost_core::GHOST_VERSION_SAFE))
                .body(body.clone());
            if let Some(secret) = hook.secret.as_deref().filter(|s| !s.is_empty()) {
                let ts = chrono::Utc::now().timestamp_millis();
                req_builder =
                    req_builder.header("X-Ghost-Signature", signature_header(secret, &body, ts));
            }
            // Ghost retries a delivery five times before giving up. A
            // receiver restarting, or rate-limiting, or briefly 502-ing
            // behind a proxy is ordinary — dropping the only notification of
            // a new member because of a two-second blip is not acceptable to
            // anything downstream.
            //
            // Only failures that might pass are retried: a connection error,
            // a 429, or a 5xx. A 4xx is the receiver saying the request
            // itself is wrong, and repeating it cannot change that.
            let mut attempt = 0u32;
            loop {
                let Some(req) = req_builder.try_clone() else { break };
                let outcome = req.send().await;
                let retryable = match &outcome {
                    Ok(resp) => {
                        let s = resp.status();
                        s.as_u16() == 429 || s.is_server_error()
                    }
                    Err(_) => true,
                };

                if !retryable || attempt >= MAX_DELIVERY_RETRIES {
                    match outcome {
                        Ok(resp) => {
                            let status = resp.status();
                            let error = if status.is_success() {
                                None
                            } else {
                                Some(
                                    status
                                        .canonical_reason()
                                        .unwrap_or("request failed")
                                        .to_string(),
                                )
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
                            // A target that cannot be reached is the common
                            // case and not this site's fault, so it is
                            // recorded rather than logged as an error.
                            tracing::debug!(
                                "webhook {} to {} failed after {} attempt(s): {e}",
                                hook.event,
                                hook.target_url,
                                attempt + 1
                            );
                            repos::webhooks::record_delivery(
                                &state.db,
                                &hook.id,
                                "error",
                                Some(&e.to_string()),
                            )
                            .await;
                        }
                    }
                    break;
                }

                // Exponential backoff, as Ghost's HTTP client does: a
                // receiver that is struggling is not helped by five
                // immediate retries.
                tokio::time::sleep(std::time::Duration::from_millis(
                    RETRY_BASE_DELAY_MS << attempt,
                ))
                .await;
                attempt += 1;
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
        // Ghost 6.73's WEBHOOKS list, all 29 of them.
        assert_eq!(all.len(), 29);
        for event in [
            "post.rescheduled", "page.scheduled", "page.unscheduled", "page.rescheduled",
        ] {
            assert!(is_known_event(event), "{event} is in Ghost's list");
        }
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
            hmac_hex(&key, "Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // same body, different secret → different signature
        assert_ne!(hmac_hex("a", "body"), hmac_hex("b", "body"));
    }

    /// The header a receiver actually parses, in Ghost's shape.
    #[test]
    fn the_signature_header_is_ghosts_scheme_digest_and_timestamp() {
        let header = signature_header("s3cret", r#"{"a":1}"#, 1_700_000_000_000);

        // `sha256=<hex>, t=<ms>` — the scheme and the timestamp are part of
        // the contract, not decoration.
        let (sig, t) = header.split_once(", ").expect("two comma-separated parts");
        assert_eq!(t, "t=1700000000000");
        let digest = sig.strip_prefix("sha256=").expect("the scheme prefix");
        assert_eq!(digest.len(), 64, "hex SHA-256");

        // The signed message is body + timestamp, which is what Ghost's
        // documented verification recomputes. Signing the body alone — what
        // this port used to do — produces a different digest, and every
        // receiver following the docs rejected it.
        assert_eq!(digest, hmac_hex("s3cret", r#"{"a":1}1700000000000"#));
        assert_ne!(digest, hmac_hex("s3cret", r#"{"a":1}"#));

        // The timestamp is inside the MAC, so a replayed delivery cannot have
        // its `t` rewritten without invalidating the signature.
        let later = signature_header("s3cret", r#"{"a":1}"#, 1_700_000_000_001);
        assert_ne!(header, later);
    }
}
