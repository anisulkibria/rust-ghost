//! Upload-time image optimisation — the decoding half of Settings → Images.
//!
//! The decisions are [`ghost_core::images`] and are tested there. This file
//! only does the work: decode, turn the right way up, scale, re-encode, and
//! hand the result back if it earned its place. Both entry points are
//! **blocking** — run them on `spawn_blocking`, never on a request's task.
//!
//! Called from two places:
//!
//! - [`crate::images::store`], for every upload, before it is written. One
//!   function, so the editor's drag-and-drop, the media library and the Admin
//!   API all get the same treatment.
//! - The Media module's "Optimise existing images", which runs the same
//!   function over files already in the library.

use ghost_core::images::{self as rules, ImageSettings};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::io::Cursor;

/// What came out, and what was done to get it.
#[derive(Debug, Clone)]
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub resized: bool,
    pub stripped: bool,
}

/// The same bounds the on-demand resizer decodes under. A 200 MB "PNG" that
/// claims to be 60,000 pixels square is refused rather than allocated.
fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits
}

/// Optimise one image, or say there was nothing worth doing.
///
/// `None` means "store the original": the format is not one this rewrites,
/// the bytes would not decode, or the rewrite did not earn its place (see
/// `rules::worth_keeping`). An error never reaches the caller — a picture
/// that cannot be optimised is still a picture, and the upload goes ahead.
pub fn optimize(bytes: &[u8], extension: &str, settings: &ImageSettings) -> Option<Optimized> {
    if !settings.optimize || !rules::optimizable(extension) {
        return None;
    }
    let is_jpeg = matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg");
    let had_metadata = is_jpeg && rules::jpeg_has_metadata(bytes);

    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder().ok()?;
    // Read the orientation BEFORE the pixels: once the metadata is stripped
    // nothing records which way up the camera was held, and a phone photo
    // taken on its side would be stored sideways.
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }

    let (width, height) = (image.width(), image.height());
    let target = rules::target_size(width, height, settings.max_dimension);
    if let Some((w, h)) = target {
        // Lanczos3, as the on-demand resizer uses, so a stored image and its
        // `/size/` variants are drawn by the same filter.
        image = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    }

    let mut out = Vec::new();
    if is_jpeg {
        // JPEG has no alpha; flattening here rather than letting the encoder
        // refuse an RGBA buffer from a PNG-in-disguise.
        let rgb = DynamicImage::ImageRgb8(image.to_rgb8());
        JpegEncoder::new_with_quality(&mut out, settings.jpeg_quality)
            .encode_image(&rgb)
            .ok()?;
    } else {
        // PNG is lossless whatever is done to it; the only lever is effort.
        // Best compression with adaptive filtering is slow per file but each
        // file is compressed once.
        let encoder = PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive);
        image.write_with_encoder(encoder).ok()?;
    }

    let resized = target.is_some();
    // A re-encode through `image` writes no EXIF or XMP, so any metadata the
    // original carried is gone — counted as "stripped" only when the setting
    // asked for it, since that is what makes keeping a larger file worth it.
    let stripped = settings.strip_metadata && had_metadata;
    if !rules::worth_keeping(bytes.len(), out.len(), resized, stripped) {
        return None;
    }
    let (width, height) = (image.width(), image.height());
    Some(Optimized { bytes: out, width, height, resized, stripped })
}

// ---------------------------------------------------------------------------
// an image's other versions
// ---------------------------------------------------------------------------

/// Where the on-demand resizer caches `/content/images/size/…` copies.
pub fn resized_cache_dir(state: &crate::AppState) -> std::path::PathBuf {
    state.config.content_path().join("cache/resized-images")
}

/// The filename prefix every resized copy of one image shares.
///
/// The cache file's name is otherwise a hash of the path *plus* the size, the
/// format and the original's length and modified time — which is right for
/// invalidation and useless for deleting: nothing that knows only the image
/// can find its copies. Prefixing with a hash of the path alone makes "every
/// cached size of this image" one directory listing. `rest` is the path under
/// `content/images`, e.g. `2026/11/cover-ab12.jpg` — the same string the
/// resizer and the media library both hold.
pub fn resized_cache_prefix(rest: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(rest.as_bytes()));
    format!("{}-", &digest[..16])
}

/// One copy of an image other than the image itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub kind: VersionKind,
    pub path: std::path::PathBuf,
    pub bytes: u64,
    /// Where the public site serves it, without the base path — `None` for
    /// the private original, which is deliberately never served.
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionKind {
    /// The original optimisation kept, in `content/data/image-originals`.
    Original,
    /// Ghost's `_o` original beside the image, on a site imported from Ghost.
    GhostOriginal,
    /// Ghost's on-disk resized copy: the size spec (`w600`) and, when it was
    /// converted, the format (`webp`).
    GhostSize { spec: String, format: Option<String> },
    /// This site's own cached resize. Which size it is lives inside a hash,
    /// so it can be counted and deleted but not named.
    Cached,
}

