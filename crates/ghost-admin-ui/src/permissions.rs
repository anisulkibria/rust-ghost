//! Permissions: what each staff role may reach, and the screen the Owner
//! sets it from.
//!
//! The rules — the levels, the defaults, the reading and writing of the one
//! `permissions` setting — are [`ghost_core::permissions`], unit-tested there
//! without a database. This file is the admin half: the catalogue of things
//! that can be granted, the enforcement, and the Owner's editor at
//! `/ghost/permissions/`.
//!
//! ## How enforcement works, and why it is not 147 edits
//!
//! Every admin handler already starts with `require_role(…, Administrator)`
//! — 147 of them. Editing each one to ask about permissions instead would be
//! 147 chances to miss one, and a missed one is a hole rather than a bug.
//!
//! Instead there is a single middleware, [`enforce`], mounted on the admin
//! router. For every request it maps the path to a [`Resource`] — a module, a
//! tool or a settings group — and refuses before the handler runs if the
//! viewer's grant does not cover it. A `GET` needs `View`; anything that
//! changes state needs `Edit`. That is the whole of "can see it" against
//! "can change it", and it is enforced in one place that fails closed.
//!
//! That handles *narrowing*. **Widening** — the Owner giving an Editor the
//! SEO module, which the handler itself would refuse — is what
//! [`request_is_granted`] is for: the middleware records, in a task-local for
//! the life of the request, that this request already passed a resource check,
//! and `require_role` accepts that in place of the role it was asking for.
//! A task-local rather than a request header, because a header carrying
//! "already authorised" is a header a client can also send.
//!
//! Three properties follow, and all three are the point:
//!
//! - A path the catalogue does not know is not affected at all. Posts, pages,
//!   tags, the dashboard, sign-in and the assets keep exactly the checks they
//!   had; content levels are enforced where that content is handled.
//! - The middleware refuses *before* the handler, so a narrowed role never
//!   reaches code that would have trusted it.
//! - `require_role` only ever becomes *more* permissive through a grant that
//!   the middleware has already checked for this exact request. It cannot be
//!   reached by a request the middleware did not bless.
//!
//! ## The Owner
//!
//! Owner-only, and the Owner is not in the table: every check short-circuits
//! to full access for them. The one person who can repair a mistake on this
//! screen must not be able to lock themselves out with one.

use super::*;
use ghost_core::permissions::{self as rules, Access, Content, Grants, Rules, Tags};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// the catalogue: everything that can be granted, and the paths it owns
// ---------------------------------------------------------------------------

/// Which family a key belongs to. The three are stored separately because a
/// module slug and a settings slug can collide — `security` and `seo` are
/// both — and a grant for one must never answer for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Tool,
    Settings,
}

/// One grantable thing: its key, what to call it, and the admin paths it owns.
pub(crate) struct Entry {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) detail: String,
    /// Path prefixes, relative to `/ghost`, that belong to this entry.
    pub(crate) paths: Vec<String>,
}

/// Modules that answer to more than the one path their card links to.
///
/// Memberships adopted the members list without moving it, and Analytics owns
/// the click report. A module whose extra screens are not listed here is
/// reachable by anybody the middleware does not know to ask about — so this
/// table is a security boundary, not a convenience.
const EXTRA_TOOL_PATHS: &[(&str, &[&str])] = &[
    ("memberships", &["/members/"]),
    ("analytics", &["/analytics/click/"]),
];

/// Keys that are deliberately not grantable.
///
/// This module is a card on the Modules hub like any other, but the screen
/// that decides who may reach what cannot be one of the things that decision
/// is made about: a grant of `permissions: edit` to a role would hand that
/// role the ability to grant itself everything else, and a grant of
/// `permissions: none` to the Owner is a locked door with the key inside.
/// It is Owner-only through its own guard, and the middleware never sees it.
///
/// It became a *tool* key on 2026-11-20 when Permissions moved off the
/// Modules hub; the filter below is applied to every family, so moving it
/// again cannot quietly make it grantable.
pub(crate) const UNGRANTABLE: &[&str] = &["permissions"];

