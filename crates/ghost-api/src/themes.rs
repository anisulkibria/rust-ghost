//! Theme installation, listing, activation and deletion.
//!
//! Themes live as directories under `content/themes/`. The active one is
//! recorded in the `active_theme` setting and held compiled in
//! `AppState::theme`; activating swaps that handle so the next request
//! renders with the new theme without a restart.
use ghost_core::{GhostError, Result};
use ghost_render::ThemeState;
use ghost_routing::RouterTree;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::state::AppState;

/// Uncompressed bytes we are willing to write out for one theme zip.
const MAX_UNPACKED_BYTES: u64 = 200 * 1024 * 1024;
/// Entries we are willing to walk in one theme zip.
const MAX_ENTRIES: usize = 10_000;

/// One directory under `content/themes/`.
pub struct InstalledTheme {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub active: bool,
    /// `Some` when the theme fails validation and so cannot be activated.
    pub error: Option<String>,
}

/// Wrap a router tree snapshot as a URL-resolving closure (used by
/// serializers and `{{#get}}`).
///
/// Tags and authors resolve to their taxonomy archive; everything else is
/// treated as a post or page and goes through the collection permalinks.
///
/// The returned paths carry the install's base path, because this is what
/// fills the `url` field that `{{url}}`, the Content API, the sitemap and the
/// feeds all read — they are browser-ready. The router tree itself works in
/// unprefixed paths (requests arrive with the base path already stripped), so
/// the prefix is added here, once, on the way out. Code turning one of these
/// into an absolute URL must use [`ghost_core::url::absolute`] rather than
/// concatenating onto a site url that carries the prefix too.
pub fn url_resolver(router: Arc<RouterTree>) -> ghost_render::helpers::UrlResolver {
    use ghost_routing::tree::TaxonomyKind;

    Arc::new(move |resource: &serde_json::Value| {
        let taxonomy = match resource_kind(resource) {
            Some(kind) => router.taxonomy_permalink(kind),
            None => None,
        };
        if let Some(permalink) = taxonomy {
            return ghost_core::url::base(&ghost_routing::tree::build_permalink(permalink, resource));
        }
        // a tag or author with no taxonomy route configured has no page
        if resource_kind(resource).is_some() {
            return ghost_core::url::base("/404/");
        }
        let _ = TaxonomyKind::Tag;
        let path = match router.permalink_for(resource) {
            Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, resource),
            None => "/404/".to_string(),
        };
        ghost_core::url::base(&path)
    })
}

/// Identify tags and authors structurally, the way Ghost's own checks do —
/// the serialized shapes carry no explicit type field.
fn resource_kind(resource: &serde_json::Value) -> Option<ghost_routing::tree::TaxonomyKind> {
    use ghost_routing::tree::TaxonomyKind;
    let obj = resource.as_object()?;
    if !obj.contains_key("slug") {
        return None;
    }
    // a post or page always carries html; a user carries bio/profile_image
    if obj.contains_key("html") || obj.contains_key("title") {
        return None;
    }
    if obj.contains_key("bio") || obj.contains_key("profile_image") {
        return Some(TaxonomyKind::Author);
    }
    if obj.contains_key("feature_image") && obj.contains_key("description") {
        return Some(TaxonomyKind::Tag);
    }
    None
}

/// Compile a theme directory into an activation state with all helper deps wired.
pub async fn build_theme_state(
    db: SqlitePool,
    router: Arc<RouterTree>,
    site_origin: String,
    theme_dir: &Path,
    locale: &str,
) -> Result<ThemeState> {
    // `{{content_api_key}}` hands themes the public frontend key
    let content_api_key = ghost_db::repos::integrations::internal_content_key(&db)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let deps = ghost_render::HelperDeps {
        pool: Arc::new(db),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: url_resolver(router),
        // load_and_activate hashes the theme's assets itself and overwrites these
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: String::new(),
        theme_dir: theme_dir.to_path_buf(),
        site_origin,
        content_api_key,
    };
    let translations = load_translations(theme_dir, locale);

    // Themes are user-supplied, so a malformed one must not be able to take
    // the process down — turn a panic in the compiler into a theme error.
    // Nothing is published on this path, so unwinding out of it is safe.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ghost_render::load_and_activate(theme_dir, &deps, translations)
    }))
    .unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(GhostError::ThemeError(format!("theme failed to compile: {detail}")))
    })
}

