//! Tools → Page Builder & Visual Editor.
//!
//! The rules and the renderer are [`vuuut_core::builder`]; the queries
//! [`vuuut_db::repos::builder`]; the public site swaps a layout in for the
//! post body in `vuuut_front::routes::post_json`. What is here:
//!
//! * **The tool** (`/ghost/builder/`, Administrator by default, grantable
//!   through Permissions like every tool) — Overview of every page that has a
//!   layout, with on/off and remove; saved Templates; Settings (who may use
//!   it, which posts, the global colours, widths and type).
//! * **The visual editor** (`/ghost/posts/{id}/builder/`) — reached from the
//!   post editor's *Page builder* button. It sits under `/posts/` on purpose:
//!   anyone who may edit a post may design it (subject to the *Who can use
//!   it* setting), without also needing the tool's admin screens.
//!   Full-screen, its own shell; driven by `assets/builder-editor.js`.
//!
//! ## The security contract
//!
//! Every handler guards first; every write is origin-checked. A layout is
//! rebuilt by `normalise` before it is stored or rendered. Elementor source
//! metadata is kept separately for validated styling and JSON export. Editor endpoints take the body as a
//! string and parse it after the guard, so a stranger gets the sign-in
//! redirect rather than an extractor's 4xx.

use super::*;
use vuuut_core::builder::{self as rules, elementor, Settings, Who};
use vuuut_db::repos::builder as repo;

const BASE: &str = "/ghost/builder/";
const TEMPLATES: &str = "/ghost/builder/templates/";
const SETTINGS: &str = "/ghost/builder/settings/";
const THEME: &str = "/ghost/builder/theme/";

pub(crate) fn builder_settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

async fn guard(state: &AppState, headers: &HeaderMap, role: Role, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, role).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn flash_to(path: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let joiner = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{path}{joiner}{encoded}")).into_response()
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(serde_json::json!({"error": message}))).into_response()
}

/// The editor's own address for a post.
pub(crate) fn editor_href(post_id: &str) -> String {
    format!("/ghost/posts/{}/builder/", esc(post_id))
}

/// Whether this person may open the builder on this kind of post. Being
/// allowed to edit the post itself is checked separately, by `may_edit_post`.
pub(crate) fn role_may_build(s: &Settings, role: Role) -> bool {
    match s.who {
        Who::Everyone => true,
        Who::Editors => role >= Role::Editor,
        Who::Administrators => role.is_admin(),
    }
}

/// What the post editor shows for the builder: the link, and whether a
/// layout is live on this post. `None` when the builder is off, does not
/// allow this kind of post, or this person may not use it.
pub(crate) async fn editor_link(state: &AppState, role: Role, post: &vuuut_core::models::Post) -> Option<(String, bool)> {
    let s = builder_settings(state);
    if !s.allows(&post.post_type) || !role_may_build(&s, role) {
        return None;
    }
    let active = repo::get(&state.db, &post.id).await.ok().flatten().map(|l| l.active).unwrap_or(false);
    Some((editor_href(&post.id), active))
}

/// The id shape posts use. Checked before an id is used in a query or a
/// link, so a hand-typed path cannot reach either.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn tabs(active: &str) -> String {
    let tab = |key: &str, label: &str, href: &str| {
        format!(r#"<a class="seg{on}" href="{href}">{label}<span style="width:8px"></span></a>"#, on = if key == active { " on" } else { "" }, label = esc(label))
    };
    format!(
        r#"<div class="seg-bar rise" style="--i:1">{}{}{}{}</div>"#,
        tab("overview", "Overview", BASE),
        tab("theme", "Theme Builder", THEME),
        tab("templates", "Templates", TEMPLATES),
        tab("settings", "Settings", SETTINGS),
    )
}

fn tile(label: &str, value: &str, detail: &str, icon: &str, tint: &str) -> String {
    format!(
        r#"<div class="card padded vb-tile"><span class="ic tint-{tint}">{ic}</span><span class="vb-tile-value">{value}</span><span class="vb-tile-label">{label}</span><span class="vb-tile-detail">{detail}</span></div>"#,
        ic = svg(icon),
        value = esc(value),
        label = esc(label),
        detail = esc(detail),
    )
}