pub(crate) fn entries(family: Family) -> Vec<Entry> {
    match family {
        Family::Tool => crate::tools::TOOLS
            .iter()
            .filter(|t| !UNGRANTABLE.contains(&t.key))
            .map(|t| {
                let mut paths = vec![t.href.trim_start_matches("/ghost").to_string()];
                for (tool, extra) in EXTRA_TOOL_PATHS {
                    if *tool == t.key {
                        paths.extend(extra.iter().map(|p| (*p).to_string()));
                    }
                }
                Entry { key: t.key.to_string(), title: t.title.to_string(), detail: t.description.to_string(), paths }
            })
            .collect(),
        Family::Settings => crate::settings::SETTINGS_GROUPS
            .iter()
            .map(|g| Entry {
                key: g.slug.to_string(),
                title: g.title.to_string(),
                detail: g.items.join(" · "),
                paths: vec![format!("/settings/{}/", g.slug)],
            })
            .collect(),
    }
}

/// A request's target, once the path has been recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resource {
    /// One module, tool or settings group.
    One(Family, String),
    /// A hub — Modules, Tools or Settings. Opening it needs at least one
    /// grant in that family; the page itself only lists what the viewer may
    /// reach.
    Hub(Family),
}

/// Map an admin path to the thing it belongs to, if the catalogue knows it.
///
/// `path` is relative to `/ghost` and the leading prefix is tolerated either
/// way, because axum's `nest` strips it for the inner router but not for
/// every caller.
pub(crate) fn resource_for_path(path: &str) -> Option<Resource> {
    let path = path.strip_prefix("/ghost").unwrap_or(path);
    let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    // A hub is an exact match: `/settings/` is the hub, `/settings/security/`
    // is a group.
    match path.as_str() {
        // The Modules hub was folded into Tools on 2026-11-21; its old
        // address redirects there and is judged as the hub it leads to.
        "/modules/" | "/modules" => return Some(Resource::Hub(Family::Tool)),
        "/tools/" | "/tools" => return Some(Resource::Hub(Family::Tool)),
        "/settings/" | "/settings" => return Some(Resource::Hub(Family::Settings)),
        _ => {}
    }
    // Settings before modules: `/settings/seo/` must not be read as the SEO
    // module because both are called "seo".
    for family in [Family::Settings, Family::Tool] {
        for entry in entries(family) {
            if entry.paths.iter().any(|p| path.starts_with(p.as_str())) {
                return Some(Resource::One(family, entry.key));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// reading a viewer's grants
// ---------------------------------------------------------------------------

/// Everything `role` is allowed on this site.
pub(crate) fn grants_for(state: &AppState, role: Role) -> Grants {
    rules::for_role(state.settings.get(rules::SETTING_KEY).as_ref(), role.label())
}

/// Every role's grants, for the editor.
pub(crate) fn all_grants(state: &AppState) -> BTreeMap<String, Grants> {
    rules::read(state.settings.get(rules::SETTING_KEY).as_ref())
}

/// The rules for one family. A free function rather than a method: `Grants`
/// belongs to `ghost-core`, and the three families are an admin-side grouping.
pub(crate) fn family_rules(grants: &Grants, family: Family) -> &Rules {
    match family {
        Family::Tool => &grants.tools,
        Family::Settings => &grants.settings,
    }
}

/// What this viewer may do with a resource.
pub(crate) fn access_to(grants: &Grants, resource: &Resource) -> Access {
    match resource {
        Resource::One(family, key) => family_rules(grants, *family).access(key),
        // The best grant in the family: a hub that lists one reachable row is
        // worth opening, and the row itself is still checked when it is
        // followed.
        Resource::Hub(family) => {
            let rules = family_rules(grants, *family);
            entries(*family)
                .iter()
                .map(|e| rules.access(&e.key))
                .max()
                .unwrap_or(rules.default)
                .max(rules.default)
        }
    }
}

// ---------------------------------------------------------------------------
// enforcement
// ---------------------------------------------------------------------------

/// What [`enforce`] works out once per request and leaves where the rest of
/// the request can read it.
#[derive(Clone)]
struct Decision {
    /// The path mapped to a resource this viewer's grants already covered.
    /// This is how a grant *widens* access past the role an old handler asks
    /// for — see [`request_is_granted`].
    granted: bool,
    /// The viewer's whole matrix, so the rail and the hubs can render only
    /// what this person may reach without each needing `&AppState`.
    grants: Option<Grants>,
}

tokio::task_local! {
    /// Set by [`enforce`] for the life of one request.
    ///
    /// A task-local rather than a request header or extension: a header
    /// saying "already authorised" is one a client can also send, and an
    /// extension cannot be read from `rail()`, which takes no request.
    static DECISION: Decision;
}

/// Whether the middleware already cleared this request against the
/// permissions matrix. Consulted by `require_role` when the viewer's role
/// alone is not enough.
pub(crate) fn request_is_granted() -> bool {
    DECISION.try_with(|d| d.granted).unwrap_or(false)
}

/// This request's grants, for code that renders navigation and has no state
/// to read them from.
///
/// Outside a request — a unit test calling `rail()` directly — this falls
/// back to the role's defaults, which reproduce the admin as it behaved
/// before this module existed.
pub(crate) fn current_grants(role: Role) -> Grants {
    DECISION
        .try_with(|d| d.grants.clone())
        .ok()
        .flatten()
        .unwrap_or_else(|| rules::defaults(role.label()))
}

/// Whether a role may open at least one thing in a family — the test for
/// whether its hub is worth offering in the rail.
pub(crate) fn any_readable(grants: &Grants, family: Family) -> bool {
    access_to(grants, &Resource::Hub(family)).may_read()
}

// ---------------------------------------------------------------------------
// content levels
//
// Posts, pages and tags are deliberately NOT in the catalogue above. A module
// is reached or it is not; content is a ladder — whose entries, and may they
// publish — and the answer differs per row rather than per path, which is
// something a path-matching middleware cannot decide. These are the readers
// the handlers that own that content ask instead.
// ---------------------------------------------------------------------------

/// What this viewer may do with posts, or with pages.
pub(crate) fn content_for(state: &AppState, role: Role, post_type: &str) -> Content {
    let grants = grants_for(state, role);
    match post_type {
        "page" => grants.pages,
        _ => grants.posts,
    }
}

/// What this viewer may do with tags.
pub(crate) fn tags_for(state: &AppState, role: Role) -> Tags {
    grants_for(state, role).tags
}

/// The refusal a content screen gives. Named for the thing rather than for a
/// role, because "Posts needs the Editor role" is not true any more — the
/// Owner decides it per role on one screen.
pub(crate) fn content_refused(actor: &Actor, headers: &HeaderMap, what: &str) -> Response {
    if !accepts_html(headers) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"errors":[{
                "message": format!("Your role does not have access to {what}."),
                "errorType": "NoPermissionError",
            }]})),
        )
            .into_response();
    }
    (
        StatusCode::FORBIDDEN,
        page_for(
            "Not allowed",
            "",
            Some(&actor.user),
            actor.role,
            &format!(
                r#"<header class="page-head rise"><div><h1>Not allowed</h1><p class="sub">You are signed in as {role}</p></div></header>
<div class="card padded rise" style="--i:1;text-align:center">
<p class="muted" style="margin:8px 0 16px">Your role does not have access to {what}. Ask the site owner if you need it.</p>
<a class="btn tonal" href="{home}">Back to the dashboard</a></div>"#,
                role = esc(actor.role.label()),
                what = esc(&what.to_lowercase()),
                home = ADMIN_HOME,
            ),
            true,
        ),
    )
        .into_response()
}