/// `<theme>/locales/<locale>.json` — flat key → string map, falling back to
/// the base language (`pt-BR` → `pt`) and then to English.
///
/// Ghost's locale files list every translatable string up front with an
/// empty value, so empty entries are dropped: the `{{t}}` helper is meant to
/// fall back to the key itself for those.
pub fn load_translations(theme_dir: &Path, locale: &str) -> HashMap<String, String> {
    let locales = theme_dir.join("locales");
    let base = locale.split(['-', '_']).next().unwrap_or("en");
    let mut candidates = vec![locale.to_string()];
    if base != locale {
        candidates.push(base.to_string());
    }
    candidates.push("en".to_string());

    for name in candidates {
        let Ok(raw) = std::fs::read_to_string(locales.join(format!("{name}.json"))) else {
            continue;
        };
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&raw)
        else {
            continue;
        };
        return map
            .into_iter()
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) if !s.is_empty() => Some((k, s)),
                _ => None,
            })
            .collect();
    }
    HashMap::new()
}

/// Reject anything that is not a single plain path segment, so a theme name
/// coming off a form or a zip can never escape `content/themes/`.
fn theme_dir_for(state: &AppState, name: &str) -> Result<PathBuf> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !valid {
        return Err(GhostError::ValidationError {
            message: format!("invalid theme name '{name}'"),
            context: None,
        });
    }
    Ok(state.config.themes_dir().join(name))
}

/// Files the in-browser editor will open. Templates, styles and scripts —
/// the text a theme is actually made of.
///
/// Anything else (images, fonts, zips) is listed but not editable: rendering
/// a binary into a textarea would corrupt it on save.
pub const EDITABLE_EXTENSIONS: &[&str] =
    &["hbs", "css", "js", "json", "md", "txt", "yaml", "yml", "svg"];

pub fn is_editable(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|e| EDITABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Which highlighter the code editor should paint a file with.
///
/// Names are the editor's own, not the extension: `.hbs`, `.html` and `.svg`
/// are all markup with mustaches allowed, and `.yaml` has no highlighter of
/// its own yet, so it falls back to plain text rather than being coloured by
/// a grammar that is not its own.
pub fn editor_language(path: &str) -> &'static str {
    match path.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("hbs") | Some("html") | Some("htm") | Some("svg") | Some("xml") => "hbs",
        Some("css") => "css",
        Some("js") | Some("mjs") | Some("cjs") => "js",
        Some("json") => "json",
        Some("md") | Some("markdown") => "md",
        _ => "txt",
    }
}

/// A relative path inside a theme, rejected unless every segment is a plain
/// name.
///
/// The whole point: `..`, absolute paths, and anything with a separator the
/// host treats specially must not reach the filesystem, or the editor becomes
/// a way to read and overwrite any file the server can.
pub fn theme_file_path(state: &AppState, theme: &str, rel: &str) -> Result<PathBuf> {
    let dir = theme_dir_for(state, theme)?;
    let bad = rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.split('/').any(|seg| {
            seg.is_empty()
                || seg == "."
                || seg == ".."
                || seg.starts_with('.')
                || !seg.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        });
    if bad {
        return Err(GhostError::ValidationError {
            message: format!("invalid file path '{rel}'"),
            context: None,
        });
    }
    let full = dir.join(rel);
    // Belt and braces: even with the segment rules above, the resolved path
    // has to still be inside the theme.
    let inside = match (full.canonicalize(), dir.canonicalize()) {
        (Ok(f), Ok(d)) => f.starts_with(&d),
        // A file being created does not exist yet, so only its parent can be
        // checked.
        (Err(_), Ok(d)) => full
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.starts_with(&d))
            .unwrap_or(false),
        _ => false,
    };
    if !inside {
        return Err(GhostError::ValidationError {
            message: "that path is outside the theme".into(),
            context: None,
        });
    }
    Ok(full)
}

