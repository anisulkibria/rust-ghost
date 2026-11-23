//! Roles, the signed-in actor, and the checks every handler starts with.

use super::*;

// ---------------------------------------------------------------------------
// roles and what each one may do
// ---------------------------------------------------------------------------

/// Ghost's staff roles, most privileged first.
///
/// The capabilities below mirror Ghost's own permission fixtures
/// (`core/server/data/schema/fixtures/fixtures.json`). The shape worth
/// remembering: Contributor, Author and Editor all have `setting =
/// browse,read` — they can *see* settings but never write them — and none of
/// the three has any `member`, `integration`, `api_key` or `db` permission at
/// all. Those screens are Owner/Administrator only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Contributor,
    Author,
    Editor,
    Administrator,
    Owner,
}

impl Role {
    pub(crate) fn from_name(name: &str) -> Role {
        match name {
            "Owner" => Role::Owner,
            "Administrator" => Role::Administrator,
            "Editor" | "Super Editor" => Role::Editor,
            "Author" => Role::Author,
            // Anything unrecognised gets the least privilege rather than the
            // most — an unknown role must never imply trust.
            _ => Role::Contributor,
        }
    }

    /// Settings, themes, design, navigation, integrations, members, and the
    /// import/export tools. Ghost grants none of these below Administrator.
    pub(crate) fn is_admin(self) -> bool {
        self >= Role::Administrator
    }

    /// Editors may edit, publish and delete anyone's posts; Authors and
    /// Contributors only their own.
    ///
    /// This is the **default** now rather than the rule: the Permissions
    /// module can move the line per role, and the screens that own content
    /// ask `crate::permissions::content_for` instead. It survives because
    /// `may_edit_post` is called from places that have no `&AppState` to read
    /// the matrix with, and because it is what
    /// `vuuut_core::permissions::defaults` reproduces.
    pub(crate) fn can_edit_any_post(self) -> bool {
        self >= Role::Editor
    }

    /// Contributors write drafts and nothing else — publishing is the line
    /// Ghost draws for them. Also a default now; see above.
    pub(crate) fn can_publish(self) -> bool {
        self >= Role::Author
    }

    /// Editors manage Authors and Contributors; admins manage everyone.
    pub(crate) fn can_manage_staff(self) -> bool {
        self >= Role::Editor
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Role::Owner => "Owner",
            Role::Administrator => "Administrator",
            Role::Editor => "Editor",
            Role::Author => "Author",
            Role::Contributor => "Contributor",
        }
    }
}

/// A signed-in staff member together with the role their session carries.
pub struct Actor {
    pub user: vuuut_core::models::User,
    pub role: Role,
}

/// Resolve the signed-in user and their role, or redirect to signin.
pub(crate) async fn require_actor(state: &AppState, headers: &HeaderMap) -> Result<Actor, Response> {
    let user = current_user(state, headers)
        .await
        .ok_or_else(|| unauthenticated(headers))?;
    let name = repos::users::role_name(&state.db, &user.id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Actor { role: Role::from_name(&name), user })
}

/// What an unauthenticated request gets.
///
/// A browser is sent to the sign-in screen, which is the useful answer for
/// somebody who followed a link. Anything else gets 401 with a JSON body:
/// several of these handlers are also Admin API endpoints, and redirecting a
/// client to an HTML login page tells it nothing it can act on. The
/// discriminator is whether the caller accepts HTML, since every browser
/// says so and no API client does.
pub(crate) fn unauthenticated(headers: &HeaderMap) -> Response {
    if accepts_html(headers) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({"errors":[{"message":"Authentication required","errorType":"NoPermissionError"}]})),
    )
        .into_response()
}

pub(crate) fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"))
}