/// Whether a method changes state. `GET` and `HEAD` need `View`; everything
/// else needs `Edit`.
fn needs_edit(method: &axum::http::Method) -> bool {
    !matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD)
}

/// The admin router's permissions layer.
pub(crate) async fn enforce(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // `nest` strips `/ghost` for the inner router but `OriginalUri` keeps the
    // whole thing; either is accepted by `resource_for_path`.
    let path = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|uri| uri.0.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    // The vendored editor runtime and the stylesheet are the bulk of the
    // admin's requests and none of them renders a rail or reads a grant.
    // Skipping them keeps this layer off the hot path entirely.
    if path.contains("/assets/") {
        return next.run(req).await;
    }
    let resource = resource_for_path(&path);
    let headers = req.headers().clone();
    let Ok(actor) = require_actor(&state, &headers).await else {
        // Not signed in. The handler's own guard owns that answer — it knows
        // whether to redirect a browser or refuse an API client.
        return next.run(req).await;
    };
    // The Owner is never restricted, and never needs to be blessed either:
    // their role already clears every handler.
    if actor.role == Role::Owner {
        let decision = Decision { granted: true, grants: Some(Grants::owner()) };
        return DECISION.scope(decision, next.run(req)).await;
    }
    let grants = grants_for(&state, actor.role);
    if let Some(resource) = &resource {
        let access = access_to(&grants, resource);
        let enough = if needs_edit(req.method()) { access.may_write() } else { access.may_read() };
        if !enough {
            return refused(&actor, &headers, resource, access);
        }
    }
    // `granted` only for a path the catalogue names: an uncatalogued screen —
    // posts, the dashboard — keeps exactly the role check it had, because
    // nothing here has checked anything about it.
    let decision = Decision { granted: resource.is_some(), grants: Some(grants) };
    DECISION.scope(decision, next.run(req)).await
}

