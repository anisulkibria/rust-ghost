//! Settings -> Navigation: the primary and secondary menus, as ordered
//! label/URL rows — one pair per language the site publishes in.
//!
//! ## Where each language's menus are stored
//!
//! The default language keeps Ghost's own `navigation` and
//! `secondary_navigation` settings, so a single-language site, the Admin API
//! and every Ghost client see exactly what they always did. Every other
//! language's menus live in the Multilingual module's two maps,
//! `navigation_locales` and `secondary_navigation_locales` (locale → menu),
//! and the front end swaps the right one into `@site.navigation` for each
//! request (`ghost_render::context::build_site_for`). A theme does not know
//! there is more than one menu.
//!
//! ## Generated, then owned
//!
//! When a language is switched on, its menus are generated from the default
//! language's ([`ensure_locale_menus`]): each item that names a post or page
//! with a published translation in that language is pointed at the
//! translation, under its title; archives and other site paths get the
//! language prefix; external links are kept. After that they are ordinary
//! menus — editing the default menu does not rewrite them, because that
//! would throw away whatever somebody had translated by hand. The
//! "Regenerate" button on a language rebuilds them on request.
//!
//! ## The language switcher
//!
//! A row whose URL is [`ghost_core::locales::SWITCHER_URL`]. The front end
//! replaces it with one link per language, pointing at the page being read
//! in that language, so a theme draws a switcher without knowing it is one.

use super::*;
use ghost_core::locales::{self, Locales};
use std::collections::BTreeMap;

/// One menu entry (`{"label": …, "url": …}` in the stored JSON array).
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct NavItem {
    pub(crate) label: String,
    pub(crate) url: String,
}

impl NavItem {
    fn is_switcher(&self) -> bool {
        locales::is_switcher(&self.url)
    }
}

/// Cap on menu length so `add` can't run away; Ghost has no fixed limit but
/// menus are single-digit in practice.
pub(crate) const NAV_MAX_ITEMS: usize = 50;

/// What the switcher row is called in the stored menu. Never shown on the
/// site — the front end replaces the row with the languages' own names — but
/// the Admin API and any client that does not know the marker see it.
const SWITCHER_LABEL: &str = "Language";

/// The two menus of one non-default language.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct LocaleMenus {
    pub(crate) primary: Vec<NavItem>,
    pub(crate) secondary: Vec<NavItem>,
}

/// The posted navigation form. Every row posts a `…_label` and a `…_url`
/// input under the same names, so this needs the same hand-rolled parsing as
/// the editor's tag checkboxes (`axum::Form` would collapse the repeats).
///
/// A row's *kind* says which menu it belongs to: `primary` and `secondary`
/// for the default language, `primary.de` and `secondary.de` for German.
#[derive(Default, Debug)]
pub(crate) struct NavForm {
    pub(crate) primary: Vec<NavItem>,
    pub(crate) secondary: Vec<NavItem>,
    /// Every other language's menus, by canonical code.
    pub(crate) locales: BTreeMap<String, LocaleMenus>,
    /// Which submit button was pressed: `save`, `add-{kind}`,
    /// `switcher-{kind}`, `regen-{code}` or `{up|down|remove}-{kind}-{index}`.
    pub(crate) action: String,
}

/// `primary` / `secondary.de` → (is secondary, language code if any).
///
/// The code has to be in canonical form already, which is how this page
/// writes it: anything else is not a field this page rendered.
fn parse_kind(kind: &str) -> Option<(bool, Option<String>)> {
    let (menu, code) = match kind.split_once('.') {
        Some((menu, code)) => (menu, Some(code)),
        None => (kind, None),
    };
    let secondary = match menu {
        "primary" => false,
        "secondary" => true,
        _ => return None,
    };
    match code {
        None => Some((secondary, None)),
        Some(code) => {
            let canonical = locales::normalise(code)?;
            (canonical == code).then_some((secondary, Some(canonical)))
        }
    }
}

fn kind_for(secondary: bool, code: Option<&str>) -> String {
    let menu = if secondary { "secondary" } else { "primary" };
    match code {
        Some(code) => format!("{menu}.{code}"),
        None => menu.to_string(),
    }
}

