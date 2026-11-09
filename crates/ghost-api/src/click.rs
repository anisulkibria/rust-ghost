//! Native Vuuut Click connector. All remote credentials and assertions stay
//! server-side. Source-table triggers commit the outbox with the CMS fact.
use crate::{identity, AppState};
use ghost_core::{GhostError, Result};
use ghost_db::types::Text;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub base_url: String,
    pub website_id: String,
    pub integration_key: String,
    #[serde(default)]
    pub collection_id: String,
    #[serde(default)]
    pub consent_required: bool,
    #[serde(default)]
    pub engagement: bool,
    #[serde(default)]
    pub persistent_identity: bool,
    #[serde(default)]
    pub pending_key: Option<String>,
}
fn fail(message: &str) -> GhostError {
    GhostError::BadRequest(message.into())
}
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Rustls client")
}
impl Connection {
    pub fn validate(&self, site: &str) -> Result<()> {
        if self.persistent_identity && !self.consent_required {
            return Err(fail("Persistent browser identity requires visitor consent"));
        }
        let u =
            reqwest::Url::parse(&self.base_url).map_err(|_| fail("Use the Click HTTPS origin"))?;
        let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let local_site = reqwest::Url::parse(site).ok().is_some_and(|s| {
            s.host_str().is_some_and(|h| {
                matches!(h, "localhost" | "127.0.0.1" | "[::1]") || h.ends_with(".localhost")
            })
        });
        if !(u.scheme() == "https" || u.scheme() == "http" && local && local_site)
            || u.host_str().is_none()
            || u.path() != "/"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
            || self.integration_key.len() < 32
            || self.integration_key.len() > 256
            || self.base_url.len() > 255
            || self.website_id.len() != 36
            || !self
                .website_id
                .bytes()
                .all(|c| c.is_ascii_hexdigit() || c == b'-')
        {
            return Err(fail(
                "Use a Click origin, website UUID and website-scoped integration credential",
            ));
        }
        Ok(())
    }
    fn endpoint(&self, action: &str) -> String {
        format!(
            "{}/native/v1/websites/{}/{action}",
            self.base_url.trim_end_matches('/'),
            self.website_id
        )
    }
    async fn request(&self, action: &str, body: &Value) -> Result<Value> {
        let response = http()
            .post(self.endpoint(action))
            .bearer_auth(&self.integration_key)
            .json(body)
            .send()
            .await
            .map_err(|_| fail("Click is unavailable; the operation can be retried"))?;
        if !response.status().is_success() {
            return Err(fail("Click refused the connector request; check origin verification, credentials and permissions"));
        }
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        response
            .json()
            .await
            .map_err(|_| fail("Click returned an invalid response"))
    }
}
pub async fn connection(state: &AppState) -> Result<Option<Connection>> {
    let raw: Text = sqlx::query_scalar("SELECT config FROM click_connection WHERE id=1")
        .fetch_one(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    let local: Value =
        serde_json::from_str(&raw).map_err(|_| fail("Stored Click configuration is invalid"))?;
    // Hosting policy remains authoritative even when it deliberately
    // disables the connector. A stale standalone record cannot override it.
    let hosted = &state.config.host_settings["click"];
    let v = if !hosted.is_null() { hosted } else { &local };
    if v.is_null() || v.get("disabled") == Some(&Value::Bool(true)) {
        return Ok(None);
    }
    let c: Connection =
        serde_json::from_value(v.clone()).map_err(|_| fail("Click configuration is incomplete"))?;
    c.validate(&state.site_url())?;
    Ok(Some(c))
}
async fn capture_target(tx: &mut sqlx::AnyConnection, c: Option<&Connection>) -> Result<()> {
    let target = c.map(|c| c.website_id.as_str()).unwrap_or("");
    // This write obtains the row lock before inspecting its previous target,
    // including against source triggers in another in-flight CMS request.
    sqlx::query("UPDATE click_connection SET capture=0 WHERE id=1")
        .execute(&mut *tx)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    let previous: String =
        sqlx::query_scalar("SELECT target_website FROM click_connection WHERE id=1")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| GhostError::Db(e.to_string()))?;
    if !previous.is_empty() && previous != target {
        sqlx::query("DELETE FROM click_outbox")
            .execute(&mut *tx)
            .await
            .map_err(|e| GhostError::Db(e.to_string()))?;
    }
    sqlx::query("UPDATE click_connection SET capture=?,target_website=? WHERE id=1")
        .bind(i64::from(c.is_some()))
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}
async fn save(state: &AppState, c: Option<&Connection>) -> Result<()> {
    let config = if let Some(c) = c {
        serde_json::to_string(c).map_err(|e| GhostError::Internal(e.to_string()))?
    } else {
        "{\"disabled\":true}".into()
    };
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    capture_target(&mut tx, c).await?;
    sqlx::query("UPDATE click_connection SET config=? WHERE id=1")
        .bind(config)
        .execute(&mut *tx)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}