/// Ghost's name for an image's unoptimised original: `cover.jpg` → `cover_o.jpg`.
pub fn ghost_original_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}_o.{ext}"),
        None => format!("{name}_o"),
    }
}

/// Every version of an image other than the image itself.
///
/// An image here can exist in up to five places. Deleting only the first
/// leaves the other four on disk — one of them public, and one holding the
/// GPS position optimisation stripped — so the media library's delete and its
/// item page both ask this one function, and cannot disagree about what
/// "every version" means:
///
/// 1. `content/images/<rest>` — the image. Not listed; the caller has it.
/// 2. `content/data/image-originals/<rest>` — the original optimisation kept.
/// 3. `content/images/<dir>/<stem>_o.<ext>` — Ghost's own original, on a site
///    imported from Ghost.
/// 4. `content/images/size/<spec>/[format/<fmt>/]<rest>` — Ghost's resized
///    copies, on a site imported from Ghost.
/// 5. `content/cache/resized-images/<prefix>…` — this site's resized copies.
///
/// `rest` has been through `parse_location`, so it holds no `..` and joining it
/// cannot leave the directory it is joined to.
pub async fn find_versions(state: &crate::AppState, rest: &str) -> Vec<Version> {
    let mut out = Vec::new();
    let join = |root: std::path::PathBuf| rest.split('/').fold(root, |p, seg| p.join(seg));
    let size_of = |path: std::path::PathBuf| async move {
        tokio::fs::metadata(&path).await.ok().filter(|m| m.is_file()).map(|m| (path, m.len()))
    };

    if let Some((path, bytes)) = size_of(join(state.config.data_dir().join("image-originals"))).await {
        out.push(Version { kind: VersionKind::Original, path, bytes, url: None });
    }

    let image = join(state.config.images_dir());
    if let (Some(dir), Some(name)) = (image.parent(), image.file_name().and_then(|n| n.to_str())) {
        let o = ghost_original_name(name);
        if let Some((path, bytes)) = size_of(dir.join(&o)).await {
            let folder = rest.rsplit_once('/').map(|(f, _)| format!("{f}/")).unwrap_or_default();
            out.push(Version {
                kind: VersionKind::GhostOriginal,
                path,
                bytes,
                url: Some(format!("/content/images/{folder}{o}")),
            });
        }
    }

    let sizes = state.config.images_dir().join("size");
    if let Ok(mut specs) = tokio::fs::read_dir(&sizes).await {
        while let Ok(Some(spec)) = specs.next_entry().await {
            if !spec.file_type().await.is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let spec_name = spec.file_name().to_string_lossy().into_owned();
            if let Some((path, bytes)) = size_of(join(spec.path())).await {
                out.push(Version {
                    kind: VersionKind::GhostSize { spec: spec_name.clone(), format: None },
                    path,
                    bytes,
                    url: Some(format!("/content/images/size/{spec_name}/{rest}")),
                });
            }
            if let Ok(mut formats) = tokio::fs::read_dir(spec.path().join("format")).await {
                while let Ok(Some(format)) = formats.next_entry().await {
                    if !format.file_type().await.is_ok_and(|t| t.is_dir()) {
                        continue;
                    }
                    let fmt = format.file_name().to_string_lossy().into_owned();
                    if let Some((path, bytes)) = size_of(join(format.path())).await {
                        out.push(Version {
                            kind: VersionKind::GhostSize { spec: spec_name.clone(), format: Some(fmt.clone()) },
                            path,
                            bytes,
                            url: Some(format!("/content/images/size/{spec_name}/format/{fmt}/{rest}")),
                        });
                    }
                }
            }
        }
    }

    let prefix = resized_cache_prefix(rest);
    if let Ok(mut cached) = tokio::fs::read_dir(resized_cache_dir(state)).await {
        while let Ok(Some(entry)) = cached.next_entry().await {
            if entry.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)) {
                if let Some((path, bytes)) = size_of(entry.path()).await {
                    out.push(Version { kind: VersionKind::Cached, path, bytes, url: None });
                }
            }
        }
    }
    out
}

/// An image's pixel size, read from its header alone — no decode, so it is
/// cheap enough to ask of every version on a page. Blocking.
pub fn dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

/// What [`purge_versions`] removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub removed: usize,
    pub failed: usize,
}