fn when(at: &str) -> String {
    at.get(..16).map(|s| s.replace('T', " ")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

pub(crate) async fn builder_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = builder_settings(&state);
    let layouts = repo::list(&state.db).await.unwrap_or_default();
    let templates = repo::templates(&state.db).await.unwrap_or_default();
    let live = layouts.iter().filter(|l| l.active).count();
    let has: std::collections::HashSet<&str> = layouts.iter().map(|l| l.post_id.as_str()).collect();

    // Candidates for "Open in the builder": recent posts and pages without
    // a layout yet, of the kinds the settings allow.
    let candidates: Vec<(String, String, String)> = sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT id, title, type FROM posts WHERE type IN ('post','page') ORDER BY updated_at DESC LIMIT 300",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .filter(|(id, _, t)| !has.contains(id.as_str()) && s.allows(t))
    .map(|(id, title, t)| (id, title.unwrap_or_else(|| "Untitled".into()), t))
    .collect();

    let off = if s.enabled {
        String::new()
    } else {
        format!(
            r#"<div class="card padded vb-live vb-off rise" style="--i:2"><span class="ic tint-amber">{}</span><div><b>The builder is switched off.</b>
<p class="muted">Layouts are kept, but every page shows its own content and the editor's button is hidden. Turn it on under Settings.</p></div><a class="btn" href="{SETTINGS}">{} Settings</a></div>"#,
            svg("hide"),
            svg("settings"),
        )
    };

    let rows: String = layouts
        .iter()
        .map(|l| {
            let (sections, widgets) = serde_json::from_str::<serde_json::Value>(&l.data).map(|v| rules::counts(&v)).unwrap_or((0, 0));
            let view = if l.status == "published" {
                format!(r#"<a class="icon-btn" href="{}" target="_blank" rel="noopener" title="View on the site">{}</a>"#, esc(&vuuut_core::url::base(&format!("/{}/", l.slug))), svg("open"))
            } else if !l.uuid.is_empty() {
                format!(r#"<a class="icon-btn" href="{}" target="_blank" rel="noopener" title="Preview">{}</a>"#, esc(&vuuut_core::url::base(&format!("/p/{}/", l.uuid))), svg("eye"))
            } else {
                String::new()
            };
            format!(
                r#"<div class="row vb-row-item"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title"><a href="{edit}">{title}</a> <span class="chip {schip}">{status}</span> <span class="chip {lchip}">{live}</span></span>
<span class="detail">{kind} · {sections} · {widgets} · saved {at}</span></div>
<div class="trailing vb-actions">
<a class="btn tonal small" href="{edit}">{pen} Edit design</a><a class="btn text small" href="{BASE}{id}/export/">Export</a>{view}
<form method="post" action="{BASE}{id}/active/"><input type="hidden" name="active" value="{flip}"><button class="btn text small" type="submit">{fliplabel}</button></form>
<form method="post" action="{BASE}{id}/delete/" data-confirm="Remove this layout?|&#8220;{title}&#8221; goes back to its own content. The layout and its history are deleted."><button class="icon-btn" type="submit" title="Remove layout">{del}</button></form>
</div></div>"#,
                tint = if l.active { "pink" } else { "slate" },
                ic = svg(if l.post_type == "page" { "pages" } else { "posts" }),
                edit = editor_href(&l.post_id),
                title = esc(if l.title.is_empty() { "Untitled" } else { &l.title }),
                schip = if l.status == "published" { "published" } else if l.status == "scheduled" { "scheduled" } else { "draft" },
                status = esc(&capitalise(&l.status)),
                lchip = if l.active { "tonal" } else { "draft" },
                live = if l.active { "Layout on" } else { "Layout off" },
                kind = if l.post_type == "page" { "Page" } else { "Post" },
                sections = countable(sections as i64, "section"),
                widgets = countable(widgets as i64, "widget"),
                at = esc(&when(&l.updated_at)),
                pen = svg("edit"),
                id = esc(&l.post_id),
                flip = if l.active { "0" } else { "1" },
                fliplabel = if l.active { "Switch off" } else { "Switch on" },
                del = svg("delete"),
            )
        })
        .collect();
    let list = if layouts.is_empty() {
        r#"<div class="card padded vb-empty-card"><p class="muted" style="margin:6px 0 0">No page has a layout yet. Create a new page below, or open any page in the builder — and from now on, every post editor has a <b>Page builder</b> button.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    let options: String = candidates
        .iter()
        .map(|(id, title, t)| format!(r#"<option value="{}">{} — {}</option>"#, esc(id), esc(title), if t == "page" { "page" } else { "post" }))
        .collect();

    page_for_role(
        actor.role,
        "Page Builder & Visual Editor",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{off}
<div class="vb-tiles rise" style="--i:2">{t1}{t2}{t3}{t4}</div>
<div class="vb-start rise" style="--i:3">
<form class="card padded vb-start-card" method="post" action="{BASE}new/"><span class="ic tint-pink">{add}</span><div class="vb-start-body"><b>Design a new page</b><p class="muted">Starts as a draft, with a blank canvas.</p>
<div class="vb-inline"><label class="field" style="margin:0;flex:1"><input name="title" maxlength="200" placeholder=" " required><span>Page title</span></label><button class="btn" type="submit">{add} Create</button></div></div></form>
<form class="card padded vb-start-card" method="get" action="{BASE}open/"><span class="ic tint-blue">{edit}</span><div class="vb-start-body"><b>Open an existing page</b><p class="muted">Its title and text come in as the first section; the original is kept.</p>
<div class="vb-inline"><label class="field fixed" style="margin:0;flex:1"><select name="post" required>{options}</select><span>Post or page</span>{caret}</label><button class="btn tonal" type="submit">Open</button></div></div></form>
</div>
<form class="card padded vb-start-card vb-import" method="post" enctype="multipart/form-data" action="{BASE}import/">
<span class="ic tint-purple">{upload}</span>
<div class="vb-start-body"><label for="vb-import-file"><b>Import a layout</b></label><p class="muted" id="vb-import-help">Open an Elementor or Vuuut JSON file as a new draft page.</p>
<div class="vb-import-actions"><input class="vb-import-file" id="vb-import-file" name="layout" type="file" aria-describedby="vb-import-help" accept=".json,application/json" required><button class="btn tonal" type="submit">Import</button></div>
<label for="vb-import-companion">Companion HTML (optional)</label>
<input class="vb-import-file" id="vb-import-companion" name="companion" type="file" accept=".html,.htm,text/html" aria-describedby="vb-import-media-help">
<p class="muted" id="vb-import-media-help">Add the matching HTML from your Elementor kit to recover missing image URLs.</p>
<label for="vb-import-kit-settings">Kit global.json (optional)</label>
<input class="vb-import-file" id="vb-import-kit-settings" name="kitSettings" type="file" accept=".json,application/json" aria-describedby="vb-import-kit-help">
<p class="muted" id="vb-import-kit-help">Use the global.json from the same templates folder for shared colors and fonts.</p>
<label for="vb-import-linked">Linked carousel templates (optional)</label>
<input class="vb-import-file" id="vb-import-linked" name="linkedTemplate" type="file" accept=".json,application/json" multiple></div></form>
<div class="section">Designed with the builder</div><div class="rise" style="--i:4">{list}</div>
<p class="muted rise" style="--i:5;margin:14px 4px 0">A layout replaces only the content area: the theme still draws the header, footer and navigation. Switch a layout off and the page shows what was written in the editor again.</p>"#,
            head = crate::tools::tool_head("builder", None, ""),
            tabs = tabs("overview"),
            banner = flash_banner(&q),
            t1 = tile("Pages with a layout", &layouts.len().to_string(), "posts and pages", "layout", "pink"),
            t2 = tile("Live", &live.to_string(), "drawn from their layout", "globe", "green"),
            t3 = tile("Templates", &templates.len().to_string(), "saved sections and pages", "copy", "purple"),
            t4 = tile("Widgets", &rules::WIDGETS.len().to_string(), "headings to maps", "dashboard", "blue"),
            add = svg("add"),
            edit = svg("edit"),
            caret = SELECT_CARET,
            upload = svg("upload"),
        ),
        true,
    )
    .into_response()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct NewPage {
    title: String,
}

/// Create a draft page and open it in the builder.
pub(crate) async fn builder_new(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<NewPage>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let title: String = form.title.trim().chars().take(200).collect();
    if title.is_empty() {
        return flash_to(BASE, "error", "Give the page a title.");
    }
    create_draft(&state, &actor, title, rules::blank(), &[]).await
}

async fn create_draft(state: &AppState, actor: &Actor, title: String, layout: serde_json::Value, warnings: &[String]) -> Response {
    let new = vuuut_db::repos::posts::NewPost {
        title,
        post_type: "page".into(),
        status: "draft".into(),
        visibility: "public".into(),
        show_title_and_feature_image: Some(false),
        author_ids: vec![actor.user.id.clone()],
        ..Default::default()
    };
    match vuuut_db::repos::posts::insert(&state.db, &new, &actor.user.id).await {
        Ok(post) => {
            let templates = repo::templates_map(&state.db).await;
            let html = draw(state, &layout, &post.id, &templates);
            if let Err(e) = repo::save(&state.db, &post.id, &layout.to_string(), &html, true, &actor.user.id).await {
                tracing::error!("builder: first save failed: {}", e.message());
                return flash_to(BASE, "error", "The draft page was created, but its layout could not be saved.");
            }
            let href = editor_href(&post.id);
            if warnings.is_empty() {
                Redirect::to(&href).into_response()
            } else {
                flash_to(&href, "import_notice", &warnings.join(" "))
            }
        }
        Err(e) => {
            tracing::error!("builder: creating a page failed: {}", e.message());
            flash_to(BASE, "error", "The page could not be created — check the logs.")
        }
    }
}

pub(crate) async fn builder_import(State(state): State<AppState>, headers: HeaderMap, form: axum::extract::Multipart) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, true).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    if !builder_settings(&state).enabled {
        return flash_to(BASE, "error", "Turn on the page builder before importing a layout.");
    }
    let upload = match read_layout_upload(form).await {
        Ok(files) => files,
        Err(error) => return flash_to(BASE, "error", error),
    };
    match elementor::import_layout_with_links(&upload.document, upload.companion.as_deref(), upload.kit.as_ref(), &upload.linked) {
        Ok(import) if import.layout["sections"].as_array().is_some_and(|sections| !sections.is_empty()) =>
            create_draft(&state, &actor, import.title, import.layout, &import.warnings).await,
        Ok(_) => flash_to(BASE, "error", "The file has no sections."),
        Err(error) => flash_to(BASE, "error", &error),
    }
}

async fn read_upload_field(
    mut field: axum::extract::multipart::Field<'_>,
    max: usize,
    too_large: &'static str,
) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE { too_large } else { "The uploaded file could not be read." }
    })? {
        if chunk.len() > max - bytes.len() {
            return Err(too_large);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

struct LayoutUpload {
    document: serde_json::Value,
    companion: Option<String>,
    kit: Option<serde_json::Value>,
    linked: Vec<serde_json::Value>,
}

async fn read_layout_upload(mut form: axum::extract::Multipart) -> Result<LayoutUpload, &'static str> {
    let mut layout = None;
    let mut companion = None;
    let mut kit = None;
    let mut linked = Vec::new();
    let mut remaining_linked_bytes = elementor::MAX_LINKED_BYTES;
    loop {
        let field = match form.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return Err("The uploaded files could not be read."),
        };
        match field.name() {
            Some("layout") if layout.is_none() => {
                let bytes = read_upload_field(field, elementor::MAX_UPLOAD_BYTES, "The layout JSON file exceeds 64 MiB.").await?;
                let json = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(&bytes);
                layout = Some(serde_json::from_slice(json).map_err(|_| "The file does not contain valid JSON.")?);
            }
            Some("companion") if companion.is_none() => {
                let bytes = read_upload_field(field, elementor::MAX_COMPANION_BYTES, "The companion HTML file exceeds 1 MB.").await?;
                if !bytes.is_empty() {
                    companion = Some(String::from_utf8(bytes).map_err(|_| "Choose a UTF-8 companion HTML file.")?);
                }
            }
            Some("kitSettings") if kit.is_none() => {
                let bytes = read_upload_field(field, elementor::MAX_KIT_SETTINGS_BYTES, "The kit settings file exceeds 1 MB.").await?;
                if !bytes.is_empty() {
                    let json = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(&bytes);
                    kit = Some(serde_json::from_slice(json).map_err(|_| "The kit settings file does not contain valid JSON.")?);
                }
            }
            Some("linkedTemplate") => {
                let bytes = read_upload_field(field, remaining_linked_bytes, "Linked carousel files exceed 8 MiB in total.").await?;
                if !bytes.is_empty() {
                    if linked.len() >= elementor::MAX_LINKED_FILES {
                        return Err("Choose at most 12 linked carousel files.");
                    }
                    remaining_linked_bytes -= bytes.len();
                    let json = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(&bytes);
                    linked.push(serde_json::from_slice(json).map_err(|_| "A linked carousel file does not contain valid JSON.")?);
                }
            }
            _ => {}
        }
    }
    Ok(LayoutUpload {document: layout.ok_or("Choose a layout JSON file.")?, companion, kit, linked})
}

pub(crate) async fn builder_export(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(response) = guard(&state, &headers, Role::Administrator, false).await {
        return response;
    }
    if !valid_id(&id) {
        return flash_to(BASE, "error", "That layout does not exist.");
    }
    let Some(saved) = repo::get(&state.db, &id).await.ok().flatten() else {
        return flash_to(BASE, "error", "That layout does not exist.");
    };
    let Ok(layout) = serde_json::from_str::<serde_json::Value>(&saved.data) else {
        return flash_to(BASE, "error", "The saved layout could not be read.");
    };
    let post = vuuut_db::repos::posts::find_by_id(&state.db, &id).await.ok().flatten();
    let title = post.as_ref().and_then(|post| post.title.as_deref()).unwrap_or("Layout");
    let export = match export_layout(&state, &layout, title, "page").await {
        Ok(export) => export,
        Err(error) => return flash_to(BASE, "error", &error),
    };
    let filename: String = vuuut_core::ecom::safe_filename(title).chars().take(175).collect();
    let disposition = vuuut_core::ecom::content_disposition(&format!("{filename}.json"));
    let body = serde_json::to_string_pretty(&export.document).unwrap_or_else(|_| export.document.to_string());
    ([(axum::http::header::CONTENT_TYPE, "application/json".to_owned()),
      (axum::http::header::CONTENT_DISPOSITION, disposition),
      (axum::http::header::CACHE_CONTROL, "private, no-store".to_owned())], body).into_response()
}

pub(crate) async fn builder_open(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, false).await {
        return r;
    }
    match q.get("post") {
        Some(id) if valid_id(id) => Redirect::to(&editor_href(id)).into_response(),
        _ => flash_to(BASE, "error", "Choose a post or page to open."),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct ActiveForm {
    active: String,
}

pub(crate) async fn builder_active(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<ActiveForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(BASE, "error", "That layout does not exist.");
    }
    let on = form.active == "1";
    match repo::set_active(&state.db, &id, on).await {
        Ok(true) => crate::tools::tool_saved(&state, BASE, if on { "Layout switched on — the page is drawn from it." } else { "Layout switched off — the page shows its own content." }),
        _ => flash_to(BASE, "error", "That layout does not exist."),
    }
}

pub(crate) async fn builder_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(BASE, "error", "That layout does not exist.");
    }
    match repo::delete(&state.db, &id).await {
        Ok(()) => crate::tools::tool_saved(&state, BASE, "Layout removed. The page shows its own content again."),
        Err(e) => {
            tracing::error!("builder: delete failed: {}", e.message());
            flash_to(BASE, "error", "Removing failed — check the logs.")
        }
    }
}

