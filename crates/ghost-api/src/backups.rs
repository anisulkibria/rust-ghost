//! Backups the site can be put back from.
//!
//! A backup is one zip under `content/backups/`:
//!
//! * `manifest.json` — what it is: when, what kind, which version, how much;
//! * `database.json` — every table, from [`ghost_db::snapshot::dump`];
//! * `content/…` — the files the database points at: images, media, files,
//!   themes and the public folder, plus `routes.yaml` and `redirects.yaml`.
//!
//! They are files, not rows, because restoring replaces the database: a
//! backup kept *in* the database would be overwritten by the older copy of
//! itself it was restoring.
//!
//! At most [`MAX_BACKUPS`] are kept. Taking one more deletes the oldest, of
//! whatever kind. Restoring always takes a backup of the site as it is first,
//! so a restore can itself be undone.
//!
//! One backup or restore runs at a time ([`LOCK`]); a scheduled backup that
//! comes due during a restore waits for it.

use crate::AppState;
use chrono::{DateTime, Datelike, Utc};
use ghost_core::{GhostError, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// How many backups are kept on disk.
pub const MAX_BACKUPS: usize = 10;

/// The content folders a backup carries, and the loose files.
const CONTENT_DIRS: &[&str] = &["images", "media", "files", "themes", "public"];
const CONTENT_FILES: &[&str] = &["routes.yaml", "redirects.yaml"];

/// Never restored over: whoever is restoring stays signed in.
const KEEP_TABLES: &[&str] = &["sessions"];

/// One backup or restore at a time.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Manual,
    Daily,
    Weekly,
    Monthly,
    PreRestore,
    Uploaded,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Manual => "manual",
            Kind::Daily => "daily",
            Kind::Weekly => "weekly",
            Kind::Monthly => "monthly",
            Kind::PreRestore => "pre-restore",
            Kind::Uploaded => "uploaded",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Manual => "Manual",
            Kind::Daily => "Daily",
            Kind::Weekly => "Weekly",
            Kind::Monthly => "Monthly",
            Kind::PreRestore => "Before a restore",
            Kind::Uploaded => "Uploaded",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Manual, Kind::Daily, Kind::Weekly, Kind::Monthly, Kind::PreRestore, Kind::Uploaded]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
    pub fn is_automatic(self) -> bool {
        matches!(self, Kind::Daily | Kind::Weekly | Kind::Monthly)
    }
}

/// The automatic schedule, from the `backup_schedule` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    Off,
    Daily,
    Weekly,
    Monthly,
}

impl Schedule {
    pub fn from_settings(state: &AppState) -> Schedule {
        Schedule::parse(&state.settings.get_str("backup_schedule").unwrap_or_default())
    }
    pub fn parse(s: &str) -> Schedule {
        match s {
            "daily" => Schedule::Daily,
            "weekly" => Schedule::Weekly,
            "monthly" => Schedule::Monthly,
            _ => Schedule::Off,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Schedule::Off => "off",
            Schedule::Daily => "daily",
            Schedule::Weekly => "weekly",
            Schedule::Monthly => "monthly",
        }
    }
    fn kind(self) -> Option<Kind> {
        match self {
            Schedule::Off => None,
            Schedule::Daily => Some(Kind::Daily),
            Schedule::Weekly => Some(Kind::Weekly),
            Schedule::Monthly => Some(Kind::Monthly),
        }
    }
    /// When the next automatic backup is due, given the last one.
    pub fn next_after(self, last: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Schedule::Off => None,
            Schedule::Daily => Some(last + chrono::Duration::days(1)),
            Schedule::Weekly => Some(last + chrono::Duration::days(7)),
            Schedule::Monthly => last.checked_add_months(chrono::Months::new(1)),
        }
    }
}

/// One backup on disk.
#[derive(Debug, Clone)]
pub struct Backup {
    /// File name, which is also its id: `backup-20261119-153000123-manual.zip`.
    pub name: String,
    pub kind: Kind,
    /// When it was written here (from the name — for an uploaded backup, the
    /// upload). Ordering and pruning go by this.
    pub stored_at: DateTime<Utc>,
    /// When the site in it was captured (from the manifest).
    pub taken_at: DateTime<Utc>,
    pub size: u64,
    pub version: String,
    pub site: String,
    pub tables: usize,
    pub rows: usize,
    pub files: usize,
    /// When the file was written, to order backups made in the same second.
    written: std::time::SystemTime,
}

