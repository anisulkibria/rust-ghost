//! Settings → Images, and optimising the library that is already there.
//!
//! Part of the Media module rather than a module of its own: what it changes
//! is the media library, and the settings page is reached from the library's
//! header. The rules are [`ghost_core::images`]; the decoding is
//! `ghost_api::image_optimize`, which every upload already passes through.
//!
//! ## The bulk run, and why it keeps backups
//!
//! Optimising files that are already published replaces them in place —
//! every post that points at `/content/images/2026/11/cover.jpg` keeps working
//! and simply gets the smaller file. Each original is first copied to
//! `content/data/image-originals/<same path>`:
//!
//! - **`content/data`, not `content/images`.** Ghost keeps `_o` originals
//!   beside the images, which publishes them — including the GPS position
//!   stripping was meant to remove. `content/data` holds the database and is
//!   never served.
//! - **The backup is the "already done" marker.** A run skips any file that
//!   has one, so it is safe to stop, resume, or press the button twice. It is
//!   also what makes a run reversible: copy a file back and it is as it was.
//! - **A batch at a time.** A library of ten thousand photos is not decoded in
//!   one request. Each press handles [`BATCH`] and says how many are left.
//!
//! Object storage (S3 and friends) is not rewritten. A site that publishes
//! uploads to a bucket serves the bucket's copy, and this says so on screen
//! rather than claiming savings the reader will never see.

use super::*;
use ghost_core::images::{self as rules, ImageSettings};

const SETTINGS_PATH: &str = "/ghost/settings/images/";
const OPTIMIZE_PATH: &str = "/ghost/media/optimize/";

/// Files decoded per press. A 24-megapixel JPEG takes a few hundred
/// milliseconds; this keeps one press well inside any proxy's timeout.
const BATCH: usize = 40;

fn image_settings(state: &AppState) -> ImageSettings {
    ImageSettings::read(|key| state.settings.get_str(key))
}

fn backups_root(state: &AppState) -> std::path::PathBuf {
    state.config.data_dir().join("image-originals")
}

// ---------------------------------------------------------------------------
// Settings → Images
// ---------------------------------------------------------------------------

pub(crate) async fn images_settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let group = crate::SETTINGS_GROUPS
        .iter()
        .find(|g| g.slug == "images")
        .expect("images settings group");
    let s = image_settings(&state);
    let switch = |name: &str, on: bool| {
        format!(
            r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{ck}><span class="track"><span class="thumb"></span></span></label>"#,
            name = esc(name),
            ck = if on { " checked" } else { "" },
        )
    };
    let row = |icon: &str, tint: &str, title: &str, detail: &str, control: &str| {
        format!(
            r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing im-control">{control}</div></div>"#,
            ic = svg(icon),
            title = esc(title),
            detail = esc(detail),
        )
    };

    page_for_role(
        actor.role,
        "Images",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<form method="post" action="{SETTINGS_PATH}" id="images-settings">
<div class="section">Optimisation</div>
<div class="card rows rise" style="--i:1">
{optimize}
</div>
<div class="section">Size</div>
<div class="card rows rise" style="--i:2">
{max}
</div>
<div class="section">Quality</div>
<div class="card rows rise" style="--i:3">
{quality}
</div>
<div class="section">Privacy</div>
<div class="card rows rise" style="--i:4">
{strip}
</div>
</form>
<p class="muted rise" style="--i:5;margin:14px 4px 0">These apply to new uploads. Images already in the library are
optimised from <a href="/ghost/media/">Tools → Media</a>, which keeps a private copy of every original it replaces.
GIF, WebP, AVIF and SVG files are always stored as they were uploaded.</p>"#,
            head = crate::group_head(group, Some("images-settings")),
            banner = flash_banner(&q),
            optimize = row(
                "image",
                if s.optimize { "green" } else { "slate" },
                "Optimise images when they are uploaded",
                if s.optimize {
                    "On: JPEG and PNG uploads are sized, recompressed and cleaned before they are stored."
                } else {
                    "Off: uploads are stored exactly as they arrive."
                },
                &switch(rules::SETTING_OPTIMIZE, s.optimize),
            ),
            max = row(
                "panel-show",
                "blue",
                "Largest side, in pixels",
                "Anything bigger is scaled down to this. 2000 covers every size a theme asks for.",
                &format!(
                    r#"<label class="field" style="margin:0;max-width:140px"><input type="number" name="{key}" value="{v}" min="{min}" max="{max}" step="100" placeholder=" "><span>Pixels</span></label>"#,
                    key = rules::SETTING_MAX_DIMENSION,
                    v = s.max_dimension,
                    min = rules::MIN_MAX_DIMENSION,
                    max = rules::MAX_MAX_DIMENSION,
                ),
            ),
            quality = row(
                "tune",
                "purple",
                "JPEG quality",
                "82 looks the same as the original on screen at about a third of the size. PNGs are compressed losslessly.",
                &format!(
                    r#"<label class="field" style="margin:0;max-width:140px"><input type="number" name="{key}" value="{v}" min="{min}" max="{max}" placeholder=" "><span>1–100</span></label>"#,
                    key = rules::SETTING_JPEG_QUALITY,
                    v = s.jpeg_quality,
                    min = rules::MIN_JPEG_QUALITY,
                    max = rules::MAX_JPEG_QUALITY,
                ),
            ),
            strip = row(
                "lock",
                "red",
                "Remove camera metadata",
                "Including the GPS position of where a photo was taken. The photo is turned the right way up first.",
                &switch(rules::SETTING_STRIP_METADATA, s.strip_metadata),
            ),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn images_settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    // Through the same reader uploads use, so what is stored is exactly what
    // will be applied: a box is ticked or not, numbers are clamped.
    let s = ImageSettings::read(|key| match key {
        rules::SETTING_OPTIMIZE | rules::SETTING_STRIP_METADATA => {
            Some(if form.get(key).is_some_and(|v| v == "1") { "true" } else { "false" }.to_string())
        }
        _ => form.get(key).cloned(),
    });
    let writes = [
        (rules::SETTING_OPTIMIZE, serde_json::Value::Bool(s.optimize)),
        (rules::SETTING_MAX_DIMENSION, serde_json::json!(s.max_dimension)),
        (rules::SETTING_JPEG_QUALITY, serde_json::json!(s.jpeg_quality)),
        (rules::SETTING_STRIP_METADATA, serde_json::Value::Bool(s.strip_metadata)),
    ];
    let mut ok = true;
    for (key, value) in writes {
        if let Err(e) = state.settings.set(&state.db, key, &value).await {
            tracing::error!("images: saving {key} failed: {}", e.message());
            ok = false;
        }
    }
    if ok {
        settings_saved(&state, "images", "Image settings saved.")
    } else {
        settings_flash("images", "error", "Saving failed — check the logs.")
    }
}

