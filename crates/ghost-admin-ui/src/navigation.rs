//! Settings -> Navigation: the primary and secondary menus, as ordered label/URL rows.

use super::*;

// ---------------------------------------------------------------------------
// navigation (Ghost-style: `navigation` + `secondary_navigation` settings,
// edited as ordered label/URL rows and rendered by themes via {{navigation}})
// ---------------------------------------------------------------------------

/// One menu entry (`{"label": …, "url": …}` in the stored JSON array).
#[derive(Default, Debug, PartialEq)]
pub(crate) struct NavItem {
    pub(crate) label: String,
    pub(crate) url: String,
}

/// Cap on menu length so `add` can't run away; Ghost has no fixed limit but
/// menus are single-digit in practice.
pub(crate) const NAV_MAX_ITEMS: usize = 50;

/// The posted navigation form. Every row posts a `…_label` and a `…_url`
/// input under the same names, so this needs the same hand-rolled parsing as
/// the editor's tag checkboxes (`axum::Form` would collapse the repeats).
#[derive(Default, Debug)]
pub(crate) struct NavForm {
    pub(crate) primary: Vec<NavItem>,
    pub(crate) secondary: Vec<NavItem>,
    /// Which submit button was pressed: `save`, `add-{kind}` or
    /// `{up|down|remove}-{kind}-{index}`.
    pub(crate) action: String,
}

impl NavForm {
    /// Labels and URLs arrive in row order, so the i-th of each belongs to
    /// the same row — zip them (padding with blanks if counts disagree).
    pub(crate) fn parse(body: &str) -> Self {
        let mut labels = (Vec::new(), Vec::new());
        let mut urls = (Vec::new(), Vec::new());
        let mut form = NavForm::default();
        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            // (primary labels, primary urls, secondary labels, secondary urls)
            match key.as_ref() {
                "primary_label" => labels.0.push(value.into_owned()),
                "primary_url" => urls.0.push(value.into_owned()),
                "secondary_label" => labels.1.push(value.into_owned()),
                "secondary_url" => urls.1.push(value.into_owned()),
                "action" => form.action = value.into_owned(),
                _ => {}
            }
        }
        let zip = |labels: Vec<String>, urls: Vec<String>| {
            (0..labels.len().max(urls.len()))
                .map(|i| NavItem {
                    label: labels.get(i).cloned().unwrap_or_default(),
                    url: urls.get(i).cloned().unwrap_or_default(),
                })
                .collect()
        };
        form.primary = zip(labels.0, urls.0);
        form.secondary = zip(labels.1, urls.1);
        form
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
/// a harmless no-op re-render.
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
    let mut parts = action.split('-');
    let (verb, kind, idx) = (parts.next(), parts.next(), parts.next());
    let list = match kind {
        Some("primary") => &mut form.primary,
        Some("secondary") => &mut form.secondary,
        _ => return true,
    };
    let index = || idx.and_then(|s| s.parse::<usize>().ok());
    match verb {
        Some("add") if idx.is_none() => {
            if list.len() < NAV_MAX_ITEMS {
                list.push(NavItem::default());
            }
        }
        Some("remove") => {
            if let Some(i) = index() {
                if i < list.len() {
                    list.remove(i);
                }
            }
        }
        Some("up") => {
            if let Some(i) = index() {
                if i > 0 && i < list.len() {
                    list.swap(i - 1, i);
                }
            }
        }
        Some("down") => {
            if let Some(i) = index() {
                if i + 1 < list.len() {
                    list.swap(i, i + 1);
                }
            }
        }
        _ => {}
    }
    true
}