async fn register(state: &AppState, c: &Connection) -> Result<Value> {
    c.validate(&state.site_url())?;
    let (_, public) = identity::keys(state).await?;
    let jwk = identity::jwk_of(&public).ok_or_else(|| fail("CMS signing key is unavailable"))?;
    c.request(
        "connect",
        &json!({"issuer":format!("{}/ghost/api/admin/",state.site_url()),"jwks":{"keys":[jwk]}}),
    )
    .await
}
static ACTION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Explicit native Admin action; only the Admin handler calls this after
/// owner authorization and CSRF checks. No setting is saved on failed setup.
pub async fn connect(state: &AppState, mut c: Connection) -> Result<()> {
    let _guard = ACTION.lock().await;
    if let Some(previous) = connection(state).await? {
        if previous.website_id != c.website_id
            || previous.base_url.trim_end_matches('/') != c.base_url.trim_end_matches('/')
        {
            return Err(fail(
                "Disconnect the previous website before connecting another",
            ));
        }
    }
    let result = register(state, &c).await?;
    c.collection_id = result["collection_id"]
        .as_str()
        .ok_or_else(|| fail("Missing public collection ID"))?
        .into();
    c.pending_key = None;
    save(state, Some(&c)).await
}
pub async fn disconnect(state: &AppState) -> Result<()> {
    let _guard = ACTION.lock().await;
    if let Some(c) = connection(state).await? {
        c.request("disconnect", &json!({})).await?;
    }
    save(state, None).await?;
    // Disconnected events must never be sent to a different future website.
    sqlx::query("DELETE FROM click_outbox")
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}
pub async fn rotate(state: &AppState) -> Result<()> {
    let _guard = ACTION.lock().await;
    let mut c = connection(state)
        .await?
        .ok_or_else(|| fail("Connect Click first"))?;
    if c.pending_key.is_none() {
        use rand::RngCore;
        let mut random = [0u8; 32];
        rand::rng().fill_bytes(&mut random);
        c.pending_key = Some(hex::encode(random));
        save(state, Some(&c)).await?;
    }
    let new = c.pending_key.clone().unwrap();
    let mut trial = c.clone();
    trial.integration_key = new.clone();
    // A successful replacement followed by a lost response is recovered by
    // trying to register with the persisted replacement before the old key.
    if register(state, &trial).await.is_err() {
        let response = http()
            .post(c.endpoint("rotate"))
            .bearer_auth(&c.integration_key)
            .header("x-click-replacement-key", &new)
            .send()
            .await
            .map_err(|_| fail("Click is unavailable; retry rotation"))?;
        if !response.status().is_success() {
            return Err(fail("Click refused key rotation; retry or reconnect"));
        }
    }
    c.integration_key = new;
    c.pending_key = None;
    register(state, &c).await?;
    save(state, Some(&c)).await
}
/// Return only public collector attributes to the renderer.
pub async fn tracker(state: &AppState) -> Result<Option<Value>> {
    if !state.settings.get_bool("web_analytics") || state.limits.is_disabled("limitAnalytics") {
        return Ok(None);
    }
    Ok(connection(state).await?.filter(|c|!c.collection_id.is_empty()).map(|c|json!({"base":c.base_url.trim_end_matches('/'),"collection":c.collection_id,"consent":c.consent_required,"engagement":c.engagement,"persistent":c.persistent_identity})))
}
pub async fn report(state: &AppState, staff_id: &str, role: &str, days: u32) -> Result<Value> {
    report_for_page(state, staff_id, role, days, None).await
}
pub async fn report_for_page(
    state: &AppState,
    staff_id: &str,
    role: &str,
    days: u32,
    post_path: Option<&str>,
) -> Result<Value> {
    report_view(state, staff_id, role, days, post_path, "traffic").await
}
pub async fn report_view(
    state: &AppState,
    staff_id: &str,
    role: &str,
    days: u32,
    post_path: Option<&str>,
    view: &str,
) -> Result<Value> {
    let c = connection(state)
        .await?
        .ok_or_else(|| fail("Vuuut Click is not connected"))?;
    let access = read_access(state, &c, staff_id, role).await?;
    let client = http();
    let mut result = serde_json::Map::new();
    let endpoints: &[(&str, &str)] = match view {
        "traffic" => &[
            ("overview", "overview"),
            ("pages", "breakdown/pages"),
            ("sources", "breakdown/referrers"),
            ("countries", "breakdown/countries"),
            ("devices", "breakdown/devices"),
            ("events", "breakdown/events"),
            ("revenue", "revenue"),
            ("native", "native-report"),
            ("newsletters", "newsletters"),
            ("comparison", "comparison"),
        ],
        "post" => &[("native", "native-report")],
        "content" => &[("content", "content"), ("comparison", "comparison")],
        "retention" => &[("retention", "retention")],
        "campaigns" => &[("campaigns", "campaign-costs")],
        "performance" => &[("performance", "vitals/comparison")],
        "newsletters" => &[("newsletters", "newsletters")],
        _ => return Err(fail("Choose a native report")),
    };
    use futures::{StreamExt, TryStreamExt};
    let mut requests = Vec::new();
    for &(key, path) in endpoints {
        let mut request = client
            .get(format!(
                "{}/v1/websites/{}/{path}",
                c.base_url.trim_end_matches('/'),
                c.website_id
            ))
            .bearer_auth(&access)
            .query(&[("days", days.clamp(1, 90))]);
        if path == "native-report" {
            if let Some(path) = post_path {
                request = request.query(&[("path", path)]);
            }
        }
        requests.push((key.to_owned(), request));
    }
    let reports: Vec<(String, Value)> = futures::stream::iter(requests)
        .map(|(key, request)| fetch_report(key, request))
        .buffer_unordered(3)
        .try_collect()
        .await?;
    for (key, data) in reports {
        result.insert(key, data);
    }
    Ok(Value::Object(result))
}
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = drain_once(&state).await {
                tracing::warn!("Click connector: {}", e.message());
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}
/// One bounded batch, with stable opaque event IDs and persisted exponential
/// retries. The single CMS worker is serialized with explicit Admin actions.
pub async fn drain_once(state: &AppState) -> Result<usize> {
    let _guard = ACTION.lock().await;
    let mut c = connection(state).await?;
    // Managed tenants start without anyone visiting Admin. Register the CMS
    // signing key and recover a missing collector ID during background setup.
    if let Some(current) = c.as_mut() {
        let registered = register(state, current).await?;
        let collection = registered["collection_id"]
            .as_str()
            .ok_or_else(|| fail("Missing public collection ID"))?;
        if state.config.host_settings["click"].is_null() && current.collection_id != collection {
            current.collection_id = collection.into();
            save(state, Some(current)).await?;
        }
    }
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    capture_target(&mut tx, c.as_ref()).await?;
    tx.commit()
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    let Some(c) = c else { return Ok(0) };
    let now = ghost_core::dates::now_db();
    type Pending = (
        String,
        String,
        Option<i64>,
        Option<String>,
        String,
        i64,
        String,
        String,
    );
    let rows:Vec<Pending>=sqlx::query_as("SELECT id,kind,amount_minor,currency,occurred_at,attempts,edition_id,recipient_id FROM click_outbox WHERE state='pending' AND available_at<=? ORDER BY available_at,id LIMIT 50")
        .bind(&now).fetch_all(&state.db).await.map_err(|e|GhostError::Db(e.to_string()))?;
    let mut delivered = 0;
    for (id, kind, amount, currency, occurred, attempts, edition, recipient) in rows {
        let timestamp = chrono::DateTime::parse_from_rfc3339(&occurred)
            .map(|d| d.timestamp_millis())
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&occurred, "%Y-%m-%d %H:%M:%S")
                    .map(|d| d.and_utc().timestamp_millis())
            })
            .map_err(|_| fail("Invalid CMS event timestamp"))?;
        let external = hex::encode(Sha256::digest(id.as_bytes()));
        let mut body = json!({"external_id":external,"kind":kind,"amount_minor":amount,"currency":currency,"occurred_at_ms":timestamp});
        if !edition.is_empty() && !recipient.is_empty() {
            body["newsletter_id"] = json!(edition_hash(&c.website_id, &edition));
            body["recipient_id"] = json!(hex::encode(Sha256::digest(
                format!("{}:edition:{edition}:recipient:{recipient}", c.website_id).as_bytes()
            )));
        }
        let payload = serde_json::to_vec(&body).map_err(|_| fail("Could not encode CMS event"))?;
        use hmac::{Hmac, Mac};
        let time = chrono::Utc::now().timestamp();
        let mut mac =
            Hmac::<Sha256>::new_from_slice(c.integration_key.as_bytes()).expect("HMAC key");
        mac.update(format!("{time}.").as_bytes());
        mac.update(&payload);
        let signature = format!("t={time},v1={}", hex::encode(mac.finalize().into_bytes()));
        let response = http()
            .post(c.endpoint("events"))
            .bearer_auth(&c.integration_key)
            .header("content-type", "application/json")
            .header("x-click-signature", signature)
            .body(payload)
            .send()
            .await;
        let status = response.as_ref().ok().map(|r| r.status());
        if status == Some(reqwest::StatusCode::ACCEPTED) {
            sqlx::query("DELETE FROM click_outbox WHERE id=?")
                .bind(&id)
                .execute(&state.db)
                .await
                .map_err(|e| GhostError::Db(e.to_string()))?;
            delivered += 1;
        } else {
            let permanent = status.is_some_and(|s| {
                s == reqwest::StatusCode::BAD_REQUEST
                    || s == reqwest::StatusCode::UNPROCESSABLE_ENTITY
            });
            let attempts = attempts + 1;
            let delay = 5i64 * 2i64.pow(attempts.min(10) as u32);
            let next = (chrono::Utc::now() + chrono::Duration::seconds(delay))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            sqlx::query(
                "UPDATE click_outbox SET attempts=?,state=?,available_at=?,last_error=? WHERE id=?",
            )
            .bind(attempts)
            .bind(if permanent || attempts >= 20 {
                "dead"
            } else {
                "pending"
            })
            .bind(next)
            .bind(if permanent {
                "Event refused; check timestamp and currency"
            } else {
                "Delivery unavailable; check connector and allowance"
            })
            .bind(&id)
            .execute(&state.db)
            .await
            .map_err(|e| GhostError::Db(e.to_string()))?;
            // Preserve ordering and avoid hammering an unavailable service.
            if !permanent {
                break;
            }
        }
    }
    // Seven-day receiver bound: never send old data with a fabricated date.
    sqlx::query("DELETE FROM click_outbox WHERE state='dead' AND occurred_at<?")
        .bind(
            (chrono::Utc::now() - chrono::Duration::days(90))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(delivered)
}
pub async fn retry(state: &AppState) -> Result<()> {
    sqlx::query(
        "UPDATE click_outbox SET state='pending',attempts=0,available_at=?,last_error=NULL",
    )
    .bind(ghost_core::dates::now_db())
    .execute(&state.db)
    .await
    .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

/// Explicit owner-requested upgrade of an existing Stripe webhook subscription.
pub async fn sync_stripe_webhooks(state: &AppState) -> Result<()> {
    let cfg = ghost_stripe::StripeConfig::from_settings(&state.settings)
        .ok_or_else(|| fail("Connect Stripe first"))?;
    let id = state
        .settings
        .get_str("members_stripe_webhook_id")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| fail("Register the Stripe webhook in payment settings first"))?;
    ghost_stripe::update_webhook_events(&cfg, &id)
        .await
        .map_err(|_| fail("Stripe could not update the webhook event subscription; try again"))
}

