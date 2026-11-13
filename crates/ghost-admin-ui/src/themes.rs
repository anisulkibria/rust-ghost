//! Settings -> Themes: the gallery, uploads, the file editor, activation and deletion.

use super::*;

// ---------------------------------------------------------------------------
// themes
// ---------------------------------------------------------------------------

/// Redirect back to the themes list carrying a one-shot banner.
pub(crate) fn themes_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/themes/?{encoded}")).into_response()
}

/// GScan's verdict on a theme, as a collapsible summary for its row.
///
/// Empty for a theme with nothing wrong, which is the common case and should
/// not cost a line. Fatal findings are named first and separately: they are
/// the ones that stop the theme being activated at all.
pub(crate) fn theme_validation_summary(theme_dir: &std::path::Path) -> String {
    let report = ghost_render::gscan::scan(theme_dir);
    if report.findings.is_empty() {
        return String::new();
    }
    let count = |n: usize, one: &str, many: &str| match n {
        0 => String::new(),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    let parts: Vec<String> = [
        count(report.errors().count(), "error", "errors"),
        count(report.warnings().count(), "warning", "warnings"),
        count(report.recommendations().count(), "recommendation", "recommendations"),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect();

    let mut rows = String::new();
    for finding in &report.findings {
        rows.push_str(&format!(
            r#"<li><code>{code}</code> <strong>{rule}</strong>{fatal}<br><span class="muted">{details}</span>{where_}</li>"#,
            code = esc(finding.code),
            rule = esc(&finding.rule),
            fatal = if finding.fatal { r#" <span class="chip draft">blocks activation</span>"# } else { "" },
            details = esc(&finding.details),
            where_ = if finding.failures.is_empty() {
                String::new()
            } else {
                // Enough to find it, not so many that the row becomes a log.
                let shown: Vec<String> = finding.failures.iter().take(5).map(|f| esc(f)).collect();
                let more = finding.failures.len().saturating_sub(shown.len());
                format!(
                    r#"<br><span class="muted"><code>{}</code>{}</span>"#,
                    shown.join("</code>, <code>"),
                    if more > 0 { format!(" and {more} more") } else { String::new() },
                )
            },
        ));
    }
    format!(
        r#"<details class="detail" style="margin-top:4px"><summary>Theme validation · {summary}</summary>
<ul style="margin:8px 0 0;padding-left:18px;line-height:1.6">{rows}</ul></details>"#,
        summary = esc(&parts.join(", ")),
        rows = rows,
    )
}

pub(crate) async fn themes_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let banner = match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    };

    let installed = ghost_api::themes::list(&state);
    let gallery = official_theme_gallery(&installed);
    let premium = premium_theme_gallery(&installed);

    let rows: String = installed
        .iter()
        .map(|t| {
            let meta = [
                t.version.as_ref().map(|v| format!("v{}", esc(v))),
                t.description.as_ref().filter(|d| !d.is_empty()).map(|d| esc(d)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");

            let status = if t.active {
                r#"<span class="chip published">Active</span>"#.to_string()
            } else if let Some(e) = &t.error {
                format!(r#"<span class="chip error" title="{}">Invalid</span>"#, esc(e))
            } else {
                String::new()
            };

            // The active theme's "edit" is its settings, which live on the
            // Design page — there is nowhere else to customise it from.
            let customise = if t.active {
                r#"<a class="btn tonal small" href="/ghost/design/">Customize</a>"#.to_string()
            } else {
                String::new()
            };
            let activate = if t.active || t.error.is_some() {
                String::new()
            } else {
                format!(
                    r#"<form method="post" action="/ghost/themes/{name}/activate/"><button class="btn tonal small" type="submit">Activate</button></form>"#,
                    name = esc(&t.name)
                )
            };
            let edit = format!(
                r#"<a class="btn tonal small" href="/ghost/themes/{name}/edit/">Edit code</a>"#,
                name = esc(&t.name),
            );
            let delete = if t.active {
                String::new()
            } else {
                format!(
                    r#"<form method="post" action="/ghost/themes/{name}/delete/" data-confirm="Delete theme|The {name} theme will be removed."><button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>"#,
                    name = esc(&t.name),
                    del = svg("delete"),
                )
            };

            // What GScan makes of it. Folded away unless there is something
            // to say, so a clean theme stays a one-line row.
            let validation = theme_validation_summary(&state.config.themes_dir().join(&t.name));

            format!(
                r#"<div class="row" data-filterable>
<span class="ic tint-purple">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{meta}</span>{validation}</div>
<div class="trailing">{status}{customise}{edit}{activate}{delete}</div>
</div>"#,
                ic = svg("themes"),
                name = esc(&t.name),
                meta = meta,
                validation = validation,
                status = status,
                customise = customise,
                activate = activate,
                edit = edit,
                delete = delete,
            )
        })
        .collect();

    page_for_role(actor_role, 
        "Themes",
        "themes",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Themes</h1><p class="sub">Design of the public site</p></div></header>
{banner}
<div class="section">Install</div>
<div class="card padded rise" style="--i:2"><form method="post" action="/ghost/themes/upload/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<input type="file" name="file" accept=".zip,application/zip" required class="filefield">
<button class="btn tonal" type="submit">{up} Upload theme</button>
</form><p class="muted" style="margin:10px 0 0">A Ghost theme zip. Uploading a theme that is already installed replaces it.</p></div>
<div class="section">Installed</div>
<div class="card rows rise" style="--i:3">{rows}</div>
<div class="section big">Premium themes</div>
<p class="muted" style="margin:-6px 0 14px">Hand-crafted themes from <a href="https://ghost-theme.com/themes/" target="_blank" rel="noopener noreferrer">ghost-theme.com</a>. Get the zip from the store, then upload it above.</p>
<div class="gallery premium rise" style="--i:4">{premium}</div>
<script src="https://assets.lemonsqueezy.com/lemon.js" defer></script>
<div class="section big">More free themes</div>
<p class="muted" style="margin:-6px 0 14px">Free themes built by Ghost. Installing downloads the latest version from GitHub; it does not change the active theme.</p>
<div class="gallery compact rise" style="--i:5">{gallery}</div>
{preview_dialog}"#,
            banner = banner,
            up = svg("upload"),
            rows = rows,
            premium = premium,
            gallery = gallery,
            preview_dialog = theme_preview_dialog()
        ),
        true,
    )
    .into_response()
}

/// The modal a gallery card's Preview opens: the theme's live demo in a
/// frame, with a width toggle, a way out to a real tab, and the card's own
/// button (Install, Activate, Get theme) repeated so a decision made while
/// looking at the demo does not mean closing it and finding the card again.
///
/// One dialog serves every card. The links that open it are ordinary
/// `target="_blank"` links carrying `data-theme-preview`, so with the script
/// absent — or on a modified click — they open the demo in a tab as before.
pub(crate) fn theme_preview_dialog() -> String {
    format!(
        r#"<dialog class="theme-preview" id="theme-preview" data-device="desktop" aria-labelledby="tp-name">
<div class="tp-bar">
<h2 class="tp-name" id="tp-name"></h2>
<div class="tp-devices" role="group" aria-label="Preview width">
<button type="button" data-device="desktop" aria-pressed="true">Desktop</button>
<button type="button" data-device="tablet" aria-pressed="false">Tablet</button>
<button type="button" data-device="mobile" aria-pressed="false">Mobile</button>
</div>
<button type="button" class="btn small tp-action" hidden></button>
<a class="icon-btn tp-open" href="about:blank" target="_blank" rel="noopener noreferrer" title="Open in a new tab" aria-label="Open in a new tab">{open}</a>
<button type="button" class="icon-btn tp-close" title="Close" aria-label="Close preview">{close}</button>
</div>
<div class="tp-stage"><iframe title="Theme demo" src="about:blank" referrerpolicy="no-referrer" sandbox="allow-scripts allow-same-origin allow-forms allow-popups"></iframe></div>
</dialog>
<script>{js}</script>"#,
        open = svg("open"),
        close = svg("close"),
        js = THEME_PREVIEW_JS,
    )
}

pub(crate) const THEME_PREVIEW_JS: &str = r#"
(function () {
  var dlg = document.getElementById('theme-preview');
  if (!dlg || !dlg.showModal) return;
  var frame = dlg.querySelector('iframe');
  var nameEl = dlg.querySelector('.tp-name');
  var openEl = dlg.querySelector('.tp-open');
  var actionEl = dlg.querySelector('.tp-action');
  // The card button the toolbar's action stands in for.
  var original = null;

  document.addEventListener('click', function (e) {
    var link = e.target.closest && e.target.closest('a[data-theme-preview]');
    if (!link) return;
    // A modified or middle click is someone asking for a tab; let them.
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();

    nameEl.textContent = link.getAttribute('data-theme-preview');
    var kind = link.getAttribute('data-theme-kind');
    if (kind) {
      var small = document.createElement('small');
      small.textContent = kind;
      nameEl.appendChild(small);
    }
    openEl.href = link.href;

    // Repeat the card's own button, whatever it currently is. An active
    // theme has a chip there instead, and so nothing to repeat.
    var card = link.closest('.card');
    original = card && card.querySelector('.actions .btn:not([data-theme-preview])');
    actionEl.hidden = !original;
    if (original) actionEl.textContent = original.textContent;

    frame.classList.remove('ready');
    frame.src = link.href;
    dlg.showModal();
  });

  frame.addEventListener('load', function () {
    if (frame.getAttribute('src') !== 'about:blank') frame.classList.add('ready');
  });

  dlg.querySelector('.tp-close').addEventListener('click', function () { dlg.close(); });
  // A click that lands on the dialog itself is on its backdrop: the toolbar
  // and the stage cover every pixel inside it.
  dlg.addEventListener('click', function (e) { if (e.target === dlg) dlg.close(); });
  // Unload the demo on the way out, so it stops playing, polling and holding
  // memory behind a closed dialog.
  dlg.addEventListener('close', function () {
    frame.classList.remove('ready');
    frame.src = 'about:blank';
  });

  // Close first, then press the card's button: the checkout overlay and the
  // confirm dialogs would otherwise open underneath a modal dialog.
  actionEl.addEventListener('click', function () {
    var target = original;
    dlg.close();
    if (target) target.click();
  });

  dlg.querySelector('.tp-devices').addEventListener('click', function (e) {
    var btn = e.target.closest('button[data-device]');
    if (!btn) return;
    dlg.setAttribute('data-device', btn.getAttribute('data-device'));
    Array.prototype.forEach.call(this.querySelectorAll('button'), function (b) {
      b.setAttribute('aria-pressed', b === btn ? 'true' : 'false');
    });
  });
})();
"#;

/// The ghost-theme.com collection, as large cards: the store's screenshot
/// with the price on it, the pitch, and a way to the demo and the store.
///
/// These cannot be installed from here — the zip is bought (or, for the free
/// ones, downloaded) on the store and comes in through the upload form — so
/// the main button opens the store's checkout for that theme as an overlay
/// (Lemon Squeezy's `lemon.js`, loaded by the page, picks up the
/// `lemonsqueezy-button` class) until the theme is found installed,
/// after which it activates it.
pub(crate) fn premium_theme_gallery(installed: &[ghost_api::themes::InstalledTheme]) -> String {
    ghost_api::themes::PREMIUM_THEMES
        .iter()
        .map(|t| {
            let action = match installed.iter().find(|i| i.name == t.slug) {
                Some(i) if i.active => r#"<span class="chip published" style="flex:1;justify-content:center">Active</span>"#.to_string(),
                Some(_) => format!(
                    r#"<form method="post" action="/ghost/themes/{slug}/activate/"><button class="btn tonal small" type="submit">Activate</button></form>"#,
                    slug = esc(t.slug)
                ),
                None => format!(
                    r#"<a class="btn small lemonsqueezy-button" href="{store}" target="_blank" rel="noopener noreferrer">{label}</a>"#,
                    store = esc(&t.checkout_url()),
                    label = if t.is_free() { "Get free" } else { "Get theme" },
                ),
            };

            format!(
                r#"<div class="card rise" data-filterable>
<a class="shot" href="{preview}" target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" data-theme-preview="{name}" data-theme-kind="{category}"><img src="{thumb}" alt="" width="800" height="600" loading="lazy" decoding="async" referrerpolicy="no-referrer" onerror="this.remove()"><span class="price{free}">{price}</span></a>
<div class="meta"><div class="body"><span class="title">{name}</span><span class="detail">{category} · {tagline}</span></div></div>
<div class="actions"><a class="btn outlined small" href="{preview}" target="_blank" rel="noopener noreferrer" data-theme-preview="{name}" data-theme-kind="{category}">Preview</a>{action}</div>
</div>"#,
                preview = esc(t.preview_url),
                thumb = esc(&t.thumbnail_url()),
                free = if t.is_free() { " free" } else { "" },
                price = esc(t.price),
                name = esc(t.name),
                category = esc(t.category),
                tagline = esc(t.tagline),
                action = action,
            )
        })
        .collect()
}

/// Ghost's official themes, as small cards: the marketplace screenshot, the
/// name with a Free tag, and one button whose job depends on whether the
/// theme is already here.
///
/// `installed` is the themes already under `content/themes/`, so an entry
/// that is installed offers Activate (or reads Active) instead of downloading
/// it a second time.
pub(crate) fn official_theme_gallery(installed: &[ghost_api::themes::InstalledTheme]) -> String {
    ghost_api::themes::OFFICIAL_THEMES
        .iter()
        .map(|t| {
            let local = installed.iter().find(|i| i.name == t.slug);

            let action = match local {
                Some(i) if i.active => r#"<span class="chip published" style="flex:1;justify-content:center">Active</span>"#.to_string(),
                Some(_) => format!(
                    r#"<form method="post" action="/ghost/themes/{slug}/activate/"><button class="btn tonal small" type="submit">Activate</button></form>"#,
                    slug = esc(t.slug)
                ),
                None => format!(
                    r#"<form method="post" action="/ghost/themes/official/{slug}/install/"><button class="btn tonal small" type="submit">Install</button></form>"#,
                    slug = esc(t.slug)
                ),
            };

            format!(
                r#"<div class="card rise" data-filterable>
<a class="shot" href="{preview}" target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" data-theme-preview="{name}" data-theme-kind="{category}"><img src="/ghost/themes/official/{slug}/thumbnail.png" alt="" width="600" height="750" loading="lazy" decoding="async" onerror="this.remove()"></a>
<div class="meta"><div class="body"><span class="title">{name}<span class="chip free">Free</span></span><span class="detail">{category}</span></div></div>
<div class="actions"><a class="btn outlined small" href="{preview}" target="_blank" rel="noopener noreferrer" data-theme-preview="{name}" data-theme-kind="{category}">Preview</a>{action}</div>
</div>"#,
                slug = esc(t.slug),
                preview = esc(t.preview_url),
                name = esc(t.name),
                category = esc(t.category),
                action = action,
            )
        })
        .collect()
}

/// Turn a multipart error into something an operator can act on.
///
/// The over-sized case is worth naming: axum reports it as a generic
/// "Error parsing `multipart/form-data` request", which gives no hint that a
/// size ceiling was hit. The status is the reliable signal — the message text
/// is not.
pub(crate) fn upload_error_message(err: &axum::extract::multipart::MultipartError) -> String {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        format!(
            "That file is larger than the {} MB upload limit.",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )
    } else {
        format!("Upload failed: {err}")
    }
}

pub(crate) async fn theme_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    loop {
        // `while let Ok(Some(_))` would treat a transport error as "no more
        // fields" and report the misleading "no file uploaded" — which is
        // how an over-sized body used to fail silently.
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return themes_flash("error", "No theme file was uploaded."),
            Err(e) => return themes_flash("error", &upload_error_message(&e)),
        };
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("theme.zip").to_string();
        let data = match field.bytes().await {
            Ok(d) => d.to_vec(),
            Err(e) => return themes_flash("error", &upload_error_message(&e)),
        };
        return match ghost_api::themes::install_zip(&state, &filename, data).await {
            Ok(name) => themes_flash("ok", &format!("Theme '{name}' uploaded.")),
            Err(e) => themes_flash("error", &e.message()),
        };
    }
}