// ---------------------------------------------------------------------------
// the library: what is left to do, and doing it
// ---------------------------------------------------------------------------

/// Every JPEG and PNG under `content/images`, relative to it.
///
/// Skips `size/` (the old on-disk layout for resized copies, in case a site
/// was migrated with one) and Ghost's `_o` originals, which are backups and
/// not images anybody links to.
fn library_images(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                // Never followed: a link out of the content directory would
                // let this rewrite files it has no business with.
                continue;
            }
            if kind.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) != Some("size") {
                    stack.push(path);
                }
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
            if rules::optimizable(ext) && !name.contains("_o.") {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
    out.sort();
    out
}

/// What the library card says: how many are done, how many are not.
pub(crate) struct LibraryState {
    pub(crate) total: usize,
    pub(crate) done: usize,
}

pub(crate) fn library_state(state: &AppState) -> LibraryState {
    let root = state.config.images_dir();
    let backups = backups_root(state);
    let all = library_images(&root);
    let done = all.iter().filter(|rel| backups.join(rel).exists()).count();
    LibraryState { total: all.len(), done }
}

/// The card on the Media screen.
pub(crate) fn library_card(state: &AppState) -> String {
    let settings = image_settings(state);
    let lib = library_state(state);
    let left = lib.total.saturating_sub(lib.done);
    let status = if !settings.optimize {
        "Optimisation is off. New uploads are stored exactly as they arrive.".to_string()
    } else {
        format!(
            "On for new uploads — scaled to {}px, JPEG quality {}{}.",
            settings.max_dimension,
            settings.jpeg_quality,
            if settings.strip_metadata { ", camera metadata removed" } else { "" },
        )
    };
    let button = if left == 0 || !settings.optimize {
        String::new()
    } else {
        format!(
            r#"<form method="post" action="{OPTIMIZE_PATH}" style="margin:0"
 data-confirm="Optimise existing images|Up to {batch} images are replaced with smaller versions. A private copy of each original is kept in content/data/image-originals.">
<button class="btn tonal" type="submit">{ic} Optimise {n}</button></form>"#,
            batch = BATCH,
            ic = svg("image"),
            n = if left > BATCH { format!("the next {BATCH}") } else { countable(left as i64, "image") },
        )
    };
    format!(
        r#"<div class="card padded im-card rise" style="--i:1">
<span class="ic tint-{tint}">{ic}</span>
<div class="im-text"><span class="im-title">Image optimisation</span>
<span class="im-detail">{status}</span>
<span class="im-detail">{progress}</span></div>
<div class="im-actions">{button}<a class="btn text" href="{SETTINGS_PATH}">{gear} Image settings</a></div>
</div>"#,
        tint = if settings.optimize { "green" } else { "slate" },
        ic = svg("image"),
        status = esc(&status),
        progress = esc(&match (lib.total, left) {
            (0, _) => "No JPEG or PNG files in the library yet.".to_string(),
            (_, 0) => format!("All {} in the library are done.", countable(lib.total as i64, "image")),
            (t, l) => format!("{l} of {} not optimised yet.", countable(t as i64, "image")),
        }),
        button = button,
        gear = svg("settings"),
    )
}

