//! Admin uploads — `/ghost/api/admin/{images,media,files}/upload/`.
//!
//! Multipart in, a URL under `/content/…` out. The three families differ only
//! in where they land and what they accept: images and media are typed
//! allowlists (a video card cannot be handed a `.exe`), while files are
//! deliberately open, the way Ghost's file card is.
use axum::extract::{Multipart, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use std::path::PathBuf;

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
        .route("/thumbnail/upload", post(move |s, h, m| upload(s, h, m, Kind::Image)))
        .route("/thumbnail/upload/", post(move |s, h, m| upload(s, h, m, Kind::Image)))
        // Axum caps request bodies at 2 MB by default, which is below every
        // limit this module enforces — without this, the size check below
        // would never be the thing that rejected a large upload.
        .layer(axum::extract::DefaultBodyLimit::max(kind.max_bytes()))
}

#[derive(Clone, Copy)]
enum Kind {
    Image,
    Media,
    File,
}

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/svg+xml", "svg"),
    ("image/avif", "avif"),
];

const MEDIA_TYPES: &[(&str, &str)] = &[
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("video/ogg", "ogv"),
    ("audio/mpeg", "mp3"),
    ("audio/mp3", "mp3"),
    ("audio/ogg", "ogg"),
    ("audio/wav", "wav"),
    ("audio/vnd.wav", "wav"),
    ("audio/wave", "wav"),
    ("audio/x-wav", "wav"),
    ("audio/mp4", "m4a"),
    ("audio/x-m4a", "m4a"),
];

impl Kind {
    fn dir(self, config: &ghost_core::Config) -> PathBuf {
        match self {
            Kind::Image => config.images_dir(),
            Kind::Media => config.media_dir(),
            Kind::File => config.files_dir(),
        }
    }

    /// Where the saved file is reachable on the public site.
    fn url_prefix(self) -> &'static str {
        match self {
            Kind::Image => "/content/images",
            Kind::Media => "/content/media",
            Kind::File => "/content/files",
        }
    }

    /// Ghost returns `{images: […]}` / `{media: […]}` / `{files: […]}`;
    /// Koenig reads the array out by this name.
    fn resource(self) -> &'static str {
        match self {
            Kind::Image => "images",
            Kind::Media => "media",
            Kind::File => "files",
        }
    }

    fn max_bytes(self) -> usize {
        match self {
            Kind::Image => 25 * 1024 * 1024,
            Kind::Media => 200 * 1024 * 1024,
            Kind::File => 100 * 1024 * 1024,
        }
    }

    /// `Ok(extension)` for an accepted upload. The file card takes anything,
    /// so it keeps the extension the browser sent instead of a mapped one.
    fn extension(self, content_type: &str, filename: Option<&str>) -> Result<String, String> {
        let allowed = match self {
            Kind::Image => IMAGE_TYPES,
            Kind::Media => MEDIA_TYPES,
            Kind::File => {
                let ext = filename
                    .and_then(|f| f.rsplit_once('.'))
                    .map(|(_, e)| e.to_ascii_lowercase())
                    .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
                    .unwrap_or_else(|| "bin".into());
                return Ok(ext);
            }
        };
        allowed
            .iter()
            .find(|(mime, _)| *mime == content_type)
            .map(|(_, ext)| (*ext).to_string())
            .ok_or_else(|| format!("unsupported type {content_type}"))
    }
}

fn error(status: StatusCode, message: &str, kind: &str) -> Response {
    (status, Json(json!({ "errors": [{ "message": message, "errorType": kind }] }))).into_response()
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
        let ext = match kind.extension(&content_type, filename.as_deref()) {
            Ok(ext) => ext,
            Err(message) => return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, &message, "ValidationError"),
        };
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string(), "BadRequestError"),
        };
        if data.len() > kind.max_bytes() {
            let message = format!("file too large (max {}MB)", kind.max_bytes() / 1024 / 1024);
            return error(StatusCode::PAYLOAD_TOO_LARGE, &message, "ValidationError");
        }

        // content/<kind>/YYYY/MM/<name>-<random>.<ext>, as Ghost lays it out.
        let now = chrono::Utc::now();
        let month = format!("{:04}/{:02}", now.format("%Y"), now.format("%m"));
        let dir = kind.dir(&state.config).join(&month);
        let _ = tokio::fs::create_dir_all(&dir).await;
        let stem = filename
            .as_deref()
            .and_then(|f| f.rsplit_once('.').map(|(s, _)| s.to_string()).or_else(|| Some(f.to_string())))
            .map(|s| ghost_core::slug::safe_string(&s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "upload".into());
        let filename = format!("{stem}-{}.{ext}", ghost_core::ids::random_hex(4));
        if let Err(e) = tokio::fs::write(dir.join(&filename), &data).await {
            return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "InternalServerError");
        }

        // Browser-ready, with the install's base path: the editor drops this
        // straight into an `<img src>` and has to be able to load it. The
        // write path puts it back into stored form, so what the editor saves
        // does not carry the base path into the database.
        let url = ghost_core::url::base(&format!("{}/{month}/{filename}", kind.url_prefix()));
        // The bare array is what this server has always answered with; the
        // named envelope is Ghost's. Both are sent so either client works.
        return (
            StatusCode::CREATED,
            Json(json!({
                kind.resource(): [{ "url": url, "ref": null }],
                "url": url,
                "thumbnailUrl": url,
            })),
        )
            .into_response();
    }

    error(StatusCode::UNPROCESSABLE_ENTITY, "no file field", "ValidationError")
}
