//! Social media: talking to the platforms.
//!
//! One function per network for **testing** a connection and one for
//! **posting**, each the platform's own documented API, called with the
//! credentials staff gave `vuuut_core::social_media::credentials`:
//!
//! | Network  | Posts with |
//! |----------|------------|
//! | X        | `POST api.x.com/2/tweets`, OAuth 1.0a user context |
//! | Bluesky  | `com.atproto.server.createSession` then `com.atproto.repo.createRecord`, with link facets and a link card |
//! | Mastodon | `POST {instance}/api/v1/statuses` |
//! | Threads  | `POST /{user}/threads` then `/{user}/threads_publish` |
//! | LinkedIn | `POST api.linkedin.com/rest/posts`, an article share |
//! | Facebook | `POST graph.facebook.com/{page}/feed` with the link |
//! | Telegram | `POST api.telegram.org/bot…/sendMessage` |
//! | Discord  | `POST {webhook}?wait=true` |
//!
//! Requests never follow redirects and time out after twenty seconds. The
//! two hosts staff type in (a Mastodon instance, a Bluesky server) are https
//! only and pass the same private-address check as every other fetch this
//! server makes, so an account cannot be pointed at the site's own network.
//!
//! Errors are summarised for people — the status and the platform's own
//! message, bounded — and never include a credential.
//!
//! Sending is asynchronous: [`spawn_send`] claims a queued share (only one
//! sender can) and records what happened on the share and on the account's
//! health. [`send_due`] is called by the scheduler for scheduled shares.

use crate::AppState;
use vuuut_core::social_media::{self as rules, Network};
use vuuut_db::repos::social_media as repo;
use hmac::{Hmac, Mac};
use serde_json::{json, Map, Value};
use std::time::Duration;

/// LinkedIn's REST API is versioned by month and each version is supported
/// for about a year. When posting starts failing with a version error, this
/// is the line to move forward.
pub const LINKEDIN_VERSION: &str = "202510";
const GRAPH: &str = "https://graph.facebook.com/v21.0";
const THREADS: &str = "https://graph.threads.net/v1.0";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Vuuut social sharing")
        .build()
        .expect("http client")
}

fn get<'a>(c: &'a Map<String, Value>, k: &str) -> &'a str {
    c.get(k).and_then(Value::as_str).unwrap_or("")
}

/// A host staff typed: https only, and outside the server's own network.
fn public_base(state: &AppState, base: &str) -> Result<String, String> {
    if !base.starts_with("https://") {
        return Err("Only https addresses are used.".into());
    }
    if crate::security_settings::private_fetch_blocked(state) && crate::mentions::fetchable(base).is_none() {
        return Err("That server is not reachable from here. Private and local addresses are refused.".into());
    }
    Ok(base.trim_end_matches('/').to_string())
}