/// One press of "Optimise".
pub(crate) async fn optimize_library(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let settings = image_settings(&state);
    if !settings.optimize {
        return flash_to("/ghost/media/", "error", "Switch optimisation on in Settings → Images first.");
    }
    let root = state.config.images_dir();
    let backups = backups_root(&state);

    let outcome = tokio::task::spawn_blocking(move || {
        let todo: Vec<_> = library_images(&root)
            .into_iter()
            .filter(|rel| !backups.join(rel).exists())
            .collect();
        let remaining = todo.len().saturating_sub(BATCH);
        let (mut changed, mut untouched, mut saved, mut failed) = (0usize, 0usize, 0i64, 0usize);
        for rel in todo.into_iter().take(BATCH) {
            let path = root.join(&rel);
            let Ok(bytes) = std::fs::read(&path) else {
                failed += 1;
                continue;
            };
            let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
            let backup = backups.join(&rel);
            // The backup is written first, and for every file the run looks
            // at — kept or not — because it is also the "already done"
            // marker. Without it, a file the optimiser decided to leave alone
            // would be decoded again on every press, forever.
            if let Some(parent) = backup.parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    failed += 1;
                    continue;
                }
            }
            if std::fs::write(&backup, &bytes).is_err() {
                failed += 1;
                continue;
            }
            match ghost_api::image_optimize::optimize(&bytes, ext, &settings) {
                Some(out) => {
                    // Written beside the original and renamed over it, so a
                    // reader never gets half a file.
                    let temporary = path.with_extension(format!("{ext}.optimising"));
                    if std::fs::write(&temporary, &out.bytes).is_ok() && std::fs::rename(&temporary, &path).is_ok() {
                        changed += 1;
                        saved += bytes.len() as i64 - out.bytes.len() as i64;
                    } else {
                        let _ = std::fs::remove_file(&temporary);
                        failed += 1;
                    }
                }
                None => untouched += 1,
            }
        }
        (changed, untouched, saved, failed, remaining)
    })
    .await;

    let Ok((changed, untouched, saved, failed, remaining)) = outcome else {
        return flash_to("/ghost/media/", "error", "The optimiser stopped unexpectedly. Nothing was lost.");
    };
    let mut message = format!("Optimised {}", countable(changed as i64, "image"));
    if saved > 0 {
        message.push_str(&format!(", saving {}", rules::human_bytes(saved as u64)));
    }
    message.push('.');
    match untouched {
        0 => {}
        1 => message.push_str(" 1 image was already as small as it would get."),
        n => message.push_str(&format!(" {n} images were already as small as they would get.")),
    }
    match failed {
        0 => {}
        1 => message.push_str(" 1 image could not be read and was left as it was."),
        n => message.push_str(&format!(" {n} images could not be read and were left as they were.")),
    }
    if remaining > 0 {
        message.push_str(&format!(" {remaining} left — press Optimise again."));
    }
    flash_to("/ghost/media/", if failed > 0 { "error" } else { "ok" }, &message)
}

// ---------------------------------------------------------------------------
// one image's versions, on its own page
// ---------------------------------------------------------------------------