/// A gallery thumbnail, served with a week-long cache window.
///
/// The bytes come from Ghost's marketplace and are kept under `content/cache/`
/// after the first request, so the themes page costs one round trip per theme
/// ever rather than one per visit. The artwork for a published theme is
/// fixed, which is what makes `immutable` honest here.
pub(crate) async fn theme_official_thumbnail(State(state): State<AppState>, Path(slug): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::official_thumbnail(&state, &slug).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png".to_string()),
                (
                    header::CACHE_CONTROL,
                    format!(
                        "public, max-age={}, immutable",
                        ghost_api::themes::THUMBNAIL_MAX_AGE_SECONDS
                    ),
                ),
            ],
            bytes,
        )
            .into_response(),
        // The gallery drops an image it cannot load, so a miss here just
        // leaves the card's placeholder panel.
        Err(e) => (StatusCode::NOT_FOUND, e.message()).into_response(),
    }
}

/// One-click install of an official theme: download the archive from GitHub
/// and unpack it like an upload. The active theme is left alone, so an
/// operator can install, preview and then activate deliberately.
pub(crate) async fn theme_install_official(State(state): State<AppState>, Path(slug): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::install_official(&state, &slug).await {
        Ok(name) => themes_flash("ok", &format!("Theme '{name}' installed. Activate it when you're ready.")),
        Err(e) => themes_flash("error", &format!("Could not install that theme: {}", e.message())),
    }
}