impl NavForm {
    /// Labels and URLs arrive in row order, so the i-th of each belongs to
    /// the same row — zip them (padding with blanks if counts disagree).
    pub(crate) fn parse(body: &str) -> Self {
        // kind → (labels, urls)
        let mut fields: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
        let mut form = NavForm::default();
        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            if key == "action" {
                form.action = value.into_owned();
                continue;
            }
            let (kind, is_label) = if let Some(kind) = key.strip_suffix("_label") {
                (kind, true)
            } else if let Some(kind) = key.strip_suffix("_url") {
                (kind, false)
            } else {
                continue;
            };
            if parse_kind(kind).is_none() {
                continue;
            }
            let entry = fields.entry(kind.to_string()).or_default();
            if is_label {
                entry.0.push(value.into_owned());
            } else {
                entry.1.push(value.into_owned());
            }
        }
        for (kind, (labels, urls)) in fields {
            let items: Vec<NavItem> = (0..labels.len().max(urls.len()))
                .map(|i| NavItem {
                    label: labels.get(i).cloned().unwrap_or_default(),
                    url: urls.get(i).cloned().unwrap_or_default(),
                })
                .collect();
            if let Some(list) = form.list_mut(&kind) {
                *list = items;
            }
        }
        form
    }

    /// The menu a kind names. A language's entry is created on first use,
    /// so its menus exist even when both were posted empty.
    fn list_mut(&mut self, kind: &str) -> Option<&mut Vec<NavItem>> {
        let (secondary, code) = parse_kind(kind)?;
        Some(match (secondary, code) {
            (false, None) => &mut self.primary,
            (true, None) => &mut self.secondary,
            (secondary, Some(code)) => {
                let menus = self.locales.entry(code).or_default();
                if secondary {
                    &mut menus.secondary
                } else {
                    &mut menus.primary
                }
            }
        })
    }

    /// A language's menus, empty when it has none.
    fn menus_for(&self, code: &str) -> LocaleMenus {
        self.locales.get(code).cloned().unwrap_or_default()
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for NavForm {
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body = String::from_utf8_lossy(&bytes);
        Ok(NavForm::parse(&body))
    }
}

/// Apply a row button press to the in-progress form. Returns `false` for
/// `save` (the caller persists instead of re-rendering); unknown actions are
/// a harmless no-op re-render. `regen-…` needs the database and is handled
/// by [`navigation_save`] before this is reached.
///
/// An *absent* action means save. A submit button only contributes its
/// name/value when it is the element that activated the form, so any submit
/// path that isn't one of the row buttons — the toolbar Save button, Enter in
/// a text field, a form submitted from script — arrives here with no action
/// at all. Treating that as "unknown" silently threw the edits away.
pub(crate) fn nav_apply_action(form: &mut NavForm, action: &str) -> bool {
    if action == "save" || action.is_empty() {
        return false;
    }
    let Some((verb, rest)) = action.split_once('-') else {
        return true;
    };
    // A language code can carry a hyphen (`pt-BR`), so the index is taken
    // off the end, and only when it is a number.
    let (kind, idx) = match rest.rsplit_once('-') {
        Some((kind, idx)) if idx.chars().all(|c| c.is_ascii_digit()) && !idx.is_empty() => {
            (kind, idx.parse::<usize>().ok())
        }
        _ => (rest, None),
    };
    let Some(list) = form.list_mut(kind) else {
        return true;
    };
    match (verb, idx) {
        ("add", None) => {
            if list.len() < NAV_MAX_ITEMS {
                list.push(NavItem::default());
            }
        }
        // At the end, and once: a menu with two switchers would draw every
        // language twice.
        ("switcher", None) => {
            if !list.iter().any(NavItem::is_switcher) && list.len() < NAV_MAX_ITEMS {
                list.push(NavItem { label: SWITCHER_LABEL.into(), url: locales::SWITCHER_URL.into() });
            }
        }
        ("remove", Some(i)) => {
            if i < list.len() {
                list.remove(i);
            }
        }
        ("up", Some(i)) => {
            if i > 0 && i < list.len() {
                list.swap(i - 1, i);
            }
        }
        ("down", Some(i)) => {
            if i + 1 < list.len() {
                list.swap(i, i + 1);
            }
        }
        _ => {}
    }
    true
}

/// Whether a menu URL is safe to put on every page of the site.
///
/// Relative paths, fragments (Portal's `#/portal/…` links and the language
/// switcher), HTTP(S), `mailto:` and `tel:` are. A `javascript:` URL would
/// run on every page that renders the menu, and a protocol-relative one
/// quietly leaves the site.
pub(crate) fn safe_menu_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (url.starts_with('/') && !url.starts_with("//"))
        || url.starts_with('#')
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
}