/// Every file in a theme, relative to its root, depth-first and sorted.
pub fn theme_files(state: &AppState, theme: &str) -> Vec<String> {
    let Ok(dir) = theme_dir_for(state, theme) else { return Vec::new() };
    let mut out = Vec::new();
    fn walk(base: &std::path::Path, cur: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(cur) else { return };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            // Never list dotfiles or dependency trees: they are not part of
            // the theme a publisher edits.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                walk(base, &path, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk(&dir, &dir, &mut out);
    out.sort();
    out
}

pub fn active_theme_name(state: &AppState) -> String {
    state.settings.get_str("active_theme").unwrap_or_else(|| "source".into())
}

/// Every installed theme, with its package metadata and validation status.
pub fn list(state: &AppState) -> Vec<InstalledTheme> {
    let active = active_theme_name(state);
    ghost_render::list_themes(&state.config.themes_dir())
        .into_iter()
        .map(|(name, path)| {
            let pkg = ghost_render::read_pkg(&path);
            InstalledTheme {
                active: name == active,
                version: pkg.as_ref().and_then(|p| p.version.clone()),
                description: pkg.as_ref().and_then(|p| p.description.clone()),
                error: ghost_render::validate(&path).err().map(|e| e.message()),
                name,
            }
        })
        .collect()
}

/// Compile `name` and swap it in as the live theme, persisting the choice.
pub async fn activate(state: &AppState, name: &str) -> Result<()> {
    let dir = theme_dir_for(state, name)?;
    if !dir.is_dir() {
        return Err(GhostError::NotFoundError {
            message: format!("theme '{name}' is not installed"),
            context: None,
        });
    }

    let router = state.router.read().await.clone();
    let locale = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
    let theme = build_theme_state(state.db.clone(), router, state.site_url(), &dir, &locale).await?;

    *state.theme.write().await = Arc::new(theme);
    state
        .settings
        .set(&state.db, "active_theme", &serde_json::json!(name))
        .await?;
    tracing::info!("theme '{name}' activated");
    Ok(())
}

/// Remove an installed theme. The active theme cannot be deleted.
pub async fn delete(state: &AppState, name: &str) -> Result<()> {
    if name == active_theme_name(state) {
        return Err(GhostError::ValidationError {
            message: "cannot delete the active theme".into(),
            context: None,
        });
    }
    let dir = theme_dir_for(state, name)?;
    if !dir.is_dir() {
        return Err(GhostError::NotFoundError {
            message: format!("theme '{name}' is not installed"),
            context: None,
        });
    }
    tokio::fs::remove_dir_all(&dir).await?;
    Ok(())
}

/// Unpack a theme zip into `content/themes/`, replacing any theme of the same
/// name. Returns the installed theme's name.
///
/// `filename` is the uploaded file's name; it names the theme when the archive
/// carries no usable `package.json` name.
pub async fn install_zip(state: &AppState, filename: &str, data: Vec<u8>) -> Result<String> {
    let themes_dir = state.config.themes_dir();
    tokio::fs::create_dir_all(&themes_dir).await?;

    // stage in a sibling directory so a bad archive never half-overwrites a
    // theme that is currently serving requests
    let staging = themes_dir.join(format!(".upload-{}", ghost_core::ids::random_hex(8)));
    let fallback = ghost_core::slug::safe_string(filename.rsplit_once('.').map_or(filename, |(s, _)| s));

    let result = {
        let staging = staging.clone();
        tokio::task::spawn_blocking(move || unpack(&data, &staging))
            .await
            .map_err(|e| GhostError::Internal(e.to_string()))?
    };

    let installed = match result.and_then(|root| finish_install(&root, &themes_dir, &fallback)) {
        Ok(name) => name,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(e);
        }
    };
    let _ = tokio::fs::remove_dir_all(&staging).await;
    Ok(installed)
}

/// Validate the staged tree and move it to its final name.
fn finish_install(root: &Path, themes_dir: &Path, fallback: &str) -> Result<String> {
    ghost_render::validate(root)?;

    let name = ghost_render::read_pkg(root)
        .map(|p| ghost_core::slug::safe_string(&p.name))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| fallback.to_string());
    let name = if name.is_empty() { "theme".to_string() } else { name };

    let dest = themes_dir.join(&name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(root, &dest)?;
    Ok(name)
}