// --- theme code editor -------------------------------------------------------

pub(crate) fn theme_edit_flash(theme: &str, file: &str, kind: &str, message: &str) -> Response {
    let mut qs = form_urlencoded::Serializer::new(String::new());
    if !file.is_empty() {
        qs.append_pair("file", file);
    }
    qs.append_pair(kind, message);
    Redirect::to(&format!("/ghost/themes/{theme}/edit/?{}", qs.finish())).into_response()
}

/// A theme's files as a tree, the way an editor shows them.
///
/// Built from the flat relative paths rather than by walking the directory
/// again, so the tree can only ever contain files the editor would list.
#[derive(Default)]
pub(crate) struct FileTree {
    /// Sub-directories by name, in path order.
    pub(crate) dirs: Vec<(String, FileTree)>,
    /// `(relative path, leaf name)` for the files directly in this directory.
    pub(crate) files: Vec<(String, String)>,
}

impl FileTree {
    fn insert(&mut self, rel: &str, rest: &str) {
        match rest.split_once('/') {
            Some((head, tail)) => {
                // Linear scan rather than a map: a theme has a handful of
                // directories, and this preserves the sorted input order.
                if let Some(i) = self.dirs.iter().position(|(n, _)| n == head) {
                    self.dirs[i].1.insert(rel, tail);
                } else {
                    let mut child = FileTree::default();
                    child.insert(rel, tail);
                    self.dirs.push((head.to_string(), child));
                }
            }
            None => self.files.push((rel.to_string(), rest.to_string())),
        }
    }