/// Normalize the posted rows into the stored array: blank labels are dropped
/// (that's how a row is deleted), blank URLs fall back to the site root, and
/// a URL that is not [`safe_menu_url`] is replaced by it too. Labels are
/// bounded and stripped of control characters.
pub(crate) fn nav_clean(items: &[NavItem]) -> Vec<serde_json::Value> {
    let mut switcher = false;
    items
        .iter()
        .filter_map(|it| {
            let label: String = it.label.chars().filter(|c| !c.is_control()).take(200).collect();
            let label = label.trim();
            if label.is_empty() {
                return None;
            }
            let url: String = it.url.chars().filter(|c| !c.is_control()).take(2000).collect();
            let url = url.trim();
            if locales::is_switcher(url) {
                // One per menu, whatever was posted.
                if switcher {
                    return None;
                }
                switcher = true;
                return Some(serde_json::json!({ "label": label, "url": locales::SWITCHER_URL }));
            }
            let url = if url.is_empty() || !safe_menu_url(url) { "/" } else { url };
            Some(serde_json::json!({ "label": label, "url": url }))
        })
        .collect()
}

/// Read a menu back out of the settings cache (the Admin API also accepts
/// navigation as a JSON-encoded string, so accept that shape too).
pub(crate) fn nav_items_from_setting(v: Option<serde_json::Value>) -> Vec<NavItem> {
    let arr = match v {
        Some(serde_json::Value::Array(a)) => a,
        Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => return vec![],
    };
    items_from_array(&arr)
}

fn items_from_array(arr: &[serde_json::Value]) -> Vec<NavItem> {
    arr.iter()
        .filter_map(|o| {
            Some(NavItem {
                label: o.get("label")?.as_str()?.trim().to_string(),
                url: o.get("url").and_then(|u| u.as_str()).unwrap_or("/").trim().to_string(),
            })
        })
        .collect()
}

/// One of the per-language maps, as an object whatever shape it was stored
/// in (older rows and form posts hold it as a JSON string).
fn locale_map(state: &AppState, key: &str) -> serde_json::Map<String, serde_json::Value> {
    match state.settings.get(key) {
        Some(serde_json::Value::Object(o)) => o,
        Some(serde_json::Value::String(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
        _ => serde_json::Map::new(),
    }
}

/// Every language's menus as stored. Languages missing from the maps are
/// absent here; [`ensure_locale_menus`] is what fills them in.
fn stored_form(state: &AppState, locales: &Locales) -> NavForm {
    let primary_map = locale_map(state, "navigation_locales");
    let secondary_map = locale_map(state, "secondary_navigation_locales");
    let mut form = NavForm {
        primary: nav_items_from_setting(state.settings.get("navigation")),
        secondary: nav_items_from_setting(state.settings.get("secondary_navigation")),
        ..Default::default()
    };
    for code in locales.enabled().iter().filter(|c| !locales.is_default(c)) {
        let read = |map: &serde_json::Map<String, serde_json::Value>| {
            map.get(code).and_then(|v| v.as_array()).map(|a| items_from_array(a)).unwrap_or_default()
        };
        form.locales.insert(code.clone(), LocaleMenus { primary: read(&primary_map), secondary: read(&secondary_map) });
    }
    form
}

// ---------------------------------------------------------------------------
// generating a language's menus from the default language's
// ---------------------------------------------------------------------------

/// The site path a menu URL points at, and whatever query or fragment
/// followed it — or `None` for a URL that leaves the site, a bare fragment,
/// `mailto:` and the like, which are kept exactly as they are.
pub(crate) fn site_path<'a>(url: &'a str, site_url: &str) -> Option<(&'a str, &'a str)> {
    let site = site_url.trim_end_matches('/');
    let path = if url.starts_with('/') && !url.starts_with("//") {
        url
    } else if !site.is_empty() && url.len() > site.len() && url[..site.len()].eq_ignore_ascii_case(site) {
        let rest = &url[site.len()..];
        if !rest.starts_with('/') {
            return None;
        }
        rest
    } else if !site.is_empty() && url.eq_ignore_ascii_case(site) {
        "/"
    } else {
        return None;
    };
    Some(match path.find(['?', '#']) {
        Some(at) => (&path[..at], &path[at..]),
        None => (path, ""),
    })
}

/// Path segments that name an archive or a route rather than an entry. An
/// archive exists in every language, so these just take the prefix.
const ARCHIVE_SEGMENTS: &[&str] = &["tag", "author", "page", "rss", "p", "email", "unsubscribe"];

/// `items` — a menu in the default language — rewritten for `code`.
///
/// * An item naming a post or page that has a **published** translation in
///   `code` links to that translation, labelled with its title: the label
///   is then in the right language even though nobody typed it.
/// * An item naming an entry with no such translation keeps its URL — the
///   original is the nearest thing there is, and `/de/<its slug>/` would be
///   a 404.
/// * The home page, archives and any other site path take the language
///   prefix (`/` → `/de/`, `/tag/news/` → `/de/tag/news/`).
/// * External links, fragments and the language switcher are kept as is.
pub(crate) async fn localise_menu(state: &AppState, locales: &Locales, items: &[NavItem], code: &str) -> Vec<NavItem> {
    let site_url = state.site_url();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if item.is_switcher() {
            out.push(item.clone());
            continue;
        }
        let Some((path, suffix)) = site_path(&item.url, &site_url) else {
            out.push(item.clone());
            continue;
        };
        // A menu already pointing into some language is read as the
        // neutral path under that prefix.
        let (_, neutral) = locales.split_path(path);
        let segments: Vec<&str> = neutral.split('/').filter(|s| !s.is_empty()).collect();
        let prefixed = |p: &str| NavItem {
            label: item.label.clone(),
            url: format!("{}{suffix}", locales.localise(p, code)),
        };
        let Some(last) = segments.last() else {
            out.push(prefixed("/"));
            continue;
        };
        if ARCHIVE_SEGMENTS.contains(&segments[0]) {
            out.push(prefixed(&neutral));
            continue;
        }
        match entry_for_slug(state, last).await {
            Some(entry) => out.push(match translated(state, locales, &entry, code).await {
                Some(t) => NavItem {
                    label: t.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&item.label).to_string(),
                    url: format!("{}{suffix}", crate::languages::public_path(state, &t).await),
                },
                None => item.clone(),
            }),
            // Not an entry — a custom route from routes.yaml, a collection —
            // which the router serves under every language's prefix.
            None => out.push(prefixed(&neutral)),
        }
    }
    out
}

