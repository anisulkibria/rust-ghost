//! The Multilingual module: publishing the same site in more than one
//! language.
//!
//! Three surfaces:
//!
//! * **`/ghost/languages/`** — what each language has, what is left to
//!   translate, and anything the setup has made unreachable.
//! * **Settings → Languages** — which languages the site publishes in, each
//!   one's navigation menu, and the `hreflang` switch.
//! * **A panel in the editor** ([`editor_panel`]) — this entry's language,
//!   the languages it already exists in, and the two buttons that change
//!   that. This is the "switch the same post to another language" that the
//!   module exists for.
//!
//! ## What a language actually is here
//!
//! One rule module, [`ghost_core::locales`], decides it for the whole
//! codebase: which codes are valid, which this site publishes in, and how a
//! URL says which one it is. The default language is unprefixed and the rest
//! carry their code (`/de/my-post/`), so **every URL an existing site
//! already has keeps working untouched** — there is nothing to redirect and
//! no ranking to re-earn. Enabling the module changes nothing a visitor can
//! see until a second language is added.
//!
//! ## There is no multilingual data
//!
//! Like the SEO module, this stores no report. Coverage and the untranslated
//! lists are computed from `posts` during the request that renders them, so
//! they cannot go stale or disagree with the editor. What it *does* store is
//! two things per entry — `posts.locale` and `posts.translation_group` — and
//! the handful of settings its own page offers.
//!
//! ## Who may use it
//!
//! Administrators, like every other module: the settings here change how the
//! whole site is addressed and indexed. The editor's panel follows the
//! editor's own permission ([`crate::may_edit_post`]), so whoever may edit an
//! entry may translate it.
//!
//! Every interpolated value goes through `esc`. Locale codes arriving from a
//! form are reduced to one this site publishes in before they reach a query,
//! a URL or an `hreflang` attribute — [`ghost_core::locales::normalise`] is
//! the trust boundary and refuses anything carrying a path separator.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use ghost_core::locales::Locales;
use ghost_db::repos;
use ghost_db::repos::locales::{LinkRefusal, Sibling};
use ghost_api::AppState;
use std::collections::HashMap;

use crate::{esc, flash_banner, page_for_role, require_role, svg, Actor, Role, SELECT_CARET};

/// The link search script, cache-busted by its own content.
///
/// A changed script is a new URL by construction, so it cannot stay stale in
/// an open browser — which is what a hand-bumped version number keeps
/// forgetting to do.
fn search_script_hash() -> &'static str {
    static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        crate::asset_hash(include_str!("../assets/translations-search.js"))
    });
    &HASH
}

/// The `<script>` tag for the editor page, emitted only when the panel is.
pub(crate) fn editor_script(locales: &Locales) -> String {
    if !locales.multilingual() {
        return String::new();
    }
    format!(
        r#"<script src="{assets}/translations-search.js?v={hash}" defer></script>"#,
        assets = crate::ASSETS,
        hash = search_script_hash(),
    )
}

/// The most entries the "left to translate" list shows per language.
const UNTRANSLATED_LIMIT: i64 = 25;
/// The most results the link search returns.
const SEARCH_LIMIT: i64 = 20;

async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Administrator).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

fn server_error(e: &ghost_core::GhostError) -> Response {
    tracing::error!("languages admin: {}", e.message());
    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong — check the logs.").into_response()
}

/// A code's label, with the site's default marked.
fn label_for(locales: &Locales, code: &str) -> String {
    let base = crate::settings::locale_label(code);
    if locales.is_default(code) {
        format!("{base} · default")
    } else {
        base
    }
}

/// Where an entry's editor is.
fn editor_href(sibling: &Sibling) -> Option<String> {
    ghost_core::ids::is_objectid(&sibling.id).then(|| {
        format!(
            "/ghost/{}s/{}/",
            if sibling.post_type == "page" { "page" } else { "post" },
            sibling.id
        )
    })
}

fn sibling_title(s: &Sibling) -> &str {
    s.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("(untitled)")
}

// ---------------------------------------------------------------------------
// the editor's panel — "this post, in another language"
// ---------------------------------------------------------------------------

