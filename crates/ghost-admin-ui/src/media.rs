//! The Media module: the publication's files, as a library you can work in.
//!
//! One screen lists everything under `content/images`, `content/media` and
//! `content/files`; a second screen is one file in full. Between them the
//! module can upload, describe, rename, replace and delete, and can say which
//! posts a file is used by before it is removed.
//!
//! ## The library is the filesystem
//!
//! There is no table listing what exists. The three content directories are
//! the record, and [`scan`] walks them on each request. That is a deliberate
//! choice and the one thing to understand about this module: an install that
//! has been publishing for years already has thousands of files there with no
//! row to their name, so an index would have been wrong about every one of
//! them from the moment it was created. Walking instead means the library is
//! correct on a fresh install, on a restored backup, and after someone has
//! copied files in over SFTP — none of which a table could claim.
//!
//! What the filesystem cannot hold is the words put beside a file. Those live
//! in `media_meta`, keyed by stored path, and a file nobody has described has
//! no row at all. See `migrations/sqlite/0012_media.sql`.
//!
//! The walk is bounded: at most [`SCAN_FILES`] files and [`SCAN_DIRS`]
//! directories, after which the page says so rather than growing without
//! limit. On a local disk a few thousand files cost single-digit
//! milliseconds; the caps exist so a pathological content directory cannot
//! turn one request into an outage.
//!
//! ## Who may see it, and what is checked
//!
//! Administrator only, through [`guard`] — uploaded files include whatever
//! staff have attached to drafts, and deleting one is destructive. Everything
//! that changes state is origin-checked as well as session-checked, so a
//! delete does not rest on the session cookie's `SameSite=Lax` alone.
//!
//! Every path arriving from a request goes through
//! [`ghost_core::media::parse_location`] before anything opens, moves or
//! unlinks a file, and the string that reaches the filesystem is then
//! *recomposed* from the parsed parts rather than being the one that arrived.
//! A `..` is refused, not popped. The destination of a rename is built by
//! [`ghost_core::media::rename_target`], which keeps the original extension —
//! so a rename cannot turn a `.png` into a `.html` and have this site serve it
//! as a document from its own origin.
//!
//! Uploads go through [`ghost_api::images::store`], the same function the
//! editor's drag-and-drop uses, so the type allowlist, the size ceilings, the
//! host's upload limit and the naming scheme are shared rather than restated
//! here.

use super::*;

mod inliner;
pub(crate) use inliner::*;

use ghost_core::media::{self as rules, Kind, Location};
use ghost_db::repos::media as repo;

/// Files shown per page of the grid.
const PER_PAGE: usize = 48;
/// The most files one walk will collect.
const SCAN_FILES: usize = 20_000;
/// The most directories one walk will open.
const SCAN_DIRS: usize = 2_000;
/// The most posts named on a file's "used in" list. The count beside it is
/// the true total.
const USAGE_SHOWN: i64 = 12;

// ---------------------------------------------------------------------------
// the module's security contract, and its redirects
// ---------------------------------------------------------------------------

/// The role check, plus — for anything that changes state — the origin check.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

/// Back to one file's own screen.
fn back_to(at: &Location) -> String {
    format!("/ghost/media/item/?{}", form_urlencoded::Serializer::new(String::new()).append_pair("path", &at.path()).finish())
}

fn server_error(message: &str) -> Response {
    tracing::error!("media admin: {message}");
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

// ---------------------------------------------------------------------------
// reading the content directories
// ---------------------------------------------------------------------------

/// One file found on disk.
pub(crate) struct Asset {
    pub(crate) at: Location,
    pub(crate) bytes: u64,
    /// Last modified, as seconds since the epoch. `0` where the filesystem
    /// would not say, which sorts such a file to the end rather than failing.
    pub(crate) modified: i64,
}

/// What one walk of the content directories found.
pub(crate) struct Library {
    pub(crate) assets: Vec<Asset>,
    /// True when a cap was reached, so the page can say the list is partial
    /// rather than quietly showing a prefix of the truth.
    pub(crate) capped: bool,
}

/// Walk the three content directories.
///
/// Breadth-first, bounded by [`SCAN_FILES`] and [`SCAN_DIRS`]. Three kinds of
/// entry are passed over, each for its own reason:
///
/// * **dot-files**, which are bookkeeping (`.DS_Store`, `.gitkeep`) and not
///   something anyone uploaded;
/// * **symlinks** — `DirEntry::metadata` does not follow one, so a link
///   reports as neither file nor directory and falls through. A link could
///   point anywhere, including out of the content tree, and the public file
///   route refuses to serve one for the same reason;
/// * **names [`rules::parse_location`] will not accept**, so nothing reaches
///   the grid that the item screen would then refuse to open.
pub(crate) async fn scan(state: &AppState) -> Library {
    let mut assets = Vec::new();
    let mut capped = false;
    let mut dirs_opened = 0usize;

    for kind in Kind::ALL.iter().copied() {
        let root = match kind {
            Kind::Image => state.config.images_dir(),
            Kind::Media => state.config.media_dir(),
            Kind::File => state.config.files_dir(),
        };
        // Relative to `root`; the empty string is the root itself.
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::from([String::new()]);
        while let Some(rel) = queue.pop_front() {
            if assets.len() >= SCAN_FILES || dirs_opened >= SCAN_DIRS {
                capped = true;
                break;
            }
            dirs_opened += 1;
            let dir = if rel.is_empty() { root.clone() } else { root.join(&rel) };
            // A missing directory is the ordinary case on a site that has
            // never uploaded a video, not an error worth reporting.
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else { continue };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let Ok(name) = entry.file_name().into_string() else { continue };
                if name.starts_with('.') {
                    continue;
                }
                let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                let Ok(meta) = entry.metadata().await else { continue };
                if meta.is_dir() {
                    queue.push_back(child);
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                let Some(at) = rules::parse_location(&format!("{}/{child}", kind.as_str())) else { continue };
                assets.push(Asset {
                    at,
                    bytes: meta.len(),
                    modified: meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0),
                });
            }
        }
    }
    Library { assets, capped }
}

/// Whether a file is still there, and how big it is.
///
/// Takes a [`Location`], so the path it opens is one `parse_location` built.
async fn stat(state: &AppState, at: &Location) -> Option<Asset> {
    let meta = tokio::fs::metadata(disk_path(state, at)).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(Asset {
        at: at.clone(),
        bytes: meta.len(),
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    })
}

/// Where a [`Location`] actually lives.
///
/// The only place a media path becomes a filesystem path, and it takes a
/// `Location` rather than a string so there is no way to call it with
/// something unparsed.
fn disk_path(state: &AppState, at: &Location) -> std::path::PathBuf {
    let root = match at.kind {
        Kind::Image => state.config.images_dir(),
        Kind::Media => state.config.media_dir(),
        Kind::File => state.config.files_dir(),
    };
    // `rest` holds no `..`, no empty segment and no absolute prefix — that is
    // what `parse_location` establishes — so joining the segments cannot
    // leave the root.
    at.rest.split('/').fold(root, |path, segment| path.join(segment))
}