pub fn dir(state: &AppState) -> PathBuf {
    state.config.content_path().join("backups")
}

/// Only names this module made, so a name from a URL cannot reach outside
/// the backups folder.
pub fn valid_name(name: &str) -> bool {
    name.starts_with("backup-")
        && name.ends_with(".zip")
        && name.len() < 80
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.contains("..")
}

pub fn path_of(state: &AppState, name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| dir(state).join(name)).filter(|p| p.is_file())
}

fn parse_name(name: &str) -> Option<(DateTime<Utc>, Kind)> {
    // backup-YYYYMMDD-HHMMSSmmm-<kind>[-n].zip — milliseconds, so a name is
    // never handed out twice: a deleted backup's download link must not
    // quietly start serving a different one.
    let stem = name.strip_prefix("backup-")?.strip_suffix(".zip")?;
    let (stamp, rest) = stem.split_at(18.min(stem.len()));
    let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S%3f").ok()?.and_utc();
    let rest = rest.strip_prefix('-')?;
    let kind = Kind::parse(rest).or_else(|| rest.rsplit_once('-').and_then(|(k, _)| Kind::parse(k)))?;
    Some((at, kind))
}

fn read_manifest(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name("manifest.json").ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

fn describe(path: &Path) -> Option<Backup> {
    let name = path.file_name()?.to_str()?.to_string();
    let (stored_at, kind) = parse_name(&name)?;
    let manifest = read_manifest(path)?;
    let n = |k: &str| manifest.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    Some(Backup {
        taken_at: manifest
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(stored_at),
        version: manifest.get("version").and_then(Value::as_str).unwrap_or("").to_string(),
        site: manifest.get("site").and_then(Value::as_str).unwrap_or("").to_string(),
        tables: n("tables"),
        rows: n("rows"),
        files: n("files"),
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        written: std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH),
        name,
        kind,
        stored_at,
    })
}

/// Every backup on disk, newest first.
pub async fn list(state: &AppState) -> Vec<Backup> {
    let dir = dir(state);
    tokio::task::spawn_blocking(move || {
        let mut out: Vec<Backup> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(valid_name))
            .filter_map(|p| describe(&p))
            .collect();
        // newest first: by the second in the name, then — for several in
        // one second — by when the file was written. Not by name alone:
        // `manual-2.zip` sorts before `manual.zip`.
        out.sort_by(|a, b| (b.stored_at, b.written).cmp(&(a.stored_at, a.written)));
        out
    })
    .await
    .unwrap_or_default()
}

fn unique_name(dir: &Path, at: DateTime<Utc>, kind: Kind) -> String {
    let base = format!("backup-{}-{}", at.format("%Y%m%d-%H%M%S%3f"), kind.as_str());
    let mut name = format!("{base}.zip");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{base}-{n}.zip");
        n += 1;
    }
    name
}

/// Delete the oldest backups beyond [`MAX_BACKUPS`]. `protect` is never
/// deleted — the backup a restore is reading from, while it reads.
async fn prune(state: &AppState, protect: Option<&str>) -> Vec<String> {
    let all = list(state).await;
    let mut removed = Vec::new();
    let mut excess = all.len().saturating_sub(MAX_BACKUPS);
    for b in all.iter().rev() {
        if excess == 0 {
            break;
        }
        if Some(b.name.as_str()) == protect {
            continue;
        }
        if tokio::fs::remove_file(dir(state).join(&b.name)).await.is_ok() {
            tracing::info!("backups: deleted {} (more than {MAX_BACKUPS} kept)", b.name);
            removed.push(b.name.clone());
            excess -= 1;
        }
    }
    removed
}

/// Take a backup now.
pub async fn create(state: &AppState, kind: Kind) -> Result<Backup> {
    let _guard = LOCK.lock().await;
    create_locked(state, kind, None).await
}