/// Local CMS title resolution uses the same opaque edition namespace as delivery.
pub fn edition_hash(website: &str, edition: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{website}:edition:{edition}").as_bytes(),
    ))
}

/// Owner-authorized CMS action; its integration credential stays server-side.
pub async fn record_cost(state: &AppState, input: &Value) -> Result<()> {
    let _guard = ACTION.lock().await;
    let c = connection(state)
        .await?
        .ok_or_else(|| fail("Connect a website first"))?;
    let response = http()
        .post(format!(
            "{}/v1/websites/{}/campaign-costs",
            c.base_url.trim_end_matches('/'),
            c.website_id
        ))
        .bearer_auth(&c.integration_key)
        .json(input)
        .send()
        .await
        .map_err(|_| fail("Click is unavailable; retry this cost record with the same version"))?;
    if !response.status().is_success() {
        return Err(fail(
            "Click refused the cost record; check its source, date, currency and version",
        ));
    }
    Ok(())
}

async fn read_access(
    state: &AppState,
    c: &Connection,
    staff_id: &str,
    role: &str,
) -> Result<String> {
    register(state, c).await?;
    let token = identity::click_assertion(
        state,
        staff_id,
        role,
        &c.website_id,
        &format!("{}/native/v1", c.base_url.trim_end_matches('/')),
    )
    .await?;
    let exchanged = c.request("exchange", &json!({"token":token})).await?;
    let access = exchanged["access_token"]
        .as_str()
        .ok_or_else(|| fail("Click did not grant report access"))?;
    Ok(access.to_owned())
}