/// A timestamp from the filesystem, in the publication's own timezone.
fn when(modified: i64, timezone: &str) -> String {
    if modified <= 0 {
        return String::new();
    }
    match chrono::DateTime::from_timestamp(modified, 0) {
        Some(dt) => ghost_core::dates::short_date_in_zone(&ghost_core::dates::to_db_string(&dt), timezone),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// the library: filters, reduced to values that are safe to link
// ---------------------------------------------------------------------------

const SORTS: &[(&str, &str)] = &[
    ("new", "Newest first"),
    ("old", "Oldest first"),
    ("big", "Largest first"),
    ("name", "By name"),
];

/// How a request asked for the library to be narrowed, after every value has
/// been reduced to one this module offers.
///
/// Nothing here can be forged into a link or a query: `kind` is one of four
/// literals, `sort` one of four, `folder` is only kept if the walk actually
/// found that folder, and `q` is bounded and de-controlled. `folder` being
/// checked against the scan rather than against a pattern is what makes it
/// safe — it can only ever be a directory that exists.
struct Filters {
    kind: Option<Kind>,
    sort: &'static str,
    folder: String,
    q: String,
}

fn filters(q: &HashMap<String, String>, folders: &[String]) -> Filters {
    let kind = q.get("kind").map(String::as_str).unwrap_or("all");
    let sort = q.get("sort").map(String::as_str).unwrap_or("new");
    Filters {
        kind: Kind::parse(kind),
        sort: SORTS.iter().map(|(v, _)| *v).find(|v| *v == sort).unwrap_or("new"),
        folder: q
            .get("folder")
            .filter(|f| folders.iter().any(|known| known == *f))
            .cloned()
            .unwrap_or_default(),
        q: rules::clean_text(q.get("q").map(String::as_str).unwrap_or(""), false, 80),
    }
}

impl Filters {
    /// These filters as a query string, optionally with one value swapped.
    /// Used for every link on the page, so a filter survives changing another.
    fn query<'a>(&'a self, over: &[(&'a str, &'a str)]) -> String {
        let mut s = form_urlencoded::Serializer::new(String::new());
        let pick = |key: &str, current: &'a str| -> &'a str {
            over.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or(current)
        };
        let kind = pick("kind", self.kind.map(Kind::as_str).unwrap_or("all"));
        if kind != "all" {
            s.append_pair("kind", kind);
        }
        let folder = pick("folder", &self.folder);
        if !folder.is_empty() {
            s.append_pair("folder", folder);
        }
        let sort = pick("sort", self.sort);
        if sort != "new" {
            s.append_pair("sort", sort);
        }
        let q = pick("q", &self.q);
        if !q.is_empty() {
            s.append_pair("q", q);
        }
        let page = pick("page", "");
        if !page.is_empty() && page != "1" {
            s.append_pair("page", page);
        }
        s.finish()
    }

    fn href<'a>(&'a self, over: &[(&'a str, &'a str)]) -> String {
        let query = self.query(over);
        if query.is_empty() {
            "/ghost/media/".to_string()
        } else {
            format!("/ghost/media/?{query}")
        }
    }

    /// Whether one file is in what was asked for.
    fn matches(&self, asset: &Asset, meta: Option<&repo::Meta>) -> bool {
        if self.kind.is_some_and(|k| k != asset.at.kind) {
            return false;
        }
        if !self.folder.is_empty() && asset.at.folder() != self.folder {
            return false;
        }
        if self.q.is_empty() {
            return true;
        }
        // Searched over everything shown about a file, so looking for the
        // words in a caption finds it as readily as looking for its name.
        let needle = self.q.to_lowercase();
        let haystack = [
            asset.at.path(),
            meta.map(|m| m.title.clone()).unwrap_or_default(),
            meta.map(|m| m.alt.clone()).unwrap_or_default(),
            meta.map(|m| m.caption.clone()).unwrap_or_default(),
        ]
        .join(" ")
        .to_lowercase();
        haystack.contains(&needle)
    }
}

// ---------------------------------------------------------------------------
// the library screen
// ---------------------------------------------------------------------------