/// The platform's answer, bounded, as a sentence for the history.
async fn failure(what: &str, resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            for path in ["/error/message", "/message", "/detail", "/errors/0/message", "/description", "/error_description", "/error", "/title"] {
                if let Some(s) = v.pointer(path).and_then(Value::as_str) {
                    return Some(s.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| body.chars().take(160).collect());
    let message: String = message.chars().filter(|c| !c.is_control()).take(240).collect();
    format!("{what} answered {}: {}", status.as_u16(), message.trim())
}

fn net(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "The platform did not answer in time.".into()
    } else if e.is_connect() {
        "The platform could not be reached.".into()
    } else {
        "The request to the platform failed.".into()
    }
}

/// What a share needs to say beyond its text: the post, for the platforms
/// that draw a link card.
#[derive(Debug, Clone, Default)]
pub struct LinkCard {
    pub url: String,
    pub title: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// testing a connection
// ---------------------------------------------------------------------------

/// Check the credentials work, and return the name the platform knows the
/// account by.
pub async fn test(state: &AppState, network: Network, c: &Map<String, Value>) -> Result<String, String> {
    let http = client();
    match network {
        Network::X => {
            let url = "https://api.x.com/2/users/me";
            let auth = oauth1_header("GET", url, &[], c, &nonce(), &timestamp());
            let r = http.get(url).header("Authorization", auth).send().await.map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("X", r).await);
            }
            let v: Value = r.json().await.map_err(|_| "X sent an answer that could not be read.".to_string())?;
            Ok(format!("@{}", v.pointer("/data/username").and_then(Value::as_str).unwrap_or("account")))
        }
        Network::Bluesky => {
            let (_, did, handle) = bluesky_session(state, c).await?;
            let _ = did;
            Ok(format!("@{handle}"))
        }
        Network::Mastodon => {
            let base = public_base(state, get(c, "instance"))?;
            let r = http
                .get(format!("{base}/api/v1/accounts/verify_credentials"))
                .bearer_auth(get(c, "access_token"))
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Mastodon", r).await);
            }
            let v: Value = r.json().await.map_err(|_| "Mastodon sent an answer that could not be read.".to_string())?;
            let host = base.trim_start_matches("https://");
            Ok(format!("@{}@{host}", v.get("acct").and_then(Value::as_str).unwrap_or("account")))
        }
        Network::Threads => {
            let r = http
                .get(format!("{THREADS}/me"))
                .query(&[("fields", "username"), ("access_token", get(c, "access_token"))])
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Threads", r).await);
            }
            let v: Value = r.json().await.map_err(|_| "Threads sent an answer that could not be read.".to_string())?;
            Ok(format!("@{}", v.get("username").and_then(Value::as_str).unwrap_or("account")))
        }
        Network::LinkedIn => {
            let r = http.get("https://api.linkedin.com/v2/userinfo").bearer_auth(get(c, "access_token")).send().await.map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("LinkedIn", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            Ok(if get(c, "author").contains("organization") {
                "Company page".into()
            } else {
                v.get("name").and_then(Value::as_str).unwrap_or("Personal profile").to_string()
            })
        }
        Network::Facebook => {
            let r = http
                .get(format!("{GRAPH}/{}", get(c, "page_id")))
                .query(&[("fields", "name"), ("access_token", get(c, "access_token"))])
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Facebook", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            Ok(v.get("name").and_then(Value::as_str).unwrap_or("Facebook Page").to_string())
        }
        Network::Telegram => {
            let r = http
                .post(format!("https://api.telegram.org/bot{}/getChat", get(c, "bot_token")))
                .json(&json!({ "chat_id": get(c, "chat_id") }))
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Telegram", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            Ok(v.pointer("/result/title").and_then(Value::as_str).unwrap_or(get(c, "chat_id")).to_string())
        }
        Network::Discord => {
            let r = http.get(get(c, "webhook_url")).send().await.map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Discord", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            Ok(format!("#{}", v.get("name").and_then(Value::as_str).unwrap_or("channel")))
        }
    }
}

// ---------------------------------------------------------------------------
// posting
// ---------------------------------------------------------------------------