/// The published page or post with this slug. Pages first: a menu links to
/// pages far more often than to posts.
async fn entry_for_slug(state: &AppState, slug: &str) -> Option<ghost_core::models::Post> {
    for post_type in ["page", "post"] {
        if let Ok(Some(post)) = repos::posts::find_by_slug(&state.db, slug, post_type, true).await {
            return Some(post);
        }
    }
    None
}

/// `entry` as it reads in `code`: itself if it is already in that language,
/// otherwise its published translation, if it has one.
async fn translated(
    state: &AppState,
    locales: &Locales,
    entry: &ghost_core::models::Post,
    code: &str,
) -> Option<ghost_core::models::Post> {
    if locales.resolve(entry.locale.as_deref()).eq_ignore_ascii_case(code) {
        return Some(entry.clone());
    }
    let group = entry.translation_group.as_deref()?;
    repos::locales::in_locale(&state.db, group, code, locales.default_locale())
        .await
        .ok()
        .flatten()
        .filter(|t| t.status == "published")
}

/// Generate menus for every enabled language that has none, and drop the
/// menus of languages that are no longer enabled.
///
/// Run when Settings → Languages is saved — so switching a language on
/// gives it a menu in that language straight away — and when this page
/// opens, which covers languages enabled before menus were generated. A
/// language that already has a menu, even a deliberately empty one, is left
/// alone: it is somebody's work.
pub(crate) async fn ensure_locale_menus(state: &AppState) -> ghost_core::Result<()> {
    let locales = crate::settings::site_locales(state);
    let mut primary_map = locale_map(state, "navigation_locales");
    let mut secondary_map = locale_map(state, "secondary_navigation_locales");
    let before = (primary_map.clone(), secondary_map.clone());

    let wanted: Vec<&String> = locales.enabled().iter().filter(|c| !locales.is_default(c)).collect();
    primary_map.retain(|code, _| wanted.iter().any(|w| *w == code));
    secondary_map.retain(|code, _| wanted.iter().any(|w| *w == code));

    let default_primary = nav_items_from_setting(state.settings.get("navigation"));
    let default_secondary = nav_items_from_setting(state.settings.get("secondary_navigation"));
    for code in wanted {
        if !primary_map.contains_key(code.as_str()) {
            let items = localise_menu(state, &locales, &default_primary, code).await;
            primary_map.insert(code.clone(), serde_json::Value::Array(nav_clean(&items)));
        }
        if !secondary_map.contains_key(code.as_str()) {
            let items = localise_menu(state, &locales, &default_secondary, code).await;
            secondary_map.insert(code.clone(), serde_json::Value::Array(nav_clean(&items)));
        }
    }

    if (primary_map.clone(), secondary_map.clone()) != before {
        state.settings.set(&state.db, "navigation_locales", &serde_json::Value::Object(primary_map)).await?;
        state.settings.set(&state.db, "secondary_navigation_locales", &serde_json::Value::Object(secondary_map)).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// the page
// ---------------------------------------------------------------------------

/// Redirect back to the navigation page carrying a one-shot banner.
pub(crate) fn navigation_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/navigations/?{encoded}")).into_response()
}

fn row_buttons(kind: &str, i: usize) -> String {
    format!(
        r#"<div class="trailing">
<button class="icon-btn" type="submit" name="action" value="up-{kind}-{i}" data-nav-move="up" title="Move up" aria-label="Move up">{up}</button>
<button class="icon-btn" type="submit" name="action" value="down-{kind}-{i}" data-nav-move="down" title="Move down" aria-label="Move down">{down}</button>
<button class="icon-btn danger" type="submit" name="action" value="remove-{kind}-{i}" data-nav-remove title="Remove" aria-label="Remove">{close}</button>
</div>"#,
        kind = esc(kind),
        i = i,
        up = svg("up"),
        down = svg("down"),
        close = svg("close"),
    )
}

/// One editable menu: a card of label/URL rows plus its add buttons. The
/// up/down/remove buttons post back with `action=…` (and move instantly via
/// JS when available); rows post in DOM order, which is the saved order.
///
/// `languages` is the switcher's preview text, or `None` where the switcher
/// is not offered — a single-language site has nothing to switch between.
pub(crate) fn nav_list_html(items: &[NavItem], kind: &str, tint: &str, languages: Option<&str>) -> String {
    let rows: String = if items.is_empty() {
        r#"<div class="row"><div class="body"><span class="muted">Nothing here yet — add your first item below.</span></div></div>"#.into()
    } else {
        items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                if it.is_switcher() {
                    return format!(
                        r#"<div class="row nav-row nav-switcher" style="align-items:center">
<span class="ic tint-teal">{globe}</span>
<div class="body"><span class="title">Language switcher</span>
<span class="detail">{detail}</span>
<input type="hidden" name="{kind}_label" value="{label}"><input type="hidden" name="{kind}_url" value="{url}"></div>
{buttons}
</div>"#,
                        globe = svg("globe"),
                        detail = esc(&format!(
                            "{} — each links to this page in that language, or to its home page where there is no translation.",
                            languages.unwrap_or("Only shown once the site has a second language")
                        )),
                        kind = esc(kind),
                        label = esc(&it.label),
                        url = esc(locales::SWITCHER_URL),
                        buttons = row_buttons(kind, i),
                    );
                }
                format!(
                    r#"<div class="row nav-row" style="align-items:center">
<span class="ic tint-{tint}">{menu}</span>
<div class="body nav-grid">
<label class="field" style="margin:0"><input name="{kind}_label" value="{label}" placeholder=" "><span>Label</span></label>
<label class="field" style="margin:0"><input name="{kind}_url" value="{url}" placeholder=" "><span>URL · e.g. /about/ or https://…</span></label>
</div>
{buttons}
</div>"#,
                    tint = tint,
                    menu = svg("menu"),
                    kind = esc(kind),
                    label = esc(&it.label),
                    url = esc(&it.url),
                    buttons = row_buttons(kind, i),
                )
            })
            .collect()
    };
    let switcher = match languages {
        Some(_) if !kind.starts_with("secondary") => format!(
            r#" <button class="btn text small" type="submit" name="action" value="switcher-{kind}" title="Adds it at the end of this menu">{globe} Add language switcher</button>"#,
            kind = esc(kind),
            globe = svg("globe"),
        ),
        _ => String::new(),
    };
    format!(
        r#"<div class="card rows rise" style="--i:{i}">{rows}</div>
<div style="padding:2px 4px 8px"><button class="btn tonal small" type="submit" name="action" value="add-{kind}">{add} Add item</button>{switcher}</div>"#,
        i = if kind.starts_with("primary") { 1 } else { 3 },
        rows = rows,
        kind = esc(kind),
        add = svg("add"),
        switcher = switcher,
    )
}