pub(crate) async fn library_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let library = scan(&state).await;
    let meta = repo::all(&state.db).await.unwrap_or_default();

    // Every folder the walk found, newest-looking first — the names are
    // `YYYY/MM`, so a reverse sort is chronological. This is also the set a
    // `folder=` parameter is checked against.
    let mut folders: Vec<String> =
        library.assets.iter().map(|a| a.at.folder().to_string()).filter(|f| !f.is_empty()).collect();
    folders.sort();
    folders.dedup();
    folders.reverse();

    let f = filters(&q, &folders);
    let page = q.get("page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).clamp(1, 100_000);

    // Counts for the tabs come from everything on disk narrowed by the
    // *other* filters, so each tab says how many it would show if picked.
    // Built once rather than per asset — it does not depend on the asset.
    let without_kind = Filters { kind: None, sort: f.sort, folder: f.folder.clone(), q: f.q.clone() };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut shown: Vec<&Asset> = Vec::new();
    let mut total_bytes = 0u64;
    for asset in &library.assets {
        let m = meta.get(&asset.at.path());
        total_bytes += asset.bytes;
        if without_kind.matches(asset, m) {
            *counts.entry(asset.at.kind.as_str()).or_default() += 1;
            *counts.entry("all").or_default() += 1;
        }
        if f.matches(asset, m) {
            shown.push(asset);
        }
    }
    // Ties are broken by family and path so two requests order the library
    // the same way, rather than following whatever the directory happened to
    // list. Compared on the borrowed fields, not on `path()`, which would
    // allocate a String on every comparison of an n·log n sort.
    fn tiebreak(a: &Asset) -> (&'static str, &str) {
        (a.at.kind.as_str(), a.at.rest.as_str())
    }
    match f.sort {
        "old" => shown.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| tiebreak(a).cmp(&tiebreak(b)))),
        "big" => shown.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| tiebreak(a).cmp(&tiebreak(b)))),
        "name" => shown.sort_by_key(|a| a.at.name().to_lowercase()),
        _ => shown.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| tiebreak(a).cmp(&tiebreak(b)))),
    }

    let total = shown.len();
    let pages = total.div_ceil(PER_PAGE).max(1);
    let page = page.min(pages);
    let tiles: String = shown
        .iter()
        .skip((page - 1) * PER_PAGE)
        .take(PER_PAGE)
        .enumerate()
        .map(|(i, asset)| tile(asset, meta.get(&asset.at.path()), &timezone, i))
        .collect();

    let tabs: String = std::iter::once(("all", "All"))
        .chain(Kind::ALL.iter().map(|k| (k.as_str(), k.label())))
        .map(|(value, label)| {
            format!(
                r#"<a class="seg{on}" href="{href}">{label}<span class="seg-count">{count}</span></a>"#,
                on = if value == f.kind.map(Kind::as_str).unwrap_or("all") { " on" } else { "" },
                href = esc(&f.href(&[("kind", value), ("page", "1")])),
                label = esc(label),
                count = counts.get(value).copied().unwrap_or(0),
            )
        })
        .collect();

    let mut folder_options: Vec<(&str, &str)> = vec![("", "Every month")];
    folder_options.extend(folders.iter().map(|f| (f.as_str(), f.as_str())));

    let grid = if total == 0 {
        format!(
            r#"<div class="card padded rise" style="--i:3;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">{title}</p>
<p class="muted" style="margin:0">{detail}</p></div>"#,
            title = if library.assets.is_empty() { "Nothing uploaded yet" } else { "Nothing matches that" },
            detail = if library.assets.is_empty() {
                // Upload first: that button works with or without JavaScript,
                // where dropping needs the script to have run.
                "Press Upload above — or drop files in here. Images, video, audio and documents all go in the same library, filed by what they are."
            } else {
                "Try a different search, month, or file type."
            },
        )
    } else {
        format!(r#"<div class="ml-grid rise" style="--i:3">{tiles}</div>"#)
    };

    let note = if library.capped {
        format!(
            // `info`, not `warn`: the stylesheet defines ok, error and info,
            // and nothing has gone wrong here — the list is just partial.
            r#"<div class="banner info">{flag}This library is larger than one page can walk, so only the first {limit} files are listed. Narrow it with the search box or a month.</div>"#,
            flag = svg("flag"),
            limit = SCAN_FILES,
        )
    } else {
        String::new()
    };

    page_for_role(
        actor.role,
        "Media",
        "media",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/modules/" title="Back to modules">{back}</a>
<div><h1>Media</h1><p class="sub">{summary}</p></div>
<div class="actions">{upload_form}</div></header>
{banner}{note}
<div class="seg-bar rise" style="--i:1">{tabs}</div>
<form class="card padded ml-bar rise" style="--i:2" method="get" action="/ghost/media/">
{kept}
<label class="field ml-search" style="margin:0"><input name="q" value="{query}" placeholder=" " maxlength="80" autocomplete="off"><span>Search names, titles and captions</span></label>
<label class="field fixed" style="margin:0"><select name="folder">{folder_opts}</select><span>Month</span>{car}</label>
<label class="field fixed" style="margin:0"><select name="sort">{sort_opts}</select><span>Order</span>{car}</label>
<button class="btn tonal" type="submit">{search} Search</button>
{clear}
</form>
<div data-batch>
<div class="ml-drop" data-ml-drop data-ml="{config}">
<p class="ml-hint" data-ml-hint hidden>{upload_ic} Drop files anywhere in here to upload</p>
{grid}
</div>
{batch}
</div>
{pager}
<script src="{assets}/media-library.js?v={script_hash}" defer></script>"#,
            back = svg("chevron"),
            summary = esc(&summary(&library, total_bytes)),
            upload_form = upload_form(),
            banner = flash_banner(&q),
            note = note,
            tabs = tabs,
            // The filters a search should not throw away travel as hidden
            // fields, since this form replaces the whole query string.
            kept = [("kind", f.kind.map(Kind::as_str).unwrap_or("all"))]
                .iter()
                .filter(|(_, v)| *v != "all")
                .map(|(k, v)| format!(r#"<input type="hidden" name="{k}" value="{}">"#, esc(v)))
                .collect::<String>(),
            query = esc(&f.q),
            folder_opts = select_options(&folder_options, &f.folder),
            sort_opts = select_options(SORTS, f.sort),
            car = SELECT_CARET,
            search = svg("search"),
            // Clears the search, the month and the order, but deliberately
            // not the family tab: that is the segmented bar above, and a
            // "Clear" inside the toolbar should not reach out and change it.
            clear = if f.q.is_empty() && f.folder.is_empty() && f.sort == "new" {
                String::new()
            } else {
                format!(
                    r#"<a class="btn text" href="{href}">{close} Clear</a>"#,
                    href = esc(&f.href(&[("q", ""), ("folder", ""), ("sort", "new"), ("page", "1")])),
                    close = svg("close"),
                )
            },
            config = esc(&serde_json::json!({
                // The script cannot read the base path off its own already
                // rewritten URL, so it is handed the prefix to post to.
                "uploadUrl": format!("{}/ghost/media/upload/", ghost_core::url::subdir()),
                "maxBytes": Kind::ALL.iter().map(|k| k.max_bytes()).max().unwrap_or(0),
            }).to_string()),
            upload_ic = svg("upload"),
            grid = grid,
            batch = batch_bar(),
            pager = pager(&f, page, pages),
            assets = ASSETS,
            script_hash = library_script_hash(),
        ),
        true,
    )
    .into_response()
}

/// A hash of the library script, for its URL. See [`crate::asset_hash`] for
/// why every admin asset is requested with one.
fn library_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/media-library.js")));
    &HASH
}

/// "1,204 files · 2.3 GB" — what the whole library adds up to, whatever the
/// filters currently show.
fn summary(library: &Library, total_bytes: u64) -> String {
    if library.assets.is_empty() {
        return "Images, video, audio and documents for this publication.".into();
    }
    format!(
        "{} · {} on disk",
        countable(library.assets.len() as i64, "file"),
        rules::format_bytes(total_bytes),
    )
}

/// The Upload control.
///
/// A real `<form>` with a real file input, so it works with no JavaScript at
/// all: pick files, press Upload, land back here with a flash. The script
/// upgrades it in place — it posts the same files to the same URL over fetch,
/// shows progress, and hides the submit button it no longer needs.
fn upload_form() -> String {
    format!(
        r#"<form class="ml-upload" method="post" action="/ghost/media/upload/" enctype="multipart/form-data" data-ml-upload>
<input class="ml-pick" type="file" name="file" id="ml-file" multiple>
<label class="btn" for="ml-file">{upload} Upload</label>
<button class="btn tonal" type="submit" data-ml-submit>Send selected files</button>
<span class="muted ml-state" role="status" data-ml-state></span>
</form>"#,
        upload = svg("upload"),
    )
}