/// Post `text` to one account. Returns the platform's id for the post and,
/// where it has one, its public address.
pub async fn publish(state: &AppState, network: Network, c: &Map<String, Value>, text: &str, card: &LinkCard, idempotency: &str) -> Result<(String, String), String> {
    let http = client();
    match network {
        Network::X => {
            let url = "https://api.x.com/2/tweets";
            let auth = oauth1_header("POST", url, &[], c, &nonce(), &timestamp());
            let r = http.post(url).header("Authorization", auth).json(&json!({ "text": text })).send().await.map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("X", r).await);
            }
            let v: Value = r.json().await.map_err(|_| "X sent an answer that could not be read.".to_string())?;
            let id = v.pointer("/data/id").and_then(Value::as_str).unwrap_or("").to_string();
            Ok((id.clone(), format!("https://x.com/i/web/status/{id}")))
        }
        Network::Bluesky => {
            let (jwt, did, handle) = bluesky_session(state, c).await?;
            let base = public_base(state, get(c, "pds"))?;
            let facets: Vec<Value> = rules::url_ranges(text)
                .into_iter()
                .map(|(s, e, uri)| json!({
                    "index": { "byteStart": s, "byteEnd": e },
                    "features": [{ "$type": "app.bsky.richtext.facet#link", "uri": uri }]
                }))
                .collect();
            let mut record = json!({
                "$type": "app.bsky.feed.post",
                "text": text,
                "createdAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "facets": facets,
            });
            if !card.url.is_empty() {
                record["embed"] = json!({
                    "$type": "app.bsky.embed.external",
                    "external": { "uri": card.url, "title": card.title, "description": card.description }
                });
            }
            let r = http
                .post(format!("{base}/xrpc/com.atproto.repo.createRecord"))
                .bearer_auth(jwt)
                .json(&json!({ "repo": did, "collection": "app.bsky.feed.post", "record": record }))
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Bluesky", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            let uri = v.get("uri").and_then(Value::as_str).unwrap_or("").to_string();
            let rkey = uri.rsplit('/').next().unwrap_or("");
            Ok((uri.clone(), format!("https://bsky.app/profile/{handle}/post/{rkey}")))
        }
        Network::Mastodon => {
            let base = public_base(state, get(c, "instance"))?;
            let r = http
                .post(format!("{base}/api/v1/statuses"))
                .bearer_auth(get(c, "access_token"))
                // Mastodon keeps this for an hour: a retry after a lost
                // answer does not post twice.
                .header("Idempotency-Key", idempotency)
                .form(&[("status", text), ("visibility", "public")])
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Mastodon", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            Ok((
                v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                v.get("url").and_then(Value::as_str).unwrap_or("").to_string(),
            ))
        }
        Network::Threads => {
            let user = get(c, "user_id");
            let token = get(c, "access_token");
            let r = http
                .post(format!("{THREADS}/{user}/threads"))
                .form(&[("media_type", "TEXT"), ("text", text), ("access_token", token)])
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Threads", r).await);
            }
            let container = r.json::<Value>().await.ok().and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string)).ok_or("Threads did not return a draft id.")?;
            let r = http
                .post(format!("{THREADS}/{user}/threads_publish"))
                .form(&[("creation_id", container.as_str()), ("access_token", token)])
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Threads", r).await);
            }
            let id = r.json::<Value>().await.ok().and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
            let link = http
                .get(format!("{THREADS}/{id}"))
                .query(&[("fields", "permalink"), ("access_token", token)])
                .send()
                .await
                .ok();
            let permalink = match link {
                Some(r) if r.status().is_success() => r.json::<Value>().await.ok().and_then(|v| v.get("permalink").and_then(Value::as_str).map(str::to_string)).unwrap_or_default(),
                _ => String::new(),
            };
            Ok((id, permalink))
        }
        Network::LinkedIn => {
            let mut body = json!({
                "author": get(c, "author"),
                "commentary": linkedin_escape(text),
                "visibility": "PUBLIC",
                "distribution": { "feedDistribution": "MAIN_FEED", "targetEntities": [], "thirdPartyDistributionChannels": [] },
                "lifecycleState": "PUBLISHED",
                "isReshareDisabledByAuthor": false
            });
            if !card.url.is_empty() {
                body["content"] = json!({ "article": { "source": card.url, "title": card.title, "description": card.description } });
            }
            let r = http
                .post("https://api.linkedin.com/rest/posts")
                .bearer_auth(get(c, "access_token"))
                .header("LinkedIn-Version", LINKEDIN_VERSION)
                .header("X-Restli-Protocol-Version", "2.0.0")
                .json(&body)
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("LinkedIn", r).await);
            }
            let id = r.headers().get("x-restli-id").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            Ok((id.clone(), if id.is_empty() { String::new() } else { format!("https://www.linkedin.com/feed/update/{id}/") }))
        }
        Network::Facebook => {
            let mut form = vec![("message", text.to_string()), ("access_token", get(c, "access_token").to_string())];
            if !card.url.is_empty() {
                form.push(("link", card.url.clone()));
            }
            let r = http.post(format!("{GRAPH}/{}/feed", get(c, "page_id"))).form(&form).send().await.map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Facebook", r).await);
            }
            let id = r.json::<Value>().await.ok().and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
            Ok((id.clone(), format!("https://www.facebook.com/{id}")))
        }
        Network::Telegram => {
            let r = http
                .post(format!("https://api.telegram.org/bot{}/sendMessage", get(c, "bot_token")))
                .json(&json!({ "chat_id": get(c, "chat_id"), "text": text }))
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Telegram", r).await);
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            let id = v.pointer("/result/message_id").and_then(Value::as_i64).map(|n| n.to_string()).unwrap_or_default();
            let chat = get(c, "chat_id");
            let url = chat.strip_prefix('@').map(|name| format!("https://t.me/{name}/{id}")).unwrap_or_default();
            Ok((id, url))
        }
        Network::Discord => {
            let url = format!("{}?wait=true", get(c, "webhook_url").split('?').next().unwrap_or(""));
            let r = http
                .post(url)
                // No @everyone, no role pings: a share announces, it does
                // not summon.
                .json(&json!({ "content": text, "allowed_mentions": { "parse": [] } }))
                .send()
                .await
                .map_err(net)?;
            if !r.status().is_success() {
                return Err(failure("Discord", r).await);
            }
            let id = r.json::<Value>().await.ok().and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
            Ok((id, String::new()))
        }
    }
}