/// The Language panel for the post/page editor, and the forms it drives.
///
/// Returns **two** fragments: the panel for the settings column, and the
/// forms that belong *outside* the editor's own `<form>`.
///
/// That split is not a style choice. The settings column lives inside
/// `<form id="editor-form">`, and HTML forbids nested forms — a browser drops
/// the inner one silently, so a button inside it submits the *editor* form
/// instead. The symptom is a "Create translation" button that appears to do
/// nothing (it saves the post). So every form here sits beside the editor
/// form and its controls reach it with `form="…"`, which is the same
/// arrangement the Delete button already uses.
///
/// Empty on a single-language site: a picker with one option reads as a
/// decision somebody has to make, and it is not one.
pub(crate) fn editor_panel(
    locales: &Locales,
    post_type: &str,
    post_id: Option<&str>,
    entry_locale: Option<&str>,
    siblings: &[Sibling],
) -> (String, String) {
    if !locales.multilingual() {
        return (String::new(), String::new());
    }
    let singular = if post_type == "page" { "page" } else { "post" };
    let base = if post_type == "page" { "pages" } else { "posts" };
    let current = entry_locale.unwrap_or("");

    // The entry's own language. This one *is* an editor-form field: it saves
    // with everything else rather than needing a button of its own.
    let picker = format!(
        r#"<div class="lg-field"><label class="field fixed" style="margin:0"><select name="locale">{options}</select><span>Language of this {singular}</span>{car}</label>
<p class="hint">Sets the URL it lives at. The default language has no prefix; the others carry their code — <code>/de/&lt;slug&gt;/</code>.</p></div>"#,
        options = crate::settings::locale_options(locales, current, None),
        singular = esc(singular),
        car = SELECT_CARET,
    );

    let Some(post_id) = post_id.filter(|id| ghost_core::ids::is_objectid(id)) else {
        // Nothing to translate until the first save gives it an id.
        return (
            crate::editor::panel(
                "globe",
                "teal",
                "Language",
                &format!(
                    r#"{picker}<p class="hint lg-note">Save this {singular} to start translating it.</p>"#,
                    singular = esc(singular)
                ),
                false,
            ),
            String::new(),
        );
    };

    // --- the forms, for outside the editor form ---------------------------
    let mut forms = String::new();
    for s in siblings {
        forms.push_str(&format!(
            r#"<form method="post" action="/ghost/{base}/{post_id}/translations/unlink/" id="xlate-unlink-{id}"
      data-confirm="Unlink this translation|&#8220;{plain}&#8221; stays exactly where it is. It just stops being listed as a translation of this {singular}."><input type="hidden" name="target" value="{id}"></form>"#,
            base = base,
            post_id = esc(post_id),
            id = esc(&s.id),
            plain = esc(sibling_title(s)),
            singular = esc(singular),
        ));
    }
    forms.push_str(&format!(
        r#"<form method="post" action="/ghost/{base}/{post_id}/translations/new/" id="xlate-new"></form>
<form method="post" action="/ghost/{base}/{post_id}/translations/link/" id="xlate-link"></form>"#,
        base = base,
        post_id = esc(post_id),
    ));

    // --- what already exists ---------------------------------------------
    let existing = if siblings.is_empty() {
        format!(
            r#"<p class="hint lg-note">No translations yet.</p>"#,
        )
    } else {
        let rows: String = siblings
            .iter()
            .map(|s| {
                let href = editor_href(s);
                let title = match &href {
                    Some(href) => format!(
                        r#"<a href="{href}">{title}</a>"#,
                        title = esc(sibling_title(s))
                    ),
                    None => esc(sibling_title(s)),
                };
                format!(
                    r#"<div class="lg-sib"><span class="chip tonal">{code}</span>
<span class="lg-sib-title">{title}</span>
<button class="btn text small danger" type="submit" form="xlate-unlink-{id}">Unlink</button></div>"#,
                    code = esc(&s.locale.clone().unwrap_or_else(|| locales.default_locale().to_string())),
                    title = title,
                    id = esc(&s.id),
                )
            })
            .collect();
        format!(r#"<div class="lg-sibs">{rows}</div>"#)
    };

    // --- the languages this entry does not have yet -----------------------
    let taken: Vec<String> = siblings
        .iter()
        .map(|s| s.locale.clone().unwrap_or_else(|| locales.default_locale().to_string()))
        .chain(std::iter::once(if current.is_empty() {
            locales.default_locale().to_string()
        } else {
            current.to_string()
        }))
        .collect();
    let free: Vec<&String> = locales
        .enabled()
        .iter()
        .filter(|code| !taken.iter().any(|t| t.eq_ignore_ascii_case(code)))
        .collect();

    let create = if free.is_empty() {
        format!(
            r#"<p class="hint lg-note">This {singular} exists in every language the site publishes in.</p>"#,
            singular = esc(singular)
        )
    } else {
        let options: String = free
            .iter()
            .map(|code| {
                format!(
                    r#"<option value="{code}">{label}</option>"#,
                    code = esc(code),
                    label = esc(&label_for(locales, code))
                )
            })
            .collect();
        format!(
            r#"<div class="lg-action">
<label class="field fixed" style="margin:0"><select name="locale" form="xlate-new">{options}</select><span>Translate into</span>{car}</label>
<button class="btn tonal" type="submit" form="xlate-new">{add} Create</button>
</div>
<p class="hint">Copies this {singular}'s text, image, tags and metadata into a new draft in that language, linked to this one. Nothing is published.</p>"#,
            options = options,
            car = SELECT_CARET,
            add = svg("add"),
            singular = esc(singular),
        )
    };

    // --- linking something that already exists ----------------------------
    //
    // A search box rather than a select: a site with a few hundred entries
    // makes a dropdown of candidates unusable, and the thing somebody is
    // looking for is almost always one they can name.
    let link = format!(
        r#"<div class="lg-action lg-search" data-xlate-search="/ghost/{base}/{post_id}/translations/search/">
<label class="field" style="margin:0"><input name="target" form="xlate-link" autocomplete="off" spellcheck="false"
  placeholder=" " data-xlate-input><span>Or link one that exists</span></label>
<button class="btn tonal" type="submit" form="xlate-link">{link_ic} Link</button>
<div class="lg-results" data-xlate-results hidden></div>
</div>
<p class="hint">Search by title, or type an exact slug. The {singular} you pick has to be in a different language from this one.</p>"#,
        base = base,
        post_id = esc(post_id),
        link_ic = svg("link"),
        singular = esc(singular),
    );

    let panel = crate::editor::panel(
        "globe",
        "teal",
        "Language",
        &format!(
            r#"{picker}
<div class="lg-group"><span class="lg-label">Translations</span>{existing}</div>
<div class="lg-group">{create}</div>
<div class="lg-group">{link}</div>"#
        ),
        false,
    );
    (panel, forms)
}

/// `GET …/translations/search/?q=` — candidate entries to link, as JSON.
///
/// Returns only entries that could actually be linked: a different language
/// from this one, the same kind, not this entry itself. Filtering here rather
/// than in the browser is what keeps the list honest — a result that
/// [`repos::locales::link`] would refuse should never be offered.
///
/// Capped at [`SEARCH_LIMIT`]; a search that matches everything is a search
/// nobody can use anyway.
pub async fn search(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) || !crate::may_edit_post(&state, &actor, &id).await {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let locales = crate::settings::site_locales(&state);
    let own = post.locale.clone().unwrap_or_else(|| locales.default_locale().to_string());
    let term = q.get("q").map(|t| t.trim()).unwrap_or("");

    let found = repos::locales::candidates(
        &state.db,
        &id,
        &post.post_type,
        &own,
        locales.default_locale(),
        term,
        SEARCH_LIMIT,
    )
    .await
    .unwrap_or_default();

    let items: Vec<serde_json::Value> = found
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "title": sibling_title(s),
                "slug": s.slug.clone().unwrap_or_default(),
                "locale": s.locale.clone().unwrap_or_else(|| locales.default_locale().to_string()),
                "status": s.status,
            })
        })
        .collect();
    axum::Json(serde_json::json!({ "entries": items })).into_response()
}

