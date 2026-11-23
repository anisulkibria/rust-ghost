//! What an integration's API key is allowed to do.
//!
//! Ported from Ghost's `services/permissions/permissions-data.js`. The
//! problem it solves is specific and worth stating: an integration key that
//! names no staff user resolves to the **owner** in this port, and the
//! five-level role hierarchy then waves it through everything. Ghost does not
//! work that way. An "Admin Integration" there is a role of its own with an
//! explicit action/resource matrix, and that matrix deliberately withholds
//! eleven resources — including the ones that would let a leaked key escalate
//! itself:
//!
//! * `api_key` and `integration` — so a key cannot mint more keys.
//! * `authentication` — so a key cannot reset the site's authentication.
//! * `db` — so a key cannot export, import or wipe the whole database.
//! * `identity` — so a key cannot mint identity tokens for members.
//! * plus `app_installation`, `content_import`, `custom_theme_setting`,
//!   `gift`, `members_stripe_connect` and `upgrade`.
//!
//! It also narrows several it does grant: newsletters, offers, tiers and
//! automations can be created and edited but not destroyed, mentions can only
//! be browsed, and a member sign-in URL can only be read.
//!
//! Scope, stated plainly: this enforces the matrix for the admin API paths
//! mapped in [`resource_for_path`]. A path this module does not recognise
//! falls through to the role checks that were there before rather than being
//! denied, so an endpoint nobody has mapped yet keeps working instead of
//! breaking an integration in the field. The eleven withheld resources are
//! all mapped, so the escalation paths are closed.
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// `(role, resource, actions)`, where `*` means every action.
///
/// Transcribed from upstream's `rolePermissions`; a drift here is a drift in
/// what a key can do, so the table is asserted against its source in tests.
const MATRIX: &[(&str, &str, &[&str])] = &[
    ("Admin Integration", "action", &["*"]),
    ("Admin Integration", "automated_email", &["*"]),
    ("Admin Integration", "automation", &["browse", "read", "edit", "add"]),
    ("Admin Integration", "collection", &["*"]),
    ("Admin Integration", "comment", &["*"]),
    ("Admin Integration", "email", &["*"]),
    ("Admin Integration", "email_design_setting", &["*"]),
    ("Admin Integration", "email_preview", &["*"]),
    ("Admin Integration", "gift_link", &["manage"]),
    ("Admin Integration", "invite", &["*"]),
    ("Admin Integration", "label", &["*"]),
    ("Admin Integration", "link", &["*"]),
    ("Admin Integration", "mail", &["*"]),
    ("Admin Integration", "member", &["*"]),
    ("Admin Integration", "member_custom_field", &["*"]),
    ("Admin Integration", "member_signin_url", &["read"]),
    ("Admin Integration", "mention", &["browse"]),
    ("Admin Integration", "newsletter", &["browse", "read", "add", "edit"]),
    ("Admin Integration", "notification", &["*"]),
    ("Admin Integration", "offer", &["browse", "read", "add", "edit"]),
    ("Admin Integration", "post", &["*"]),
    ("Admin Integration", "product", &["browse", "read", "add", "edit"]),
    ("Admin Integration", "recommendation", &["*"]),
    ("Admin Integration", "redirect", &["*"]),
    ("Admin Integration", "role", &["*"]),
    ("Admin Integration", "setting", &["*"]),
    ("Admin Integration", "slug", &["*"]),
    ("Admin Integration", "snippet", &["*"]),
    ("Admin Integration", "tag", &["*"]),
    ("Admin Integration", "theme", &["*"]),
    ("Admin Integration", "user", &["*"]),
    ("Admin Integration", "webhook", &["*"]),
    ("DB Backup Integration", "db", &["*"]),
    ("DB Backup Integration", "member", &["browse"]),
    ("DB Backup Integration", "post", &["browse"]),
    ("Scheduler Integration", "automation", &["poll"]),
    ("Scheduler Integration", "gift", &["flushReminders", "flushDeliveries"]),
    ("Scheduler Integration", "post", &["publish"]),
    ("Self-Serve Migration Integration", "content_import", &["importContent"]),
    ("Self-Serve Migration Integration", "db", &["importContent"]),
    ("Self-Serve Migration Integration", "member", &["add"]),
    ("Self-Serve Migration Integration", "tag", &["read"]),
];

