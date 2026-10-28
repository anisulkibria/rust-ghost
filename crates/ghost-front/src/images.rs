//! On-demand native image resizing, with bounded decoding and a disk cache.
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use ghost_api::AppState;
use image::{ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::{Component, Path as FilePath},
    sync::Arc,
};
use tokio::sync::Semaphore;
static WORKERS: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
fn size(spec: &str) -> Option<(u32, u32)> {
    let (width, height) = if let Some(spec) = spec.strip_prefix('w') {
        spec.split_once('h').unwrap_or((spec, ""))
    } else if let Some(spec) = spec.strip_prefix('h') {
        ("", spec)
    } else {
        spec.split_once('x')?
    };
    let parse = |value: &str| {
        if value.is_empty() {
            Some(0)
        } else {
            value.parse::<u32>().ok().filter(|n| *n <= 4096)
        }
    };
    let width = parse(width)?;
    let height = parse(height)?;
    (width > 0 || height > 0).then_some((width, height))
}

fn resize(
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    requested: Option<ImageFormat>,
) -> Result<(Vec<u8>, ImageFormat), String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = requested
        .or(reader.format())
        .ok_or("Unknown image format")?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    let width = if width == 0 {
        decoded.width()
    } else {
        width.min(decoded.width())
    };
    let height = if height == 0 {
        decoded.height()
    } else {
        height.min(decoded.height())
    };
    let resized = decoded.resize(width, height, image::imageops::FilterType::Lanczos3);
    let resized = if format == ImageFormat::Jpeg {
        image::DynamicImage::ImageRgb8(resized.to_rgb8())
    } else {
        resized
    };
    let mut output = Cursor::new(Vec::new());
    resized
        .write_to(&mut output, format)
        .map_err(|e| e.to_string())?;
    Ok((output.into_inner(), format))
}
pub async fn serve(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let Some(rest) = path.strip_prefix("size/") else {
        return super::routes::original_image(&state, &path);
    };
    let Some((spec, relative)) = rest.split_once('/') else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some((width, height)) = size(spec) else {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid image dimensions; maximum 4096 pixels",
        )
            .into_response();
    };
    let (requested, relative) = if let Some(formatted) = relative.strip_prefix("format/") {
        let Some((format, path)) = formatted.split_once('/') else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let format = match format {
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "png" => ImageFormat::Png,
            "webp" => ImageFormat::WebP,
            "gif" => ImageFormat::Gif,
            _ => {
                return (
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "Unsupported image output format",
                )
                    .into_response()
            }
        };
        (Some(format), path)
    } else {
        (None, relative)
    };
    if FilePath::new(relative)
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let root = state.config.images_dir();
    let root = match tokio::fs::canonicalize(&root).await {
        Ok(p) => p,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let original = match tokio::fs::canonicalize(root.join(relative)).await {
        Ok(p) if p.starts_with(&root) => p,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    // Preserve animation and vector semantics rather than flattening them.
    let extension = original
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if matches!(extension.as_str(), "svg" | "gif" | "avif") {
        return super::routes::original_image(&state, relative);
    }
    let meta = match tokio::fs::metadata(&original).await {
        Ok(m) if m.len() <= 25 * 1024 * 1024 => m,
        _ => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|m| m.as_nanos())
        .unwrap_or_default();
    let output = requested
        .or_else(|| ImageFormat::from_extension(&extension))
        .unwrap_or(ImageFormat::Png);
    let output_extension = match output {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => "webp",
        ImageFormat::Gif => "gif",
        _ => "png",
    };
    let key = hex::encode(Sha256::digest(
        format!(
            "{relative}:{spec}:{output_extension}:{}:{modified}",
            meta.len()
        )
        .as_bytes(),
    ));
    let cache = state
        .config
        .content_path()
        .join("cache/resized-images")
        .join(format!("{key}.{output_extension}"));
    let bytes = if let Ok(bytes) = tokio::fs::read(&cache).await {
        bytes
    } else {
        let workers = WORKERS.get_or_init(|| Arc::new(Semaphore::new(2))).clone();
        let Ok(_permit) = workers.acquire_owned().await else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        let bytes = match tokio::fs::read(&original).await {
            Ok(b) => b,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };
        let bytes =
            match tokio::task::spawn_blocking(move || resize(bytes, width, height, Some(output)))
                .await
            {
                Ok(Ok((bytes, _))) => bytes,
                _ => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Could not resize this image",
                    )
                        .into_response()
                }
            };
        if let Some(parent) = cache.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let temporary = cache.with_extension(format!("{}.tmp", ghost_core::ids::new_id()));
        if tokio::fs::write(&temporary, &bytes).await.is_ok() {
            if tokio::fs::rename(&temporary, &cache).await.is_err() {
                let _ = tokio::fs::remove_file(&temporary).await;
            }
        }
        bytes
    };
    let mime = match output {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Gif => "image/gif",
        _ => "image/png",
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resizing_changes_bytes_and_preserves_aspect_ratio() {
        let input = image::DynamicImage::ImageRgb8(image::RgbImage::new(600, 900));
        let mut bytes = Cursor::new(Vec::new());
        input.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let (bytes, _) = resize(bytes.into_inner(), 200, 0, None).unwrap();
        let result = image::load_from_memory(&bytes).unwrap();
        assert_eq!((result.width(), result.height()), (200, 300));
    }
    #[test]
    fn dimensions_are_bounded() {
        assert_eq!(size("200x"), Some((200, 0)));
        assert_eq!(size("w200"), Some((200, 0)));
        assert_eq!(size("h200"), Some((0, 200)));
        assert_eq!(size("w200h300"), Some((200, 300)));
        assert_eq!(size("x200"), Some((0, 200)));
        for invalid in ["0x0", "999999x1", "-2x1", "100", "100x200/secret"] {
            assert!(size(invalid).is_none());
        }
    }
}