// ---------------------------------------------------------------------------
// the editor's translation actions
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct TargetForm {
    target: String,
}

#[derive(serde::Deserialize)]
pub struct LocaleForm {
    locale: String,
}

fn back_to(post_type: &str, id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    let base = if post_type == "page" { "pages" } else { "posts" };
    Redirect::to(&format!("/ghost/{base}/{id}/?{encoded}")).into_response()
}

/// Link an existing entry to this one as its translation.
pub async fn link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<TargetForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) {
        return (StatusCode::BAD_REQUEST, "Not an entry.").into_response();
    }
    if !crate::may_edit_post(&state, &actor, &id).await {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    // The box takes either an id — which the search script fills in when a
    // result is clicked — or a slug somebody typed. Supporting the slug is
    // what makes the box work with JavaScript off, rather than being a
    // dead input.
    let target = match resolve_target(&state, &form.target, &post.post_type).await {
        Some(target) => target,
        None => {
            return back_to(
                &post.post_type,
                &id,
                "error",
                "No entry of that name. Pick one from the search results, or type its exact slug.",
            )
        }
    };
    // Both sides are checked, because linking writes to both.
    if !crate::may_edit_post(&state, &actor, &target).await {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let locales = crate::settings::site_locales(&state);
    match repos::locales::link(&state.db, &id, &target, locales.default_locale()).await {
        Ok(Ok(_)) => back_to(&post.post_type, &id, "ok", "Linked as a translation."),
        Ok(Err(refusal)) => back_to(&post.post_type, &id, "error", &refusal_message(&refusal)),
        Err(e) => {
            tracing::error!("translation link: {}", e.message());
            back_to(&post.post_type, &id, "error", "That could not be linked — check the logs.")
        }
    }
}

/// An id or a slug, resolved to an entry id of the right kind.
///
/// Tried as an id first, because that is what the search script posts.
/// Anything else is looked up as a slug — bounded and compared as a whole
/// value, never spliced into SQL.
async fn resolve_target(state: &AppState, raw: &str, post_type: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().count() > 200 {
        return None;
    }
    if ghost_core::ids::is_objectid(raw) {
        return repos::posts::find_by_id(&state.db, raw).await.ok().flatten().map(|p| p.id);
    }
    let slug = raw.trim_matches('/');
    repos::posts::find_by_slug(&state.db, slug, post_type, false)
        .await
        .ok()
        .flatten()
        .map(|p| p.id)
}

/// Why a link was refused, in words the person who tried can act on.
fn refusal_message(refusal: &LinkRefusal) -> String {
    match refusal {
        LinkRefusal::Missing => "One of those entries no longer exists.".into(),
        LinkRefusal::Same => "An entry cannot be a translation of itself.".into(),
        LinkRefusal::SameLocale(code) => format!(
            "Both are in {}. A translation has to be in a different language.",
            crate::settings::locale_label(code)
        ),
        LinkRefusal::Occupied(code) => format!(
            "There is already a {} translation in this group. Unlink that one first.",
            crate::settings::locale_label(code)
        ),
        LinkRefusal::KindMismatch => "A post and a page cannot be translations of each other.".into(),
    }
}

pub async fn unlink(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<TargetForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) || !ghost_core::ids::is_objectid(&form.target) {
        return (StatusCode::BAD_REQUEST, "Not an entry.").into_response();
    }
    if !crate::may_edit_post(&state, &actor, &id).await {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    match repos::locales::unlink(&state.db, &form.target).await {
        Ok(()) => back_to(&post.post_type, &id, "ok", "No longer linked as a translation."),
        Err(e) => {
            tracing::error!("translation unlink: {}", e.message());
            back_to(&post.post_type, &id, "error", "That could not be unlinked — check the logs.")
        }
    }
}

/// Copy this entry into another language as a linked draft.
///
/// A copy rather than an empty entry: a translator works from the original,
/// and starting from a blank page means re-doing the feature image, the tags,
/// the excerpt and every SEO field by hand. It is created as a **draft** in
/// every case, including from a published original — publishing a translation
/// nobody has translated yet would put the source language on a URL that
/// claims to be another one.
pub async fn create(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<LocaleForm>,
) -> Response {
    let actor = match guard(&state, &headers, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !ghost_core::ids::is_objectid(&id) {
        return (StatusCode::BAD_REQUEST, "Not an entry.").into_response();
    }
    if !crate::may_edit_post(&state, &actor, &id).await {
        return crate::forbidden_page(&actor, Role::Editor);
    }
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let locales = crate::settings::site_locales(&state);
    // Only a language this site publishes in, and only one this entry does
    // not already have — the second check is `link`'s, which runs below.
    let Some(target) = ghost_core::locales::normalise(&form.locale).filter(|c| locales.is_enabled(c))
    else {
        return back_to(&post.post_type, &id, "error", "That is not a language this site publishes in.");
    };

    let copy = match repos::publishing::duplicate(&state.db, &id, &actor.user.id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("translation create: {}", e.message());
            return back_to(&post.post_type, &id, "error", "The translation could not be created — check the logs.");
        }
    };
    // `duplicate` titles the copy "… (Copy)" and drafts it, which is right
    // for a duplicate and wrong for a translation: name it for the language
    // instead, so the list does not fill with "(Copy)".
    let titled = format!(
        "{} [{}]",
        post.title.clone().unwrap_or_default(),
        target
    );
    let _ = repos::posts::update(
        &state.db,
        &copy.id,
        &repos::posts::PostChanges {
            title: Some(titled),
            locale: Some(target.clone()),
            explicit_save: true,
            editor_id: Some(actor.user.id.clone()),
            ..Default::default()
        },
    )
    .await;

    match repos::locales::link(&state.db, &id, &copy.id, locales.default_locale()).await {
        Ok(Ok(_)) => {
            let base = if copy.post_type == "page" { "pages" } else { "posts" };
            let encoded: String = form_urlencoded::Serializer::new(String::new())
                .append_pair(
                    "ok",
                    &format!(
                        "A {} draft, copied from the original. Translate it and publish when you are ready.",
                        crate::settings::locale_label(&target)
                    ),
                )
                .finish();
            Redirect::to(&format!("/ghost/{base}/{}/?{encoded}", copy.id)).into_response()
        }
        Ok(Err(refusal)) => back_to(&post.post_type, &id, "error", &refusal_message(&refusal)),
        Err(e) => {
            tracing::error!("translation create link: {}", e.message());
            back_to(&post.post_type, &id, "error", "The copy was made but could not be linked.")
        }
    }
}

// ---------------------------------------------------------------------------
// the module page
// ---------------------------------------------------------------------------

pub async fn languages_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let locales = crate::settings::site_locales(&state);
    let coverage: HashMap<String, repos::locales::Coverage> =
        match repos::locales::coverage(&state.db, locales.default_locale()).await {
            Ok(c) => c.into_iter().collect(),
            Err(e) => return server_error(&e),
        };

    let tiles = {
        let total: i64 = coverage.values().map(|c| c.posts + c.pages).sum();
        let drafts: i64 = coverage.values().map(|c| c.drafts).sum();
        format!(
            "{}{}{}{}",
            tile("Languages", &locales.enabled().len().to_string(), "Including the default"),
            tile("Published", &total.to_string(), "Posts and pages, every language"),
            tile("Drafts", &drafts.to_string(), "Not yet live in any language"),
            tile(
                "URL shape",
                if locales.multilingual() { "Prefixed" } else { "Single" },
                if locales.multilingual() {
                    "The default language has no prefix"
                } else {
                    "Add a language in settings to begin"
                },
            ),
        )
    };

    // One row per language: what it has, and how to see what it is missing.
    let rows: String = locales
        .enabled()
        .iter()
        .map(|code| {
            let c = coverage.get(code).cloned().unwrap_or_default();
            let prefix = locales.prefix(code);
            format!(
                r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{label}</span>
<span class="detail">{posts} · {pages} · {drafts}{tags} — {url}</span></div>
<div class="trailing">{chip}<a class="btn tonal small" href="{site}{prefix}/" target="_blank" rel="noopener">View</a></div></div>"#,
                tint = if locales.is_default(code) { "teal" } else { "blue" },
                ic = svg("globe"),
                label = esc(&label_for(&locales, code)),
                posts = esc(&crate::countable(c.posts, "post")),
                pages = esc(&crate::countable(c.pages, "page")),
                drafts = esc(&format!("{} draft", c.drafts).replace("1 drafts", "1 draft")),
                tags = if c.tags > 0 {
                    format!(" · {}", esc(&crate::countable(c.tags, "tag")))
                } else {
                    String::new()
                },
                url = if prefix.is_empty() {
                    "no prefix".to_string()
                } else {
                    format!("<code>{}/</code>", esc(&prefix))
                },
                chip = if locales.is_default(code) {
                    r#"<span class="chip published">default</span>"#
                } else {
                    r#"<span class="chip tonal">translation</span>"#
                },
                site = esc(state.site_url().trim_end_matches('/')),
                prefix = esc(&prefix),
            )
        })
        .collect();

    // What is left to translate, per language.
    let mut todo = String::new();
    if locales.multilingual() {
        for code in locales.enabled() {
            if locales.is_default(code) {
                continue;
            }
            let missing = repos::locales::untranslated(
                &state.db,
                code,
                locales.default_locale(),
                UNTRANSLATED_LIMIT,
            )
            .await
            .unwrap_or_default();
            if missing.is_empty() {
                todo.push_str(&format!(
                    r#"<div class="row"><span class="ic tint-green">{check}</span>
<div class="body"><span class="title">{label}</span>
<span class="detail">Everything published in {default} has a translation.</span></div></div>"#,
                    check = svg("check"),
                    label = esc(&crate::settings::locale_label(code)),
                    default = esc(&crate::settings::locale_label(locales.default_locale())),
                ));
                continue;
            }
            let items: String = missing
                .iter()
                .map(|s| match editor_href(s) {
                    Some(href) => format!(
                        r#"<a class="chip tonal" href="{href}">{title}</a>"#,
                        title = esc(sibling_title(s))
                    ),
                    None => format!(r#"<span class="chip tonal">{}</span>"#, esc(sibling_title(s))),
                })
                .collect();
            todo.push_str(&format!(
                r#"<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{ic}</span>
<div class="body"><span class="title">{label}</span>
<span class="detail">{n} with no {label} translation. Open one and use its Language panel.</span>
<div class="chips" style="margin-top:8px">{items}</div></div></div>"#,
                ic = svg("warning"),
                label = esc(&crate::settings::locale_label(code)),
                n = esc(&crate::countable(missing.len() as i64, "entry").replace("entrys", "entries")),
                items = items,
            ));
        }
    }
    let todo_card = if todo.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:4;text-align:center">
<p style="margin:10px 0 4px;font-size:17px;font-weight:600">One language</p>
<p class="muted" style="margin:0 0 12px">This site publishes in {label} only. Add a language in settings and this becomes a translation checklist.</p>
<a class="btn tonal" href="/ghost/settings/languages/">{gear} Add a language</a></div>"#,
            label = esc(&crate::settings::locale_label(locales.default_locale())),
            gear = svg("settings"),
        )
    } else {
        format!(r#"<div class="card rows rise" style="--i:4">{todo}</div>"#)
    };

    // Slugs a language prefix has made unreachable — the one way this URL
    // shape can bite, so it is reported rather than left to be found.
    let shadowed = repos::locales::shadowed_slugs(&state.db, &locales).await.unwrap_or_default();
    let warning = if shadowed.is_empty() {
        String::new()
    } else {
        let items: String = shadowed
            .iter()
            .map(|s| match editor_href(s) {
                Some(href) => format!(
                    r#"<a class="chip error" href="{href}">/{slug}/ — {title}</a>"#,
                    slug = esc(s.slug.as_deref().unwrap_or("")),
                    title = esc(sibling_title(s))
                ),
                None => format!(r#"<span class="chip error">/{}/</span>"#, esc(s.slug.as_deref().unwrap_or(""))),
            })
            .collect();
        format!(
            r#"<div class="section">Unreachable</div>
<div class="card rows rise" style="--i:5"><div class="row" style="align-items:flex-start">
<span class="ic tint-red">{ic}</span>
<div class="body"><span class="title">A language prefix is shadowing these</span>
<span class="detail">Their slug is the same as a language code, so the prefix is read first and the entry cannot be opened. Change the slug.</span>
<div class="chips" style="margin-top:8px">{items}</div></div></div></div>"#,
            ic = svg("warning"),
            items = items,
        )
    };

    page_for_role(
        actor.role,
        "Languages",
        "languages",
        Some(&actor.user),
        &format!(
            r#"{banner}<header class="page-head rise"><div><h1>Languages</h1>
<p class="sub">Publishing this site in more than one language.</p></div>
<div class="actions"><a class="btn tonal" href="/ghost/settings/languages/">{gear} Language settings</a></div></header>
<div class="tiles rise" style="--i:1">{tiles}</div>
<div class="section">Languages</div>
<div class="card rows rise" style="--i:2">{rows}</div>
<div class="section">Left to translate</div>
{todo_card}
{warning}
<p class="muted rise" style="--i:6;margin:16px 0 0">Nothing here is stored: every count is read from your content as this page loads.</p>"#,
            banner = flash_banner(&q),
            gear = svg("settings"),
        ),
        true,
    )
    .into_response()
}

fn tile(label: &str, value: &str, detail: &str) -> String {
    format!(
        r#"<div class="tile"><div class="label">{label}</div><div class="value">{value}</div>
<div class="delta">{detail}</div></div>"#,
        label = esc(label),
        value = esc(value),
        detail = esc(detail),
    )
}

// ---------------------------------------------------------------------------
// settings
// ---------------------------------------------------------------------------

pub async fn settings_page(
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
        .find(|g| g.slug == "languages")
        .expect("languages settings group");
    let locales = crate::settings::site_locales(&state);
    let hreflang = ghost_core::locales::hreflang_enabled(&serde_json::json!({
        "locale_hreflang": state.settings.get("locale_hreflang")
    }));

    // Every language the site could publish in, ticked where it does. The
    // default is shown locked: it is the publication language, changed in
    // General settings, and unticking it here would be meaningless.
    let boxes: String = crate::settings::LOCALES
        .iter()
        .map(|(code, label)| {
            let is_default = locales.is_default(code);
            let on = locales.is_enabled(code);
            format!(
                r#"<label class="lg-pick{cls}"><input type="checkbox" name="locales" value="{code}"{ck}{dis}><span>{label}</span>{note}</label>"#,
                cls = if is_default { " is-default" } else { "" },
                code = esc(code),
                ck = if on { " checked" } else { "" },
                dis = if is_default { " disabled" } else { "" },
                label = esc(label),
                note = if is_default {
                    r#"<em>default</em>"#
                } else {
                    ""
                },
            )
        })
        .collect();

    // One navigation box per non-default language, pre-filled with what it
    // has or with the site-wide menu as a starting point.
    let nav_maps = |key: &str| -> serde_json::Value {
        state
            .settings
            .get(key)
            .and_then(|v| match v {
                serde_json::Value::Object(o) => Some(serde_json::Value::Object(o)),
                serde_json::Value::String(raw) => serde_json::from_str(&raw).ok(),
                _ => None,
            })
            .unwrap_or_else(|| serde_json::json!({}))
    };
    let primary = nav_maps("navigation_locales");
    let secondary = nav_maps("secondary_navigation_locales");
    let as_lines = |map: &serde_json::Value, code: &str| -> String {
        map.get(code)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| {
                        let label = i.get("label").and_then(|v| v.as_str())?;
                        let url = i.get("url").and_then(|v| v.as_str()).unwrap_or("/");
                        Some(format!("{label} | {url}"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };

    let nav_cards: String = if locales.multilingual() {
        locales
            .enabled()
            .iter()
            .filter(|code| !locales.is_default(code))
            .map(|code| {
                format!(
                    r#"<div class="card padded rise" style="--i:4">
<p style="margin:0 0 10px;font-weight:600">{label}</p>
<label class="field" style="margin:0 0 10px"><textarea name="nav_{code}" rows="4" placeholder=" " spellcheck="false" class="lg-nav">{primary}</textarea><span>Primary menu</span></label>
<label class="field" style="margin:0"><textarea name="nav2_{code}" rows="3" placeholder=" " spellcheck="false" class="lg-nav">{secondary}</textarea><span>Secondary menu</span></label>
<p class="muted" style="margin:8px 0 0">One item per line, as <code>Label | /url/</code>. Leave a box empty to use the site-wide menu from <a href="/ghost/settings/navigation/">Navigation</a>. A relative URL is served under this language's prefix automatically.</p>
</div>"#,
                    label = esc(&crate::settings::locale_label(code)),
                    code = esc(code),
                    primary = esc(&as_lines(&primary, code)),
                    secondary = esc(&as_lines(&secondary, code)),
                )
            })
            .collect()
    } else {
        r#"<div class="card padded rise" style="--i:4"><p class="muted" style="margin:0">Add a language above to give it a menu of its own.</p></div>"#.to_string()
    };

    page_for_role(
        actor.role,
        "Languages",
        "settings",
        Some(&actor.user),
        &format!(
            r#"{head}{banner}
<p class="rise" style="--i:1"><a class="btn tonal" href="/ghost/languages/">{globe} Open the Languages report</a></p>
<form method="post" action="/ghost/settings/languages/" id="languages-settings">
<div class="section">Languages</div>
<div class="card padded rise" style="--i:2">
<p class="muted" style="margin:0 0 12px">The <b>{default}</b> is this site's default language, set in <a href="/ghost/settings/general/">General settings</a>. It has no URL prefix, so nothing that works today changes. Every language you tick here is published under its own code — <code>/de/my-post/</code>.</p>
<div class="lg-picks">{boxes}</div>
</div>

<div class="section">Search engines</div>
<div class="card rows rise" style="--i:3">
<div class="row"><span class="ic tint-green">{share}</span>
<div class="body"><span class="title">Publish <code>hreflang</code> alternates</span>
<span class="detail">Tells a search engine that two URLs are one page in two languages rather than duplicates</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="locale_hreflang" value="1"{hreflang_on} form="languages-settings"><span class="track"><span class="thumb"></span></span></label></div></div>
</div>

<div class="section">Navigation per language</div>
{nav_cards}
</form>"#,
            head = crate::group_head(group, Some("languages-settings")),
            banner = flash_banner(&q),
            globe = svg("globe"),
            share = svg("share"),
            default = esc(&crate::settings::locale_label(locales.default_locale())),
            boxes = boxes,
            hreflang_on = if hreflang { " checked" } else { "" },
            nav_cards = nav_cards,
        ),
        true,
    )
    .into_response()
}

/// `Label | /url/` lines into the navigation shape the settings table holds.
///
/// Rebuilt rather than stored as sent: the label is bounded and stripped of
/// control characters, and a URL is kept only if it is relative or plainly
/// HTTP(S) — a `javascript:` menu item would run on every page of the site.
pub(crate) fn parse_nav(raw: &str) -> Vec<serde_json::Value> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (label, url) = match line.split_once('|') {
                Some((l, u)) => (l.trim(), u.trim()),
                None => (line, "/"),
            };
            let label: String = label.chars().filter(|c| !c.is_control()).take(200).collect();
            if label.trim().is_empty() {
                return None;
            }
            let url = url.chars().filter(|c| !c.is_control()).take(2000).collect::<String>();
            let url = url.trim();
            let safe = url.starts_with('/')
                || url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with('#');
            // A protocol-relative URL leaves the site, and a `javascript:`
            // one runs on every page that renders the menu.
            let safe = safe && !url.starts_with("//");
            Some(serde_json::json!({
                "label": label.trim(),
                "url": if safe { url } else { "/" },
            }))
        })
        .collect()
}

pub async fn settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(r) = guard(&state, &headers, true).await {
        return r;
    }
    // Parsed by hand: `locales` posts once per ticked box, and
    // `serde_urlencoded` collapses a repeated key to one value.
    let mut ticked: Vec<String> = Vec::new();
    let mut navs: HashMap<String, String> = HashMap::new();
    let mut hreflang = false;
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "locales" => {
                if let Some(code) = ghost_core::locales::normalise(&value) {
                    if !ticked.contains(&code) {
                        ticked.push(code);
                    }
                }
            }
            "locale_hreflang" => hreflang = value == "1",
            k if k.starts_with("nav_") || k.starts_with("nav2_") => {
                navs.insert(k.to_string(), value.into_owned());
            }
            _ => {}
        }
    }

    let default = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
    let default = ghost_core::locales::normalise(&default).unwrap_or_else(|| "en".into());
    // The default language is always published in; the checkbox for it is
    // disabled and so posts nothing.
    ticked.retain(|c| *c != default);
    let enabled = ticked.clone();
    let locales = Locales::from_settings(Some(&default), Some(&serde_json::json!(enabled)));

    // Only the languages that survived, so a menu cannot be stored for a
    // language the site does not publish in.
    let mut primary = serde_json::Map::new();
    let mut secondary = serde_json::Map::new();
    for code in locales.enabled() {
        if let Some(raw) = navs.get(&format!("nav_{code}")) {
            let items = parse_nav(raw);
            if !items.is_empty() {
                primary.insert(code.clone(), serde_json::json!(items));
            }
        }
        if let Some(raw) = navs.get(&format!("nav2_{code}")) {
            let items = parse_nav(raw);
            if !items.is_empty() {
                secondary.insert(code.clone(), serde_json::json!(items));
            }
        }
    }

    let mut ok = state
        .settings
        .set(&state.db, "locales_enabled", &serde_json::json!(enabled))
        .await
        .is_ok();
    ok &= state
        .settings
        .set(&state.db, "locale_hreflang", &serde_json::Value::Bool(hreflang))
        .await
        .is_ok();
    ok &= state
        .settings
        .set(&state.db, "navigation_locales", &serde_json::Value::Object(primary))
        .await
        .is_ok();
    ok &= state
        .settings
        .set(&state.db, "secondary_navigation_locales", &serde_json::Value::Object(secondary))
        .await
        .is_ok();

    if !ok {
        return crate::settings_flash("languages", "error", "Saving failed — check the logs.");
    }
    // The theme's compiled registry holds one locale file per language, so a
    // language added here is not renderable until the theme is rebuilt. Done
    // now rather than left for the next restart, which is when somebody
    // would otherwise notice their new language rendering in English.
    if let Err(e) = ghost_api::themes::reactivate(&state).await {
        tracing::error!("languages: theme reload failed: {}", e.message());
        return crate::settings_flash(
            "languages",
            "error",
            "Languages saved, but the theme could not be reloaded — restart to pick up its locale files.",
        );
    }
    crate::settings_flash("languages", "ok", "Language settings saved.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sib(id: &str, locale: Option<&str>, title: &str) -> Sibling {
        Sibling {
            id: id.into(),
            title: Some(title.into()),
            slug: Some("slug".into()),
            locale: locale.map(str::to_string),
            status: "published".into(),
            post_type: "post".into(),
            updated_at: None,
        }
    }

    const ID: &str = "6553f7a1b2c3d4e5f6a7b8c9";
    /// A second real ObjectID: `editor_href` only links an id of this shape,
    /// so a short fixture id would silently test the unlinked path.
    const SIB_ID: &str = "6553f7a1b2c3d4e5f6a7b8ca";

    #[test]
    fn a_single_language_site_gets_no_panel_at_all() {
        // Installing the module must change nothing until a second language
        // is added: a picker with one option is a decision that is not one.
        let one = Locales::default();
        assert_eq!(editor_panel(&one, "post", Some(ID), None, &[]), (String::new(), String::new()));
        assert_eq!(editor_script(&one), "");
    }

    /// The bug this panel actually had. The settings column lives inside
    /// `<form id="editor-form">` and HTML forbids nested forms — a browser
    /// drops the inner one, so every button in the panel silently submitted
    /// the editor instead. "Create translation" appeared to do nothing.
    ///
    /// So: no `<form` may appear in the panel fragment, and every control
    /// that submits has to name its form by id.
    #[test]
    fn the_panel_contains_no_form_of_its_own() {
        let locales = Locales::new("en", &["de", "fr"]);
        let (panel, forms) = editor_panel(
            &locales,
            "post",
            Some(ID),
            None,
            &[sib(SIB_ID, Some("de"), "Hallo")],
        );
        assert!(!panel.contains("<form"), "a nested form is dropped by the browser:\n{panel}");
        // Every submit in the panel points at a form in the other fragment.
        for id in ["xlate-new", "xlate-link", &format!("xlate-unlink-{SIB_ID}")] {
            assert!(panel.contains(&format!(r#"form="{id}""#)), "{id} is not referenced: {panel}");
            assert!(forms.contains(&format!(r#"id="{id}""#)), "{id} has no form: {forms}");
        }
        // And those forms post somewhere real.
        assert!(forms.contains(&format!("/ghost/posts/{ID}/translations/new/")), "{forms}");
        assert!(forms.contains(&format!("/ghost/posts/{ID}/translations/link/")), "{forms}");
        assert!(forms.contains(&format!("/ghost/posts/{ID}/translations/unlink/")), "{forms}");
    }

    #[test]
    fn the_panel_offers_only_the_languages_the_entry_does_not_have() {
        let locales = Locales::new("en", &["de", "fr"]);
        // An English entry already translated into German: only French left.
        let (panel, _) = editor_panel(
            &locales,
            "post",
            Some(ID),
            None,
            &[sib(SIB_ID, Some("de"), "Hallo")],
        );
        // Scoped to the "translate into" select: the entry's *own* language
        // picker correctly lists every language, because the entry can be
        // set to any of them.
        let create = panel.split(r#"<select name="locale" form="xlate-new">"#).nth(1).expect("the create select");
        let create = create.split("</select>").next().unwrap();
        assert!(create.contains(r#"<option value="fr">"#), "{create}");
        assert!(!create.contains(r#"<option value="de">"#), "German is taken: {create}");
        assert!(!create.contains(r#"<option value="en">"#), "English is the entry itself: {create}");
        // The sibling is listed, openable, and unlinkable.
        assert!(panel.contains("Hallo"));
        assert!(panel.contains(&format!("/ghost/posts/{SIB_ID}/")), "{panel}");
    }

    /// Linking is a search box, not a dropdown: a site with a few hundred
    /// entries makes a `<select>` of candidates unusable.
    #[test]
    fn linking_is_a_search_box_that_works_without_javascript() {
        let locales = Locales::new("en", &["de"]);
        let (panel, _) = editor_panel(&locales, "post", Some(ID), None, &[]);
        assert!(panel.contains("data-xlate-search"), "{panel}");
        assert!(panel.contains("data-xlate-input"), "{panel}");
        // A plain text input named `target`, so a typed slug posts and the
        // server resolves it — not a hidden field only a script can fill.
        assert!(panel.contains(r#"<input name="target" form="xlate-link""#), "{panel}");
        assert!(!panel.contains(r#"<select name="target""#), "it should not be a dropdown: {panel}");
        // The script is only served where the panel is.
        assert!(editor_script(&locales).contains("translations-search.js"));
    }

    #[test]
    fn an_entry_in_every_language_is_told_so_rather_than_shown_an_empty_picker() {
        let locales = Locales::new("en", &["de"]);
        let (panel, _) = editor_panel(
            &locales,
            "post",
            Some(ID),
            None,
            &[sib(SIB_ID, Some("de"), "Hallo")],
        );
        assert!(panel.contains("exists in every language"), "{panel}");
        assert!(!panel.contains(r#"form="xlate-new""#), "{panel}");
    }

    #[test]
    fn an_unsaved_entry_is_asked_to_save_first() {
        // There is no id to hang a translation off yet.
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(&locales, "post", None, None, &[]);
        assert!(panel.contains("Save this post to start translating"), "{panel}");
        assert!(forms.is_empty(), "{forms}");
        // And an id that is not an ObjectID is treated the same, never linked.
        let (hostile, _) = editor_panel(&locales, "post", Some("../../settings"), None, &[]);
        assert!(!hostile.contains("../../settings"), "{hostile}");
    }

    #[test]
    fn a_hostile_title_is_escaped_in_the_panel() {
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(
            &locales,
            "post",
            Some(ID),
            None,
            &[sib(SIB_ID, Some("de"), r#"<img src=x onerror="alert(1)">"#)],
        );
        for html in [&panel, &forms] {
            assert!(!html.contains("<img"), "{html}");
            assert!(html.contains("&lt;img"), "{html}");
        }
    }

    #[test]
    fn a_page_panel_talks_about_pages_and_posts_to_the_pages_editor() {
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(&locales, "page", Some(ID), None, &[]);
        assert!(panel.contains("Language of this page"), "{panel}");
        assert!(forms.contains(&format!("/ghost/pages/{ID}/translations/new/")), "{forms}");
        assert!(!forms.contains("/ghost/posts/"), "{forms}");
    }

    #[test]
    fn a_menu_line_that_would_run_script_is_refused() {
        // The menu renders on every page of the site, so a `javascript:` URL
        // in it would run everywhere.
        let items = parse_nav("Home | /\nEvil | javascript:alert(1)\nOff-site | //evil.example/x");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["url"], "/");
        assert_eq!(items[1]["url"], "/", "a javascript: URL is replaced, not stored");
        assert_eq!(items[2]["url"], "/", "a protocol-relative URL leaves the site");
        // An ordinary external link is kept.
        let ext = parse_nav("Docs | https://example.com/docs");
        assert_eq!(ext[0]["url"], "https://example.com/docs");
    }

    #[test]
    fn menu_lines_are_rebuilt_rather_than_stored_as_sent() {
        let items = parse_nav("  Home  |  /  \n\n\nAbout|/about/\nBare\n   \n");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], serde_json::json!({"label": "Home", "url": "/"}));
        assert_eq!(items[1], serde_json::json!({"label": "About", "url": "/about/"}));
        // A line with no separator is a label pointing at the home page.
        assert_eq!(items[2], serde_json::json!({"label": "Bare", "url": "/"}));
        // Control characters cannot survive into a label.
        let dirty = parse_nav("Ho\u{0}me | /");
        assert_eq!(dirty[0]["label"], "Home");
        // A line that is only a separator contributes nothing.
        assert!(parse_nav(" | /x/").is_empty());
    }

    #[test]
    fn a_refusal_says_which_language_is_in_the_way() {
        let occupied = refusal_message(&LinkRefusal::Occupied("de".into()));
        assert!(occupied.contains("German"), "{occupied}");
        assert!(occupied.contains("Unlink"), "{occupied}");
        let same = refusal_message(&LinkRefusal::SameLocale("en".into()));
        assert!(same.contains("English"), "{same}");
        assert!(refusal_message(&LinkRefusal::KindMismatch).contains("page"));
    }
}