/// What a refusal looks like. A page for a browser, JSON for anything else —
/// the same split `require_role` makes, for the same reason.
fn refused(actor: &Actor, headers: &HeaderMap, resource: &Resource, access: Access) -> Response {
    let what = match resource {
        Resource::One(family, key) => entries(*family)
            .into_iter()
            .find(|e| &e.key == key)
            .map(|e| e.title)
            .unwrap_or_else(|| key.clone()),
        Resource::Hub(Family::Tool) => "Tools".to_string(),
        Resource::Hub(Family::Settings) => "Settings".to_string(),
    };
    if !accepts_html(headers) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"errors":[{
                "message": format!("Your role does not have access to {what}."),
                "errorType": "NoPermissionError",
            }]})),
        )
            .into_response();
    }
    // "View" plus a write is a different sentence from "no access at all":
    // one is a mistake the viewer can understand, the other looks broken.
    let detail = if access == Access::View {
        format!("You can view {what} but not change it. Ask the site owner if you need to.")
    } else {
        format!("Your role does not have access to {what}. Ask the site owner if you need it.")
    };
    (
        StatusCode::FORBIDDEN,
        page_for(
            "Not allowed",
            "",
            Some(&actor.user),
            actor.role,
            &format!(
                r#"<header class="page-head rise"><div><h1>Not allowed</h1><p class="sub">You are signed in as {role}</p></div></header>
<div class="card padded rise" style="--i:1;text-align:center">
<p class="muted" style="margin:8px 0 16px">{detail}</p>
<a class="btn tonal" href="{home}">Back to the dashboard</a></div>"#,
                role = esc(actor.role.label()),
                detail = esc(&detail),
                home = ADMIN_HOME,
            ),
            true,
        ),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// the Owner's editor
// ---------------------------------------------------------------------------

/// Owner only, and an origin check on the save. Nothing below Owner may read
/// this screen either: the matrix names every module on the site, which is a
/// map of the admin for somebody who was given one corner of it.
async fn guard(state: &AppState, headers: &HeaderMap, writes: bool) -> Result<Actor, Response> {
    let actor = require_role(state, headers, Role::Owner).await?;
    if writes && !ghost_api::forms::same_origin(headers, &state.site_url()) {
        return Err((StatusCode::FORBIDDEN, "This request did not come from the admin.").into_response());
    }
    Ok(actor)
}

/// The role whose matrix is on screen. Anything unrecognised — including
/// `Owner`, who is deliberately not editable — falls back to the first tab.
fn selected_role(q: &HashMap<String, String>) -> String {
    q.get("role")
        .map(|r| r.trim())
        .filter(|r| rules::EDITABLE_ROLES.contains(r))
        .unwrap_or(rules::EDITABLE_ROLES[0])
        .to_string()
}

fn access_select(name: &str, current: Access) -> String {
    let options: String = [Access::None, Access::View, Access::Edit]
        .iter()
        .map(|a| {
            format!(
                r#"<option value="{value}"{sel}>{label}</option>"#,
                value = a.name(),
                sel = if *a == current { " selected" } else { "" },
                label = esc(a.label()),
            )
        })
        .collect();
    format!(
        r#"<label class="field fixed pm-pick"><select name="{name}">{options}</select><span class="sr-only">Access</span>{car}</label>"#,
        name = esc(name),
        car = SELECT_CARET,
    )
}

fn content_select(name: &str, current: Content) -> String {
    let options: String = Content::LADDER
        .iter()
        .map(|c| {
            format!(
                r#"<option value="{value}"{sel}>{label}</option>"#,
                value = c.name(),
                sel = if *c == current { " selected" } else { "" },
                label = esc(c.label()),
            )
        })
        .collect();
    format!(
        r#"<label class="field fixed pm-pick"><select name="{name}">{options}</select><span class="sr-only">Access</span>{car}</label>"#,
        name = esc(name),
        car = SELECT_CARET,
    )
}

fn tags_select(current: Tags) -> String {
    let options: String = [Tags::None, Tags::View, Tags::Manage]
        .iter()
        .map(|t| {
            format!(
                r#"<option value="{value}"{sel}>{label}</option>"#,
                value = t.name(),
                sel = if *t == current { " selected" } else { "" },
                label = esc(t.label()),
            )
        })
        .collect();
    format!(
        r#"<label class="field fixed pm-pick"><select name="tags">{options}</select><span class="sr-only">Access</span>{car}</label>"#,
        car = SELECT_CARET,
    )
}

/// One family's block: the fallback for anything not named, then a row per
/// thing that exists today.
fn family_card(family: Family, label: &str, rules: &Rules, index: usize) -> String {
    let prefix = match family {
        Family::Tool => "tools",
        Family::Settings => "settings",
    };
    let rows: String = entries(family)
        .iter()
        .map(|e| {
            format!(
                r#"<div class="row"><div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{pick}</div></div>"#,
                title = esc(&e.title),
                detail = esc(&e.detail),
                pick = access_select(&format!("{prefix}:{}", e.key), rules.access(&e.key)),
            )
        })
        .collect();
    format!(
        r#"<div class="section">{label}</div>
<div class="card rows rise" style="--i:{index}">
<div class="row pm-default"><div class="body"><span class="title">Everything else</span>
<span class="detail">Anything added to this site later, until it is given a setting of its own.</span></div>
<div class="trailing">{default}</div></div>
{rows}</div>"#,
        label = esc(label),
        default = access_select(&format!("{prefix}:{}", rules::DEFAULT_KEY), rules.default),
        rows = rows,
    )
}

pub(crate) async fn permissions_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match guard(&state, &headers, false).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let role = selected_role(&q);
    let all = all_grants(&state);
    let grants = all.get(&role).cloned().unwrap_or_else(|| rules::defaults(&role));

    let tabs: String = rules::EDITABLE_ROLES
        .iter()
        .map(|r| {
            format!(
                r#"<a class="seg{on}" href="/ghost/permissions/?role={r}">{label}</a>"#,
                on = if *r == role { " on" } else { "" },
                r = r,
                label = esc(r),
            )
        })
        .collect();

    let content = format!(
        r#"<div class="section">Content</div>
<div class="card rows rise" style="--i:1">
<div class="row"><div class="body"><span class="title">Posts</span><span class="detail">Whose posts this role may open, and whether it may publish.</span></div>
<div class="trailing">{posts}</div></div>
<div class="row"><div class="body"><span class="title">Pages</span><span class="detail">The same choice for pages.</span></div>
<div class="trailing">{pages}</div></div>
<div class="row"><div class="body"><span class="title">Tags</span><span class="detail">Viewing the tag list, and creating or editing tags.</span></div>
<div class="trailing">{tags}</div></div>
</div>"#,
        posts = content_select("posts", grants.posts),
        pages = content_select("pages", grants.pages),
        tags = tags_select(grants.tags),
    );

    let body = format!(
        r#"<header class="page-head rise"><div><h1>Permissions</h1>
<p class="sub">What each role may reach. The Owner always reaches everything.</p></div>
<div class="actions">
<button class="btn text" type="submit" name="action" value="reset" form="permissions-form"
 data-confirm="Reset {role}|Every choice on this tab goes back to what {article} {role} could reach before anybody changed it. The other roles are not touched.">{undo} Reset to default</button>
<button class="btn" type="submit" form="permissions-form">{check} Save</button></div></header>
{banner}
<div class="seg-bar rise" style="--i:1" role="tablist">{tabs}</div>
<p class="muted pm-intro">Editing <b>{role}</b>. Changes apply to everyone holding that role, the next time they load a page.</p>
<form method="post" action="/ghost/permissions/" id="permissions-form">
<input type="hidden" name="role" value="{role_value}">
{content}
{tools}
{settings}
</form>"#,
        check = svg("check"),
        // "history" is the clock-with-an-arrow-back: the admin's existing
        // glyph for putting something back as it was.
        undo = svg("history"),
        // Three of the four role names begin with a vowel, so a hardcoded
        // "a" is wrong more often than it is right.
        article = if role.starts_with(['A', 'E', 'I', 'O', 'U']) { "an" } else { "a" },
        banner = flash_banner(&q),
        tabs = tabs,
        role = esc(&role),
        role_value = esc(&role),
        content = content,
        tools = family_card(Family::Tool, "Tools", &grants.tools, 3),
        settings = family_card(Family::Settings, "Settings", &grants.settings, 4),
    );
    page_for_role(actor.role, "Permissions", "permissions", Some(&actor.user), &body, true).into_response()
}