async fn create_locked(state: &AppState, kind: Kind, protect: Option<&str>) -> Result<Backup> {
    let (snapshot, counts) = ghost_db::snapshot::dump(&state.db).await?;
    let content = state.config.content_path();
    let dir = dir(state);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| GhostError::Internal(format!("Could not create the backups folder: {e}")))?;
    let now = Utc::now();
    let name = unique_name(&dir, now, kind);
    let manifest = json!({
        "format": 1,
        "kind": kind.as_str(),
        "created_at": now.to_rfc3339(),
        "version": ghost_core::GHOST_VERSION,
        "engine": snapshot.get("engine").cloned().unwrap_or(Value::Null),
        "site": state.settings.site_name(),
        "tables": counts.tables,
        "rows": counts.rows,
    });
    let final_path = dir.join(&name);
    let tmp_path = dir.join(format!(".tmp-{name}"));
    let tmp = tmp_path.clone();
    let written = tokio::task::spawn_blocking(move || write_archive(&tmp, &content, manifest, &snapshot))
        .await
        .map_err(|e| GhostError::Internal(e.to_string()))?;
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    tokio::fs::rename(&tmp_path, &final_path).await.map_err(|e| GhostError::Internal(e.to_string()))?;
    tracing::info!("backups: wrote {name}");
    prune(state, protect.or(Some(&name))).await;
    describe(&final_path).ok_or_else(|| GhostError::Internal("The backup was written but cannot be read back.".into()))
}

fn io(e: impl std::fmt::Display) -> GhostError {
    GhostError::Internal(format!("Writing the backup failed: {e}"))
}

fn write_archive(path: &Path, content: &Path, mut manifest: Value, snapshot: &Value) -> Result<()> {
    let file = std::fs::File::create(path).map_err(io)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let deflate = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);

    zip.start_file("database.json", deflate).map_err(io)?;
    serde_json::to_writer(&mut zip, snapshot).map_err(io)?;

    let mut files = 0usize;
    let mut add = |zip: &mut zip::ZipWriter<_>, abs: &Path, rel: &str| -> Result<()> {
        // Already-compressed formats are stored as they are; deflating a
        // JPEG again costs time and saves nothing.
        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let packed = ["jpg", "jpeg", "png", "gif", "webp", "avif", "mp4", "mov", "webm", "mp3", "m4a", "zip", "woff", "woff2", "pdf"];
        let opts = if packed.contains(&ext.as_str()) {
            deflate.compression_method(zip::CompressionMethod::Stored)
        } else {
            deflate
        };
        zip.start_file(format!("content/{rel}"), opts).map_err(io)?;
        let mut f = std::fs::File::open(abs).map_err(io)?;
        std::io::copy(&mut f, zip).map_err(io)?;
        files += 1;
        Ok(())
    };
    for d in CONTENT_DIRS {
        let root = content.join(d);
        if !root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(content) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            add(&mut zip, entry.path(), &rel)?;
        }
    }
    for f in CONTENT_FILES {
        let abs = content.join(f);
        if abs.is_file() {
            add(&mut zip, &abs, f)?;
        }
    }
    manifest["files"] = json!(files);
    zip.start_file("manifest.json", deflate).map_err(io)?;
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes()).map_err(io)?;
    zip.finish().map_err(io)?.flush().map_err(io)?;
    Ok(())
}

pub async fn delete(state: &AppState, name: &str) -> Result<()> {
    let _guard = LOCK.lock().await;
    let path = path_of(state, name).ok_or_else(|| GhostError::NotFoundError { message: "No such backup.".into(), context: None })?;
    tokio::fs::remove_file(path).await.map_err(|e| GhostError::Internal(e.to_string()))
}

/// Keep a backup file someone uploaded (taken on this site or another), once
/// it has been checked to be one.
pub async fn store_upload(state: &AppState, bytes: Vec<u8>) -> Result<Backup> {
    let _guard = LOCK.lock().await;
    let checked = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .map_err(|_| GhostError::BadRequest("That file is not a zip.".into()))?;
        let mut text = String::new();
        zip.by_name("manifest.json")
            .map_err(|_| GhostError::BadRequest("That zip is not a Vuuut backup — it has no manifest.json.".into()))?
            .read_to_string(&mut text)
            .map_err(|_| GhostError::BadRequest("The backup's manifest cannot be read.".into()))?;
        let manifest: Value = serde_json::from_str(&text).map_err(|_| GhostError::BadRequest("The backup's manifest cannot be read.".into()))?;
        if manifest.get("format").and_then(Value::as_u64) != Some(1) {
            return Err(GhostError::BadRequest("This backup was made by a version that cannot be read here.".into()));
        }
        if zip.by_name("database.json").is_err() {
            return Err(GhostError::BadRequest("That backup holds no database.".into()));
        }
        Ok(bytes)
    })
    .await
    .map_err(|e| GhostError::Internal(e.to_string()))??;
    let dir = dir(state);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| GhostError::Internal(e.to_string()))?;
    let name = unique_name(&dir, Utc::now(), Kind::Uploaded);
    let path = dir.join(&name);
    tokio::fs::write(&path, checked).await.map_err(|e| GhostError::Internal(e.to_string()))?;
    prune(state, Some(&name)).await;
    describe(&path).ok_or_else(|| GhostError::Internal("The upload was saved but cannot be read back.".into()))
}