async fn bluesky_session(state: &AppState, c: &Map<String, Value>) -> Result<(String, String, String), String> {
    let base = public_base(state, get(c, "pds"))?;
    let r = client()
        .post(format!("{base}/xrpc/com.atproto.server.createSession"))
        .json(&json!({ "identifier": get(c, "handle"), "password": get(c, "app_password") }))
        .send()
        .await
        .map_err(net)?;
    if !r.status().is_success() {
        return Err(failure("Bluesky", r).await);
    }
    let v: Value = r.json().await.map_err(|_| "Bluesky sent an answer that could not be read.".to_string())?;
    let jwt = v.get("accessJwt").and_then(Value::as_str).ok_or("Bluesky did not open a session.")?.to_string();
    let did = v.get("did").and_then(Value::as_str).unwrap_or("").to_string();
    let handle = v.get("handle").and_then(Value::as_str).unwrap_or(get(c, "handle")).to_string();
    Ok((jwt, did, handle))
}

/// LinkedIn's "little text" format treats these as markup; a literal one
/// has to be escaped or the post is rejected or mangled.
fn linkedin_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '|' | '{' | '}' | '@' | '[' | ']' | '(' | ')' | '<' | '>' | '#' | '*' | '_' | '~') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------
// OAuth 1.0a for X
// ---------------------------------------------------------------------------

fn nonce() -> String {
    (0..32).map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rand::random::<u8>() as usize % 36])).collect()
}

fn timestamp() -> String {
    chrono::Utc::now().timestamp().to_string()
}

/// RFC 3986 percent-encoding, as OAuth 1.0a requires (every byte but the
/// unreserved set).
fn pct(s: &str) -> String {
    rules::encode(s)
}

/// The `Authorization` header for one request. `params` are the request's
/// query or form parameters (none for X's JSON endpoints — a JSON body is
/// not part of the signature).
pub fn oauth1_header(method: &str, url: &str, params: &[(&str, &str)], c: &Map<String, Value>, nonce: &str, timestamp: &str) -> String {
    let oauth = [
        ("oauth_consumer_key", get(c, "api_key")),
        ("oauth_nonce", nonce),
        ("oauth_signature_method", "HMAC-SHA1"),
        ("oauth_timestamp", timestamp),
        ("oauth_token", get(c, "access_token")),
        ("oauth_version", "1.0"),
    ];
    let mut all: Vec<(String, String)> = oauth
        .iter()
        .chain(params.iter())
        .map(|(k, v)| (pct(k), pct(v)))
        .collect();
    all.sort();
    let param_string = all.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let base = format!("{}&{}&{}", method.to_uppercase(), pct(url), pct(&param_string));
    let key = format!("{}&{}", pct(get(c, "api_secret")), pct(get(c, "access_secret")));
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key.as_bytes()).expect("any key length");
    mac.update(base.as_bytes());
    let signature = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, mac.finalize().into_bytes());
    let mut header: Vec<String> = oauth.iter().map(|(k, v)| format!("{k}=\"{}\"", pct(v))).collect();
    header.push(format!("oauth_signature=\"{}\"", pct(&signature)));
    header.sort();
    format!("OAuth {}", header.join(", "))
}