/// The floating bar the selection acts through.
///
/// A sibling of the tiles rather than their parent, so each tile's own link
/// stays valid HTML; the checkboxes join this form with `form=`, exactly as
/// the posts list does. Visibility is CSS, so selecting needs no script.
fn batch_bar() -> String {
    format!(
        r#"<form id="batch-media" class="batch" method="post" action="/ghost/media/delete/" data-word="file">
<div class="batch-bar">
<span class="bb-count"><b data-count>0 files</b> selected</span>
<button class="bb-btn danger" type="submit" formnovalidate
 data-confirm="Delete selected files|The selected files will be permanently deleted."
 data-confirm-one="Delete 1 file|This file will be permanently deleted, including from any post that uses it."
 data-confirm-many="Delete {{n}} files|The {{n}} selected files will be permanently deleted, including from any post that uses them.">{del} Delete</button>
</div>
</form>"#,
        del = svg("delete"),
    )
}

/// One file as a tile.
///
/// An image shows itself, lazily and at no stated size — a 4000px photo and a
/// 40px icon both have to sit in the same square, which CSS `object-fit` does
/// and a width attribute cannot. Everything else shows its family's glyph with
/// the extension under it, which says more at tile size than a generic
/// document icon does.
fn tile(asset: &Asset, meta: Option<&repo::Meta>, timezone: &str, index: usize) -> String {
    let at = &asset.at;
    let name = at.name();
    let label = meta.map(|m| m.title.as_str()).filter(|t| !t.is_empty()).unwrap_or(name);
    let preview = if at.is_viewable_image() {
        // In site terms, with no base path on it: `src` is one of the
        // attributes `apply_base_path` rewrites, and prefixing it here as well
        // would make it `/blog/blog/content/…` on a subdirectory install.
        format!(r#"<img src="{src}" alt="" loading="lazy" decoding="async">"#, src = esc(&at.url()))
    } else {
        format!(
            r#"<span class="ml-glyph ic tint-{tint}">{glyph}</span><span class="ml-ext">{ext}</span>"#,
            tint = match at.kind {
                Kind::Image => "pink",
                Kind::Media => "purple",
                Kind::File => "slate",
            },
            glyph = svg(at.kind.glyph()),
            ext = esc(&at.extension().to_uppercase()),
        )
    };
    format!(
        r#"<article class="ml-tile rise" style="--i:{delay}">
<label class="row-check"><input type="checkbox" form="batch-media" name="paths" value="{path}" aria-label="Select {label_attr}"><span class="box">{check}</span></label>
<a class="ml-open" href="{href}" title="{label_attr}">
<span class="ml-shot{empty}">{preview}</span>
<span class="ml-meta"><span class="ml-name">{label}</span>
<span class="ml-sub">{size}{dot}{when}</span></span>
</a>
<button class="icon-btn ml-copy" type="button" data-copy="{url}" title="Copy the link to this file" aria-label="Copy the link to {label_attr}">{copy}</button>
</article>"#,
        // Stagger the reveal, but cap it: on a full page the last tile would
        // otherwise arrive long after the first.
        delay = index.min(14),
        path = esc(&at.path()),
        label_attr = esc(label),
        check = svg("check"),
        href = esc(&back_to(at)),
        empty = if at.is_viewable_image() { "" } else { " ml-shot-glyph" },
        preview = preview,
        label = esc(label),
        size = esc(&rules::format_bytes(asset.bytes)),
        dot = if when(asset.modified, timezone).is_empty() { "" } else { " · " },
        when = esc(&when(asset.modified, timezone)),
        // Unlike `src` and `href`, `data-copy` is not an attribute
        // `apply_base_path` rewrites — so what is copied to the clipboard has
        // to carry the install's base path already, or it would be a link that
        // only works at a domain root.
        url = esc(&ghost_core::url::base(&at.url())),
        copy = svg("copy"),
    )
}

fn pager(f: &Filters, page: usize, pages: usize) -> String {
    if pages <= 1 {
        return String::new();
    }
    let link = |p: usize, label: &str| {
        format!(
            r#"<a class="btn tonal small" href="{href}">{label}</a>"#,
            href = esc(&f.href(&[("page", &p.to_string())])),
        )
    };
    format!(
        r#"<div style="display:flex;gap:10px;align-items:center;padding:14px 4px">
{prev}<span class="muted">Page {page} of {pages}</span>{next}</div>"#,
        prev = if page > 1 { link(page - 1, "← Newer") } else { String::new() },
        next = if page < pages { link(page + 1, "Older →") } else { String::new() },
    )
}

// ---------------------------------------------------------------------------
// uploading
// ---------------------------------------------------------------------------

/// Accept however many files one post carries.
///
/// No family picker: each file is filed by what it is
/// ([`rules::kind_for_upload`]), and the family it lands in then applies its
/// own allowlist — so claiming `image/png` for a shell script gets it
/// refused rather than filed as a picture.
///
/// Answers JSON when the request asked for JSON, and redirects otherwise.
/// That is what lets one endpoint serve both the script's fetch (which wants
/// per-file results to report) and a plain form post (which wants to land back
/// on the library with a flash).
pub(crate) async fn upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("application/json"));

    let mut stored: Vec<serde_json::Value> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name().unwrap_or("") != "file" {
            continue;
        }
        let filename = field.file_name().map(str::to_string);
        let content_type = field
            .content_type()
            .map(str::to_string)
            .unwrap_or_else(|| "application/octet-stream".into());
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => {
                refused.push(format!("{}: {e}", shown_name(filename.as_deref())));
                continue;
            }
        };
        if data.is_empty() {
            // An empty file input posts a field with no filename and no
            // bytes; that is "nothing was chosen", not a failure.
            continue;
        }
        let kind = rules::kind_for_upload(&content_type);
        match ghost_api::images::store(&state, kind, filename.as_deref(), &content_type, &data).await {
            Ok(saved) => stored.push(serde_json::json!({
                "url": saved.url,
                "path": saved.path,
                "name": shown_name(filename.as_deref()),
            })),
            Err(e) => refused.push(format!("{}: {}", shown_name(filename.as_deref()), e.message)),
        }
    }

    if wants_json {
        let status = if stored.is_empty() && !refused.is_empty() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };
        return (status, axum::Json(serde_json::json!({ "stored": stored, "refused": refused }))).into_response();
    }
    match (stored.len(), refused.len()) {
        (0, 0) => flash_to("/ghost/media/", "error", "Choose a file to upload first."),
        (0, _) => flash_to("/ghost/media/", "error", &format!("Nothing was uploaded. {}", refused.join("; "))),
        (n, 0) => flash_to("/ghost/media/", "ok", &format!("Uploaded {}.", countable(n as i64, "file"))),
        (n, _) => flash_to(
            "/ghost/media/",
            "error",
            &format!("Uploaded {}, but not all of them. {}", countable(n as i64, "file"), refused.join("; ")),
        ),
    }
}

