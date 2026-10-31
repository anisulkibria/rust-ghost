//! The part of Ghost's Admin API that reuses this crate's own handlers.
//!
//! Content import and export, theme management and the routing and redirect
//! files all exist here already, behind HTML forms. These are the same
//! operations as REST, so a deploy script, a migration tool or Ghost's own
//! CLI can reach them — rather than only a person with a browser.
//!
//! Deliberately thin. Where a handler already does the work, the REST route
//! points at it and the handler answers with JSON when the caller asked for
//! JSON; there is no second copy of the import, the validation or the file
//! writing to drift from the first. The router is merged into the Admin API
//! nest by the binary, so everything lands at `/ghost/api/admin/…` beside
//! its siblings in `ghost-api`.
use crate::{require_role, AppState, Role};
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            // --- database -------------------------------------------------
            // Ghost's three spellings of the export, and the import and the
            // wipe on the same path.
            .route(&format!("/db{suffix}"), get(crate::migration_export_content).post(crate::migration_import).delete(delete_all_content))
            // Ghost's self-service site export: a selected-component ZIP
            // synchronously, or a queued build for anything with media in it.
            .route(&format!("/exports{suffix}"), post(request_export).get(browse_exports))
            .route(&format!("/exports/download{suffix}"), get(download_export))
            .route(&format!("/exports/{{id}}/download{suffix}"), get(download_export_request))
            .route(&format!("/db/backup{suffix}"), post(backup))
            .route(&format!("/db/media/inline{suffix}"), post(inline_media))
            // --- themes ---------------------------------------------------
            .route(&format!("/themes{suffix}"), get(browse_themes))
            .route(&format!("/themes/active{suffix}"), get(active_theme))
            .route(&format!("/themes/install{suffix}"), post(install_theme))
            .route(&format!("/themes/upload{suffix}"), post(upload_theme))
            .route(&format!("/themes/{{name}}{suffix}"), axum::routing::delete(delete_theme))
            .route(&format!("/themes/{{name}}/activate{suffix}"), axum::routing::put(activate_theme))
            .route(&format!("/themes/{{name}}/download{suffix}"), get(download_theme))
            // --- settings files -------------------------------------------
            .route(&format!("/settings/routes/yaml{suffix}"), get(crate::routes_download).post(crate::routes_upload))
            .route(&format!("/redirects/download{suffix}"), get(crate::redirects_download))
            .route(&format!("/redirects/upload{suffix}"), post(crate::redirects_upload))
            // --- two-factor sign-in ---------------------------------------
            .route(&format!("/session/verify{suffix}"), post(resend_signin_code).put(verify_signin_code));
    }
    router
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"errors":[{"message":message,"errorType":"ValidationError"}]}))).into_response()
}

/// A request a brute-force budget refused, with the wait a client should keep.
fn throttled(blocked: &ghost_api::brute::Blocked) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({"errors":[{"message":blocked.message(),"errorType":"TooManyRequestsError"}]})),
    )
        .into_response();
    if let Ok(value) = blocked.retry_after_secs().to_string().parse() {
        response.headers_mut().insert(axum::http::header::RETRY_AFTER, value);
    }
    response
}

// ---------------------------------------------------------------------------
// database
// ---------------------------------------------------------------------------

/// `DELETE /db` — delete all content.
///
/// The destructive one. Ghost gates it on an administrator and so does this;
/// it also wants the publication's own name echoed back in `?confirm=`,
/// because a REST client has no confirmation dialog and a mistyped `curl`
/// should not empty a publication.
async fn delete_all_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(actor) => actor,
        Err(r) => return r,
    };
    let name = state.settings.site_name();
    if q.get("confirm").map(String::as_str) != Some(name.as_str()) {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("This deletes every post, page and tag. Send ?confirm={name} to proceed."),
        );
    }
    let cleared = crate::wipe_tables(&state.db, crate::CONTENT_TABLES).await;
    let _ = ghost_db::repos::actions::record(&state.db, "db", "content", &actor.user.id, "deleted").await;
    Json(json!({"db": [{"deleted_tables": cleared}]})).into_response()
}

/// `POST /db/backup` — write an export to the content directory.
///
/// Ghost's CLI calls this before an upgrade. The file lands in
/// `content/data/`, next to the database it came from.
async fn backup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let dir = state.config.content_path().join("data");
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("The backup directory could not be created: {e}"));
    }
    let name = format!("ghost-backup.{}.json", crate::export_date_stamp());
    match tokio::fs::write(dir.join(&name), crate::build_content_export(&state).await).await {
        Ok(()) => Json(json!({"db": [{"filename": name}]})).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("The backup could not be written: {e}")),
    }
}