/// CMS compatibility reports keep private credentials and read tokens server-side.
pub async fn cms_stats(
    state: &AppState,
    staff_id: &str,
    role: &str,
    metric: &str,
    from: &str,
    to: &str,
    path: Option<&str>,
) -> Result<Value> {
    let c = connection(state)
        .await?
        .ok_or_else(|| fail("Click is not connected"))?;
    let access = read_access(state, &c, staff_id, role).await?;
    let mut request = http()
        .get(format!(
            "{}/v1/websites/{}/native-report",
            c.base_url.trim_end_matches('/'),
            c.website_id
        ))
        .bearer_auth(access)
        .query(&[("metric", metric), ("date_from", from), ("date_to", to)]);
    if let Some(path) = path {
        request = request.query(&[("path", path)]);
    }
    let response = request
        .send()
        .await
        .map_err(|_| fail("Click reports are temporarily unavailable"))?;
    if !response.status().is_success() {
        return Err(fail("Click could not load the CMS report"));
    }
    response
        .json()
        .await
        .map_err(|_| fail("Click returned an invalid CMS report"))
}

async fn fetch_report(key: String, request: reqwest::RequestBuilder) -> Result<(String, Value)> {
    let response = request
        .send()
        .await
        .map_err(|_| fail("Click reports are temporarily unavailable"))?;
    if !response.status().is_success() {
        return Err(fail("Click could not load the report; try again shortly"));
    }
    let data = response
        .json()
        .await
        .map_err(|_| fail("Click returned an invalid report"))?;
    Ok((key, data))
}