pub(crate) async fn permissions_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(response) = guard(&state, &headers, true).await {
        return response;
    }
    // The role comes back in the body, and is checked against the list rather
    // than trusted: a forged value must not create a role nobody can see.
    let Some(role) = form
        .get("role")
        .map(|r| r.trim())
        .filter(|r| rules::EDITABLE_ROLES.contains(r))
        .map(str::to_string)
    else {
        return flash("error", "That is not a role this screen can set.");
    };

    let mut all = all_grants(&state);

    // Reset puts this one role back to `ghost_core::permissions::defaults` —
    // what it could reach before this module existed — and leaves the other
    // roles alone. It shares the Save form, so the selects come with it; they
    // are ignored rather than read, because the point of the button is to
    // discard what is on screen.
    if form.get("action").map(String::as_str) == Some("reset") {
        all.insert(role.clone(), rules::defaults(&role));
        return match state.settings.set(&state.db, rules::SETTING_KEY, &rules::write(&all)).await {
            Ok(()) => {
                fire_site_changed(&state);
                flash_for(&role, "ok", &format!("Put {role} back to the default access."))
            }
            Err(e) => {
                tracing::error!("permissions reset failed: {e}");
                flash_for(&role, "error", "That could not be reset.")
            }
        };
    }

    let grants = all.entry(role.clone()).or_insert_with(|| rules::defaults(&role));

    // Rebuilt from the catalogue rather than from the submitted names, so a
    // posted key nobody offered cannot enter the stored object.
    for (family, prefix) in [
        (Family::Tool, "tools"),
        (Family::Settings, "settings"),
    ] {
        let mut rebuilt = Rules {
            default: form
                .get(&format!("{prefix}:{}", rules::DEFAULT_KEY))
                .map(|v| Access::from_name(v))
                .unwrap_or(Access::None),
            by_key: BTreeMap::new(),
        };
        for entry in entries(family) {
            if let Some(value) = form.get(&format!("{prefix}:{}", entry.key)) {
                rebuilt.set(&entry.key, Access::from_name(value));
            }
        }
        match family {
            Family::Tool => grants.tools = rebuilt,
            Family::Settings => grants.settings = rebuilt,
        }
    }
    if let Some(v) = form.get("posts") {
        grants.posts = Content::from_name(v);
    }
    if let Some(v) = form.get("pages") {
        grants.pages = Content::from_name(v);
    }
    if let Some(v) = form.get("tags") {
        grants.tags = Tags::from_name(v);
    }

    let value = rules::write(&all);
    match state.settings.set(&state.db, rules::SETTING_KEY, &value).await {
        Ok(()) => {
            fire_site_changed(&state);
            flash_for(&role, "ok", &format!("Saved what {role}s may reach."))
        }
        Err(e) => {
            tracing::error!("permissions save failed: {e}");
            flash_for(&role, "error", "That could not be saved.")
        }
    }
}