/// Extract `data` into `staging` and return the directory holding the theme
/// files — the staging dir itself, or its single top-level directory when the
/// archive is wrapped (the usual `my-theme.zip` → `my-theme/…` shape).
fn unpack(data: &[u8], staging: &Path) -> Result<PathBuf> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|e| GhostError::ValidationError { message: format!("invalid zip: {e}"), context: None })?;
    if zip.len() > MAX_ENTRIES {
        return Err(GhostError::ValidationError {
            message: format!("zip has too many entries (max {MAX_ENTRIES})"),
            context: None,
        });
    }

    std::fs::create_dir_all(staging)?;
    let mut written: u64 = 0;
    // `None` = not established yet, `Some(None)` = entries at several roots
    let mut single_root: Option<Option<String>> = None;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| GhostError::ValidationError { message: format!("invalid zip: {e}"), context: None })?;
        let is_dir = entry.is_dir();
        let Some(rel) = safe_entry_path(entry.name()) else {
            continue;
        };

        // a file sitting at the archive root means it is not wrapped
        let top = if is_dir || rel.components().count() > 1 {
            rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string())
        } else {
            None
        };
        single_root = Some(match (single_root.take(), top) {
            (None, t) => t,
            (Some(prev), t) if prev == t => prev,
            _ => None,
        });

        let out = staging.join(&rel);
        if is_dir {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        written += entry.size();
        if written > MAX_UNPACKED_BYTES {
            return Err(GhostError::ValidationError {
                message: "theme is too large (max 200MB unpacked)".into(),
                context: None,
            });
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        std::fs::write(&out, &buf)?;
    }

    Ok(match single_root.flatten() {
        Some(root) => staging.join(root),
        None => staging.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// official themes
// ---------------------------------------------------------------------------

/// One theme from Ghost's "Official themes" gallery — the set the hosted admin
/// offers under `/ghost/#/settings/design/change-theme`.
///
/// They are all free, MIT-licensed and published by Ghost itself from a known
/// GitHub organisation, which is what makes a one-click install reasonable:
/// the download target is fixed at compile time rather than coming off the
/// request.
pub struct OfficialTheme {
    /// Display name, as Ghost lists it.
    pub name: &'static str,
    /// Directory it installs as — the `name` in the theme's `package.json`,
    /// and so also the key the install/activate routes take.
    pub slug: &'static str,
    /// Ghost's own grouping: Blog, Newsletter, Magazine, …
    pub category: &'static str,
    /// Live demo site. Doubles as the gallery thumbnail.
    pub preview_url: &'static str,
    /// `Org/Repo` on GitHub. The zipball of its default branch is what gets
    /// installed, which is how Ghost installs these too.
    pub repo: &'static str,
}

/// Ghost's official themes, in the order the hosted admin lists them.
pub const OFFICIAL_THEMES: &[OfficialTheme] = &[
    OfficialTheme { name: "Source",   slug: "source",   category: "News",          preview_url: "https://source.ghost.io/",   repo: "TryGhost/Source" },
    OfficialTheme { name: "Casper",   slug: "casper",   category: "Blog",          preview_url: "https://demo.ghost.io/",     repo: "TryGhost/Casper" },
    OfficialTheme { name: "Edition",  slug: "edition",  category: "Newsletter",    preview_url: "https://edition.ghost.io/",  repo: "TryGhost/Edition" },
    OfficialTheme { name: "Solo",     slug: "solo",     category: "Blog",          preview_url: "https://solo.ghost.io/",     repo: "TryGhost/Solo" },
    OfficialTheme { name: "Taste",    slug: "taste",    category: "Blog",          preview_url: "https://taste.ghost.io/",    repo: "TryGhost/Taste" },
    OfficialTheme { name: "Episode",  slug: "episode",  category: "Podcast",       preview_url: "https://episode.ghost.io/",  repo: "TryGhost/Episode" },
    OfficialTheme { name: "Digest",   slug: "digest",   category: "Newsletter",    preview_url: "https://digest.ghost.io/",   repo: "TryGhost/Digest" },
    OfficialTheme { name: "Bulletin", slug: "bulletin", category: "Newsletter",    preview_url: "https://bulletin.ghost.io/", repo: "TryGhost/Bulletin" },
    OfficialTheme { name: "Alto",     slug: "alto",     category: "Blog",          preview_url: "https://alto.ghost.io/",     repo: "TryGhost/Alto" },
    OfficialTheme { name: "Dope",     slug: "dope",     category: "Magazine",      preview_url: "https://dope.ghost.io/",     repo: "TryGhost/Dope" },
    OfficialTheme { name: "Wave",     slug: "wave",     category: "Podcast",       preview_url: "https://wave.ghost.io/",     repo: "TryGhost/Wave" },
    OfficialTheme { name: "Edge",     slug: "edge",     category: "Photography",   preview_url: "https://edge.ghost.io/",     repo: "TryGhost/Edge" },
    OfficialTheme { name: "Dawn",     slug: "dawn",     category: "Newsletter",    preview_url: "https://dawn.ghost.io/",     repo: "TryGhost/Dawn" },
    OfficialTheme { name: "Ease",     slug: "ease",     category: "Documentation", preview_url: "https://ease.ghost.io/",     repo: "TryGhost/Ease" },
    OfficialTheme { name: "Headline", slug: "headline", category: "News",          preview_url: "https://headline.ghost.io/", repo: "TryGhost/Headline" },
    OfficialTheme { name: "Ruby",     slug: "ruby",     category: "Magazine",      preview_url: "https://ruby.ghost.io/",     repo: "TryGhost/Ruby" },
    OfficialTheme { name: "London",   slug: "london",   category: "Photography",   preview_url: "https://london.ghost.io/",   repo: "TryGhost/London" },
    OfficialTheme { name: "Journal",  slug: "journal",  category: "Newsletter",    preview_url: "https://journal.ghost.io/",  repo: "TryGhost/Journal" },
];

/// Look up a gallery entry by the slug the install route carries.
pub fn official(slug: &str) -> Option<&'static OfficialTheme> {
    OFFICIAL_THEMES.iter().find(|t| t.slug == slug)
}

/// How long we give GitHub to hand over a theme archive. Generous, because
/// this runs on an operator's click and the archives carry built assets.
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Ceiling on a downloaded archive, well above the largest official theme.
const MAX_DOWNLOAD_BYTES: usize = 50 * 1024 * 1024;

/// Download an official theme from GitHub and install it, returning the name
/// it installed as.
///
/// Only the slugs in [`OFFICIAL_THEMES`] resolve, so no caller — and no
/// request — can point this at an arbitrary host.
pub async fn install_official(state: &AppState, slug: &str) -> Result<String> {
    let theme = official(slug).ok_or_else(|| GhostError::NotFoundError {
        message: format!("'{slug}' is not an official Ghost theme"),
        context: None,
    })?;
    let data = download_zipball(theme.repo).await?;
    let installed = install_zip(state, &format!("{}.zip", theme.slug), data).await?;
    tracing::info!("official theme '{}' installed from {}", installed, theme.repo);
    Ok(installed)
}

/// Ghost's marketplace screenshots, which is where the gallery thumbnails
/// come from. 600x750 PNGs, one per theme slug.
const THUMBNAIL_BASE: &str = "https://ghost.org/images/marketplace/themes";
/// Ceiling on a thumbnail; the largest of Ghost's own is around 160KB.
const MAX_THUMBNAIL_BYTES: usize = 4 * 1024 * 1024;
/// How long the admin tells the browser to keep a gallery thumbnail.
///
/// The upstream images are served with `max-age=0, must-revalidate`, so
/// pointing the gallery straight at ghost.org would mean 18 conditional
/// requests on every visit to the themes page. Copying them in and serving
/// them from here is what makes a real cache window possible — the art for a
/// published theme does not change, so a week is conservative.
pub const THUMBNAIL_MAX_AGE_SECONDS: u32 = 7 * 24 * 60 * 60;

/// Where copied thumbnails live between restarts.
fn thumbnail_cache_dir(state: &AppState) -> PathBuf {
    state.config.content_path().join("cache").join("theme-thumbnails")
}

/// PNG bytes for an official theme's gallery thumbnail, fetched from Ghost's
/// marketplace the first time and read off disk after that.
pub async fn official_thumbnail(state: &AppState, slug: &str) -> Result<Vec<u8>> {
    let theme = official(slug).ok_or_else(|| GhostError::NotFoundError {
        message: format!("'{slug}' is not an official Ghost theme"),
        context: None,
    })?;

    let dir = thumbnail_cache_dir(state);
    let path = dir.join(format!("{}.png", theme.slug));
    match tokio::fs::read(&path).await {
        Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
        _ => {}
    }

    let bytes = download_thumbnail(theme.slug).await?;

    // Written through a temporary name so a second request arriving mid-write
    // reads either the previous file or the complete new one, never a
    // half-written PNG. A failure to cache is not a failure to serve.
    tokio::fs::create_dir_all(&dir).await?;
    let tmp = dir.join(format!(".{}-{}.png", theme.slug, ghost_core::ids::random_hex(6)));
    if tokio::fs::write(&tmp, &bytes).await.is_ok() && tokio::fs::rename(&tmp, &path).await.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    Ok(bytes)
}

async fn download_thumbnail(slug: &str) -> Result<Vec<u8>> {
    let url = format!("{THUMBNAIL_BASE}/{slug}.png");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("Ghost-rust theme installer")
        .build()
        .map_err(|e| GhostError::Internal(e.to_string()))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| GhostError::Internal(format!("could not reach ghost.org: {e}")))?;
    if !response.status().is_success() {
        return Err(GhostError::NotFoundError {
            message: format!("no gallery image for '{slug}'"),
            context: Some(url),
        });
    }
    if response.content_length().is_some_and(|n| n as usize > MAX_THUMBNAIL_BYTES) {
        return Err(GhostError::ValidationError {
            message: "that gallery image is too large".into(),
            context: None,
        });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| GhostError::Internal(format!("download from ghost.org failed: {e}")))?;
    // Only ever written out and served back as image/png, so check the magic
    // rather than trusting the content type.
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() > MAX_THUMBNAIL_BYTES {
        return Err(GhostError::ValidationError {
            message: format!("the gallery image for '{slug}' is not a usable PNG"),
            context: None,
        });
    }
    Ok(bytes.to_vec())
}

/// Fetch the default branch of `Org/Repo` as a zip, the way Ghost's own
/// installer does (`/zipball` with no ref).
async fn download_zipball(repo: &str) -> Result<Vec<u8>> {
    let url = format!("https://api.github.com/repos/{}/zipball", repo.to_lowercase());
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        // GitHub rejects requests with no user agent outright
        .user_agent("Ghost-rust theme installer")
        .build()
        .map_err(|e| GhostError::Internal(e.to_string()))?;

    let mut response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| GhostError::Internal(format!("could not reach GitHub: {e}")))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(GhostError::NotFoundError {
            message: format!("the {repo} repository is not available on GitHub"),
            context: Some(url),
        });
    }
    if !response.status().is_success() {
        return Err(GhostError::Internal(format!(
            "GitHub returned {} for {repo}",
            response.status()
        )));
    }
    if response.content_length().is_some_and(|n| n as usize > MAX_DOWNLOAD_BYTES) {
        return Err(GhostError::ValidationError {
            message: "that theme archive is too large to install".into(),
            context: None,
        });
    }

    // Streamed rather than `bytes()` so a mis-declared (or absent)
    // content-length cannot make us buffer the whole response anyway.
    let mut data: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| GhostError::Internal(format!("download from GitHub failed: {e}")))?
    {
        if data.len() + chunk.len() > MAX_DOWNLOAD_BYTES {
            return Err(GhostError::ValidationError {
                message: "that theme archive is too large to install".into(),
                context: None,
            });
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// Map a zip entry name to a relative path under the staging dir, or `None`
/// when it must be skipped: absolute paths, `..` traversal (zip-slip),
/// archiver cruft and dotfiles.
fn safe_entry_path(name: &str) -> Option<PathBuf> {
    use std::path::Component;

    let name = name.replace('\\', "/");
    if name.starts_with("__MACOSX/") {
        return None;
    }
    let mut rel = PathBuf::new();
    for component in Path::new(&name).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str()?;
                if part.starts_with('.') {
                    return None;
                }
                rel.push(part);
            }
            // RootDir / Prefix / ParentDir / CurDir are all refusals: a theme
            // zip has no business writing outside its own directory
            Component::CurDir => continue,
            _ => return None,
        }
    }
    if rel.as_os_str().is_empty() {
        None
    } else {
        Some(rel)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_editable, official, safe_entry_path, OFFICIAL_THEMES};
    use std::path::PathBuf;

    #[test]
    fn official_themes_are_addressable() {
        for theme in OFFICIAL_THEMES {
            // the slug names a directory under content/themes/ and is what the
            // install and activate routes carry, so it has to survive the same
            // single-plain-segment check theme_dir_for applies
            assert!(
                theme.slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} has a slug that cannot name a theme directory",
                theme.name
            );
            assert_eq!(official(theme.slug).map(|t| t.name), Some(theme.name));
            assert!(theme.repo.starts_with("TryGhost/"), "{} is not a Ghost repo", theme.name);
        }
        assert!(official("not-a-theme").is_none());
    }

    #[test]
    fn rejects_traversal_and_cruft() {
        assert_eq!(safe_entry_path("../../etc/passwd"), None);
        assert_eq!(safe_entry_path("/etc/passwd"), None);
        assert_eq!(safe_entry_path("theme/../../x"), None);
        assert_eq!(safe_entry_path("__MACOSX/theme/._index.hbs"), None);
        assert_eq!(safe_entry_path(".git/config"), None);
        assert_eq!(safe_entry_path("theme/.DS_Store"), None);
    }

    #[test]
    fn keeps_ordinary_entries() {
        assert_eq!(safe_entry_path("flux/index.hbs"), Some(PathBuf::from("flux/index.hbs")));
        assert_eq!(safe_entry_path("./flux/assets/a.css"), Some(PathBuf::from("flux/assets/a.css")));
        assert_eq!(safe_entry_path("flux\\partials\\x.hbs"), Some(PathBuf::from("flux/partials/x.hbs")));
    }

    /// The editor writes whatever path it is given, so a traversal here would
    /// let anyone with admin access read and overwrite arbitrary files.
    #[test]
    fn a_theme_file_path_cannot_escape_its_theme() {
        for bad in [
            "../../../etc/passwd",
            "..",
            "a/../../b.hbs",
            "/etc/passwd",
            "partials\\..\\..\\x",
            ".env",
            "sub/.hidden",
            "",
        ] {
            assert!(
                is_rejected_relative(bad),
                "expected {bad:?} to be rejected"
            );
        }
        for ok in ["index.hbs", "partials/post-card.hbs", "assets/built/screen.css"] {
            assert!(!is_rejected_relative(ok), "expected {ok:?} to be allowed");
        }
    }

    /// The segment rules on their own, without needing an AppState or a real
    /// theme on disk.
    fn is_rejected_relative(rel: &str) -> bool {
        rel.is_empty()
            || rel.starts_with('/')
            || rel.contains('\\')
            || rel.split('/').any(|seg| {
                seg.is_empty()
                    || seg == "."
                    || seg == ".."
                    || seg.starts_with('.')
                    || !seg.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            })
    }

    #[test]
    fn only_text_files_are_editable() {
        assert!(is_editable("index.hbs"));
        assert!(is_editable("assets/screen.CSS"));
        assert!(!is_editable("assets/logo.png"));
        assert!(!is_editable("noextension"));
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    #[test]
    fn every_editable_extension_resolves_a_highlighter() {
        // A file the editor will open must get a language, or it opens
        // uncoloured with no indication why.
        for ext in EDITABLE_EXTENSIONS {
            let lang = editor_language(&format!("thing.{ext}"));
            assert!(
                ["hbs", "css", "js", "json", "md", "txt"].contains(&lang),
                "{ext} resolved to the unknown language {lang}"
            );
        }
    }

    #[test]
    fn markup_shares_one_grammar_and_the_rest_fall_back() {
        assert_eq!(editor_language("default.hbs"), "hbs");
        assert_eq!(editor_language("partials/icons/arrow.svg"), "hbs");
        assert_eq!(editor_language("assets/css/screen.CSS"), "css", "extensions are case-insensitive");
        assert_eq!(editor_language("gulpfile.js"), "js");
        assert_eq!(editor_language("package.json"), "json");
        assert_eq!(editor_language("README.md"), "md");
        // No YAML grammar yet, so routes.yaml is plain text rather than
        // coloured by a grammar that is not its own.
        assert_eq!(editor_language("routes.yaml"), "txt");
        assert_eq!(editor_language("LICENSE"), "txt", "a file with no extension at all");
    }
}
