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

/// Where an entry lives on the public site, as a site-relative path that
/// carries its language prefix — `/de/hallo/` for a German post.
///
/// The router's permalinks are language-neutral; the language is a prefix
/// in front of them (see [`ghost_core::locales`]). Anything in the admin
/// that links to an entry has to add it, or a German post's "View post"
/// opens `/hallo/` — a 404, because the unprefixed path is the default
/// language's.
pub(crate) async fn public_path(state: &AppState, post: &ghost_core::models::Post) -> String {
    ghost_api::locales::entry_path(state, post).await
}

/// [`public_path`] as an absolute URL.
pub(crate) async fn public_url(state: &AppState, post: &ghost_core::models::Post) -> String {
    format!("{}{}", state.site_url().trim_end_matches('/'), public_path(state, post).await)
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
    title: &str,
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

    // --- the whole linked set ---------------------------------------------
    //
    // Every entry linked to this one, *and this one*, in the site's language
    // order. A set is one thing whichever member is open: the card reads the
    // same from the English post as from the Bengali one, and unlinking any
    // member from any other is the same operation.
    let existing = if siblings.is_empty() {
        r#"<p class="hint lg-note">Not linked to anything yet. Link an entry in another language below, from this side or from theirs — it is the same link.</p>"#.to_string()
    } else {
        let order = |code: &str| {
            locales.enabled().iter().position(|c| c.eq_ignore_ascii_case(code)).unwrap_or(usize::MAX)
        };
        let own_code = if current.is_empty() { locales.default_locale().to_string() } else { current.to_string() };
        // (code, row html)
        let mut rows: Vec<(String, String)> = vec![(
            own_code.clone(),
            format!(
                r#"<div class="lg-sib is-self"><span class="chip tonal">{code}</span>
<span class="lg-sib-title">{title}</span><span class="lg-sib-note">This {singular}</span></div>"#,
                code = esc(&own_code),
                title = esc(if title.trim().is_empty() { "(untitled)" } else { title.trim() }),
                singular = esc(singular),
            ),
        )];
        for s in siblings {
            let code = s.locale.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| locales.default_locale().to_string());
            let title = match editor_href(s) {
                Some(href) => format!(r#"<a href="{href}">{title}</a>"#, title = esc(sibling_title(s))),
                None => esc(sibling_title(s)),
            };
            let status = if s.status == "published" {
                String::new()
            } else {
                format!(r#"<span class="lg-sib-note">{}</span>"#, esc(&s.status))
            };
            rows.push((
                code.clone(),
                format!(
                    r#"<div class="lg-sib"><span class="chip tonal">{code}</span>
<span class="lg-sib-title">{title}</span>{status}
<button class="btn text small danger" type="submit" form="xlate-unlink-{id}">Unlink</button></div>"#,
                    code = esc(&code),
                    title = title,
                    status = status,
                    id = esc(&s.id),
                ),
            ));
        }
        rows.sort_by_key(|(code, _)| order(code));
        let total = rows.len();
        format!(
            r#"<p class="hint lg-note" style="margin:0 0 6px">{total} linked entries, one per language.</p><div class="lg-sibs">{rows}</div>"#,
            total = total,
            rows = rows.into_iter().map(|(_, r)| r).collect::<String>(),
        )
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

    // One more than is shown, to know whether there is more than is shown.
    let mut found = repos::locales::candidates(
        &state.db,
        &id,
        &post.post_type,
        &own,
        locales.default_locale(),
        term,
        SEARCH_LIMIT + 1,
    )
    .await
    .unwrap_or_default();
    let more = found.len() as i64 > SEARCH_LIMIT;
    found.truncate(SEARCH_LIMIT as usize);

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
    axum::Json(serde_json::json!({ "entries": items, "more": more })).into_response()
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
    // A published page slugged with a language's code is not lost: it holds
    // that language's home-page details (see `ghost_front::routes::
    // home_details`), so it is listed below rather than warned about.
    let shadowed: Vec<Sibling> = repos::locales::shadowed_slugs(&state.db, &locales)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !(s.post_type == "page" && s.status == "published"))
        .collect();
    let homes = home_pages_card(&state, &locales).await;
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
<span class="detail">Their slug is the same as a language code, so the prefix is read first and the entry cannot be opened. Change the slug — or, for a page, publish it to make it that language's home-page details.</span>
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
<div class="section">Language home pages</div>
{homes}
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

/// Which page each language's home page takes its details from: the page
/// slugged with the language's code, if there is one.
async fn home_pages_card(state: &AppState, locales: &Locales) -> String {
    if !locales.multilingual() {
        return String::new();
    }
    let mut rows = String::new();
    for code in locales.enabled() {
        let slug = code.to_ascii_lowercase();
        let home = locales.localise("/", code);
        let page = repos::posts::find_by_slug(&state.db, &slug, "page", false).await.ok().flatten();
        let detail = match &page {
            Some(p) if p.status == "published" => format!(
                r#"Title, description and share cards from <a href="/ghost/pages/{id}/">{title}</a>."#,
                id = esc(&p.id),
                title = esc(p.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("(untitled)")),
            ),
            Some(p) => format!(
                r#"<a href="/ghost/pages/{id}/">{title}</a> is a {status} — publish it to use it."#,
                id = esc(&p.id),
                title = esc(p.title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("(untitled)")),
                status = esc(&p.status),
            ),
            None => format!(
                r#"The site-wide details. To give it its own, publish a page with the slug <code>{slug}</code>."#,
                slug = esc(&slug),
            ),
        };
        rows.push_str(&format!(
            r#"<div class="row"><span class="ic tint-teal">{globe}</span>
<div class="body"><span class="title">{name} <span class="chip tonal">{home}</span></span>
<span class="detail">{detail}</span></div></div>"#,
            globe = svg("globe"),
            name = esc(&label_for(locales, code)),
            home = esc(&home),
            detail = detail,
        ));
    }
    format!(r#"<div class="card rows rise" style="--i:3">{rows}</div>"#)
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

    // Menus are edited on the Navigation page, which holds both menus for
    // every language; this only points there.
    let nav_cards: String = if locales.multilingual() {
        let chips: String = locales
            .enabled()
            .iter()
            .map(|code| {
                format!(
                    r##"<a class="chip tonal" href="/ghost/navigation/#nav-{anchor}">{name}</a> "##,
                    anchor = if locales.is_default(code) { "default".to_string() } else { esc(&code.to_ascii_lowercase()) },
                    name = esc(&ghost_core::locales::native_name(code)),
                )
            })
            .collect();
        format!(
            r#"<div class="card padded rise" style="--i:4">
<p class="muted" style="margin:0 0 10px">Every language has its own primary and secondary menu. A language you switch on here gets both, generated from the default language's menus and pointing at their translations. Edit them, and add the language switcher, in <a href="/ghost/navigation/">Navigation</a>.</p>
<p style="margin:0">{chips}</p></div>"#
        )
    } else {
        r#"<div class="card padded rise" style="--i:4"><p class="muted" style="margin:0">Add a language above to give it menus of its own.</p></div>"#.to_string()
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
<div class="section">Default publication language</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:center"><span class="ic tint-teal">{globe}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="locale" data-lg-default>{default_opts}</select><span>Default publication language</span>{car}</label>
<p class="muted" style="margin:6px 0 0">The language of the site's own address — no URL prefix — and of every entry not given one. Themes read it as <code>&lt;html lang&gt;</code>. Changing it keeps the current default as a language under its own prefix, with its posts and menus.</p></div></div>
</div>

<div class="section">Languages</div>
<div class="card padded rise" style="--i:2">
<p class="muted" style="margin:0 0 12px"><b>{default}</b> is the default language: no URL prefix, so nothing that works today changes. Every language you tick here is published under its own code — <code>/de/my-post/</code>.</p>
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
</form>
<script>
/* Picking a new default updates the boxes before saving: the new default is
   locked on, and the current one is shown as kept — the server keeps it as a
   language either way, so the page should not suggest otherwise. */
(function () {{
  var pick = document.querySelector('[data-lg-default]');
  if (!pick) {{ return; }}
  var saved = pick.value;
  pick.addEventListener('change', function () {{
    document.querySelectorAll('.lg-pick').forEach(function (label) {{
      var box = label.querySelector('input');
      var note = label.querySelector('em');
      var isNew = box.value === pick.value;
      var isOld = box.value === saved && !isNew;
      label.classList.toggle('is-default', isNew || isOld);
      if (isNew || isOld) {{ box.checked = true; }}
      box.disabled = isNew || isOld;
      if (!note && (isNew || isOld)) {{ note = document.createElement('em'); label.appendChild(note); }}
      if (note) {{ note.textContent = isNew ? 'default' : (isOld ? 'kept' : ''); if (!isNew && !isOld) {{ note.remove(); }} }}
    }});
  }});
}})();
</script>"#,
            head = crate::group_head(group, Some("languages-settings")),
            banner = flash_banner(&q),
            globe = svg("globe"),
            share = svg("share"),
            default = esc(&crate::settings::locale_label(locales.default_locale())),
            default_opts = crate::settings::select_options(crate::settings::LOCALES, locales.default_locale()),
            car = SELECT_CARET,
            boxes = boxes,
            hreflang_on = if hreflang { " checked" } else { "" },
            nav_cards = nav_cards,
        ),
        true,
    )
    .into_response()
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
    let mut hreflang = false;
    let mut wanted_default: Option<String> = None;
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
            "locale" => wanted_default = ghost_core::locales::normalise(&value),
            _ => {}
        }
    }

    let previous = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
    let previous = ghost_core::locales::normalise(&previous).unwrap_or_else(|| "en".into());

    // A new default publication language goes through the one path that
    // keeps everything else consistent: the old default stays a language,
    // its entries keep their language, the menus move with them and the
    // theme is reloaded (`ghost_api::locales::change_default`).
    let mut changed_default = false;
    if let Some(wanted) = wanted_default.filter(|w| *w != previous) {
        if let Err(e) = ghost_api::locales::change_default(&state, &wanted).await {
            tracing::error!("languages: changing the default language: {}", e.message());
            return crate::settings_flash("languages", "error", "The default language could not be changed — check the logs.");
        }
        changed_default = true;
        // Its checkbox was locked as the default, so it posted nothing; it
        // stays a language, as the page said it would.
        if !ticked.contains(&previous) {
            ticked.push(previous.clone());
        }
    }
    let default = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
    let default = ghost_core::locales::normalise(&default).unwrap_or_else(|| "en".into());
    // The default language is always published in; the checkbox for it is
    // disabled and so posts nothing.
    ticked.retain(|c| *c != default);
    let enabled = ticked.clone();

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
    // A language switched on gets both menus now, generated from the
    // default language's; one switched off loses its menus. Existing menus
    // are left alone — they are somebody's work.
    if let Err(e) = crate::navigation::ensure_locale_menus(&state).await {
        tracing::error!("languages: generating menus failed: {}", e.message());
        ok = false;
    }

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
    crate::settings_flash(
        "languages",
        "ok",
        if changed_default {
            "Language settings saved. The previous default is now a language under its own prefix."
        } else {
            "Language settings saved."
        },
    )
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
        assert_eq!(editor_panel(&one, "post", Some(ID), None, "This one", &[]), (String::new(), String::new()));
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
        let (panel, forms) = editor_panel(&locales, "post", Some(ID), None, "This one", &[sib(SIB_ID, Some("de"), "Hallo")],
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
        let (panel, _) = editor_panel(&locales, "post", Some(ID), None, "This one", &[sib(SIB_ID, Some("de"), "Hallo")],
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
        let (panel, _) = editor_panel(&locales, "post", Some(ID), None, "This one", &[]);
        assert!(panel.contains("data-xlate-search"), "{panel}");
        assert!(panel.contains("data-xlate-input"), "{panel}");
        // A plain text input named `target`, so a typed slug posts and the
        // server resolves it — not a hidden field only a script can fill.
        assert!(panel.contains(r#"<input name="target" form="xlate-link""#), "{panel}");
        assert!(!panel.contains(r#"<select name="target""#), "it should not be a dropdown: {panel}");
        // The script is only served where the panel is.
        assert!(editor_script(&locales).contains("translations-search.js"));
    }

    /// A linked set is one thing whichever member is open: the card lists
    /// every member, this one included, in the site's language order.
    #[test]
    fn the_card_lists_the_whole_set_in_language_order_from_any_member() {
        let locales = Locales::new("en", &["bn", "fr"]);
        const FR_ID: &str = "6553f7a1b2c3d4e5f6a7b8cb";
        let mut draft = sib(FR_ID, Some("fr"), "Bonjour");
        draft.status = "draft".into();
        // Opened from the Bengali member, with the English one stored as NULL.
        let (panel, _) = editor_panel(&locales, "post", Some(ID), Some("bn"), "Nomoskar", &[draft, sib(SIB_ID, None, "Hello")]);
        let at = |needle: &str| panel.find(needle).unwrap_or_else(|| panic!("{needle} missing:\n{panel}"));
        assert!(at("Hello") < at("Nomoskar") && at("Nomoskar") < at("Bonjour"), "not in site order:\n{panel}");
        assert!(panel.contains("3 linked entries"), "{panel}");
        assert!(panel.contains("This post"), "the open entry is not listed");
        assert!(panel.contains(">draft<"), "a draft member is not marked");
        // This entry cannot be unlinked from itself; the other two can.
        assert_eq!(panel.matches(">Unlink<").count(), 2, "{panel}");
        // Every language is taken, so nothing is offered to create.
        assert!(panel.contains("exists in every language"), "{panel}");
    }

    #[test]
    fn an_entry_in_every_language_is_told_so_rather_than_shown_an_empty_picker() {
        let locales = Locales::new("en", &["de"]);
        let (panel, _) = editor_panel(&locales, "post", Some(ID), None, "This one", &[sib(SIB_ID, Some("de"), "Hallo")],
        );
        assert!(panel.contains("exists in every language"), "{panel}");
        assert!(!panel.contains(r#"form="xlate-new""#), "{panel}");
    }

    #[test]
    fn an_unsaved_entry_is_asked_to_save_first() {
        // There is no id to hang a translation off yet.
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(&locales, "post", None, None, "This one", &[]);
        assert!(panel.contains("Save this post to start translating"), "{panel}");
        assert!(forms.is_empty(), "{forms}");
        // And an id that is not an ObjectID is treated the same, never linked.
        let (hostile, _) = editor_panel(&locales, "post", Some("../../settings"), None, "This one", &[]);
        assert!(!hostile.contains("../../settings"), "{hostile}");
    }

    #[test]
    fn a_hostile_title_is_escaped_in_the_panel() {
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(&locales, "post", Some(ID), None, "This one", &[sib(SIB_ID, Some("de"), r#"<img src=x onerror="alert(1)">"#)],
        );
        for html in [&panel, &forms] {
            assert!(!html.contains("<img"), "{html}");
            assert!(html.contains("&lt;img"), "{html}");
        }
    }

    #[test]
    fn a_page_panel_talks_about_pages_and_posts_to_the_pages_editor() {
        let locales = Locales::new("en", &["de"]);
        let (panel, forms) = editor_panel(&locales, "page", Some(ID), None, "This one", &[]);
        assert!(panel.contains("Language of this page"), "{panel}");
        assert!(forms.contains(&format!("/ghost/pages/{ID}/translations/new/")), "{forms}");
        assert!(!forms.contains("/ghost/posts/"), "{forms}");
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