/// Normalize the posted rows into the stored array: blank labels are dropped
/// (that's how a row is deleted), blank URLs fall back to the site root.
pub(crate) fn nav_clean(items: &[NavItem]) -> Vec<serde_json::Value> {
    items
        .iter()
        .filter_map(|it| {
            let label = it.label.trim();
            if label.is_empty() {
                return None;
            }
            let url = if it.url.trim().is_empty() { "/" } else { it.url.trim() };
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
    arr.iter()
        .filter_map(|o| {
            Some(NavItem {
                label: o.get("label")?.as_str()?.trim().to_string(),
                url: o.get("url").and_then(|u| u.as_str()).unwrap_or("/").trim().to_string(),
            })
        })
        .collect()
}

/// Redirect back to the navigation page carrying a one-shot banner.
pub(crate) fn navigation_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/navigation/?{encoded}")).into_response()
}

/// One editable menu: a card of label/URL rows plus an add button. The
/// up/down/remove buttons post back with `action=…` (and move instantly via
/// JS when available); rows post in DOM order, which is the saved order.
pub(crate) fn nav_list_html(items: &[NavItem], kind: &str, tint: &str) -> String {
    let rows: String = if items.is_empty() {
        r#"<div class="row"><div class="body"><span class="muted">Nothing here yet — add your first item below.</span></div></div>"#.into()
    } else {
        items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                format!(
                    r#"<div class="row nav-row" style="align-items:center">
<span class="ic tint-{tint}">{menu}</span>
<div class="body nav-grid">
<label class="field" style="margin:0"><input name="{kind}_label" value="{label}" placeholder=" "><span>Label</span></label>
<label class="field" style="margin:0"><input name="{kind}_url" value="{url}" placeholder=" "><span>URL · e.g. /about/ or https://…</span></label>
</div>
<div class="trailing">
<button class="icon-btn" type="submit" name="action" value="up-{kind}-{i}" data-nav-move="up" title="Move up" aria-label="Move up">{up}</button>
<button class="icon-btn" type="submit" name="action" value="down-{kind}-{i}" data-nav-move="down" title="Move down" aria-label="Move down">{down}</button>
<button class="icon-btn danger" type="submit" name="action" value="remove-{kind}-{i}" data-nav-remove title="Remove" aria-label="Remove">{close}</button>
</div>
</div>"#,
                    tint = tint,
                    menu = svg("menu"),
                    kind = kind,
                    i = i,
                    label = esc(&it.label),
                    url = esc(&it.url),
                    up = svg("up"),
                    down = svg("down"),
                    close = svg("close"),
                )
            })
            .collect()
    };
    format!(
        r#"<div class="card rows rise" style="--i:{i}">{rows}</div>
<div style="padding:2px 4px 8px"><button class="btn tonal small" type="submit" name="action" value="add-{kind}">{add} Add item</button></div>"#,
        i = if kind == "primary" { 1 } else { 3 },
        rows = rows,
        kind = kind,
        add = svg("add"),
    )
}

pub(crate) fn navigation_html(primary: &[NavItem], secondary: &[NavItem], banner: &str) -> String {
    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Navigation</h1><p class="sub">Menus shown on your site</p></div>
<div class="actions"><button class="btn" type="submit" form="nav-form" name="action" value="save">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/navigation/" id="nav-form">
<button type="submit" name="action" value="save" hidden></button>
<div class="section">Primary navigation</div>
<p class="muted" style="margin:0 0 8px 16px">The main menu, rendered by the active theme via <code>{{{{navigation}}}}</code>.</p>
{primary}
<div class="section">Secondary navigation</div>
<p class="muted" style="margin:0 0 8px 16px">A second menu — usually footer links — via <code>{{{{navigation type="secondary"}}}}</code>.</p>
{secondary}
</form>"#,
        back = svg("chevron"),
        check = svg("check"),
        banner = banner,
        primary = nav_list_html(primary, "primary", "red"),
        secondary = nav_list_html(secondary, "secondary", "blue"),
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
    let primary = nav_items_from_setting(state.settings.get("navigation"));
    let secondary = nav_items_from_setting(state.settings.get("secondary_navigation"));
    page_for_role(actor_role, 
        "Navigation",
        "settings",
        Some(&user),
        &navigation_html(&primary, &secondary, &banner),
        true,
    )
    .into_response()
}

pub(crate) async fn navigation_save(State(state): State<AppState>, headers: HeaderMap, form: NavForm) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let mut form = form;
    let action = form.action.clone();
    // A row button (add/move/remove) re-renders the page with that edit
    // applied but unsaved, keeping whatever was typed into the other rows.
    if nav_apply_action(&mut form, &action) {
        return page_for_role(actor_role, 
            "Navigation",
            "navigation",
            Some(&user),
            &navigation_html(&form.primary, &form.secondary, ""),
            true,
        )
        .into_response();
    }
    let saved = match state
        .settings
        .set(&state.db, "navigation", &serde_json::Value::Array(nav_clean(&form.primary)))
        .await
    {
        Ok(()) => {
            state
                .settings
                .set(&state.db, "secondary_navigation", &serde_json::Value::Array(nav_clean(&form.secondary)))
                .await
        }
        Err(e) => Err(e),
    };
    match saved {
        Ok(()) => navigation_flash("ok", "Navigation saved."),
        Err(e) => navigation_flash("error", &e.message()),
    }
}