// ---------------------------------------------------------------------------
// Templates (library) and the Theme Builder
// ---------------------------------------------------------------------------

fn kind_label(kind: &str) -> &'static str {
    rules::TEMPLATE_KINDS.iter().find(|(k, _)| *k == kind).map(|k| k.1).unwrap_or("Template")
}

fn kind_icon(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "page" => ("pages", "blue"),
        "widget" => ("star", "amber"),
        "header" => ("up", "teal"),
        "footer" => ("down", "slate"),
        "single" => ("posts", "green"),
        "popup" => ("flag", "pink"),
        _ => ("layout", "purple"),
    }
}

fn template_row(t: &repo::Template, theme: bool) -> String {
    let (sections, widgets) = serde_json::from_str::<serde_json::Value>(&t.data).map(|v| rules::counts(&v)).unwrap_or((0, 0));
    let (icon, tint) = kind_icon(&t.kind);
    let opts = rules::parts::PartOptions::from_value(&serde_json::from_str(&t.options).unwrap_or(serde_json::json!({})));
    let detail = if theme {
        format!("{} · saved {}", esc(&opts.summary(&t.kind)), esc(&when(&t.updated_at)))
    } else {
        format!("{} · {} · {} · saved {}", kind_label(&t.kind), countable(sections as i64, "section"), countable(widgets as i64, "widget"), esc(&when(&t.created_at)))
    };
    let toggle = if theme {
        format!(
            r#"<form method="post" action="{TEMPLATES}{id}/active/"><input type="hidden" name="active" value="{flip}"><button class="btn text small" type="submit">{label}</button></form>"#,
            id = esc(&t.id),
            flip = if t.active { "0" } else { "1" },
            label = if t.active { "Switch off" } else { "Switch on" },
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="row vb-row-item"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title"><a href="{TEMPLATES}{id}/edit/">{name}</a>{chip}</span><span class="detail">{detail}</span></div>
<div class="trailing vb-actions"><a class="btn tonal small" href="{TEMPLATES}{id}/edit/">{pen} Edit</a>{toggle}
<form method="post" action="{TEMPLATES}{id}/delete/" data-confirm="Delete &#8220;{name}&#8221;?|It will no longer be used anywhere. Pages that embedded a copy keep it; global widgets and Template widgets pointing at it go blank."><button class="icon-btn" type="submit" title="Delete">{del}</button></form></div></div>"#,
        ic = svg(icon),
        id = esc(&t.id),
        name = esc(&t.name),
        chip = if theme { format!(r#" <span class="chip {}">{}</span>"#, if t.active { "published" } else { "draft" }, if t.active { "On" } else { "Off" }) } else { format!(r#" <span class="chip tonal">{}</span>"#, kind_label(&t.kind)) },
        pen = svg("edit"),
        del = svg("delete"),
    )
}

fn new_form(kinds: &[(&str, &str)], label: &str) -> String {
    let opts: String = kinds.iter().map(|(k, l)| format!(r#"<option value="{k}">{l}</option>"#)).collect();
    format!(
        r#"<form class="card padded vb-start-card" method="post" action="{TEMPLATES}new/"><span class="ic tint-pink">{add}</span><div class="vb-start-body"><b>{label}</b><p class="muted">Opens in the visual editor with a starting layout.</p>
<div class="vb-inline"><label class="field fixed" style="margin:0;min-width:180px"><select name="kind">{opts}</select><span>Type</span>{caret}</label><label class="field" style="margin:0;flex:1"><input name="name" maxlength="120" placeholder=" " required><span>Name</span></label><button class="btn" type="submit">{add} Create</button></div></div></form>"#,
        add = svg("add"),
        caret = SELECT_CARET,
    )
}

pub(crate) async fn templates_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let templates: Vec<repo::Template> = repo::templates(&state.db).await.unwrap_or_default().into_iter().filter(|t| !rules::is_theme_part(&t.kind)).collect();
    let rows: String = templates.iter().map(|t| template_row(t, false)).collect();
    let body = if templates.is_empty() {
        r#"<div class="card padded"><p class="muted" style="margin:0">No saved templates yet. In the editor, right-click a section and choose <b>Save as template</b>, a widget for <b>Save as global widget</b>, or save a whole page from the Page panel.</p></div>"#.to_string()
    } else {
        format!(r#"<div class="card rows">{rows}</div>"#)
    };
    page_for_role(
        actor.role,
        "Builder templates",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<div class="vb-start rise" style="--i:2">{new}</div>
<div class="section">Saved sections, pages and global widgets</div><div class="rise" style="--i:3">{body}</div>
<p class="muted rise" style="--i:4;margin:14px 4px 0">A <b>global widget</b> is edited here once and every page that uses it changes with it. The editor also has 17 ready-made sections under Add → Templates.</p>"#,
            head = crate::tools::tool_head_back("builder", BASE, None, ""),
            tabs = tabs("templates"),
            banner = flash_banner(&q),
            new = new_form(&[("section", "Section"), ("page", "Page"), ("widget", "Global widget")], "New template"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn theme_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = builder_settings(&state);
    let parts: Vec<repo::Template> = repo::templates(&state.db).await.unwrap_or_default().into_iter().filter(|t| rules::is_theme_part(&t.kind)).collect();
    let group = |kind: &str, title: &str, blurb: &str| {
        let rows: String = parts.iter().filter(|t| t.kind == kind).map(|t| template_row(t, true)).collect();
        let body = if rows.is_empty() { format!(r#"<div class="card padded"><p class="muted" style="margin:0">{}</p></div>"#, esc(blurb)) } else { format!(r#"<div class="card rows">{rows}</div>"#) };
        format!(r#"<div class="section">{}</div>{body}"#, esc(title))
    };
    let maint = if s.maintenance != rules::Maintenance::Off {
        format!(
            r#"<div class="card padded vb-live vb-off rise" style="--i:2"><span class="ic tint-amber">{}</span><div><b>{} mode is on.</b><p class="muted">Visitors see one page; you see the site because you are signed in.</p></div><a class="btn" href="{SETTINGS}">{} Settings</a></div>"#,
            svg("lock"),
            if s.maintenance == rules::Maintenance::ComingSoon { "Coming soon" } else { "Maintenance" },
            svg("settings")
        )
    } else {
        String::new()
    };
    page_for_role(
        actor.role,
        "Theme Builder",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}{maint}
<div class="vb-start rise" style="--i:2">{new}</div>
<div class="rise" style="--i:3">
{h}{f}{single}{pop}
</div>
<p class="muted rise" style="--i:4;margin:14px 4px 0">A switched-on header or footer replaces the theme's own (hidden with the selectors under Settings). When two apply to a page, the higher priority wins. Popups can also be opened from any link to <code>#popup-&lt;id&gt;</code>.</p>"#,
            head = crate::tools::tool_head_back("builder", BASE, None, ""),
            tabs = tabs("theme"),
            banner = flash_banner(&q),
            new = new_form(&[("header", "Header"), ("footer", "Footer"), ("single", "Single post template"), ("popup", "Popup")], "New Theme Builder part"),
            h = group("header", "Headers", "No header yet: the theme draws its own."),
            f = group("footer", "Footers", "No footer yet: the theme draws its own."),
            single = group("single", "Single post templates", "No template yet: posts without their own layout show the theme's post page."),
            pop = group("popup", "Popups", "No popups yet. For simple sign-up popups there is also Tools → Popups."),
        ),
        true,
    )
    .into_response()
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct NewTemplate {
    kind: String,
    name: String,
}

pub(crate) async fn template_new(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<NewTemplate>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some((kind, label)) = rules::TEMPLATE_KINDS.iter().find(|(k, _)| *k == form.kind) else {
        return flash_to(TEMPLATES, "error", "Choose what to create.");
    };
    let back = if rules::is_theme_part(kind) { THEME } else { TEMPLATES };
    let name: String = form.name.trim().chars().take(120).collect();
    let name = if name.is_empty() { label.to_string() } else { name };
    let layout = rules::normalise(&rules::starter_for(kind)).unwrap_or_else(|_| rules::blank());
    let opts = rules::parts::PartOptions::from_value(&serde_json::json!({"post_types": "post"})).to_value().to_string();
    match repo::create_template_full(&state.db, &name, kind, &layout.to_string(), &opts, "", false, &actor.user.id).await {
        Ok(id) => {
            let templates = repo::templates_map(&state.db).await;
            let html = draw(&state, &layout, &id, &templates);
            let _ = repo::set_template_html(&state.db, &id, &html).await;
            Redirect::to(&format!("{TEMPLATES}{id}/edit/")).into_response()
        }
        Err(e) => {
            tracing::error!("builder: template create failed: {}", e.message());
            flash_to(back, "error", "Creating failed — check the logs.")
        }
    }
}

pub(crate) async fn template_active(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<ActiveForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(THEME, "error", "That part does not exist.");
    }
    match repo::set_template_active(&state.db, &id, form.active == "1").await {
        Ok(true) => crate::tools::tool_saved(&state, THEME, if form.active == "1" { "Switched on — it shows on the site now." } else { "Switched off." }),
        _ => flash_to(THEME, "error", "That part does not exist."),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct RenameForm {
    name: String,
}

pub(crate) async fn template_rename(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<RenameForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let name: String = form.name.trim().chars().take(120).collect();
    if name.is_empty() || !valid_id(&id) {
        return flash_to(TEMPLATES, "error", "A template needs a name.");
    }
    match repo::rename_template(&state.db, &id, &name).await {
        Ok(true) => flash_to(TEMPLATES, "ok", "Template renamed."),
        _ => flash_to(TEMPLATES, "error", "That template does not exist."),
    }
}

pub(crate) async fn template_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(TEMPLATES, "error", "That template does not exist.");
    }
    let theme = repo::template(&state.db, &id).await.ok().flatten().is_some_and(|t| rules::is_theme_part(&t.kind));
    match repo::delete_template(&state.db, &id).await {
        Ok(true) => {
            rerender_all(&state).await;
            flash_to(if theme { THEME } else { TEMPLATES }, "ok", "Deleted.")
        }
        _ => flash_to(TEMPLATES, "error", "That template does not exist."),
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn switch(name: &str, on: bool) -> String {
    format!(r#"<label class="switch"><input type="checkbox" name="{name}" value="1"{c}><span class="track"><span class="thumb"></span></span></label>"#, c = if on { " checked" } else { "" })
}

fn setting_row(icon: &str, tint: &str, title: &str, detail: &str, control: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing vb-control">{control}</div></div>"#,
        ic = svg(icon),
        title = esc(title),
        detail = esc(detail),
    )
}

fn select(name: &str, label: &str, opts: &[(String, String)], current: &str) -> String {
    let o: String = opts.iter().map(|(v, l)| format!(r#"<option value="{}"{s}>{}</option>"#, esc(v), esc(l), s = if v == current { " selected" } else { "" })).collect();
    format!(r#"<label class="field fixed" style="margin:0"><select name="{name}">{o}</select><span>{label}</span>{caret}</label>"#, label = esc(label), caret = SELECT_CARET)
}

fn text_input(name: &str, label: &str, value: &str, placeholder: &str) -> String {
    format!(r#"<label class="field" style="margin:0;min-width:300px"><input name="{name}" value="{}" placeholder="{}" maxlength="400"><span>{}</span></label>"#, esc(value), esc(placeholder), esc(label))
}

pub(crate) async fn settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Administrator, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = builder_settings(&state);
    let px = |list: &[u32], unit: &str| -> Vec<(String, String)> { list.iter().map(|n| (n.to_string(), format!("{n}{unit}"))).collect() };
    let radii: Vec<(String, String)> = rules::RADII.iter().map(|n| (n.to_string(), match n { 0 => "Square".to_string(), 999 => "Pill".to_string(), n => format!("{n}px") })).collect();
    let fonts: Vec<(String, String)> = rules::FONTS.iter().map(|f| (f.0.to_string(), f.1.to_string())).collect();
    let pages: Vec<(String, String)> = std::iter::once((String::new(), "Choose a page…".to_string()))
        .chain(
            sqlx::query_as::<_, (String, Option<String>)>("SELECT id, title FROM posts WHERE type = 'page' ORDER BY title LIMIT 300")
                .fetch_all(&state.db)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(id, t)| (id, t.unwrap_or_else(|| "Untitled".into()))),
        )
        .collect();
    let homepage = state.settings.get_str(rules::SETTING_HOMEPAGE).unwrap_or_default();
    let mut home_options = vec![(String::new(), "Default homepage".to_string())];
    home_options.extend(sqlx::query_as::<_, (String, Option<String>, Option<String>, String)>(
        "SELECT id, title, slug, status FROM posts WHERE type = 'page' ORDER BY title, id",
    ).fetch_all(&state.db).await.unwrap_or_default().into_iter().map(|(id, title, slug, status)| {
        let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
        let state = if status == "published" { String::new() } else { format!(" · {}", capitalise(&status)) };
        (id, format!("{title} (/{}/){state}", slug.unwrap_or_default()))
    }));
    if !home_options.iter().any(|(id, _)| id == &homepage) {
        home_options.push((homepage.clone(), "Unavailable page (using the default homepage)".into()));
    }
    let palette: String = s
        .palette
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let shown = if c.is_empty() { "#ff1a75".to_string() } else { c.clone() };
            format!(
                r#"<label class="vb-swatch"><input type="color" name="palette_{i}" value="{shown}" data-vb-swatch><span class="vb-swatch-name">{name}</span><span class="vb-swatch-hex" data-vb-hex>{label}</span>{accent}</label>"#,
                shown = esc(&shown[..7.min(shown.len())]),
                name = esc(rules::PALETTE_NAMES[i]),
                label = if c.is_empty() { "Site accent".to_string() } else { esc(c) },
                accent = if i == 0 { format!(r#"<span class="vb-swatch-follow"><input type="checkbox" name="palette_accent" value="1"{}> Follow the site accent</span>"#, if c.is_empty() { " checked" } else { "" }) } else { String::new() },
            )
        })
        .collect();
    page_for_role(
        actor.role,
        "Builder settings",
        "tools",
        Some(&actor.user),
        &format!(
            r#"{head}{tabs}{banner}
<form method="post" action="{SETTINGS}" id="vb-settings">
<div class="section">The builder</div>
<div class="card rows rise" style="--i:2">{enabled}{types}{who}</div>
<div class="section">Homepage</div>
<div class="card rows rise" style="--i:3">{home}</div>
<div class="section">Global colours</div>
<div class="card padded rise" style="--i:3"><p class="muted" style="margin:0 0 14px">Every colour control in the editor offers these first. Change one here and every page that uses it changes with it.</p><div class="vb-swatches">{palette}</div></div>
<div class="section">Typography</div>
<div class="card rows rise" style="--i:4">{hfont}{bfont}{google}{cname}{curl}</div>
<div class="section">Layout</div>
<div class="card rows rise" style="--i:5">{width}{space}{radius}{anim}</div>
<div class="section">Theme Builder</div>
<div class="card rows rise" style="--i:6">{hh}{hf}</div>
<div class="section">Coming soon and maintenance</div>
<div class="card rows rise" style="--i:7">{mode}{mpage}</div>
<p class="muted rise" style="--i:8;margin:12px 4px 0">Breakpoints: tablet up to 1024px, mobile up to 640px. Signed-in staff always see the real site, even in maintenance mode.</p>
</form>
<script>document.querySelectorAll('[data-vb-swatch]').forEach(function(i){{i.addEventListener('input',function(){{var h=i.parentNode.querySelector('[data-vb-hex]');h.textContent=i.value;var f=i.parentNode.querySelector('[name=palette_accent]');if(f)f.checked=false;}});}});</script>"#,
            head = crate::tools::tool_head_back("builder", BASE, Some("vb-settings"), ""),
            tabs = tabs("settings"),
            banner = flash_banner(&q),
            enabled = setting_row("layout", if s.enabled { "green" } else { "slate" }, "Use the page builder", if s.enabled { "Layouts and Theme Builder parts are live." } else { "Off: every page shows its own content. Layouts are kept." }, &switch(rules::SETTING_ENABLED, s.enabled)),
            types = setting_row("pages", "blue", "Available on", "Which entries can be designed with the builder.", &select(rules::SETTING_POST_TYPES, "Available on", &[("all".into(), "Posts and pages".into()), ("pages".into(), "Pages only".into())], if s.post_types == rules::PostTypes::Pages { "pages" } else { "all" })),
            who = setting_row("group", "purple", "Who can use it", "On top of being allowed to edit the post or page.", &select(rules::SETTING_WHO, "Who", &[("everyone".into(), "Anyone who can edit it".into()), ("editors".into(), "Editors and above".into()), ("administrators".into(), "Administrators only".into())], s.who.key())),
            home = setting_row("home", "teal", "Page to show at the site root", "The selected page takes over once published. Active builder layouts and page access rules apply.", &select(rules::SETTING_HOMEPAGE, "Homepage", &home_options, &homepage)),
            hfont = setting_row("title", "slate", "Heading font", "Used by every heading unless a widget picks its own.", &select(rules::SETTING_FONT, "Headings", &fonts, &s.font)),
            bfont = setting_row("title", "slate", "Body font", "Text in builder layouts.", &select(rules::SETTING_BODY_FONT, "Body", &fonts, &s.body_font)),
            google = setting_row("globe", "blue", "Load Google Fonts", "Only fonts a page uses are requested, from fonts.googleapis.com. Off: those fall back to the system font.", &switch(rules::SETTING_GOOGLE_FONTS, s.google_fonts)),
            cname = setting_row("title", "amber", "Custom font name", "Offered as “Your custom font” in every font menu.", &text_input(rules::SETTING_CUSTOM_FONT_NAME, "Name", &s.custom_font_name, "e.g. Brand Sans")),
            curl = setting_row("upload", "amber", "Custom font file", "An https:// or /content/ address of a .woff2 file.", &text_input(rules::SETTING_CUSTOM_FONT_URL, "Address", &s.custom_font_url, "https://…/brand.woff2")),
            width = setting_row("layout", "teal", "Content width", "How wide a boxed section's content is.", &select(rules::SETTING_WIDTH, "Width", &px(rules::WIDTHS, "px"), &s.width.to_string())),
            space = setting_row("tune", "amber", "Section spacing", "The space above and below each section, unless it sets its own.", &select(rules::SETTING_SPACE, "Spacing", &px(rules::SPACES, "px"), &s.space.to_string())),
            radius = setting_row("tune", "pink", "Corner rounding", "Buttons, boxes and cards.", &select(rules::SETTING_RADIUS, "Corners", &radii, &s.radius.to_string())),
            anim = setting_row("play", "green", "Motion effects", "Entrance, scroll and mouse effects. Visitors who ask for less motion never see them.", &switch(rules::SETTING_ANIMATIONS, s.animations)),
            hh = setting_row("up", "teal", "Theme header to hide", "CSS selectors for the theme's own header, hidden while a builder header is on.", &text_input(rules::SETTING_HIDE_HEADER, "Selectors", &s.hide_header, rules::DEFAULT_HIDE_HEADER)),
            hf = setting_row("down", "slate", "Theme footer to hide", "Hidden while a builder footer is on.", &text_input(rules::SETTING_HIDE_FOOTER, "Selectors", &s.hide_footer, rules::DEFAULT_HIDE_FOOTER)),
            mode = setting_row("lock", if s.maintenance == rules::Maintenance::Off { "slate" } else { "amber" }, "Mode", "Coming soon answers 200 with noindex; maintenance answers 503 so search engines come back later.", &select(rules::SETTING_MAINTENANCE, "Mode", &[("off".into(), "Off — the site is open".into()), ("coming_soon".into(), "Coming soon".into()), ("maintenance".into(), "Maintenance".into())], s.maintenance.key())),
            mpage = setting_row("pages", "pink", "Page to show", "Design it with the builder; its theme title is hidden.", &select(rules::SETTING_MAINTENANCE_PAGE, "Page", &pages, &s.maintenance_page)),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Administrator, true).await {
        return r;
    }
    let current = state.settings.get_str(rules::SETTING_HOMEPAGE).unwrap_or_default();
    let homepage = form.get(rules::SETTING_HOMEPAGE).map(String::as_str).unwrap_or(&current);
    if !homepage.is_empty() && homepage != current {
        let eligible = valid_id(homepage)
            && vuuut_db::repos::posts::find_by_id(&state.db, homepage).await.ok().flatten()
                .is_some_and(|p| p.post_type == "page");
        if !eligible {
            return flash_to(SETTINGS, "error", "Choose an existing page for the homepage.");
        }
    }
    let flag = |k: &str| form.get(k).is_some_and(|v| v == "1");
    let palette: Vec<String> = (0..6)
        .map(|i| {
            if i == 0 && flag("palette_accent") {
                String::new()
            } else {
                form.get(&format!("palette_{i}")).and_then(|v| rules::hex_colour(v)).unwrap_or_else(|| rules::DEFAULT_PALETTE[i].to_string())
            }
        })
        .collect();
    let s = Settings::read(|key| match key {
        rules::SETTING_ENABLED | rules::SETTING_ANIMATIONS | rules::SETTING_GOOGLE_FONTS => Some(flag(key).to_string()),
        rules::SETTING_PALETTE => Some(palette.join(",")),
        _ => form.get(key).cloned(),
    });
    let writes = [
        (rules::SETTING_ENABLED, serde_json::json!(s.enabled)),
        (rules::SETTING_POST_TYPES, serde_json::json!(if s.post_types == rules::PostTypes::Pages { "pages" } else { "all" })),
        (rules::SETTING_WHO, serde_json::json!(s.who.key())),
        (rules::SETTING_WIDTH, serde_json::json!(s.width)),
        (rules::SETTING_SPACE, serde_json::json!(s.space)),
        (rules::SETTING_RADIUS, serde_json::json!(s.radius)),
        (rules::SETTING_PALETTE, serde_json::json!(s.palette_value())),
        (rules::SETTING_FONT, serde_json::json!(s.font)),
        (rules::SETTING_BODY_FONT, serde_json::json!(s.body_font)),
        (rules::SETTING_GOOGLE_FONTS, serde_json::json!(s.google_fonts)),
        (rules::SETTING_CUSTOM_FONT_NAME, serde_json::json!(s.custom_font_name)),
        (rules::SETTING_CUSTOM_FONT_URL, serde_json::json!(s.custom_font_url)),
        (rules::SETTING_ANIMATIONS, serde_json::json!(s.animations)),
        (rules::SETTING_HIDE_HEADER, serde_json::json!(s.hide_header)),
        (rules::SETTING_HIDE_FOOTER, serde_json::json!(s.hide_footer)),
        (rules::SETTING_MAINTENANCE, serde_json::json!(s.maintenance.key())),
        (rules::SETTING_MAINTENANCE_PAGE, serde_json::json!(s.maintenance_page)),
        (rules::SETTING_HOMEPAGE, serde_json::json!(homepage)),
    ];
    if let Err(error) = state.settings.set_many(&state.db, &writes).await {
        tracing::error!("builder: saving settings failed: {}", error.message());
        return flash_to(SETTINGS, "error", "Saving failed — check the logs.");
    }
    if s.maintenance != rules::Maintenance::Off && s.maintenance_page.is_empty() {
        return flash_to(SETTINGS, "error", "Saved — but maintenance mode needs a page to show, so the site stays open until you choose one.");
    }
    rerender_all(&state).await;
    crate::tools::tool_saved(&state, SETTINGS, "Builder settings saved. Every layout now uses them.")
}

/// Draw a layout for the site (not the editor), with every template
/// available to the Template, Global and Off-canvas widgets.
fn draw(state: &AppState, layout: &serde_json::Value, scope: &str, templates: &HashMap<String, serde_json::Value>) -> String {
    let s = builder_settings(state);
    rules::render_with(layout, &rules::Ctx { settings: &s, scope, editing: false, templates })
}

/// Bring previously imported pages up to date before serving their stored HTML.
pub async fn refresh_elementor_layouts(state: &AppState) {
    let layouts = match repo::stale_elementor_layouts(&state.db, elementor::STYLE_MARKER).await {
        Ok(layouts) if layouts.is_empty() => return,
        Ok(layouts) => layouts,
        Err(error) => {
            tracing::warn!("builder: checking Elementor layouts failed: {}", error.message());
            return;
        }
    };
    let templates = repo::templates_map(&state.db).await;
    for (id, data) in layouts {
        let Ok(value) = serde_json::from_str(&data) else { continue };
        let Ok(layout) = rules::normalise(&value) else { continue };
        let updated = draw(state, &layout, &id, &templates);
        if updated.contains(elementor::STYLE_MARKER) {
            if let Err(error) = repo::refresh_html(&state.db, &id, &data, &updated).await {
                tracing::warn!("builder: updating Elementor layout {id} failed: {}", error.message());
            }
        }
    }
}

/// Colours, widths, fonts and global widgets are drawn into every stored
/// layout, so a change to any of them draws every layout again.
pub(crate) async fn rerender_all(state: &AppState) {
    let templates = repo::templates_map(&state.db).await;
    for (post_id, data) in repo::all_data(&state.db).await.unwrap_or_default() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else { continue };
        let Ok(layout) = rules::normalise(&v) else { continue };
        let _ = repo::set_html(&state.db, &post_id, &draw(state, &layout, &post_id, &templates)).await;
    }
    for t in repo::templates(&state.db).await.unwrap_or_default() {
        if !rules::is_theme_part(&t.kind) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t.data) else { continue };
        let Ok(layout) = rules::normalise(&v) else { continue };
        let _ = repo::set_template_html(&state.db, &t.id, &draw(state, &layout, &t.id, &templates)).await;
    }
}

// ---------------------------------------------------------------------------
// The visual editor — for a post, or for a template
// ---------------------------------------------------------------------------

/// Everything the editor needs before it may act on a post: a session, the
/// right to edit this post, and the builder's own settings allowing it.
async fn editor_guard(state: &AppState, headers: &HeaderMap, id: &str, writes: bool) -> Result<(Actor, vuuut_core::models::Post), Response> {
    let actor = require_actor(state, headers).await?;
    if writes && !vuuut_api::forms::same_origin(headers, &state.site_url()) {
        return Err(json_error(StatusCode::FORBIDDEN, "This request did not come from the admin."));
    }
    if !valid_id(id) || !may_edit_post(state, &actor, id).await {
        return Err(if writes { json_error(StatusCode::FORBIDDEN, "You cannot edit this page.") } else { forbidden_page(&actor, Role::Editor) });
    }
    let Some(post) = vuuut_db::repos::posts::find_by_id(&state.db, id).await.ok().flatten() else {
        return Err(if writes { json_error(StatusCode::NOT_FOUND, "This page no longer exists.") } else { (StatusCode::NOT_FOUND, "not found").into_response() });
    };
    let s = builder_settings(state);
    if !s.allows(&post.post_type) || !role_may_build(&s, actor.role) {
        let msg = if !s.enabled {
            "The page builder is switched off under Tools → Page Builder."
        } else if !s.allows(&post.post_type) {
            "The page builder is set to pages only."
        } else {
            "Your role cannot use the page builder on this site."
        };
        return Err(if writes { json_error(StatusCode::FORBIDDEN, msg) } else { flash_to(&format!("/ghost/posts/{}/", esc(id)), "error", msg) });
    }
    Ok((actor, post))
}

/// Templates are site-wide: Administrator, and origin-checked for writes.
async fn template_guard(state: &AppState, headers: &HeaderMap, id: &str, writes: bool) -> Result<(Actor, repo::Template), Response> {
    let actor = guard(state, headers, Role::Administrator, writes).await.map_err(|r| if writes { json_error(StatusCode::FORBIDDEN, "Not allowed.") } else { r })?;
    if !valid_id(id) {
        return Err(json_error(StatusCode::NOT_FOUND, "No such template."));
    }
    match repo::template(&state.db, id).await.ok().flatten() {
        Some(t) => Ok((actor, t)),
        None => Err(if writes { json_error(StatusCode::NOT_FOUND, "No such template.") } else { flash_to(TEMPLATES, "error", "That template does not exist.") }),
    }
}

/// Escape the less-than character so HTML inside an imported template is
/// safe to embed and remains valid JSON.
fn script_json(v: &serde_json::Value) -> String {
    v.to_string().replace('<', r"\u003c")
}

fn templates_json(list: &[repo::Template]) -> serde_json::Value {
    serde_json::Value::Array(
        list.iter()
            .filter(|t| !rules::is_theme_part(&t.kind))
            .filter_map(|t| {
                let data: serde_json::Value = serde_json::from_str(&t.data).ok()?;
                Some(serde_json::json!({"id": t.id, "name": t.name, "kind": t.kind, "data": data}))
            })
            .collect(),
    )
}

fn editor_js_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(include_str!("../assets/builder-editor.js")));
    &HASH
}

fn editor_css_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::asset_hash(crate::assets::BUILDER_EDITOR_CSS));
    &HASH
}

/// What every editor boot carries, whatever it edits.
async fn boot_common(state: &AppState, canvas_uuid: Option<String>) -> serde_json::Value {
    let s = builder_settings(state);
    let templates = repo::templates(&state.db).await.unwrap_or_default();
    let forms: Vec<serde_json::Value> = vuuut_db::repos::forms::list(&state.db).await.unwrap_or_default().into_iter().map(|f| serde_json::json!([f.id, f.name])).collect();
    serde_json::json!({
        "canvasUrl": canvas_uuid.map(|u| vuuut_core::url::base(&format!("/p/{u}/?vb=canvas"))).unwrap_or_default(),
        "cssUrl": format!("{}?v={}", vuuut_core::url::base("/public/vuuut-builder.css"), crate::asset_hash(rules::CSS)),
        "jsUrl": format!("{}?v={}", vuuut_core::url::base("/public/vuuut-builder.js"), crate::asset_hash(rules::RUNTIME)),
        "vars": s.css_vars(),
        "settings": {
            "palette": s.palette.iter().enumerate().map(|(i, c)| serde_json::json!({"key": format!("c{}", i + 1), "name": rules::PALETTE_NAMES[i], "value": if c.is_empty() { "var(--ghost-accent-color, #ff1a75)".to_string() } else { c.clone() }})).collect::<Vec<_>>(),
            "animations": s.animations,
            "width": s.width,
        },
        "spec": rules::spec::spec_json(),
        "icons": rules::ICONS.iter().map(|(k, d)| serde_json::json!([k, d])).collect::<Vec<_>>(),
        "templates": templates_json(&templates),
        "refs": templates.iter().map(|t| serde_json::json!([t.id, t.name, t.kind])).collect::<Vec<_>>(),
        "forms": forms,
        "unsplash": crate::editor::unsplash_headers(state),
        "toolUrl": BASE,
        "layoutLimits": {"bytes": rules::spec::MAX_BYTES, "uploadBytes": elementor::MAX_UPLOAD_BYTES, "sections": rules::spec::MAX_SECTIONS, "companionBytes": elementor::MAX_COMPANION_BYTES, "kitSettingsBytes": elementor::MAX_KIT_SETTINGS_BYTES, "linkedBytes": elementor::MAX_LINKED_BYTES, "linkedFiles": elementor::MAX_LINKED_FILES},
        "upload": "/ghost/api/admin/images/upload/",
    })
}

fn editor_shell(title: &str, back: &str, boot: &serde_json::Value) -> Response {
    Html(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="stylesheet" href="{assets}/builder-editor.css?v={css_hash}">
</head><body class="vbe">
<div id="vbe-app" class="vbe-app" aria-busy="true">
<header class="vbe-top" id="vbe-top"></header>
<aside class="vbe-side" id="vbe-side"></aside>
<main class="vbe-stage" id="vbe-stage"><div class="vbe-frame" id="vbe-frame" data-device="desktop"><iframe id="vbe-canvas" title="Page canvas"></iframe></div></main>
</div>
<noscript><div class="vbe-noscript"><h1>The visual editor needs JavaScript</h1><p>Turn it on, or <a href="{back}">go back</a>.</p></div></noscript>
<script type="application/json" id="vb-boot">{boot}</script>
<script src="{assets}/unsplash-picker.js" defer></script>
<script src="{assets}/builder-editor.js?v={js_hash}" defer></script>
</body></html>"#,
        title = esc(title),
        assets = ASSETS,
        css_hash = editor_css_hash(),
        js_hash = editor_js_hash(),
        back = esc(back),
        boot = script_json(boot),
    ))
    .into_response()
}

pub(crate) async fn editor_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> Response {
    let (_actor, post) = match editor_guard(&state, &headers, &id, false).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let existing = repo::get(&state.db, &post.id).await.ok().flatten();
    let (layout, active, saved_at) = match &existing {
        Some(l) => (serde_json::from_str::<serde_json::Value>(&l.data).ok().and_then(|v| rules::normalise(&v).ok()).unwrap_or_else(rules::blank), l.active, l.updated_at.clone()),
        None => (rules::normalise(&rules::starter(post.title.as_deref().unwrap_or(""), post.html.as_deref().unwrap_or(""))).unwrap_or_else(|_| rules::blank()), true, String::new()),
    };
    let base = format!("/ghost/posts/{}/builder/", post.id);
    let uuid = post.uuid.clone().unwrap_or_default();
    let view_url = if post.status == "published" { crate::languages::public_url(&state, &post).await } else { String::new() };
    let mut boot = boot_common(&state, (!uuid.is_empty()).then(|| uuid.clone())).await;
    let extra = serde_json::json!({
        "target": {"kind": "post"},
        "post": {
            "id": post.id, "title": post.title.clone().unwrap_or_default(), "type": post.post_type, "status": post.status,
            "showTitle": post.show_title_and_feature_image, "editUrl": format!("/ghost/posts/{}/", post.id), "viewUrl": view_url,
            "previewUrl": if uuid.is_empty() { String::new() } else { vuuut_core::url::base(&format!("/p/{uuid}/")) },
        },
        "layout": layout, "active": active, "savedAt": saved_at, "isNew": existing.is_none(), "importNotice": query.get("import_notice"),
        "endpoints": {"save": format!("{base}save/"), "render": format!("{base}render/"), "import": format!("{base}import/"), "interchange": format!("{base}interchange/"), "catalogue": format!("{base}catalogue/"), "revisions": format!("{base}revisions/"), "templates": format!("{base}templates/")},
    });
    merge(&mut boot, extra);
    editor_shell(&format!("Design · {}", post.title.as_deref().unwrap_or("Untitled")), &format!("/ghost/posts/{}/", post.id), &boot)
}

fn merge(a: &mut serde_json::Value, b: serde_json::Value) {
    if let (Some(a), serde_json::Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
}

/// A recent published entry whose preview hosts a template's canvas, so a
/// header or popup is designed inside the real theme.
async fn host_uuid(state: &AppState) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT uuid FROM posts WHERE status = 'published' AND type = 'page' ORDER BY published_at DESC LIMIT 1")
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .flatten()
}

pub(crate) async fn template_editor(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let (_actor, t) = match template_guard(&state, &headers, &id, false).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let layout = serde_json::from_str::<serde_json::Value>(&t.data).ok().and_then(|v| rules::normalise(&v).ok()).unwrap_or_else(rules::blank);
    let mut boot = boot_common(&state, host_uuid(&state).await).await;
    let opts = rules::parts::PartOptions::from_value(&serde_json::from_str(&t.options).unwrap_or(serde_json::json!({})));
    let base = format!("{TEMPLATES}{}/", t.id);
    let back = if rules::is_theme_part(&t.kind) { THEME } else { TEMPLATES };
    let extra = serde_json::json!({
        "target": {"kind": "template", "id": t.id, "templateKind": t.kind, "kindLabel": kind_label(&t.kind), "name": t.name, "options": opts.to_value(), "theme": rules::is_theme_part(&t.kind),
            "triggers": rules::parts::TRIGGERS, "freqs": rules::parts::FREQS, "positions": rules::parts::POSITIONS, "back": back},
        "post": {"id": "", "title": t.name, "type": t.kind, "status": if t.active { "on" } else { "off" }, "showTitle": true, "editUrl": back, "viewUrl": "", "previewUrl": ""},
        "layout": layout, "active": t.active, "savedAt": t.updated_at, "isNew": false,
        "endpoints": {"save": format!("{base}save/"), "render": format!("{base}render/"), "import": format!("{base}import/"), "interchange": format!("{base}interchange/"), "catalogue": format!("{base}catalogue/"), "revisions": "", "templates": ""},
    });
    merge(&mut boot, extra);
    editor_shell(&format!("{} · {}", kind_label(&t.kind), t.name), back, &boot)
}

#[derive(serde::Deserialize)]
pub(crate) struct CatalogueQuery {
    kind: String,
    search: Option<String>,
    offset: Option<i64>,
    selected: Option<String>,
}

pub(crate) async fn editor_catalogue(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(query): Query<CatalogueQuery>) -> Response {
    if let Err(response) = editor_guard(&state, &headers, &id, false).await {
        return response;
    }
    catalogue(&state, query).await
}

pub(crate) async fn template_catalogue(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(query): Query<CatalogueQuery>) -> Response {
    if let Err(response) = template_guard(&state, &headers, &id, false).await {
        return response;
    }
    catalogue(&state, query).await
}

async fn catalogue(state: &AppState, query: CatalogueQuery) -> Response {
    use vuuut_db::repos::{directory, ecom};

    const PAGE_SIZE: i64 = 20;
    let search: String = query.search.as_deref().unwrap_or("").chars().take(100).collect();
    let terms: Vec<_> = search.split_whitespace()
        .map(|word| word.to_lowercase().replace(['%', '_'], ""))
        .filter(|word| !word.is_empty()).take(8).collect();
    let offset = query.offset.unwrap_or_default().max(0);
    let now = vuuut_core::dates::now_db();
    let selected = query.selected.map(|ids| ids.split(',').filter(|id| valid_id(id))
        .take(rules::spec::MAX_CATALOGUE_ITEMS).map(String::from).collect::<Vec<_>>());

    let result = match (query.kind.as_str(), selected.as_deref()) {
        ("products", Some(ids)) => {
            let rows = ecom::products_by_ids(&state.db, ids).await.into_iter().filter(ecom::Product::is_active);
            Ok((rows.map(|p| (p.id, p.title)).collect::<Vec<_>>(), 0))
        }
        ("directory", Some(ids)) => {
            let rows = directory::visible_by_ids(&state.db, ids, &now).await;
            Ok((rows.into_iter().map(|l| (l.id, l.title)).collect(), 0))
        }
        ("products", None) => {
            let browse = ecom::ProductBrowse { terms: &terms, status: Some("active"), sort: "title", limit: PAGE_SIZE, offset, ..Default::default() };
            ecom::browse_products(&state.db, &browse).await.map(|(rows, total)| {
                (rows.into_iter().map(|p| (p.id, p.title)).collect(), total)
            })
        }
        ("directory", None) => {
            let browse = directory::Browse { terms: &terms, visible_at: Some(&now), limit: PAGE_SIZE, offset, ..Default::default() };
            directory::browse(&state.db, &browse).await.map(|(rows, total)| {
                (rows.into_iter().map(|l| (l.id, l.title)).collect(), total)
            })
        }
        _ => return json_error(StatusCode::BAD_REQUEST, "Unknown catalogue."),
    };
    match result {
        Ok((items, total)) => axum::Json(serde_json::json!({"items": items, "next": (offset.saturating_add(PAGE_SIZE) < total).then(|| offset + PAGE_SIZE), "pageSize": PAGE_SIZE})).into_response(),
        Err(error) => {
            tracing::error!("builder: loading catalogue failed: {}", error.message());
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "The catalogue could not be loaded.")
        }
    }
}

async fn export_layout(state: &AppState, value: &serde_json::Value, title: &str, kind: &str) -> Result<elementor::Export, String> {
    let layout = rules::normalise(value)?;
    let settings = builder_settings(state);
    let templates = repo::templates_map(&state.db).await;
    elementor::export(&layout, title, kind, &settings, &state.site_url(), &templates)
}

pub(crate) async fn editor_interchange(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (_, post) = match editor_guard(&state, &headers, &id, true).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    interchange(&state, &body, post.title.as_deref().unwrap_or("Layout"), "page").await
}

pub(crate) async fn template_interchange(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (_, template) = match template_guard(&state, &headers, &id, true).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    interchange(&state, &body, &template.name, &template.kind).await
}

pub(crate) async fn editor_upload(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, form: axum::extract::Multipart) -> Response {
    if let Err(response) = editor_guard(&state, &headers, &id, true).await {
        return response;
    }
    upload_layout(form).await
}

pub(crate) async fn template_upload(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, form: axum::extract::Multipart) -> Response {
    if let Err(response) = template_guard(&state, &headers, &id, true).await {
        return response;
    }
    upload_layout(form).await
}

async fn upload_layout(form: axum::extract::Multipart) -> Response {
    let upload = match read_layout_upload(form).await {
        Ok(files) => files,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    match interchange_import(&upload.document, upload.companion.as_deref(), upload.kit.as_ref(), &upload.linked) {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => json_error(StatusCode::UNPROCESSABLE_ENTITY, &error),
    }
}

fn interchange_import(document: &serde_json::Value, companion: Option<&str>, kit: Option<&serde_json::Value>, linked: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    elementor::import_layout_with_links(document, companion, kit, linked).map(|result| serde_json::json!({
        "layout": result.layout, "warnings": result.warnings, "elementor": document.get("content").is_some(),
    }))
}

async fn interchange(state: &AppState, body: &str, title: &str, kind: &str) -> Response {
    if body.len() > elementor::MAX_INTERCHANGE_BODY {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "This layout is too large.");
    }
    let Ok(input) = serde_json::from_str::<serde_json::Value>(body) else {
        return json_error(StatusCode::BAD_REQUEST, "The layout could not be read.");
    };
    let response = if let Some(document) = input.get("import") {
        if document.to_string().len() > elementor::MAX_UPLOAD_BYTES {
            return json_error(StatusCode::PAYLOAD_TOO_LARGE, "The layout JSON exceeds 64 MiB.");
        }
        if input.get("kitSettings").is_some_and(|kit| kit.to_string().len() > elementor::MAX_KIT_SETTINGS_BYTES) {
            return json_error(StatusCode::PAYLOAD_TOO_LARGE, "The kit settings file exceeds 1 MB.");
        }
        interchange_import(document, input["companion"].as_str(), input.get("kitSettings"), &[])
    } else {
        if body.len() > rules::spec::MAX_LAYOUT_BODY {
            return json_error(StatusCode::PAYLOAD_TOO_LARGE, "This layout is too large.");
        }
        let export_title = input["title"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(title);
        let export_title: String = export_title.trim().chars().take(200).collect();
        export_layout(state, &input["export"], &export_title, kind).await.map(|result| serde_json::json!({
            "document": result.document, "warnings": result.warnings,
        }))
    };
    match response {
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => json_error(StatusCode::UNPROCESSABLE_ENTITY, &error),
    }
}

fn parse_layout(body: &str) -> Result<(serde_json::Value, serde_json::Value), Response> {
    if body.len() > rules::spec::MAX_LAYOUT_BODY {
        return Err(json_error(StatusCode::PAYLOAD_TOO_LARGE, "The editable layout exceeds 12 MiB. Split the layout into smaller pages."));
    }
    let input: serde_json::Value = serde_json::from_str(body).map_err(|_| json_error(StatusCode::BAD_REQUEST, "The layout could not be read."))?;
    let layout = rules::normalise(&input["layout"]).map_err(|m| json_error(StatusCode::UNPROCESSABLE_ENTITY, &m))?;
    rules::spec::check_layout_size(&layout).map_err(|m| json_error(StatusCode::PAYLOAD_TOO_LARGE, &m))?;
    Ok((input, layout))
}

/// The canvas: the layout drawn for the editor and expanded as the site
/// would, for this post.
pub(crate) async fn editor_render(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (_, post) = match editor_guard(&state, &headers, &id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (_, layout) = match parse_layout(&body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let rel = vuuut_db::repos::posts::load_relations(&state.db, std::slice::from_ref(&post)).await.unwrap_or_default();
    let mut pj = vuuut_core::models::post_to_json(&post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
    let resolver = vuuut_front::resolver_now(&state).await;
    pj["url"] = serde_json::json!(resolver(&pj));
    vuuut_render::util::resolve_nested_urls(&mut pj, &*resolver);
    canvas_html(&state, &layout, &post.id, Some(&pj), post.html.as_deref()).await
}

pub(crate) async fn template_render(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (_, t) = match template_guard(&state, &headers, &id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (_, layout) = match parse_layout(&body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    canvas_html(&state, &layout, &t.id, None, None).await
}

async fn canvas_html(state: &AppState, layout: &serde_json::Value, scope: &str, post: Option<&serde_json::Value>, original: Option<&str>) -> Response {
    let s = builder_settings(state);
    let templates = repo::templates_map(&state.db).await;
    let html = rules::render_with(layout, &rules::Ctx { settings: &s, scope, editing: true, templates: &templates });
    let html = vuuut_front::builder_site::expand(state, &html, post, original.or(Some("<p>The post's own content appears here.</p>")), &serde_json::Value::Null, "/").await;
    axum::Json(serde_json::json!({"html": html})).into_response()
}

/// Save a layout. Body: `{"layout": {...}, "active": bool, "showTitle": bool?}`.
pub(crate) async fn editor_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (actor, post) = match editor_guard(&state, &headers, &id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (input, layout) = match parse_layout(&body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let active = input["active"].as_bool().unwrap_or(true);
    let templates = repo::templates_map(&state.db).await;
    let html = draw(&state, &layout, &post.id, &templates);
    if let Err(e) = repo::save(&state.db, &post.id, &layout.to_string(), &html, active, &actor.user.id).await {
        tracing::error!("builder: save failed: {}", e.message());
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Saving failed — check the logs.");
    }
    if let Some(show) = input["showTitle"].as_bool() {
        if show != post.show_title_and_feature_image {
            let changes = vuuut_db::repos::posts::PostChanges { show_title_and_feature_image: Some(show), ..Default::default() };
            if let Err(e) = vuuut_db::repos::posts::update(&state.db, &post.id, &changes).await {
                tracing::error!("builder: title switch failed: {}", e.message());
            }
        }
    }
    axum::Json(serde_json::json!({"ok": true, "savedAt": vuuut_core::dates::now_db(), "layout": layout, "active": active})).into_response()
}

/// Save a template from its own editor. Body adds `name`, `options`.
pub(crate) async fn template_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (_, t) = match template_guard(&state, &headers, &id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (input, layout) = match parse_layout(&body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let name: String = input["name"].as_str().unwrap_or(&t.name).trim().chars().take(120).collect();
    let name = if name.is_empty() { t.name.clone() } else { name };
    let opts = rules::parts::PartOptions::from_value(&input["options"]).to_value().to_string();
    let active = input["active"].as_bool().unwrap_or(t.active);
    let mut templates = repo::templates_map(&state.db).await;
    templates.insert(t.id.clone(), layout.clone());
    let html = draw(&state, &layout, &t.id, &templates);
    if let Err(e) = repo::update_template(&state.db, &t.id, &name, &layout.to_string(), &opts, &html, active).await {
        tracing::error!("builder: template save failed: {}", e.message());
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Saving failed — check the logs.");
    }
    // a global widget or embedded template changed: every page using it
    // draws again
    if !rules::is_theme_part(&t.kind) {
        rerender_all(&state).await;
    }
    axum::Json(serde_json::json!({"ok": true, "savedAt": vuuut_core::dates::now_db(), "layout": layout, "active": active})).into_response()
}

pub(crate) async fn editor_revisions(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let (_, post) = match editor_guard(&state, &headers, &id, false).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let list = repo::revisions(&state.db, &post.id).await.unwrap_or_default();
    let names: HashMap<String, String> = sqlx::query_as::<_, (String, String)>("SELECT id, name FROM users").fetch_all(&state.db).await.unwrap_or_default().into_iter().collect();
    axum::Json(serde_json::json!({
        "revisions": list.iter().map(|r| serde_json::json!({"id": r.id, "at": r.created_at, "by": names.get(&r.created_by).cloned().unwrap_or_default()})).collect::<Vec<_>>()
    }))
    .into_response()
}

pub(crate) async fn editor_revision(State(state): State<AppState>, headers: HeaderMap, Path((id, rid)): Path<(String, String)>) -> Response {
    let (_, post) = match editor_guard(&state, &headers, &id, false).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !valid_id(&rid) {
        return json_error(StatusCode::NOT_FOUND, "No such version.");
    }
    match repo::revision(&state.db, &post.id, &rid).await.ok().flatten().and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok()) {
        Some(v) => axum::Json(serde_json::json!({"layout": rules::normalise(&v).unwrap_or_else(|_| rules::blank())})).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "No such version."),
    }
}

/// Save a section, page or widget as a template from the post editor.
/// Body: `{"name": "...", "kind": "section"|"page"|"widget", "data": <layout>}`.
pub(crate) async fn editor_template_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: String) -> Response {
    let (actor, _) = match editor_guard(&state, &headers, &id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "The template could not be read.");
    };
    let name: String = input["name"].as_str().unwrap_or("").trim().chars().take(120).collect();
    if name.is_empty() {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "A template needs a name.");
    }
    let kind = match input["kind"].as_str() {
        Some("page") => "page",
        Some("widget") => "widget",
        _ => "section",
    };
    let data = match rules::normalise(&input["data"]) {
        Ok(l) => l,
        Err(msg) => return json_error(StatusCode::UNPROCESSABLE_ENTITY, &msg),
    };
    let id = match repo::create_template(&state.db, &name, kind, &data.to_string(), &actor.user.id).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("builder: template save failed: {}", e.message());
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Saving failed — check the logs.");
        }
    };
    let list = repo::templates(&state.db).await.unwrap_or_default();
    axum::Json(serde_json::json!({"ok": true, "id": id, "templates": templates_json(&list), "refs": list.iter().map(|t| serde_json::json!([t.id, t.name, t.kind])).collect::<Vec<_>>()})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_checked() {
        assert!(valid_id("65f1a2b3c4d5e6f7a8b9c0d1"));
        assert!(!valid_id("../etc"));
        assert!(!valid_id(""));
        assert!(!valid_id("a b"));
    }

    #[test]
    fn boot_json_cannot_close_its_script() {
        let v = serde_json::json!({"t": "</script><script>alert(1)</script><!--"});
        let s = script_json(&v);
        assert!(!s.contains("</script>"));
        assert!(!s.contains("<!--"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&s).unwrap(), v);
    }

    #[test]
    fn who_setting_gates_roles() {
        let mut s = Settings::default();
        assert!(role_may_build(&s, Role::Contributor));
        s.who = Who::Editors;
        assert!(!role_may_build(&s, Role::Author));
        assert!(role_may_build(&s, Role::Editor));
        s.who = Who::Administrators;
        assert!(!role_may_build(&s, Role::Editor));
        assert!(role_may_build(&s, Role::Administrator));
    }
}