/// Remove every version [`find_versions`] knows about. Best-effort and
/// independent: one that fails is logged, and the rest still go.
pub async fn purge_versions(state: &crate::AppState, rest: &str) -> Purged {
    let mut purged = Purged::default();
    for version in find_versions(state, rest).await {
        match tokio::fs::remove_file(&version.path).await {
            Ok(()) => purged.removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!("media: could not remove {}: {e}", version.path.display());
                purged.failed += 1;
            }
        }
    }
    purged
}

/// Move the versions that are keyed by the image's path to its new path.
///
/// A rename keeps the picture, so it keeps its private original — moved, not
/// lost, or the library run would treat the renamed file as never optimised
/// and compress it a second time. Ghost's `_o` follows its image too. Resized
/// copies, Ghost's and this site's, are dropped instead: they are derived and
/// regenerate on demand at the new address.
pub async fn move_versions(state: &crate::AppState, from: &str, to: &str) {
    let join = |root: std::path::PathBuf, rest: &str| rest.split('/').fold(root, |p, seg| p.join(seg));
    for version in find_versions(state, from).await {
        let target = match version.kind {
            VersionKind::Original => Some(join(state.config.data_dir().join("image-originals"), to)),
            VersionKind::GhostOriginal => {
                let image = join(state.config.images_dir(), to);
                match (image.parent(), image.file_name().and_then(|n| n.to_str())) {
                    (Some(dir), Some(name)) => Some(dir.join(ghost_original_name(name))),
                    _ => None,
                }
            }
            VersionKind::GhostSize { .. } | VersionKind::Cached => None,
        };
        let moved = match target {
            Some(target) => {
                if let Some(parent) = target.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                tokio::fs::rename(&version.path, &target).await
            }
            None => tokio::fs::remove_file(&version.path).await,
        };
        if let Err(e) = moved {
            tracing::warn!("media: could not carry {} across a rename: {e}", version.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> ImageSettings {
        ImageSettings { optimize: true, max_dimension: 1000, jpeg_quality: 80, strip_metadata: true }
    }

    fn jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
        // A gradient rather than a flat colour, so the encoder has real work
        // and quality changes the size.
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, quality)
            .encode_image(&DynamicImage::ImageRgb8(img))
            .unwrap();
        out
    }

    #[test]
    fn every_size_of_one_image_shares_a_prefix_and_no_other_image_does() {
        let a = resized_cache_prefix("2026/11/cover.jpg");
        assert_eq!(a, resized_cache_prefix("2026/11/cover.jpg"), "stable");
        assert_ne!(a, resized_cache_prefix("2026/11/cover2.jpg"));
        assert_eq!(a.len(), 17);
        assert!(a.ends_with('-'));
    }

    #[test]
    fn ghost_names_its_original_with_an_o_before_the_extension() {
        assert_eq!(ghost_original_name("cover-ab12.jpg"), "cover-ab12_o.jpg");
        assert_eq!(ghost_original_name("archive.tar.gz"), "archive.tar_o.gz");
        assert_eq!(ghost_original_name("noext"), "noext_o");
    }

    #[test]
    fn switched_off_it_never_touches_anything() {
        let mut settings = on();
        settings.optimize = false;
        assert!(optimize(&jpeg(3000, 2000, 95), "jpg", &settings).is_none());
    }

    #[test]
    fn an_oversized_image_is_scaled_to_the_limit() {
        let out = optimize(&jpeg(3000, 2000, 95), "jpg", &on()).expect("resized");
        assert!(out.resized);
        assert_eq!((out.width, out.height), (1000, 667));
        // and the result is a JPEG that decodes
        let back = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!((back.width(), back.height()), (1000, 667));
    }

    #[test]
    fn a_small_high_quality_jpeg_is_recompressed_smaller() {
        let original = jpeg(800, 600, 100);
        let out = optimize(&original, "jpg", &on()).expect("recompressed");
        assert!(!out.resized);
        assert!(out.bytes.len() < original.len(), "{} >= {}", out.bytes.len(), original.len());
    }

    #[test]
    fn a_rewrite_that_would_grow_the_file_is_thrown_away() {
        // Already at low quality and small: re-encoding at 80 can only grow
        // it, and there is no resize or metadata to justify that.
        let original = jpeg(400, 300, 40);
        assert!(optimize(&original, "jpg", &on()).is_none());
    }

    #[test]
    fn formats_it_does_not_rewrite_are_left_alone() {
        let original = jpeg(3000, 2000, 95);
        assert!(optimize(&original, "gif", &on()).is_none());
        assert!(optimize(&original, "webp", &on()).is_none());
        assert!(optimize(b"not an image", "jpg", &on()).is_none(), "undecodable is stored as-is");
    }
}
