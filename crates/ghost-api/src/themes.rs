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
pub fn url_resolver(router: Arc<RouterTree>) -> ghost_render::helpers::UrlResolver {
    use ghost_routing::tree::TaxonomyKind;

    Arc::new(move |resource: &serde_json::Value| {
        let taxonomy = match resource_kind(resource) {
            Some(kind) => router.taxonomy_permalink(kind),
            None => None,
        };
        if let Some(permalink) = taxonomy {
            return ghost_routing::tree::build_permalink(permalink, resource);
        }
        // a tag or author with no taxonomy route configured has no page
        if resource_kind(resource).is_some() {
            return "/404/".to_string();
        }
        let _ = TaxonomyKind::Tag;
        match router.permalink_for(resource) {
            Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, resource),
            None => "/404/".to_string(),
        }
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
    use super::safe_entry_path;
    use std::path::PathBuf;

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
}