// ---------------------------------------------------------------------------
// sending queued shares
// ---------------------------------------------------------------------------

/// Send one share in the background. Returns at once.
pub fn spawn_send(state: &AppState, share_id: String) {
    let state = state.clone();
    tokio::spawn(async move {
        send(&state, &share_id).await;
    });
}

/// Send one share now, if nobody else already is.
pub async fn send(state: &AppState, share_id: &str) {
    if !repo::claim(&state.db, share_id).await {
        return;
    }
    let Ok(Some(share)) = repo::share(&state.db, share_id).await else { return };
    let Ok(Some(account)) = repo::account(&state.db, &share.account_id).await else {
        repo::finish(&state.db, share_id, Err("The account was disconnected.".into())).await;
        return;
    };
    let Some(network) = Network::from_key(&account.network) else {
        repo::finish(&state.db, share_id, Err("That network is not supported any more.".into())).await;
        return;
    };
    let card = card_for(state, &share.post_id, &share.body).await;
    let result = publish(state, network, &account.creds(), &share.body, &card, share_id).await;
    match &result {
        Ok(_) => repo::set_health(&state.db, &account.id, true, "", None).await,
        Err(e) => {
            tracing::warn!("social: sharing {} to {} failed: {e}", share.post_id, network.key());
            repo::set_health(&state.db, &account.id, false, e, None).await;
        }
    }
    repo::finish(&state.db, share_id, result).await;
}

/// The link card: the first link in the share's own text (so the card points
/// where the text does, campaign tags and all), the post's title and excerpt.
async fn card_for(state: &AppState, post_id: &str, body: &str) -> LinkCard {
    let url = rules::url_ranges(body).into_iter().next().map(|(_, _, u)| u).unwrap_or_default();
    let post = vuuut_db::repos::posts::find_by_id(&state.db, post_id).await.ok().flatten();
    LinkCard {
        url,
        title: post.as_ref().and_then(|p| p.title.clone()).unwrap_or_default(),
        description: post
            .as_ref()
            .and_then(|p| p.custom_excerpt.clone().or_else(|| vuuut_core::models::excerpt_from_plaintext(p.plaintext.as_deref(), 30)))
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect(),
    }
}

/// Scheduled shares whose time has come. Called by the scheduler's tick.
pub async fn send_due(state: &AppState) {
    for share in repo::due(&state.db, &vuuut_core::dates::now_db()).await {
        send(state, &share.id).await;
    }
}

// ---------------------------------------------------------------------------
// share buttons under a post
// ---------------------------------------------------------------------------

pub static SHARE_JS: &str = include_str!("../assets/share.js");
pub static SHARE_CSS: &str = include_str!("../assets/share.css");

fn share_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in SHARE_JS.bytes().chain(SHARE_CSS.bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:x}")
    });
    &V
}