/// The page for a single-language site with only these two menus.
#[cfg(test)]
pub(crate) fn navigation_html(primary: &[NavItem], secondary: &[NavItem], banner: &str) -> String {
    let form = NavForm { primary: primary.to_vec(), secondary: secondary.to_vec(), ..Default::default() };
    navigation_form_html(&form, &Locales::default(), banner)
}

/// The Navigation page: Primary and Secondary for the default language
/// and, on a multilingual site, for every other language beneath it.
pub(crate) fn navigation_form_html(form: &NavForm, locales: &Locales, banner: &str) -> String {
    let multilingual = locales.multilingual();
    let languages = multilingual.then(|| {
        locales.enabled().iter().map(|c| locales::native_name(c)).collect::<Vec<_>>().join(" · ")
    });
    let default_name = crate::settings::locale_label(locales.default_locale());

    let section = |title: &str, code: Option<&str>, menus: (&[NavItem], &[NavItem])| -> String {
        let (primary, secondary) = menus;
        let anchor = code.map(|c| format!("nav-{}", c.to_ascii_lowercase())).unwrap_or_else(|| "nav-default".into());
        let prefix = code.map(|c| locales.prefix(c)).unwrap_or_default();
        let head = if multilingual {
            let regen = match code {
                Some(c) => format!(
                    r#"<button class="btn text small" type="submit" name="action" value="regen-{c}" data-confirm="Regenerate {name} menus|Both {name} menus are rebuilt from the {default} ones, linking to {name} translations where they exist. Nothing is saved until you press Save.">{repeat} Regenerate from {default}</button>"#,
                    c = esc(c),
                    name = esc(&locales::native_name(c)),
                    default = esc(&locales::native_name(locales.default_locale())),
                    repeat = svg("repeat"),
                ),
                None => String::new(),
            };
            format!(
                r#"<div class="nav-lang-head rise" id="{anchor}" style="display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:28px 0 4px">
<h2 style="margin:0;font-size:1.15rem">{title}</h2><span class="chip tonal">{url}</span><span style="flex:1"></span>{regen}</div>"#,
                anchor = esc(&anchor),
                title = esc(title),
                url = esc(&format!("{prefix}/")),
                regen = regen,
            )
        } else {
            String::new()
        };
        format!(
            r#"{head}
<div class="section">Primary navigation</div>
<p class="muted" style="margin:0 0 8px 16px">The main menu, rendered by the active theme via <code>{{{{navigation}}}}</code>.</p>
{primary}
<div class="section">Secondary navigation</div>
<p class="muted" style="margin:0 0 8px 16px">A second menu — usually footer links — via <code>{{{{navigation type="secondary"}}}}</code>.</p>
{secondary}"#,
            head = head,
            primary = nav_list_html(primary, &kind_for(false, code), "red", languages.as_deref()),
            secondary = nav_list_html(secondary, &kind_for(true, code), "blue", None),
        )
    };

    let mut body = section(
        &format!("{default_name} · default"),
        None,
        (&form.primary, &form.secondary),
    );
    let mut jump = String::new();
    if multilingual {
        jump.push_str(&format!(
            r##"<a class="chip tonal" href="#nav-default">{}</a>"##,
            esc(&locales::native_name(locales.default_locale()))
        ));
        for code in locales.enabled().iter().filter(|c| !locales.is_default(c)) {
            let menus = form.menus_for(code);
            body.push_str(&section(
                &crate::settings::locale_label(code),
                Some(code),
                (&menus.primary, &menus.secondary),
            ));
            jump.push_str(&format!(
                r##" <a class="chip tonal" href="#nav-{anchor}">{name}</a>"##,
                anchor = esc(&code.to_ascii_lowercase()),
                name = esc(&locales::native_name(code)),
            ));
        }
    }
    let intro = if multilingual {
        format!(
            r#"<p class="muted rise" style="--i:1;margin:0 0 6px">Every language has its own menus, shown on that language's pages. A new language's menus are generated from the {default} ones, pointing at their translations; after that they are yours to edit. Languages are chosen in <a href="/ghost/settings/languages/">Language settings</a>.</p>
<p class="rise" style="--i:1;margin:0 0 4px">{jump}</p>"#,
            default = esc(&locales::native_name(locales.default_locale())),
            jump = jump,
        )
    } else {
        String::new()
    };

    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/tools/" title="Back to tools">{back}</a>
<div><h1>Navigations</h1><p class="sub">Menus shown on your site</p></div>
<div class="actions"><button class="btn" type="submit" form="nav-form" name="action" value="save">{check} Save</button></div></header>
{banner}
{intro}
<form method="post" action="/ghost/navigations/" id="nav-form">
<button type="submit" name="action" value="save" hidden></button>
{body}
</form>"#,
        back = svg("chevron"),
        check = svg("check"),
        banner = banner,
        intro = intro,
        body = body,
    )
}