/// Every resource Ghost defines a permission for.
///
/// A resource on this list that a role's matrix does not mention is denied to
/// that role. A path mapping to something *not* on this list is unknown
/// rather than forbidden, and falls through.
const KNOWN_RESOURCES: &[&str] = &[
    "action", "api_key", "app_installation", "authentication", "automated_email", "automation",
    "collection", "comment", "content_import", "custom_theme_setting", "db", "email",
    "email_design_setting", "email_preview", "gift", "gift_link", "identity", "integration",
    "invite", "label", "link", "mail", "member", "member_custom_field", "member_signin_url",
    "members_stripe_connect", "mention", "newsletter", "notification", "offer", "post", "product",
    "recommendation", "redirect", "role", "setting", "slug", "snippet", "tag", "theme", "upgrade",
    "user", "webhook",
];

/// Is `role` allowed to perform `action` on `resource`?
///
/// `None` when the resource is not one Ghost governs, which the caller should
/// read as "no opinion" rather than as a refusal.
pub fn permits(role: &str, resource: &str, action: &str) -> Option<bool> {
    if !KNOWN_RESOURCES.contains(&resource) {
        return None;
    }
    let granted = MATRIX
        .iter()
        .find(|(r, res, _)| *r == role && *res == resource)
        .map(|(_, _, actions)| actions);
    Some(match granted {
        // A resource the role's matrix does not mention is withheld.
        None => false,
        Some(actions) => actions.contains(&"*") || actions.contains(&action),
    })
}

/// The resource an admin API path addresses, in Ghost's naming.
///
/// Returns `None` for a path this module has not been taught, which the
/// middleware treats as "not my business".
pub fn resource_for_path(segments: &[&str]) -> Option<&'static str> {
    let first = *segments.first()?;
    Some(match first {
        // Pages are posts as far as permissions go, as they are upstream.
        "posts" | "pages" => "post",
        "tags" => "tag",
        "users" => "user",
        "roles" => "role",
        "invites" => "invite",
        "members" => match segments.get(1) {
            // Two member sub-paths are governed as resources of their own.
            Some(&"stripe_connect") => "members_stripe_connect",
            _ => match segments.get(2) {
                Some(&"signin_urls") | Some(&"signin_url") => "member_signin_url",
                _ => "member",
            },
        },
        "labels" => "label",
        "member_custom_fields" | "custom_fields" => "member_custom_field",
        "newsletters" => "newsletter",
        "offers" => "offer",
        "tiers" | "products" => "product",
        "snippets" => "snippet",
        "settings" => match segments.get(1) {
            Some(&"stripe") => "members_stripe_connect",
            _ => "setting",
        },
        "custom_theme_settings" => "custom_theme_setting",
        "themes" => "theme",
        "webhooks" => "webhook",
        "notifications" => "notification",
        "redirects" => "redirect",
        "actions" => "action",
        "emails" => "email",
        "email_previews" => "email_preview",
        "email_design_settings" => "email_design_setting",
        "comments" => "comment",
        "mentions" => "mention",
        "links" => "link",
        "slugs" => "slug",
        "recommendations" => "recommendation",
        "automations" => "automation",
        "automated_emails" => "automated_email",
        "collections" => "collection",
        "mail" => "mail",
        "gift_links" => "gift_link",
        "gifts" => "gift",
        // Publishing something that was scheduled is an action on the post,
        // which is how the Scheduler Integration's single grant is written.
        "schedules" => "post",
        // The escalation surface. All withheld from an Admin Integration.
        "db" | "exports" => "db",
        "content_import" => "content_import",
        "integrations" => "integration",
        "api_keys" => "api_key",
        "authentication" => "authentication",
        "identities" => "identity",
        "app_installations" => "app_installation",
        "upgrades" => "upgrade",
        _ => return None,
    })
}