pub fn share_router() -> axum::Router<AppState> {
    use axum::{response::IntoResponse, routing::get};
    let asset = |body: &'static str, mime: &'static str| {
        (
            [
                (axum::http::header::CONTENT_TYPE, mime),
                (axum::http::header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            body,
        )
            .into_response()
    };
    axum::Router::new()
        .route("/social/share/assets/{v}/share.js", get(move || async move { asset(SHARE_JS, "text/javascript; charset=utf-8") }))
        .route("/social/share/assets/{v}/share.css", get(move || async move { asset(SHARE_CSS, "text/css; charset=utf-8") }))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The bar of share links for one post. Every button is a plain link to the
/// network's own share page (or a mailto), opened only when a reader chooses
/// it — nothing loads from any network before that, and no cookie is set.
pub fn share_bar(s: &rules::ShareSettings, url: &str, title: &str) -> String {
    let buttons: String = s
        .list
        .iter()
        .map(|b| match b.href(url, title) {
            Some(href) => format!(
                r#"<a class="vsb-btn vsb-{key}" href="{href}" target="_blank" rel="noopener nofollow" aria-label="Share on {label}">{label}</a>"#,
                key = b.key(),
                href = esc(&href),
                label = esc(b.label()),
            ),
            None => format!(r#"<button type="button" class="vsb-btn vsb-copy" data-copy="{u}">{label}</button>"#, u = esc(url), label = esc(b.label())),
        })
        .collect();
    format!(r#"<nav class="vsb" aria-label="{l}"><span class="vsb-label">{l}</span><div class="vsb-row">{buttons}</div></nav>"#, l = esc(&s.label))
}

/// Put the share bar into a post's body before the theme renders it. Returns
/// whether it did, so the renderer adds the stylesheet only when needed.
pub fn apply_share_buttons(state: &AppState, data: &mut Value, is_post: bool) -> bool {
    let s = rules::ShareSettings::read(|k| state.settings.get_str(k));
    if !s.buttons || !is_post || s.list.is_empty() {
        return false;
    }
    let (Some(html), Some(url)) = (
        data.pointer("/post/html").and_then(Value::as_str).map(str::to_string),
        data.pointer("/post/url").and_then(Value::as_str).map(str::to_string),
    ) else {
        return false;
    };
    let url = vuuut_core::url::absolute(&state.site_url(), &url);
    let title = data.pointer("/post/title").and_then(Value::as_str).unwrap_or("").to_string();
    let bar = share_bar(&s, &url, &title);
    let placed = match s.position.as_str() {
        "top" => format!("{bar}{html}"),
        "both" => format!("{bar}{html}{bar}"),
        _ => format!("{html}{bar}"),
    };
    data["post"]["html"] = Value::String(placed);
    true
}

/// The share bar's stylesheet and its one small script (Copy link).
pub fn share_assets(html: String) -> String {
    let base = vuuut_core::url::base("");
    let v = share_version();
    let tags = format!(
        r#"<link rel="stylesheet" href="{base}/social/share/assets/{v}/share.css"><script src="{base}/social/share/assets/{v}/share.js" defer data-cookie-ignore></script>"#
    );
    match html.find("</head>").or_else(|| html.rfind("</body>")) {
        Some(at) => format!("{}{tags}{}", &html[..at], &html[at..]),
        None => html,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oauth1_signature_matches_the_documented_example() {
        // X's own worked example ("Creating a signature"), which every OAuth
        // 1.0a client is checked against.
        let c: Map<String, Value> = serde_json::from_value(json!({
            "api_key": "xvz1evFS4wEEPTGEFPHBog",
            "api_secret": "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw",
            "access_token": "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb",
            "access_secret": "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE"
        })).unwrap();
        let header = oauth1_header(
            "post",
            "https://api.twitter.com/1.1/statuses/update.json",
            &[("include_entities", "true"), ("status", "Hello Ladies + Gentlemen, a signed OAuth request!")],
            &c,
            "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg",
            "1318622958",
        );
        assert!(header.contains(r#"oauth_signature="hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D""#), "{header}");
        assert!(header.starts_with("OAuth oauth_consumer_key=\"xvz1evFS4wEEPTGEFPHBog\""));
    }

    #[test]
    fn the_share_bar_is_links_escaped_and_copy_is_a_button() {
        let s = rules::ShareSettings::read(|k| (k == rules::SETTING_BUTTON_LIST).then(|| "x,copy".into()));
        let bar = share_bar(&s, "https://a.b/p?x=1&y=2", "\"><script>");
        assert!(bar.contains(r#"href="https://x.com/intent/post?text=%22%3E%3Cscript%3E&amp;url=https%3A%2F%2Fa.b"#), "{bar}");
        assert!(bar.contains(r#"data-copy="https://a.b/p?x=1&amp;y=2""#));
        assert!(!bar.contains("<script>"));
    }

    #[test]
    fn linkedin_markup_characters_are_escaped() {
        assert_eq!(linkedin_escape("Hi (all) #news @me"), r"Hi \(all\) \#news \@me");
    }
}