pub(crate) async fn navigation_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let banner = match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    };
    // A language enabled before menus were generated gets them now, so the
    // page never shows a language with no menus to edit.
    if let Err(e) = ensure_locale_menus(&state).await {
        tracing::error!("navigation: generating language menus failed: {}", e.message());
    }
    let locales = crate::settings::site_locales(&state);
    let form = stored_form(&state, &locales);
    page_for_role(actor_role,
        "Navigations",
        "tools",
        Some(&user),
        &navigation_form_html(&form, &locales, &banner),
        true,
    )
    .into_response()
}

pub(crate) async fn navigation_save(State(state): State<AppState>, headers: HeaderMap, form: NavForm) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let locales = crate::settings::site_locales(&state);
    let mut form = form;
    let action = form.action.clone();
    let rerender = |form: &NavForm, banner: &str| {
        page_for_role(actor_role,
            "Navigations",
            "tools",
            Some(&user),
            &navigation_form_html(form, &locales, banner),
            true,
        )
        .into_response()
    };

    // Rebuild one language's menus from the default language's *posted*
    // menus — what is on screen, saved or not — and show the result unsaved.
    if let Some(code) = action.strip_prefix("regen-") {
        if let Some(code) = locales::normalise(code).filter(|c| locales.is_enabled(c) && !locales.is_default(c)) {
            let primary = localise_menu(&state, &locales, &form.primary, &code).await;
            let secondary = localise_menu(&state, &locales, &form.secondary, &code).await;
            form.locales.insert(code.clone(), LocaleMenus { primary, secondary });
            let banner = format!(
                r#"<div class="banner ok rise" style="--i:1">{check}{} menus regenerated. Press Save to keep them.</div>"#,
                esc(&locales::native_name(&code)),
                check = svg("check"),
            );
            return rerender(&form, &banner);
        }
        return rerender(&form, "");
    }
    // A row button (add/move/remove) re-renders the page with that edit
    // applied but unsaved, keeping whatever was typed into the other rows.
    if nav_apply_action(&mut form, &action) {
        return rerender(&form, "");
    }

    // Only languages the site publishes in: a menu cannot be stored for one
    // it does not. A language's posted menus are stored even when empty —
    // that is a deliberate "no menu here", which the front end honours.
    let mut primary_map = serde_json::Map::new();
    let mut secondary_map = serde_json::Map::new();
    for code in locales.enabled().iter().filter(|c| !locales.is_default(c)) {
        let menus = form.menus_for(code);
        primary_map.insert(code.clone(), serde_json::Value::Array(nav_clean(&menus.primary)));
        secondary_map.insert(code.clone(), serde_json::Value::Array(nav_clean(&menus.secondary)));
    }
    let writes = [
        ("navigation", serde_json::Value::Array(nav_clean(&form.primary))),
        ("secondary_navigation", serde_json::Value::Array(nav_clean(&form.secondary))),
        ("navigation_locales", serde_json::Value::Object(primary_map)),
        ("secondary_navigation_locales", serde_json::Value::Object(secondary_map)),
    ];
    for (key, value) in &writes {
        // The maps are only worth writing on a site that has them in use;
        // a single-language site's page never posts any.
        if key.ends_with("_locales") && !locales.multilingual() {
            continue;
        }
        if let Err(e) = state.settings.set(&state.db, key, value).await {
            return navigation_flash("error", &e.message());
        }
    }
    navigation_flash("ok", "Navigation saved.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_menu_row_is_parsed_into_that_language() {
        let form = NavForm::parse(
            "primary_label=Home&primary_url=%2F\
             &primary.de_label=Start&primary.de_url=%2Fde%2F\
             &secondary.pt-BR_label=Sobre&secondary.pt-BR_url=%2Fpt-br%2Fsobre%2F\
             &primary.xx%2F.._label=Evil&primary.xx%2F.._url=%2F",
        );
        assert_eq!(form.primary.len(), 1);
        assert_eq!(form.locales["de"].primary, vec![NavItem { label: "Start".into(), url: "/de/".into() }]);
        assert_eq!(form.locales["pt-BR"].secondary[0].label, "Sobre");
        // A kind that is not a canonical language code is not a field this
        // page wrote, and is dropped rather than stored under a strange key.
        assert_eq!(form.locales.len(), 2, "{:?}", form.locales.keys());
    }

    #[test]
    fn row_actions_reach_a_language_whose_code_has_a_hyphen() {
        let mut form = NavForm::default();
        form.locales.insert(
            "pt-BR".into(),
            LocaleMenus {
                primary: vec![
                    NavItem { label: "A".into(), url: "/a/".into() },
                    NavItem { label: "B".into(), url: "/b/".into() },
                ],
                ..Default::default()
            },
        );
        assert!(nav_apply_action(&mut form, "down-primary.pt-BR-0"));
        assert_eq!(form.locales["pt-BR"].primary[0].label, "B");
        assert!(nav_apply_action(&mut form, "add-secondary.pt-BR"));
        assert_eq!(form.locales["pt-BR"].secondary.len(), 1);
        assert!(nav_apply_action(&mut form, "remove-primary.pt-BR-1"));
        assert_eq!(form.locales["pt-BR"].primary.len(), 1);
    }

    #[test]
    fn the_switcher_is_added_once_at_the_end() {
        let mut form = NavForm {
            primary: vec![NavItem { label: "Home".into(), url: "/".into() }],
            ..Default::default()
        };
        assert!(nav_apply_action(&mut form, "switcher-primary"));
        assert!(nav_apply_action(&mut form, "switcher-primary"));
        assert_eq!(form.primary.len(), 2, "a second press must not add a second switcher");
        assert!(form.primary[1].is_switcher());
        // And a language menu takes one too.
        assert!(nav_apply_action(&mut form, "switcher-primary.de"));
        assert!(form.locales["de"].primary[0].is_switcher());
    }

    #[test]
    fn a_switcher_survives_cleaning_once_per_menu() {
        let items = vec![
            NavItem { label: "Language".into(), url: "#language-switcher".into() },
            NavItem { label: "Again".into(), url: "#Language-Switcher".into() },
        ];
        let clean = nav_clean(&items);
        assert_eq!(clean, vec![serde_json::json!({"label": "Language", "url": "#language-switcher"})]);
    }

    #[test]
    fn a_menu_url_that_would_run_script_is_refused() {
        // The menu renders on every page of the site, so a `javascript:` URL
        // in it would run everywhere.
        let items = vec![
            NavItem { label: "Evil".into(), url: "javascript:alert(1)".into() },
            NavItem { label: "Off-site".into(), url: "//evil.example/x".into() },
            NavItem { label: "Docs".into(), url: "https://example.com/docs".into() },
            NavItem { label: "Mail".into(), url: "mailto:hi@example.com".into() },
            NavItem { label: "Ho\u{0}me".into(), url: "/".into() },
        ];
        let clean = nav_clean(&items);
        assert_eq!(clean[0]["url"], "/", "a javascript: URL is replaced, not stored");
        assert_eq!(clean[1]["url"], "/", "a protocol-relative URL leaves the site");
        assert_eq!(clean[2]["url"], "https://example.com/docs");
        assert_eq!(clean[3]["url"], "mailto:hi@example.com");
        assert_eq!(clean[4]["label"], "Home", "control characters cannot survive into a label");
    }

    #[test]
    fn a_site_path_is_found_in_relative_and_absolute_urls() {
        let site = "https://site.test";
        assert_eq!(site_path("/about/", site), Some(("/about/", "")));
        assert_eq!(site_path("/about/?x=1#top", site), Some(("/about/", "?x=1#top")));
        assert_eq!(site_path("https://site.test/about/", site), Some(("/about/", "")));
        assert_eq!(site_path("https://site.test", site), Some(("/", "")));
        // Not the site, or not a path: kept exactly as typed.
        assert_eq!(site_path("https://site.test.evil/x/", site), None);
        assert_eq!(site_path("https://other.test/about/", site), None);
        assert_eq!(site_path("//site.test/about/", site), None);
        assert_eq!(site_path("#/portal/", site), None);
        assert_eq!(site_path("mailto:x@y", site), None);
    }

    #[test]
    fn a_multilingual_page_has_both_menus_for_every_language() {
        let locales = Locales::new("en", &["de", "pt-BR"]);
        let mut form = NavForm {
            primary: vec![NavItem { label: "Home".into(), url: "/".into() }],
            ..Default::default()
        };
        form.locales.insert(
            "de".into(),
            LocaleMenus {
                primary: vec![
                    NavItem { label: "Start".into(), url: "/de/".into() },
                    NavItem { label: "Language".into(), url: locales::SWITCHER_URL.into() },
                ],
                ..Default::default()
            },
        );
        let html = navigation_form_html(&form, &locales, "");
        for kind in ["primary", "secondary", "primary.de", "secondary.de", "primary.pt-BR", "secondary.pt-BR"] {
            assert!(html.contains(&format!(r#"value="add-{kind}""#)), "{kind} has no menu:\n{html}");
        }
        // The switcher is offered on primary menus only, and drawn as one.
        assert!(html.contains(r#"value="switcher-primary.de""#));
        assert!(!html.contains(r#"value="switcher-secondary"#));
        assert!(html.contains("Language switcher"));
        assert!(html.contains("English · Deutsch · Português"), "{html}");
        // Every other language can be rebuilt from the default.
        assert!(html.contains(r#"value="regen-de""#));
        assert!(html.contains(r#"value="regen-pt-BR""#));
        assert!(!html.contains(r#"value="regen-en""#));
    }

    #[test]
    fn a_single_language_page_offers_no_language_machinery() {
        let html = navigation_html(&[NavItem { label: "Home".into(), url: "/".into() }], &[], "");
        assert!(!html.contains("switcher-"), "{html}");
        assert!(!html.contains("regen-"), "{html}");
        assert!(html.contains(r#"value="add-primary""#));
        assert!(html.contains(r#"value="add-secondary""#));
    }
}