/// The action a method and path perform, in Ghost's naming.
pub fn action_for(method: &Method, segments: &[&str], resource: &str) -> &'static str {
    // `/db` names its actions rather than using CRUD, and they are the ones
    // the matrix is written in terms of.
    if resource == "db" || resource == "content_import" {
        return match *method {
            Method::GET => "exportContent",
            Method::POST => "importContent",
            Method::DELETE => "deleteAllContent",
            _ => "edit",
        };
    }
    // `PUT /schedules/{resource}/{id}` publishes it. Without this the method
    // alone would read as `edit`, which the Scheduler Integration — granted
    // exactly `post: publish` and nothing else — would be refused.
    if segments.first() == Some(&"schedules") {
        return "publish";
    }
    // A trailing verb segment is the action: `/posts/:id/publish`.
    if let Some(last) = segments.last() {
        match *last {
            "publish" => return "publish",
            "poll" => return "poll",
            "flush_reminders" | "flushReminders" => return "flushReminders",
            "flush_deliveries" | "flushDeliveries" => return "flushDeliveries",
            _ => {}
        }
    }
    match *method {
        Method::POST => "add",
        Method::PUT | Method::PATCH => "edit",
        Method::DELETE => "destroy",
        // One path segment is a collection; more than one addresses a record.
        _ => {
            let addressed = segments.iter().filter(|s| !s.is_empty()).count() > 1;
            if addressed { "read" } else { "browse" }
        }
    }
}

/// Ghost's refusal, worded the way its permission layer words it.
pub fn denied(resource: &str, action: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(json!({ "errors": [{
            "message": format!("Authorization failed: this integration may not {action} {resource}."),
            "errorType": "NoPermissionError",
        }]})),
    )
        .into_response()
}

/// Split an admin API path into its meaningful segments.
pub fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/')
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Refuse an integration key the action it is asking for, when Ghost would.
///
/// Only integration keys are judged here. A session request, or a key that
/// names a staff user, is left to the role checks that already run — this
/// exists because a key naming *no* user resolves to the owner, and the role
/// hierarchy then has nothing left to say.
///
/// The path is read from the original URI, because the middleware is attached
/// above the `/ghost/api/admin` nest and the matched path has been stripped
/// by then.
pub async fn enforce(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(role) = integration_role(&state, request.headers()).await else {
        return next.run(request).await;
    };
    let path = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|uri| uri.0.path().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    // Trim the mount and any version prefix so the first segment is the
    // resource: `/ghost/api/admin/v5/posts/` → `posts`.
    let trimmed = path
        .split("/admin")
        .nth(1)
        .map(str::to_string)
        .unwrap_or(path);
    let mut parts = segments(&trimmed);
    if parts.first().is_some_and(|s| {
        s.len() == 2 && s.starts_with('v') && s[1..].chars().all(|c| c.is_ascii_digit())
    }) {
        parts.remove(0);
    }
    let Some(resource) = resource_for_path(&parts) else {
        return next.run(request).await;
    };
    let action = action_for(request.method(), &parts, resource);
    match permits(&role, resource, action) {
        Some(false) => {
            tracing::info!("integration ({role}) refused {action} on {resource}");
            denied(resource, action)
        }
        _ => next.run(request).await,
    }
}