/// Require at least `needed`. The refusal is a page rather than a bare 403 so
/// a mistyped URL explains itself instead of looking broken — unless the
/// caller is an API client, which gets the status and nothing to render.
pub(crate) async fn require_role(state: &AppState, headers: &HeaderMap, needed: Role) -> Result<Actor, Response> {
    let actor = require_actor(state, headers).await?;
    if actor.role >= needed {
        return Ok(actor);
    }
    // The Permissions module may have given this role this exact screen.
    //
    // This is only ever reached for a request the permissions middleware has
    // already cleared against the matrix — it sets the flag for the life of
    // the request, and only after checking that the viewer's grant covers
    // both the resource and the method. A request it refused never reaches a
    // handler at all, and a request whose path it does not catalogue leaves
    // the flag false, so every uncatalogued screen keeps exactly the role
    // check it had. See `crate::permissions`.
    if crate::permissions::request_is_granted() {
        return Ok(actor);
    }
    if !accepts_html(headers) {
        return Err((
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"errors":[{
                "message": format!("This needs the {} role or higher.", needed.label()),
                "errorType": "NoPermissionError",
            }]})),
        )
            .into_response());
    }
    Err(forbidden_page(&actor, needed))
}

pub(crate) fn forbidden_page(actor: &Actor, needed: Role) -> Response {
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
<p class="muted" style="margin:8px 0 16px">This area needs the {needed} role or higher. Ask an administrator if you need access.</p>
<a class="btn tonal" href="/ghost/posts/">Back to posts</a></div>"#,
                role = actor.role.label(),
                needed = needed.label(),
            ),
            true,
        ),
    )
        .into_response()
}

/// Whether this user may act on a given post. Authors and Contributors are
/// limited to posts they are an author of.
pub(crate) async fn may_edit_post(state: &AppState, actor: &Actor, post_id: &str) -> bool {
    if actor.role.can_edit_any_post() {
        return true;
    }
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts_authors WHERE post_id = ? AND author_id = ?")
        .bind(post_id)
        .bind(&actor.user.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0)
        > 0
}

pub(crate) async fn require_session(state: &AppState, headers: &HeaderMap) -> Result<vuuut_core::models::User, Redirect> {
    match current_user(state, headers).await {
        Some(u) => Ok(u),
        None => Err(Redirect::to("/ghost/signin/")),
    }
}

pub(crate) async fn current_user(state: &AppState, headers: &HeaderMap) -> Option<vuuut_core::models::User> {
    // An Admin API key, when one is presented. These handlers also back the
    // REST routes merged into `/ghost/api/admin` — the content export, the
    // theme endpoints, the routing files — and reading only the session
    // cookie made every one of them unreachable to an integration. Ghost's
    // DB Backup integration exists precisely to call the export with a key.
    if headers.contains_key(axum::http::header::AUTHORIZATION) {
        return vuuut_api::admin::current_user(state, headers).await;
    }
    // Through the shared reader, so this and `vuuut_api` can never disagree
    // about what the cookie is called.
    let sid = vuuut_api::admin::session_id_from(headers)?;
    // And through `get_user_within`, so the site's session policy applies to
    // the admin UI and the Admin API alike — see
    // `vuuut_db::repos::sessions::get_user_within`.
    let (max_age, idle) = vuuut_api::security_settings::session_policy(state);
    let user_id = repos::sessions::get_user_within(&state.db, &sid, max_age, idle).await.ok()??;
    // `status` is checked here as well as in `verify_password`, and the two
    // are not redundant: that one refuses a *new* sign-in from a suspended
    // account, and this one refuses a browser that was already signed in when
    // the suspension happened. Suspending somebody now revokes their sessions
    // outright (see `staff::staff_set_status`), so in ordinary operation
    // there is nothing here to catch — but a suspension that fails half way,
    // or a status changed by any route that does not revoke, must not leave a
    // working admin session behind. The Admin API's own `current_user` has
    // always filtered on this; the admin UI did not, which made the UI the
    // more permissive of the two doors.
    repos::users::find_by_id(&state.db, &user_id)
        .await
        .ok()?
        .filter(|user| user.status.as_deref() == Some("active"))
}