    pub(crate) fn build(paths: &[String]) -> Self {
        let mut root = FileTree::default();
        for p in paths {
            root.insert(p, p);
        }
        root
    }
}

/// Render the tree as nested `<details>` elements.
///
/// `<details>` gives the open/close behaviour, the disclosure state and
/// keyboard operation for free, so the explorer needs no script of its own —
/// which also means it still works on the first paint, before the page's
/// JavaScript has run. Directories on the path to the open file start
/// expanded; everything else starts closed, so a theme with a deep `assets`
/// tree does not bury the templates under it.
pub(crate) fn render_file_tree(tree: &FileTree, theme: &str, open: &str, prefix: &str, depth: usize) -> String {
    let mut out = String::new();
    for (dir_name, child) in &tree.dirs {
        let path = if prefix.is_empty() {
            dir_name.clone()
        } else {
            format!("{prefix}/{dir_name}")
        };
        let holds_open = open.starts_with(&format!("{path}/"));
        out.push_str(&format!(
            r#"<details class="fdir"{open_attr}><summary style="--d:{depth}"><span class="tw">{chev}</span><span class="fic">{folder}</span>{name}</summary>{kids}</details>"#,
            open_attr = if holds_open { " open" } else { "" },
            depth = depth,
            chev = svg("chevron"),
            folder = svg("folder"),
            name = esc(dir_name),
            kids = render_file_tree(child, theme, open, &path, depth + 1),
        ));
    }
    for (rel, leaf) in &tree.files {
        let editable = ghost_api::themes::is_editable(rel);
        out.push_str(&format!(
            r#"<a class="frow{on}" style="--d:{depth}" href="/ghost/themes/{theme}/edit/?file={enc}" title="{full}"><span class="fic">{ic}</span><span class="fname">{leaf}</span>{chip}</a>"#,
            on = if rel == open { " active" } else { "" },
            depth = depth,
            theme = esc(theme),
            enc = esc(&form_urlencoded::byte_serialize(rel.as_bytes()).collect::<String>()),
            full = esc(rel),
            ic = svg(if editable { "file" } else { "image" }),
            leaf = esc(leaf),
            chip = if editable { "" } else { r#"<span class="chip draft">binary</span>"# },
        ));
    }
    out
}

/// Ghost's theme code editor: the theme's files on the left, the open one in
/// a textarea on the right.
pub(crate) async fn theme_edit_page(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let files = ghost_api::themes::theme_files(&state, &name);
    if files.is_empty() {
        return not_found_page_for(actor_role, &user, "That theme could not be read.");
    }
    // Default to the file a publisher most likely came to change.
    let open = q
        .get("file")
        .cloned()
        .filter(|f| files.contains(f))
        .or_else(|| files.iter().find(|f| f.as_str() == "index.hbs").cloned())
        .or_else(|| files.iter().find(|f| ghost_api::themes::is_editable(f)).cloned())
        .unwrap_or_default();

    let editable = ghost_api::themes::is_editable(&open);
    let contents = if editable {
        ghost_api::themes::theme_file_path(&state, &name, &open)
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let tree = render_file_tree(&FileTree::build(&files), &name, &open, "", 0);

    let editor = if editable {
        // Two sibling forms, never nested. A `<form>` inside a `<form>` is
        // dropped by the parser while its children are kept, which put the
        // delete form's hidden `file` input into the save form — the save
        // then posted `file` twice ("duplicate field `file`") and the Delete
        // button submitted the save form instead of deleting. The save
        // controls reach their form by id, so the markup can stay flat.
        format!(
            r#"<form method="post" action="/ghost/themes/{theme}/edit/" id="theme-file-form">
<input type="hidden" name="file" value="{file}">
<div class="ced" data-lang="{lang}">
<div class="ced-bar"><span>{file}</span><span class="lang">{lang_label}</span></div>
<div class="ced-body">
<div class="ced-gut" aria-hidden="true"></div>
<div class="ced-wrap">
<pre class="ced-layer ced-hl" aria-hidden="true"></pre>
<textarea class="ced-layer ced-in" name="contents" spellcheck="false" autocapitalize="off" autocorrect="off" autocomplete="off" aria-label="{file} contents">{contents}</textarea>
</div></div></div>
</form>
<div style="display:flex;gap:10px;margin-top:12px;align-items:center;flex-wrap:wrap">
<button class="btn" type="submit" form="theme-file-form">{check} Save file</button>
<form method="post" action="/ghost/themes/{theme}/edit/delete/" data-confirm="Delete file|&#8220;{file}&#8221; will be removed from the theme.">
<input type="hidden" name="file" value="{file}">
<button class="btn tonal small danger" type="submit">Delete file</button></form>
</div>
<p class="muted" style="margin:12px 0 0">Saving reloads the theme immediately. A template that fails to parse will be reported here rather than breaking the site. Tab indents, Shift+Tab outdents, Ctrl/⌘+S saves.</p>"#,
            theme = esc(&name),
            file = esc(&open),
            contents = esc(&contents),
            check = svg("check"),
            lang = ghost_api::themes::editor_language(&open),
            lang_label = ghost_api::themes::editor_language(&open),
        )
    } else {
        format!(
            r#"<div class="card padded"><p class="muted" style="margin:0">{file} is not a text file, so it cannot be edited here. Replace it by uploading a new version of the theme.</p></div>"#,
            file = esc(&open),
        )
    };

    page_for_role(actor_role,
        "Edit theme",
        "themes",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/themes/" title="Back to themes">{back}</a>
<div><h1>{name}</h1><p class="sub">Editing theme files</p></div></header>
{banner}
<div style="display:grid;grid-template-columns:minmax(220px,300px) 1fr;gap:18px;align-items:start">
<div>
<div class="section" style="margin-top:0">Files</div>
<div class="card rise ftree" style="--i:1">{tree}</div>
<div class="card padded rise" style="--i:2">
<form method="post" action="/ghost/themes/{theme}/edit/new/" class="inline-form">
<label class="field" style="flex:1;min-width:140px;margin:0"><input name="file" placeholder="partials/thing.hbs" required><span>New file</span></label>
<button class="btn tonal small" type="submit">Create</button>
</form></div>
</div>
<div><div class="section" style="margin-top:0">{open_label}</div>{editor}</div>
</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            name = esc(&name),
            theme = esc(&name),
            tree = tree,
            open_label = esc(if open.is_empty() { "No file open" } else { &open }),
            editor = editor,
        ),
        true,
    )
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct ThemeFileForm {
    file: String,
    contents: String,
}

pub(crate) async fn theme_file_save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Form(form): Form<ThemeFileForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if !ghost_api::themes::is_editable(&form.file) {
        return theme_edit_flash(&name, "", "error", "That file cannot be edited here.");
    }
    let path = match ghost_api::themes::theme_file_path(&state, &name, &form.file) {
        Ok(p) => p,
        Err(e) => return theme_edit_flash(&name, "", "error", &e.message()),
    };
    // Browsers submit CRLF in a textarea; a theme file with those line endings
    // still works, but it would rewrite the whole file on first save.
    let contents = form.contents.replace("\r\n", "\n");
    if let Err(e) = std::fs::write(&path, contents) {
        return theme_edit_flash(&name, &form.file, "error", &format!("Could not write it: {e}"));
    }
    match reload_if_active(&state, &name).await {
        Some(err) => theme_edit_flash(
            &name,
            &form.file,
            "error",
            &format!("Saved, but the theme would not reload: {err}"),
        ),
        None => theme_edit_flash(&name, &form.file, "ok", "File saved."),
    }
}

/// Recompile the theme if it is the one being served, reporting why not.
///
/// The active theme is compiled in memory, so a file written to disk does
/// nothing until it is reloaded — and that applies to creating and deleting a
/// template just as much as to editing one. Returns the failure, if the
/// reload found the theme in a state it cannot render.
pub(crate) async fn reload_if_active(state: &AppState, name: &str) -> Option<String> {
    if ghost_api::themes::active_theme_name(state) != name {
        return None;
    }
    ghost_api::themes::activate(state, name).await.err().map(|e| e.message())
}

pub(crate) async fn theme_file_new(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Form(form): Form<ThemeFileForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let rel = form.file.trim().trim_start_matches('/').to_string();
    if !ghost_api::themes::is_editable(&rel) {
        return theme_edit_flash(
            &name,
            "",
            "error",
            "Give the file an editable extension, such as .hbs or .css.",
        );
    }
    let path = match ghost_api::themes::theme_file_path(&state, &name, &rel) {
        Ok(p) => p,
        Err(e) => return theme_edit_flash(&name, "", "error", &e.message()),
    };
    if path.exists() {
        return theme_edit_flash(&name, &rel, "error", "That file already exists.");
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, "") {
        return theme_edit_flash(&name, "", "error", &format!("Could not create it: {e}"));
    }
    // A new partial is not loadable until the theme is recompiled, so a
    // template referencing it would 500 in the window before the next save.
    match reload_if_active(&state, &name).await {
        Some(err) => theme_edit_flash(
            &name,
            &rel,
            "error",
            &format!("Created, but the theme would not reload: {err}"),
        ),
        None => theme_edit_flash(&name, &rel, "ok", "File created."),
    }
}

pub(crate) async fn theme_file_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Form(form): Form<ThemeFileForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let path = match ghost_api::themes::theme_file_path(&state, &name, &form.file) {
        Ok(p) => p,
        Err(e) => return theme_edit_flash(&name, "", "error", &e.message()),
    };
    if let Err(e) = std::fs::remove_file(&path) {
        return theme_edit_flash(&name, &form.file, "error", &format!("Could not delete it: {e}"));
    }
    // Deleting a template the theme still references is the one case where the
    // reload is load-bearing: saying so here is what tells the publisher to
    // put it back, rather than letting the site 500 on the next request.
    match reload_if_active(&state, &name).await {
        Some(err) => theme_edit_flash(
            &name,
            "",
            "error",
            &format!("Deleted, but the theme would not reload: {err}"),
        ),
        None => theme_edit_flash(&name, "", "ok", "File deleted."),
    }
}

pub(crate) async fn theme_activate(State(state): State<AppState>, Path(name): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::activate(&state, &name).await {
        Ok(()) => {
            // A new active theme re-renders every page on the site.
            fire_site_changed(&state);
            themes_flash("ok", &format!("Theme '{name}' is now active."))
        }
        Err(e) => themes_flash("error", &e.message()),
    }
}

pub(crate) async fn theme_delete(State(state): State<AppState>, Path(name): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::delete(&state, &name).await {
        Ok(()) => themes_flash("ok", &format!("Theme '{name}' deleted.")),
        Err(e) => themes_flash("error", &e.message()),
    }
}
