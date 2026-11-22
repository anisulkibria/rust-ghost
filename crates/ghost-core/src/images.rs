//! Image optimisation: the rules, and Settings → Images.
//!
//! The Media module's half of what happens to a photograph between somebody's
//! camera and a reader's screen. No decoding here — that is
//! `ghost_api::image_optimize`, which needs the `image` crate — only the
//! decisions that can be wrong and are worth testing without a file: what the
//! settings mean when a row is missing, how big a resized image comes out, and
//! whether a file is worth touching at all.
//!
//! ## What is, and is not, done
//!
//! **Done at upload, when switched on:** an image larger than the limit is
//! scaled down to it; a JPEG is re-encoded at the chosen quality; and the
//! metadata a camera writes — including the GPS position of where the photo
//! was taken — is removed. The EXIF orientation is applied to the pixels
//! first, or a photo taken with the phone on its side would come out sideways
//! once the tag that said so was gone.
//!
//! **Left alone:** GIFs (re-encoding drops animation frames for no gain),
//! WebP, AVIF and SVG. The `image` crate's WebP encoder is lossless only, so
//! "converting to WebP" would make most photographs *larger*, and an
//! optimiser that grows files is worse than none.
//!
//! Responsive sizes already exist — `/content/images/size/w600/…`, resized on
//! demand and cached — so they are not this module's job.

/// The settings keys.
pub const SETTING_OPTIMIZE: &str = "images_optimize";
pub const SETTING_MAX_DIMENSION: &str = "images_max_dimension";
pub const SETTING_JPEG_QUALITY: &str = "images_jpeg_quality";
pub const SETTING_STRIP_METADATA: &str = "images_strip_metadata";

/// Ghost's own figure for "large enough for any screen this image will be
/// shown on": the widest a theme ever asks for is 2000px.
pub const DEFAULT_MAX_DIMENSION: u32 = 2000;
/// Below this an image is a thumbnail, and capping uploads there would ruin
/// every hero image on the site.
pub const MIN_MAX_DIMENSION: u32 = 600;
/// The ceiling the on-demand resizer also refuses past.
pub const MAX_MAX_DIMENSION: u32 = 8000;

/// Visually indistinguishable from the original for photographs at screen
/// sizes, and typically a third of the bytes of a camera's 95.
pub const DEFAULT_JPEG_QUALITY: u8 = 82;
pub const MIN_JPEG_QUALITY: u8 = 40;
pub const MAX_JPEG_QUALITY: u8 = 100;

/// Settings → Images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSettings {
    /// The master switch. Off: uploads are stored exactly as they arrived,
    /// which is what the site did before this setting existed.
    pub optimize: bool,
    /// The longest side, in pixels, an upload is allowed to keep.
    pub max_dimension: u32,
    /// JPEG quality, 40–100.
    pub jpeg_quality: u8,
    /// Remove camera metadata, including GPS.
    pub strip_metadata: bool,
}

impl ImageSettings {
    /// `get` is the settings cache's string lookup.
    ///
    /// A missing row means the default — and the default for the master
    /// switch is **off**, because turning optimisation on rewrites people's
    /// uploads, and that is a decision somebody has to make. The other three
    /// default to the values the screen recommends, so switching the master
    /// on does something sensible without any further choices.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> ImageSettings {
        let flag = |key: &str, default: bool| match get(key).as_deref().map(str::trim) {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => default,
        };
        let number = |key: &str| get(key).and_then(|v| v.trim().parse::<i64>().ok());
        ImageSettings {
            optimize: flag(SETTING_OPTIMIZE, false),
            max_dimension: number(SETTING_MAX_DIMENSION)
                .map(|n| n.clamp(MIN_MAX_DIMENSION as i64, MAX_MAX_DIMENSION as i64) as u32)
                .unwrap_or(DEFAULT_MAX_DIMENSION),
            jpeg_quality: number(SETTING_JPEG_QUALITY)
                .map(|n| n.clamp(MIN_JPEG_QUALITY as i64, MAX_JPEG_QUALITY as i64) as u8)
                .unwrap_or(DEFAULT_JPEG_QUALITY),
            strip_metadata: flag(SETTING_STRIP_METADATA, true),
        }
    }
}

/// The formats this optimiser rewrites. Everything else is stored untouched —
/// see the module docs for why each of the others is left alone.
pub fn optimizable(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png")
}

/// The size an image of `width` × `height` should be stored at, or `None`
/// when it already fits.
///
/// The *longest* side is held to the limit and the other follows, so a tall
/// portrait and a wide panorama are treated alike. Never scales up, and never
/// produces a zero-pixel side however extreme the aspect ratio.
pub fn target_size(width: u32, height: u32, max: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || max == 0 {
        return None;
    }
    let longest = width.max(height);
    if longest <= max {
        return None;
    }
    let scale = max as f64 / longest as f64;
    let w = ((width as f64 * scale).round() as u32).max(1);
    let h = ((height as f64 * scale).round() as u32).max(1);
    Some((w, h))
}