/// `POST /db/media/inline` — rehost the remote images posts point at.
///
/// An imported post usually still links to the site it came from, so the
/// pictures disappear the day that site does. This fetches each one and
/// rewrites the post to the local copy.
async fn inline_media(State(state): State<AppState>, headers: HeaderMap, body: Option<Json<Value>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Ghost takes the domains to inline from, which is the difference
    // between "fix my import" and "download the whole web".
    let domains: Vec<String> = body
        .and_then(|Json(body)| body.get("domains").and_then(Value::as_array).cloned())
        .map(|list| list.iter().filter_map(Value::as_str).map(|d| d.trim().to_ascii_lowercase()).collect())
        .unwrap_or_default();
    let domains: Vec<String> = domains.into_iter().filter(|d| !d.is_empty()).collect();
    if domains.is_empty() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "Send the domains to inline images from.");
    }
    match crate::inline_remote_media(&state, &domains).await {
        Ok(report) => Json(json!({"db": [report]})).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
    }
}

// ---------------------------------------------------------------------------
// themes
// ---------------------------------------------------------------------------

fn theme_json(theme: &ghost_api::themes::InstalledTheme) -> Value {
    json!({
        "name": theme.name,
        "package": {"name": theme.name, "version": theme.version},
        "active": theme.active,
        // Ghost reports gscan's findings here. This port runs its own
        // template check, so a problem is reported as one error rather than
        // as gscan's rule codes, which it does not have.
        "errors": theme.error.as_ref().map(|e| json!([{"level": "error", "message": e}])).unwrap_or_else(|| json!([])),
    })
}

async fn browse_themes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let themes: Vec<Value> = ghost_api::themes::list(&state).iter().map(theme_json).collect();
    Json(json!({"themes": themes})).into_response()
}