/// "Versions" on an image's page: the copy visitors get, then everything else
/// that exists of it — the same list `purge_versions` deletes, from the same
/// function, so the page never shows a version a delete would leave behind.
pub(crate) async fn versions_section(
    state: &AppState,
    at: &Location,
    current_path: &std::path::Path,
    current_bytes: u64,
) -> String {
    use ghost_api::image_optimize::{self as opt, VersionKind};
    if at.kind != Kind::Image {
        return String::new();
    }
    let versions = opt::find_versions(state, &at.rest).await;

    // Dimensions for every file that will be shown on its own row, read from
    // headers on a blocking thread. Cached resizes are counted, not listed.
    let mut paths: Vec<std::path::PathBuf> = vec![current_path.to_path_buf()];
    paths.extend(versions.iter().filter(|v| v.kind != VersionKind::Cached).map(|v| v.path.clone()));
    let dims: Vec<Option<(u32, u32)>> = tokio::task::spawn_blocking(move || {
        paths.iter().map(|p| opt::dimensions(p)).collect()
    })
    .await
    .unwrap_or_default();
    let dim = |i: usize| match dims.get(i).copied().flatten() {
        Some((w, h)) => format!("{w} × {h}"),
        None => "—".to_string(),
    };

    let row = |icon: &str, tint: &str, title: &str, detail: &str, trailing: &str| {
        format!(
            r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{trailing}</div></div>"#,
            ic = svg(icon),
            title = esc(title),
            detail = esc(detail),
        )
    };
    let open = |url: &str| {
        format!(
            r#"<a class="btn text small" href="{url}" target="_blank" rel="noopener">{ic} Open</a>"#,
            url = esc(url),
            ic = svg("open"),
        )
    };

    let original = versions.iter().find(|v| v.kind == VersionKind::Original);
    let mut rows = row(
        "image",
        "green",
        "This file",
        &format!(
            "{} · {} · what visitors are served{}",
            dim(0),
            ghost_core::media::format_bytes(current_bytes),
            match original {
                Some(o) if o.bytes > current_bytes => format!(
                    " · {:.0}% smaller than the original",
                    100.0 - current_bytes as f64 * 100.0 / o.bytes as f64
                ),
                _ => String::new(),
            },
        ),
        &open(&at.url()),
    );

    let mut shown = 1usize;
    for v in versions.iter().filter(|v| v.kind != VersionKind::Cached) {
        let d = dim(shown);
        shown += 1;
        rows.push_str(&match &v.kind {
            VersionKind::Original => row(
                "lock",
                "slate",
                "Original, before optimisation",
                &format!(
                    "{d} · {} · private: kept in content/data and never served",
                    ghost_core::media::format_bytes(v.bytes)
                ),
                &format!(
                    r#"<a class="btn text small" href="/ghost/media/item/original/?{q}" target="_blank" rel="noopener">{ic} View</a>"#,
                    q = form_urlencoded::Serializer::new(String::new()).append_pair("path", &at.path()).finish(),
                    ic = svg("eye"),
                ),
            ),
            VersionKind::GhostOriginal => row(
                "history",
                "amber",
                "Ghost's original",
                &format!("{d} · {} · from the site this was imported from, and public", ghost_core::media::format_bytes(v.bytes)),
                &v.url.as_deref().map(open).unwrap_or_default(),
            ),
            VersionKind::GhostSize { spec, format } => row(
                "panel-show",
                "blue",
                &match format {
                    Some(f) => format!("Ghost size {spec} · {f}"),
                    None => format!("Ghost size {spec}"),
                },
                &format!("{d} · {} · from the site this was imported from", ghost_core::media::format_bytes(v.bytes)),
                &v.url.as_deref().map(open).unwrap_or_default(),
            ),
            VersionKind::Cached => unreachable!("filtered above"),
        });
    }

    let cached: Vec<_> = versions.iter().filter(|v| v.kind == VersionKind::Cached).collect();
    if !cached.is_empty() {
        let bytes: u64 = cached.iter().map(|v| v.bytes).sum();
        rows.push_str(&row(
            "tune",
            "purple",
            &match cached.len() {
                1 => "1 resized copy for themes".to_string(),
                n => format!("{n} resized copies for themes"),
            },
            &format!(
                "{} · made on demand at the sizes the theme asks for, and made again if removed",
                ghost_core::media::format_bytes(bytes)
            ),
            "",
        ));
    }

    let others = versions.len();
    format!(
        r#"<div class="section">Versions</div>
<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted rise" style="--i:4;margin:10px 4px 0">{note}</p>"#,
        note = esc(&match others {
            0 => "Only the one copy exists. Optimising it, or a theme asking for a smaller size, adds versions here.".to_string(),
            n => format!(
                "Deleting this image deletes {} with it, so nothing is left behind — including the private original.",
                if n == 1 { "this other version".to_string() } else { format!("all {n} other versions") }
            ),
        }),
    )
}

/// The private original, for an administrator to look at.
///
/// `content/data` is never served to the public — that is the point of
/// keeping originals there — so the only way to see one is through the admin,
/// behind the module's guard, and never cached by anything on the way.
pub(crate) async fn original_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, false).await {
        return r;
    }
    let Some(at) = q.get("path").and_then(|p| ghost_core::media::parse_location(p)).filter(|at| at.kind == Kind::Image) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let path = at.rest.split('/').fold(backups_root(&state), |p, seg| p.join(seg));
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return (StatusCode::NOT_FOUND, "This image has no kept original.").into_response();
    };
    let mime = match at.extension().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        _ => "image/jpeg",
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}