/// Whether a JPEG carries camera metadata — an `APP1` segment holding EXIF
/// (where GPS lives) or XMP.
///
/// Only the segments before the image data are walked, by their declared
/// lengths, so a byte pattern that happens to occur inside the compressed
/// picture is never mistaken for one. Anything malformed reads as "no".
pub fn jpeg_has_metadata(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return false;
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return false;
        }
        let marker = bytes[at + 1];
        // Start of scan: the picture itself begins, and no metadata follows.
        if marker == 0xDA || marker == 0xD9 {
            return false;
        }
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if length < 2 || at + 2 + length > bytes.len() {
            return false;
        }
        if marker == 0xE1 {
            let payload = &bytes[at + 4..at + 2 + length];
            if payload.starts_with(b"Exif\0") || payload.starts_with(b"http://ns.adobe.com/xap/1.0/") {
                return true;
            }
        }
        at += 2 + length;
    }
    false
}

/// Whether an optimised file is worth keeping over the original.
///
/// A smaller file always is. A file that had to be **resized** or **stripped**
/// is kept even when it came out larger, because each of those was asked for
/// for its own sake — a size limit is a limit, and metadata removal is about
/// privacy, not bytes. Otherwise a re-encode that grew the file is thrown
/// away and the original stands.
pub fn worth_keeping(original_len: usize, optimized_len: usize, resized: bool, stripped: bool) -> bool {
    optimized_len < original_len || resized || stripped
}

/// "1.2 MB", "340 KB" — the savings line on the Media screen.
pub fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / KB / KB / KB)
    } else if b >= KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_mean_off_but_sensible_once_switched_on() {
        let none = ImageSettings::read(|_| None);
        assert!(!none.optimize, "rewriting uploads is a choice somebody makes");
        assert_eq!(none.max_dimension, DEFAULT_MAX_DIMENSION);
        assert_eq!(none.jpeg_quality, DEFAULT_JPEG_QUALITY);
        assert!(none.strip_metadata);
    }

    #[test]
    fn out_of_range_values_are_clamped_not_refused() {
        let s = ImageSettings::read(|k| match k {
            SETTING_OPTIMIZE => Some("true".into()),
            SETTING_MAX_DIMENSION => Some("50".into()),
            SETTING_JPEG_QUALITY => Some("250".into()),
            SETTING_STRIP_METADATA => Some("false".into()),
            _ => None,
        });
        assert!(s.optimize && !s.strip_metadata);
        assert_eq!(s.max_dimension, MIN_MAX_DIMENSION);
        assert_eq!(s.jpeg_quality, MAX_JPEG_QUALITY);
        let garbage = ImageSettings::read(|k| (k == SETTING_JPEG_QUALITY).then(|| "high".into()));
        assert_eq!(garbage.jpeg_quality, DEFAULT_JPEG_QUALITY);
    }

    #[test]
    fn the_longest_side_is_held_to_the_limit() {
        assert_eq!(target_size(4000, 3000, 2000), Some((2000, 1500)));
        assert_eq!(target_size(3000, 4000, 2000), Some((1500, 2000)), "portrait too");
        assert_eq!(target_size(2000, 1000, 2000), None, "already fits");
        assert_eq!(target_size(800, 600, 2000), None, "never scales up");
        // An extreme panorama still keeps a pixel of height.
        assert_eq!(target_size(100_000, 10, 2000), Some((2000, 1)));
        assert_eq!(target_size(0, 100, 2000), None);
    }

    #[test]
    fn jpeg_metadata_is_found_by_walking_segments_not_by_searching() {
        // SOI, APP1 "Exif\0\0" (length 8), SOS, EOI.
        let with_exif = [
            0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x08, b'E', b'x', b'i', b'f', 0, 0, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9,
        ];
        assert!(jpeg_has_metadata(&with_exif));
        // APP0 JFIF only.
        let plain = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x07, b'J', b'F', b'I', b'F', 0, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9,
        ];
        assert!(!jpeg_has_metadata(&plain));
        // "Exif" appearing inside the scan data is not metadata.
        let mut in_scan = plain.to_vec();
        in_scan.splice(14..14, b"Exif\0".iter().copied());
        assert!(!jpeg_has_metadata(&in_scan));
        // Not a JPEG at all, or truncated.
        assert!(!jpeg_has_metadata(b"\x89PNG\r\n"));
        assert!(!jpeg_has_metadata(&with_exif[..5]));
    }

    #[test]
    fn a_rewrite_is_kept_only_when_it_earned_its_place() {
        assert!(worth_keeping(1000, 600, false, false), "smaller");
        assert!(!worth_keeping(1000, 1200, false, false), "a re-encode that grew it");
        assert!(worth_keeping(1000, 1200, true, false), "a limit is a limit");
        assert!(worth_keeping(1000, 1200, false, true), "privacy is not about bytes");
    }

    #[test]
    fn only_jpeg_and_png_are_rewritten() {
        for yes in ["jpg", "JPEG", "png"] {
            assert!(optimizable(yes), "{yes}");
        }
        for no in ["gif", "webp", "avif", "svg", "ico", ""] {
            assert!(!optimizable(no), "{no}");
        }
    }

    #[test]
    fn bytes_read_the_way_people_say_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(340 * 1024), "340 KB");
        assert_eq!(human_bytes(1_258_291), "1.2 MB");
    }
}