/// What a restore did.
#[derive(Debug, Clone)]
pub struct Restored {
    pub safety: String,
    pub tables: usize,
    pub rows: usize,
    pub files: usize,
    pub warnings: Vec<String>,
}

/// Put the site back to a backup.
///
/// In order: read and check the backup; back up the site as it is now (so
/// this can be undone); replace the database; replace the content folders
/// the backup carries; then reload everything that was read from either at
/// startup — settings, routes, redirects, the theme, the firewall.
///
/// The automatic schedule is the one setting carried across: restoring a
/// backup from before backups were switched on should not switch them off.
pub async fn restore(state: &AppState, name: &str) -> Result<Restored> {
    let _guard = LOCK.lock().await;
    let path = path_of(state, name).ok_or_else(|| GhostError::NotFoundError { message: "No such backup.".into(), context: None })?;

    let read_path = path.clone();
    let snapshot = tokio::task::spawn_blocking(move || -> Result<Value> {
        let file = std::fs::File::open(&read_path).map_err(|e| GhostError::Internal(e.to_string()))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|_| GhostError::BadRequest("The backup file is damaged.".into()))?;
        let entry = zip.by_name("database.json").map_err(|_| GhostError::BadRequest("That backup holds no database.".into()))?;
        serde_json::from_reader(std::io::BufReader::new(entry)).map_err(|e| GhostError::BadRequest(format!("The backup's database cannot be read: {e}")))
    })
    .await
    .map_err(|e| GhostError::Internal(e.to_string()))??;

    let safety = create_locked(state, Kind::PreRestore, Some(name)).await?;
    let schedule = Schedule::from_settings(state);

    let counts = ghost_db::snapshot::restore(&state.db, &snapshot, KEEP_TABLES).await?;
    drop(snapshot);

    let content = state.config.content_path();
    let files = tokio::task::spawn_blocking(move || extract_content(&path, &content))
        .await
        .map_err(|e| GhostError::Internal(e.to_string()))??;

    let mut warnings = Vec::new();
    if let Err(e) = state.settings.reload(&state.db).await {
        warnings.push(format!("Settings could not be reloaded: {}", e.message()));
    }
    if schedule != Schedule::from_settings(state) {
        let _ = state.settings.set(&state.db, "backup_schedule", &json!(schedule.as_str())).await;
    }
    *state.frontend_key.write().await = None;
    state.firewall.reload(&state.db).await;

    let routes_path = state.config.content_path().join("routes.yaml");
    let routes_yaml = tokio::fs::read_to_string(&routes_path)
        .await
        .unwrap_or_else(|_| ghost_routing::DEFAULT_ROUTES_YAML.to_string());
    match ghost_routing::routes_yaml::parse_routes_yaml(&routes_yaml).and_then(|s| ghost_routing::RouterTree::build(&s)) {
        Ok(tree) => *state.router.write().await = std::sync::Arc::new(tree),
        Err(e) => warnings.push(format!("The backup's routes.yaml could not be used, so routing was left as it was: {}", e.message())),
    }
    match ghost_routing::redirects::load_from(&state.config.content_path().join("redirects.yaml")).await {
        Ok(r) => *state.redirects.write().await = std::sync::Arc::new(r),
        Err(e) => warnings.push(format!("The backup's redirects.yaml could not be used: {}", e.message())),
    }
    let theme = crate::themes::active_theme_name(state);
    if let Err(e) = crate::themes::activate(state, &theme).await {
        warnings.push(format!("The {theme} theme could not be activated: {}", e.message()));
    }
    tracing::info!("backups: restored {name} ({} rows, {files} files); safety copy {}", counts.rows, safety.name);
    Ok(Restored { safety: safety.name, tables: counts.tables, rows: counts.rows, files, warnings })
}