/// A filename, safe to put in a sentence.
///
/// Bounded and de-controlled here rather than at each point of output; it is
/// still escaped when written into the page, like every other value.
fn shown_name(filename: Option<&str>) -> String {
    let name = filename
        .unwrap_or("")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let name = rules::clean_text(&name, false, 80);
    if name.is_empty() {
        "That file".into()
    } else {
        name
    }
}

// ---------------------------------------------------------------------------
// one file
// ---------------------------------------------------------------------------

/// Read the `path` parameter, or say why it cannot be read.
///
/// Every handler below opens with this, so there is exactly one way a request
/// turns into a file this module will act on.
fn requested(q: &HashMap<String, String>) -> Option<Location> {
    rules::parse_location(q.get("path").map(String::as_str).unwrap_or(""))
}

pub(crate) async fn item_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(at) = requested(&q) else {
        return crate::not_found_page_for(actor.role, &actor.user, "That is not a file in this library.");
    };
    let Some(asset) = stat(&state, &at).await else {
        return crate::not_found_page_for(actor.role, &actor.user, "That file is no longer in this library.");
    };
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());
    let meta = repo::find(&state.db, &at).await.unwrap_or_default().unwrap_or_default();
    let (used_by, used_total) = repo::usage(&state.db, &at.url(), USAGE_SHOWN).await.unwrap_or_default();
    // Two spellings of the same place, and the difference matters.
    //
    // `site` is the stored form, with no base path on it: it goes into `src`
    // and `href`, which `apply_base_path` rewrites on the way out, and
    // prefixing it here as well would yield `/blog/blog/content/…`.
    //
    // `link` carries the base path, because it goes where that middleware
    // does not reach — `data-copy`, which is not a URL attribute, and the
    // address shown as text in the details list. Both have to be a link that
    // works as pasted.
    let site = at.url();
    let link = ghost_core::url::base(&site);

    let row = |icon: &str, tint: &str, title: &str, detail: String| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div></div>"#,
            icon = svg(icon),
            title = esc(title),
        )
    };

    let usage_card = if used_total == 0 {
        format!(
            r#"<div class="card padded rise" style="--i:5"><p class="muted" style="margin:0">No post or page points at this file. Deleting it will not leave a gap in anything published.</p></div>"#
        )
    } else {
        let rows: String = used_by
            .iter()
            .map(|u| {
                let base = if u.post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" };
                format!(
                    r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><a class="title" style="color:inherit" href="{base}{id}/">{title}</a>
<span class="detail">{what} · {status}</span></div><div class="trailing">{chev}</div></div>"#,
                    ic = svg(if u.post_type == "page" { "pages" } else { "posts" }),
                    base = base,
                    id = esc(&u.id),
                    title = esc(if u.title.is_empty() { "Untitled" } else { &u.title }),
                    what = if u.post_type == "page" { "Page" } else { "Post" },
                    status = esc(&u.status),
                    chev = svg("chevron"),
                )
            })
            .collect();
        format!(
            r#"<div class="card rows rise" style="--i:5">{rows}</div>{more}"#,
            more = if used_total > used_by.len() as i64 {
                format!(
                    r#"<p class="muted" style="margin:12px 4px 0">…and {} more.</p>"#,
                    used_total - used_by.len() as i64
                )
            } else {
                String::new()
            },
        )
    };

    page_for_role(
        actor.role,
        &shown_name(Some(at.name())),
        "media",
        Some(&actor.user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/media/" title="Back to the library">{chev}</a>
<div><h1>{heading}</h1><p class="sub">{family} · {size}{dot}{when}</p></div>
<div class="actions"><button class="btn tonal" type="button" data-copy="{link}">{copy} Copy link</button>
<a class="btn text" href="{site}" target="_blank" rel="noopener">{open} Open</a></div></header>
{banner}
<div class="ml-item rise" style="--i:1">
<div class="ml-preview">{preview}</div>
<div class="ml-side">
<form method="post" action="/ghost/media/item/save/" class="card padded">
<input type="hidden" name="path" value="{path}">
<label class="field"><input name="title" value="{title}" placeholder=" " maxlength="{max_title}"><span>Title · shown in this library</span></label>
<label class="field"><input name="alt" value="{alt}" placeholder=" " maxlength="{max_alt}"><span>Alt text · describes the image to a screen reader</span></label>
<label class="field" style="margin:0 0 14px"><textarea name="caption" placeholder=" " maxlength="{max_caption}" rows="2">{caption}</textarea><span>Caption</span></label>
<button class="btn" type="submit">{check} Save details</button>
</form>
<form method="post" action="/ghost/media/item/rename/" class="card padded">
<input type="hidden" name="path" value="{path}">
<label class="field" style="margin:0 0 6px"><input name="name" value="{name}" placeholder=" " maxlength="{max_name}" autocomplete="off" spellcheck="false"><span>File name</span></label>
<p class="muted" style="margin:0 0 14px">The extension stays as it is. Every post that links to the old name will stop finding it, so rename before a file is used rather than after.</p>
<button class="btn tonal" type="submit">{edit} Rename</button>
</form>
<form method="post" action="/ghost/media/item/replace/" class="card padded" enctype="multipart/form-data">
<input type="hidden" name="path" value="{path}">
<input class="ml-pick" type="file" name="file" id="ml-replace" accept="{accept}">
<label class="btn tonal" for="ml-replace">{upload} Choose a replacement</label>
<p class="muted" style="margin:10px 0 14px">Keeps this link and this name, so everything already using the file picks up the new version. It has to be the same kind of file — a .{ext} for a .{ext}.</p>
<button class="btn tonal" type="submit">{check} Replace the file</button>
</form>
</div>
</div>

<div class="section">Where it lives</div>
<div class="card rows rise" style="--i:4">
{link_row}{path_row}{kind_row}{size_row}{modified_row}{described_row}
</div>

<div class="section">Used by</div>
{usage_card}

<div class="section">Delete</div>
<div class="card padded rise" style="--i:6">
<p class="muted" style="margin:0 0 12px">{warning}</p>
<form method="post" action="/ghost/media/item/delete/" data-confirm="Delete file|&#8220;{name_attr}&#8221; will be permanently deleted from this server.{used_warning}">
<input type="hidden" name="path" value="{path}">
<button class="btn tonal small danger" type="submit">{del} Delete this file</button></form>
</div>"#,
            chev = svg("chevron"),
            heading = esc(if meta.title.is_empty() { at.name() } else { &meta.title }),
            family = esc(at.kind.label()),
            size = esc(&rules::format_bytes(asset.bytes)),
            dot = if when(asset.modified, &timezone).is_empty() { "" } else { " · " },
            when = esc(&when(asset.modified, &timezone)),
            link = esc(&link),
            site = esc(&site),
            copy = svg("copy"),
            open = svg("open"),
            banner = flash_banner(&q),
            preview = preview(&at, &site),
            path = esc(&at.path()),
            title = esc(&meta.title),
            max_title = rules::MAX_TITLE,
            alt = esc(&meta.alt),
            max_alt = rules::MAX_ALT,
            caption = esc(&meta.caption),
            max_caption = rules::MAX_CAPTION,
            check = svg("check"),
            name = esc(at.name()),
            max_name = rules::MAX_NAME,
            edit = svg("edit"),
            accept = esc(&format!(".{}", at.extension())),
            upload = svg("upload"),
            ext = esc(&at.extension()),
            link_row = row("link", "blue", "Link", format!(r#"<code>{}</code>"#, esc(&link))),
            path_row = row("folder", "amber", "On this server", esc(&format!("content/{}", at.path()))),
            kind_row = row(at.kind.glyph(), "purple", "Kind", esc(&format!("{} · .{}", at.kind.label(), at.extension()))),
            size_row = row("archive", "slate", "Size", esc(&rules::format_bytes(asset.bytes))),
            modified_row = row("calendar", "green", "Last modified", esc(&when(asset.modified, &timezone))),
            described_row = if meta.updated_at.is_empty() {
                String::new()
            } else {
                row(
                    "notes",
                    "teal",
                    "Details last edited",
                    esc(&ghost_core::dates::short_date_in_zone(&meta.updated_at, &timezone)),
                )
            },
            usage_card = usage_card,
            warning = if used_total == 0 {
                "Nothing points at this file, so deleting it is safe.".to_string()
            } else {
                format!(
                    "{} still {} at this file. Deleting it leaves a broken image or link there — this does not edit those posts.",
                    posts_and_pages(used_total),
                    if used_total == 1 { "points" } else { "point" },
                )
            },
            name_attr = esc(at.name()),
            used_warning = if used_total == 0 {
                String::new()
            } else {
                format!(" It is used by {}, which will be left pointing at nothing.", posts_and_pages(used_total))
            },
            del = svg("delete"),
        ),
        true,
    )
    .into_response()
}

/// "1 post or page" / "4 posts and pages".
///
/// `countable` cannot do this one: it pluralizes by adding an `s` to the word
/// it is given, which turns "post or page" into "post or pages".
fn posts_and_pages(n: i64) -> String {
    if n == 1 {
        "1 post or page".to_string()
    } else {
        format!("{n} posts and pages")
    }
}

/// The file itself, as large as the screen allows.
///
/// A picture is shown, a video and a sound get native players, and anything
/// else gets its glyph — there is nothing useful to render for a PDF without
/// embedding a document viewer, and a glyph is honest about that.
///
/// An SVG is shown in an `<img>` and never inlined. The public file route
/// already serves one under `Content-Security-Policy: sandbox` so its script
/// cannot touch this origin, and an `<img>` never runs script in the first
/// place; inlining the markup into this page would undo both.
fn preview(at: &Location, url: &str) -> String {
    if at.is_viewable_image() {
        return format!(r#"<img src="{}" alt="">"#, esc(url));
    }
    if at.is_video() {
        return format!(r#"<video src="{}" controls preload="metadata"></video>"#, esc(url));
    }
    if at.is_audio() {
        return format!(
            r#"<div class="ml-sound"><span class="ic tint-purple">{glyph}</span><audio src="{url}" controls preload="metadata"></audio></div>"#,
            glyph = svg("play"),
            url = esc(url),
        );
    }
    format!(
        r#"<div class="ml-nothing"><span class="ic tint-slate">{glyph}</span><p>{ext} file</p>
<p class="muted">There is no preview for this kind of file. Open it to see what it holds.</p></div>"#,
        glyph = svg("file"),
        ext = esc(&at.extension().to_uppercase()),
    )
}

/// Save the words beside a file.
pub(crate) async fn item_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let Some(at) = requested(&posted) else {
        return flash_to("/ghost/media/", "error", "That is not a file in this library.");
    };
    if stat(&state, &at).await.is_none() {
        return flash_to("/ghost/media/", "error", "That file is no longer in this library.");
    }
    let value = |key: &str| posted.get(key).map(String::as_str).unwrap_or("");
    let title = rules::clean_text(value("title"), false, rules::MAX_TITLE);
    let alt = rules::clean_text(value("alt"), false, rules::MAX_ALT);
    let caption = rules::clean_text(value("caption"), true, rules::MAX_CAPTION);
    match repo::save(&state.db, &at, &repo::MetaInput { title: &title, alt: &alt, caption: &caption }).await {
        Ok(()) => flash_to(&back_to(&at), "ok", "Details saved."),
        Err(e) => server_error(&e.message()),
    }
}

/// Rename a file, taking its metadata with it.
///
/// The destination is composed by [`rules::rename_target`] and is in the same
/// folder by construction, so a rename cannot move a file between families or
/// out of the content tree. A name already taken is refused rather than
/// overwritten — that would destroy whatever is there, which no rename should
/// be able to do by accident.
pub(crate) async fn item_rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let Some(at) = requested(&posted) else {
        return flash_to("/ghost/media/", "error", "That is not a file in this library.");
    };
    if stat(&state, &at).await.is_none() {
        return flash_to("/ghost/media/", "error", "That file is no longer in this library.");
    }
    let name = match rules::rename_target(at.name(), posted.get("name").map(String::as_str).unwrap_or("")) {
        Ok(name) => name,
        Err(message) => return flash_to(&back_to(&at), "error", &message),
    };
    if name == at.name() {
        return flash_to(&back_to(&at), "ok", "That is already its name.");
    }
    let to = at.with_name(&name);
    if tokio::fs::metadata(disk_path(&state, &to)).await.is_ok() {
        return flash_to(&back_to(&at), "error", &format!("There is already a file called {name} here."));
    }
    if let Err(e) = tokio::fs::rename(disk_path(&state, &at), disk_path(&state, &to)).await {
        return server_error(&format!("rename {} → {}: {e}", at.path(), to.path()));
    }
    if let Err(e) = repo::rename(&state.db, &at, &to).await {
        // The file has moved; losing its description is worth a log, not a
        // failed request that would leave the page claiming the old name.
        tracing::error!("media admin: metadata did not follow {} → {}: {}", at.path(), to.path(), e.message());
    }
    flash_to(&back_to(&to), "ok", &format!("Renamed to {name}. Anything that linked to the old name no longer finds it."))
}

/// Overwrite a file's contents, keeping its name and its link.
///
/// The replacement must map to the same extension as the file it replaces, so
/// what is served at a URL stays the kind of thing that was served there. The
/// size and the host's upload limit are checked first, as they are for a new
/// upload, and the bytes are written only after all three pass.
pub(crate) async fn item_replace(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let mut at: Option<Location> = None;
    let mut upload: Option<(Option<String>, String, Vec<u8>)> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "path" => {
                at = field.text().await.ok().as_deref().and_then(rules::parse_location);
            }
            "file" => {
                let filename = field.file_name().map(str::to_string);
                let content_type = field
                    .content_type()
                    .map(str::to_string)
                    .unwrap_or_else(|| "application/octet-stream".into());
                match field.bytes().await {
                    Ok(data) if !data.is_empty() => upload = Some((filename, content_type, data.to_vec())),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let Some(at) = at else {
        return flash_to("/ghost/media/", "error", "That is not a file in this library.");
    };
    if stat(&state, &at).await.is_none() {
        return flash_to("/ghost/media/", "error", "That file is no longer in this library.");
    }
    let Some((filename, content_type, data)) = upload else {
        return flash_to(&back_to(&at), "error", "Choose the replacement file first.");
    };
    let extension = match at.kind.extension(&content_type, filename.as_deref()) {
        Ok(extension) => extension,
        Err(message) => return flash_to(&back_to(&at), "error", &format!("That cannot replace this file — {message}.")),
    };
    if !rules::same_format(&extension, &at.extension()) {
        return flash_to(
            &back_to(&at),
            "error",
            &format!("A .{extension} cannot replace a .{}. Upload it as a new file instead.", at.extension()),
        );
    }
    if data.len() > at.kind.max_bytes() {
        return flash_to(
            &back_to(&at),
            "error",
            &format!("That file is larger than the {}MB limit.", at.kind.max_bytes() / 1024 / 1024),
        );
    }
    if let Err(e) = state
        .limits
        .error_if_is_over(&state.db, "uploads", ghost_db::limits::Check::of_size(data.len() as i64))
        .await
    {
        return flash_to(&back_to(&at), "error", &e.message());
    }
    if let Err(e) = tokio::fs::write(disk_path(&state, &at), &data).await {
        return server_error(&format!("replace {}: {e}", at.path()));
    }
    // Through the storage adapter too, so an S3-backed install replaces the
    // object a visitor actually loads and not only the local original.
    if let Err(e) = ghost_api::storage::publish_upload(&state, &at.path(), &content_type, &data, at.url()).await {
        tracing::error!("media admin: replaced {} locally but not in object storage: {}", at.path(), e.message());
        return flash_to(&back_to(&at), "error", "The file was replaced on this server, but object storage refused it — check the logs.");
    }
    flash_to(
        &back_to(&at),
        "ok",
        "File replaced. Browsers that already have the old version may show it until their cache expires.",
    )
}

pub(crate) async fn item_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let Some(at) = requested(&posted) else {
        return flash_to("/ghost/media/", "error", "That is not a file in this library.");
    };
    match remove(&state, &at).await {
        Ok(true) => flash_to("/ghost/media/", "ok", &format!("{} deleted.", shown_name(Some(at.name())))),
        Ok(false) => flash_to("/ghost/media/", "error", "That file is no longer in this library."),
        Err(e) => server_error(&e),
    }
}

/// Delete everything the selection names.
///
/// Each path is parsed on its own, so one bad entry in the list cannot take
/// the request down or make the others act on something else. The flash says
/// how many went and how many did not.
pub(crate) async fn bulk_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(posted): Form<Vec<(String, String)>>,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    let paths: Vec<Location> = posted
        .iter()
        .filter(|(key, _)| key == "paths")
        .filter_map(|(_, value)| rules::parse_location(value))
        .collect();
    if paths.is_empty() {
        return flash_to("/ghost/media/", "error", "Nothing was selected.");
    }
    let mut gone = 0usize;
    let mut failed = 0usize;
    for at in &paths {
        match remove(&state, at).await {
            Ok(true) => gone += 1,
            Ok(false) => failed += 1,
            Err(e) => {
                tracing::error!("media admin: {e}");
                failed += 1;
            }
        }
    }
    match (gone, failed) {
        (0, _) => flash_to("/ghost/media/", "error", "Nothing could be deleted — check the logs."),
        (n, 0) => flash_to("/ghost/media/", "ok", &format!("{} deleted.", countable(n as i64, "file"))),
        (n, f) => flash_to(
            "/ghost/media/",
            "error",
            &format!("{} deleted; {} could not be.", countable(n as i64, "file"), f),
        ),
    }
}

/// Take one file off the disk and forget what was written about it.
///
/// `Ok(false)` means there was no such file, which a second delete of the
/// same thing — a double-submitted form, a stale tab — produces and which is
/// not an error. The metadata row goes either way: if the file is gone, the
/// description of it should be too.
async fn remove(state: &AppState, at: &Location) -> Result<bool, String> {
    let path = disk_path(state, at);
    let existed = tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file());
    if existed {
        tokio::fs::remove_file(&path).await.map_err(|e| format!("delete {}: {e}", at.path()))?;
    }
    if let Err(e) = repo::delete(&state.db, at).await {
        tracing::error!("media admin: {} was deleted but its details were not: {}", at.path(), e.message());
    }
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str) -> Location {
        rules::parse_location(path).expect("a test path")
    }

    fn asset(path: &str, bytes: u64, modified: i64) -> Asset {
        Asset { at: at(path), bytes, modified }
    }

    fn query(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn library_filters_only_ever_carry_values_this_module_offers() {
        let folders = vec!["2026/11".to_string()];
        let f = filters(&query(&[]), &folders);
        assert!(f.kind.is_none() && f.sort == "new" && f.folder.is_empty() && f.q.is_empty());

        let f = filters(&query(&[("kind", "media"), ("sort", "big"), ("folder", "2026/11")]), &folders);
        assert_eq!(f.kind, Some(Kind::Media));
        assert_eq!((f.sort, f.folder.as_str()), ("big", "2026/11"));

        // A forged kind, a forged order and a folder the walk never found all
        // collapse to the default rather than reaching a link or a filter.
        let f = filters(
            &query(&[
                ("kind", r#""><script>"#),
                ("sort", "'; DROP TABLE posts;--"),
                ("folder", "../../etc"),
            ]),
            &folders,
        );
        assert!(f.kind.is_none());
        assert_eq!(f.sort, "new");
        assert_eq!(f.folder, "");
        assert_eq!(f.query(&[]), "");
    }

    #[test]
    fn a_filter_link_keeps_the_other_filters_and_resets_the_page() {
        let f = filters(&query(&[("kind", "images"), ("folder", "2026/11"), ("q", "cover")]), &["2026/11".into()]);
        let href = f.href(&[("kind", "files"), ("page", "1")]);
        assert!(href.starts_with("/ghost/media/?"), "{href}");
        for part in ["kind=files", "folder=2026%2F11", "q=cover"] {
            assert!(href.contains(part), "{href} is missing {part}");
        }
        assert!(!href.contains("page="), "a new filter starts at page one: {href}");
        // and "all" is the absence of the parameter, not a value
        assert_eq!(f.href(&[("kind", "all"), ("folder", ""), ("sort", "new")]), "/ghost/media/?q=cover");
        // Clear empties every filter in the toolbar, search included, and
        // leaves the family tab where it was.
        assert_eq!(
            f.href(&[("q", ""), ("folder", ""), ("sort", "new"), ("page", "1")]),
            "/ghost/media/?kind=images"
        );
    }

    #[test]
    fn a_search_looks_at_the_words_written_about_a_file_as_well_as_its_name() {
        let photo = asset("images/2026/11/dsc-0041-ab12.jpg", 1000, 100);
        let described = repo::Meta { caption: "Sunrise over the harbour".into(), ..Default::default() };
        let hunt = |needle: &str| filters(&query(&[("q", needle)]), &[]);

        assert!(hunt("dsc").matches(&photo, None));
        assert!(hunt("HARBOUR").matches(&photo, Some(&described)));
        assert!(!hunt("harbour").matches(&photo, None));
        // the family and the month narrow it independently of the words
        assert!(!filters(&query(&[("kind", "files")]), &[]).matches(&photo, None));
        assert!(filters(&query(&[("folder", "2026/11")]), &["2026/11".into()]).matches(&photo, None));
        assert!(!filters(&query(&[("folder", "2026/10")]), &["2026/10".into()]).matches(&photo, None));
    }

    #[test]
    fn a_tile_escapes_the_name_and_the_title_it_shows() {
        let hostile = asset(r#"images/2026/11/a-b.jpg"#, 2048, 1_760_000_000);
        let meta = repo::Meta {
            title: r#"<img src=x onerror=alert(1)>"#.into(),
            ..Default::default()
        };
        let html = tile(&hostile, Some(&meta), "Etc/UTC", 0);
        assert!(!html.contains("<img src=x"), "{html}");
        assert!(html.contains("&lt;img src=x"), "{html}");
        // the tile links to the item screen with the path as a parameter, and
        // offers the public URL to copy
        assert!(html.contains("/ghost/media/item/?path=images%2F2026%2F11%2Fa-b.jpg"), "{html}");
        assert!(html.contains(r#"data-copy="/content/images/2026/11/a-b.jpg""#), "{html}");
        // an image shows itself; the checkbox joins the batch form
        assert!(html.contains(r#"<img src="/content/images/2026/11/a-b.jpg" alt="" loading="lazy""#), "{html}");
        assert!(html.contains(r#"form="batch-media" name="paths" value="images/2026/11/a-b.jpg""#), "{html}");
        assert!(html.contains("2 KB"), "{html}");
    }

    /// The base path is applied in exactly one place for each kind of slot,
    /// and this is the regression guard for getting that wrong.
    ///
    /// `src` is rewritten by `apply_base_path` on the way out, so the tile
    /// must emit it in site terms — prefixing it here as well would produce
    /// `/blog/blog/content/…`. `data-copy` is not a URL attribute and that
    /// middleware never sees it, so it has to carry the prefix already.
    #[test]
    fn a_tile_leaves_the_base_path_to_the_one_place_that_applies_it() {
        let html = tile(&asset("images/2026/11/cover-ab12.jpg", 1024, 1), None, "Etc/UTC", 0);
        assert!(html.contains(r#"src="/content/images/2026/11/cover-ab12.jpg""#), "{html}");
        let rewritten = ghost_core::url::rewrite_html_with("/blog", &html);
        assert!(rewritten.contains(r#"src="/blog/content/images/2026/11/cover-ab12.jpg""#), "{rewritten}");
        assert!(!rewritten.contains("/blog/blog/"), "the base path was applied twice: {rewritten}");
        // …and the middleware is not what puts it on the copied link, which is
        // why `tile` passes that one through `url::base` itself.
        assert!(rewritten.contains(r#"data-copy="/content/images/2026/11/cover-ab12.jpg""#), "{rewritten}");
    }

    #[test]
    fn a_file_with_no_preview_shows_its_family_rather_than_an_empty_frame() {
        let pdf = asset("files/2026/11/report-ab12.pdf", 4096, 0);
        let html = tile(&pdf, None, "Etc/UTC", 0);
        assert!(html.contains("ml-shot-glyph") && html.contains(">PDF<"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        // …and on its own screen, says so in words
        let full = preview(&pdf.at, "/content/files/2026/11/report-ab12.pdf");
        assert!(full.contains("no preview") && full.contains("PDF file"), "{full}");
        // video and audio get players, an image gets an <img>
        assert!(preview(&at("media/clip.mp4"), "/x.mp4").contains("<video"));
        assert!(preview(&at("media/song.mp3"), "/x.mp3").contains("<audio"));
        assert!(preview(&at("images/a.png"), "/x.png").starts_with("<img"));
        // an SVG is referenced, never inlined
        let svg_preview = preview(&at("images/logo.svg"), "/content/images/logo.svg");
        assert!(svg_preview.starts_with("<img") && !svg_preview.contains("<svg"), "{svg_preview}");
    }

    #[test]
    fn the_pager_only_appears_when_there_is_more_than_one_page() {
        let f = filters(&query(&[("kind", "images")]), &[]);
        assert_eq!(pager(&f, 1, 1), "");
        let two = pager(&f, 1, 2);
        assert!(two.contains("Page 1 of 2") && two.contains("Older") && !two.contains("Newer"), "{two}");
        assert!(two.contains("kind=images&amp;page=2"), "{two}");
        let last = pager(&f, 2, 2);
        assert!(last.contains("Newer") && !last.contains("Older"), "{last}");
    }

    #[test]
    fn a_name_put_in_a_sentence_is_bounded_and_stripped_of_its_directories() {
        assert_eq!(shown_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(shown_name(Some("")), "That file");
        assert_eq!(shown_name(None), "That file");
        assert_eq!(shown_name(Some("a\nb.jpg")), "a b.jpg");
        assert!(shown_name(Some(&"x".repeat(500))).chars().count() <= 80);
    }

    #[test]
    fn counts_of_posts_and_pages_agree_with_their_verb() {
        assert_eq!(posts_and_pages(1), "1 post or page");
        assert_eq!(posts_and_pages(0), "0 posts and pages");
        assert_eq!(posts_and_pages(4), "4 posts and pages");
        // the shared helper is the reason this one exists
        assert_eq!(countable(4, "post or page"), "4 post or pages");
    }

    #[test]
    fn the_summary_counts_the_whole_library_not_the_filtered_view() {
        let empty = Library { assets: vec![], capped: false };
        assert!(summary(&empty, 0).contains("Images, video, audio"));
        let two = Library {
            assets: vec![asset("images/a.jpg", 1024, 1), asset("files/b.pdf", 2_097_152, 2)],
            capped: false,
        };
        assert_eq!(summary(&two, 1024 + 2_097_152), "2 files · 2.0 MB on disk");
    }
}