async fn active_theme(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::list(&state).into_iter().find(|theme| theme.active) {
        Some(theme) => Json(json!({"themes": [theme_json(&theme)]})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /themes/upload` — install a theme from a zip.
async fn upload_theme(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("theme.zip").to_string();
        let Ok(bytes) = field.bytes().await else { continue };
        if bytes.is_empty() {
            continue;
        }
        return match ghost_api::themes::install_zip(&state, &filename, bytes.to_vec()).await {
            Ok(name) => installed_response(&state, &name),
            Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
        };
    }
    error(StatusCode::UNPROCESSABLE_ENTITY, "No theme file was sent.")
}

/// `POST /themes/install?source=github&ref=TryGhost/Source` — Ghost's
/// installer for its official themes.
async fn install_theme(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let reference = q.get("ref").map(String::as_str).unwrap_or("");
    let slug = reference.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    if ghost_api::themes::official(&slug).is_none() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Only Ghost's official themes can be installed by name. Upload a zip for anything else.",
        );
    }
    match ghost_api::themes::install_official(&state, &slug).await {
        Ok(name) => installed_response(&state, &name),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
    }
}

fn installed_response(state: &AppState, name: &str) -> Response {
    let installed = ghost_api::themes::list(state).into_iter().find(|theme| theme.name == name);
    let body = installed.map(|theme| theme_json(&theme)).unwrap_or_else(|| json!({"name": name}));
    (StatusCode::CREATED, Json(json!({"themes": [body]}))).into_response()
}

async fn activate_theme(State(state): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::activate(&state, &name).await {
        Ok(()) => {
            let active = ghost_api::themes::list(&state).into_iter().find(|theme| theme.name == name);
            let body = active.map(|theme| theme_json(&theme)).unwrap_or_else(|| json!({"name": name}));
            Json(json!({"themes": [body]})).into_response()
        }
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
    }
}

async fn delete_theme(State(state): State<AppState>, headers: HeaderMap, Path(name): Path<String>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::delete(&state, &name).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, &e.message()),
    }
}

// ---------------------------------------------------------------------------
// two-factor sign-in
// ---------------------------------------------------------------------------

/// `POST /session/verify` — email the sign-in code again.
///
/// Reached while holding the unverified session `POST /session` handed back,
/// which is the only thing that identifies who the code is for. Without one
/// there is nobody to send it to, and guessing from a body parameter would
/// make this an endpoint for mailing codes to other people.
async fn resend_signin_code(
    State(state): State<AppState>,
    ghost_api::brute::ClientAddr(peer): ghost_api::brute::ClientAddr,
    headers: HeaderMap,
) -> Response {
    // Sending mail on request is worth rate-limiting on its own: without this
    // the endpoint is a way to post a stream of mail to a staff address.
    // Ghost's `sendVerificationCode` budget, per IP.
    let ip = ghost_api::brute::client_ip(&headers, peer);
    if let Err(blocked) = ghost_api::brute::check(
        &state.db,
        &format!("send_verification_code:{ip}"),
        &ghost_api::brute::SEND_VERIFICATION_CODE,
    )
    .await
    {
        return throttled(&blocked);
    }
    let Some(session) = ghost_api::admin::session_id_from(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "Start signing in first.");
    };
    let pending = ghost_db::repos::sessions::get_pending_user(&state.db, &session).await.ok().flatten();
    let Some(user) = pending else {
        return error(StatusCode::UNAUTHORIZED, "That sign-in is no longer in progress.");
    };
    let Ok(Some(user)) = ghost_db::repos::users::find_by_id(&state.db, &user).await else {
        return error(StatusCode::UNAUTHORIZED, "That sign-in is no longer in progress.");
    };
    if crate::issue_signin_code(&state, &session, &user).await {
        StatusCode::OK.into_response()
    } else {
        error(StatusCode::INTERNAL_SERVER_ERROR, "The code could not be sent.")
    }
}

/// `PUT /session/verify` — complete the sign-in with the code.
///
/// The same check the session endpoint makes when a code rides along with
/// the password; Ghost's client calls this one when it already has a pending
/// session and only needs to finish.
async fn verify_signin_code(
    State(state): State<AppState>,
    ghost_api::brute::ClientAddr(peer): ghost_api::brute::ClientAddr,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // A six-digit code is guessable in a million tries; Ghost's
    // `userVerification` budget is what keeps that out of reach.
    let ip = ghost_api::brute::client_ip(&headers, peer);
    let key = format!("user_verification:{ip}");
    if let Err(blocked) =
        ghost_api::brute::check(&state.db, &key, &ghost_api::brute::USER_VERIFICATION).await
    {
        return throttled(&blocked);
    }
    let Some(session) = ghost_api::admin::session_id_from(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "Start signing in first.");
    };
    let code = body
        .get("token")
        .or_else(|| body.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if code.is_empty() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "A verification code is required.");
    }
    use ghost_db::repos::sessions::CodeCheck;
    match ghost_db::repos::sessions::verify_auth_code(&state.db, &session, &code).await {
        Ok(CodeCheck::Verified) => {
            // The right code is not a guess, so it does not count against the
            // budget — a publisher who mistyped twice first is not then
            // locked out of their own admin.
            ghost_api::brute::reset(&state.db, &key).await;
            if let Ok(Some(user)) = ghost_db::repos::sessions::get_user(&state.db, &session).await {
                let _ = ghost_db::repos::users::update_last_seen(&state.db, &user).await;
            }
            StatusCode::OK.into_response()
        }
        _ => error(StatusCode::UNAUTHORIZED, "That verification code is not valid."),
    }
}

async fn download_theme(State(state):State<AppState>,headers:HeaderMap,Path(name):Path<String>)->Response{
    if let Err(r)=require_role(&state,&headers,Role::Administrator).await{return r}
    if name.is_empty()||name.starts_with('.')||!name.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'-'||c==b'_'){return error(StatusCode::UNPROCESSABLE_ENTITY,"Invalid theme name")}
    let root=match std::fs::canonicalize(state.config.themes_dir()){Ok(p)=>p,Err(_)=>return StatusCode::NOT_FOUND.into_response()};
    let path=match std::fs::canonicalize(root.join(&name)){Ok(p)if p.starts_with(&root)&&p.is_dir()=>p,_=>return StatusCode::NOT_FOUND.into_response()};
    let filename=format!("{name}.zip");
    let body=tokio::task::spawn_blocking(move||->Option<Vec<u8>>{let mut zip=zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);crate::zip_add_dir(&mut zip,&path,&name,options).ok()?;Some(zip.finish().ok()?.into_inner())}).await.ok().flatten();
    match body{Some(body)=>crate::zip_download(&filename,body),None=>error(StatusCode::INTERNAL_SERVER_ERROR,"Could not archive theme")}
}

// ---------------------------------------------------------------------------
// self-service site export
// ---------------------------------------------------------------------------

/// What `GET /exports/download` can put in an archive.
///
/// Upstream's `SYNC_EXPORT_COMPONENTS`. Media is deliberately absent: it is
/// the one component that can run to gigabytes, and a request that holds a
/// connection open while it is assembled would time out long before it
/// finished. `POST /exports` is the route for that.
const SYNC_EXPORT_COMPONENTS: &[&str] = &["members", "analytics", "content", "themes", "routes"];

/// What `POST /exports` can put in an archive — the above, plus media.
const ASYNC_EXPORT_COMPONENTS: &[&str] =
    &["members", "analytics", "content", "themes", "routes", "media"];

/// Ghost's export filename: `<site-slug>.ghost.<type>.<date>.<ext>`.
///
/// The admin reads this back off `Content-Disposition`, so the convention
/// lives in one place (upstream `utils/csv-export-filename.js`).
fn export_filename(state: &AppState, kind: &str, extension: &str) -> String {
    let slug = ghost_core::slug::safe_string(&state.settings.site_name());
    let date = crate::export_date_stamp();
    if slug.is_empty() {
        format!("ghost.{kind}.{date}.{extension}")
    } else {
        format!("{slug}.ghost.{kind}.{date}.{extension}")
    }
}

/// `GET /exports/download?components=content,themes` — the archive, now.
///
/// Absent `components` means every synchronous component; an explicitly empty
/// list is upstream's "No export components selected" validation error rather
/// than an empty zip. An unknown name is refused rather than ignored, so a
/// typo does not quietly produce a smaller archive than the caller asked for.
async fn download_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, crate::Role::Administrator).await {
        return r;
    }
    let selected: Vec<String> = match q.get("components") {
        None => SYNC_EXPORT_COMPONENTS.iter().map(|c| c.to_string()).collect(),
        Some(raw) => raw.split(',').map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect(),
    };
    if selected.is_empty() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "No export components selected");
    }
    let unknown: Vec<&str> = selected
        .iter()
        .map(String::as_str)
        .filter(|c| !SYNC_EXPORT_COMPONENTS.contains(c))
        .collect();
    if !unknown.is_empty() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("Unknown export components: {}", unknown.join(", ")),
        );
    }

    let want = move |key: &str| selected.iter().any(|c| c == key);
    // Written through a temporary file and streamed back: a themes-and-content
    // archive is modest, but "every installed theme" is unbounded in
    // principle and this costs nothing when it is not.
    let path = state
        .config
        .content_path()
        .join("exports")
        .join(format!("tmp-{}.zip", ghost_core::ids::new_id()));
    match crate::build_export_archive_to_file(&state, &want, &path).await {
        Some(size) => {
            let filename = export_filename(&state, "export", "zip");
            stream_zip_then_delete(&path, &filename, size).await
        }
        None => {
            let _ = tokio::fs::remove_file(&path).await;
            error(StatusCode::INTERNAL_SERVER_ERROR, "The export archive could not be built.")
        }
    }
}

/// Send a file as a ZIP download in chunks, removing it afterwards.
///
/// A temporary archive must not outlive its one download, and the response
/// must not require the whole file in memory — which is the entire reason the
/// archive was written to disk.
async fn stream_zip_then_delete(
    path: &std::path::Path,
    filename: &str,
    size: u64,
) -> Response {
    let Ok(file) = tokio::fs::File::open(path).await else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The export archive could not be read.");
    };
    // Unlinking now means the file is gone whatever happens to the transfer —
    // including a client that disconnects halfway — while this handle keeps it
    // readable until the body is finished with.
    let _ = tokio::fs::remove_file(path).await;

    let stream = futures::stream::unfold(Some(file), |state| async move {
        let mut file = state?;
        let mut chunk = vec![0u8; 64 * 1024];
        match tokio::io::AsyncReadExt::read(&mut file, &mut chunk).await {
            Ok(0) => None,
            Ok(read) => {
                chunk.truncate(read);
                Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(chunk)), Some(file)))
            }
            Err(e) => Some((Err(e), None)),
        }
    });
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/zip".to_string()),
            (axum::http::header::CONTENT_LENGTH, size.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}

/// `POST /exports` — queue a full export, including media.
///
/// 202 and a request id. Upstream hands this to its host service, which mails
/// a link when the archive is ready; a self-hosted install has no such
/// service, so the archive is built here in the background and the requester
/// is mailed a link to collect it from. Either way the contract is the same:
/// the response means "accepted", not "exported".
async fn request_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match require_role(&state, &headers, crate::Role::Administrator).await {
        Ok(a) => a.user,
        Err(r) => return r,
    };
    // Upstream's validation, message for message: the client needs to know
    // which of these four things it got wrong.
    let Some(components) = body.get("components").and_then(Value::as_object) else {
        return error(StatusCode::BAD_REQUEST, "components must be an object");
    };
    let unknown: Vec<&str> = components
        .keys()
        .map(String::as_str)
        .filter(|key| !ASYNC_EXPORT_COMPONENTS.contains(key))
        .collect();
    if !unknown.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("Unknown export components: {}", unknown.join(", ")),
        );
    }
    if components.values().any(|v| !v.is_boolean()) {
        return error(StatusCode::BAD_REQUEST, "Export component values must be booleans");
    }
    if !components.values().any(|v| v == &Value::Bool(true)) {
        return error(StatusCode::BAD_REQUEST, "At least one export component must be selected");
    }

    // Normalised to every known component, so the stored row records what was
    // excluded as well as what was asked for.
    let mut selection = serde_json::Map::new();
    for key in ASYNC_EXPORT_COMPONENTS {
        selection.insert((*key).to_string(), json!(components.get(*key) == Some(&json!(true))));
    }
    let selection = Value::Object(selection);

    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    if sqlx::query(
        "INSERT INTO export_requests (id, components, requested_by, status, created_at) \
         VALUES (?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(selection.to_string())
    .bind(&actor.id)
    .bind(&now)
    .execute(&state.db)
    .await
    .is_err()
    {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The export could not be queued.");
    }

    crate::spawn_export_build(state.clone(), id.clone(), selection.clone());

    (
        StatusCode::ACCEPTED,
        Json(json!({ "exports": [{
            "id": id,
            "status": "pending",
            "components": selection,
            "created_at": now,
        }]})),
    )
        .into_response()
}

/// `GET /exports` — the export requests this site has queued.
///
/// Not an upstream route; upstream's equivalent state lives on its host. A
/// queued export is useless without a way to find out whether it finished, so
/// the port answers for its own.
async fn browse_exports(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, crate::Role::Administrator).await {
        return r;
    }
    let rows = sqlx::query_as::<
        _,
        (String, String, String, Option<String>, Option<i64>, Option<String>, String, Option<String>),
    >(
        "SELECT id, components, status, filename, size_bytes, error, created_at, finished_at \
         FROM export_requests ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, components, status, filename, size, err, created_at, finished_at)| {
            json!({
                "id": id,
                "components": serde_json::from_str::<Value>(&components).unwrap_or(Value::Null),
                "status": status,
                "size_bytes": size,
                "error": err,
                "created_at": created_at,
                "finished_at": finished_at,
                // The path to collect it from, once there is something there.
                "download_url": filename.as_ref().map(|_| format!("/ghost/api/admin/exports/{id}/download/")),
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({ "exports": items }))).into_response()
}

/// `GET /exports/:id/download` — collect a finished asynchronous export.
async fn download_export_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, crate::Role::Administrator).await {
        return r;
    }
    let row: Option<(String, Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT status, filename, size_bytes FROM export_requests WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some((status, filename, size)) = row else {
        return error(StatusCode::NOT_FOUND, "Export not found.");
    };
    let Some(filename) = filename.filter(|_| status == "finished") else {
        // 409 rather than 404: the export exists, it is just not ready. A
        // client polling this needs to tell those two apart.
        return error(
            StatusCode::CONFLICT,
            &format!("That export is {status}; it is not ready to download yet."),
        );
    };
    let path = state.config.content_path().join("exports").join(&filename);
    let size = match size {
        Some(size) if size >= 0 => size as u64,
        _ => match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(_) => return error(StatusCode::NOT_FOUND, "That export's archive is no longer on disk."),
        },
    };
    // Kept on disk: an asynchronous export is collected by a link in an email
    // and may be fetched more than once, so this one is not unlinked.
    let Ok(file) = tokio::fs::File::open(&path).await else {
        return error(StatusCode::NOT_FOUND, "That export's archive is no longer on disk.");
    };
    let stream = futures::stream::unfold(Some(file), |state| async move {
        let mut file = state?;
        let mut chunk = vec![0u8; 64 * 1024];
        match tokio::io::AsyncReadExt::read(&mut file, &mut chunk).await {
            Ok(0) => None,
            Ok(read) => {
                chunk.truncate(read);
                Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(chunk)), Some(file)))
            }
            Err(e) => Some((Err(e), None)),
        }
    });
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/zip".to_string()),
            (axum::http::header::CONTENT_LENGTH, size.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}
