//! Admin uploads — `/ghost/api/admin/{images,media,files}/upload/`.
//!
//! Multipart in, a URL under `/content/…` out. The three families differ only
//! in where they land and what they accept; which types each one takes, and
//! the extension a type is stored under, are
//! [`ghost_core::media::Kind`]'s business, not this module's.
//!
//! The actual storing is [`store`], which this module's JSON endpoint and the
//! admin's media manager both call. Keeping it in one function is what makes
//! "an upload lands in `content/<family>/YYYY/MM/` under a slugged name and a
//! random tail" true of every upload however it arrived — the editor's drag
//! and drop, the media library, or a no-JavaScript form post.
use axum::extract::{Multipart, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use ghost_core::media::Kind;
use serde_json::json;

use crate::state::AppState;

pub fn images_router() -> Router<AppState> {
    upload_router(Kind::Image)
}

pub fn media_router() -> Router<AppState> {
    upload_router(Kind::Media)
}

pub fn files_router() -> Router<AppState> {
    upload_router(Kind::File)
}

fn upload_router(kind: Kind) -> Router<AppState> {
    Router::new()
        .route("/upload", post(move |s, h, m| upload(s, h, m, kind)))
        .route("/upload/", post(move |s, h, m| upload(s, h, m, kind)))
        // Koenig's video/audio cards send a generated poster frame here.
        .route("/thumbnail/upload", post(move |s, h, m| upload(s, h, m, Kind::Image)).put(move |s, h, m| upload(s, h, m, Kind::Image)))
        .route("/thumbnail/upload/", post(move |s, h, m| upload(s, h, m, Kind::Image)).put(move |s, h, m| upload(s, h, m, Kind::Image)))
        // Axum caps request bodies at 2 MB by default, which is below every
        // limit this module enforces — without this, the size check below
        // would never be the thing that rejected a large upload.
        .layer(axum::extract::DefaultBodyLimit::max(kind.max_bytes()))
}

/// Ghost returns `{images: […]}` / `{media: […]}` / `{files: […]}`; Koenig
/// reads the array out by this name, which is also the family's directory.
fn resource(kind: Kind) -> &'static str {
    kind.as_str()
}

fn error(status: StatusCode, message: &str, kind: &str) -> Response {
    (status, Json(json!({ "errors": [{ "message": message, "errorType": kind }] }))).into_response()
}

/// Why an upload was refused: the status to answer with, and a sentence.
pub struct Refused {
    pub status: StatusCode,
    pub message: String,
}

impl Refused {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

/// Where one accepted upload was stored.
pub struct Stored {
    /// Browser-ready, with the install's base path on it — or, under S3, the
    /// URL object storage answered with.
    pub url: String,
    /// The stored path the media manager knows the file by,
    /// `images/2026/11/cover-ab12.jpg`. `None` under a storage adapter that
    /// put the file somewhere this server cannot list.
    pub path: Option<String>,
}

/// Store one uploaded file, wherever it came from.
///
/// Checks, in the order the request meets them: the family's type allowlist
/// (which also decides the extension written to disk), the family's size
/// ceiling, then the host's own upload limit if it has set one. Only after all
/// three does anything touch the disk.
///
/// The name is rebuilt rather than reused — the browser's stem through
/// `slug::safe_string`, the extension from the allowlist, and a short random
/// tail so two uploads of `photo.jpg` do not overwrite one another. So an
/// uploaded filename never reaches the filesystem as given.
pub async fn store(
    state: &AppState,
    kind: Kind,
    filename: Option<&str>,
    content_type: &str,
    data: &[u8],
) -> Result<Stored, Refused> {
    let ext = kind
        .extension(content_type, filename)
        .map_err(|message| Refused::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, message))?;
    if data.len() > kind.max_bytes() {
        return Err(Refused::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file too large (max {}MB)", kind.max_bytes() / 1024 / 1024),
        ));
    }
    // And the host's own ceiling, if it has set one. This one is unusual among
    // the limits: the figure being compared is the size of the file in hand,
    // not a count taken from the database, so it is passed in.
    if let Err(e) = state
        .limits
        .error_if_is_over(&state.db, "uploads", ghost_db::limits::Check::of_size(data.len() as i64))
        .await
    {
        return Err(Refused::new(StatusCode::UNPROCESSABLE_ENTITY, e.message()));
    }

    // content/<family>/YYYY/MM/<name>-<random>.<ext>, as Ghost lays it out.
    let now = chrono::Utc::now();
    let month = format!("{:04}/{:02}", now.format("%Y"), now.format("%m"));
    let dir = match kind {
        Kind::Image => state.config.images_dir(),
        Kind::Media => state.config.media_dir(),
        Kind::File => state.config.files_dir(),
    }
    .join(&month);
    let _ = tokio::fs::create_dir_all(&dir).await;
    let stem = filename
        .and_then(|f| f.rsplit_once('.').map(|(s, _)| s.to_string()).or_else(|| Some(f.to_string())))
        .map(|s| ghost_core::slug::safe_string(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "upload".into());
    // Bounded so a very long name cannot push the stored path past the length
    // the media manager's metadata key allows.
    let stem: String = stem.chars().take(60).collect();
    let name = format!("{stem}-{}.{ext}", ghost_core::ids::random_hex(4));
    if let Err(e) = tokio::fs::write(dir.join(&name), data).await {
        tracing::error!("upload: could not write {}: {e}", dir.join(&name).display());
        return Err(Refused::new(StatusCode::INTERNAL_SERVER_ERROR, "The file could not be written."));
    }

    let rest = format!("{month}/{name}");
    let path = format!("{}/{rest}", kind.as_str());
    // Browser-ready, with the install's base path: the editor drops this
    // straight into an `<img src>` and has to be able to load it. The write
    // path puts it back into stored form, so what the editor saves does not
    // carry the base path into the database.
    let local_url = ghost_core::url::base(&format!("{}/{rest}", kind.url_prefix()));
    let published = crate::storage::publish_upload(state, &path, content_type, data, local_url.clone())
        .await
        .map_err(|e| Refused::new(StatusCode::BAD_GATEWAY, e.message()))?;
    // The local original is written either way (object storage is published
    // *in addition*), so the media manager can always list it by path.
    Ok(Stored { url: published, path: Some(path) })
}

async fn upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart, kind: Kind) -> Response {
    if crate::admin::current_user(&state, &headers).await.is_none() {
        return error(StatusCode::UNAUTHORIZED, "Authentication required", "NoPermissionError");
    }

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("file").to_string();
        if name != "file" && name != "image" && !name.contains("upload") {
            continue;
        }
        let filename = field.file_name().map(str::to_string);
        let content_type = field.content_type().map(str::to_string).unwrap_or_else(|| "application/octet-stream".into());
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string(), "BadRequestError"),
        };
        let stored = match store(&state, kind, filename.as_deref(), &content_type, &data).await {
            Ok(stored) => stored,
            Err(refused) => {
                let kind = match refused.status {
                    StatusCode::INTERNAL_SERVER_ERROR => "InternalServerError",
                    StatusCode::BAD_GATEWAY => "StorageError",
                    _ => "ValidationError",
                };
                return error(refused.status, &refused.message, kind);
            }
        };
        let url = stored.url;
        // The bare array is what this server has always answered with; the
        // named envelope is Ghost's. Both are sent so either client works.
        return (
            StatusCode::CREATED,
            Json(json!({
                resource(kind): [{ "url": url, "ref": null }],
                "url": url,
                "thumbnailUrl": url,
            })),
        )
            .into_response();
    }

    error(StatusCode::UNPROCESSABLE_ENTITY, "no file field", "ValidationError")
}