fn flash(kind: &str, message: &str) -> Response {
    flash_for(rules::EDITABLE_ROLES[0], kind, message)
}

fn flash_for(role: &str, kind: &str, message: &str) -> Response {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("role", role)
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/permissions/?{query}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_paths_are_not_read_as_modules_of_the_same_name() {
        // `seo` and `security` are both a module and a settings group.
        assert_eq!(
            resource_for_path("/ghost/settings/seo/"),
            Some(Resource::One(Family::Settings, "seo".into()))
        );
        assert_eq!(
            resource_for_path("/ghost/seo/"),
            Some(Resource::One(Family::Tool, "seo".into()))
        );
        assert_eq!(
            resource_for_path("/ghost/settings/security/"),
            Some(Resource::One(Family::Settings, "security".into()))
        );
        assert_eq!(
            resource_for_path("/ghost/security/firewall/"),
            Some(Resource::One(Family::Tool, "security".into()))
        );
    }

    #[test]
    fn the_hubs_are_their_own_resource() {
        assert_eq!(resource_for_path("/ghost/settings/"), Some(Resource::Hub(Family::Settings)));
        assert_eq!(resource_for_path("/ghost/modules/"), Some(Resource::Hub(Family::Tool)), "the old hub address is the Tools hub");
        assert_eq!(resource_for_path("/ghost/tools/"), Some(Resource::Hub(Family::Tool)));
    }

    #[test]
    fn the_nested_path_is_accepted_with_or_without_the_prefix() {
        assert_eq!(
            resource_for_path("/settings/general/"),
            Some(Resource::One(Family::Settings, "general".into()))
        );
        assert_eq!(
            resource_for_path("/ghost/settings/general/"),
            Some(Resource::One(Family::Settings, "general".into()))
        );
    }

    #[test]
    fn an_adopted_screen_belongs_to_the_module_that_adopted_it() {
        // Memberships owns /ghost/members/ without having moved it.
        assert_eq!(
            resource_for_path("/ghost/members/"),
            Some(Resource::One(Family::Tool, "memberships".into()))
        );
        assert_eq!(
            resource_for_path("/ghost/analytics/click/"),
            Some(Resource::One(Family::Tool, "analytics".into()))
        );
    }

    #[test]
    fn content_and_sign_in_paths_are_left_alone() {
        // Everything the catalogue does not name keeps the checks it had.
        for path in ["/ghost/posts/", "/ghost/pages/", "/ghost/tags/", "/ghost/dashboard/",
                     "/ghost/signin/", "/ghost/assets/1.8.35/admin.css", "/ghost/permissions/"] {
            assert_eq!(resource_for_path(path), None, "{path} should not be catalogued");
        }
    }

    #[test]
    fn a_hub_opens_for_the_one_row_a_role_can_reach() {
        let mut grants = rules::defaults("Editor");
        assert_eq!(access_to(&grants, &Resource::Hub(Family::Tool)), Access::None);
        grants.tools.set("seo", Access::View);
        assert_eq!(access_to(&grants, &Resource::Hub(Family::Tool)), Access::View);
        assert_eq!(
            access_to(&grants, &Resource::One(Family::Tool, "forms".into())),
            Access::None,
            "the hub opening does not open every row in it"
        );
    }

    #[test]
    fn the_permissions_screen_itself_is_never_grantable() {
        // It is a Modules card, but granting it would let a role grant itself
        // everything else, and denying it could lock the Owner out.
        assert_eq!(resource_for_path("/ghost/permissions/"), None);
        assert!(
            !entries(Family::Tool).iter().any(|e| e.key == "permissions"),
            "the permissions matrix must not list itself"
        );
    }

    #[test]
    fn every_module_card_and_tool_is_grantable() {
        // A card whose path the catalogue does not know would be reachable by
        // anybody, because the middleware would never ask about it.
        for t in crate::tools::TOOLS {
            let path = t.href.trim_start_matches("/ghost");
            if UNGRANTABLE.contains(&t.key) {
                continue;
            }
            assert!(
                matches!(resource_for_path(path), Some(Resource::One(Family::Tool, _))),
                "tool path {} is not catalogued", t.href
            );
        }
        for g in crate::settings::SETTINGS_GROUPS {
            let path = format!("/settings/{}/", g.slug);
            assert!(
                matches!(resource_for_path(&path), Some(Resource::One(Family::Settings, _))),
                "settings path {path} is not catalogued"
            );
        }
    }
}