/// Lay the backup's `content/` over the site's. A content folder the backup
/// carries anything for is replaced whole, so files added since the backup
/// go as well — they were in the safety backup taken a moment ago. A folder
/// the backup has nothing for is left alone, so restoring a backup that held
/// no themes cannot leave the site without one. `redirects.yaml` is the one
/// file whose absence is meaningful: no file then means no redirects now.
fn extract_content(path: &Path, content: &Path) -> Result<usize> {
    let file = std::fs::File::open(path).map_err(|e| GhostError::Internal(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|_| GhostError::BadRequest("The backup file is damaged.".into()))?;
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    for d in CONTENT_DIRS {
        let prefix = format!("content/{d}/");
        if names.iter().any(|n| n.starts_with(&prefix)) {
            let target = content.join(d);
            if target.exists() {
                std::fs::remove_dir_all(&target).map_err(|e| GhostError::Internal(format!("Could not clear {d}: {e}")))?;
            }
        }
    }
    if !names.iter().any(|n| n == "content/redirects.yaml") {
        let _ = std::fs::remove_file(content.join("redirects.yaml"));
    }
    let mut files = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| GhostError::Internal(e.to_string()))?;
        // `enclosed_name` refuses absolute paths and `..`, so an entry can
        // only ever land inside the content folder.
        let Some(rel) = entry.enclosed_name() else { continue };
        let Ok(rel) = rel.strip_prefix("content") else { continue };
        if entry.is_dir() || rel.as_os_str().is_empty() {
            continue;
        }
        let top = rel.components().next().and_then(|c| c.as_os_str().to_str()).unwrap_or("");
        if !(CONTENT_DIRS.contains(&top) || CONTENT_FILES.contains(&top)) {
            continue;
        }
        let out = content.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| GhostError::Internal(e.to_string()))?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| GhostError::Internal(e.to_string()))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| GhostError::Internal(e.to_string()))?;
        files += 1;
    }
    Ok(files)
}

/// When the next automatic backup will run, if one is scheduled.
pub fn next_due(schedule: Schedule, backups: &[Backup]) -> Option<DateTime<Utc>> {
    let kind = schedule.kind()?;
    let last = backups.iter().filter(|b| b.kind == kind).map(|b| b.stored_at).max();
    match last {
        Some(at) => schedule.next_after(at),
        None => Some(Utc::now()),
    }
}

/// The scheduler. Checks every ten minutes whether the schedule's next
/// backup is due — so a schedule switched on runs its first backup within
/// minutes, and a server that was down when one was due catches up when it
/// comes back rather than waiting a whole period.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            let schedule = Schedule::from_settings(&state);
            if let Some(kind) = schedule.kind() {
                let due = next_due(schedule, &list(&state).await);
                if due.is_some_and(|d| d <= Utc::now()) {
                    if let Err(e) = create(&state, kind).await {
                        tracing::error!("backups: the {} backup failed: {}", kind.as_str(), e.message());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });
}

/// A time as the site's own clock reads it, e.g. `19 Nov 2026, 15:30`.
pub fn local_time(state: &AppState, at: DateTime<Utc>) -> String {
    let tz: chrono_tz::Tz = state.settings.get_str("timezone").and_then(|t| t.parse().ok()).unwrap_or(chrono_tz::UTC);
    let local = at.with_timezone(&tz);
    let today = Utc::now().with_timezone(&tz).date_naive();
    if local.date_naive() == today {
        format!("Today, {}", local.format("%H:%M"))
    } else if local.year() == today.year() {
        local.format("%-d %b, %H:%M").to_string()
    } else {
        local.format("%-d %b %Y, %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_refuse_paths() {
        let (at, kind) = parse_name("backup-20261119-153000123-pre-restore.zip").unwrap();
        assert_eq!(kind, Kind::PreRestore);
        assert_eq!(at.format("%Y-%m-%d %H:%M:%S%.3f").to_string(), "2026-11-19 15:30:00.123");
        assert_eq!(parse_name("backup-20261119-153000123-daily-3.zip").unwrap().1, Kind::Daily);
        let at = Utc::now();
        let dir = std::env::temp_dir();
        assert_eq!(parse_name(&unique_name(&dir, at, Kind::Manual)).unwrap().0.timestamp_millis(), at.timestamp_millis());
        assert!(valid_name("backup-20261119-153000123-manual.zip"));
        assert!(!valid_name("backup-../../etc.zip"));
        assert!(!valid_name("backup-x/y.zip"));
    }

    #[test]
    fn a_month_is_a_calendar_month() {
        let jan31 = DateTime::parse_from_rfc3339("2026-01-31T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(Schedule::Monthly.next_after(jan31).unwrap().format("%m-%d").to_string(), "02-28");
        assert!(Schedule::Off.next_after(jan31).is_none());
    }
}
