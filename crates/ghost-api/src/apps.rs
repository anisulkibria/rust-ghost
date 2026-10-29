//! Experimental app installation lifecycle. Only server-fetched, reviewed
//! manifests can be activated. A session rechecks the approved address and
//! suspends changes until a publisher explicitly approves the new digest.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}
fn bad(msg: &str) -> GhostError {
    GhostError::BadRequest(msg.into())
}
fn gated(state: &AppState) -> bool {
    let labs = state.settings.get("labs").unwrap_or(Value::Null);
    labs["apps"] == true
}
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    role: Role,
) -> std::result::Result<(), Response> {
    permissions::require(state, headers, role).await?;
    if !gated(state) {
        return Err(StatusCode::NOT_FOUND.into_response());
    }
    Ok(())
}
pub fn router() -> Router<AppState> {
    let mut r = Router::new();
    for s in ["", "/"] {
        r = r
            .route(&format!("/apps/installations/preview{s}"), post(preview))
            .route(
                &format!("/apps/installations{s}"),
                get(browse).post(install),
            )
            .route(
                &format!("/apps/installations/{{id}}{s}"),
                get(read).put(approve).delete(uninstall),
            )
            .route(
                &format!("/apps/installations/{{id}}/sessions{s}"),
                post(session),
            );
    }
    r
}
struct Loaded {
    url: String,
    manifest: Value,
    digest: String,
}
async fn load(state: &AppState, url: &str) -> Result<Loaded> {
    let (client, url) = crate::federation::safe_client(url).await?;
    let site = reqwest::Url::parse(&state.site_url()).map_err(|_| bad("Invalid site URL"))?;
    if url.origin() == site.origin() {
        return Err(bad("Apps must be served from a different origin"));
    }
    let mut response = client
        .get(url.clone())
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| bad("App manifest could not be fetched"))?;
    if !response.status().is_success() {
        return Err(bad("App manifest fetch failed"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| bad("Could not read app manifest"))?
    {
        if bytes.len() + chunk.len() > 128 * 1024 {
            return Err(bad("App manifest exceeds 128KB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut manifest: Value =
        serde_json::from_slice(&bytes).map_err(|_| bad("App manifest must be JSON"))?;
    validate(&mut manifest, &url, &site)?;
    let digest = hex::encode(Sha256::digest(format!("{}\n{}", url, manifest).as_bytes()));
    Ok(Loaded {
        url: url.to_string(),
        manifest,
        digest,
    })
}
fn validate(manifest: &mut Value, base: &reqwest::Url, site: &reqwest::Url) -> Result<()> {
    let object = manifest
        .as_object()
        .ok_or_else(|| bad("Expected an app manifest object"))?;
    if object.keys().any(|key| {
        ![
            "id",
            "name",
            "description",
            "author",
            "accent_color",
            "icon",
            "surfaces",
        ]
        .contains(&key.as_str())
    }) {
        return Err(bad("Unknown app manifest field"));
    }
    let id = manifest["id"].as_str().unwrap_or("");
    if id.len() > 191
        || !regex::Regex::new(r"^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)+$")
            .unwrap()
            .is_match(id)
    {
        return Err(bad("Use a reverse-domain app id"));
    }
    for (key, max) in [("name", 50), ("description", 200)] {
        let text = manifest[key].as_str().unwrap_or("");
        if text.is_empty() || text.chars().count() > max || text.contains(['<', '>', '\n', '\r']) {
            return Err(bad("Invalid app name or description"));
        }
    }
    let author = manifest["author"]["name"].as_str().unwrap_or("");
    if author.is_empty() || author.len() > 50 || author.contains(['<', '>']) {
        return Err(bad("App author is required"));
    }
    if !regex::Regex::new(r"^#[a-fA-F0-9]{6}$")
        .unwrap()
        .is_match(manifest["accent_color"].as_str().unwrap_or(""))
    {
        return Err(bad("Use a six-digit hex accent color"));
    }
    fn resolve(
        value: &mut Value,
        base: &reqwest::Url,
        site: &reqwest::Url,
        same: bool,
    ) -> Result<()> {
        let raw = value
            .as_str()
            .ok_or_else(|| bad("An app URL is required"))?;
        let url = base.join(raw).map_err(|_| bad("Invalid app URL"))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || (same && (url.origin() != base.origin() || url.origin() == site.origin()))
        {
            return Err(bad(
                "App surfaces and assets must use the manifest HTTPS origin",
            ));
        }
        *value = json!(url.as_str());
        Ok(())
    }
    resolve(&mut manifest["author"]["url"], base, site, false)?;
    let icon = &mut manifest["icon"];
    if icon.get("url").is_some() {
        if icon.get("name").is_some() {
            return Err(bad("Choose one app icon"));
        }
        resolve(&mut icon["url"], base, site, true)?;
    } else {
        let name = icon["name"].as_str().unwrap_or("");
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err(bad("Invalid app icon"));
        }
    }
    let surfaces = manifest["surfaces"]
        .as_array_mut()
        .ok_or_else(|| bad("App surfaces are required"))?;
    if surfaces.len() != 1 || surfaces[0]["type"] != "admin_page" {
        return Err(bad("Exactly one admin_page surface is supported"));
    }
    resolve(&mut surfaces[0]["url"], base, site, true)?;
    Ok(())
}
async fn resource(state: &AppState, id: &str, history: bool) -> Result<Option<Value>> {
    let row=sqlx::query_as::<_,(String,String,String,String,String,i64,String,String)>("SELECT id,app_id,status,manifest_url,manifest,revision,created_at,updated_at FROM app_installations WHERE id=?").bind(id).fetch_optional(&state.db).await.map_err(db)?;
    let Some((id, app, status, url, raw, revision, created, updated)) = row else {
        return Ok(None);
    };
    let mut value = json!({"id":id,"app_id":app,"status":status,"manifest_url":url,"manifest":serde_json::from_str::<Value>(&raw).map_err(|_|bad("Stored manifest is corrupt"))?,"revision":revision,"created_at":created,"updated_at":updated});
    if history {
        let rows=sqlx::query_as::<_,(String,String,String,String)>("SELECT manifest_url,manifest,source,created_at FROM app_manifest_history WHERE installation_id=? ORDER BY rowid DESC").bind(&id).fetch_all(&state.db).await.map_err(db)?;
        value["history"]=json!(rows.into_iter().map(|(url,raw,source,at)|json!({"manifest_url":url,"manifest":serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null),"source":source,"created_at":at})).collect::<Vec<_>>());
    }
    Ok(Some(value))
}
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Administrator).await {
        return r;
    }
    let data = body
        .pointer("/app_installation_previews/0")
        .unwrap_or(&body);
    match load(&state,data["manifest_url"].as_str().unwrap_or("")).await{Ok(v)=>Json(json!({"app_installation_previews":[{"manifest_url":v.url,"manifest":v.manifest,"digest":v.digest}]})).into_response(),Err(e)=>crate::error_json(&e)}
}
async fn browse(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Editor).await {
        return r;
    }
    let ids = match sqlx::query_scalar::<_, String>(
        "SELECT id FROM app_installations WHERE status!='uninstalled' ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(ids) => ids,
        Err(e) => return crate::error_json(&db(e)),
    };
    let mut values = Vec::new();
    for id in ids {
        match resource(&state, &id, false).await {
            Ok(Some(v)) => values.push(v),
            Err(e) => return crate::error_json(&e),
            _ => {}
        }
    }
    Json(json!({"app_installations":values})).into_response()
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Editor).await {
        return r;
    }
    match resource(
        &state,
        &id,
        q.get("include").is_some_and(|v| v == "history"),
    )
    .await
    {
        Ok(Some(v)) => Json(json!({"app_installations":[v]})).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => crate::error_json(&e),
    }
}
async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    write(state, headers, None, body).await
}
async fn approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    write(state, headers, Some(id), body).await
}
async fn write(state: AppState, headers: HeaderMap, id: Option<String>, body: Value) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Administrator).await {
        return r;
    }
    let data = body.pointer("/app_installations/0").unwrap_or(&body);
    let loaded = match load(&state, data["manifest_url"].as_str().unwrap_or("")).await {
        Ok(v) => v,
        Err(e) => return crate::error_json(&e),
    };
    if data["digest"] != loaded.digest {
        return (StatusCode::CONFLICT,Json(json!({"errors":[{"message":"The manifest changed; review the current manifest","code":"APP_MANIFEST_CHANGED"}]}))).into_response();
    }
    let now = dates::now_db();
    let new = id.is_none();
    let id = id.unwrap_or_else(ids::new_id);
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return crate::error_json(&db(e)),
    };
    let result = if new {
        sqlx::query("INSERT INTO app_installations(id,app_id,manifest,manifest_url,digest,created_at,updated_at) VALUES(?,?,?,?,?,?,?)").bind(&id).bind(loaded.manifest["id"].as_str()).bind(loaded.manifest.to_string()).bind(&loaded.url).bind(&loaded.digest).bind(&now).bind(&now).execute(&mut *tx).await
    } else {
        sqlx::query("UPDATE app_installations SET manifest=?,manifest_url=?,digest=?,status='active',revision=revision+1,updated_at=? WHERE id=? AND revision=? AND app_id=? AND status!='uninstalled'").bind(loaded.manifest.to_string()).bind(&loaded.url).bind(&loaded.digest).bind(&now).bind(&id).bind(data["revision"].as_i64().unwrap_or(-1)).bind(loaded.manifest["id"].as_str()).execute(&mut *tx).await
    };
    match result {
        Ok(row) if row.rows_affected() == 1 => {}
        Ok(_) => return StatusCode::CONFLICT.into_response(),
        Err(e) => return crate::error_json(&db(e)),
    }
    if let Err(e)=sqlx::query("INSERT INTO app_manifest_history(id,installation_id,manifest_url,manifest,digest,source,created_at) VALUES(?,?,?,?,?,?,?)").bind(ids::new_id()).bind(&id).bind(loaded.url).bind(loaded.manifest.to_string()).bind(loaded.digest).bind(if new{"install"}else{"approval"}).bind(&now).execute(&mut *tx).await{return crate::error_json(&db(e))}
    if let Err(e) = tx.commit().await {
        return crate::error_json(&db(e));
    }
    match resource(&state, &id, false).await {
        Ok(Some(v)) => (
            if new {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(json!({"app_installations":[v]})),
        )
            .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn uninstall(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Administrator).await {
        return r;
    }
    match sqlx::query("UPDATE app_installations SET status='uninstalled',revision=revision+1,updated_at=? WHERE id=? AND status!='uninstalled'").bind(dates::now_db()).bind(id).execute(&state.db).await{Ok(r)if r.rows_affected()==1=>StatusCode::NO_CONTENT.into_response(),Ok(_)=>StatusCode::NOT_FOUND.into_response(),Err(e)=>crate::error_json(&db(e))}
}
async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers, Role::Editor).await {
        return r;
    }
    let old = match resource(&state, &id, false).await {
        Ok(Some(v)) if v["status"] != "uninstalled" => v,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return crate::error_json(&e),
    };
    let current = load(&state, old["manifest_url"].as_str().unwrap_or("")).await;
    let unchanged = current
        .as_ref()
        .is_ok_and(|m| m.manifest == old["manifest"]);
    if !unchanged {
        if let Err(e)=sqlx::query("UPDATE app_installations SET status='suspended',revision=revision+1,updated_at=? WHERE id=? AND status='active'").bind(dates::now_db()).bind(&id).execute(&state.db).await{return crate::error_json(&db(e))}
    }
    match resource(&state, &id, false).await {
        Ok(Some(v)) => Json(json!({"app_sessions":[{"installation":v}]})).into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Value {
        json!({"id":"com.example.app","name":"Audio","description":"An audio tool","author":{"name":"Publisher","url":"https://example.org"},"accent_color":"#ff5500","icon":{"name":"audio-lines"},"surfaces":[{"type":"admin_page","url":"/app"}]})
    }
    #[test]
    fn manifest_urls_are_normalized_and_cannot_use_the_publication_origin() {
        let base = reqwest::Url::parse("https://apps.example.org/manifest.json").unwrap();
        let site = reqwest::Url::parse("https://publication.example.org").unwrap();
        let mut valid = manifest();
        validate(&mut valid, &base, &site).unwrap();
        assert_eq!(valid["surfaces"][0]["url"], "https://apps.example.org/app");
        let mut invalid = manifest();
        invalid["surfaces"][0]["url"] = json!("https://publication.example.org/ghost/");
        assert!(validate(&mut invalid, &base, &site).is_err());
        let mut invalid = manifest();
        invalid["surfaces"][0]["url"] = json!("javascript:alert(1)");
        assert!(validate(&mut invalid, &base, &site).is_err());
        let mut invalid = manifest();
        invalid["permissions"] = json!(["all"]);
        assert!(validate(&mut invalid, &base, &site).is_err());
    }
}