/// The integration role behind this request, if it is an integration at all.
async fn integration_role(
    state: &crate::AppState,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    let header = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let token = crate::apikey_auth::token_from_header(header)?;
    let verified = crate::apikey_auth::verify(state, token, "admin").await?;
    // A key that names a staff user acts as that user, and their role governs.
    if verified.user_id.is_some() {
        return None;
    }
    verified.integration_id.as_ref()?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT r.name FROM api_keys k JOIN roles r ON r.id = k.role_id WHERE k.id = ?",
    )
    .bind(&verified.key_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    // A key with no role recorded is treated as the ordinary integration
    // role rather than as unrestricted, which is the safe reading.
    Some(role.unwrap_or_else(|| "Admin Integration".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: &str = "Admin Integration";

    #[test]
    fn an_admin_integration_cannot_mint_more_credentials() {
        // The point of the whole module: a leaked key must not be able to
        // issue itself fresh ones, or create an integration that has them.
        assert_eq!(permits(ADMIN, "api_key", "add"), Some(false));
        assert_eq!(permits(ADMIN, "integration", "add"), Some(false));
        assert_eq!(permits(ADMIN, "integration", "browse"), Some(false));
    }

    #[test]
    fn an_admin_integration_cannot_reset_authentication_or_the_database() {
        assert_eq!(permits(ADMIN, "authentication", "add"), Some(false));
        assert_eq!(permits(ADMIN, "db", "exportContent"), Some(false));
        assert_eq!(permits(ADMIN, "db", "deleteAllContent"), Some(false));
        assert_eq!(permits(ADMIN, "identity", "read"), Some(false));
        assert_eq!(permits(ADMIN, "members_stripe_connect", "read"), Some(false));
    }

    #[test]
    fn an_admin_integration_can_do_the_everyday_things() {
        for (resource, action) in [
            ("post", "add"), ("post", "destroy"), ("tag", "edit"), ("member", "add"),
            ("setting", "edit"), ("theme", "add"), ("user", "browse"), ("webhook", "add"),
        ] {
            assert_eq!(permits(ADMIN, resource, action), Some(true), "{resource}.{action}");
        }
    }

    #[test]
    fn the_narrowed_grants_are_narrowed() {
        // Create and edit, but not destroy.
        for resource in ["newsletter", "offer", "product"] {
            assert_eq!(permits(ADMIN, resource, "add"), Some(true), "{resource}");
            assert_eq!(permits(ADMIN, resource, "edit"), Some(true), "{resource}");
            assert_eq!(permits(ADMIN, resource, "destroy"), Some(false), "{resource}");
        }
        assert_eq!(permits(ADMIN, "automation", "add"), Some(true));
        assert_eq!(permits(ADMIN, "automation", "destroy"), Some(false));
        assert_eq!(permits(ADMIN, "mention", "browse"), Some(true));
        assert_eq!(permits(ADMIN, "mention", "add"), Some(false));
        assert_eq!(permits(ADMIN, "member_signin_url", "read"), Some(true));
        assert_eq!(permits(ADMIN, "member_signin_url", "add"), Some(false));
    }

    #[test]
    fn the_narrow_integration_roles_are_narrow() {
        assert_eq!(permits("Scheduler Integration", "post", "publish"), Some(true));
        assert_eq!(permits("Scheduler Integration", "post", "destroy"), Some(false));
        assert_eq!(permits("Scheduler Integration", "member", "browse"), Some(false));
        assert_eq!(permits("DB Backup Integration", "db", "exportContent"), Some(true));
        assert_eq!(permits("DB Backup Integration", "post", "browse"), Some(true));
        assert_eq!(permits("DB Backup Integration", "post", "add"), Some(false));
        assert_eq!(permits("Self-Serve Migration Integration", "member", "add"), Some(true));
        assert_eq!(permits("Self-Serve Migration Integration", "member", "browse"), Some(false));
    }

    #[test]
    fn an_unknown_resource_is_no_opinion_rather_than_a_refusal() {
        // So an endpoint nobody has mapped keeps working instead of breaking
        // an integration in the field.
        assert_eq!(permits(ADMIN, "support_requests", "add"), None);
        assert_eq!(resource_for_path(&["support_requests"]), None);
        assert_eq!(resource_for_path(&["featurebase", "token"]), None);
    }

    #[test]
    fn paths_map_to_ghosts_resource_names() {
        for (path, expected) in [
            ("/posts/", Some("post")),
            ("/pages/abc/", Some("post")),
            ("/tags/", Some("tag")),
            ("/tiers/", Some("product")),
            ("/db/", Some("db")),
            ("/exports/download/", Some("db")),
            ("/integrations/", Some("integration")),
            ("/api_keys/x/", Some("api_key")),
            ("/authentication/setup/", Some("authentication")),
            ("/identities/", Some("identity")),
            ("/settings/stripe/connect/", Some("members_stripe_connect")),
            ("/members/stripe_connect/", Some("members_stripe_connect")),
            ("/members/abc/signin_urls/", Some("member_signin_url")),
            ("/members/abc/", Some("member")),
            ("/nonsense/", None),
        ] {
            assert_eq!(resource_for_path(&segments(path)), expected, "{path}");
        }
    }

    #[test]
    fn methods_map_to_ghosts_action_names() {
        let s = segments("/posts/");
        assert_eq!(action_for(&Method::GET, &s, "post"), "browse");
        assert_eq!(action_for(&Method::POST, &s, "post"), "add");
        let one = segments("/posts/abc/");
        assert_eq!(action_for(&Method::GET, &one, "post"), "read");
        assert_eq!(action_for(&Method::PUT, &one, "post"), "edit");
        assert_eq!(action_for(&Method::DELETE, &one, "post"), "destroy");
    }

    #[test]
    fn the_db_resource_uses_its_own_action_names() {
        let s = segments("/db/");
        assert_eq!(action_for(&Method::GET, &s, "db"), "exportContent");
        assert_eq!(action_for(&Method::POST, &s, "db"), "importContent");
        assert_eq!(action_for(&Method::DELETE, &s, "db"), "deleteAllContent");
    }

    #[test]
    fn the_scheduler_can_publish_what_it_was_made_for_and_nothing_else() {
        let s = segments("/schedules/posts/abc/");
        assert_eq!(resource_for_path(&s), Some("post"));
        // A PUT here is a publish, not an edit — the distinction is the
        // whole of the Scheduler Integration's authority.
        assert_eq!(action_for(&Method::PUT, &s, "post"), "publish");
        assert_eq!(permits("Scheduler Integration", "post", "publish"), Some(true));
        assert_eq!(permits("Scheduler Integration", "post", "edit"), Some(false));
        assert_eq!(permits("Scheduler Integration", "setting", "edit"), Some(false));
        assert_eq!(permits("Scheduler Integration", "theme", "add"), Some(false));
    }

    #[test]
    fn the_backup_integration_can_read_and_export_but_not_write() {
        assert_eq!(permits("DB Backup Integration", "db", "exportContent"), Some(true));
        assert_eq!(permits("DB Backup Integration", "post", "browse"), Some(true));
        assert_eq!(permits("DB Backup Integration", "member", "browse"), Some(true));
        assert_eq!(permits("DB Backup Integration", "post", "edit"), Some(false));
        assert_eq!(permits("DB Backup Integration", "setting", "edit"), Some(false));
        assert_eq!(permits("DB Backup Integration", "user", "browse"), Some(false));
    }

    #[test]
    fn a_trailing_verb_is_the_action() {
        let s = segments("/schedules/post/abc/publish/");
        assert_eq!(action_for(&Method::PUT, &s, "post"), "publish");
    }

    #[test]
    fn the_matrix_matches_the_permissions_data_it_was_taken_from() {
        // Transcribed from `services/permissions/permissions-data.js`; a
        // drift here is a drift in what a key can do.
        assert_eq!(MATRIX.len(), 42, "role/resource pairs");
        assert_eq!(KNOWN_RESOURCES.len(), 43, "resources Ghost governs");
        let admin_resources = MATRIX.iter().filter(|(r, _, _)| *r == ADMIN).count();
        assert_eq!(admin_resources, 32);
        // The eleven Ghost withholds from an Admin Integration entirely —
        // absent from its matrix, rather than merely narrowed. `gift_link`
        // and `member_signin_url` are granted but for one action each, so
        // "cannot browse it" is not the same question.
        let withheld: Vec<&str> = KNOWN_RESOURCES
            .iter()
            .copied()
            .filter(|r| !MATRIX.iter().any(|(role, res, _)| *role == ADMIN && res == r))
            .collect();
        assert_eq!(
            withheld,
            vec!["api_key", "app_installation", "authentication", "content_import",
                 "custom_theme_setting", "db", "gift", "identity", "integration",
                 "members_stripe_connect", "upgrade"]
        );
    }
}
