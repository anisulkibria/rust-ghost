//! Built-in server-rendered admin UI (setup wizard, login, posts editor,
//! tags, settings, integrations). Forms post to themselves and call the
//! repos directly.
use axum::extract::{Form, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use ghost_db::repos;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use ghost_api::AppState;

pub mod automations;
pub mod comments;
pub mod posts;
pub mod schedule;

/// Upload ceiling for theme zips and content imports. Ghost's own limit is
/// also in this range; it exists to bound memory, since multipart bodies are
/// buffered before being written out.
const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
/// Redirects and routes files are a few kilobytes; a megabyte is already
/// generous and keeps a mistaken upload of something else from being read.
const ONE_MB_BYTES: usize = 1024 * 1024;

tokio::task_local! {
    static ADMIN_SETTINGS: Arc<ghost_db::SettingsCache>;
}

async fn with_admin_settings(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    ADMIN_SETTINGS.scope(state.settings, next.run(req)).await
}

fn admin_site_name() -> String {
    ADMIN_SETTINGS.try_with(|settings| settings.site_name()).unwrap_or_else(|_| "Ghost".into())
}

fn admin_brand_mark() -> String {
    let image = ADMIN_SETTINGS.try_with(|settings| {
        ["logo", "icon"].iter().find_map(|key| {
            settings.get_str(key).filter(|value| !value.trim().is_empty())
        })
    }).ok().flatten();
    match image {
        Some(url) => format!(
            r#"<span class="mark brand-image"><img src="{}" alt=""></span>"#,
            esc(&ghost_core::url::base_asset(url.trim())),
        ),
        None => format!(r#"<span class="mark brand-admin">{}</span>"#, include_str!("../assets/admin-panel.svg")),
    }
}

pub fn admin_ui_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/setup/", get(setup_page).post(setup_submit))
        .route("/signin/", get(login_page).post(login_submit))
        .route("/forgot-password/", get(forgot_password_page))
        // Static ahead of nothing in particular, but kept together: the code
        // screen a login lands on when email 2FA is required.
        .route("/signin/verify/", get(verify_page).post(verify_submit))
        .route("/signin/resend/", post(verify_resend))
        .route("/signout/", get(logout))
        .route("/analytics/", get(analytics_page))
        .route("/modules/", get(modules_page))
        .route("/social/", get(social_page))
        .route("/machine-payments/", get(machine_payments_page))
        .route("/settings/checkout/", get(checkout_design_page))
        .route("/members/", get(members_page).post(member_create))
        .route("/members/export/", get(members_export))
        .route("/members/import/", post(members_import).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/members/import/run/", post(members_import_run))
        .route("/members/{id}/", get(member_edit_page).post(member_edit_save))
        .route("/members/{id}/delete/", post(member_delete))
        .route("/members/{id}/commenting/disable/", post(comments::member_commenting_disable))
        .route("/members/{id}/commenting/enable/", post(comments::member_commenting_enable))
        // Comment moderation. Ghost puts this at /ghost/#/comments, so the
        // path a report notification deep-links to is the same one here.
        .route("/comments/", get(comments::comments_page))
        .route("/comments/{id}/", get(comments::comment_page))
        .route("/comments/{id}/hide/", post(comments::comment_hide))
        .route("/comments/{id}/show/", post(comments::comment_show))
        .route("/comments/{id}/delete/", post(comments::comment_delete))
        // Automations. `new` is a static segment and must precede `{id}`.
        .route("/automations/", get(automations::automations_page))
        .route("/automations/new/", post(automations::automation_create))
        .route("/automations/{id}/", get(automations::automation_page).post(automations::automation_save))
        .route("/automations/{id}/status/", post(automations::automation_set_status))
        .route("/automations/{id}/delete/", post(automations::automation_delete))
        .route("/automations/{id}/actions/add/", post(automations::action_add))
        .route("/automations/{id}/actions/{action_id}/delete/", post(automations::action_delete))
        .route("/automations/{id}/actions/{action_id}/move/", post(automations::action_move))
        .route("/automations/{id}/actions/{action_id}/test/", post(automations::action_test))
        .route("/posts/", get(posts_list))
        .route("/posts/new/", get(editor_new).post(editor_create))
        .route("/posts/{id}/", get(editor_page).post(editor_save))
        .route("/posts/{id}/delete/", post(post_delete))
        .route("/pages/", get(pages_list))
        .route("/pages/new/", get(page_new).post(page_create))
        .route("/pages/{id}/", get(editor_page).post(editor_save))
        .route("/pages/{id}/delete/", post(page_delete))
        .route("/tags/", get(tags_list).post(tag_create))
        .route("/tags/{id}/", get(tag_edit_page).post(tag_edit_save))
        .route("/tags/{id}/delete/", post(tag_delete))
        .route("/themes/", get(themes_page))
        // Axum caps request bodies at 2 MB by default, which silently fails
        // any real theme — Casper alone is bigger. Raised per-route rather
        // than globally so an oversized body can only arrive where one is
        // actually expected.
        .route("/themes/upload/", post(theme_upload).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        // Static ahead of `{name}`, so "official" is never read as a theme name.
        .route("/themes/official/{slug}/install/", post(theme_install_official))
        .route("/themes/official/{slug}/thumbnail.png", get(theme_official_thumbnail))
        .route("/themes/{name}/edit/", get(theme_edit_page).post(theme_file_save))
        .route("/themes/{name}/edit/new/", post(theme_file_new))
        .route("/themes/{name}/edit/delete/", post(theme_file_delete))
        .route("/themes/{name}/activate/", post(theme_activate))
        .route("/themes/{name}/delete/", post(theme_delete))
        .route("/design/", get(design_page).post(design_save))
        .route("/design/preview/", post(design_preview))
        .route("/design/reset/", post(design_reset))
        .route("/settings/", get(settings_page))
        .route("/settings/general/", get(general_page).post(general_save))
        // Meta data and Social accounts are sections of General settings now.
        // The old paths stay as redirects: they are what an existing bookmark,
        // or a link from elsewhere in this admin, still points at.
        .route("/settings/metadata/", get(|| async { Redirect::to("/ghost/settings/general/") }))
        .route("/settings/social/", get(|| async { Redirect::to("/ghost/settings/general/") }))
        .route("/settings/analytics/", get(analytics_settings_page).post(analytics_settings_save))
        .route("/settings/features/", get(features_page).post(features_save))
        .route("/settings/site/", get(site_page).post(site_save))
        .route("/settings/membership/", get(membership_page).post(membership_save))
        .route("/settings/membership/stripe/connect/", post(stripe_connect))
        .route("/settings/membership/stripe/oauth/start/", post(stripe_oauth_start))
        .route("/settings/membership/stripe/oauth/finish/", post(stripe_oauth_finish))
        .route("/settings/membership/stripe/disconnect/", post(stripe_disconnect))
        .route("/settings/membership/stripe/webhook/", post(stripe_set_webhook_secret))
        .route("/settings/membership/stripe/webhook/deactivate/", post(stripe_deactivate_webhook))
        .route("/settings/membership/tiers/{id}/", get(tier_edit_page).post(tier_edit_save))
        .route("/settings/membership/tiers/new/", post(tier_create))
        .route("/settings/membership/fields/new/", post(metafield_new))
        .route("/settings/membership/fields/{id}/", post(metafield_save))
        .route("/settings/membership/fields/{id}/delete/", post(metafield_delete))
        .route("/settings/membership/fields/{id}/move/", post(metafield_move))
        .route("/settings/email/", get(email_page).post(email_save))
        .route("/settings/email/test/", post(email_test))
        .route("/settings/newsletters/", get(newsletters_settings_page).post(newsletters_settings_save))
        // The static segment is matched ahead of `{id}`, the same way the
        // tier routes above rely on.
        .route("/settings/newsletters/new/", post(newsletter_create))
        .route("/settings/newsletters/{id}/", get(newsletter_edit_page).post(newsletter_edit_save))
        .route("/settings/newsletters/{id}/preview/", get(newsletter_preview))
        // the page was called "Bulk email" before it took Ghost's name
        .route("/settings/bulkemail/", get(|| async { Redirect::permanent("/ghost/settings/newsletters/") }))
        .route("/settings/growth/", get(growth_page).post(growth_save))
        .route("/settings/growth/explore/", post(growth_explore_save))
        .route("/settings/growth/recommendations/new/", post(recommendation_new))
        .route("/settings/growth/recommendations/{id}/", post(recommendation_save))
        .route("/settings/growth/recommendations/{id}/delete/", post(recommendation_delete))
        .route("/settings/growth/offers/new/", post(offer_new))
        .route("/settings/growth/offers/{id}/", get(offer_edit_page).post(offer_save))
        .route("/settings/growth/offers/{id}/status/", post(offer_set_status))
        .route("/settings/history/", get(history_page))
        .route("/settings/danger/", get(danger_page))
        .route("/settings/danger/content/", post(danger_delete_content))
        .route("/settings/danger/site/", post(danger_delete_site))
        .route("/settings/danger/auth/", post(danger_reset_auth))
        .route("/settings/danger/gifts/", post(danger_reset_gifts))
        .route("/settings/about/", get(about_page))
        .route("/settings/code-injection/", get(code_injection_page).post(code_injection_save))
        .route("/settings/advanced/", get(advanced_page))
        // Migration tools. Import and export used to live under Advanced;
        // Ghost gives them a screen of their own, and so does this.
        .route("/settings/migration/", get(migration_page))
        .route(
            "/settings/migration/import/",
            post(migration_import).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/settings/migration/export/", get(migration_export_all))
        .route("/settings/migration/export/content/", get(migration_export_content))
        .route("/settings/migration/export/analytics/", get(migration_export_analytics))
        .route("/settings/migration/export/themes/", get(migration_export_themes))
        .route(
            "/settings/advanced/redirects/",
            get(redirects_download).post(redirects_upload).layer(axum::extract::DefaultBodyLimit::max(ONE_MB_BYTES)),
        )
        .route(
            "/settings/advanced/routes/",
            get(routes_download).post(routes_upload).layer(axum::extract::DefaultBodyLimit::max(ONE_MB_BYTES)),
        )
        .route("/settings/staff/", get(staff_page))
        .route("/settings/staff/security/", post(staff_security_save))
        .route("/settings/staff/invite/", post(staff_invite))
        .route("/settings/staff/invite/{id}/resend/", post(staff_invite_resend))
        .route("/settings/staff/invite/{id}/revoke/", post(staff_invite_revoke))
        .route("/settings/staff/{slug}/", get(profile_page).post(profile_save))
        .route("/settings/staff/{slug}/social/", post(profile_social_save))
        .route("/settings/staff/{slug}/notifications/", post(profile_notifications))
        .route("/settings/staff/{slug}/password/", post(profile_password))
        .route("/settings/staff/{slug}/role/", post(staff_set_role))
        .route("/settings/staff/{slug}/status/", post(staff_set_status))
        .route("/settings/staff/{slug}/delete/", post(staff_delete))
        // redeeming an invitation is the one staff route a signed-out person
        // must reach
        .route("/invite/{token}/", get(invite_accept_page).post(invite_accept_submit))
        .route("/settings/integrations/", get(integrations_page).post(integration_create))
        // the built-in apps. Static segments, so they cannot be shadowed by a
        // custom integration's id — which is why those live under /custom/.
        .route("/settings/integrations/zapier/", get(zapier_page))
        .route("/settings/integrations/slack/", get(slack_page).post(slack_save))
        .route("/settings/integrations/slack/test/", post(slack_test))
        .route("/settings/integrations/unsplash/", get(unsplash_page).post(unsplash_save))
        .route("/settings/integrations/firstpromoter/", get(firstpromoter_page).post(firstpromoter_save))
        .route("/settings/integrations/pintura/", get(pintura_page).post(pintura_save))
        .route("/settings/integrations/transistor/", get(transistor_page).post(transistor_save))
        .route(
            "/settings/integrations/custom/{id}/",
            get(custom_integration_page).post(custom_integration_save),
        )
        .route(
            "/settings/integrations/custom/{id}/webhooks/new/",
            post(webhook_new),
        )
        .route(
            "/settings/integrations/custom/{id}/webhooks/{wid}/delete/",
            post(webhook_delete),
        )
        .route(
            "/settings/integrations/custom/{id}/delete/",
            post(custom_integration_delete),
        )
        // bookmarks and in-page links from before Integrations became a
        // settings page of its own still resolve, wherever they pointed
        .route("/integrations/", get(integrations_redirect))
        .route(
            "/settings/advanced/integrations/",
            get(|| async { Redirect::permanent("/ghost/settings/integrations/") }),
        )
        .route(
            "/settings/advanced/integrations/{*rest}",
            get(|Path(rest): Path<String>| async move {
                Redirect::permanent(&format!("/ghost/settings/integrations/{rest}"))
            }),
        )
        .route("/navigation/", get(navigation_page).post(navigation_save))
        // the editor's vendored runtime (React + Koenig), served from the
        // binary and cached hard behind the version in the path
        .route("/assets/{version}/{file}", get(admin_asset))
        .layer(axum::middleware::from_fn(apply_base_path))
        .layer(axum::middleware::from_fn_with_state(state, with_admin_settings))
}

/// The admin landing route, bound outside the `/ghost` nest.
///
/// See [`index`] for why it is needed. It carries the same base-path layer as
/// the rest of the admin UI, so its redirect to signin or setup is rewritten
/// like any other.
pub fn admin_root_router() -> Router<AppState> {
    Router::new()
        .route("/ghost/", get(index))
        .layer(axum::middleware::from_fn(apply_base_path))
}

/// Put the install's base path onto the admin UI's own links and redirects.
///
/// Every path in this module is written in site terms (`/ghost/posts/`,
/// `href="/"`), and this is the single place they are turned into URLs a
/// browser can follow. Doing it here rather than at ~200 call sites keeps the
/// markup readable and makes it impossible to add a screen that forgets.
///
/// Only two things are touched: a root-relative `Location` header, and the
/// URL-bearing attributes of an HTML body. Scoping the body rewrite to
/// attribute values is what makes it safe to run over pages that embed user
/// content — a post title or a code-injection field holding `href="/x"` is
/// HTML-escaped by the time it lands in the page, so it does not match and is
/// left exactly as it was. Non-HTML responses (CSV exports, the editor's JS
/// bundles, JSON) are passed straight through.
///
/// A no-op for an install at a domain root.
async fn apply_base_path(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    use axum::body::Body;

    let mut resp = next.run(req).await;
    if !ghost_core::url::has_subdir() {
        return resp;
    }

    if let Some(location) = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()) {
        if let Ok(value) = ghost_core::url::base(location).parse() {
            resp.headers_mut().insert(header::LOCATION, value);
        }
    }

    let is_html = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    if !is_html {
        return resp;
    }

    let (mut parts, body) = resp.into_parts();
    // admin pages are rendered into a String before they are sent, so they are
    // already whole in memory — collecting here buffers nothing that was not
    // buffered anyway
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "admin render failed").into_response();
    };
    let Ok(html) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let rewritten = ghost_core::url::rewrite_html(html);
    // the length changed, so a stale Content-Length would truncate the page
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(rewritten))
}

// ---------------------------------------------------------------------------
// layout — Material You (Android Settings / ChromeOS style) shell
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Inline 24dp Material Design icons (outlined style, strokeless fill paths).
fn icon(name: &str) -> &'static str {
    match name {
        "posts" => r#"<path d="M19 5v14H5V5h14m0-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-2.5 14H7v-2h9.5v2zm0-4H7v-2h9.5v2zm0-4H7V7h9.5v2z"/>"#,
        "pages" => r#"<path d="M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z"/>"#,
        "tags" => r#"<path d="M21.41 11.58l-9-9C12.05 2.22 11.55 2 11 2H4c-1.1 0-2 .9-2 2v7c0 .55.22 1.05.59 1.42l9 9c.36.36.86.58 1.41.58s1.05-.22 1.41-.59l7-7c.37-.36.59-.86.59-1.41s-.23-1.06-.59-1.42zM5.5 7C4.67 7 4 6.33 4 5.5S4.67 4 5.5 4 7 4.67 7 5.5 6.33 7 5.5 7z"/>"#,
        "archive" => r#"<path d="M20.54 5.23l-1.39-1.68C18.88 3.21 18.47 3 18 3H6c-.47 0-.88.21-1.16.55L3.46 5.23C3.17 5.57 3 6.02 3 6.5V19c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V6.5c0-.48-.17-.93-.46-1.27zM12 17.5L6.5 12H10v-2h4v2h3.5L12 17.5zM5.12 5l.81-1h12l.94 1H5.12z"/>"#,
        "themes" => r#"<path d="M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 8 6.5 8 8 8.67 8 9.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"/>"#,
        "tune" => r#"<path d="M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z"/>"#,
        "settings" => r#"<path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/>"#,
        "integrations" => r#"<path d="M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V19c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V21H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z"/>"#,
        "add" => r#"<path d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"/>"#,
        "chevron" => r#"<path d="M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"/>"#,
        "logout" => r#"<path d="M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z"/>"#,
        "open" => r#"<path d="M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"/>"#,
        "delete" => r#"<path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/>"#,
        "edit" => r#"<path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/>"#,
        "search" => r#"<path d="M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/>"#,
        "code" => r#"<path d="M9.4 16.6L4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0l4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z"/>"#,
        "mail" => r#"<path d="M20 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4l-8 5-8-5V6l8 5 8-5v2z"/>"#,
        "send" => r#"<path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>"#,
        "globe" => r#"<path d="M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zm6.93 6h-2.95c-.32-1.25-.78-2.45-1.38-3.56 1.84.63 3.37 1.91 4.33 3.56zM12 4.04c.83 1.2 1.48 2.53 1.91 3.96h-3.82c.43-1.43 1.08-2.76 1.91-3.96zM4.26 14C4.1 13.36 4 12.69 4 12s.1-1.36.26-2h3.38c-.08.66-.14 1.32-.14 2 0 .68.06 1.34.14 2H4.26zm.82 2h2.95c.32 1.25.78 2.45 1.38 3.56-1.84-.63-3.37-1.9-4.33-3.56zm2.95-8H5.08c.96-1.66 2.49-2.93 4.33-3.56C8.81 5.55 8.35 6.75 8.03 8zM12 19.96c-.83-1.2-1.48-2.53-1.91-3.96h3.82c-.43 1.43-1.08 2.76-1.91 3.96zM14.34 14H9.66c-.09-.66-.16-1.32-.16-2 0-.68.07-1.35.16-2h4.68c.09.65.16 1.32.16 2 0 .68-.07 1.34-.16 2zm.25 5.56c.6-1.11 1.06-2.31 1.38-3.56h2.95c-.96 1.65-2.49 2.93-4.33 3.56zM16.36 14c.08-.66.14-1.32.14-2 0-.68-.06-1.34-.14-2h3.38c.16.64.26 1.31.26 2s-.1 1.36-.26 2h-3.38z"/>"#,
        "image" => r#"<path d="M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z"/>"#,
        "link" => r#"<path d="M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z"/>"#,
        "title" => r#"<path d="M5 4v3h5.5v12h3V7H19V4z"/>"#,
        "notes" => r#"<path d="M3 18h12v-2H3v2zM3 6v2h18V6H3zm0 7h18v-2H3v2z"/>"#,
        "excerpt" => r#"<path d="M4 9h16v2H4V9zm0 4h10v2H4v-2z"/>"#,
        "flag" => r#"<path d="M14.4 6L14 4H5v17h2v-7h5.6l.4 2h7V6z"/>"#,
        "star" => r#"<path d="M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/>"#,
        "folder" => r#"<path d="M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/>"#,
        "folder-open" => r#"<path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z"/>"#,
        "file" => r#"<path d="M6 2c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6H6zm7 7V3.5L18.5 9H13z"/>"#,
        "copy" => r#"<path d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"/>"#,
        "check" => r#"<path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/>"#,
        "ghost" => r#"<path d="M12 2a8 8 0 0 0-8 8v10.6c0 .7.85 1.05 1.35.55l1.9-1.9 2.2 2.2c.3.3.78.3 1.08 0L12 20l1.47 1.45c.3.3.78.3 1.08 0l2.2-2.2 1.9 1.9c.5.5 1.35.15 1.35-.55V10a8 8 0 0 0-8-8zm-3 8.5A1.5 1.5 0 1 1 10.5 9 1.5 1.5 0 0 1 9 10.5zm6 0A1.5 1.5 0 1 1 16.5 9 1.5 1.5 0 0 1 15 10.5z"/>"#,
        "menu" => r#"<path d="M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"/>"#,
        "comment" => r#"<path d="M20 2H4c-1.1 0-2 .9-2 2v18l4-4h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 9h12v2H6V9zm8 5H6v-2h8v2zm4-6H6V6h12v2z"/>"#,
        // a node wired to two others: the shape of an automation flow
        // an eye, for page views
        "eye" => r#"<path d="M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zm0 12.5a5 5 0 1 1 0-10 5 5 0 0 1 0 10zm0-8a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"/>"#,
        "flow" => r#"<path d="M5 3a2.5 2.5 0 0 1 2.45 2H11a3 3 0 0 1 3 3v1.05A2.5 2.5 0 1 1 14 14v1a3 3 0 0 1-3 3H7.45a2.5 2.5 0 1 1 0-2H11a1 1 0 0 0 1-1v-1.05A2.5 2.5 0 0 1 12 9.5V8a1 1 0 0 0-1-1H7.45A2.5 2.5 0 1 1 5 3zm0 2a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1zm0 11a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1zm11.5-5a.5.5 0 1 0 0 1 .5.5 0 0 0 0-1z"/>"#,
        "hide" => r#"<path d="M12 7c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92c1.51-1.26 2.7-2.89 3.43-4.75-1.73-4.39-6-7.5-11-7.5-1.4 0-2.74.25-3.98.7l2.16 2.16C10.74 7.13 11.35 7 12 7zM2 4.27l2.28 2.28.46.46C3.08 8.3 1.78 10.02 1 12c1.73 4.39 6 7.5 11 7.5 1.55 0 3.03-.3 4.38-.84l.42.42L19.73 22 21 20.73 3.27 3 2 4.27zM7.53 9.8l1.55 1.55c-.05.21-.08.43-.08.65 0 1.66 1.34 3 3 3 .22 0 .44-.03.65-.08l1.55 1.55c-.67.33-1.41.53-2.2.53-2.76 0-5-2.24-5-5 0-.79.2-1.53.53-2.2z"/>"#,
        "close" => r#"<path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/>"#,
        "up" => r#"<path d="M7.41 15.41L12 10.83l4.59 4.58L18 14l-6-6-6 6z"/>"#,
        "down" => r#"<path d="M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z"/>"#,
        "chart" => r#"<path d="M5 9.2h3V19H5V9.2zm5.6-4.2h2.8v14h-2.8V5zm5.6 8H19v6h-2.8v-6z"/>"#,
        "history" => r#"<path d="M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L8.99 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/>"#,
        "upload" => r#"<path d="M9 16h6v-6h4l-7-7-7 7h4v6zm-4 2h14v2H5v-2z"/>"#,
        "calendar" => r#"<path d="M19 3h-1V1h-2v2H8V1H6v2H5c-1.11 0-1.99.9-1.99 2L3 19a2 2 0 0 0 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V9h14v10zm-9-6H7v-2h3v2zm4 0h-3v-2h3v2zm4 0h-3v-2h3v2z"/>"#,
        "person" => r#"<path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"/>"#,
        "lock" => r#"<path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z"/>"#,
        "share" => r#"<path d="M18 16.08c-.76 0-1.44.3-1.96.77L8.91 12.7c.05-.23.09-.46.09-.7s-.04-.47-.09-.7l7.05-4.11c.54.5 1.25.81 2.04.81 1.66 0 3-1.34 3-3s-1.34-3-3-3-3 1.34-3 3c0 .24.04.47.09.7L8.04 9.81C7.5 9.31 6.79 9 6 9c-1.66 0-3 1.34-3 3s1.34 3 3 3c.79 0 1.5-.31 2.04-.81l7.12 4.16c-.05.21-.08.43-.08.65 0 1.61 1.31 2.92 2.92 2.92s2.92-1.31 2.92-2.92-1.31-2.92-2.92-2.92z"/>"#,
        "layout" => r#"<path d="M3 5v14h18V5H3zm16 2v3H5V7h14zM5 12h5v5H5v-5zm7 5v-5h7v5h-7z"/>"#,
        // a panel open on the right, and the same panel closed
        "panel-hide" => r#"<path d="M3 3h18c1.1 0 2 .9 2 2v14c0 1.1-.9 2-2 2H3c-1.1 0-2-.9-2-2V5c0-1.1.9-2 2-2zm11 2v14h7V5h-7z"/>"#,
        "panel-show" => r#"<path d="M3 3h18c1.1 0 2 .9 2 2v14c0 1.1-.9 2-2 2H3c-1.1 0-2-.9-2-2V5c0-1.1.9-2 2-2zm0 2v14h18V5H3zm11 1.5 1.4-1.4L20.9 12l-5.5 5.5-1.4-1.4 4.1-4.1-4.1-4.5z"/>"#,
        _ => "",
    }
}

fn svg(name: &str) -> String {
    format!(r#"<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">{}</svg>"#, icon(name))
}

/// The Material You design system: a tonal palette generated from Ghost's
/// brand pink (#FF1A75), light + dark schemes, and M3 component styles.
const CSS: &str = r#"
:root {
  --primary: #984059; --on-primary: #fff;
  --primary-container: #FFD9E0; --on-primary-container: #3E071D;
  --secondary-container: #FADDE1; --on-secondary-container: #2B151A;
  --tertiary-container: #FBDEA7; --on-tertiary-container: #291800;
  --error: #BA1A1A; --error-container: #FFDAD6; --on-error-container: #410002;
  --surface: #FFF8F8; --on-surface: #201A1B; --on-surface-variant: #514347;
  --sc-lowest: #FFFFFF; --sc-low: #FAEFF1; --sc: #F4EAEC; --sc-high: #EFDFE3; --sc-highest: #E9DBDF;
  --outline: #847375; --outline-variant: #D7C3C7;
  --inverse-surface: #352F30; --inverse-on-surface: #F9EDEF;
  --blue-bg: #D8E2FF; --blue-fg: #001A41;
  --green-bg: #C4EED0; --green-fg: #0C3D24;
  --amber-bg: #FFDF9E; --amber-fg: #483000;
  --purple-bg: #EADDFF; --purple-fg: #21005D;
  --pink-bg: #FFD9E0; --pink-fg: #3E071D;
  --teal-bg: #BEE9F2; --teal-fg: #0A495B;
  --red-bg: #FFDAD6; --red-fg: #410002;
  --slate-bg: #E0E2EC; --slate-fg: #1B1C22;
  --font: 'Outfit', 'Google Sans', 'Product Sans', system-ui, -apple-system, sans-serif;
  /* Editorial serif for headlines. Fraunces is a variable face with an
     optical-size axis, so a 15px card title and a 30px hero title are drawn
     with different contrast rather than the same outline scaled. */
  --display: 'Fraunces', 'Iowan Old Style', Georgia, 'Times New Roman', serif;
  --mono: 'Roboto Mono', ui-monospace, 'SF Mono', Menlo, monospace;
  --ease: cubic-bezier(.2,0,0,1);
  --shadow-1: 0 1px 2px rgba(0,0,0,.08), 0 1px 3px 1px rgba(0,0,0,.04);
  --shadow-2: 0 1px 2px rgba(0,0,0,.1), 0 2px 6px 2px rgba(0,0,0,.05);
}
@media (prefers-color-scheme: dark) {
  :root {
    --primary: #FFB1C8; --on-primary: #5E1133;
    --primary-container: #7B294B; --on-primary-container: #FFD9E0;
    --secondary-container: #5B4149; --on-secondary-container: #FFD9DE;
    --tertiary-container: #6B5300; --on-tertiary-container: #FFE396;
    --error: #FFB4AB; --error-container: #93000A; --on-error-container: #FFDAD6;
    --surface: #171214; --on-surface: #ECE0E3; --on-surface-variant: #D0C2C6;
    --sc-lowest: #120E10; --sc-low: #1F1A1C; --sc: #241F21; --sc-high: #2F292B; --sc-highest: #3A3436;
    --outline: #998D91; --outline-variant: #4B3F42;
    --inverse-surface: #ECE0E3; --inverse-on-surface: #352F30;
    --blue-bg: #1F3E75; --blue-fg: #AECBFF;
    --green-bg: #173B23; --green-fg: #9CD8AA;
    --amber-bg: #54420A; --amber-fg: #F1CE7F;
    --purple-bg: #4A3085; --purple-fg: #D0BCFF;
    --pink-bg: #672739; --pink-fg: #FFB1C8;
    --teal-bg: #0C4A58; --teal-fg: #8FD8EA;
    --red-bg: #6A1B16; --red-fg: #FFB4AB;
    --slate-bg: #2C2F36; --slate-fg: #C6CAD8;
    --shadow-1: 0 1px 2px rgba(0,0,0,.5); --shadow-2: 0 2px 8px rgba(0,0,0,.55);
  }
}
* { box-sizing: border-box; }
html { scrollbar-color: var(--outline-variant) transparent; }
body { font-family: var(--font); margin: 0; background: var(--surface); color: var(--on-surface);
  font-size: 15px; line-height: 1.5; -webkit-font-smoothing: antialiased; }
::selection { background: var(--primary-container); color: var(--on-primary-container); }
a { color: inherit; }
svg.i { width: 22px; height: 22px; flex: none; display: block; }

/* ---- shell: nav rail + content ---- */
.shell { display: flex; min-height: 100vh; }
.rail { width: 196px; flex: none; padding: 8px 10px 12px; display: flex; flex-direction: column; gap: 2px;
  background: var(--sc-low); position: sticky; top: 0; height: 100vh; overflow-y: auto;
  scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent;
  border-right: 1px solid color-mix(in srgb, var(--outline-variant) 55%, transparent); }
.brand { display: flex; align-items: center; gap: 10px; padding: 10px 10px 14px;
  color: inherit; text-decoration: none; border-radius: 12px; }
.brand:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.brand:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.brand .mark { width: 40px; height: 40px; flex: none; color: var(--primary); display: grid; place-items: center; }
.brand .brand-image { width: 64px; height: 52px; }
.brand .mark img { display: block; width: 100%; height: 100%; object-fit: contain; }
.brand .brand-admin svg { width: 32px; height: 32px; }
.brand .brand-name { min-width: 0; overflow-wrap: anywhere; color: inherit; }
.brand b { font-size: 14px; font-weight: 600; letter-spacing: .1px; display: block; line-height: 1.2; }
.brand span { font-size: 11px; color: var(--on-surface-variant); }
.nav-item { display: flex; align-items: center; gap: 10px; padding: 6px 10px; border-radius: 10px;
  text-decoration: none; color: var(--on-surface-variant); font-size: 13.5px; font-weight: 500;
  transition: background .15s var(--ease), color .15s var(--ease); position: relative; }
.nav-item:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.nav-item.active { background: var(--secondary-container); color: var(--on-secondary-container); font-weight: 600; }
.nav-item .nav-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* The rail runs at its own, tighter scale: the tinted squares elsewhere in
   the admin lead 56px-tall list rows, where 40px is right, but as navigation
   they only have to read at a glance. */
.rail .ic { width: 28px; height: 28px; border-radius: 8px; }
.rail .ic svg { width: 17px; height: 17px; }
.ic { width: 40px; height: 40px; border-radius: 14px; display: grid; place-items: center; flex: none; }
.ic svg { width: 21px; height: 21px; }
.tint-blue { background: var(--blue-bg); color: var(--blue-fg); }
.tint-green { background: var(--green-bg); color: var(--green-fg); }
.tint-amber { background: var(--amber-bg); color: var(--amber-fg); }
.tint-purple { background: var(--purple-bg); color: var(--purple-fg); }
.tint-pink { background: var(--pink-bg); color: var(--pink-fg); }
.tint-teal { background: var(--teal-bg); color: var(--teal-fg); }
.tint-red { background: var(--red-bg); color: var(--red-fg); }
.tint-slate { background: var(--slate-bg); color: var(--slate-fg); }
.rail-foot { margin-top: auto; padding-top: 10px;
  border-top: 1px solid color-mix(in srgb, var(--outline-variant) 55%, transparent); }
.who { display: flex; align-items: center; gap: 8px; padding: 8px 6px 2px; }
.avatar { width: 30px; height: 30px; border-radius: 50%; background: var(--primary); color: var(--on-primary);
  display: grid; place-items: center; font-weight: 600; font-size: 13px; flex: none; }
.avatar.lg { width: 64px; height: 64px; font-size: 26px; object-fit: cover; }
.who b { font-size: 13px; font-weight: 600; display: block; line-height: 1.2; }
.who span { font-size: 11px; color: var(--on-surface-variant); }
.rail .icon-btn { width: 32px; height: 32px; }
.rail .icon-btn svg.i { width: 18px; height: 18px; }
/* the account block is a link to your own profile; keep it looking like the
   block it replaced, and keep long names/emails from pushing out the rail */
.who-link { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0;
  color: inherit; text-decoration: none; border-radius: 999px; padding: 4px; margin: -4px;
  transition: background var(--dur) var(--ease); }
.who-link:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.who-link:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.who-link > div { min-width: 0; }
.who-link b, .who-link span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; }

.main { flex: 1; min-width: 0; }
.content { max-width: 1120px; margin: 0 auto; padding: 0 24px 64px; }

/* Material 3 filled cards for the publication's modules. */
.module-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 20px; margin-top: 20px; }
.module-card { display: flex; flex-direction: column; align-items: flex-start; gap: 20px; min-width: 0;
  padding: 28px; border-radius: 28px; background: var(--sc-low); color: var(--on-surface);
  text-decoration: none; border: 1px solid var(--outline-variant);
  transition: background var(--dur) var(--ease), box-shadow var(--dur) var(--ease); }
.module-card:hover { background: var(--sc-high); box-shadow: var(--shadow-1); }
.module-card:focus-visible { outline: 3px solid var(--primary); outline-offset: 4px; }
.module-card .ic { width: 56px; height: 56px; border-radius: 18px; }
.module-card .ic svg { width: 28px; height: 28px; }
.module-card h2 { margin: 0 0 10px; font-size: 22px; font-weight: 600; letter-spacing: -.3px; }
.module-card p { margin: 0; color: var(--on-surface-variant); font-size: 14px; line-height: 1.65; }
.module-open { display: flex; align-items: center; gap: 8px; margin-top: auto; padding-top: 8px;
  color: var(--primary); font-size: 14px; font-weight: 600; }
.module-open svg { width: 18px; height: 18px; }
@media (max-width: 1100px) { .module-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (max-width: 700px) { .module-grid { grid-template-columns: minmax(0, 1fr); gap: 16px; } }

/* ---- page head (sticky top app bar per page) ---- */
.page-head { position: sticky; top: 0; z-index: 20; display: flex; align-items: center; gap: 16px;
  padding: 22px 4px 16px; margin-bottom: 8px;
  background: color-mix(in srgb, var(--surface) 82%, transparent); backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px); }
.page-head h1 { margin: 0; font-size: 24px; font-weight: 600; letter-spacing: 0; }
.page-head .sub { margin: 2px 0 0; font-size: 13.5px; color: var(--on-surface-variant); }
.page-head .actions { margin-left: auto; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; justify-content: flex-end; }

/* ---- groups + cards + rows ---- */
.section { font-size: 13px; font-weight: 600; color: var(--on-surface-variant);
  padding: 20px 16px 10px; letter-spacing: .2px; }
.card { background: var(--sc-lowest); border-radius: 24px; margin-bottom: 12px; overflow: hidden;
  box-shadow: var(--shadow-1); }
.card.padded { padding: 20px; }
.rows .row { display: flex; align-items: center; gap: 16px; padding: 13px 20px; position: relative;
  text-decoration: none; color: inherit; transition: background .15s var(--ease); }
.rows .row + .row::before { content: ""; position: absolute; top: 0; left: 76px; right: 0; height: 1px;
  background: var(--outline-variant); }
.rows .row:hover { background: color-mix(in srgb, var(--on-surface) 4%, transparent); }
.rows .row:active { background: color-mix(in srgb, var(--on-surface) 8%, transparent); }
.rows .row .body { flex: 1; min-width: 0; }
/* Title over detail, each clipped to the body's width. They used to be plain
   inline spans, so a long value (a post's slug, say) ran past the body box
   and straight through the date, status chip and delete button to its right —
   ellipsis only applies to a block-level box with a width to overflow. */
.rows .row .title { display: block; font-size: 15.5px; font-weight: 500;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rows .row .detail { display: block; font-size: 13px; color: var(--on-surface-variant);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.rows .row .detail code { font-family: var(--mono); font-size: 12px; }
.trailing { margin-left: auto; display: flex; align-items: center; gap: 6px; flex: none; }
.chev { color: var(--on-surface-variant); opacity: .7; }

/* ---- chips ---- */
.chip { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 12px;
  border-radius: 8px; font-size: 12px; font-weight: 600; letter-spacing: .2px; white-space: nowrap; }
.chip.published { background: var(--green-bg); color: var(--green-fg); }
.chip.draft { background: var(--amber-bg); color: var(--amber-fg); }
.chip.scheduled { background: var(--blue-bg); color: var(--blue-fg); }
.chip.sent { background: var(--purple-bg); color: var(--purple-fg); }
.chip.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.chip.error { background: var(--red-bg); color: var(--red-fg); }

/* ---- filter chips (tag toggles) ---- */
.chips { display: flex; flex-wrap: wrap; gap: 8px; }
.fchip { position: relative; display: inline-flex; align-items: center; gap: 0; height: 32px;
  border: 1px solid var(--outline); border-radius: 8px; padding: 0 14px; cursor: pointer;
  font-size: 13.5px; font-weight: 500; color: var(--on-surface-variant); user-select: none;
  transition: all .15s var(--ease); }
.fchip:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.fchip input { position: absolute; opacity: 0; pointer-events: none; }
.fchip .ck { width: 0; overflow: hidden; transition: width .2s var(--ease), margin .2s var(--ease); display: flex; }
.fchip .ck svg { width: 17px; height: 17px; }
.fchip:has(input:checked) { background: var(--secondary-container); border-color: transparent;
  color: var(--on-secondary-container); font-weight: 600; }
.fchip:has(input:checked) .ck { width: 19px; margin-right: 4px; }
.fchip:has(input:focus-visible) { outline: 2px solid var(--primary); outline-offset: 2px; }

/* ---- buttons ---- */
.btn { display: inline-flex; align-items: center; justify-content: center; gap: 8px; height: 42px;
  padding: 0 24px; border-radius: 999px; border: none; background: var(--primary); color: var(--on-primary);
  font-family: inherit; font-size: 14px; font-weight: 600; letter-spacing: .1px; text-decoration: none;
  cursor: pointer; transition: box-shadow .15s var(--ease), transform .1s var(--ease), background .15s var(--ease); }
.btn:hover { box-shadow: var(--shadow-1); }
.btn:active { transform: scale(.97); }
.btn svg.i { width: 18px; height: 18px; }
.btn.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.btn.outlined { background: transparent; border: 1px solid var(--outline); color: var(--primary); }
.btn.text { background: transparent; color: var(--primary); padding: 0 14px; }
.btn.text:hover { background: color-mix(in srgb, var(--primary) 10%, transparent); box-shadow: none; }
.btn.text.danger { color: var(--error); }
.btn.text.danger:hover { background: color-mix(in srgb, var(--error) 10%, transparent); }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0 0 0 0); white-space: nowrap; border: 0; }
.btn.small { height: 34px; padding: 0 16px; font-size: 13px; }
.btn:focus-visible, .icon-btn:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.icon-btn { width: 40px; height: 40px; border-radius: 50%; border: none; background: transparent;
  color: var(--on-surface-variant); display: inline-grid; place-items: center; cursor: pointer; flex: none;
  transition: background .15s var(--ease), color .15s var(--ease); text-decoration: none; }
.icon-btn:hover { background: color-mix(in srgb, var(--on-surface) 8%, transparent); color: var(--on-surface); }
.icon-btn:active { background: color-mix(in srgb, var(--on-surface) 12%, transparent); }
.icon-btn.danger:hover { background: var(--error-container); color: var(--on-error-container); }
.icon-btn svg.i { width: 20px; height: 20px; }

/* ---- file picker ----
   A bare `<input type=file>` draws the operating system's button, the one
   control in the admin that belongs to no design system and looks like it.
   The input stays — it carries the file and the form posts without script —
   and only its two halves are restyled: the box around it, and the button
   itself through `::file-selector-button`. The label on that button and the
   "no file chosen" text beside it are the browser's and cannot be changed
   from CSS, which is the one thing this cannot fix.

   Those two halves are centred with `line-height` and `vertical-align`, not
   with flexbox: they live in the input's shadow tree, which takes no part in
   a flex container declared out here, so `align-items` silently does nothing
   and both sit a few pixels high. The line height is the content box — 42px
   less the two 1px borders. */
.filefield { flex: 1; min-width: 200px; max-width: 420px; height: 42px; box-sizing: border-box;
  display: block; line-height: 40px; padding: 0 12px 0 5px; border: 1px dashed var(--outline);
  border-radius: 12px; background: transparent; color: var(--on-surface-variant);
  font-family: inherit; font-size: 13px; cursor: pointer;
  transition: border-color .15s var(--ease), background .15s var(--ease); }
.filefield:hover { border-color: var(--primary); background: color-mix(in srgb, var(--on-surface) 4%, transparent); }
.filefield:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.filefield::file-selector-button { appearance: none; -webkit-appearance: none; height: 32px;
  vertical-align: middle; margin: 0 12px 0 0; padding: 0 16px; border: none; border-radius: 999px;
  background: var(--secondary-container); color: var(--on-secondary-container);
  font-family: inherit; font-size: 13px; font-weight: 600; letter-spacing: .1px; cursor: pointer;
  transition: background .15s var(--ease); }
.filefield:hover::file-selector-button { background: color-mix(in srgb, var(--on-secondary-container) 8%, var(--secondary-container)); }

/* ---- text fields (M3 outlined, floating label) ---- */
.field { position: relative; display: block; margin-bottom: 14px; }
.field input, .field textarea, .field select { width: 100%; font-family: inherit; font-size: 15px;
  color: var(--on-surface); background: transparent; border: 1px solid var(--outline);
  border-radius: 12px; padding: 22px 14px 6px; outline: none; caret-color: var(--primary);
  transition: border-color .15s var(--ease), box-shadow .15s var(--ease); }
.field textarea { resize: vertical; min-height: 96px; }
.field textarea.code, .field input.code { font-family: var(--mono); font-size: 13px; line-height: 1.6; }
.field > span { position: absolute; left: 11px; top: 15px; font-size: 15px; color: var(--on-surface-variant);
  padding: 0 5px; pointer-events: none; transition: all .15s var(--ease); background: var(--sc-lowest); }
.field input:focus, .field textarea:focus, .field select:focus { border-color: var(--primary);
  box-shadow: inset 0 0 0 1px var(--primary); }
/* Asked of the field, not of the input's next sibling. `input + span` is the
   obvious way to write a floating label and it breaks in the wild: password
   managers inject their own button directly after the input they offer to
   fill, which puts an element between the input and its label and leaves the
   label sitting on top of the value. `:has()` does not care where in the
   field the control is. */
/* A field with a real placeholder floats its label straight away. Most
   fields here carry `placeholder=" "` purely to drive `:placeholder-shown`,
   and for those the label correctly rests over an empty box — but where the
   placeholder is an actual hint ("partials/thing.hbs"), leaving the label at
   rest prints the two on top of each other. */
.field:focus-within > span,
.field:has(input:not(:placeholder-shown)) > span,
.field:has(textarea:not(:placeholder-shown)) > span,
.field:has(input[placeholder]:not([placeholder=" "]):not([placeholder=""])) > span,
.field.fixed > span, .field:has(textarea) > span { top: -8px; font-size: 12px; font-weight: 600; color: var(--primary);
  background: var(--sc-lowest); border-radius: 4px; }
.field:has(textarea.code) > span { background: var(--sc-lowest); }
.field select { appearance: none; cursor: pointer; padding-right: 40px; }
.field .car { position: absolute; right: 12px; top: 50%; transform: translateY(-50%); pointer-events: none;
  color: var(--on-surface-variant); width: 20px; height: 20px; }
.grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.grid2 .field { margin-bottom: 14px; }
.nav-grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 12px; }

/* ---- switch (M3) ---- */
.switch { position: relative; display: inline-flex; flex: none; cursor: pointer; }
.switch input { position: absolute; opacity: 0; width: 100%; height: 100%; margin: 0; cursor: pointer; }
.switch .track { width: 52px; height: 32px; border-radius: 16px; background: var(--sc-highest);
  box-shadow: inset 0 0 0 2px var(--outline); transition: all .2s var(--ease); display: flex; align-items: center; }
.switch .thumb { width: 16px; height: 16px; border-radius: 50%; background: var(--outline);
  margin-left: 6px; transition: all .2s var(--ease); }
.switch input:checked ~ .track { background: var(--primary); box-shadow: none; }
.switch input:checked ~ .track .thumb { width: 24px; height: 24px; background: var(--on-primary); margin-left: auto; margin-right: 6px; }
.switch input:focus-visible ~ .track { outline: 2px solid var(--primary); outline-offset: 3px; }

/* ---- settings toggle row (label + supporting text + switch) ----
   Global so pages outside the editor rail get the same list-item shape;
   the rail keeps its own, tighter scale below. */
.toggle-row { display: flex; align-items: center; gap: 14px; min-height: 48px; }
.toggle-row .label { flex: 1; min-width: 0; }
.toggle-row .label b { display: block; font-size: 14.5px; font-weight: 500; }
.toggle-row .label span { display: block; font-size: 12.5px; line-height: 1.4; color: var(--on-surface-variant); }

/* ---- M3 segmented button ----
   Connected outlined segments; the selected segment inverts to the
   secondary container and reveals a leading check. */
.seg { display: inline-flex; align-items: stretch; height: 40px; border: 1px solid var(--outline);
  border-radius: 999px; overflow: hidden; flex: none; background: var(--sc-lowest); }
.seg > label { display: inline-flex; align-items: center; gap: 0; padding: 0 16px; cursor: pointer;
  font-size: 13.5px; font-weight: 500; color: var(--on-surface-variant); user-select: none;
  border-left: 1px solid var(--outline); transition: background .15s var(--ease), color .15s var(--ease); }
.seg > label:first-of-type { border-left: none; }
.seg > label:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.seg input { position: absolute; opacity: 0; pointer-events: none; }
.seg .ck { width: 0; overflow: hidden; display: flex; transition: width .2s var(--ease), margin .2s var(--ease); }
.seg .ck svg { width: 17px; height: 17px; }
.seg label:has(input:checked) { background: var(--secondary-container); color: var(--on-secondary-container); font-weight: 600; }
.seg label:has(input:checked) .ck { width: 19px; margin-right: 6px; }
.seg label:has(input:focus-visible) { outline: 2px solid var(--primary); outline-offset: -3px; }

/* ---- a field and the button that submits it, on one line ----
   The fields carry 14px of bottom margin for stacked forms, and in a flex row
   that margin is part of the item: `align-items: flex-end` then lines the
   button up with the *margin's* edge, leaving it sitting 14px below the input
   it belongs to. Dropping the margin is what makes the two bottom edges
   actually meet; the row's own gap still spaces things when it wraps. */
.inline-form { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
.inline-form .field { margin-bottom: 0; }

/* ---- compact trailing controls for settings rows ----
   A row already states what the setting is, on the left. The control beside
   it is the value alone: one line, vertically centred, no floating label and
   none of the 22px of head room a label would need. `.field` is the opposite
   — a label *is* its header — so it belongs in a row's body, never here. */
.selbox, .textbox { position: relative; display: inline-flex; flex: none; }
.selbox select, .textbox input { height: 40px; min-width: 150px; max-width: 240px;
  border: 1px solid var(--outline); border-radius: 12px; background: transparent; color: var(--on-surface);
  font-family: inherit; font-size: 13.5px; font-weight: 500; outline: none;
  transition: border-color .15s var(--ease), box-shadow .15s var(--ease); }
.selbox select { appearance: none; -webkit-appearance: none; padding: 0 38px 0 16px; cursor: pointer; }
.textbox { flex: 1; }
.textbox input { width: 100%; padding: 0 14px; }
.selbox select:hover, .textbox input:hover { background: color-mix(in srgb, var(--on-surface) 5%, transparent); }
.selbox select:focus, .textbox input:focus { border-color: var(--primary); box-shadow: inset 0 0 0 1px var(--primary); }
.textbox input:focus { background: transparent; }
.selbox .car { position: absolute; right: 10px; top: 50%; transform: translateY(-50%); pointer-events: none;
  color: var(--on-surface-variant); width: 20px; height: 20px; }

/* ---- colour swatch chips ---- */
.swatches { display: inline-flex; align-items: center; gap: 8px; flex: none; flex-wrap: wrap; justify-content: flex-end; }
.swatch { position: relative; display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 14px 0 10px;
  border: 1px solid var(--outline); border-radius: 999px; cursor: pointer; font-size: 13px; font-weight: 500;
  color: var(--on-surface-variant); user-select: none; background: var(--sc-lowest);
  transition: background .15s var(--ease), color .15s var(--ease); }
.swatch:hover { background: color-mix(in srgb, var(--on-surface) 6%, transparent); }
.swatch input[type=radio] { position: absolute; opacity: 0; pointer-events: none; }
.swatch .dot { width: 22px; height: 22px; border-radius: 50%; box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--on-surface) 18%, transparent); }
/* "no colour" reads as the classic transparent checkerboard */
.swatch .dot.none { background: conic-gradient(var(--sc-highest) 25%, transparent 0 50%, var(--sc-highest) 0 75%, transparent 0) 0 0/10px 10px; }
.swatch:has(input:checked) { background: var(--secondary-container); color: var(--on-secondary-container);
  border-color: transparent; font-weight: 600; box-shadow: inset 0 0 0 1px var(--outline); }
.swatch:has(input:focus-visible) { outline: 2px solid var(--primary); outline-offset: 2px; }
/* The custom chip's dot is the native picker itself, so one click both
   selects the mode and opens the colour picker. */
.swatch.custom { padding: 0 14px 0 6px; }
.swatch.custom input[type=color] { position: static; opacity: 1; pointer-events: auto; width: 28px; height: 28px;
  border: none; border-radius: 50%; background: none; padding: 0; cursor: pointer; flex: none; }
.swatch.custom input[type=color]::-webkit-color-swatch-wrapper { padding: 2px; }
.swatch.custom input[type=color]::-webkit-color-swatch { border-radius: 50%; border: none; }
.swatch.custom:has(input[type=radio]:checked) input[type=color] { outline: 2px solid var(--on-secondary-container); outline-offset: 1px; }
/* Colour rows sit their chips on a second line under the label, where the
   hint and the chips can never fight for the same width. */
.swatches.under { margin-top: 10px; justify-content: flex-start; }

/* Settings pages: supporting lines are allowed to wrap — an ellipsized
   explanation reads as broken, while list pages want the clip. */
.settings .rows .row .detail { white-space: normal; overflow: visible; text-overflow: clip; }
.settings .rows .row .title { white-space: normal; overflow: visible; }
/* Controls that fell out of the trailing slot on a narrow screen line up
   under the text instead of hanging at the card's edge. */
@media (max-width: 919px) {
  .settings .rows .row .trailing { padding-left: 56px; }
  .swatches.under { padding-left: 0; }
}

/* ---- banners, hints, code ---- */
.banner { display: flex; gap: 12px; align-items: flex-start; border-radius: 16px; padding: 14px 18px;
  font-size: 14px; margin-bottom: 12px; }
.banner svg.i { width: 20px; height: 20px; margin-top: 1px; flex: none; }
/* the message is one flex item: without this wrapper every <code> or
   <strong> inside it becomes its own flex item and the sentence breaks
   into columns */
.banner > .msg { flex: 1; min-width: 0; }
.banner.ok { background: var(--green-bg); color: var(--green-fg); }
.banner.error { background: var(--red-bg); color: var(--red-fg); }
.banner.info { background: var(--blue-bg); color: var(--blue-fg); }
code { font-family: var(--mono); }
.key { display: inline-flex; align-items: center; gap: 8px; background: var(--sc);
  border-radius: 8px; padding: 5px 6px 5px 12px; font-family: var(--mono); font-size: 12.5px; }
.key button { border: none; background: transparent; color: inherit; opacity: .6; cursor: pointer;
  display: grid; place-items: center; border-radius: 6px; padding: 3px; }
.key button:hover { opacity: 1; background: color-mix(in srgb, currentColor 12%, transparent); }
.key button svg { width: 15px; height: 15px; }
.muted { color: var(--on-surface-variant); font-size: 13px; }

/* ---- Portal links reference ----
   A label column against a copyable value, repeated. The value wraps rather
   than scrolls: a tier signup URL carries a 24-character id, and a link the
   publisher cannot read in full is a link they cannot check. */
.plinks h3 { margin: 22px 0 10px; font-size: 13px; font-weight: 600; letter-spacing: .02em;
  text-transform: uppercase; color: var(--on-surface-variant);
  border-bottom: 1px solid var(--outline-variant); padding-bottom: 6px; }
.plinks h3:first-of-type { margin-top: 4px; }
.plink { display: flex; align-items: center; gap: 16px; padding: 5px 0; flex-wrap: wrap; }
.plink > .name { flex: none; width: 190px; font-size: 13.5px; font-weight: 500; }
.plink > .key { flex: 1; min-width: 240px; word-break: break-all; }
.plink > .key code { flex: 1; }
@media (max-width: 720px) { .plink > .name { width: 100%; } }

/* ---- signup form preview ----
   The frame is given the height the embed asks for, so what is shown is the
   same box the snippet produces on someone else's page. */
.sf-preview { width: 100%; height: 380px; border: 1px solid var(--outline-variant);
  border-radius: 14px; background: var(--sc-lowest); display: block; }
.sf-preview.minimal { height: 110px; }

/* ---- underline tabs ----
   Two radios drive the panels through `:has()`, so switching tabs costs no
   script and survives a page that has not finished loading. The radios
   themselves are never submitted: they sit outside the forms in the panels. */
.mtabs > input[name="mtab"] { position: absolute; opacity: 0; pointer-events: none; }
.mtab-bar { display: flex; gap: 4px; border-bottom: 1px solid var(--outline-variant);
  margin-bottom: 4px; }
.mtab-bar label { padding: 10px 16px; font-size: 14px; font-weight: 600; cursor: pointer;
  color: var(--on-surface-variant); border-bottom: 2px solid transparent; margin-bottom: -1px;
  transition: color .15s var(--ease), border-color .15s var(--ease); }
.mtab-bar label:hover { color: var(--on-surface); }
.mtabs:has(#mtab-import:checked) label[for="mtab-import"],
.mtabs:has(#mtab-export:checked) label[for="mtab-export"] { color: var(--primary); border-bottom-color: var(--primary); }
.mtabs:has(#mtab-import:checked) [data-tab="export"],
.mtabs:has(#mtab-export:checked) [data-tab="import"] { display: none; }
.mtabs > input:focus-visible + input + .mtab-bar label[for="mtab-import"],
.mtabs > input + input:focus-visible + .mtab-bar label[for="mtab-export"] { outline: 2px solid var(--primary); outline-offset: 2px; border-radius: 6px; }

/* ---- export component picker ----
   The checkbox chips carry two lines each, so they get their own grid rather
   than the inline row `.fchip` is shaped for. */
.expgrid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 10px; }
.fchip.exp { height: auto; align-items: flex-start; padding: 12px 14px; border-radius: 14px;
  text-align: left; gap: 0; }
.fchip.exp .ck { margin-top: 2px; }
.fchip.exp:has(input:checked) .ck { width: 19px; margin-right: 10px; }
.fchip.exp .expl { display: block; min-width: 0; }
.fchip.exp .expl b { display: block; font-size: 14px; font-weight: 600; }
.fchip.exp .expl span { display: block; font-size: 12.5px; line-height: 1.45; font-weight: 400;
  color: var(--on-surface-variant); white-space: normal; margin-top: 2px; }
.fchip.exp:has(input:checked) .expl span { color: inherit; opacity: .8; }

/* ---- file explorer ----
   A tree of <details> elements. `--d` is the row's depth, and the single
   padding rule below is what turns it into indentation, so nesting costs no
   extra markup. The disclosure triangle is suppressed in favour of the
   chevron, which rotates with the open state. */
.ftree { padding: 6px; max-height: 60vh; overflow: auto; font-size: 13px; }
.ftree summary, .ftree .frow { display: flex; align-items: center; gap: 7px;
  padding: 5px 8px 5px calc(8px + var(--d, 0) * 13px); border-radius: 8px;
  color: var(--on-surface); text-decoration: none; cursor: pointer;
  white-space: nowrap; overflow: hidden; }
.ftree summary { list-style: none; font-weight: 500; }
.ftree summary::-webkit-details-marker { display: none; }
.ftree summary:hover, .ftree .frow:hover { background: color-mix(in srgb, var(--on-surface) 7%, transparent); }
.ftree .frow.active { background: var(--secondary-container); color: var(--on-secondary-container); font-weight: 600; }
.ftree .fname { overflow: hidden; text-overflow: ellipsis; font-family: var(--mono); font-size: 12.5px; }
.ftree .fic { flex: none; display: flex; color: var(--on-surface-variant); }
.ftree .fic svg { width: 16px; height: 16px; }
.ftree .frow.active .fic { color: inherit; }
.ftree .tw { flex: none; display: flex; color: var(--on-surface-variant);
  transition: transform .15s var(--ease); }
.ftree .tw svg { width: 16px; height: 16px; }
.ftree details[open] > summary > .tw { transform: rotate(90deg); }
.ftree .chip { margin-left: auto; flex: none; }

/* ---- code editor ----
   A highlighted <pre> sitting exactly under a transparent <textarea>, with a
   line-number gutter beside them. The textarea keeps every native editing
   behaviour — caret, selection, undo, spellcheck off, IME — and the layer
   below only paints. The two must agree on font, size, line-height, padding,
   tab-size and white-space to the pixel, or the text drifts apart as you
   type; that is what the shared `.ced-layer` rule is for. */
.ced { border: 1px solid var(--outline); border-radius: 12px; overflow: hidden;
  background: var(--sc-lowest); }
.ced-bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px;
  background: var(--sc); border-bottom: 1px solid var(--outline-variant);
  font-family: var(--mono); font-size: 12.5px; color: var(--on-surface-variant); }
.ced-bar .lang { margin-left: auto; text-transform: uppercase; letter-spacing: .06em;
  font-size: 11px; font-weight: 600; background: var(--sc-high); border-radius: 999px; padding: 2px 9px; }
/* The gutter, the highlight layer and the textarea are all held to the one
   height the script works out from the line count (`--ced-h`), clamped
   between a usable minimum and a share of the viewport. Letting the gutter
   size itself is what left a tall blank column beside a long file: its one
   div per line ran to the full line count and stretched the flex row, while
   the text beside it stayed at its own fixed height. */
.ced-body { display: flex; align-items: stretch; position: relative; }
.ced-gut { flex: none; padding: 12px 10px 12px 14px; text-align: right; user-select: none;
  background: var(--sc-low); color: var(--on-surface-variant); opacity: .65;
  border-right: 1px solid var(--outline-variant); overflow: hidden;
  height: var(--ced-h, 420px); box-sizing: border-box;
  font-family: var(--mono); font-size: 13px; line-height: 1.6; }
.ced-wrap { position: relative; flex: 1; min-width: 0; }
.ced-layer { margin: 0; padding: 12px 14px; font-family: var(--mono); font-size: 13px;
  line-height: 1.6; tab-size: 2; white-space: pre; overflow: auto;
  border: 0; width: 100%; height: var(--ced-h, 420px); box-sizing: border-box; }
.ced-wrap .ced-hl { position: absolute; inset: 0; pointer-events: none; color: var(--on-surface); }
.ced-wrap textarea.ced-in { position: relative; display: block; resize: none;
  background: transparent; color: transparent; caret-color: var(--primary);
  outline: none; -webkit-text-fill-color: transparent; }
.ced-wrap textarea.ced-in::selection { background: var(--primary-container);
  -webkit-text-fill-color: transparent; }
.ced:focus-within { border-color: var(--primary); box-shadow: inset 0 0 0 1px var(--primary); }

/* Token colours. Hue carries the meaning, so they are held apart in lightness
   too — a reviewer reading this in grayscale still sees structure. */
.tok-com { color: #6E7A8A; font-style: italic; }
.tok-str { color: #B2522F; }
.tok-key { color: #8A3FA8; font-weight: 600; }
.tok-num { color: #1F6F5C; }
.tok-tag { color: #9B2A4A; }
.tok-atr { color: #7A5C12; }
.tok-mus { color: #1D5FA8; font-weight: 600; }
.tok-sel { color: #9B2A4A; }
.tok-prp { color: #1D5FA8; }
.tok-hed { color: #9B2A4A; font-weight: 700; }
@media (prefers-color-scheme: dark) {
  .tok-com { color: #8A95A6; }
  .tok-str { color: #F0A67F; }
  .tok-key { color: #DDA7F0; }
  .tok-num { color: #7FD8BE; }
  .tok-tag { color: #FF9FB6; }
  .tok-atr { color: #E7C77A; }
  .tok-mus { color: #9FC4FF; }
  .tok-sel { color: #FF9FB6; }
  .tok-prp { color: #9FC4FF; }
  .tok-hed { color: #FF9FB6; }
}

/* ---- analytics: stat tiles + chart ---- */
.tiles { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; margin-bottom: 12px; }
.tile { background: var(--sc-lowest); border-radius: 24px; padding: 18px 20px; box-shadow: var(--shadow-1); }
.tile .label { font-size: 13px; font-weight: 500; color: var(--on-surface-variant); }
.tile .value { font-size: 30px; font-weight: 600; font-variant-numeric: proportional-nums;
  line-height: 1.25; margin-top: 2px; }
.tile .delta { font-size: 12.5px; color: var(--on-surface-variant); }
.chart text { font-family: var(--font); fill: var(--on-surface-variant); font-size: 11px; }
.chart .bar { transition: opacity .12s var(--ease); }
.chart g:hover .bar { opacity: .75; }
.range-tabs { display: inline-flex; background: var(--sc); border-radius: 999px; padding: 4px; gap: 4px; }
.range-tabs a { padding: 6px 16px; border-radius: 999px; font-size: 13px; font-weight: 600;
  color: var(--on-surface-variant); text-decoration: none; }
.range-tabs a.active { background: var(--secondary-container); color: var(--on-secondary-container); }
@media (max-width: 919px) { .tiles { grid-template-columns: 1fr 1fr; } }

/* ---- meta data: search and social card previews ----
   Mirrors what Ghost shows above each group of fields: a mock search result,
   and a mock post carrying the card. The shapes are Ghost's; the surfaces and
   type are this admin's own. */
.seo-prev { background: var(--sc); border-radius: 20px; padding: 18px 20px 20px; margin-bottom: 12px; }
.seo-prev .cap { display: block; font-size: 11.5px; font-weight: 600; letter-spacing: .6px;
  text-transform: uppercase; color: var(--on-surface-variant); margin-bottom: 12px; }
/* Both sheets are held to roughly the width the real thing gets, so the
   1.91:1 card image reads as a thumbnail rather than filling the page. */
.seo-prev .sheet { background: var(--sc-lowest); border-radius: 14px; padding: 16px;
  box-shadow: var(--shadow-1); max-width: 620px; margin: 0 auto; }
.seo-prev.social .sheet { max-width: 480px; }
/* search result */
.seo-prev .site { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; }
.seo-prev .favicon { width: 28px; height: 28px; border-radius: 999px; flex: none;
  background: var(--secondary-container) center/contain no-repeat; }
.seo-prev .host { font-size: 13.5px; line-height: 1.25; }
.seo-prev .addr { font-size: 12px; color: var(--on-surface-variant); word-break: break-all; }
.seo-prev .g-title { display: block; font-size: 19px; line-height: 1.3; color: #1a0dab; margin-bottom: 2px; }
.seo-prev .g-desc { font-size: 13.5px; color: var(--on-surface-variant); }
/* social post */
.seo-prev .who { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; }
.seo-prev .who .avatar { width: 40px; height: 40px; border-radius: 999px; flex: none;
  background: var(--secondary-container) center/cover no-repeat; }
.seo-prev .who .name { font-weight: 600; font-size: 14.5px; line-height: 1.2; }
.seo-prev .who .when { font-size: 13px; color: var(--on-surface-variant); }
.seo-prev .bar { height: 11px; border-radius: 999px; background: var(--outline-variant); margin-bottom: 8px; }
.seo-prev .bar.short { width: 60%; margin-bottom: 14px; }
.seo-prev .card-mock { border: 1px solid var(--outline-variant); border-radius: 14px; overflow: hidden; }
.seo-prev .card-mock .shot { display: block; aspect-ratio: 1.91 / 1; background: var(--sc); }
.seo-prev .card-mock .shot img { display: block; width: 100%; height: 100%; object-fit: cover; }
.seo-prev .card-mock .shot.empty { display: grid; place-items: center;
  font-size: 13px; color: var(--on-surface-variant); }
.seo-prev .card-mock .body { padding: 12px 14px; border-top: 1px solid var(--outline-variant); }
.seo-prev .card-mock .body .t { display: block; font-weight: 600; font-size: 14.5px; line-height: 1.35; }
.seo-prev .card-mock .body .d { display: block; font-size: 13px; color: var(--on-surface-variant);
  margin-top: 3px; line-height: 1.4; }
.seo-prev .card-mock .body .u { display: block; font-size: 12px; color: var(--on-surface-variant);
  margin-top: 6px; text-transform: lowercase; }

/* ---- themes: the official theme gallery ---- */
.gallery { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; margin-bottom: 12px; }
/* The thumbnails are Ghost's own marketplace screenshots, 600x750, served
   back by the admin itself so they can carry a long cache header. The box
   keeps its height whether or not the image arrives, so a missing thumbnail
   leaves a tinted panel rather than collapsing the card. */
.gallery .shot { display: block; position: relative; aspect-ratio: 4 / 5; overflow: hidden;
  background: var(--sc); border-bottom: 1px solid var(--outline-variant); }
.gallery .shot img { display: block; width: 100%; height: 100%;
  object-fit: cover; object-position: top center; }
.gallery .shot::after { content: ""; position: absolute; inset: 0;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--on-surface) 6%, transparent); }
.gallery .meta { display: flex; align-items: center; gap: 10px; padding: 14px 16px; }
.gallery .meta .body { flex: 1; min-width: 0; }
.gallery .meta .title { display: block; font-size: 15px; font-weight: 600; }
.gallery .meta .detail { display: block; font-size: 12.5px; color: var(--on-surface-variant); }
.gallery .actions { display: flex; align-items: center; gap: 8px; padding: 0 16px 14px; }
.gallery .actions form { display: contents; }
.gallery .actions .btn { flex: 1; }
@media (max-width: 1100px) { .gallery { grid-template-columns: 1fr 1fr; } }
@media (max-width: 719px) { .gallery { grid-template-columns: 1fr; } }

/* ---- post/page editor: Koenig canvas + an always-open settings rail ----
   Ghost hides the post settings behind a slide-over panel; here they are a
   second column that is simply always there, so the editor needs more room
   than the 860px the rest of the admin reads best at. */
.content:has(.editor-shell) { max-width: 1400px; }
.editor-shell { display: grid; grid-template-columns: minmax(0, 1fr) 316px; gap: 24px; align-items: start; }
/* The left gutter is where Koenig parks the ＋ card button, which it
   positions outside the text column. Without the room it would hang off the
   edge of the editor; with it, the button sits inside and the title stays
   aligned with the body text. */
.editor-main { min-width: 0; padding-left: 72px; position: relative; }
.editor-title { width: 100%; border: none; outline: none; background: transparent; resize: none; overflow: hidden;
  font-family: var(--font); font-size: 30px; font-weight: 700; line-height: 1.2; letter-spacing: -.3px;
  color: var(--on-surface); padding: 0; margin: 2px 0 10px; caret-color: var(--primary); display: block; }
.editor-title::placeholder { color: color-mix(in srgb, var(--on-surface-variant) 42%, var(--surface)); opacity: 1; font-weight: 500; }
#koenig-mount { min-height: 52vh; }
/* Koenig paints its own surface; this only gives the canvas somewhere to sit
   and keeps its floating toolbars above the sticky page head. */
.koenig-react-editor { position: relative; z-index: 1; }
/* The word count, as Ghost has it since its own editor's redesign: a quiet
   pill floating in the bottom corner of the canvas, not a footer that has to
   be scrolled to. It says nothing until there are words to count. */
.wc-pill { position: absolute; right: 0; bottom: 10px; z-index: 6; padding: 6px 14px; border-radius: 999px;
  background: color-mix(in srgb, var(--sc-lowest) 82%, transparent); backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px); color: var(--on-surface-variant); font-size: 12.5px;
  font-weight: 500; pointer-events: none; box-shadow: var(--shadow-1); }
.wc-pill:empty { display: none; }
#editor-view[hidden] { display: none; }

/* The save state beside the header's Save button: "Saving…", "Saved", "*
   Unsaved changes" while the autosave is beaten only by the typing. */
.save-state { font-size: 12.5px; color: var(--on-surface-variant); align-self: center; white-space: nowrap; }
.save-state:empty { display: none; }
.save-state.saved { color: var(--green-fg); }
.save-state.error { color: var(--error); }
.save-state.unsaved { font-style: italic; }

/* Koenig ships Ghost's own typography — Inter headings over a Georgia body,
   sized for Ghost's admin. Here it reads as a foreign document dropped into
   the page, so the prose is restated in this admin's typeface and scale.
   The #id prefix is needed to outweigh Koenig's own rules: its stylesheet is
   injected by the bundle, so it lands after this one with equal specificity. */
#koenig-mount .kg-prose :where(p, blockquote, aside, ul, ol, li):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) {
  font-family: var(--font); font-size: 1.65rem; line-height: 1.65em; letter-spacing: 0; }
#koenig-mount .kg-prose :where(h1, h2, h3, h4, h5, h6):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) {
  font-family: var(--font); color: var(--on-surface); letter-spacing: -.2px; }
#koenig-mount .kg-prose :where(h1):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) { font-size: 2.8rem; }
#koenig-mount .kg-prose :where(h2):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) { font-size: 2.3rem; }
#koenig-mount .kg-prose :where(h3):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) { font-size: 1.95rem; }
#koenig-mount .kg-prose :where(h4, h5, h6):not(:where(.not-kg-prose, [class~=not-kg-prose] *)) { font-size: 1.7rem; }
#koenig-mount .koenig-lexical > * { color: var(--on-surface); }

/* Keep breakout cards inside the editor.
   Koenig sizes "wide" and "full" cards against the viewport — `75vw` and
   `100vw`, pulled back by a `--kg-breakout-adjustment` — because Ghost's
   editor is a narrow column of text in an otherwise empty page, so a card
   can spill into the margins. Here the settings rail is in those margins and
   the column is the full width already, which sent full-width cards out past
   the rail and under the window edge.
   The viewport maths is replaced outright rather than re-tuned: the
   adjustment would have to track the rail, the gutter and the toggle, and
   `75vw` is narrower than this column anyway, so a "wide" card would have
   come out smaller than a regular one. Cards render at the column's width
   while editing; `cardWidth` is untouched, so the post still breaks out on
   the site. */
#koenig-mount [data-kg-card] {
  width: 100% !important;
  min-width: 0 !important;
  max-width: 100% !important;
  margin-left: 0 !important;
  margin-right: 0 !important;
  transform: none !important;
  left: auto !important;
  right: auto !important;
}
/* Nothing inside a card may push past it either — an image carrying its own
   intrinsic width, say. */
#koenig-mount [data-kg-card] img,
#koenig-mount [data-kg-card] video,
#koenig-mount [data-kg-card] iframe { max-width: 100%; }
#koenig-mount .koenig-lexical { overflow-x: clip; }
/* The "Begin writing…" placeholder is a separate element with its own font. */
#koenig-mount .koenig-lexical .font-serif { font-family: var(--font); }
#koenig-mount .editor-body-placeholder { color: color-mix(in srgb, var(--on-surface-variant) 42%, var(--surface)); font-weight: 400; }
/* The card menu's rows are left exactly as Koenig draws them — the icon,
   label and spacing are Ghost's. Only the panel around them is resized: each
   row is a label on the left and a keyboard hint on the right, and this
   build never passes a hint, so at Ghost's 312px the right half of every row
   is permanently blank. Narrowing the panel makes it hug its contents.
   The width is a Tailwind utility class in the bundle's own stylesheet,
   which is injected after this one — hence the attribute selector, which
   sidesteps escaping the brackets, and `!important` to outrank it. */
.koenig-lexical ul[class*="w-[312px]"] { width: 232px !important; }

/* ---- the settings rail, and the control that hides it ---- */
.editor-side { position: sticky; top: 76px; max-height: calc(100vh - 96px); overflow-y: auto;
  display: flex; flex-direction: column; gap: 10px; padding: 2px 4px 24px 0;
  scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent; }
/* Flex children shrink to fit a capped container by default, which collapsed
   every panel to a sliver and overlapped the one below it. The rail scrolls;
   the panels keep their own height. */
.editor-side > * { flex: none; }
.editor-side .card { margin: 0; }
#side-toggle { position: absolute; opacity: 0; pointer-events: none; }
.side-btn { cursor: pointer; }
.side-btn .when-shown { display: inline-flex; }
.side-btn .when-hidden { display: none; }
body:has(#side-toggle:checked) .editor-side { display: none; }
body:has(#side-toggle:checked) .editor-shell { grid-template-columns: minmax(0, 1fr); }
body:has(#side-toggle:checked) .side-btn .when-shown { display: none; }
body:has(#side-toggle:checked) .side-btn .when-hidden { display: inline-flex; }
/* With the rail gone the editor takes the whole column back. */
body:has(#side-toggle:checked) .editor-main { max-width: none; }

.panel > summary { display: flex; align-items: center; gap: 11px; padding: 12px 15px; cursor: pointer;
  list-style: none; font-size: 13.5px; font-weight: 600; user-select: none; }
.panel > summary::-webkit-details-marker { display: none; }
.panel > summary .ic { width: 30px; height: 30px; border-radius: 9px; }
.panel > summary .ic svg { width: 16px; height: 16px; }
.panel > summary .twist { margin-left: auto; color: var(--on-surface-variant); transition: transform .2s var(--ease); }
.panel > summary .twist svg { width: 18px; height: 18px; display: block; }
.panel[open] > summary .twist { transform: rotate(180deg); }
.panel > summary:hover { background: color-mix(in srgb, var(--on-surface) 4%, transparent); }
.panel .panel-body { padding: 2px 15px 15px; }
.panel .panel-body > .field:last-child { margin-bottom: 0; }
/* The shared field is sized for a 860px settings page; in a 316px rail the
   same padding reads as oversized, so the rail runs a tighter variant. */
.panel .field { margin-bottom: 11px; }
.panel .field input, .panel .field textarea, .panel .field select {
  font-size: 13.5px; padding: 17px 12px 5px; border-radius: 10px; }
.panel .field textarea { min-height: 62px; }
.panel .field textarea.code { font-size: 12px; }
.panel .field > span { background: var(--sc-lowest); font-size: 13.5px; top: 12px; left: 9px; }
.panel .field:focus-within > span,
.panel .field:has(input:not(:placeholder-shown)) > span,
.panel .field:has(textarea:not(:placeholder-shown)) > span,
.panel .field.fixed > span, .panel .field:has(textarea) > span { top: -7px; font-size: 11px; }
.panel .field .car { width: 18px; height: 18px; right: 10px; }
.panel .fchip { height: 28px; padding: 0 11px; font-size: 12.5px; }
/* A hint sits tight under the field it explains — but the fields carry their
   own 14px of bottom margin, which the negative top margin absorbs. After
   anything else (chips, a select) it needs its own breathing room. */
.panel .hint { font-size: 12px; color: var(--on-surface-variant); margin: -6px 0 14px; }
.panel .hint:first-child, .panel .chips + .hint { margin-top: 10px; }
.panel .panel-body > .hint:last-child { margin-bottom: 0; }
.panel .toggle-row { padding: 8px 0; min-height: 0; }
.panel .toggle-row .label b { font-size: 14px; }
.panel .toggle-row .label span { font-size: 12px; }
.panel .chips { max-height: 220px; overflow-y: auto; }
.editor-side .btn.text.danger { width: 100%; justify-content: center; }
/* ---- the feature image box ---- */
.fimg-drop { position: relative; border-radius: 14px; overflow: hidden; background: var(--sc);
  margin-bottom: 11px; }
.fimg-empty { display: grid; place-items: center; aspect-ratio: 16 / 10; padding: 12px; }
.fimg-btns { display: flex; gap: 8px; }
/* Ghost draws its image actions over the picture as translucent pills. The
   fixed light surface keeps them legible over any photograph, in either
   colour scheme — the danger variant keeps the red only for Remove. */
.btn.ghosted { background: rgba(255,255,255,.86); color: #201A1B; backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px); }
.btn.ghosted:hover { box-shadow: var(--shadow-1); }
.btn.ghosted.danger { color: #BA1A1A; }
.fimg-drop img { display: none; width: 100%; aspect-ratio: 16 / 10; object-fit: cover; }
.fimg-drop.is-set img { display: block; }
.fimg-drop.is-set .fimg-empty { display: none; }
.fimg-acts { display: none; position: absolute; left: 0; right: 0; bottom: 10px; justify-content: center; gap: 6px; }
.fimg-drop.is-set .fimg-acts { display: flex; }
.fimg-prog { display: none; position: absolute; inset: auto 0 0 0; height: 4px; background: color-mix(in srgb, var(--on-surface) 12%, transparent); }
.fimg-prog.on { display: block; }
.fimg-prog i { display: block; height: 100%; width: 0; background: var(--primary); transition: width .15s linear; }

/* The feature image's own fields, folded behind one line: the box above is
   what the panel is opened for; the alt text, caption and URL are set once
   and rarely revisited. */
.panel-sub { border-top: 1px solid var(--outline-variant); margin-top: 12px; }
.panel-sub > summary { display: flex; align-items: center; gap: 6px; padding: 9px 0 2px; cursor: pointer;
  list-style: none; font-size: 12.5px; font-weight: 600; color: var(--on-surface-variant); user-select: none; }
.panel-sub > summary::-webkit-details-marker { display: none; }
.panel-sub > summary .twist { margin-left: auto; }
.panel-sub > summary .twist svg { width: 15px; height: 15px; display: block; transition: transform .2s var(--ease); }
.panel-sub[open] > summary .twist svg { transform: rotate(180deg); }
.panel-sub .panel-sub-body { padding-top: 10px; }
.panel-sub .field:last-child { margin-bottom: 2px; }

/* ---- the Unsplash picker — laid out like the one in Ghost's editor ----.
   The shared `dialog` rule above clamps confirm dialogs to 420px; the picker
   is a different animal, so every clamp it sets is restated here. */
dialog.unsplash { width: min(1920px, calc(100vw - 24px)); height: calc(100vh - 24px);
  height: calc(100dvh - 24px); max-height: none;
  min-width: 0; max-width: none; border: none; border-radius: 16px; padding: 0;
  background: var(--sc-lowest); color: var(--on-surface); box-shadow: var(--shadow-2);
  overflow: hidden; }
dialog.unsplash[open] { display: grid; grid-template-rows: auto minmax(0, 1fr); }
dialog.unsplash [hidden] { display: none !important; }
dialog.unsplash::backdrop { background: rgba(0,0,0,.6); }
.u-close { display: grid; place-items: center; width: 40px; height: 40px; border: none; background: transparent;
  color: var(--on-surface-variant); cursor: pointer; padding: 0; border-radius: 50%; }
.u-close:hover { background: color-mix(in srgb, var(--on-surface) 8%, transparent); color: var(--on-surface); }
.u-close svg.i { width: 18px; height: 18px; display: block; }
.u-head { display: grid; grid-template-columns: 1fr minmax(240px, 480px) 40px; align-items: center; gap: 20px;
  padding: 20px 24px; border-bottom: 1px solid var(--outline-variant); }
.u-head h3 { margin: 0; font-size: 22px; font-weight: 700; letter-spacing: -.3px; white-space: nowrap; }
.u-searchbox { position: relative; min-width: 0; }
.u-searchbox svg.i { position: absolute; left: 15px; top: 50%; transform: translateY(-50%);
  width: 17px; height: 17px; color: var(--on-surface-variant); pointer-events: none; }
.u-searchbox input { width: 100%; height: 40px; border-radius: 999px; border: 1px solid var(--outline);
  background: transparent; color: var(--on-surface); font: inherit; font-size: 14px;
  padding: 0 16px 0 42px; outline: none; }
.u-searchbox input:focus { border-color: var(--primary); box-shadow: 0 0 0 2px color-mix(in srgb, var(--primary) 15%, transparent); }
.u-searchbox input::placeholder { color: var(--on-surface-variant); }
.u-body { position: relative; min-height: 0; overflow-y: auto; overscroll-behavior: contain;
  padding: 24px; scrollbar-width: thin;
  scrollbar-color: var(--outline-variant) transparent; }
/* Four masonry columns keep each photograph's original proportions. */
.u-grid { columns: 4; column-gap: 16px; }
@media (max-width: 1100px) { .u-grid { columns: 3; } }
@media (max-width: 800px) { .u-grid { columns: 2; } }
@media (max-width: 500px) { .u-grid { columns: 1; } }
@media (max-width: 600px) {
  .u-head { grid-template-columns: minmax(0, 1fr) 40px; padding: 16px; gap: 12px; }
  .u-searchbox { grid-row: 2; grid-column: 1 / -1; }
  .u-close { grid-row: 1; grid-column: 2; }
  .u-body { padding: 16px; }
}
.u-tile { position: relative; min-width: 0; break-inside: avoid; margin-bottom: 16px; border-radius: 10px;
  overflow: hidden; cursor: zoom-in; }
.u-tile > img { display: block; width: 100%; height: auto; }
.u-hover { position: absolute; inset: 0; display: flex; flex-direction: column; justify-content: space-between;
  padding: 12px; opacity: 0; transition: opacity .15s var(--ease);
  background: linear-gradient(180deg, rgba(0,0,0,.15), transparent 40%, rgba(0,0,0,.65)); }
.u-tile:hover .u-hover, .u-tile:focus-within .u-hover { opacity: 1; }
@media (hover: none) { .u-hover { opacity: 1; } }
.u-top { display: flex; justify-content: flex-end; gap: 8px; }
.u-chip { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px;
  border-radius: 999px; background: rgba(255,255,255,.85); color: #201A1B; font-size: 12.5px;
  font-weight: 600; text-decoration: none; backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px); }
.u-chip svg.i { width: 15px; height: 15px; }
.u-chip:hover { background: #fff; }
.u-bottom { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.u-credit { display: inline-flex; align-items: center; gap: 8px; color: #fff; font-size: 13px;
  font-weight: 500; min-width: 0; text-shadow: 0 1px 3px rgba(0,0,0,.5); }
.u-credit-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.u-credit > img.u-avatar { display: block; width: 26px; height: 26px; min-width: 26px; max-width: 26px;
  object-fit: cover; border-radius: 50%; flex: none; }
.u-insert { border: none; height: 34px; padding: 0 12px; border-radius: 999px; background: #fff;
  color: #201A1B; font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; flex: none; }
.u-insert:hover { box-shadow: var(--shadow-1); }
dialog.unsplash button:focus-visible, .u-chip:focus-visible { outline: 2px solid var(--primary); outline-offset: 3px; }
.u-empty { column-span: all; padding: 9vh 0; text-align: center; color: var(--on-surface-variant);
  font-size: 14px; }
.u-zoom { display: flex; justify-content: center; align-items: center; height: 100%; cursor: zoom-out; }
.u-zoom .u-tile { cursor: zoom-out; aspect-ratio: auto; margin: 0; max-width: 100%; max-height: 100%; }
.u-zoom .u-tile > img { max-height: calc(100dvh - 220px); height: auto; width: auto; max-width: 100%; object-fit: contain; }
/* Ghost's spinner: a ring with a dot, hung in the middle of the gallery. */
.u-loading { position: absolute; inset: 0; display: grid; place-items: center; pointer-events: none; }
.u-loading i { display: block; width: 46px; height: 46px; border-radius: 50%;
  border: 1px solid color-mix(in srgb, var(--on-surface) 10%, transparent); position: relative;
  animation: u-spin 1s linear infinite; }
.u-loading i::before { content: ""; position: absolute; top: 6px; left: 50%; margin-left: -3.5px;
  width: 7px; height: 7px; border-radius: 50%; background: var(--on-surface-variant); }
@keyframes u-spin { to { transform: rotate(360deg); } }

/* Below this the two columns leave the editor too narrow to write in — the
   rail, the gutter and the settings take ~680px before any text — so the
   settings drop underneath and the editor takes the width back. */
@media (max-width: 1199px) {
  .editor-shell, body:has(#side-toggle:checked) .editor-shell { grid-template-columns: minmax(0, 1fr); }
  .editor-side { position: static; max-height: none; overflow: visible; padding-right: 0; }
  .editor-main { padding-left: 0; }
  .editor-title { font-size: 26px; }
}

/* ---- centered auth pages ---- */
.auth { min-height: 100vh; display: grid; place-items: center; padding: 24px; position: relative; overflow: hidden; }
.content:has(> .auth) { max-width: none; padding: 0; }
.auth .blob { position: absolute; border-radius: 50%; filter: blur(90px); opacity: .45; pointer-events: none; }
.auth .b1 { width: 420px; height: 420px; background: var(--pink-bg); top: -140px; left: -120px; }
.auth .b2 { width: 380px; height: 380px; background: var(--blue-bg); bottom: -140px; right: -100px; }
.auth .b3 { width: 300px; height: 300px; background: var(--purple-bg); bottom: -80px; left: 18%; opacity: .35; }
.auth-box { width: min(400px, 100%); position: relative; animation: rise .5s var(--ease) both; }
.auth-logo { width: 60px; height: 60px; border-radius: 20px; background: var(--primary-container);
  color: var(--on-primary-container); display: grid; place-items: center; margin: 0 auto 20px; }
.auth-logo svg { width: 38px; height: 38px; }
.auth-box h1 { text-align: center; font-size: 26px; font-weight: 600; margin: 0 0 6px; }
.auth-box .sub { text-align: center; color: var(--on-surface-variant); font-size: 14px; margin: 0 0 28px; }
.auth-box .card { padding: 26px; }
.auth-box .btn { width: 100%; height: 46px; font-size: 15px; margin-top: 4px; }

/* ---- search bar ---- */
.search { display: flex; align-items: center; gap: 12px; background: var(--sc-lowest); border-radius: 999px;
  box-shadow: var(--shadow-1); padding: 0 18px; height: 46px; margin: 4px 0 14px; color: var(--on-surface-variant); }
.search input { flex: 1; border: none; outline: none; background: transparent; font-family: inherit;
  font-size: 14.5px; color: var(--on-surface); height: 100%; }

/* ---- post list: the posts and pages index ---------------------------------
   Rows, not cards. The column order follows Ghost's own list — thumbnail,
   title and byline, what happened to it, then the numbers — because a list
   of a hundred posts is scanned down a column, and fixed column widths are
   the whole reason rows beat cards here.
--------------------------------------------------------------------------- */

/* tools row: search takes the space, sort sits at the end */
.list-tools { display: flex; gap: 12px; align-items: center; margin: 4px 0 16px; }
.list-tools .search { flex: 1; margin: 0; }
.sort-picker { position: relative; display: flex; align-items: center; flex: none; }
.sort-picker select { appearance: none; font-family: inherit; font-size: 14px; font-weight: 500;
  height: 46px; padding: 0 38px 0 18px; border-radius: 999px; border: none; cursor: pointer;
  background: var(--sc-lowest); color: var(--on-surface); box-shadow: var(--shadow-1); }
.sort-picker .car { position: absolute; right: 12px; width: 20px; height: 20px; pointer-events: none;
  color: var(--on-surface-variant); }
.sort-picker select:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }

/* status tabs */
.seg-bar { display: flex; flex-wrap: wrap; gap: 8px; margin: 2px 0 16px; }
.seg { display: inline-flex; align-items: center; gap: 8px; height: 36px; padding: 0 8px 0 16px;
  border-radius: 999px; font-size: 13.5px; font-weight: 500; text-decoration: none;
  color: var(--on-surface-variant); background: transparent;
  border: 1px solid var(--outline-variant); transition: all .18s var(--ease); }
.seg:hover { background: color-mix(in srgb, var(--on-surface) 5%, transparent); color: var(--on-surface); }
.seg.on { background: var(--secondary-container); color: var(--on-secondary-container);
  border-color: transparent; font-weight: 600; }
.seg-count { display: inline-grid; place-items: center; min-width: 22px; height: 22px; padding: 0 6px;
  border-radius: 999px; font-size: 11.5px; font-weight: 600; font-variant-numeric: tabular-nums;
  background: color-mix(in srgb, var(--on-surface) 8%, transparent); color: inherit; }
.seg.on .seg-count { background: color-mix(in srgb, var(--on-secondary-container) 14%, transparent); }

/* ---- the list ---- */
.post-list { background: var(--sc-lowest); border-radius: 20px; overflow: hidden;
  box-shadow: var(--shadow-1); }
.post-row { display: flex; align-items: center; position: relative; }
.post-row + .post-row::before { content: ""; position: absolute; top: 0; left: 122px; right: 16px;
  height: 1px; background: var(--outline-variant); }
.post-row:hover { background: color-mix(in srgb, var(--on-surface) 4%, transparent); }
.post-row:focus-within { background: color-mix(in srgb, var(--on-surface) 4%, transparent); }

.post-row-link { flex: 1; min-width: 0; display: flex; align-items: center; gap: 16px;
  padding: 11px 6px 11px 16px; text-decoration: none; color: inherit; }
.post-row-link:focus-visible { outline: 2px solid var(--primary); outline-offset: -3px;
  border-radius: 12px; }

/* thumbnail — Ghost uses an 80–100px 16:10 tile */
.post-row-thumb { position: relative; width: 90px; aspect-ratio: 16 / 10; flex: none;
  border-radius: 9px; overflow: hidden; background: var(--sc-high); }
.post-row-img { width: 100%; height: 100%; object-fit: cover; display: block; }
/* no feature image: a gradient keyed to the post id, so the column still
   carries some recognition when scanning */
.post-row-blank { position: absolute; inset: 0; display: grid; place-items: center;
  background: linear-gradient(135deg, var(--from), var(--to)); }
.post-row-blank span { font-size: 25px; font-weight: 700; line-height: 1;
  color: rgba(255,255,255,.78); }
.post-row-star { position: absolute; right: 4px; top: 4px; width: 19px; height: 19px;
  display: grid; place-items: center; border-radius: 50%; color: #fff;
  background: rgba(0,0,0,.5); backdrop-filter: blur(4px); }
.post-row-star svg.i { width: 11px; height: 11px; }

/* title and byline */
.post-row-main { flex: 1; min-width: 0; }
.post-row-title { margin: 0; font-family: var(--display); font-optical-sizing: auto;
  font-size: 15.5px; font-weight: 600; line-height: 1.3; letter-spacing: -.008em;
  color: var(--on-surface); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  transition: color .18s var(--ease); }
.post-row:hover .post-row-title { color: var(--primary); }
.post-row-meta { margin: 2px 0 0; font-size: 12.5px; color: var(--on-surface-variant);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.post-row-dot { opacity: .45; margin: 0 1px; }

/* status column */
.post-row-status { width: 192px; flex: none; display: flex; flex-direction: column;
  align-items: flex-start; gap: 3px; }
.post-row-chip { display: inline-flex; align-items: center; gap: 5px; font-size: 12.5px;
  font-weight: 600; color: var(--on-surface-variant); }
.post-row-chip::before { content: ""; width: 7px; height: 7px; border-radius: 50%;
  background: currentColor; flex: none; }
/* Ghost colours the dot by state: pink for a draft, green for scheduled */
.post-row-chip.draft { color: var(--primary); }
.post-row-chip.scheduled { color: var(--green-fg); }
.post-row-chip.published, .post-row-chip.sent { color: var(--on-surface-variant); }
@media (prefers-color-scheme: dark) { .post-row-chip.scheduled { color: var(--green-fg); } }
.post-row-detail { font-size: 11.8px; color: var(--on-surface-variant); opacity: .8;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; }

/* the numbers — fixed-width cells so they line up down the page */
.post-row-stats { flex: none; display: flex; }
.post-stat { width: 66px; flex: none; display: flex; flex-direction: column; align-items: flex-end;
  gap: 1px; font-size: 11px; color: var(--on-surface-variant); }
.post-stat b { font-weight: 600; font-size: 13.5px; color: var(--on-surface);
  font-variant-numeric: tabular-nums; letter-spacing: -.01em; }


/* delete, revealed on hover — it must not be the thing you hit while
   reaching for the post */
.post-row-action { flex: none; padding-right: 8px; opacity: 0;
  transition: opacity .16s var(--ease); }
.post-row:hover .post-row-action, .post-row:focus-within .post-row-action { opacity: 1; }
.post-row-action .icon-btn { width: 34px; height: 34px; }
.post-row-action .icon-btn svg.i { width: 18px; height: 18px; }
@media (hover: none) { .post-row-action { opacity: 1; } }

/* ---- page atmosphere ----
   A warm wash behind the head and a fine grain over everything, so the page
   has some depth instead of being a flat fill behind a floating slab. */
.content:has(.post-list)::before { content: ""; position: fixed; inset: 0 0 auto 0; height: 420px;
  pointer-events: none; z-index: 0;
  background:
    radial-gradient(60% 100% at 18% 0%, color-mix(in srgb, var(--primary) 10%, transparent), transparent 70%),
    radial-gradient(44% 86% at 88% 4%, color-mix(in srgb, var(--teal-fg) 6%, transparent), transparent 70%); }
.content:has(.post-list)::after { content: ""; position: fixed; inset: 0; pointer-events: none;
  z-index: 0; opacity: .03; mix-blend-mode: multiply;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='140' height='140'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='3'/%3E%3C/filter%3E%3Crect width='140' height='140' filter='url(%23n)'/%3E%3C/svg%3E"); }
@media (prefers-color-scheme: dark) {
  .content:has(.post-list)::after { mix-blend-mode: screen; opacity: .045; }
}
.content:has(.post-list) > * { position: relative; z-index: 1; }

/* No fill on the sticky header here. Elsewhere its translucent surface is
   invisible against a flat page; over the wash below it reads as a lighter
   band across the top. The backdrop blur stays, so rows scrolling under it
   are still frosted rather than running into the title. */
.content:has(.post-list) .page-head { background: transparent; }

/* the list heading borrows the editorial face, so the page has one voice */
.content:has(.post-list) .page-head h1,
.content:has(.seg-bar) .page-head h1 { font-family: var(--display); font-optical-sizing: auto;
  font-weight: 600; letter-spacing: -.02em; font-size: 27px; }

/* a row is a lot of columns; drop them as the space runs out, numbers first
   since the title is what you are actually looking for */
@media (max-width: 1040px) {
  .post-row-stats .post-stat:nth-child(3) { display: none; }
}
@media (max-width: 900px) {
  .post-row-stats { display: none; }
  .post-row-status { width: 150px; }
}
@media (max-width: 680px) {
  .post-row-status { display: none; }
  .post-row-thumb { width: 64px; }
  .post-row + .post-row::before { left: 92px; }
  .post-row-link { gap: 12px; padding-left: 12px; }
  .post-row-action { opacity: 1; }
}

/* ---- color input ---- */
.color-field { display: flex; align-items: center; gap: 14px; }
.color-field input[type=color] { width: 44px; height: 44px; border-radius: 50%; border: none; cursor: pointer;
  background: none; padding: 0; flex: none; }
.color-field input[type=color]::-webkit-color-swatch-wrapper { padding: 3px; }
/* The hairline ring is what keeps a white or near-white value legible: a
   #ffffff swatch on a white card is otherwise an empty hole. */
.color-field input[type=color]::-webkit-color-swatch { border-radius: 50%; border: 2px solid var(--surface);
  box-shadow: var(--shadow-1), 0 0 0 1px var(--outline-variant); }

/* ---- dialog + snackbar ---- */
dialog { border: none; border-radius: 28px; background: var(--sc-lowest); color: var(--on-surface);
  padding: 24px; min-width: 320px; max-width: min(420px, 90vw); box-shadow: var(--shadow-2); }
dialog::backdrop { background: rgba(0,0,0,.4); }
dialog h3 { margin: 0 0 8px; font-size: 20px; font-weight: 600; }
dialog p { margin: 0 0 22px; font-size: 14px; color: var(--on-surface-variant); }
dialog .d-actions { display: flex; justify-content: flex-end; gap: 8px; }
#toast, #publish-toast { position: fixed; left: 50%; bottom: 24px; transform: translate(-50%, 80px); background: var(--inverse-surface);
  color: var(--inverse-on-surface); border-radius: 12px; padding: 12px 20px; font-size: 14px; font-weight: 500;
  box-shadow: var(--shadow-2); transition: transform .25s var(--ease), opacity .25s var(--ease); opacity: 0;
  z-index: 100; max-width: 90vw; }
#toast.show, #publish-toast.show { transform: translate(-50%, 0); opacity: 1; }

/* ---- entrance motion ---- */
@keyframes rise { from { opacity: 0; transform: translateY(14px); } }
.rise { animation: rise .45s var(--ease) both; animation-delay: calc(var(--i, 0) * 70ms); }
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation: none !important; transition: none !important; }
}

/* ---- responsive: rail becomes a top strip ---- */
@media (max-width: 919px) {
  .shell { flex-direction: column; }
  .rail { width: 100%; height: auto; position: static; flex-direction: column; overflow: visible; padding-bottom: 0; }
  .brand { padding: 14px 8px 10px; }
  .rail .nav-scroll { display: flex; gap: 4px; overflow-x: auto; padding: 4px 8px 12px; scrollbar-width: none; }
  .rail .nav-scroll::-webkit-scrollbar { display: none; }
  .nav-item { flex: none; padding: 8px 14px 8px 10px; }
  .nav-item .ic { display: none; }
  .rail-foot { margin: 0; padding: 0 8px 10px; }
  .who { padding: 6px 6px; }
  .content { padding: 0 16px 48px; }
  .page-head { flex-wrap: wrap; }
  .grid2 { grid-template-columns: 1fr; }
  .nav-grid { grid-template-columns: 1fr; }
  .rows .row + .row::before { left: 20px; }

  /* List rows are a single nowrap flex line on desktop, which on a phone
     made the title, slug, date, status chip and delete button pile onto each
     other and overflow sideways. Let the row wrap: leading icon + body on
     the first line, trailing meta on its own line under them. */
  .rows .row { flex-wrap: wrap; gap: 10px 12px; padding: 12px 16px; }
  .rows .row .body { flex: 1 1 100%; min-width: 0; order: 2; }
  .rows .row > .ic, .rows .row > .avatar { order: 1; }
  .rows .row .title { white-space: normal; overflow-wrap: anywhere; }
  .rows .row .detail { white-space: normal; overflow-wrap: anywhere; }
  /* trailing drops below and left-aligns instead of being pushed off-screen
     by margin-left:auto */
  .trailing { order: 3; flex: 1 1 100%; margin-left: 0; flex-wrap: wrap;
    padding-left: 56px; justify-content: flex-start; }
  /* …except where the leading icon is hidden, so nothing is indented past it */
  .rows .row:not(:has(> .ic)):not(:has(> .avatar)) .trailing { padding-left: 0; }
}

/* Very narrow phones: drop the leading-icon indent entirely so titles get
   the full width. */
@media (max-width: 479px) {
  .rows .row { padding: 12px 14px; }
  .trailing { padding-left: 0; }
  .rows .row + .row::before { left: 14px; }
}
"#;

/// Tiny progressive-enhancement script: confirm dialogs, list filtering,
/// copy-to-clipboard with a snackbar. Everything works without it too.
const JS: &str = r#"
(function () {
  document.querySelectorAll('[data-brand-image]').forEach(function (row) {
    var url = row.querySelector('[data-image-url]');
    var file = row.querySelector('input[type="file"]');
    var preview = row.querySelector('img');
    var upload = row.querySelector('[data-image-upload]');
    var remove = row.querySelector('[data-image-remove]');
    var status = row.querySelector('[role="status"]');
    var form = url.form;
    var unsplash = row.querySelector('[data-image-unsplash]');
    if (unsplash) unsplash.addEventListener('click', function () {
      window.GhostUnsplash.open({
        headers: JSON.parse(form.dataset.unsplash || 'null'),
        onSelect: function (src) {
          url.value = src;
          refresh();
          status.textContent = 'Cover selected. Save to apply.';
        }
      });
    });
    function refresh() {
      var value = url.value.trim();
      preview.hidden = !value;
      if (value) preview.src = value;
      else preview.removeAttribute('src');
      remove.hidden = !value;
    }
    url.addEventListener('input', refresh);
    upload.addEventListener('click', function () { file.click(); });
    remove.addEventListener('click', function () {
      url.value = '';
      refresh();
      status.textContent = 'Image removed. Save to apply.';
    });
    file.addEventListener('change', async function () {
      var image = file.files[0];
      if (!image) return;
      var pending = Number(form.dataset.pendingUploads || 0) + 1;
      form.dataset.pendingUploads = String(pending);
      upload.disabled = remove.disabled = url.disabled = true;
      if (unsplash) unsplash.disabled = true;
      status.textContent = 'Uploading…';
      try {
        var data = new FormData();
        data.append('file', image);
        var response = await fetch(row.dataset.uploadUrl, {method: 'POST', credentials: 'same-origin', body: data});
        var result = await response.json().catch(function () { return {}; });
        if (!response.ok) throw new Error(result.errors?.[0]?.message || 'Upload failed. Please try again.');
        var uploadedUrl = result.images?.[0]?.url;
        if (!uploadedUrl) throw new Error('The upload did not return an image URL.');
        url.value = uploadedUrl;
        refresh();
        status.textContent = 'Image uploaded. Save to apply.';
      } catch (error) {
        status.textContent = error.message || 'Upload failed. Please try again.';
      } finally {
        upload.disabled = remove.disabled = url.disabled = false;
        if (unsplash) unsplash.disabled = false;
        file.value = '';
        form.dataset.pendingUploads = String(Number(form.dataset.pendingUploads) - 1);
      }
    });
    refresh();
  });
  var form = document.getElementById('design-form');
  if (form) form.addEventListener('submit', function (event) {
    if (Number(form.dataset.pendingUploads) > 0) {
      event.preventDefault();
      form.querySelectorAll('[data-brand-image] [role="status"]').forEach(function (status) {
        if (status.textContent === 'Uploading…') status.textContent = 'Please wait for the upload to finish before saving.';
      });
    }
  });
})();
(function () {
  // M3 confirm dialog for forms carrying data-confirm.
  //
  // `pending` has to live outside the handler: the dialog's close listener is
  // registered once, so if it closed over the form from the first submit it
  // would re-submit that same form every time afterwards — on a page with
  // several delete buttons, confirming the second one would act on the first.
  var dlg = null, pending = null, pendingBtn = null;
  document.addEventListener('submit', function (e) {
    var f = e.target;
    if (!f.dataset.confirm || f.dataset.confirmed) return;
    e.preventDefault();
    pending = f;
    pendingBtn = e.submitter || null;
    if (!dlg) {
      dlg = document.createElement('dialog');
      // The buttons must live in a <form method="dialog">. A bare button
      // inside a <dialog> does not close it and never sets returnValue, so
      // without this the Confirm button is inert and nothing is ever
      // submitted — which is what made every delete look broken.
      dlg.innerHTML = '<h3></h3><p></p><form method="dialog" class="d-actions">' +
        '<button class="btn text" value="cancel">Cancel</button>' +
        '<button class="btn tonal" value="ok">Confirm</button></form>';
      document.body.appendChild(dlg);
      dlg.addEventListener('close', function () {
        var form = pending, btn = pendingBtn;
        pending = null; pendingBtn = null;
        if (dlg.returnValue !== 'ok' || !form) return;
        form.dataset.confirmed = '1';
        // requestSubmit keeps the clicked button's name/value in the payload;
        // form.submit() would drop it and break action-carrying buttons
        if (form.requestSubmit) { form.requestSubmit(btn || undefined); }
        else { form.submit(); }
      });
    }
    var parts = f.dataset.confirm.split('|');
    dlg.querySelector('h3').textContent = parts[0];
    dlg.querySelector('p').textContent = parts[1] || 'This cannot be undone.';
    dlg.showModal();
  });

  // Meta data previews follow the fields as they are typed into. Each
  // preview element is tagged data-prev="<field>", and empty fields fall back
  // the way the front end does: the specific tag, then the meta tag, then the
  // site's own title/description.
  var form = document.querySelector('[data-meta-previews]');
  if (form && document.querySelector('[data-prev]')) {
    var val = function (name) {
      var el = form.elements[name];
      return el ? el.value.trim() : '';
    };
    // The site title and description are edited on this same form now, so
    // they are read live and the stored values are only the starting point.
    var site = { title: form.dataset.siteTitle || '', description: form.dataset.siteDescription || '' };
    var put = function (key, text) {
      var el = document.querySelector('[data-prev="' + key + '"]');
      if (el) { el.textContent = text; }
    };
    var refresh = function () {
      var metaTitle = val('meta_title') || val('title') || site.title;
      var metaDesc = val('meta_description') || val('description') || site.description;
      put('search-title', metaTitle);
      put('search-description', metaDesc);
      put('twitter-title', val('twitter_title') || metaTitle);
      put('twitter-description', val('twitter_description') || metaDesc);
      put('og-title', val('og_title') || metaTitle);
      put('og-description', val('og_description') || metaDesc);
      [['twitter_image', 'twitter-shot'], ['og_image', 'og-shot']].forEach(function (pair) {
        var slot = document.querySelector('[data-prev="' + pair[1] + '"]');
        if (!slot) return;
        var url = val(pair[0]);
        if (url) {
          slot.className = 'shot';
          slot.innerHTML = '';
          var img = document.createElement('img');
          img.alt = '';
          img.src = url;
          slot.appendChild(img);
        } else {
          slot.className = 'shot empty';
          slot.textContent = 'No image set';
        }
      });
    };
    form.addEventListener('input', refresh);
  }

  // client-side list filtering
  var q = document.querySelector('[data-filter]');
  if (q) {
    var rows = Array.from(document.querySelectorAll('[data-filterable]'));
    q.addEventListener('input', function () {
      var v = q.value.toLowerCase();
      rows.forEach(function (r) { r.style.display = r.textContent.toLowerCase().includes(v) ? '' : 'none'; });
    });
  }

  // settings hub search: typing swaps the page list for a flat index of
  // pages, sub-pages, headings and fields, so any setting is one query away
  // no matter which page buries it. The haystack rides in data-search
  // because a row's visible text says what a thing is, not everything it
  // should answer to.
  var hub = document.getElementById('settings-search');
  if (hub) {
    var pages = document.getElementById('settings-groups');
    var hitsCard = document.getElementById('settings-hits');
    var emptyRow = document.getElementById('settings-empty');
    var hits = Array.from(document.querySelectorAll('#settings-hits [data-search]'));
    hub.addEventListener('input', function () {
      var v = hub.value.trim().toLowerCase();
      var shown = 0;
      hits.forEach(function (h) {
        var on = !!v && h.dataset.search.toLowerCase().indexOf(v) !== -1;
        h.style.display = on ? '' : 'none';
        if (on) shown++;
      });
      // a field only shows while something is being asked; clearing the box
      // restores the pages, or every field in the index would appear
      pages.style.display = v ? 'none' : '';
      hitsCard.style.display = v ? '' : 'none';
      emptyRow.style.display = v && !shown ? '' : 'none';
    });
  }

  // colour settings: picking a custom colour selects the custom mode too, so
  // the swatch chip and the picker can never disagree about which is in force
  document.addEventListener('input', function (e) {
    if (e.target.type !== 'color') return;
    var chip = e.target.closest('.swatch.custom');
    if (!chip) return;
    var radio = chip.querySelector('input[type=radio]');
    if (radio) radio.checked = true;
  });

  // navigation editor: reorder/remove rows in place instead of round-tripping
  // (the action= buttons still work without JS — the server applies them)
  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-nav-move], [data-nav-remove]');
    if (!b) return;
    e.preventDefault();
    var row = b.closest('.nav-row');
    if (!row) return;
    if (b.hasAttribute('data-nav-remove')) { row.remove(); return; }
    if (b.dataset.navMove === 'up' && row.previousElementSibling) {
      row.parentElement.insertBefore(row, row.previousElementSibling);
    } else if (b.dataset.navMove === 'down' && row.nextElementSibling) {
      row.parentElement.insertBefore(row.nextElementSibling, row);
    }
  });

  // copy-to-clipboard + snackbar
  var toast = null, timer = null;
  function say(msg) {
    if (!toast) { toast = document.createElement('div'); toast.id = 'toast'; document.body.appendChild(toast); }
    toast.textContent = msg; toast.classList.add('show');
    clearTimeout(timer); timer = setTimeout(function () { toast.classList.remove('show'); }, 2200);
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest('[data-copy]');
    if (!b) return;
    navigator.clipboard.writeText(b.dataset.copy).then(function () { say('Copied to clipboard'); },
      function () { say('Copy failed'); });
  });

  // ---- Portal links reference ----
  // Every row carries both spellings of the same destination: the href a
  // theme puts in an <a>, and the data-portal attribute that turns any
  // element into the same trigger. The switch rewrites the shown value and
  // the copy button together, so what is copied is always what is read.
  // Tier rows hold a template with {tier} in it, filled from the picker.
  function paintPortalLinks() {
    var root = document.querySelector('[data-plinks]');
    if (!root) return;
    var mode = root.querySelector('input[name="portal_link_mode"]:checked');
    var attrs = mode && mode.value === 'attrs';
    var tierSel = root.querySelector('[data-plink-tier]');
    var tier = tierSel ? tierSel.value : '';
    root.querySelectorAll('[data-plink]').forEach(function (row) {
      var value = (attrs ? row.dataset.attr : row.dataset.link).split('{tier}').join(tier);
      var out = row.querySelector('code');
      var btn = row.querySelector('[data-copy]');
      if (out) out.textContent = value;
      if (btn) btn.dataset.copy = value;
    });
    var note = root.querySelector('[data-plink-note]');
    if (note) note.textContent = attrs ? 'data attributes' : 'links';
  }
  document.addEventListener('change', function (e) {
    if (e.target.name === 'portal_link_mode' || e.target.hasAttribute('data-plink-tier')) {
      paintPortalLinks();
    }
  });
  paintPortalLinks();

  // ---- code editor ----
  // Highlighting is a paint pass over the textarea's own value, so nothing
  // here can lose or reorder what you typed: if the tokenizer throws, the
  // worst case is uncoloured text over a working textarea.
  function esc(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
  // Walk `re` over `src`, handing each match to `paint`. Text between matches
  // goes to `rest`, which defaults to escaping it — passing a `rest` is how a
  // second pass gets layered over the gaps the first one left.
  function scan(src, re, paint, rest) {
    var out = '', last = 0, m;
    re.lastIndex = 0;
    while ((m = re.exec(src)) !== null) {
      // A zero-width match would spin forever; step past it.
      if (m.index === re.lastIndex) { re.lastIndex++; continue; }
      var gap = src.slice(last, m.index);
      out += rest ? rest(gap) : esc(gap);
      out += paint(m);
      last = m.index + m[0].length;
    }
    var tail = src.slice(last);
    return out + (rest ? rest(tail) : esc(tail));
  }
  function span(cls, text) { return '<span class="' + cls + '">' + esc(text) + '</span>'; }

  var MUSTACHE = /\{\{\{?[\s\S]*?\}?\}\}/g;
  // A quoted attribute value, where a mustache may carry quotes of its own:
  // `src="{{img_url x size="m"}}"`. Consuming a whole mustache as one
  // alternative is what stops the inner `"` from closing the value early —
  // pair them naively and the rest of the tag colours as a string.
  var ATTR_VALUE = '"(?:\\{\\{[\\s\\S]*?\\}\\}|[^"])*"|\'(?:\\{\\{[\\s\\S]*?\\}\\}|[^\'])*\'';

  // String-coloured, except for the mustaches inside it.
  function paintValue(v) {
    return '<span class="tok-str">' +
      scan(v, MUSTACHE, function (m) { return span('tok-mus', m[0]); }) +
      '</span>';
  }

  // Inside one `<tag ...>`: the name, its attribute names, and their values.
  function paintTag(tag) {
    return scan(
      tag,
      new RegExp('(' + ATTR_VALUE + ')|(\\{\\{\\{?[\\s\\S]*?\\}?\\}\\})|([\\w:@.-]+)(?==)|(<\\/?[A-Za-z][\\w:-]*|\\/?>)', 'g'),
      function (m) {
        if (m[1]) return paintValue(m[1]);
        if (m[2]) return span('tok-mus', m[2]);
        if (m[3]) return span('tok-atr', m[3]);
        return span('tok-tag', m[4]);
      }
    );
  }

  var JS_WORDS = /\b(const|let|var|function|return|if|else|for|while|of|in|new|class|extends|import|export|from|default|async|await|try|catch|finally|throw|typeof|instanceof|this|null|undefined|true|false|break|continue|switch|case|do|delete|void|yield|static)\b/;

  var LANGS = {
    // Handlebars is HTML plus mustaches, so one pass handles both. Comments
    // come first in the alternation: `{{!-- <div> --}}` is a comment, not a
    // tag, and the leftmost-longest rule only holds if it is offered first.
    hbs: function (src) {
      // A tag runs to the `>` that is not inside one of its attribute values,
      // so `<a title="a > b">` stays one tag; the alternation spells that out
      // rather than using `[^>]*`.
      var TAG = '<\\/?[A-Za-z][\\w:-]*(?:' + ATTR_VALUE + '|\\{\\{[\\s\\S]*?\\}\\}|[^>"\'])*\\/?>';
      return scan(
        src,
        new RegExp('(\\{\\{!--[\\s\\S]*?--\\}\\}|<!--[\\s\\S]*?-->)|(\\{\\{\\{?[\\s\\S]*?\\}?\\}\\})|(' + TAG + ')|(<![A-Za-z][^>]*>)', 'g'),
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-mus', m[2]);
          if (m[3]) return paintTag(m[3]);
          return span('tok-key', m[4]);
        }
      );
    },
    css: function (src) {
      return scan(
        src,
        /(\/\*[\s\S]*?\*\/)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(@[\w-]+)|(#[0-9a-fA-F]{3,8}\b)|([{;]\s*)([\w-]+)(\s*:)|(-?\d*\.?\d+(?:px|em|rem|%|vh|vw|vmin|vmax|s|ms|deg|fr|ch|ex|pt)?\b)/g,
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-str', m[2]);
          if (m[3]) return span('tok-key', m[3]);
          if (m[4]) return span('tok-num', m[4]);
          // A property is only a property after `{` or `;` — anchoring on
          // that is what keeps `a:hover` a selector rather than a declaration.
          if (m[6]) return esc(m[5]) + span('tok-prp', m[6]) + esc(m[7]);
          return span('tok-num', m[8]);
        }
      );
    },
    js: function (src) {
      return scan(
        src,
        /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|(`(?:[^`\\]|\\.)*`|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(\b(?:const|let|var|function|return|if|else|for|while|of|in|new|class|extends|import|export|from|default|async|await|try|catch|finally|throw|typeof|instanceof|this|null|undefined|true|false|break|continue|switch|case|do|delete|void|yield|static)\b)|(\b\d+\.?\d*\b)/g,
        function (m) {
          if (m[1]) return span('tok-com', m[1]);
          if (m[2]) return span('tok-str', m[2]);
          if (m[3]) return span('tok-key', m[3]);
          return span('tok-num', m[4]);
        }
      );
    },
    json: function (src) {
      return scan(
        src,
        /("(?:[^"\\]|\\.)*")(\s*:)?|(\btrue\b|\bfalse\b|\bnull\b)|(-?\d+\.?\d*(?:[eE][+-]?\d+)?)/g,
        function (m) {
          // A string followed by a colon is a key, and reads differently.
          if (m[1]) return span(m[2] ? 'tok-prp' : 'tok-str', m[1]) + (m[2] ? esc(m[2]) : '');
          if (m[3]) return span('tok-key', m[3]);
          return span('tok-num', m[4]);
        }
      );
    },
    md: function (src) {
      return scan(
        src,
        /(^```[\s\S]*?^```|^~~~[\s\S]*?^~~~)|(^#{1,6} .*$)|(^>.*$)|(`[^`\n]+`)|(\*\*[^*\n]+\*\*|__[^_\n]+__)|(\[[^\]\n]*\]\([^)\n]*\))/gm,
        function (m) {
          if (m[1]) return span('tok-str', m[1]);
          if (m[2]) return span('tok-hed', m[2]);
          if (m[3]) return span('tok-com', m[3]);
          if (m[4]) return span('tok-str', m[4]);
          if (m[5]) return span('tok-key', m[5]);
          return span('tok-mus', m[6]);
        }
      );
    },
    txt: function (src) { return esc(src); },
  };

  function paintEditor(ed) {
    var ta = ed.querySelector('.ced-in');
    var hl = ed.querySelector('.ced-hl');
    var gut = ed.querySelector('.ced-gut');
    if (!ta || !hl) return;
    var src = ta.value;
    var lang = LANGS[ed.dataset.lang] || LANGS.txt;
    var painted;
    try {
      painted = lang(src);
    } catch (err) {
      painted = esc(src);
    }
    // A value ending in a newline leaves the last (empty) line with nothing
    // to give the block height, so the highlight stops one row short of the
    // caret. The trailing newline keeps the two in step.
    hl.innerHTML = painted + '\n';
    var lines = src.split('\n').length;
    if (gut && gut.childElementCount !== lines) {
      var nums = '';
      for (var i = 1; i <= lines; i++) nums += '<div>' + i + '</div>';
      gut.innerHTML = nums;
    }
    // Size to the content, so a twelve-line partial is not framed by half a
    // screen of nothing and a long stylesheet still gets a usable window.
    // The line box is measured rather than assumed: it follows the font the
    // browser actually resolved, which is not always the one asked for.
    var lineH = (gut && gut.firstElementChild && gut.firstElementChild.offsetHeight) || 21;
    var min = 320;
    var max = Math.max(min, Math.round(window.innerHeight * 0.68));
    var want = lines * lineH + 24; // the layers' 12px of padding, top and bottom
    ed.style.setProperty('--ced-h', Math.min(Math.max(want, min), max) + 'px');
    hl.scrollTop = ta.scrollTop;
    hl.scrollLeft = ta.scrollLeft;
    if (gut) gut.scrollTop = ta.scrollTop;
  }

  // Indent or outdent the lines the selection touches; with no selection,
  // Tab is just two spaces. Without this, Tab leaves the editor entirely.
  function tabKey(ta, outdent) {
    var v = ta.value, s = ta.selectionStart, e = ta.selectionEnd;
    var lineStart = v.lastIndexOf('\n', s - 1) + 1;
    if (s === e && !outdent) {
      ta.value = v.slice(0, s) + '  ' + v.slice(e);
      ta.selectionStart = ta.selectionEnd = s + 2;
      return;
    }
    var lineEnd = v.indexOf('\n', e);
    if (lineEnd === -1) lineEnd = v.length;
    var block = v.slice(lineStart, lineEnd);
    var next = outdent
      ? block.replace(/^[ \t]{1,2}/gm, '')
      : block.replace(/^/gm, '  ');
    ta.value = v.slice(0, lineStart) + next + v.slice(lineEnd);
    ta.selectionStart = lineStart;
    ta.selectionEnd = lineStart + next.length;
  }

  // Enter keeps the current line's indentation, which is most of what an
  // editor's auto-indent is worth in a template.
  function enterKey(ta) {
    var v = ta.value, s = ta.selectionStart;
    var lineStart = v.lastIndexOf('\n', s - 1) + 1;
    var indent = (v.slice(lineStart, s).match(/^[ \t]*/) || [''])[0];
    if (!indent) return false;
    ta.value = v.slice(0, s) + '\n' + indent + v.slice(ta.selectionEnd);
    ta.selectionStart = ta.selectionEnd = s + 1 + indent.length;
    return true;
  }

  var editors = Array.from(document.querySelectorAll('.ced'));
  if (editors.length) {
    // The height cap is a share of the viewport, so it has to be recomputed
    // when the viewport changes.
    window.addEventListener('resize', function () {
      editors.forEach(paintEditor);
    });
  }
  editors.forEach(function (ed) {
    var ta = ed.querySelector('.ced-in');
    if (!ta) return;
    paintEditor(ed);
    ta.addEventListener('input', function () { paintEditor(ed); });
    ta.addEventListener('scroll', function () { paintEditor(ed); });
    ta.addEventListener('keydown', function (e) {
      if (e.key === 'Tab') {
        e.preventDefault();
        tabKey(ta, e.shiftKey);
        paintEditor(ed);
      } else if (e.key === 'Enter' && !e.shiftKey) {
        if (enterKey(ta)) { e.preventDefault(); paintEditor(ed); }
      } else if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        // Reflex from every other editor; without it the browser offers to
        // save the admin page as a file instead.
        var form = ta.form || document.getElementById(ta.getAttribute('form'));
        if (form) { e.preventDefault(); form.requestSubmit(); }
      }
    });
  });
})();
"#;

/// Side navigation. `active` is the nav key of the current page ("" for none),
/// `user` supplies the account block at the bottom of the rail.
fn rail(active: &str, user: Option<&ghost_core::models::User>, role: Role) -> String {
    let active = match active {
        "analytics" | "comments" | "automations" => "modules",
        other => other,
    };
    // Showing a link that only leads to "Not allowed" is worse than not
    // showing it, so the rail is filtered by the same capabilities the
    // handlers enforce.
    let items: &[(&str, &str, &str, &str, &str)] = &[
        ("posts", "/ghost/posts/", "Posts", "posts", "blue"),
        ("pages", "/ghost/pages/", "Pages", "pages", "green"),
        ("tags", "/ghost/tags/", "Tags", "tags", "amber"),
        ("themes", "/ghost/themes/", "Themes", "themes", "slate"),
        ("members", "/ghost/members/", "Members", "ghost", "blue"),
        ("modules", "/ghost/modules/", "Modules", "integrations", "purple"),
        // Integrations is not a rail item: like Ghost, it lives under
        // Settings → Integrations.
        ("settings", "/ghost/settings/", "Settings", "settings", "pink"),
    ];
    let mut links: String = items
        .iter()
        .filter(|(key, _, _, _, _)| match *key {
            // Ghost grants the `comment` permission set to Administrator and
            // above only, so moderation is not offered below that.
            "modules" | "themes" | "design" | "navigation" | "members" | "settings" => role.is_admin(),
            "tags" => role.can_manage_tags(),
            _ => true,
        })
        .map(|(key, href, label, ic, tint)| {
            format!(
                r#"<a class="nav-item{cls}" href="{href}"><span class="ic tint-{tint}">{svg}</span><span class="nav-label">{label}</span></a>"#,
                cls = if *key == active { " active" } else { "" },
                href = href,
                tint = tint,
                svg = svg(ic),
                label = esc(label),
            )
        })
        .collect();
    links.push_str(&format!(
        r#"<a class="nav-item" href="/" target="_blank" rel="noopener"><span class="ic tint-blue">{open}</span><span class="nav-label">View site</span></a>"#,
        open = svg("open"),
    ));
    let (initial, name, email) = match user {
        Some(u) => (
            u.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string(),
            esc(u.name.as_deref().unwrap_or("Owner")),
            esc(u.email.as_deref().unwrap_or("")),
        ),
        None => ("G".into(), "Owner".into(), String::new()),
    };
    // The avatar and name open the signed-in user's profile, the way Ghost's
    // own account block does. With no session there is no profile to open, so
    // it stays inert rather than linking somewhere that would bounce to
    // signin.
    let who_inner = format!(
        r#"<span class="avatar">{initial}</span><div><b>{name}</b><span>{email}</span></div>"#,
        initial = esc(&initial),
        name = name,
        email = email,
    );
    let who = match user.and_then(|u| u.slug.as_deref()) {
        // The rail is narrow enough that a full name or address is usually
        // ellipsed, so the tooltip carries them rather than restating what
        // the link obviously is.
        Some(slug) => format!(
            r#"<a class="who-link" href="/ghost/settings/staff/{slug}/" title="{name} · {email}">{who_inner}</a>"#,
            slug = esc(slug),
            name = name,
            email = email,
            who_inner = who_inner,
        ),
        None => who_inner,
    };
    format!(
        r#"<aside class="rail">
<a class="brand" href="/ghost/analytics/" title="Analytics">{brand_mark}<span class="brand-name"><b>{site_name}</b></span></a>
<div class="nav-scroll">{links}</div>
<div class="rail-foot">
<div class="who">{who}
<a class="icon-btn" href="/ghost/signout/" title="Sign out">{logout}</a></div>
</div>
</aside>"#,
        brand_mark = admin_brand_mark(),
        site_name = esc(&admin_site_name()),
        links = links,
        who = who,
        logout = svg("logout"),
    )
}

/// Full HTML document. `nav` pages get the side rail (with the signed-in
/// `user` in the account block); auth/setup pages render bare.
/// Default shell. The role defaults to the *least* privileged, so a page
/// that forgets to say who is looking shows a minimal rail rather than
/// advertising screens the viewer cannot open.
fn page_u(title: &str, active: &str, user: Option<&ghost_core::models::User>, body: &str, nav: bool) -> Html<String> {
    page_for(title, active, user, Role::Contributor, body, nav)
}

/// `page_u` for a handler that already proved the viewer holds `role`.
fn page_for_role(role: Role, title: &str, active: &str, user: Option<&ghost_core::models::User>, body: &str, nav: bool) -> Html<String> {
    page_for(title, active, user, role, body, nav)
}

/// `page_u` with an explicit role, so the rail can be filtered.
fn page_for(title: &str, active: &str, user: Option<&ghost_core::models::User>, role: Role, body: &str, nav: bool) -> Html<String> {
    let shell = if nav {
        format!(r#"<div class="shell">{rail}<main class="main"><div class="content">{body}</div></main></div>"#, rail = rail(active, user, role))
    } else {
        format!(r#"<div class="content" style="max-width:640px;padding-top:40px">{body}</div>"#)
    };
    Html(format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — {site_name}</title>
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='%23984059'%3E%3Cpath d='M12 2a8 8 0 0 0-8 8v10.6c0 .7.85 1.05 1.35.55l1.9-1.9 2.2 2.2c.3.3.78.3 1.08 0L12 20l1.47 1.45c.3.3.78.3 1.08 0l2.2-2.2 1.9 1.9c.5.5 1.35.15 1.35-.55V10a8 8 0 0 0-8-8z'/%3E%3C/svg%3E">
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Outfit:wght@400;500;600;700&family=Fraunces:opsz,wght@9..144,500..700&family=Roboto+Mono:wght@400;500&display=swap">
<style>{css}</style></head><body>{shell}<script>{js}</script><script src="/ghost/assets/1.8.14/publishing-workflows.js" defer></script></body></html>"#,
        title = esc(title),
        site_name = esc(&admin_site_name()),
        css = CSS,
        shell = shell,
        js = JS,
    ))
}

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
    fn from_name(name: &str) -> Role {
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
    fn is_admin(self) -> bool {
        self >= Role::Administrator
    }

    /// Tags are `tag = all` for Editor and above; Authors may only add tags
    /// while writing, Contributors not at all.
    fn can_manage_tags(self) -> bool {
        self >= Role::Editor
    }

    /// Editors may edit, publish and delete anyone's posts; Authors and
    /// Contributors only their own.
    fn can_edit_any_post(self) -> bool {
        self >= Role::Editor
    }

    /// Contributors write drafts and nothing else — publishing is the line
    /// Ghost draws for them.
    fn can_publish(self) -> bool {
        self >= Role::Author
    }

    /// Editors manage Authors and Contributors; admins manage everyone.
    fn can_manage_staff(self) -> bool {
        self >= Role::Editor
    }

    fn label(self) -> &'static str {
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
    pub user: ghost_core::models::User,
    pub role: Role,
}

/// Resolve the signed-in user and their role, or redirect to signin.
async fn require_actor(state: &AppState, headers: &HeaderMap) -> Result<Actor, Response> {
    let user = current_user(state, headers)
        .await
        .ok_or_else(|| Redirect::to("/ghost/signin/").into_response())?;
    let name = repos::users::role_name(&state.db, &user.id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Actor { role: Role::from_name(&name), user })
}

/// Require at least `needed`. The refusal is a page rather than a bare 403 so
/// a mistyped URL explains itself instead of looking broken.
async fn require_role(state: &AppState, headers: &HeaderMap, needed: Role) -> Result<Actor, Response> {
    let actor = require_actor(state, headers).await?;
    if actor.role >= needed {
        return Ok(actor);
    }
    Err(forbidden_page(&actor, needed))
}

fn forbidden_page(actor: &Actor, needed: Role) -> Response {
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
async fn may_edit_post(state: &AppState, actor: &Actor, post_id: &str) -> bool {
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

async fn require_session(state: &AppState, headers: &HeaderMap) -> Result<ghost_core::models::User, Redirect> {
    match current_user(state, headers).await {
        Some(u) => Ok(u),
        None => Err(Redirect::to("/ghost/signin/")),
    }
}

async fn current_user(state: &AppState, headers: &HeaderMap) -> Option<ghost_core::models::User> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let sid = cookie
        .split(';')
        .filter_map(|c| c.trim().strip_prefix("ghost-admin-api-session="))
        .next()?
        .to_string();
    let user_id = repos::sessions::get_user(&state.db, &sid).await.ok()??;
    repos::users::find_by_id(&state.db, &user_id).await.ok()?
}

// ---------------------------------------------------------------------------
// index / setup / login
// ---------------------------------------------------------------------------

/// The admin landing handler, routed inside this crate at the nest root and
/// re-exported so the app can also bind it at `/ghost/`.
///
/// `Router::nest("/ghost", …)` maps an inner `/` route to `/ghost` only, not to
/// `/ghost/`, so without that second binding the canonical admin URL falls
/// through to the site's catch-all — which bounces every `/ghost`-prefixed
/// path straight back to `/ghost/` and loops.
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/setup/").into_response();
    }
    match require_session(&state, &headers).await {
        Ok(_) => Redirect::to("/ghost/posts/").into_response(),
        Err(r) => r.into_response(),
    }
}

async fn setup_page(State(state): State<AppState>) -> Response {
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    page_u(
        "Setup",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Create your site</h1>
<p class="sub">Set up the owner account for {title}</p>
<div class="card"><form method="post" action="/ghost/setup/">
<label class="field"><input name="blog_title" value="{title_v}" placeholder=" " required><span>Site title</span></label>
<label class="field"><input name="name" placeholder=" " required autocomplete="name"><span>Your name</span></label>
<label class="field"><input name="email" type="email" placeholder=" " required autocomplete="email"><span>Email</span></label>
<label class="field"><input name="password" type="password" minlength="10" required autocomplete="new-password"><span>Password · min 10 characters</span></label>
<button class="btn" type="submit">Create account</button>
</form></div>
</div></div>"#,
            logo = svg("ghost"),
            title = esc(&title),
            title_v = esc(&title),
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
struct SetupForm {
    name: String,
    email: String,
    password: String,
    blog_title: String,
}

async fn setup_submit(State(state): State<AppState>, Form(form): Form<SetupForm>) -> Response {
    if repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return page_u("Setup", "", None, "<div class='banner error'>Password hashing failed</div>", false).into_response(),
    };
    if form.password.len() < 10 {
        return page_u(
            "Setup",
            "",
            None,
            "<div class='banner error'>Password must be at least 10 characters.</div>",
            false,
        )
        .into_response();
    }
    match repos::users::setup_owner(&state.db, &form.name, &form.email, &hash).await {
        Ok(_) => {
            let seeded_title = state.settings.get_str("title").unwrap_or_default();
            let _ = state.settings.set(&state.db, "title", &serde_json::json!(form.blog_title)).await;
            // Only here: the seeded newsletter takes the site's real name at
            // setup and keeps whatever it is called after that.
            let _ = repos::newsletters::name_after_site(&state.db, &seeded_title, &form.blog_title).await;
            let _ = sqlx::query("UPDATE posts SET title = ? WHERE slug = 'coming-soon'").bind(&form.blog_title).execute(&state.db).await;
            state.refresh_setup_flag().await;
            Redirect::to("/ghost/signin/").into_response()
        }
        Err(e) => page_u("Setup", "", None, &format!("<div class='banner error'>{}</div>", esc(&e.message())), false).into_response(),
    }
}

async fn login_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if !repos::users::is_setup(&state.db).await.unwrap_or(false) {
        return Redirect::to("/ghost/setup/").into_response();
    }
    if current_user(&state, &headers).await.is_some() {
        return Redirect::to("/ghost/posts/").into_response();
    }
    let err = if q.contains_key("invalid") {
        r#"<div class="banner error"><svg class="i" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="10"/></svg>Invalid email or password</div>"#
    } else {
        ""
    };
    page_u(
        "Sign in",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>{title}</h1>
<p class="sub">Sign in to the admin panel</p>
<div class="card"><form method="post" action="/ghost/signin/">{err}
<label class="field"><input name="email" type="email" placeholder=" " required autofocus autocomplete="username"><span>Email</span></label>
<label class="field"><input name="password" type="password" placeholder=" " required autocomplete="current-password"><span>Password</span></label>
<button class="btn" type="submit">Sign in</button>
<a class="btn text" href="/ghost/forgot-password/">Forgot password?</a>
</form></div>
</div></div>"#,
            logo = svg("ghost"),
            title = esc(&state.settings.get_str("title").unwrap_or_else(|| "Ghost".into())),
            err = err
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
struct LoginForm {
    email: String,
    password: String,
}

/// Attach a freshly minted session cookie to a response.
fn set_session_cookie(resp: &mut Response, sid: &str) {
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "ghost-admin-api-session={sid}; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age={}",
            ghost_core::url::subdir(),
            60 * 60 * 24 * 7
        )
        .parse()
        .unwrap(),
    );
}

async fn login_submit(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let Ok(Some(user)) = repos::users::verify_password(&state.db, &form.email, &form.password).await
    else {
        return Redirect::to("/ghost/signin/?invalid=1").into_response();
    };

    // With 2FA off the password is the whole login, as before. A first-ever
    // sign-in also goes straight through: the person only has a password
    // because they followed an emailed invite or setup link, which is the
    // same proof a code would ask for. Ghost's session endpoint takes this
    // shortcut too (`skipVerification` when `!user.hasLoggedIn()`).
    if !state.settings.get_bool("require_email_mfa") || user.last_seen.is_none() {
        return match repos::sessions::create(&state.db, &user.id).await {
            Ok(sid) => {
                let mut resp = Redirect::to("/ghost/posts/").into_response();
                set_session_cookie(&mut resp, &sid);
                resp
            }
            Err(_) => Redirect::to("/ghost/signin/?invalid=1").into_response(),
        };
    }

    // With it on, the password only names the user: the session is created
    // unverified, so none of the admin guards will resolve it until the code
    // that just went out by email comes back.
    let Ok(sid) = repos::sessions::create_unverified(&state.db, &user.id).await else {
        return Redirect::to("/ghost/signin/?invalid=1").into_response();
    };
    let sent = issue_signin_code(&state, &sid, &user).await;
    let mut resp = Redirect::to(if sent { "/ghost/signin/verify/" } else { "/ghost/signin/verify/?send=failed" })
        .into_response();
    set_session_cookie(&mut resp, &sid);
    resp
}

/// Generate a sign-in code, store its digest on the session and email it.
/// Returns whether the mail actually went out.
async fn issue_signin_code(state: &AppState, sid: &str, user: &ghost_core::models::User) -> bool {
    let Some(email) = user.email.clone() else {
        tracing::error!("user {} has no email address to send a sign-in code to", user.id);
        return false;
    };
    // Six digits, zero-padded, off the same CSPRNG that mints session ids —
    // the shape of code Ghost emails.
    let code = ghost_core::ids::random_code(6);
    if repos::sessions::set_auth_code(&state.db, sid, &code).await.is_err() {
        return false;
    }

    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let msg = ghost_mail::Message {
        to: email.clone(),
        subject: format!("{code} is your sign-in verification code"),
        html: format!(
            r#"<div style="font-family:-apple-system,Segoe UI,sans-serif;font-size:16px;line-height:1.6;color:#15171a">
<p>Hey{name},</p>
<p>Here is the verification code you need to sign in to <strong>{title}</strong>:</p>
<p style="font-size:30px;font-weight:700;letter-spacing:6px;margin:28px 0">{code}</p>
<p style="color:#626d79;font-size:14px">The code is good for five minutes. If you didn't try to sign in, you can ignore this email — someone knows your password, so it is worth changing it.</p>
</div>"#,
            name = user.name.as_deref().filter(|n| !n.is_empty()).map(|n| format!(" {}", esc(n))).unwrap_or_default(),
            title = esc(&title),
            code = code,
        ),
        text: format!(
            "Your verification code for {title} is {code}.\n\nIt is good for five minutes. \
             If you didn't try to sign in, someone knows your password and it is worth changing."
        ),
    };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("could not send the sign-in code to {}: {}", email, e.message());
            false
        }
    }
}

/// The session id on the request, verified or not.
fn session_id_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())?
        .split(';')
        .find_map(|c| c.trim().strip_prefix("ghost-admin-api-session="))
        .map(str::to_string)
}

/// The code-entry screen, shown between password and session when email 2FA
/// is on. Reached with an unverified session cookie already set.
async fn verify_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    // A verified session has nothing to do here; an unknown one has to start over.
    match repos::sessions::is_unverified(&state.db, &sid).await {
        Ok(true) => {}
        Ok(false) => return Redirect::to("/ghost/posts/").into_response(),
        Err(_) => return Redirect::to("/ghost/signin/").into_response(),
    }

    let banner = match q.get("send").map(String::as_str) {
        Some("failed") => r#"<div class="banner error">Could not send the code — check the mail settings, or ask another administrator.</div>"#,
        Some("resent") => r#"<div class="banner ok">A new code is on its way.</div>"#,
        _ => match q.get("error").map(String::as_str) {
            Some("incorrect") => r#"<div class="banner error">That code is not right. Check the email and try again.</div>"#,
            Some("expired") => r#"<div class="banner error">That code has expired. Send yourself a new one.</div>"#,
            _ => "",
        },
    };

    page_u(
        "Verify sign-in",
        "",
        None,
        &format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Check your email</h1>
<p class="sub">We sent a six-digit code to your inbox. Enter it to finish signing in.</p>
<div class="card"><form method="post" action="/ghost/signin/verify/">{banner}
<label class="field"><input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]*" maxlength="6" placeholder=" " required autofocus><span>Verification code</span></label>
<button class="btn" type="submit">Verify</button>
</form>
<form method="post" action="/ghost/signin/resend/" style="margin-top:10px">
<button class="btn text" type="submit">Send a new code</button>
</form>
<form method="post" action="/ghost/signout/" style="margin-top:2px">
<button class="btn text" type="submit">Sign in as someone else</button>
</form></div>
</div></div>"#,
            logo = svg("ghost"),
            banner = banner,
        ),
        false,
    )
    .into_response()
}

#[derive(Deserialize)]
struct VerifyForm {
    code: String,
}

async fn verify_submit(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<VerifyForm>) -> Response {
    use ghost_db::repos::sessions::CodeCheck;
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    match repos::sessions::verify_auth_code(&state.db, &sid, &form.code).await {
        Ok(CodeCheck::Verified) => {
            if let Ok(Some(user_id)) = repos::sessions::get_user(&state.db, &sid).await {
                let _ = repos::users::update_last_seen(&state.db, &user_id).await;
            }
            Redirect::to("/ghost/posts/").into_response()
        }
        Ok(CodeCheck::Incorrect) => Redirect::to("/ghost/signin/verify/?error=incorrect").into_response(),
        _ => Redirect::to("/ghost/signin/verify/?error=expired").into_response(),
    }
}

/// Issue a replacement code for a session still waiting on one.
async fn verify_resend(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(sid) = session_id_from(&headers) else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    if !repos::sessions::is_unverified(&state.db, &sid).await.unwrap_or(false) {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let Ok(Some(user_id)) = repos::sessions::get_pending_user(&state.db, &sid).await else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let Ok(Some(user)) = repos::users::find_by_id(&state.db, &user_id).await else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    let sent = issue_signin_code(&state, &sid, &user).await;
    Redirect::to(if sent { "/ghost/signin/verify/?send=resent" } else { "/ghost/signin/verify/?send=failed" })
        .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(sid) = cookie.split(';').find_map(|c| c.trim().strip_prefix("ghost-admin-api-session=")) {
            let _ = repos::sessions::destroy(&state.db, sid).await;
        }
    }
    let mut resp = Redirect::to("/ghost/signin/").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("ghost-admin-api-session=; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age=0", ghost_core::url::subdir()).parse().unwrap(),
    );
    resp
}

// ---------------------------------------------------------------------------
// Modules: a single entry point for the publication's administrative tools.
const MODULES: &[(&str, &str, &str, &str, &str)] = &[
    ("Analytics", "/ghost/analytics/", "chart", "green", "Track visitors, content performance, and member growth."),
    ("Comments", "/ghost/comments/", "comment", "purple", "Review discussions and manage comments across your posts."),
    ("Automations", "/ghost/automations/", "flow", "blue", "Manage automated workflows and their activity."),
    ("Social web", "/ghost/social/", "share", "teal", "Read, publish notes and connect with the fediverse."),
    ("Machine payments", "/ghost/machine-payments/", "integrations", "pink", "Offer per-article paid Markdown access to AI agents."),
];

async fn modules_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let cards: String = MODULES.iter().map(|(name, href, icon, tint, description)| {
        format!(
            r#"<a class="module-card" href="{href}">
<span class="ic tint-{tint}">{icon}</span>
<div><h2>{name}</h2><p>{description}</p></div>
<span class="module-open">Open {name} {arrow}</span>
</a>"#,
            href = href, tint = tint, icon = svg(icon), name = esc(name),
            description = esc(description), arrow = svg("chevron"),
        )
    }).collect();
    page_for_role(actor.role, "Modules", "modules", Some(&actor.user), &format!(
        r#"<header class="page-head rise"><div><h1>Modules</h1>
<p class="sub">Insights, conversations, and workflows for your publication.</p></div></header>
<div class="module-grid rise" style="--i:1">{cards}</div>"#,
    ), true).into_response()
}

// analytics (Ghost's Dashboard: page views recorded by ghost-front's
// web_analytics middleware, members, and content counts)
// ---------------------------------------------------------------------------

/// One day of the views chart.
struct DayViews {
    day: String,
    views: i64,
}

/// Compact number for tile values and ticks (1,284 → 1.3K).
fn compact(n: i64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Round up to a clean axis maximum from a ladder of chart-friendly tops
/// (1 → 4, 6 → 8, 70 → 80, 1_234 → 2_000) so the half/whole ticks stay even.
fn clean_max(max: i64) -> i64 {
    let mut candidates = vec![4, 8, 10];
    let mut base = 10;
    while base <= 1_000_000_000 {
        for m in [2, 4, 6, 8, 10] {
            candidates.push(m * base);
        }
        base *= 10;
    }
    candidates.into_iter().find(|c| *c >= max).unwrap_or(max.max(1))
}

/// SVG column chart of daily views — thin columns with 4px rounded caps
/// (square at the baseline), hairline grid at clean ticks, the max value
/// labeled, and a native tooltip on every day. Single series, so the card
/// title carries the legend and no legend box is drawn.
fn views_chart(days: &[DayViews]) -> String {
    if days.is_empty() {
        return r#"<p class="muted" style="margin:8px 0 0">No views recorded yet.</p>"#.into();
    }
    let (w, h, pad_l, pad_b, pad_t) = (728usize, 180usize, 34usize, 22usize, 18usize);
    let plot_w = w - pad_l - 8;
    let plot_h = h - pad_b - pad_t;
    let slot = plot_w as f64 / days.len() as f64;
    let bar_w = slot.min(24.0) * 0.75; // ≤24px thick, air in the band
    let axis = clean_max(days.iter().map(|d| d.views).max().unwrap_or(1));
    let y = |v: i64| pad_t as f64 + plot_h as f64 * (1.0 - v as f64 / axis as f64);
    let max_i = days
        .iter()
        .enumerate()
        .max_by_key(|(_, d)| d.views)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut grid = String::new();
    for frac in [0.0, 0.5, 1.0] {
        let value = (axis as f64 * frac).round() as i64;
        let gy = y(value);
        grid.push_str(&format!(
            r#"<line x1="{pad_l}" x2="{}" y1="{gy:.1}" y2="{gy:.1}" stroke="var(--outline-variant)" stroke-width="1"/>"#,
            w - 8,
            gy = gy,
        ));
        grid.push_str(&format!(
            r#"<text x="{}" y="{:.1}" text-anchor="end" dominant-baseline="middle">{}</text>"#,
            pad_l - 8,
            gy + 3.0,
            compact(value),
        ));
    }

    let mut bars = String::new();
    let mut labels = String::new();
    for (i, d) in days.iter().enumerate() {
        let cx = pad_l as f64 + slot * (i as f64 + 0.5);
        // x labels: first, last, and the midpoint; every day on a 7-day range
        let show_label = days.len() <= 7 || i == 0 || i == days.len() - 1 || i == days.len() / 2;
        if show_label {
            labels.push_str(&format!(
                r#"<text x="{cx:.0}" y="{}" text-anchor="middle">{}</text>"#,
                h - 6,
                esc(&d.day[5..]), // MM-DD
                cx = cx,
            ));
        }
        if d.views == 0 {
            continue;
        }
        let (x0, y_top) = (cx - bar_w / 2.0, y(d.views));
        let height = (pad_t + plot_h) as f64 - y_top;
        let r = 4.0_f64.min(height); // a cap never taller than the bar
        let (x1, cap_l, cap_r) = (x0 + bar_w, x0 + r, x0 + bar_w - r);
        let (yt, top, base) = (y_top + r, y_top, (pad_t + plot_h) as f64);
        let path = format!(
            "M{x0:.1},{base} L{x0:.1},{yt:.1} Q{x0:.1},{top:.1} {cap_l:.1},{top:.1} L{cap_r:.1},{top:.1} Q{x1:.1},{top:.1} {x1:.1},{yt:.1} L{x1:.1},{base} Z",
        );
        let value_label = if i == max_i {
            format!(
                r#"<text x="{cx:.0}" y="{:.1}" text-anchor="middle" font-weight="600">{}</text>"#,
                y_top - 6.0,
                compact(d.views),
            )
        } else {
            String::new()
        };
        bars.push_str(&format!(
            r#"<g><title>{}: {} views</title><path class="bar" d="{path}" fill="var(--primary)"/>{value_label}</g>"#,
            esc(&d.day),
            d.views,
            path = path,
            value_label = value_label,
        ));
    }

    format!(
        r#"<svg class="chart" viewBox="0 0 {w} {h}" role="img" aria-label="Views per day">{grid}{bars}{labels}</svg>"#,
        w = w,
        h = h,
        grid = grid,
        bars = bars,
        labels = labels,
    )
}

// ---------------------------------------------------------------------------
// analytics — Ghost's native analytics, split into the same tabs
// ---------------------------------------------------------------------------

/// The reporting windows Ghost offers, as (query value, label, days back).
/// `None` days means all time.
const ANALYTICS_RANGES: &[(&str, &str, Option<i64>)] = &[
    ("today", "Today", Some(0)),
    ("7", "7 days", Some(7)),
    ("30", "30 days", Some(30)),
    ("90", "3 months", Some(90)),
    ("365", "12 months", Some(365)),
    ("all", "All time", None),
];

/// The tabs the Analytics page is split into, mirroring Ghost's.
const ANALYTICS_TABS: &[(&str, &str)] = &[
    ("overview", "Overview"),
    ("web", "Web traffic"),
    ("growth", "Growth"),
    ("newsletters", "Newsletters"),
];

/// Ghost's posts/pages split on the Top content table.
const ANALYTICS_CONTENT_KINDS: &[(&str, &str)] =
    &[("all", "Posts & pages"), ("posts", "Posts"), ("pages", "Pages")];

/// Audience segments for the web traffic breakdown. The values match the
/// `audience` column written by the page-view middleware.
const ANALYTICS_AUDIENCES: &[(&str, &str)] = &[
    ("all", "All traffic"),
    ("public", "Public visitors"),
    ("free", "Free members"),
    ("paid", "Paid members"),
];

/// A reporting window resolved into something the queries can use.
struct AnalyticsRange {
    slug: String,
    days: Option<i64>,
    /// SQLite modifier for `datetime('now', ?)`, or `None` for all time.
    cutoff: String,
    label: String,
}

impl AnalyticsRange {
    fn from_query(q: &HashMap<String, String>) -> Self {
        let slug = q.get("range").map(String::as_str).unwrap_or("30");
        let (slug, label, days) = ANALYTICS_RANGES
            .iter()
            .find(|(s, _, _)| *s == slug)
            .copied()
            .unwrap_or(("30", "30 days", Some(30)));
        // A cutoff far enough back to include everything stands in for "all
        // time", so every query keeps the same shape and the same index.
        let cutoff = match days {
            Some(0) => "start of day".to_string(),
            Some(n) => format!("-{n} days"),
            None => "-100 years".to_string(),
        };
        Self { slug: slug.to_string(), days, cutoff, label: label.to_string() }
    }

    /// How many daily buckets the chart should show.
    async fn chart_days(&self, db: &sqlx::SqlitePool) -> i64 {
        match self.days {
            Some(0) => 1,
            Some(n) => n,
            // All time: back to the first view there is, capped so the chart
            // stays readable.
            None => {
                let first: Option<String> =
                    sqlx::query_scalar("SELECT MIN(substr(created_at, 1, 10)) FROM page_views")
                        .fetch_one(db)
                        .await
                        .unwrap_or(None);
                match first {
                    Some(day) => sqlx::query_scalar::<_, i64>(
                        "SELECT MIN(365, MAX(1, CAST(julianday('now') - julianday(?) AS INTEGER) + 1))",
                    )
                    .bind(day)
                    .fetch_one(db)
                    .await
                    .unwrap_or(30),
                    None => 30,
                }
            }
        }
    }
}

/// Daily unique-visitor buckets across the window, with gaps filled in as
/// zero.
///
/// Counted as distinct viewers per day rather than raw hits, so the chart and
/// the "unique visitors" figure above it are measuring the same thing — a
/// reader who opens five posts is one visitor on both.
async fn analytics_daily_views(
    db: &sqlx::SqlitePool,
    range: &AnalyticsRange,
    audience: &str,
) -> Vec<DayViews> {
    let span = range.chart_days(db).await;
    let mut day_map: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let sql = format!(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(DISTINCT viewer) FROM page_views \
         WHERE created_at >= datetime('now', ?){} GROUP BY day",
        audience_clause(audience)
    );
    if let Ok(rows) = sqlx::query_as::<_, (String, i64)>(&sql)
        .bind(&range.cutoff)
        .fetch_all(db)
        .await
    {
        for (day, views) in rows {
            day_map.insert(day, views);
        }
    }
    let mut days = Vec::new();
    for back in (0..span).rev() {
        let day: String = sqlx::query_scalar("SELECT date('now', ?)")
            .bind(format!("-{back} days"))
            .fetch_one(db)
            .await
            .unwrap_or_default();
        days.push(DayViews { views: day_map.get(&day).copied().unwrap_or(0), day });
    }
    days
}

/// `AND audience = '…'` for a chosen segment, or nothing for all traffic.
///
/// Inlined rather than bound because it is chosen from a fixed list, never
/// from the query string directly.
fn audience_clause(audience: &str) -> &'static str {
    match audience {
        "public" => " AND audience = 'public'",
        "free" => " AND audience = 'free'",
        "paid" => " AND audience = 'paid'",
        _ => "",
    }
}

/// Unique visitors and total views over the window.
async fn analytics_traffic(
    db: &sqlx::SqlitePool,
    range: &AnalyticsRange,
    audience: &str,
) -> (i64, i64) {
    let sql = format!(
        "SELECT COUNT(DISTINCT viewer), COUNT(*) FROM page_views \
         WHERE created_at >= datetime('now', ?){}",
        audience_clause(audience)
    );
    sqlx::query_as(&sql).bind(&range.cutoff).fetch_one(db).await.unwrap_or((0, 0))
}

/// Most-viewed posts and pages over the window.
///
/// `kind` filters to `post`, `page`, or everything. A view counts for a piece
/// when the path is its own, which is how the front end serves it.
async fn analytics_top_content(
    db: &sqlx::SqlitePool,
    range: &AnalyticsRange,
    audience: &str,
    kind: &str,
    limit: i64,
) -> Vec<(String, String, String, i64)> {
    let type_clause = match kind {
        "posts" => " AND p.type = 'post'",
        "pages" => " AND p.type = 'page'",
        _ => "",
    };
    let sql = format!(
        "SELECT COALESCE(p.title, p.slug), p.slug, p.type, COUNT(*) AS views FROM page_views v \
         JOIN posts p ON v.path = '/' || p.slug || '/' OR v.path LIKE '/' || p.slug || '/%' \
         WHERE v.created_at >= datetime('now', ?) AND p.status = 'published'{}{} \
         GROUP BY p.id ORDER BY views DESC LIMIT ?",
        type_clause,
        audience_clause(audience)
    );
    sqlx::query_as(&sql).bind(&range.cutoff).bind(limit).fetch_all(db).await.unwrap_or_default()
}

/// Where the window's traffic came from. A view with no referrer is Direct,
/// which is how Ghost labels it.
async fn analytics_top_sources(
    db: &sqlx::SqlitePool,
    range: &AnalyticsRange,
    audience: &str,
    limit: i64,
) -> Vec<(String, i64)> {
    let sql = format!(
        "SELECT COALESCE(NULLIF(referrer, ''), 'Direct') AS source, COUNT(DISTINCT viewer) AS visitors \
         FROM page_views WHERE created_at >= datetime('now', ?){} \
         GROUP BY source ORDER BY visitors DESC LIMIT ?",
        audience_clause(audience)
    );
    sqlx::query_as(&sql).bind(&range.cutoff).bind(limit).fetch_all(db).await.unwrap_or_default()
}

/// Members, split the way the Growth tab reports them, plus monthly recurring
/// revenue in minor units.
async fn analytics_members(db: &sqlx::SqlitePool) -> (i64, i64, i64, i64) {
    let (total, paid): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status = 'paid' THEN 1 ELSE 0 END), 0) FROM members",
    )
    .fetch_one(db)
    .await
    .unwrap_or((0, 0));
    // MRR comes off the live subscriptions, the same forward-looking figure
    // Ghost reports: cancelled ones stop counting the moment they lapse.
    let mrr: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(mrr), 0) FROM subscriptions WHERE status IN ('active', 'trialing')",
    )
    .fetch_one(db)
    .await
    .unwrap_or(0.0);
    (total, total - paid, paid, mrr.round() as i64)
}

/// Minor units (what Stripe and the `subscriptions.mrr` column hold) rendered
/// as an amount, grouped and without trailing zeroes on a whole figure.
fn money_minor(minor: i64) -> String {
    let major = minor as f64 / 100.0;
    let whole = major.trunc() as i64;
    let mut grouped = String::new();
    let digits = whole.abs().to_string();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let cents = minor.abs() % 100;
    let sign = if minor < 0 { "-" } else { "" };
    if cents == 0 {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{cents:02}")
    }
}

/// `3 visitors` / `1 visitor` — a count with its noun agreeing.
fn countable(n: i64, singular: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{} {singular}s", compact(n))
    }
}

/// A simple rows card, or a muted note when there is nothing to show.
fn analytics_rows(rows: String, empty: &str) -> String {
    if rows.is_empty() {
        format!(
            r#"<div class="row"><div class="body"><span class="muted">{}</span></div></div>"#,
            esc(empty)
        )
    } else {
        rows
    }
}

/// One `title · detail · figure` row.
fn analytics_row(icon: &str, tint: &str, title: &str, detail: &str, figure: &str, href: Option<&str>) -> String {
    let body = match href {
        Some(h) => format!(
            r#"<a class="body" href="{h}" target="_blank" rel="noopener"><span class="title">{title}</span><span class="detail">{detail}</span></a>"#,
            h = esc(h),
            title = esc(title),
            detail = detail,
        ),
        None => format!(
            r#"<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>"#,
            title = esc(title),
            detail = detail,
        ),
    };
    format!(
        r#"<div class="row"><span class="ic tint-{tint}">{icon}</span>{body}
<div class="trailing"><span class="chip tonal">{figure}</span></div></div>"#,
        tint = tint,
        icon = icon,
        body = body,
        figure = esc(figure),
    )
}

async fn analytics_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let range = AnalyticsRange::from_query(&q);
    let tab = q.get("tab").map(String::as_str).unwrap_or("overview");
    let tab = ANALYTICS_TABS.iter().find(|(s, _)| *s == tab).map(|(s, _)| *s).unwrap_or("overview");
    let audience = q.get("audience").map(String::as_str).unwrap_or("all");
    let audience = ANALYTICS_AUDIENCES.iter().find(|(s, _)| *s == audience).map(|(s, _)| *s).unwrap_or("all");
    let content = q.get("content").map(String::as_str).unwrap_or("all");
    let content = ANALYTICS_CONTENT_KINDS.iter().find(|(s, _)| *s == content).map(|(s, _)| *s).unwrap_or("all");

    // Collection is a setting, and a page of zeroes is confusing without
    // saying why — Ghost surfaces the same thing on its traffic tab.
    let tracking_off = !state.settings.get_bool("web_analytics");
    let notice = if tracking_off && matches!(tab, "overview" | "web") {
        format!(
            r#"<div class="banner info rise" style="--i:1">{ic}<span class="msg">Web analytics is switched off, so no new visits are being recorded. Turn it on in <a href="/ghost/settings/analytics/">Settings &rsaquo; Analytics</a>.</span></div>"#,
            ic = svg("chart"),
        )
    } else {
        String::new()
    };

    let link = |tab: &str, range_slug: &str, audience: &str| {
        let mut url = format!("/ghost/analytics/?tab={tab}&range={range_slug}");
        if audience != "all" {
            url.push_str(&format!("&audience={audience}"));
        }
        url
    };

    let tabs: String = ANALYTICS_TABS
        .iter()
        .map(|(slug, label)| {
            format!(
                r#"<a class="seg{on}" href="{href}">{label}</a>"#,
                on = if *slug == tab { " on" } else { "" },
                href = esc(&link(slug, &range.slug, audience)),
                label = esc(label),
            )
        })
        .collect();

    let ranges: String = ANALYTICS_RANGES
        .iter()
        .map(|(slug, label, _)| {
            format!(
                r#"<a class="{on}" href="{href}">{label}</a>"#,
                on = if *slug == range.slug { "active" } else { "" },
                href = esc(&link(tab, slug, audience)),
                label = esc(label),
            )
        })
        .collect();

    let body = match tab {
        "web" => {
            let segs: String = ANALYTICS_AUDIENCES
                .iter()
                .map(|(slug, label)| {
                    format!(
                        r#"<a class="seg{on}" href="{href}">{label}</a>"#,
                        on = if *slug == audience { " on" } else { "" },
                        href = esc(&format!("{}&content={content}", link("web", &range.slug, slug))),
                        label = esc(label),
                    )
                })
                .collect();
            let kind_segs: String = ANALYTICS_CONTENT_KINDS
                .iter()
                .map(|(slug, label)| {
                    format!(
                        r#"<a class="seg{on}" href="{href}">{label}</a>"#,
                        on = if *slug == content { " on" } else { "" },
                        href = esc(&format!("{}&content={slug}", link("web", &range.slug, audience))),
                        label = esc(label),
                    )
                })
                .collect();
            analytics_web_tab(&state, &range, audience, content, segs, kind_segs).await
        }
        "growth" => analytics_growth_tab(&state, &range).await,
        "newsletters" => analytics_newsletters_tab(&state, &range).await,
        _ => analytics_overview_tab(&state, &range).await,
    };

    page_for_role(actor_role,
        "Analytics",
        "analytics",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Analytics</h1><p class="sub">{sub}</p></div>
<div class="actions"><div class="range-tabs">{ranges}</div></div></header>
<div class="seg-bar">{tabs}</div>
{notice}{body}"#,
            sub = esc(&range.label),
            ranges = ranges,
            tabs = tabs,
            notice = notice,
            body = body,
        ),
        true,
    )
    .into_response()
}

/// Overview: the headline figures, the traffic shape, and what did best.
async fn analytics_overview_tab(state: &AppState, range: &AnalyticsRange) -> String {
    let (visitors, views) = analytics_traffic(&state.db, range, "all").await;
    let (members, _free, paid, mrr) = analytics_members(&state.db).await;
    let days = analytics_daily_views(&state.db, range, "all").await;

    // "Right now" is the live counter Ghost puts at the top of the overview.
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT viewer) FROM page_views WHERE created_at >= datetime('now', '-5 minutes')",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let top: String = analytics_top_content(&state.db, range, "all", "all", 5)
        .await
        .into_iter()
        .map(|(title, slug, kind, views)| {
            analytics_row(
                &svg(if kind == "page" { "pages" } else { "posts" }),
                "green",
                &title,
                &format!("<code>/{}/</code>", esc(&slug)),
                &countable(views, "view"),
                Some(&format!("/{slug}/")),
            )
        })
        .collect();

    let sources: String = analytics_top_sources(&state.db, range, "all", 5)
        .await
        .into_iter()
        .map(|(source, visitors)| {
            analytics_row(
                &svg(if source == "Direct" { "link" } else { "globe" }),
                "blue",
                &source,
                if source == "Direct" { "Typed, bookmarked or untracked" } else { "Referral" },
                &countable(visitors, "visitor"),
                None,
            )
        })
        .collect();

    format!(
        r#"<div class="tiles rise" style="--i:1">
<div class="tile"><div class="label">Unique visitors</div><div class="value">{visitors}</div><div class="delta">{live} in the last 5 minutes</div></div>
<div class="tile"><div class="label">Total views</div><div class="value">{views}</div><div class="delta">page views, repeats included</div></div>
<div class="tile"><div class="label">Members</div><div class="value">{members}</div><div class="delta">{paid} paid · <a href="/ghost/members/">manage</a></div></div>
<div class="tile"><div class="label">MRR</div><div class="value">{mrr}</div><div class="delta">from active subscriptions</div></div>
</div>
<div class="section">Unique visitors per day</div>
<div class="card padded rise" style="--i:2">{chart}</div>
<div class="section">Top content</div>
<div class="card rows rise" style="--i:3">{top}</div>
<div class="section">Top sources</div>
<div class="card rows rise" style="--i:4">{sources}</div>"#,
        visitors = compact(visitors),
        live = live,
        views = compact(views),
        members = compact(members),
        paid = paid,
        mrr = money_minor(mrr),
        chart = views_chart(&days),
        top = analytics_rows(top, "No content views in this range yet."),
        sources = analytics_rows(sources, "No traffic recorded in this range yet."),
    )
}

/// Web traffic: the same figures broken down by audience, with the content
/// table filterable by posts or pages the way Ghost's is.
async fn analytics_web_tab(
    state: &AppState,
    range: &AnalyticsRange,
    audience: &str,
    kind: &str,
    segs: String,
    kind_segs: String,
) -> String {
    let (visitors, views) = analytics_traffic(&state.db, range, audience).await;
    let days = analytics_daily_views(&state.db, range, audience).await;

    let top: String = analytics_top_content(&state.db, range, audience, kind, 10)
        .await
        .into_iter()
        .map(|(title, slug, kind, views)| {
            analytics_row(
                &svg(if kind == "page" { "pages" } else { "posts" }),
                "green",
                &title,
                &format!("<code>/{}/</code> · {}", esc(&slug), if kind == "page" { "Page" } else { "Post" }),
                &countable(views, "view"),
                Some(&format!("/{slug}/")),
            )
        })
        .collect();

    let sources: String = analytics_top_sources(&state.db, range, audience, 10)
        .await
        .into_iter()
        .map(|(source, visitors)| {
            analytics_row(
                &svg(if source == "Direct" { "link" } else { "globe" }),
                "blue",
                &source,
                if source == "Direct" { "Typed, bookmarked or untracked" } else { "Referral" },
                &countable(visitors, "visitor"),
                None,
            )
        })
        .collect();

    format!(
        r#"<div class="tiles rise" style="--i:1" data-cols="2">
<div class="tile"><div class="label">Unique visitors</div><div class="value">{visitors}</div><div class="delta">distinct people in this range</div></div>
<div class="tile"><div class="label">Total views</div><div class="value">{views}</div><div class="delta">page views, repeats included</div></div>
</div>
<div class="section">Audience</div>
<div class="seg-bar">{segs}</div>
<div class="card padded rise" style="--i:2">{chart}</div>
<div class="section">Top content</div>
<div class="seg-bar">{kind_segs}</div>
<div class="card rows rise" style="--i:3">{top}</div>
<div class="section">Top sources</div>
<div class="card rows rise" style="--i:4">{sources}</div>"#,
        visitors = compact(visitors),
        views = compact(views),
        segs = segs,
        kind_segs = kind_segs,
        chart = views_chart(&days),
        top = analytics_rows(top, "No content views in this range yet."),
        sources = analytics_rows(sources, "No traffic recorded in this range yet."),
    )
}

/// Growth: members over time, the free/paid split, MRR, and where signups
/// came from.
async fn analytics_growth_tab(state: &AppState, range: &AnalyticsRange) -> String {
    let (total, free, paid, mrr) = analytics_members(&state.db).await;

    // New members per day across the window, which is the shape the chart
    // shows; the tiles carry the running totals.
    let span = range.chart_days(&state.db).await;
    let mut day_map: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    if let Ok(rows) = sqlx::query_as::<_, (String, i64)>(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(*) FROM members \
         WHERE created_at >= datetime('now', ?) GROUP BY day",
    )
    .bind(&range.cutoff)
    .fetch_all(&state.db)
    .await
    {
        for (day, n) in rows {
            day_map.insert(day, n);
        }
    }
    let mut days = Vec::new();
    for back in (0..span).rev() {
        let day: String = sqlx::query_scalar("SELECT date('now', ?)")
            .bind(format!("-{back} days"))
            .fetch_one(&state.db)
            .await
            .unwrap_or_default();
        days.push(DayViews { views: day_map.get(&day).copied().unwrap_or(0), day });
    }
    let new_members: i64 = days.iter().map(|d| d.views).sum();

    // Ghost attributes signups to the post or source that brought them in;
    // this port records the signup source, which is the part it has.
    let sources: String = sqlx::query_as::<_, (String, i64)>(
        "SELECT source, COUNT(*) AS n FROM members_created_events \
         WHERE created_at >= datetime('now', ?) GROUP BY source ORDER BY n DESC LIMIT 10",
    )
    .bind(&range.cutoff)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(source, n)| {
        analytics_row(
            &svg("person"),
            "teal",
            &analytics_source_label(&source),
            "Signup source",
            &countable(n, "member"),
            None,
        )
    })
    .collect();

    let recent: String = sqlx::query_as::<_, (Option<String>, String, String, String)>(
        "SELECT name, email, status, substr(created_at, 1, 10) FROM members ORDER BY created_at DESC LIMIT 8",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(name, email, status, created)| {
        let display = name.clone().unwrap_or_else(|| email.clone());
        let initial = display.chars().next().unwrap_or('?').to_uppercase().to_string();
        format!(
            r#"<div class="row"><span class="avatar">{initial}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{email} · joined {created}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span></div></div>"#,
            initial = esc(&initial),
            name = esc(&display),
            email = esc(&email),
            created = esc(&created),
            chip = if status == "paid" { "published" } else { "draft" },
            status = esc(&status),
        )
    })
    .collect();

    format!(
        r#"<div class="tiles rise" style="--i:1">
<div class="tile"><div class="label">Total members</div><div class="value">{total}</div><div class="delta">+{new} in this range</div></div>
<div class="tile"><div class="label">Free members</div><div class="value">{free}</div><div class="delta">no active subscription</div></div>
<div class="tile"><div class="label">Paid members</div><div class="value">{paid}</div><div class="delta">subscribed</div></div>
<div class="tile"><div class="label">MRR</div><div class="value">{mrr}</div><div class="delta">from active subscriptions</div></div>
</div>
<div class="section">New members per day</div>
<div class="card padded rise" style="--i:2">{chart}</div>
<div class="section">Signup sources</div>
<div class="card rows rise" style="--i:3">{sources}</div>
<div class="section">Recent members</div>
<div class="card rows rise" style="--i:4">{recent}</div>"#,
        total = compact(total),
        new = new_members,
        free = compact(free),
        paid = compact(paid),
        mrr = money_minor(mrr),
        chart = views_chart(&days),
        sources = analytics_rows(sources, "No signups in this range yet."),
        recent = analytics_rows(recent, "No members yet."),
    )
}

/// Ghost's signup sources read as words rather than the stored key.
fn analytics_source_label(source: &str) -> String {
    match source {
        "member" => "Signup form".to_string(),
        "api" => "API".to_string(),
        "admin" => "Added in admin".to_string(),
        "import" => "Imported".to_string(),
        "system" => "System".to_string(),
        other => other.to_string(),
    }
}

/// Newsletters: subscribers, what went out, and how it was opened.
async fn analytics_newsletters_tab(state: &AppState, range: &AnalyticsRange) -> String {
    let subscribers: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT member_id) FROM members_newsletters",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let (sent, opened): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN r.opened_at IS NOT NULL THEN 1 ELSE 0 END), 0) \
         FROM email_recipients r JOIN emails e ON e.id = r.email_id \
         WHERE r.created_at >= datetime('now', ?) AND r.status = 'sent'",
    )
    .bind(&range.cutoff)
    .fetch_one(&state.db)
    .await
    .unwrap_or((0, 0));
    let open_rate = if sent > 0 { (opened as f64 / sent as f64) * 100.0 } else { 0.0 };

    // One row per email sent in the window, with its own open rate — Ghost's
    // sortable top-newsletters table, ordered by what was opened most.
    let rows: String = sqlx::query_as::<_, (String, String, i64, i64)>(
        "SELECT COALESCE(p.title, e.subject), substr(e.created_at, 1, 10), \
                COUNT(r.id), COALESCE(SUM(CASE WHEN r.opened_at IS NOT NULL THEN 1 ELSE 0 END), 0) \
         FROM emails e LEFT JOIN posts p ON p.id = e.post_id \
         LEFT JOIN email_recipients r ON r.email_id = e.id AND r.status = 'sent' \
         WHERE e.created_at >= datetime('now', ?) \
         GROUP BY e.id ORDER BY e.created_at DESC LIMIT 15",
    )
    .bind(&range.cutoff)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(title, sent_on, delivered, opens)| {
        let rate = if delivered > 0 { (opens as f64 / delivered as f64) * 100.0 } else { 0.0 };
        analytics_row(
            &svg("send"),
            "purple",
            &title,
            &format!("Sent {} · {} delivered", esc(&sent_on), delivered),
            &format!("{rate:.0}% opened"),
            None,
        )
    })
    .collect();

    format!(
        r#"<div class="tiles rise" style="--i:1">
<div class="tile"><div class="label">Subscribers</div><div class="value">{subscribers}</div><div class="delta">across all newsletters</div></div>
<div class="tile"><div class="label">Emails delivered</div><div class="value">{sent}</div><div class="delta">in this range</div></div>
<div class="tile"><div class="label">Average open rate</div><div class="value">{open_rate:.0}%</div><div class="delta">{opened} opened</div></div>
</div>
<div class="section">Newsletters sent</div>
<div class="card rows rise" style="--i:2">{rows}</div>
<p class="muted" style="margin:10px 2px 0">Open rates need <strong>Email opens</strong> switched on in
<a href="/ghost/settings/analytics/">Settings &rsaquo; Analytics</a>. Enable <strong>Email clicks</strong> to track links. Open <strong>Newsletter links</strong> in the post editor to view clicks and update destinations after sending.</p>"#,
        subscribers = compact(subscribers),
        sent = compact(sent),
        open_rate = open_rate,
        opened = opened,
        rows = analytics_rows(rows, "No newsletters sent in this range."),
    )
}


// ---------------------------------------------------------------------------
// members (Ghost's Members list: everyone subscribed to the site)
// ---------------------------------------------------------------------------

use ghost_core::ids::uuid_v4;

async fn members_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let banner = flash_banner(&q);
    let mut rows = String::new();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let page = q.get("page").and_then(|v|v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let (members, filtered_count) = match repos::members::browse(&state.db,q.get("nql").map(String::as_str),100,page).await {
        Ok(result)=>result,Err(e)=>return (StatusCode::UNPROCESSABLE_ENTITY,e.message()).into_response()
    };
    {
        for member in members {
            let (id,name,email,status,note,created) = (member.id,member.name,member.email,member.status,member.note,member.created_at.chars().take(10).collect::<String>());
            let initial = name.as_deref().unwrap_or(&email).chars().next().unwrap_or('?').to_uppercase().to_string();
            rows.push_str(&format!(
                r#"<div class="row" data-filterable>
<span class="avatar">{initial}</span>
<a class="body" href="/ghost/members/{id}/"><span class="title">{name}</span><span class="detail">{email} · since {created}</span></a>
<div class="trailing">{note}<span class="chip {chip}">{status}</span>
<form method="post" action="/ghost/members/{id}/delete/" data-confirm="Delete member|&#8220;{name}&#8221; will be permanently removed.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>{chev}</div>
</div>"#,
                initial = esc(&initial),
                name = esc(name.as_deref().unwrap_or(&email)),
                email = esc(&email),
                created = esc(&created),
                chip = if status == "paid" { "published" } else { "draft" },
                status = esc(&status),
                note = note
                    .filter(|n| !n.is_empty())
                    .map(|n| format!(r#"<span class="muted" title="{}">…</span> "#, esc(n.as_str())))
                    .unwrap_or_default(),
                id = esc(&id),
                del = svg("delete"),
                chev = svg("chevron"),
            ));
        }
    }
    let list = if count == 0 {
        r#"<div class="card padded rise" style="--i:3;text-align:center"><p class="muted" style="margin:8px 0 16px">No members yet — add the first one below.</p></div>"#.to_string()
    } else {
        format!(
            r#"<div class="search rise" style="--i:4">{search}<input data-filter type="search" placeholder="Search members" aria-label="Search members"></div>
<div class="card rows rise" style="--i:5">{rows}</div>"#,
            search = svg("search"),
            rows = rows,
        )
    };
    let filter_query = form_urlencoded::Serializer::new(String::new()).append_pair("nql",q.get("nql").map(String::as_str).unwrap_or("")).finish();
    let pagination = format!(r#"<div style="display:flex;gap:16px;align-items:center;margin:16px 0"><span>{filtered_count} matching members · page {page}</span>{previous}{next}</div>"#,
        previous=if page>1 {format!(r#"<a class="btn tonal small" href="?{filter_query}&amp;page={}">Previous</a>"#,page-1)} else {String::new()},
        next=if page*100<filtered_count {format!(r#"<a class="btn tonal small" href="?{filter_query}&amp;page={}">Next</a>"#,page+1)} else {String::new()});
    let list=format!("{list}{pagination}");
    page_for_role(actor_role, 
        "Members",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Members</h1><p class="sub">{count} {word}</p></div></header>
{banner}
<div class="section">New member</div>
<div class="card padded rise" style="--i:2"><form method="post" action="/ghost/members/">
<div class="grid2">
<label class="field"><input name="name" placeholder=" " ><span>Name · optional</span></label>
<label class="field"><input name="email" type="email" placeholder=" " required><span>Email</span></label>
</div>
<label class="field"><input name="note" placeholder=" "><span>Note · optional</span></label>
<button class="btn tonal" type="submit">{add} Add member</button>
</form></div>
<div class="section">Import &amp; export</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{upload}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Import members</span>
<form method="post" action="/ghost/members/import/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<input type="file" name="file" accept=".csv,text/csv" required class="filefield">
<button class="btn tonal small" type="submit">Import CSV</button>
</form>
<p class="muted" style="margin:10px 0 0">A CSV with at least an <code>email</code> column. <code>name</code>, <code>note</code>, <code>labels</code>, <code>subscribed_to_emails</code>, <code>complimentary_plan</code> and <code>stripe_customer_id</code> are used when present; anything else is ignored. Rows whose email already exists update that member instead of duplicating it.</p>
</div></div>
<div class="row"><span class="ic tint-blue">{download}</span>
<div class="body"><span class="title">Export members</span><span class="detail">All members as CSV, in Ghost's column order</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/export/">Export CSV</a></div></div>
</div>
<div class="section">All members</div>
{list}"#,
            upload = svg("upload"),
            download = svg("open"),
            count = count,
            word = if count == 1 { "member" } else { "members" },
            banner = banner,
            add = svg("add"),
            list = list,
        ),
        true,
    )
    .into_response()
}

#[derive(Deserialize)]
struct MemberForm {
    name: Option<String>,
    email: String,
    note: Option<String>,
}

// --- members CSV: export and import, in Ghost's column order ---

/// Ghost's export columns, from `@tryghost/members-csv`. Keeping the order
/// and names identical means a file exported here imports into Ghost, and
/// vice versa.
const MEMBER_CSV_COLUMNS: &[&str] = &[
    "id", "email", "name", "note", "subscribed_to_emails", "complimentary_plan",
    "stripe_customer_id", "created_at", "deleted_at", "labels", "tiers",
];

/// Every member as CSV.
///
/// Separate from the download handler because the migration bundle puts the
/// same bytes inside a zip: one definition of what a member export *is*, so
/// the standalone file and the backup can never disagree.
async fn build_members_csv(state: &AppState) -> Option<Vec<u8>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String, Option<String>)>(
        "SELECT m.id, m.email, m.name, m.note, m.status, m.created_at, sc.customer_id \
         FROM members m \
         LEFT JOIN members_stripe_customers sc ON sc.member_id = m.id \
         ORDER BY m.created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(MEMBER_CSV_COLUMNS).ok()?;
    for (id, email, name, note, status, created_at, customer_id) in rows {
        let labels = member_labels(state, &id).await.join(",");
        let tiers = sqlx::query_scalar::<_, String>(
            "SELECT p.name FROM products p JOIN members_products mp ON mp.product_id = p.id WHERE mp.member_id = ?",
        )
        .bind(&id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
        .join(",");
        // Ghost's `subscribed_to_emails` is "is this member on any
        // newsletter", not a column of its own.
        let subscribed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members_newsletters WHERE member_id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
        let record = [
            id,
            email,
            name.unwrap_or_default(),
            note.unwrap_or_default(),
            if subscribed > 0 { "true".into() } else { "false".into() },
            if status == "comped" { "true".into() } else { "false".into() },
            customer_id.unwrap_or_default(),
            created_at,
            String::new(), // deleted_at: rows are hard-deleted here
            labels,
            tiers,
        ];
        w.write_record(&record).ok()?;
    }
    w.into_inner().ok()
}

/// Today's date, for the dated filenames Ghost's exports carry.
fn export_date_stamp() -> String {
    ghost_core::dates::now_db().chars().take(10).collect()
}

fn csv_download(filename: &str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        body,
    )
        .into_response()
}

/// Every member as CSV, as a file download.
async fn members_export(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match build_members_csv(&state).await {
        Some(body) => csv_download(&format!("members.{}.csv", export_date_stamp()), body),
        None => members_flash("error", "Could not build the export."),
    }
}

/// Outcome of an import, reported back to the operator.
#[derive(Default)]
struct ImportTally {
    created: usize,
    updated: usize,
    skipped: usize,
}

/// Import members from a CSV.
///
/// Matching is by email, which is the member's identity: a row whose address
/// already exists updates that member rather than creating a duplicate.
/// Unknown columns are ignored, so a file exported from Ghost — which carries
/// more columns than this port uses — imports cleanly.
/// Where a stashed upload waits between the mapping screen and the import.
///
/// The browser cannot hold the file across two posts, and re-uploading it to
/// confirm a mapping would be worse, so it is parked on disk under a random
/// name and removed as soon as it has been read.
fn import_stash_path(state: &AppState, token: &str) -> Option<std::path::PathBuf> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let dir = state.config.content_path().join("data").join("imports");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(format!("{token}.csv")))
}

/// The member fields an uploaded column can be pointed at.
const IMPORT_FIELDS: &[(&str, &str)] = &[
    ("email", "Email"),
    ("name", "Name"),
    ("note", "Note"),
    ("subscribed_to_emails", "Subscribed to emails"),
    ("complimentary_plan", "Complimentary plan"),
    ("labels", "Labels"),
    ("stripe_customer_id", "Stripe customer ID"),
];

/// Step one: read the upload, park it, and ask which column is which.
///
/// Ghost guesses by name and lets you correct it, which is the whole point —
/// before this, a column called `Email Address` simply did not import.
async fn members_import(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let data = loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return members_flash("error", "No CSV file was uploaded."),
            Err(e) => return members_flash("error", &upload_error_message(&e)),
        };
        if field.name() != Some("file") {
            continue;
        }
        match field.bytes().await {
            Ok(d) => break d.to_vec(),
            Err(e) => return members_flash("error", &upload_error_message(&e)),
        }
    };

    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(data.as_slice());
    let headers_row = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => return members_flash("error", &format!("That file is not valid CSV: {e}")),
    };
    let columns: Vec<String> = headers_row.iter().map(|h| h.trim().to_string()).collect();
    if columns.is_empty() {
        return members_flash("error", "That CSV has no header row.");
    }
    // A sample row makes the mapping checkable at a glance.
    let sample: Vec<String> = reader
        .records()
        .next()
        .and_then(|r| r.ok())
        .map(|r| r.iter().map(|v| v.trim().to_string()).collect())
        .unwrap_or_default();

    let token = ghost_core::ids::random_hex(16);
    let Some(path) = import_stash_path(&state, &token) else {
        return members_flash("error", "Could not stage the upload.");
    };
    if std::fs::write(&path, &data).is_err() {
        return members_flash("error", "Could not stage the upload.");
    }

    // Ghost's guess: an exact-ish name match, so a tidy CSV needs no edits.
    let guess = |field: &str| -> Option<usize> {
        let norm = |s: &str| s.trim().to_lowercase().replace([' ', '-'], "_");
        let target = norm(field);
        columns
            .iter()
            .position(|c| norm(c) == target)
            .or_else(|| match field {
                "email" => columns.iter().position(|c| norm(c).contains("email")),
                "subscribed_to_emails" => columns.iter().position(|c| norm(c) == "subscribed"),
                _ => None,
            })
    };

    let options = |selected: Option<usize>| -> String {
        let mut out = format!(
            r#"<option value="">{}</option>"#,
            if selected.is_none() { "— not imported —" } else { "— not imported —" }
        );
        for (i, c) in columns.iter().enumerate() {
            out.push_str(&format!(
                r#"<option value="{i}"{sel}>{name}</option>"#,
                i = i,
                sel = if selected == Some(i) { " selected" } else { "" },
                name = esc(c),
            ));
        }
        out
    };

    let field_rows: String = IMPORT_FIELDS
        .iter()
        .map(|(key, label)| {
            let picked = guess(key);
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="map_{key}">{opts}</select><span>{label}{req}</span>{car}</label>
<p class="muted" style="margin:6px 0 0">{preview}</p></div></div>"#,
                ic = svg("person"),
                key = esc(key),
                opts = options(picked),
                label = esc(label),
                req = if *key == "email" { " · required" } else { "" },
                car = SELECT_CARET,
                preview = match picked.and_then(|i| sample.get(i)) {
                    Some(v) if !v.is_empty() => format!("First row: <code>{}</code>", esc(v)),
                    _ => String::new(),
                },
            )
        })
        .collect();

    // Custom fields are importable too, which is most of why mapping matters:
    // a column nobody anticipated can still land somewhere useful.
    let metafields = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let metafield_rows: String = metafields
        .iter()
        .filter(|m| m.is_active() && m.parts().is_none())
        .map(|m| {
            let picked = columns
                .iter()
                .position(|c| c.trim().to_lowercase().replace(' ', "_") == m.key);
            format!(
                r#"<div class="row"><span class="ic tint-purple">{ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="mf_{key}">{opts}</select><span>{label}</span>{car}</label></div></div>"#,
                ic = svg("notes"),
                key = esc(&m.key),
                opts = options(picked),
                label = esc(&m.name),
                car = SELECT_CARET,
            )
        })
        .collect();

    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    page_for_role(actor_role,
        "Import members",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/members/" title="Back to members">{back}</a>
<div><h1>Import members</h1><p class="sub">{count} column{plural} found — check where each one goes</p></div>
<div class="actions"><button class="btn" type="submit" form="import-form">{check} Import</button></div></header>
<form method="post" action="/ghost/members/import/run/" id="import-form">
<input type="hidden" name="token" value="{token}">
<div class="section">Member fields</div>
<div class="card rows rise" style="--i:1">{field_rows}</div>
{metafield_section}
<p class="muted" style="margin:14px 0 0 16px">Rows without a valid email are skipped. An email that already exists is updated rather than duplicated.</p>
</form>"#,
            back = svg("chevron"),
            check = svg("check"),
            count = columns.len(),
            plural = if columns.len() == 1 { "" } else { "s" },
            token = esc(&token),
            field_rows = field_rows,
            metafield_section = if metafield_rows.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<div class="section">Custom fields</div><div class="card rows rise" style="--i:2">{metafield_rows}</div>"#
                )
            },
        ),
        true,
    )
    .into_response()
}

/// Step two: apply the mapping and import.
async fn members_import_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let token = form.get("token").cloned().unwrap_or_default();
    let Some(path) = import_stash_path(&state, &token) else {
        return members_flash("error", "That upload has expired — please upload the file again.");
    };
    let Ok(data) = std::fs::read(&path) else {
        return members_flash("error", "That upload has expired — please upload the file again.");
    };
    // Read once: the file is gone whether the import succeeds or not, so a
    // stash cannot be replayed or left lying around.
    let _ = std::fs::remove_file(&path);

    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(data.as_slice());
    if reader.headers().is_err() {
        return members_flash("error", "That file is not valid CSV.");
    }
    let mapped = |field: &str| -> Option<usize> {
        form.get(&format!("map_{field}")).and_then(|v| v.parse::<usize>().ok())
    };
    let (c_email, c_name, c_note, c_sub, c_comp, c_labels, c_stripe) = (
        mapped("email"),
        mapped("name"),
        mapped("note"),
        mapped("subscribed_to_emails"),
        mapped("complimentary_plan"),
        mapped("labels"),
        mapped("stripe_customer_id"),
    );
    let Some(c_email) = c_email else {
        return members_flash("error", "Choose which column holds the email address.");
    };

    // Custom-field columns, by key.
    let metafield_cols: Vec<(String, usize)> = form
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("mf_")
                .and_then(|key| v.parse::<usize>().ok().map(|i| (key.to_string(), i)))
        })
        .collect();

    // Default newsletters, for rows that ask to be subscribed.
    let default_newsletters: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM newsletters WHERE status = 'active' AND subscribe_on_signup = 1 ORDER BY sort_order",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let truthy = |v: &str| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes");
    let mut tally = ImportTally::default();
    let now = ghost_core::dates::now_db();

    for record in reader.records().flatten() {
        let get = |i: Option<usize>| i.and_then(|i| record.get(i)).unwrap_or("").trim().to_string();
        let email = get(Some(c_email)).to_lowercase();
        if email.is_empty() || !email.contains('@') {
            tally.skipped += 1;
            continue;
        }
        let name = get(c_name);
        let note = get(c_note);
        let comped = truthy(&get(c_comp));
        let subscribe = c_sub.map(|_| truthy(&get(c_sub))).unwrap_or(true);
        let status = if comped { "comped" } else { "free" };

        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM members WHERE lower(email) = ?")
            .bind(&email)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

        let member_id = match existing {
            Some(id) => {
                let res = sqlx::query(
                    "UPDATE members SET name = COALESCE(NULLIF(?, ''), name), \
                     note = COALESCE(NULLIF(?, ''), note), status = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&name).bind(&note).bind(status).bind(&now).bind(&id)
                .execute(&state.db).await;
                if res.is_err() {
                    tally.skipped += 1;
                    continue;
                }
                tally.updated += 1;
                id
            }
            None => {
                let id = ghost_core::ids::new_id();
                let res = sqlx::query(
                    "INSERT INTO members (id, uuid, transient_id, email, name, note, status, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(ghost_core::ids::uuid_v4())
                .bind(ghost_core::ids::uuid_v4())
                .bind(&email)
                .bind(if name.is_empty() { None } else { Some(&name) })
                .bind(if note.is_empty() { None } else { Some(&note) })
                .bind(status)
                .bind(&now).bind(&now)
                .execute(&state.db).await;
                if res.is_err() {
                    tally.skipped += 1;
                    continue;
                }
                let _ = sqlx::query("INSERT INTO members_created_events (id, member_id, source, created_at) VALUES (?, ?, 'import', ?)")
                    .bind(ghost_core::ids::new_id()).bind(&id).bind(&now)
                    .execute(&state.db).await;
                tally.created += 1;
                id
            }
        };

        let labels = get(c_labels);
        if !labels.is_empty() {
            sync_member_labels(&state, &member_id, &labels).await;
        }

        // Custom fields, where a column was pointed at one. Written through
        // the same path the admin uses, so the same validation applies and a
        // bad cell is skipped rather than stored.
        if !metafield_cols.is_empty() {
            let mut writes: HashMap<String, HashMap<String, String>> = HashMap::new();
            for (key, idx) in &metafield_cols {
                let value = get(Some(*idx));
                if !value.is_empty() {
                    writes.entry(key.clone()).or_default().insert(String::new(), value);
                }
            }
            if !writes.is_empty() {
                let writer = repos::metafields::Writer {
                    writer_type: "import".into(),
                    writer_id: None,
                    source: "import".into(),
                };
                let _ = repos::metafields::write_values(&state.db, &member_id, &writes, &writer).await;
            }
        }
        if subscribe {
            for nid in &default_newsletters {
                let _ = sqlx::query(
                    "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) \
                     SELECT ?, ?, ?, 'confirmed' WHERE NOT EXISTS \
                     (SELECT 1 FROM members_newsletters WHERE member_id = ? AND newsletter_id = ?)",
                )
                .bind(ghost_core::ids::new_id()).bind(&member_id).bind(nid).bind(&member_id).bind(nid)
                .execute(&state.db).await;
            }
        } else {
            let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
                .bind(&member_id)
                .execute(&state.db)
                .await;
        }
        let stripe = get(c_stripe);
        if !stripe.is_empty() {
            let _ = sqlx::query(
                "INSERT INTO members_stripe_customers (id, member_id, customer_id, created_at, updated_at) \
                 SELECT ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM members_stripe_customers WHERE customer_id = ?)",
            )
            .bind(ghost_core::ids::new_id()).bind(&member_id).bind(&stripe).bind(&now).bind(&now).bind(&stripe)
            .execute(&state.db).await;
        }
    }

    let skipped = if tally.skipped > 0 {
        format!(", {} skipped (missing or invalid email)", tally.skipped)
    } else {
        String::new()
    };
    members_flash(
        "ok",
        &format!("Imported {} new and updated {}{skipped}.", tally.created, tally.updated),
    )
}

// --- member editor (Ghost's Members → <member>) ---

/// The posted member editor. Newsletter checkboxes repeat under one name, so
/// this needs the same hand-rolled parsing the editor's tags use — `Form`
/// would keep only the last one.
#[derive(Debug, Default)]
struct MemberEditForm {
    name: String,
    email: String,
    note: String,
    labels: String,
    status: String,
    newsletters: Vec<String>,
    /// Custom field values, as `key` → (`path` → value). The inputs are named
    /// `mf:<key>:<path>`, with an empty path for a scalar.
    metafields: HashMap<String, HashMap<String, String>>,
}

impl MemberEditForm {
    fn parse(body: &str) -> Self {
        let mut f = MemberEditForm::default();
        for (k, v) in form_urlencoded::parse(body.as_bytes()) {
            match k.as_ref() {
                "name" => f.name = v.into_owned(),
                "email" => f.email = v.into_owned(),
                "note" => f.note = v.into_owned(),
                "labels" => f.labels = v.into_owned(),
                "status" => f.status = v.into_owned(),
                "newsletters" => f.newsletters.push(v.into_owned()),
                other => {
                    // `mf:<key>:<path>` — split on the first two colons only,
                    // so a path is free to contain one.
                    if let Some(rest) = other.strip_prefix("mf:") {
                        if let Some((key, path)) = rest.split_once(':') {
                            f.metafields
                                .entry(key.to_string())
                                .or_default()
                                .insert(path.to_string(), v.into_owned());
                        }
                    }
                }
            }
        }
        f
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for MemberEditForm {
    type Rejection = Response;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(MemberEditForm::parse(&String::from_utf8_lossy(&bytes)))
    }
}

fn member_edit_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/members/{id}/?{encoded}")).into_response()
}

/// Labels attached to a member, as a comma-separated string for the form.
async fn member_labels(state: &AppState, member_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT l.name FROM labels l JOIN members_labels ml ON ml.label_id = l.id \
         WHERE ml.member_id = ? ORDER BY l.name",
    )
    .bind(member_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

async fn member_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let row = sqlx::query_as::<_, (String, Option<String>, String, String, Option<String>, String, Option<String>, Option<String>)>(
        "SELECT id, name, email, status, note, created_at, uuid, commenting FROM members WHERE id = ? LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((mid, name, email, status, note, created, _uuid, commenting)) = row else {
        return not_found_page(&user, "That member could not be found.");
    };

    // Commenting is per-member state a moderator acts on from here: a
    // suspension is aimed at a person, not at a comment, so it belongs on
    // their profile rather than on the moderation queue.
    let commenting = ghost_api::comments::Commenting::parse(commenting.as_deref());
    let comment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE member_id = ? AND status <> 'deleted'")
            .bind(&mid)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let labels = member_labels(&state, &mid).await;
    let subscribed: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT newsletter_id FROM members_newsletters WHERE member_id = ?",
    )
    .bind(&mid)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let newsletters = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT id, name, description FROM newsletters WHERE status = 'active' ORDER BY sort_order ASC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let newsletter_rows: String = if newsletters.is_empty() {
        r#"<p class="muted" style="margin:0">No active newsletters.</p>"#.into()
    } else {
        newsletters
            .iter()
            .map(|(nid, nname, ndesc)| {
                format!(
                    r#"<label class="fchip" title="{desc}"><input type="checkbox" name="newsletters" value="{nid}" {checked}><span class="ck">{check}</span>{nname}</label>"#,
                    nid = esc(nid),
                    checked = if subscribed.iter().any(|s| s == nid) { "checked" } else { "" },
                    nname = esc(nname),
                    check = svg("check"),
                    desc = esc(ndesc.as_deref().unwrap_or("")),
                )
            })
            .collect()
    };

    // --- the member's custom fields -----------------------------------------
    //
    // Staff see every field whatever its member access, which is what that
    // setting is for: `none` means the member cannot see it, not that nobody
    // can. Archived fields show only when this member still holds a value, so
    // a retired field's data stays visible without inviting new entries.
    let definitions = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let held = repos::metafields::values_for_member(&state.db, &mid).await.unwrap_or_default();
    let mut metafield_rows = String::new();
    for def in &definitions {
        let values = held.get(&def.key);
        if !def.is_active() && values.map(|v| v.is_empty()).unwrap_or(true) {
            continue;
        }
        let at = |path: &str| {
            values.and_then(|v| v.get(path)).map(String::as_str).unwrap_or_default().to_string()
        };
        // Inputs are named `mf:<key>:<path>` so one flat form carries every
        // field and part without the names colliding with the member's own.
        let body = match def.parts() {
            Some(parts) => {
                let inputs: String = parts
                    .iter()
                    .map(|(path, label, part_type)| {
                        let max = match *part_type {
                            "postal_code" => ghost_core::metafields::MAX_POSTAL_CODE_CHARS,
                            "country_code" => 2,
                            _ => ghost_core::metafields::MAX_SHORT_TEXT_CHARS,
                        };
                        format!(
                            r#"<label class="field" style="margin:0"><input name="mf:{key}:{path}" value="{v}" placeholder=" " maxlength="{max}"><span>{label}</span></label>"#,
                            key = esc(&def.key),
                            path = esc(path),
                            v = esc(&at(path)),
                            max = max,
                            label = esc(label),
                        )
                    })
                    .collect();
                format!(r#"<div class="grid2" style="gap:12px">{inputs}</div>"#)
            }
            None if def.field_type == "long_text" => format!(
                r#"<label class="field" style="margin:0"><textarea name="mf:{key}:" rows="3" placeholder=" ">{v}</textarea><span>{name}</span></label>"#,
                key = esc(&def.key),
                v = esc(&at("")),
                name = esc(&def.name),
            ),
            None => format!(
                r#"<label class="field" style="margin:0"><input name="mf:{key}:" value="{v}" placeholder=" " maxlength="{max}"><span>{name}</span></label>"#,
                key = esc(&def.key),
                v = esc(&at("")),
                max = ghost_core::metafields::MAX_SHORT_TEXT_CHARS,
                name = esc(&def.name),
            ),
        };
        metafield_rows.push_str(&format!(
            r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{ic}</span>
<div class="body" style="width:100%">{heading}{body}</div></div>"#,
            ic = svg("notes"),
            // The inputs below float their own label into their top border, so
            // a heading needs clearance or the two overlap.
            heading = if def.parts().is_some() {
                format!(
                    r#"<span class="title" style="display:block;margin:0 0 14px">{}{}</span>"#,
                    esc(&def.name),
                    if def.is_active() { "" } else { " · archived" }
                )
            } else if def.is_active() {
                String::new()
            } else {
                r#"<span class="detail" style="display:block;margin:0 0 14px">archived</span>"#
                    .to_string()
            },
            body = body,
        ));
    }
    let metafields_card = if metafield_rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="section">Custom fields</div>
<div class="card rows rise" style="--i:3">{metafield_rows}</div>"#
        )
    };

    let initial = name.as_deref().unwrap_or(&email).chars().next().unwrap_or('?').to_uppercase().to_string();
    page_for_role(actor_role,
        "Member",
        "members",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/members/" title="Back to members">{back}</a>
<div><h1>{title}</h1><p class="sub">Member since {created}</p></div>
<div class="actions"><button class="btn" type="submit" form="member-form">{check} Save</button></div></header>
{banner}
<div class="card padded rise" style="--i:1;display:flex;align-items:center;gap:16px;flex-wrap:wrap">
<span class="avatar lg">{initial}</span>
<div style="min-width:0"><b style="font-size:17px;display:block">{title}</b><span class="muted">{email}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span></div>
</div>
<form method="post" action="/ghost/members/{id}/" id="member-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{ghost_ic}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" "><span>Name</span></label>
<label class="field" style="margin:0"><input name="email" type="email" value="{email_v}" placeholder=" " required><span>Email</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="note" rows="2" placeholder=" " maxlength="2000">{note}</textarea><span>Note · staff only, never shown to the member</span></label>
<label class="field" style="margin:12px 0 0"><input name="labels" value="{labels}" placeholder=" "><span>Labels · comma separated</span></label>
<p class="muted" style="margin:10px 0 0">Labels group members for filtering and segmentation. New names are created as you type them.</p>
</div></div>
</div>
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{mail}</span>
<div class="body" style="width:100%"><div class="chips">{newsletter_rows}</div>
<p class="muted" style="margin:10px 0 0">Unticking every newsletter unsubscribes the member from email entirely.</p>
</div></div>
</div>
{metafields_card}
<div class="section">Access</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:center"><span class="ic tint-amber">{tags_ic}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="status">{status_opts}</select><span>Status</span>{car}</label>
<p class="muted" style="margin:6px 0 0">{status_note}</p></div></div>
</div>
</form>
{commenting_card}
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:5">
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Delete this member</span><span class="detail">Removes the account, its subscriptions and its labels. This cannot be undone.</span></div>
<div class="trailing">
<form method="post" action="/ghost/members/{id}/delete/" data-confirm="Delete member|&#8220;{title}&#8221; will be permanently removed.">
<button class="btn tonal small danger" type="submit">Delete</button></form></div></div>
</div>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&mid),
            initial = esc(&initial),
            title = esc(name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&email)),
            email = esc(&email),
            email_v = esc(&email),
            name_v = esc(name.as_deref().unwrap_or("")),
            created = esc(&created[..10.min(created.len())]),
            chip = if status == "free" { "draft" } else { "published" },
            status = esc(&status),
            note = esc(note.as_deref().unwrap_or("")),
            labels = esc(&labels.join(", ")),
            ghost_ic = svg("ghost"),
            mail = svg("mail"),
            tags_ic = svg("tags"),
            del = svg("delete"),
            car = SELECT_CARET,
            newsletter_rows = newsletter_rows,
            metafields_card = metafields_card,
            commenting_card = comments::member_commenting_card(&mid, &commenting, comment_count),
            status_opts = select_options(
                &[("free", "Free"), ("comped", "Complimentary — paid access, no charge")],
                &status
            ),
            status_note = if status == "paid" {
                "This member has a paid subscription. Changing it here does not touch billing, so the list is limited to the states this port can honour."
            } else {
                "Complimentary grants paid-tier access without Stripe. Real paid subscriptions are not managed here."
            },
        ),
        true,
    )
    .into_response()
}

async fn member_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    form: MemberEditForm,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let email = form.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return member_edit_flash(&id, "error", "A valid email is required.");
    }
    // Email identifies a member at signin, so a collision would let one
    // person's magic link land on another's account.
    let clash: Option<String> = sqlx::query_scalar("SELECT id FROM members WHERE lower(email) = ? AND id != ?")
        .bind(&email)
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if clash.is_some() {
        return member_edit_flash(&id, "error", "Another member already uses that email.");
    }

    let status = match form.status.as_str() {
        "comped" => "comped",
        "paid" => "paid",
        _ => "free",
    };
    let now = ghost_core::dates::now_db();
    let updated = sqlx::query(
        "UPDATE members SET name = ?, email = ?, note = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(form.name.trim())
    .bind(&email)
    .bind(form.note.trim())
    .bind(status)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await;
    if let Err(e) = updated {
        return member_edit_flash(&id, "error", &format!("Could not save: {e}"));
    }

    // Newsletters: the posted set replaces whatever was stored, so an
    // unticked box really does unsubscribe.
    let _ = sqlx::query("DELETE FROM members_newsletters WHERE member_id = ?")
        .bind(&id)
        .execute(&state.db)
        .await;
    for nid in &form.newsletters {
        let _ = sqlx::query(
            "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, 'confirmed')",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&id)
        .bind(nid)
        .execute(&state.db)
        .await;
    }

    sync_member_labels(&state, &id, &form.labels).await;

    // Custom fields. A rejected value is reported rather than swallowed: the
    // rest of the member has already saved, so the message says what did not.
    if !form.metafields.is_empty() {
        let actor = match require_session(&state, &headers).await {
            Ok(u) => u.id,
            Err(_) => String::new(),
        };
        let writer = repos::metafields::Writer::staff(&actor);
        if let Err(e) =
            repos::metafields::write_values(&state.db, &id, &form.metafields, &writer).await
        {
            return member_edit_flash(
                &id,
                "error",
                &format!("Member saved, but a custom field was not — {}", e.message()),
            );
        }
    }

    fire_member_webhook(&state, "member.edited", &id, &email, form.name.trim());
    member_edit_flash(&id, "ok", "Member saved.")
}

/// Replace a member's labels with the comma-separated list, creating any
/// label that does not exist yet (Ghost does the same from its label picker).
async fn sync_member_labels(state: &AppState, member_id: &str, csv: &str) {
    let _ = sqlx::query("DELETE FROM members_labels WHERE member_id = ?")
        .bind(member_id)
        .execute(&state.db)
        .await;
    let now = ghost_core::dates::now_db();
    let mut seen: Vec<String> = Vec::new();
    for raw in csv.split(',') {
        let name = raw.trim();
        if name.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            continue;
        }
        seen.push(name.to_string());
        // labels.name is unique, so reuse an existing row rather than failing
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM labels WHERE name = ?")
            .bind(name)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
        let label_id = match existing {
            Some(lid) => lid,
            None => {
                let lid = ghost_core::ids::new_id();
                if sqlx::query("INSERT INTO labels (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
                    .bind(&lid)
                    .bind(name)
                    .bind(&now)
                    .bind(&now)
                    .execute(&state.db)
                    .await
                    .is_err()
                {
                    continue;
                }
                lid
            }
        };
        let _ = sqlx::query(
            "INSERT INTO members_labels (id, member_id, label_id, sort_order) VALUES (?, ?, ?, 0)",
        )
        .bind(ghost_core::ids::new_id())
        .bind(member_id)
        .bind(&label_id)
        .execute(&state.db)
        .await;
    }
}

async fn member_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<MemberForm>) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let email = form.email.trim().to_string();
    if email.is_empty() || !email.contains('@') {
        return members_flash("error", "A valid email is required.");
    }
    let now = ghost_core::dates::now_db();
    let id = ghost_core::ids::new_id();
    let res = sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, name, note, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'free', ?, ?)",
    )
    .bind(&id)
    .bind(uuid_v4())
    .bind(ghost_core::ids::new_id())
    .bind(&email)
    .bind(form.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
    .bind(form.note.as_deref().map(str::trim).filter(|n| !n.is_empty()))
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => {
            let _ = repos::actions::record(&state.db, "members", &id, &user.id, "added").await;
            fire_member_webhook(&state, "member.added", &id, &email, form.name.as_deref().unwrap_or("").trim());
            members_flash("ok", format!("Member {email} added.").as_str())
        }
        Err(e) if e.to_string().contains("UNIQUE") => members_flash("error", "A member with that email already exists."),
        Err(e) => members_flash("error", &format!("Could not add member: {e}")),
    }
}

async fn member_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Who they were, read before the row goes: a delete webhook carrying
    // only an id tells a CRM nothing it can act on.
    let gone: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT email, name FROM members WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

    // clear the join tables first — FKs point at members
    for table in ["members_products", "members_newsletters", "members_labels", "members_stripe_customers", "subscriptions"] {
        let _ = sqlx::query(&format!("DELETE FROM {table} WHERE member_id = ?"))
            .bind(&id)
            .execute(&state.db)
            .await;
    }
    let _ = sqlx::query("DELETE FROM members WHERE id = ?").bind(&id).execute(&state.db).await;
    let _ = repos::actions::record(&state.db, "members", &id, &user.id, "deleted").await;
    if let Some((email, name)) = gone {
        fire_member_webhook(&state, "member.deleted", &id, &email, name.as_deref().unwrap_or(""));
    }
    members_flash("ok", "Member deleted.")
}

/// Redirect back to the members page carrying a one-shot banner.
fn members_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/members/?{encoded}")).into_response()
}

// ---------------------------------------------------------------------------
// posts list / editor
// ---------------------------------------------------------------------------

async fn posts_list_impl(state: &AppState, headers: &HeaderMap, post_type: &str, label: &str, flash: &HashMap<String, String>) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let user = &actor.user;
    let filter = match flash.get("nql").filter(|s|!s.is_empty()).map(|s|ghost_nql::parse(s)).transpose() {
        Ok(filter)=>filter, Err(e)=>return (StatusCode::UNPROCESSABLE_ENTITY,e.message()).into_response()
    };
    let q = repos::posts::BrowsePosts {
        filter,
        order: None,
        limit: 100,
        page: 1,
        post_type: Some(post_type.to_string()),
        public: false,
        include_authors: false,
        include_tags: false,
    };
    let mut listing = repos::posts::browse(&state.db, &q).await.unwrap_or(ghost_db::repos::posts::PostsPage { posts: vec![], total: 0, limit: 15, page: 1 });
    // Authors and Contributors only ever see their own work, so the list is
    // filtered rather than merely hiding the edit links.
    if !actor.role.can_edit_any_post() {
        let mine: Vec<String> = sqlx::query_scalar("SELECT post_id FROM posts_authors WHERE author_id = ?")
            .bind(&actor.user.id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
        listing.posts.retain(|p| mine.iter().any(|id| id == &p.id));
        listing.total = listing.posts.len() as i64;
    }

    let base = if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" };
    let singular = if post_type == "page" { "page" } else { "post" };
    let plural = if post_type == "page" { "pages" } else { "posts" };

    // The status tabs count the whole list, then the selected one narrows it,
    // so a tab's number does not change when you click a different tab.
    let mut counts: HashMap<String, i64> = HashMap::new();
    for p in &listing.posts {
        *counts.entry(p.status.clone()).or_insert(0) += 1;
    }
    let selected = flash.get("filter").map(String::as_str).unwrap_or("all");
    let selected = posts::FILTERS
        .iter()
        .find(|(key, _, _)| *key == selected)
        .unwrap_or(&posts::FILTERS[0]);
    if let Some(status) = selected.2 {
        listing.posts.retain(|p| p.status == status);
    }
    let shown = listing.posts.len() as i64;

    let sort = flash.get("sort").map(String::as_str).unwrap_or("newest");
    let sort = posts::SORTS
        .iter()
        .find(|(key, _)| *key == sort)
        .map(|(key, _)| *key)
        .unwrap_or("newest");

    let mut cards = posts::load_cards(state, listing.posts).await;
    posts::sort_cards(&mut cards, sort);
    let list = if cards.is_empty() {
        let (title, detail) = if selected.0 == "all" {
            ("Nothing here yet", "Everything you write shows up here, newest first.")
        } else {
            ("Nothing in this view", "Switch tabs to see the rest.")
        };
        format!(
            r#"<div class="card padded rise" style="--i:1;text-align:center;padding:48px 20px">
<p style="margin:0 0 4px;font-weight:600;font-size:17px">{title}</p>
<p class="muted" style="margin:0 0 20px">{detail}</p>
<a class="btn" href="{base}new/">{add} New {singular}</a></div>"#,
            title = esc(title),
            detail = esc(detail),
            base = base,
            add = svg("add"),
            singular = singular,
        )
    } else {
        format!(
            r#"<div class="list-tools rise" style="--i:0">
<div class="search">{search}<input data-filter type="search" placeholder="Search {plural} by title, author or excerpt" aria-label="Search {label}"></div>
{sorter}</div>
{grid}"#,
            search = svg("search"),
            plural = plural,
            label = label,
            sorter = posts::sort_picker(base, selected.0, sort),
            grid = posts::grid(&cards, base),
        )
    };

    page_for(
        label,
        plural,
        Some(user),
        actor.role,
        &format!(
            r#"<header class="page-head rise"><div><h1>{label}</h1><p class="sub">{shown} of {total} {total_word}</p></div>
<div class="actions"><a class="btn" href="{base}new/">{add} New {singular}</a></div></header>
{banner}
{filters}
{list}"#,
            banner = flash_banner(flash),
            label = label,
            shown = shown,
            total = listing.total,
            total_word = plural,
            base = base,
            add = svg("add"),
            singular = singular,
            filters = posts::filter_bar(base, selected.0, sort, &counts),
            list = list,
        ),
        true,
    )
    .into_response()
}

async fn posts_list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    posts_list_impl(&state, &headers, "post", "Posts", &q).await
}

async fn pages_list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    posts_list_impl(&state, &headers, "page", "Pages", &q).await
}
// ---------------------------------------------------------------------------
// the post/page editor
//
// Content is edited in Koenig — Ghost's own editor, loaded from the bundle
// vendored in `assets/` — and everything Ghost keeps behind its slide-over
// "post settings" panel lives in a second column that is always open.
//
// Koenig is a React component, so the content field only exists once its
// JavaScript has run. The markdown textarea the editor replaced is still in
// the page underneath it, disabled on load and re-enabled if the bundle fails
// to start, which keeps the editor usable without JavaScript at all.
// ---------------------------------------------------------------------------

/// Everything the editor page renders from. Gathered by the handlers so the
/// markup function stays a pure function of its inputs.
struct Editor<'a> {
    post_type: &'a str,
    /// `None` when composing something that does not exist yet.
    post: Option<&'a ghost_core::models::Post>,
    tags: Vec<ghost_core::models::Tag>,
    all_tags: Vec<ghost_core::models::Tag>,
    authors: Vec<ghost_core::models::User>,
    all_authors: Vec<ghost_core::models::User>,
    /// `custom-*.hbs` offered by the active theme, as (file stem, label).
    templates: Vec<(String, String)>,
    /// True once this post has been emailed; the newsletter picker is then
    /// withheld so an edit cannot send it a second time.
    already_sent: bool,
    /// What a scheduled post is already set to do when its date arrives, so
    /// re-opening it shows that choice rather than resetting it.
    delivery: repos::posts::Delivery,
    /// False for Contributors, who may only save drafts.
    can_publish: bool,
    all_tiers: Vec<(String, String)>,
    members_enabled: bool,
    /// Request headers for api.unsplash.com, or `Null` when the Unsplash app
    /// is off or has no access key. Koenig uses this object verbatim.
    unsplash_headers: serde_json::Value,
    pintura_config: serde_json::Value,
    site_url: String,
    site_title: String,
    site_description: String,
}

/// Ghost's registered Unsplash application — a public Client-ID, the one
/// Ghost's own admin ships and talks to api.unsplash.com with. An install
/// that has not supplied its own key gets the same out-of-the-box picker
/// Ghost gives.
const GHOST_UNSPLASH_KEY: &str = "8672af113b0a8573edae3aa3713886265d9bb741d707f6c01a486cde8c278980";

/// The Authorization header Koenig needs to query Unsplash, if this site can.
///
/// The app has to be switched on; the access key is optional, because Ghost's
/// own registered application key stands in for one, exactly as Ghost's admin
/// does it. Koenig uses this object verbatim.
fn unsplash_headers(state: &AppState) -> serde_json::Value {
    if !state.settings.get_bool("unsplash") {
        return serde_json::Value::Null;
    }
    let key = state.settings.get_str("unsplash_access_key").unwrap_or_default();
    let key = key.trim();
    let key = if key.is_empty() { GHOST_UNSPLASH_KEY } else { key };
    serde_json::json!({
        "Authorization": format!("Client-ID {key}"),
        "Accept-Version": "v1",
    })
}

fn pintura_config(settings: &ghost_db::SettingsCache) -> serde_json::Value {
    if !settings.get_bool("pintura") { return serde_json::Value::Null; }
    let js = settings.get_str("pintura_js_url").unwrap_or_default();
    let css = settings.get_str("pintura_css_url").unwrap_or_default();
    if js.trim().is_empty() || css.trim().is_empty() { return serde_json::Value::Null; }
    serde_json::json!({"jsUrl": js.trim(), "cssUrl": css.trim()})
}

impl Editor<'_> {
    fn singular(&self) -> &'static str {
        if self.post_type == "page" {
            "page"
        } else {
            "post"
        }
    }

    fn base(&self) -> &'static str {
        if self.post_type == "page" {
            "/ghost/pages/"
        } else {
            "/ghost/posts/"
        }
    }

    fn id(&self) -> Option<&str> {
        self.post.map(|p| p.id.as_str())
    }

    /// A `posts` column, or "" for a post being composed.
    fn field(&self, get: impl Fn(&ghost_core::models::Post) -> Option<&str>) -> &str {
        self.post.and_then(get).unwrap_or("")
    }

    /// A `posts_meta` column, or "" when there is no meta row yet.
    fn meta(&self, get: impl Fn(&ghost_core::models::PostMeta) -> Option<&str>) -> &str {
        self.post.and_then(|p| p.meta.as_ref()).and_then(get).unwrap_or("")
    }
}

/// A collapsible settings panel in the right-hand column. "Post settings" —
/// the URL, date and excerpt a post is opened to settle — is the one panel
/// that starts open; every other panel stands folded until asked for.
/// `<details>` means every panel folds and unfolds without any JavaScript.
fn panel(icon_name: &str, tint: &str, title: &str, body: &str, open: bool) -> String {
    format!(
        r#"<details class="card panel"{open_attr}><summary><span class="ic tint-{tint}">{ic}</span>{title}<span class="twist">{twist}</span></summary>
<div class="panel-body">{body}</div></details>"#,
        open_attr = if open { " open" } else { "" },
        tint = tint,
        ic = svg(icon_name),
        title = esc(title),
        twist = svg("down"),
        body = body,
    )
}

/// A `text_field` the editor's script has to reach by id — the feature image
/// URL, the slug — same markup, plus the handle.
fn text_field_id(name: &str, id: &str, label: &str, value: &str) -> String {
    format!(
        r#"<label class="field"><input name="{name}" id="{id}" value="{value}" placeholder=" "><span>{label}</span></label>"#,
        name = esc(name),
        id = esc(id),
        value = esc(value),
        label = esc(label),
    )
}

/// An outlined text field with a floating label.
fn text_field(name: &str, label: &str, value: &str) -> String {
    format!(
        r#"<label class="field"><input name="{name}" value="{value}" placeholder=" "><span>{label}</span></label>"#,
        name = esc(name),
        value = esc(value),
        label = esc(label),
    )
}

fn textarea_field(name: &str, label: &str, value: &str, rows: u32, code: bool) -> String {
    format!(
        r#"<label class="field"><textarea name="{name}" rows="{rows}" placeholder=" "{cls}>{value}</textarea><span>{label}</span></label>"#,
        name = esc(name),
        rows = rows,
        cls = if code { r#" class="code""# } else { "" },
        value = esc(value),
        label = esc(label),
    )
}

fn select_field(name: &str, label: &str, options: &str) -> String {
    format!(
        r#"<label class="field fixed"><select name="{name}">{options}</select><span>{label}</span><svg class="car" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 10l5 5 5-5z"/></svg></label>"#,
        name = esc(name),
        options = options,
        label = esc(label),
    )
}

fn option_row(value: &str, label: &str, selected: bool) -> String {
    format!(
        r#"<option value="{value}"{sel}>{label}</option>"#,
        value = esc(value),
        sel = if selected { " selected" } else { "" },
        label = esc(label),
    )
}

fn toggle_row(name: &str, title: &str, detail: &str, on: bool) -> String {
    format!(
        r#"<div class="row toggle-row"><div class="label"><b>{title}</b><span>{detail}</span></div>
<label class="switch"><input type="checkbox" name="{name}" value="1"{on}><span class="track"><span class="thumb"></span></span></label></div>"#,
        title = esc(title),
        detail = esc(detail),
        name = esc(name),
        on = if on { " checked" } else { "" },
    )
}

/// A set of checkbox chips posting under one repeated name.
fn chip_set(name: &str, options: &[(String, String)], selected: &[String], empty: &str) -> String {
    if options.is_empty() {
        return format!(r#"<p class="hint">{}</p>"#, esc(empty));
    }
    let chips: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                r#"<label class="fchip"><input type="checkbox" name="{name}" value="{value}"{on}><span class="ck">{check}</span>{label}</label>"#,
                name = esc(name),
                value = esc(value),
                on = if selected.iter().any(|s| s == value) { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    format!(r#"<div class="chips">{chips}</div>"#)
}

/// The segmented filter bar the list pages share. Each tab is a `filter=`
/// value with a label and the bucket it counts — `None` is the "everything"
/// tab, whose count is the sum of the rest — and `suffix` carries any other
/// query params that should survive the click (the posts list keeps its sort
/// this way). A tab that would show nothing is not offered, except the one
/// already selected, which has to stay clickable to be unselectable.
pub(crate) fn seg_filter_bar(
    tabs: &[(&str, &str, Option<&str>)],
    base: &str,
    active: &str,
    suffix: &str,
    counts: &HashMap<String, i64>,
) -> String {
    let segs: String = tabs
        .iter()
        .map(|(key, label, bucket)| {
            let count = match bucket {
                Some(b) => counts.get(*b).copied().unwrap_or(0),
                None => counts.values().sum(),
            };
            if count == 0 && *key != active {
                return String::new();
            }
            format!(
                r#"<a class="seg{on}" href="{base}?filter={key}{suffix}">{label}<span class="seg-count">{count}</span></a>"#,
                on = if *key == active { " on" } else { "" },
                base = esc(base),
                key = esc(key),
                suffix = suffix,
                label = esc(label),
                count = count,
            )
        })
        .collect();
    format!(r#"<div class="seg-bar">{segs}</div>"#)
}

/// `2026-10-19 14:30:00` (how the column is stored) → the `YYYY-MM-DDTHH:MM`
/// a `datetime-local` input wants.
fn to_datetime_local(stored: &str) -> String {
    if stored.len() < 16 {
        return String::new();
    }
    format!("{}T{}", &stored[..10], &stored[11..16])
}

/// The inverse. Times are read and written as UTC, which is what the column
/// holds and what the rest of this admin displays.
fn from_datetime_local(input: &str) -> Option<String> {
    let input = input.trim();
    if input.len() < 16 {
        return None;
    }
    let (date, time) = input.split_once('T')?;
    Some(format!("{date} {time}:00"))
}

fn editor_html(ed: &Editor<'_>) -> String {
    let base = ed.base();
    let singular = ed.singular();
    let action = match ed.id() {
        Some(id) => format!("{base}{id}/"),
        None => format!("{base}new/"),
    };
    let status = ed.post.map(|p| p.status.as_str()).unwrap_or(if ed.can_publish { "published" } else { "draft" });
    let title = ed.field(|p| p.title.as_deref());

    // --- the content column ------------------------------------------------

    // Koenig reopens the Lexical state it saved. A post written before this
    // editor existed has only markdown (or, for a seeded row, only HTML), so
    // that is wrapped into a one-card document instead — opening an old post
    // must never be able to lose anything in it.
    let markdown = ed.field(|p| p.markdown.as_deref());
    let saved = ed.field(|p| p.lexical.as_deref().filter(|l| !l.trim().is_empty()));
    let initial = if saved.is_empty() {
        ghost_render::lexical::wrap_existing(Some(markdown), Some(ed.field(|p| p.html.as_deref())))
    } else {
        Some(saved.to_string())
    };
    // Content is stored without the install's base path, but Koenig renders
    // it live — an image card has to be able to load its `src`. The write path
    // normalizes it back, so reopening and saving a post does not persist the
    // base path. A no-op at a domain root.
    let initial = initial
        .as_deref()
        .map(ghost_core::url::unbase_assets)
        .map(|lexical| ghost_core::url::base_assets(&lexical));
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": format!("Begin writing your {singular}..."),
        "postType": ed.post_type,
        "isNew": ed.id().is_none(),
        "savedStatus": ed.post.map(|p| p.status.as_str()).unwrap_or("draft"),
        "siteUrl": ed.site_url,
        "siteTitle": ed.site_title,
        "siteDescription": ed.site_description,
        "membersEnabled": ed.members_enabled,
        // the bridge prefixes its upload and oembed endpoints with this; it
        // cannot read the base path off the page, since its own script URL has
        // already been rewritten
        "basePath": ghost_core::url::subdir(),
        // Unsplash: the editor talks to api.unsplash.com directly, so what it
        // needs is the Authorization header. Sent only when the app is on and
        // a key exists; otherwise Koenig shows its bundled sample gallery.
        "unsplash": ed.unsplash_headers,
        "pinturaConfig": ed.pintura_config,
    });

    let content = format!(
        r#"<div class="editor-main">
<textarea class="editor-title" id="field-title" name="title" rows="1" placeholder="{singular_title} title">{title}</textarea>
<div id="koenig-mount" data-config="{config}"></div>
<input type="hidden" name="lexical" id="field-lexical" value="">
<input type="hidden" name="html" id="field-html" value="">
<div id="markdown-fallback">
<p class="hint" data-fallback-note>JavaScript is off, so the rich editor cannot run. This {singular} is editable as Markdown.</p>
{markdown_field}
</div>
<span class="wc-pill" id="word-count" aria-live="polite"></span>
</div>"#,
        singular_title = if singular == "page" { "Page" } else { "Post" },
        title = esc(title),
        config = esc(&config.to_string()),
        singular = singular,
        markdown_field = textarea_field("markdown", "Content · Markdown", markdown, 20, true),
    );

    // --- the settings column -----------------------------------------------
    //
    // The panels run in Ghost's own order — feature image first, then the
    // post's URL and date, tags, access, authors, template, code injection,
    // meta data, the social cards. "Post settings" is the one panel that
    // starts open; the rest stand folded, one summary line each, so the rail
    // reads as a menu of what there is to set rather than a wall of forms.
    // Publishing, holding the status select that stands in for Ghost's header
    // publish flow, is this port's own.
    let mut side = String::new();

    // Feature image: an image box — upload or Unsplash while empty, a
    // preview with Change and Remove once set — over the URL field and the
    // alt text and caption Ghost keeps in posts_meta.
    let feature_image = ed.field(|p| p.feature_image.as_deref());
    // The Unsplash picker is always offered; without an access key it opens
    // and says what is missing rather than hiding the option entirely.
    let unsplash_btn =
        format!(r#"<button type="button" class="btn small ghosted" id="fimg-unsplash">{search} Unsplash</button>"#, search = svg("search"));
    side.push_str(&panel(
        "image",
        "green",
        "Feature image",
        &format!(
            r#"<input type="file" id="fimg-file" accept="image/gif,image/jpeg,image/png,image/svg+xml,image/webp" hidden>
<div class="fimg-drop{set}" id="fimg-drop">
<div class="fimg-empty" id="fimg-empty"><div class="fimg-btns">
<button type="button" class="btn tonal small" id="fimg-upload">{upload} Upload</button>{unsplash}
</div></div>
<img id="fimg-img" src="{src}" alt="">
<div class="fimg-acts">
<button type="button" class="btn small ghosted" id="fimg-change">Change image</button>
<button type="button" class="btn small ghosted danger" id="fimg-remove">Remove</button>
</div>
<div class="fimg-prog" id="fimg-prog"><i></i></div>
</div>
<details class="panel-sub"><summary>Alt text, caption &amp; image URL{twist}</summary>
<div class="panel-sub-body">{alt}{caption}{url}</div></details>"#,
            set = if feature_image.is_empty() { "" } else { " is-set" },
            src = esc(&ghost_core::url::base_asset(feature_image)),
            upload = svg("upload"),
            unsplash = unsplash_btn,
            twist = svg("down"),
            alt = text_field("feature_image_alt", "Alt text", ed.meta(|m| m.feature_image_alt.as_deref())),
            caption = text_field_id(
                "feature_image_caption",
                "field-feature_image_caption",
                "Caption",
                ed.meta(|m| m.feature_image_caption.as_deref()),
            ),
            url = text_field_id("feature_image", "field-feature_image", "Image URL", feature_image),
        ),
        true,
    ));

    // General: URL, publish date, excerpt — the panel an open editor starts from.
    let slug = ed.field(|p| p.slug.as_deref());
    side.push_str(&panel(
        "link",
        "blue",
        &format!("{} settings", if singular == "page" { "Page" } else { "Post" }),
        &format!(
            r#"{slug_field}<p class="hint" id="slug-preview">{preview}</p>
<label class="field fixed"><input type="datetime-local" name="published_at" value="{published_at}"><span>Publish date · UTC</span></label>
{excerpt}<p class="hint">Shown in post lists and feeds in place of the opening lines.</p>"#,
            slug_field = text_field_id("slug", "field-slug", "URL slug", slug),
            // Only worth showing once there is a slug: the bare origin with
            // two slashes after it is not a URL anyone needs to see.
            preview = if slug.is_empty() {
                String::new()
            } else {
                format!("{}/<b>{}</b>/", esc(&ed.site_url), esc(slug))
            },
            published_at = esc(&to_datetime_local(ed.field(|p| p.published_at.as_deref()))),
            excerpt = textarea_field("custom_excerpt", "Excerpt", ed.field(|p| p.custom_excerpt.as_deref()), 3, false),
        ),
        true,
    ));

    // Status and, for a brand-new post, whether it also goes out as email.
    let status_options = format!(
        "{draft}{rest}",
        draft = option_row("draft", "Draft", status == "draft"),
        rest = if ed.can_publish {
            format!(
                "{}{}",
                option_row("published", "Published", status == "published"),
                option_row("scheduled", "Scheduled", status == "scheduled"),
            )
        } else {
            String::new()
        },
    );
    // Offered when creating a post, and while one is scheduled: the email for
    // a scheduled post has not gone out yet, so the choice is still live and
    // has to be editable right up to its date. Pages are never emailed, and
    // an edit to anything already out must never be able to mail subscribers
    // a second time.
    let scheduled = status == "scheduled";
    let publish_action = if ed.post_type == "page"
        || ed.already_sent
        || !ed.can_publish
        || (ed.id().is_some() && !scheduled)
    {
        String::new()
    } else {
        let current = ed.delivery;
        format!(
            r#"{select}<p class="hint">{hint}</p>"#,
            select = select_field(
                "publish_action",
                "When publishing",
                &format!(
                    "{}{}{}",
                    option_row("publish_only", "Publish only", !current.email),
                    option_row("publish_email", "Publish and email", current.email && !current.email_only),
                    option_row("email_only", "Email only", current.email_only),
                ),
            ),
            hint = if scheduled {
                "Decided when the date arrives. Change it any time before then."
            } else {
                "Emailing happens once, when the post is first created."
            },
        )
    };
    // The date is what a scheduled post is waiting for, so say so where it is
    // set rather than leaving an empty box to guess at.
    let schedule_hint = if scheduled {
        match ed.field(|p| p.published_at.as_deref()) {
            "" => r#"<p class="hint">Set a publish date above — a scheduled post needs one in the future.</p>"#.to_string(),
            at => format!(
                r#"<p class="hint">Goes out {} UTC, on its own.</p>"#,
                esc(&to_datetime_local(at).replace('T', " at ")),
            ),
        }
    } else {
        String::new()
    };
    side.push_str(&panel(
        "flag",
        "pink",
        "Publishing",
        &format!(
            "{status}{schedule_hint}{publish_action}{featured}{show_title}",
            status = select_field("status", "Status", &status_options),
            featured = toggle_row(
                "featured",
                "Feature this",
                "Themes can give featured posts their own treatment.",
                ed.post.map(|p| p.featured).unwrap_or(false),
            ),
            show_title = toggle_row(
                "show_title_and_feature_image",
                "Show title and feature image",
                "Turn off for a bare canvas with no header.",
                ed.post.map(|p| p.show_title_and_feature_image).unwrap_or(true),
            ),
        ),
        false,
    ));

    let tag_options: Vec<(String, String)> = ed
        .all_tags
        .iter()
        .filter(|t| t.visibility == "public")
        .map(|t| (t.id.clone(), t.name.clone().unwrap_or_default()))
        .collect();
    let tag_selected: Vec<String> = ed.tags.iter().map(|t| t.id.clone()).collect();
    side.push_str(&panel(
        "tags",
        "amber",
        "Tags",
        &chip_set("tags", &tag_options, &tag_selected, "No tags yet — create one under Tags."),
        false,
    ));

    // Access includes explicit paid-tier entitlements.
    let visibility = ed.post.map(|p| p.visibility.as_str()).unwrap_or("public");
    let access_body = if ed.members_enabled {
        select_field(
            "visibility",
            "Who can read this",
            &format!(
                "{}{}{}{}",
                option_row("public", "Everyone", visibility == "public"),
                option_row("members", "Members only", visibility == "members"),
                option_row("paid", "Paid members only", visibility == "paid"),
                option_row("tiers", "Specific paid tiers", visibility == "tiers"),
            ),
        )
    } else {
        format!(
            r#"<input type="hidden" name="visibility" value="{visibility}"><p class="hint">Memberships are off, so everything published is public. Turn them on under Settings → Membership.</p>"#,
            visibility = esc(visibility),
        )
    };
    let access_body = if ed.members_enabled {
        format!("{}<input type=\"hidden\" name=\"_tiers\" value=\"1\"><p class=\"hint\">Allowed tiers · used when access is set to Specific paid tiers.</p>{}", access_body,
            chip_set("tiers", &ed.all_tiers, ed.post.map(|p| p.tier_ids.as_slice()).unwrap_or(&[]), "Create a paid tier under Membership to restrict access by tier."))
    } else { access_body };
    side.push_str(&panel("lock", "slate", "Access", &access_body, false));

    // Email, when this post can still become one.
    if ed.post_type == "post" {
        let sent_note = if ed.already_sent {
            r#"<p class="hint">This post has already been emailed to subscribers. The subject is kept for the record.</p>"#
        } else {
            r#"<p class="hint">Used instead of the title in the subject line of the newsletter.</p>"#
        };
        side.push_str(&panel(
            "mail",
            "teal",
            "Email",
            &format!(
                "{subject}{sent_note}",
                subject = text_field("email_subject", "Email subject", ed.meta(|m| m.email_subject.as_deref())),
            ),
        false,
        ));
    }

    let author_options: Vec<(String, String)> =
        ed.all_authors.iter().map(|u| (u.id.clone(), u.name.clone().unwrap_or_default())).collect();
    let author_selected: Vec<String> = ed.authors.iter().map(|u| u.id.clone()).collect();
    side.push_str(&panel(
        "person",
        "purple",
        "Authors",
        &format!(
            r#"{chips}<p class="hint">With none ticked, the {singular} stays with whoever created it.</p>"#,
            chips = chip_set("authors", &author_options, &author_selected, "No staff users yet."),
            singular = singular,
        ),
        false,
    ));

    // Theme template, when the active theme offers a choice.
    let current_template = ed.field(|p| p.custom_template.as_deref());
    let template_body = if ed.templates.is_empty() {
        format!(
            r#"<input type="hidden" name="custom_template" value="{current}"><p class="hint">The active theme has no <code>custom-*.hbs</code> templates, so this {singular} uses the theme's default.</p>"#,
            current = esc(current_template),
            singular = singular,
        )
    } else {
        let options: String = std::iter::once(option_row("", "Default", current_template.is_empty()))
            .chain(ed.templates.iter().map(|(file, label)| option_row(file, label, file == current_template)))
            .collect();
        select_field("custom_template", "Template", &options)
    };
    side.push_str(&panel("layout", "slate", "Template", &template_body, false));

    // Per-post code injection.
    side.push_str(&panel(
        "code",
        "red",
        "Code injection",
        &format!(
            "{head}{foot}",
            head = textarea_field("codeinjection_head", "Head", ed.field(|p| p.codeinjection_head.as_deref()), 4, true),
            foot = textarea_field("codeinjection_foot", "Footer", ed.field(|p| p.codeinjection_foot.as_deref()), 4, true),
        ),
        false,
    ));

    // Search engines.
    side.push_str(&panel(
        "search",
        "blue",
        "Meta data",
        &format!(
            r#"{title}{description}{canonical}<p class="hint">Point search engines at the original when this is a repost.</p>"#,
            title = text_field("meta_title", "Meta title", ed.meta(|m| m.meta_title.as_deref())),
            description = textarea_field("meta_description", "Meta description", ed.meta(|m| m.meta_description.as_deref()), 3, false),
            canonical = text_field("canonical_url", "Canonical URL", ed.field(|p| p.canonical_url.as_deref())),
        ),
        false,
    ));

    // The two social cards Ghost lets you override per post.
    side.push_str(&panel(
        "share",
        "pink",
        "Social cards",
        &format!(
            r#"<p class="hint">Facebook, LinkedIn and anything else reading Open Graph.</p>{og_title}{og_desc}{og_image}
<p class="hint">X (Twitter).</p>{tw_title}{tw_desc}{tw_image}"#,
            og_title = text_field("og_title", "Open Graph title", ed.meta(|m| m.og_title.as_deref())),
            og_desc = textarea_field("og_description", "Open Graph description", ed.meta(|m| m.og_description.as_deref()), 2, false),
            og_image = text_field("og_image", "Open Graph image URL", ed.meta(|m| m.og_image.as_deref())),
            tw_title = text_field("twitter_title", "X title", ed.meta(|m| m.twitter_title.as_deref())),
            tw_desc = textarea_field("twitter_description", "X description", ed.meta(|m| m.twitter_description.as_deref()), 2, false),
            tw_image = text_field("twitter_image", "X image URL", ed.meta(|m| m.twitter_image.as_deref())),
        ),
        false,
    ));

    // --- head, and the page around both columns -----------------------------

    let heading = if ed.id().is_some() {
        format!("Edit {singular}")
    } else {
        format!("New {singular}")
    };
    let preview = format!(
        r#"<a class="btn text" id="editor-view" href="{site}/{slug}/" target="_blank" rel="noopener"{hidden}>{open} View {singular}</a>"#,
        site = esc(&ed.site_url),
        slug = esc(ed.field(|p| p.slug.as_deref())),
        hidden = if status == "published" && ed.id().is_some() { "" } else { " hidden" },
        open = svg("open"),
    );
    // The delete form cannot be nested inside the editor form, so it sits in
    // the head and the settings column links to it by id.
    let delete = match ed.id() {
        Some(id) => format!(
            r#"<form method="post" action="{base}{id}/delete/" id="delete-form" data-confirm="Delete {singular}|&#8220;{title}&#8221; will be permanently deleted."></form>"#,
            base = base,
            id = esc(id),
            singular = singular,
            title = esc(if title.is_empty() { "Untitled" } else { title }),
        ),
        None => String::new(),
    };
    if ed.id().is_some() {
        side.push_str(&format!(
            r#"<div class="card panel"><div class="panel-body" style="padding-top:14px">
<button class="btn text danger" type="submit" form="delete-form">{del} Delete {singular}</button></div></div>"#,
            del = svg("delete"),
            singular = singular,
        ));
    } else {
        // Nothing to delete until the first save creates the post; the
        // editor's script reveals this once it has an id to delete.
        side.push_str(&format!(
            r#"<div class="card panel" id="new-delete" hidden><div class="panel-body" style="padding-top:14px">
<button class="btn text danger" type="button">{del} Delete {singular}</button></div></div>"#,
            del = svg("delete"),
            singular = singular,
        ));
    }

    format!(
        r#"<style>
/* Koenig's type scale is written in rem against Ghost's admin, where the
   root is 62.5% so 1rem reads as 10px. This admin sizes everything in px and
   uses no rem at all, so re-basing the root here is invisible to the rest of
   the page and puts the editor — prose, toolbars, card menu — at the size it
   was drawn for. Scoped to this page rather than the shared stylesheet so a
   future rem elsewhere cannot be caught out by it. */
html {{ font-size: 62.5%; }}
</style>
<header class="page-head rise"><a class="icon-btn" href="{base}" title="Back to {plural}">{back}</a>
<div><h1>{heading}</h1></div>
<div class="actions"><a class="btn text" href="{base}">Cancel</a>{preview}
<span class="save-state" id="save-state" aria-live="polite"></span>
<input type="checkbox" id="side-toggle">
<label class="icon-btn side-btn" for="side-toggle" title="Show or hide settings">
<span class="when-shown">{hide}</span><span class="when-hidden">{show}</span></label>
<button class="btn" id="editor-save" type="submit" form="editor-form">{save} <span data-save-label>{save_label}</span></button></div></header>
{delete}
<form method="post" action="{action}" id="editor-form" class="editor-shell">
{content}
<aside class="editor-side">{side}</aside>
</form>
{pintura_assets}
<script src="{assets}/react.js" defer></script>
<script src="{assets}/react-jsx.js" defer></script>
<script src="{assets}/react-dom.js" defer></script>
<script src="{assets}/koenig-lexical.js" defer></script>
<script src="{assets}/unsplash-picker.js" defer></script>
<script src="{assets}/koenig-bridge.js" defer></script>
<script>
/* Hide the Markdown fallback before anything is painted, and disable it so a
   stale copy of the content cannot be posted alongside the editor's. The
   bridge puts it back if Koenig fails to start. */
(function () {{
  var fallback = document.getElementById('markdown-fallback');
  if (!fallback) {{ return; }}
  fallback.hidden = true;
  fallback.querySelector('textarea').disabled = true;
  /* The title is a textarea so it can wrap; it grows with its content. */
  var title = document.getElementById('field-title');
  var grow = function () {{ title.style.height = 'auto'; title.style.height = title.scrollHeight + 'px'; }};
  title.addEventListener('input', grow);
  title.addEventListener('keydown', function (e) {{ if (e.key === 'Enter') {{ e.preventDefault(); }} }});
  grow();

  /* The settings rail hides and shows through a checkbox and CSS alone, so
     it still works with this script absent — all the script adds is
     remembering the choice for the next post you open. */
  var toggle = document.getElementById('side-toggle');
  try {{
    toggle.checked = localStorage.getItem('ghost:editor-settings-hidden') === '1';
    toggle.addEventListener('change', function () {{
      localStorage.setItem('ghost:editor-settings-hidden', toggle.checked ? '1' : '0');
    }});
  }} catch (e) {{ /* private browsing: the toggle still works, it just forgets */ }}
}})();
</script>"#,
        base = base,
        plural = if ed.post_type == "page" { "pages" } else { "posts" },
        back = svg("chevron"),
        heading = esc(&heading),
        hide = svg("panel-hide"),
        show = svg("panel-show"),
        preview = preview,
        save = svg("check"),
        save_label = if ed.id().is_none() && ed.can_publish { "Publish".to_string() } else { format!("Save {singular}") },
        delete = delete,
        action = action,
        content = content,
        side = side,
        assets = ASSETS,
        pintura_assets = match (ed.pintura_config["jsUrl"].as_str(), ed.pintura_config["cssUrl"].as_str()) {
            (Some(js), Some(css)) => format!(r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script>"#, esc(css), esc(js)),
            _ => String::new(),
        },
    )
}

// ---------------------------------------------------------------------------
// editor assets
// ---------------------------------------------------------------------------

/// Koenig's version, used to bust caches. The bundle under `assets/` is
/// immutable for a given build, so the URL carries the version and the
/// response can then be cached for as long as the browser likes. The bridge
/// script changes with this admin rather than with Koenig, so the number
/// moves for either: a browser holding a stale bridge would keep a broken
/// editor (and a word count that never counts) until its cache expired.
const KOENIG_VERSION: &str = "1.8.14";

/// Where the editor's scripts live, version-stamped.
const ASSETS: &str = concat!("/ghost/assets/", "1.8.14");

/// Vendored editor runtime. Serving these from the binary keeps the admin a
/// single self-contained process — no CDN, and no network at all once the
/// page is open.
async fn admin_asset(Path((version, file)): Path<(String, String)>) -> Response {
    if version != KOENIG_VERSION {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let body: &'static str = match file.as_str() {
        "react.js" => include_str!("../assets/react.production.min.js"),
        "react-jsx.js" => include_str!("../assets/react-jsx-runtime.js"),
        "react-dom.js" => include_str!("../assets/react-dom.production.min.js"),
        // Ships its own stylesheet, which it injects into <head> on load.
        "koenig-lexical.js" => include_str!("../assets/koenig-lexical.umd.js"),
        "koenig-bridge.js" => include_str!("../assets/koenig-bridge.js"),
        "unsplash-picker.js" => include_str!("../assets/unsplash-picker.js"),
        "publishing-workflows.js" => include_str!("../assets/publishing-workflows.js"),
        "automation-editor.js" => include_str!("../assets/automation-editor.js"),
        "native-social.js" => include_str!("../assets/native-social.js"),
        "machine-payments.js" => include_str!("../assets/machine-payments.js"),
        "checkout-design.js" => include_str!("../assets/checkout-design.js"),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// loading the editor
// ---------------------------------------------------------------------------

/// `custom-*.hbs` in the active theme, as (stem, human label).
async fn custom_templates(state: &AppState) -> Vec<(String, String)> {
    let theme = state.theme.read().await.clone();
    let mut out: Vec<(String, String)> = theme
        .templates
        .iter()
        .filter_map(|t| t.strip_prefix("custom-"))
        .map(|rest| {
            let file = format!("custom-{rest}");
            // "custom-full-width" reads as "Full width" in Ghost's picker.
            let mut label = rest.replace(['-', '_'], " ");
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            (file, label)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Fill in everything the editor page needs that does not come from the post.
async fn editor_context<'a>(
    state: &AppState,
    post_type: &'a str,
    post: Option<&'a ghost_core::models::Post>,
    tags: Vec<ghost_core::models::Tag>,
    authors: Vec<ghost_core::models::User>,
    can_publish: bool,
) -> Editor<'a> {
    let already_sent = match post {
        Some(p) => newsletter_already_sent(state, &p.id).await,
        None => false,
    };
    let delivery = match post {
        Some(p) => repos::posts::delivery(&state.db, &p.id).await,
        None => repos::posts::Delivery::default(),
    };
    Editor {
        post_type,
        post,
        tags,
        all_tags: repos::tags::all_public(&state.db).await.unwrap_or_default(),
        authors,
        all_authors: repos::users::list(&state.db, false).await.unwrap_or_default(),
        templates: custom_templates(state).await,
        already_sent,
        delivery,
        can_publish,
        all_tiers: sqlx::query_as("SELECT id, name FROM products WHERE type = 'paid' ORDER BY name").fetch_all(&state.db).await.unwrap_or_default(),
        members_enabled: state.settings.get_str("members_signup_access").as_deref() != Some("none"),
        unsplash_headers: unsplash_headers(state),
        pintura_config: pintura_config(&state.settings),
        site_url: state.site_url(),
        site_title: state.settings.get_str("title").unwrap_or_default(),
        site_description: state.settings.get_str("description").unwrap_or_default(),
    }
}

async fn editor_new_impl(state: &AppState, headers: &HeaderMap, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let ed = editor_context(state, post_type, None, vec![], vec![], actor.role.can_publish()).await;
    page_for(
        "New",
        if post_type == "page" { "pages" } else { "posts" },
        Some(&actor.user),
        actor.role,
        &editor_html(&ed),
        true,
    )
    .into_response()
}

async fn editor_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    // ?type=page kept for links that predate the dedicated /pages/new/ route
    let post_type = if q.get("type").map(String::as_str) == Some("page") { "page" } else { "post" };
    editor_new_impl(&state, &headers, post_type).await
}

async fn page_new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    editor_new_impl(&state, &headers, "page").await
}

async fn editor_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(&state, &actor, &id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    let user = actor.user;
    let Some(post) = repos::posts::find_by_id(&state.db, &id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(&post))
        .await
        .unwrap_or_default();
    let tags = rel.tags_for(&post.id);
    let authors = rel.authors_for(&post.id);
    let ed = editor_context(&state, &post.post_type, Some(&post), tags, authors, actor.role.can_publish()).await;
    page_for(
        "Edit",
        if post.post_type == "page" { "pages" } else { "posts" },
        Some(&user),
        actor.role,
        &format!("{}{}", flash_banner(&q), editor_html(&ed)),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// saving
// ---------------------------------------------------------------------------

#[derive(Default, Debug, PartialEq)]
struct EditorForm {
    title: String,
    slug: Option<String>,
    /// Koenig's editor state, posted by the bridge script.
    lexical: Option<String>,
    /// The HTML Koenig rendered from that state in the browser. The cards
    /// know their own markup, so this is the authority; the server only
    /// renders the Lexical itself when this is missing.
    html: Option<String>,
    /// Only posted when the rich editor is not running.
    markdown: Option<String>,
    status: String,
    published_at: Option<String>,
    visibility: Option<String>,
    featured: bool,
    show_title_and_feature_image: bool,
    feature_image: Option<String>,
    feature_image_alt: Option<String>,
    feature_image_caption: Option<String>,
    custom_excerpt: Option<String>,
    /// One entry per ticked checkbox — they all post under the same name.
    tags: Vec<String>,
    authors: Vec<String>,
    tiers: Vec<String>,
    tiers_present: bool,
    custom_template: Option<String>,
    canonical_url: Option<String>,
    codeinjection_head: Option<String>,
    codeinjection_foot: Option<String>,
    meta_title: Option<String>,
    meta_description: Option<String>,
    og_title: Option<String>,
    og_description: Option<String>,
    og_image: Option<String>,
    twitter_title: Option<String>,
    twitter_description: Option<String>,
    twitter_image: Option<String>,
    email_subject: Option<String>,
    /// `publish_only` | `publish_email` | `email_only`.
    publish_action: String,
    audience: ghost_api::newsletter_audience::Audience,
    audience_present: bool,
}

impl EditorForm {
    /// Parse an `application/x-www-form-urlencoded` body.
    ///
    /// `axum::Form` cannot be used here: it goes through `serde_urlencoded`,
    /// which has no notion of a repeated key. The tag and author checkboxes
    /// each post under one name, so a single ticked box fails to deserialize
    /// as a sequence at all and several would collapse to one value.
    fn parse(body: &str) -> Self {
        let mut form = EditorForm::default();
        let keep = |v: String| Some(v).filter(|s| !s.trim().is_empty());

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            let value = value.into_owned();
            match key.as_ref() {
                "title" => form.title = value.trim().to_string(),
                "slug" => form.slug = keep(value),
                "lexical" => form.lexical = keep(value),
                "html" => form.html = keep(value),
                "markdown" => form.markdown = Some(value),
                "status" => form.status = value,
                "published_at" => form.published_at = from_datetime_local(&value),
                "visibility" => form.visibility = keep(value),
                "publish_action" => form.publish_action = value,
                "newsletter_id" => {form.audience.newsletter_id=keep(value);form.audience_present=true;},
                "audience_filter" => {form.audience.filter=keep(value);form.audience_present=true;},
                "newsletter_include_preview" => form.audience.include_preview=value=="1",
                "_audience" => form.audience_present=true,
                "featured" => form.featured = true,
                "show_title_and_feature_image" => form.show_title_and_feature_image = true,
                "feature_image" => form.feature_image = keep(value),
                "feature_image_alt" => form.feature_image_alt = keep(value),
                "feature_image_caption" => form.feature_image_caption = keep(value),
                "custom_excerpt" => form.custom_excerpt = keep(value),
                "custom_template" => form.custom_template = keep(value),
                "canonical_url" => form.canonical_url = keep(value),
                "codeinjection_head" => form.codeinjection_head = keep(value),
                "codeinjection_foot" => form.codeinjection_foot = keep(value),
                "meta_title" => form.meta_title = keep(value),
                "meta_description" => form.meta_description = keep(value),
                "og_title" => form.og_title = keep(value),
                "og_description" => form.og_description = keep(value),
                "og_image" => form.og_image = keep(value),
                "twitter_title" => form.twitter_title = keep(value),
                "twitter_description" => form.twitter_description = keep(value),
                "twitter_image" => form.twitter_image = keep(value),
                "email_subject" => form.email_subject = keep(value),
                "tags" => {
                    if !value.trim().is_empty() {
                        form.tags.push(value);
                    }
                }
                "_tiers" => form.tiers_present = true,
                "tiers" => { form.tiers_present = true; if !value.trim().is_empty() { form.tiers.push(value); } },
                "authors" => {
                    if !value.trim().is_empty() {
                        form.authors.push(value);
                    }
                }
                _ => {}
            }
        }
        form
    }

    /// (html, plaintext, reading time) for whichever editor was used.
    ///
    /// Koenig's own HTML is preferred because the cards render themselves;
    /// the Rust renderer covers a document that arrived without it.
    fn rendered(&self) -> (String, String, i64) {
        let html = match (&self.html, &self.lexical) {
            (Some(html), _) => html.clone(),
            (None, Some(lexical)) => ghost_render::lexical::to_html(lexical),
            (None, None) => ghost_render::markdown::to_html(self.markdown.as_deref().unwrap_or("")),
        };
        let plaintext = ghost_render::markdown::html_to_plaintext(&html);
        let reading_time = ghost_render::markdown::reading_time(&plaintext);
        (html, plaintext, reading_time)
    }

    /// Whether the rich editor was the one that posted this.
    fn is_koenig(&self) -> bool {
        self.lexical.is_some()
    }

    /// Whether this request carries the body of the post at all.
    ///
    /// It might not: the hidden fields are filled in by the editor as the
    /// form is submitted, so a save that somehow beats Koenig's startup —
    /// or a hand-made request — arrives with no content in it. Rendering
    /// that would write an empty post over a real one, so the save leaves
    /// the content columns alone instead and only applies the settings.
    fn has_content(&self) -> bool {
        self.lexical.is_some() || self.html.is_some() || self.markdown.is_some()
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for EditorForm {
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body = String::from_utf8_lossy(&bytes);
        Ok(EditorForm::parse(&body))
    }
}

fn form_to_new(form: &EditorForm, post_type: &str) -> repos::posts::NewPost {
    let (html, plaintext, reading_time) = form.rendered();
    repos::posts::NewPost {
        title: form.title.clone(),
        slug: form.slug.clone(),
        lexical: form.lexical.clone(),
        // A Koenig post has no markdown source, and storing the old field
        // alongside it would leave two disagreeing copies of the content.
        markdown: if form.is_koenig() { None } else { form.markdown.clone() },
        html: Some(html),
        plaintext: Some(plaintext),
        custom_excerpt: form.custom_excerpt.clone(),
        feature_image: form.feature_image.clone(),
        featured: form.featured,
        post_type: post_type.to_string(),
        status: form.status.clone(),
        visibility: form.visibility.clone().unwrap_or_else(|| "public".into()),
        published_at: form.published_at.clone(),
        codeinjection_head: form.codeinjection_head.clone(),
        codeinjection_foot: form.codeinjection_foot.clone(),
        custom_template: form.custom_template.clone(),
        canonical_url: form.canonical_url.clone(),
        show_title_and_feature_image: Some(form.show_title_and_feature_image),
        meta_title: form.meta_title.clone(),
        meta_description: form.meta_description.clone(),
        og_title: form.og_title.clone(),
        og_description: form.og_description.clone(),
        og_image: form.og_image.clone(),
        twitter_title: form.twitter_title.clone(),
        twitter_description: form.twitter_description.clone(),
        twitter_image: form.twitter_image.clone(),
        email_subject: form.email_subject.clone(),
        feature_image_alt: form.feature_image_alt.clone(),
        feature_image_caption: form.feature_image_caption.clone(),
        author_ids: form.authors.clone(),
        tag_ids: form.tags.clone(),
        tier_ids: form.tiers.clone(),
        reading_time,
    }
}

/// The columns the editor form owns. Listing them lets `update` tell "the
/// author emptied this box" from "this request never mentioned the field",
/// so clearing a meta title actually clears it.
fn editor_owned_fields(is_koenig: bool, has_content: bool) -> Vec<&'static str> {
    let mut fields = vec![
        "custom_excerpt",
        "feature_image",
        "codeinjection_head",
        "codeinjection_foot",
        "custom_template",
        "canonical_url",
        "meta_title",
        "meta_description",
        "og_title",
        "og_description",
        "og_image",
        "twitter_title",
        "twitter_description",
        "twitter_image",
        "email_subject",
        "feature_image_alt",
        "feature_image_caption",
    ];
    if has_content {
        fields.push("html");
        fields.push("plaintext");
        // Only the editor that posted may blank the other's source, so a
        // save from the Markdown fallback cannot wipe a Koenig document, or
        // vice versa — and a save carrying no body at all clears neither.
        fields.push(if is_koenig { "markdown" } else { "lexical" });
    }
    fields
}

/// What a save that asks for "Scheduled" actually amounts to.
enum Scheduling {
    /// Not a scheduled save; nothing to arrange.
    No,
    /// Hold it until this moment, then deliver it this way.
    At(repos::posts::Delivery),
    /// Scheduled to a date that has already passed, or to none at all.
    Undated,
}

/// Read the schedule off the form.
///
/// A date in the past is treated as no date rather than as "publish now": the
/// author picked a moment, and silently deciding they meant *this* moment is
/// how a half-written post ends up emailed to everyone.
fn scheduling(form: &EditorForm, post_type: &str, action: PublishAction) -> Scheduling {
    if form.status != "scheduled" {
        return Scheduling::No;
    }
    // `YYYY-MM-DD HH:MM:SS`, so later times sort later as text — no date
    // library needed to tell the future from the past.
    let future = form
        .published_at
        .as_deref()
        .is_some_and(|at| at.len() >= 19 && at > ghost_core::dates::now_db().as_str());
    if !future {
        return Scheduling::Undated;
    }
    // Pages have no newsletter; a page scheduled "and email" still only
    // appears on the site.
    let sends = post_type == "post" && action.sends_email();
    Scheduling::At(repos::posts::Delivery {
        email: sends,
        email_only: sends && action == PublishAction::EmailOnly,
    })
}

/// What the author is told when a schedule has no usable date. The post is
/// still saved — as a draft, with every word in it — because losing the
/// writing to a mis-set date would be the worse failure by far.
const UNDATED: &str = "Saved as a draft: a scheduled post needs a publish date in the future.";

/// True when a save came from the editor's own script, which asks for JSON —
/// a redirect would be followed to a full editor page it has no use for.
fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("application/json"))
}

/// What a save answers to the editor's script: the fields it has to adopt —
/// a first save creates the post, and the page then has to behave as though
/// it had always been `/posts/<id>/` — plus anything worth a banner.
async fn editor_saved_json(state: &AppState, post: &ghost_core::models::Post, message: &str, level: &str) -> Response {
    let url = if post.status == "published" {
        let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
        let data = ghost_core::models::post_to_json(post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
        let router = state.router.read().await;
        let path = router.permalink_for(&data)
            .map(|(_, permalink)| ghost_routing::tree::build_permalink(&permalink, &data))
            .unwrap_or_else(|| format!("/{}/", post.slug.as_deref().unwrap_or_default()));
        Some(format!("{}{path}", state.site_url().trim_end_matches('/')))
    } else {
        None
    };
    let body = serde_json::json!({
        "id": post.id,
        "slug": post.slug,
        "status": post.status,
        "title": post.title,
        "url": url,
        "message": message,
        "level": level,
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

/// A save the script made that the server refused, worded for the banner.
fn editor_error_json(message: &str, status: StatusCode) -> Response {
    let body = serde_json::json!({"message": message, "level": "error"});
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn editor_create_impl(state: &AppState, headers: &HeaderMap, mut form: EditorForm, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !actor.role.can_publish() {
        form.status = "draft".into();
    }
    let user = actor.user;
    if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::validate(state,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
    let action = PublishAction::parse(&form.publish_action);
    let schedule = scheduling(&form, post_type, action);
    if matches!(schedule, Scheduling::Undated) {
        form.status = "draft".into();
    }
    // A scheduled post sends nothing now — that is the whole point of it.
    let going_out = !matches!(schedule, Scheduling::At(_))
        && post_type == "post"
        && action.sends_email()
        && (form.status == "published" || action == PublishAction::EmailOnly);
    let mut new = form_to_new(&form, post_type);
    // `status_for` turns an email-only post into Ghost's `sent`, which is
    // where it lands when it actually goes out, not while it waits.
    new.status = match schedule {
        Scheduling::At(_) => "scheduled".to_string(),
        _ => action.status_for(&form.status),
    };
    let json_mode = wants_json(headers);
    match repos::posts::insert(&state.db, &new, &user.id).await {
        Ok(post) => {
            if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::save(state,&post.id,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
            let _ = repos::actions::record(&state.db, post_type, &post.id, &user.id, "added").await;
            fire_post_webhooks(state, post_type, &post, "added", false);
            if post.status == "published" {
                notify_slack_published(state, &post).await;
            }
            if let Scheduling::At(delivery) = schedule {
                let _ = repos::posts::set_delivery(&state.db, &post.id, delivery).await;
            }
            if going_out {
                return match send_post_as_newsletter(state, &post.id).await {
                    Ok(n) => {
                        let message = format!("Published and emailed to {n} subscriber{}.", if n == 1 { "" } else { "s" });
                        if json_mode { editor_saved_json(&state, &post, &message, "ok").await } else { posts_flash("ok", &message) }
                    }
                    Err(e) => {
                        let message = format!("Published, but the newsletter was not sent: {e}");
                        if json_mode {
                            editor_saved_json(&state, &post, &message, "error").await
                        } else {
                            posts_flash("error", &message)
                        }
                    }
                };
            }
            // Straight back into the editor, now that it has somewhere to
            // save to — the same place Ghost leaves you.
            let editor = format!("{}{}/", if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" }, post.id);
            match schedule {
                Scheduling::Undated => {
                    if json_mode {
                        editor_saved_json(&state, &post, UNDATED, "error").await
                    } else {
                        Redirect::to(&with_flash(&editor, "error", UNDATED)).into_response()
                    }
                }
                _ => {
                    if json_mode {
                        let message = match post.status.as_str() {
                            "published" => "Published.",
                            "scheduled" => "Scheduled.",
                            _ => "",
                        };
                        editor_saved_json(&state, &post, message, "ok").await
                    } else {
                        Redirect::to(&editor).into_response()
                    }
                }
            }
        }
        Err(e) => {
            if json_mode {
                editor_error_json(&e.message(), StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                page_u("Error", "", Some(&user), &format!("<div class='banner error'>{}</div>", esc(&e.message())), true).into_response()
            }
        }
    }
}

async fn editor_create(State(state): State<AppState>, headers: HeaderMap, form: EditorForm) -> Response {
    editor_create_impl(&state, &headers, form, "post").await
}

async fn page_create(State(state): State<AppState>, headers: HeaderMap, form: EditorForm) -> Response {
    editor_create_impl(&state, &headers, form, "page").await
}

/// Redirect back to the posts list with a one-shot banner.
fn posts_flash(kind: &str, message: &str) -> Response {
    Redirect::to(&with_flash("/ghost/posts/", kind, message)).into_response()
}

/// A path carrying a one-shot banner for the page it lands on.
fn with_flash(path: &str, kind: &str, message: &str) -> String {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    format!("{path}?{encoded}")
}

// --- sending a post as a newsletter ---

/// What the editor asked for when saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishAction {
    PublishOnly,
    PublishAndEmail,
    /// Ghost's `status = 'sent'`: delivered to subscribers but not listed on
    /// the site.
    EmailOnly,
}

impl PublishAction {
    fn parse(s: &str) -> Self {
        match s {
            "publish_email" => PublishAction::PublishAndEmail,
            "email_only" => PublishAction::EmailOnly,
            _ => PublishAction::PublishOnly,
        }
    }
    fn sends_email(self) -> bool {
        !matches!(self, PublishAction::PublishOnly)
    }
    /// The status a post ends up in. Email-only posts are `sent`, which keeps
    /// them off the site's own listings.
    fn status_for(self, requested: &str) -> String {
        match self {
            PublishAction::EmailOnly => "sent".to_string(),
            _ => requested.to_string(),
        }
    }
}

/// One member a newsletter is going to.
///
/// `status` and `created_at` are carried along because the "Subscription
/// details" box in the email prints them per recipient.
struct Subscriber {
    email: String,
    name: String,
    uuid: String,
    status: String,
    created_at: String,
}

/// Who a post goes to, from its visibility. Ghost derives the recipient
/// filter the same way: a paid-members post is not emailed to free members.
async fn newsletter_recipients(
    pool: &sqlx::SqlitePool,
    newsletter_id: &str,
    visibility: &str,
    post_id: &str,
) -> Vec<Subscriber> {
    let paid_only = matches!(visibility, "paid" | "tiers");
    let base = "SELECT DISTINCT m.email, coalesce(m.name, ''), coalesce(m.uuid, ''), m.status, \
                coalesce(m.created_at, '') FROM members m \
                JOIN members_newsletters mn ON mn.member_id = m.id \
                WHERE mn.newsletter_id = ? AND m.email_disabled = 0 AND NOT EXISTS(SELECT 1 FROM email_suppressions es WHERE es.email=m.email)";
    let sql = if visibility == "tiers" {
        format!("{base} AND m.status IN ('paid', 'comped', 'gift') AND EXISTS(SELECT 1 FROM members_products mp JOIN posts_products pp ON pp.product_id = mp.product_id WHERE mp.member_id = m.id AND pp.post_id = ? AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at) > datetime('now')))")
    } else if paid_only {
        format!("{base} AND m.status IN ('paid', 'comped', 'gift')")
    } else {
        base.to_string()
    };
    let query = sqlx::query_as::<_, (String, String, String, String, String)>(&sql).bind(newsletter_id);
    let query = if visibility == "tiers" { query.bind(post_id) } else { query };
    query.fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(email, name, uuid, status, created_at)| Subscriber {
            email,
            name,
            uuid,
            status,
            created_at,
        })
        .collect()
}

/// The byline a newsletter prints under the post title.
///
/// Ghost's own joining: one or two authors are listed in full, and beyond
/// that it becomes "First & N others" rather than a run-on line.
async fn newsletter_byline(state: &AppState, post_id: &str) -> String {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT u.name FROM posts_authors pa JOIN users u ON u.id = pa.author_id \
         WHERE pa.post_id = ? ORDER BY pa.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    match names.len() {
        0 => String::new(),
        1 | 2 => names.join(" & "),
        n => format!("{} & {} others", names[0], n - 1),
    }
}

/// Cut to `max` characters on a character boundary, with an ellipsis when
/// something was actually removed.
fn truncate_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Tell Slack a post went live, if the app is configured.
///
/// Fire-and-forget by design: a wedged or misconfigured webhook must not fail
/// a publish that has already happened, so a failure is logged and the
/// editor's response is unaffected. Ghost treats it the same way.
async fn notify_slack_published(state: &AppState, post: &ghost_core::models::Post) {
    let Some(cfg) = ghost_api::slack::SlackConfig::from_settings(&state.settings) else {
        return;
    };
    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let router = state.router.read().await.clone();
    let j = ghost_core::models::post_to_json(post, &[], &[], "");
    let path = match router.permalink_for(&j) {
        Some((_, pl)) => ghost_routing::tree::build_permalink(&pl, &j),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let excerpt = post
        .custom_excerpt
        .clone()
        .or_else(|| ghost_core::models::excerpt_from_plaintext(post.plaintext.as_deref(), 30))
        .unwrap_or_default();
    let author = repos::users::find_by_id(&state.db, post.published_by.as_deref().unwrap_or(""))
        .await
        .ok()
        .flatten()
        .and_then(|u| u.name);

    if let Err(e) = ghost_api::slack::notify_published(
        &cfg,
        &site_title,
        post.title.as_deref().unwrap_or("Untitled"),
        &format!("{site_url}{path}"),
        &excerpt,
        author.as_deref(),
    )
    .await
    {
        tracing::warn!("slack: publish notification failed: {}", e.message());
    }
}

/// Which transport carries a newsletter.
///
/// Mailgun is the right answer for a real list — it is built for bulk, and
/// pushing hundreds of messages through a transactional relay is how a sending
/// domain gets blocked. But requiring it outright makes a newsletter
/// untestable without a Mailgun account, and leaves a site with a dozen
/// members no way to mail them at all, so a configured SMTP relay is accepted
/// as a fallback. A local relay such as MailHog or Mailpit then works for
/// testing exactly as you would expect.
enum BulkTransport {
    Mailgun(ghost_mail::MailgunConfig),
    /// The relay from Settings → Email, one message per recipient.
    Smtp,
}

/// Render and send `post` to its subscribers.
///
/// Returns how many messages were accepted.
async fn send_post_as_newsletter(state: &AppState, post_id: &str) -> Result<usize, String> {
    let Ok(Some(post)) = repos::posts::find_by_id(&state.db, post_id).await else {
        return Err("post not found".into());
    };
    if newsletter_already_sent(state, post_id).await {
        return Err("This post has already been emailed.".into());
    }

    // Which newsletter's design and sender identity this goes out with: the
    // one the post names, else the default. Without one there is nothing to
    // send — the design settings live on the newsletter, not in `settings`.
    let newsletter = match post.newsletter_id.as_deref() {
        Some(id) => repos::newsletters::find_by_id(&state.db, id).await.ok().flatten(),
        None => repos::newsletters::default_newsletter(&state.db).await.ok().flatten(),
    };
    let Some(newsletter) = newsletter else {
        return Err("No active newsletter to send on. Create one in Settings → Newsletters.".into());
    };

    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let accent = state.settings.get_str("accent_color").unwrap_or_default();
    let mail_cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let bulk = match ghost_mail::MailgunConfig::from_settings(&state.settings, &mail_cfg) {
        Some(mg) => BulkTransport::Mailgun(mg),
        // Direct is deliberately not accepted here: it delivers by looking up
        // each recipient's MX records, which is both slow per message and the
        // surest way to have a newsletter filed as spam.
        None if mail_cfg.uses_relay() => {
            tracing::info!("newsletter: Mailgun not configured, sending via the SMTP relay");
            BulkTransport::Smtp
        }
        None => {
            return Err("No bulk transport is configured. Set up Mailgun in \
                        Settings → Newsletters, or an SMTP relay in Settings → Email."
                .into())
        }
    };

    let audience=ghost_api::newsletter_audience::load(state,&post.id).await.map_err(|e|e.message())?;
    let mut recipients = newsletter_recipients(&state.db, &newsletter.id, if audience.include_preview {"public"} else {&post.visibility}, &post.id).await;
    if let Some(filter)=audience.filter.as_deref().filter(|s|!s.is_empty()) {
        let (members,_)=repos::members::browse(&state.db,Some(filter),-1,1).await.map_err(|e|e.message())?;
        let emails:std::collections::HashSet<String>=members.into_iter().map(|member|member.email.to_lowercase()).collect();
        recipients.retain(|member|emails.contains(&member.email.to_lowercase()));
    }
    if recipients.is_empty() {
        return Err("No subscribers to send to.".into());
    }

    let router = state.router.read().await.clone();
    let j = ghost_core::models::post_to_json(&post, &[], &[], "");
    let path = match router.permalink_for(&j) {
        Some((_, p)) => ghost_routing::tree::build_permalink(&p, &j),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let post_url = format!("{site_url}{path}");
    let post_title = post.title.clone().unwrap_or_else(|| "Untitled".into());
    // an email has no base to resolve `/content/images/…` against, so the
    // body goes out with full URLs
    let content_html = ghost_core::url::absolutize_html(
        &site_url,
        &post.html.clone().unwrap_or_default(),
    );
    let plaintext = post.plaintext.clone().unwrap_or_default();

    let secret = state
        .settings
        .get_str("members_email_auth_secret")
        .unwrap_or_default();
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    let site_ctx = ghost_mail::newsletter::SiteContext {
        title: title.clone(),
        url: site_url.clone(),
        icon_url: state
            .settings
            .get_str("icon")
            .filter(|i| !i.is_empty())
            .map(|i| ghost_core::url::absolute(&site_url, &ghost_core::url::base_asset(&i))),
        locale: state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
        // Ghost takes this from i18next's `dir(locale)`. There is no
        // catalogue here, so left-to-right it is until one exists.
        direction: "ltr".into(),
        accent_color: accent,
    };

    // `posts_meta` is not attached by `find_by_id`, and three of its columns
    // are visible in the email, so it is fetched directly.
    let (email_subject, feature_image_alt, feature_image_caption) =
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT email_subject, feature_image_alt, feature_image_caption \
             FROM posts_meta WHERE post_id = ?",
        )
        .bind(&post.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or((None, None, None));

    let post_ctx = ghost_mail::newsletter::PostContext {
        id: post.id.clone(),
        title: post_title.clone(),
        url: post_url.clone(),
        html: content_html.clone(),
        plaintext: plaintext.clone(),
        custom_excerpt: post.custom_excerpt.clone(),
        published_at: ghost_core::dates::short_date_in_zone(
            post.published_at.as_deref().unwrap_or(""),
            &timezone,
        ),
        authors: newsletter_byline(state, &post.id).await,
        feature_image: post.feature_image.clone(),
        feature_image_width: None,
        feature_image_alt,
        feature_image_caption,
        email_only: post.status == "sent",
        // Paywalled sends would need per-segment bodies; one body for
        // everyone means nothing is truncated, so there is no paywall card.
        paywall: false,
    };

    // "Keep reading": the three most recent published posts, excluding this
    // one. Permalinks go through the router rather than being assumed to be
    // `/{slug}/`, so a site with a dated or prefixed structure links
    // correctly from the email.
    let mut latest: Vec<ghost_mail::newsletter::LatestPost> = Vec::new();
    if newsletter.show_latest_posts {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE status = 'published' AND type = 'post' AND id != ? \
             ORDER BY published_at DESC LIMIT 3",
        )
        .bind(&post.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for id in ids {
            let Ok(Some(p)) = repos::posts::find_by_id(&state.db, &id).await else {
                continue;
            };
            let pj = ghost_core::models::post_to_json(&p, &[], &[], "");
            let ppath = match router.permalink_for(&pj) {
                Some((_, pl)) => ghost_routing::tree::build_permalink(&pl, &pj),
                None => format!("/{}/", p.slug.clone().unwrap_or_default()),
            };
            latest.push(ghost_mail::newsletter::LatestPost {
                title: p.title.clone().unwrap_or_else(|| "Untitled".into()),
                url: format!("{site_url}{ppath}"),
                feature_image: p.feature_image.clone(),
                excerpt: p
                    .custom_excerpt
                    .clone()
                    .or_else(|| Some(ghost_mail::newsletter::plaintext(&ghost_core::models::public_html(&p))))
                    .map(|e| truncate_chars(&e, 120))
                    .filter(|e| !e.is_empty()),
            });
        }
    }

    let rendered = ghost_mail::newsletter::NewsletterEmail {
        site: &site_ctx,
        newsletter: &newsletter,
        post: &post_ctx,
        latest_posts: &latest,
        comments_enabled: state
            .settings
            .get_str("comments_enabled")
            .map(|v| v != "off")
            .unwrap_or(false),
    };

    let template_html=rendered.render().map_err(|e|e.message())?;
    let template_text=ghost_mail::newsletter::plaintext(&template_html);

    // Ghost lets the editor override the subject without changing the title.
    let subject = email_subject
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| post_title.clone());

    let mut sent = 0usize;
    let mut last_error: Option<String> = None;
    let mut variants:HashMap<String,(String,String)>=HashMap::new();
    for sub in &recipients {
        let tiers=ghost_db::repos::tiers::member_tier_ids(&state.db,&sqlx::query_scalar::<_,String>("SELECT id FROM members WHERE uuid=?").bind(&sub.uuid).fetch_one(&state.db).await.map_err(|e|e.to_string())?).await.map_err(|e|e.message())?;
        let access=ghost_core::comments::check_post_tier_access(&post.visibility,Some(&sub.status),&post.tier_ids,&tiers);
        let variant_key=format!("{}:{access}",sub.status);
        if !variants.contains_key(&variant_key){
            let mut variant=post_ctx.clone();
            let raw=if access{post.html.as_deref().unwrap_or("")}else{post.html.as_deref().unwrap_or("").split_once("<!--members-only-->").map(|(preview,_)|preview).unwrap_or("")};
            variant.html=ghost_core::url::absolutize_html(&site_url,&ghost_core::models::segment_html(raw,Some(&sub.status)));
            variant.plaintext=ghost_mail::newsletter::plaintext(&variant.html);variant.paywall=!access;
            let body=ghost_mail::newsletter::NewsletterEmail{site:&site_ctx,newsletter:&newsletter,post:&variant,latest_posts:&latest,comments_enabled:rendered.comments_enabled}.render().map_err(|e|e.message())?;
            let body=ghost_api::newsletter_events::track_links(state,&post.id,&body).await.map_err(|e|e.message())?;
            variants.insert(variant_key.clone(),(body.clone(),ghost_mail::newsletter::plaintext(&body)));
        }
        let (template_html,template_text)=variants.get(&variant_key).unwrap();
        // Per-recipient unsubscribe link, same HMAC the members API verifies.
        let key = hmac_hex(&secret, &sub.uuid);
        let member = ghost_mail::newsletter::Recipient {
            uuid: sub.uuid.clone(),
            email: sub.email.clone(),
            name: Some(sub.name.clone()).filter(|n| !n.trim().is_empty()),
            status: sub.status.clone(),
            created_at: ghost_core::dates::short_date_in_zone(&sub.created_at, &timezone),
            unsubscribe_url: format!(
                "{site_url}/unsubscribe/?uuid={}&key={key}",
                sub.uuid
            ),
            manage_account_url: format!("{site_url}/#/portal/account"),
            key,
        };
        let msg = ghost_mail::Message {
            to: sub.email.clone(),
            subject: subject.clone(),
            html: ghost_mail::newsletter::personalize(&template_html, &member),
            text: ghost_mail::newsletter::personalize(&template_text, &member),
        };
        let delivered = match &bulk {
            BulkTransport::Mailgun(mg) => ghost_mail::send_mailgun(mg, &msg).await.map(|_| ()),
            BulkTransport::Smtp => ghost_mail::send(&mail_cfg, &msg).await,
        };
        match delivered {
            Ok(()) => sent += 1,
            Err(e) => {
                tracing::warn!("newsletter: {} failed: {}", sub.email, e.message());
                last_error = Some(e.message());
            }
        }
    }

    // Record the send so the post cannot be mailed twice.
    let now = ghost_core::dates::now_db();
    let newsletter_id = Some(newsletter.id.clone());
    let _ = sqlx::query(
        "INSERT INTO emails (id, post_id, status, recipient_filter, email_count, delivered_count,          subject, `from`, html, plaintext, source, submitted_at, newsletter_id, created_at, updated_at)          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'html', ?, ?, ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(post_id)
    .bind(if sent > 0 { "submitted" } else { "failed" })
    .bind(&post.visibility)
    .bind(recipients.len() as i64)
    .bind(sent as i64)
    .bind(&subject)
    // the envelope sender, whichever transport carried it
    .bind(&mail_cfg.from_address)
    // the email as it went out, not the post body — this is the record of
    // what subscribers actually received
    .bind(&template_html)
    .bind(&template_text)
    .bind(&now)
    .bind(newsletter_id)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;

    if sent == 0 {
        return Err(last_error.unwrap_or_else(|| "no messages were accepted".into()));
    }
    Ok(sent)
}

/// HMAC-SHA256 hex, for the unsubscribe key. Mirrors the members API so the
/// links this emits validate there.
fn hmac_hex(key: &str, msg: &str) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = key.as_bytes().to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg.as_bytes());
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    hex::encode(outer.finalize())
}

/// Whether a newsletter has already gone out for this post. The `emails`
/// row is the record of that, and it is unique per post — which is what
/// stops an ordinary edit from mailing subscribers a second time.
async fn newsletter_already_sent(state: &AppState, post_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM emails WHERE post_id = ?")
        .bind(post_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0)
        > 0
}

/// The post's first author, for an audit entry written with nobody signed in.
async fn post_author_id(state: &AppState, post_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM posts_authors WHERE post_id = ? ORDER BY sort_order, id LIMIT 1",
    )
    .bind(post_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

async fn editor_save(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, mut form: EditorForm) -> Response {
    let actor = match require_actor(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(&state, &actor, &id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    // Contributors may write but not publish, so any status they ask for
    // collapses back to draft. Enforced here rather than only in the form:
    // the status field is a plain select anyone can retype.
    if !actor.role.can_publish() {
        form.status = "draft".into();
    }
    let user = actor.user;
    if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::validate(&state,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
    let existing = repos::posts::find_by_id(&state.db, &id).await.ok().flatten();
    let post_type = existing
        .as_ref()
        .map(|p| p.post_type.clone())
        .unwrap_or_else(|| "post".into());
    // Slack hears about a post the moment it goes live, and only then — an
    // edit to something already published is not news.
    let was_published = existing.as_ref().is_some_and(|p| p.status == "published");
    // Kept alongside, because the scheduling webhooks need to know what the
    // status *was*, not only whether it was published.
    let was_status = existing.as_ref().map(|p| p.status.clone()).unwrap_or_default();
    // The tags before this save, so attach/detach can be told apart from a
    // plain edit. Read as ids, which is what the form posts back.
    let tags_before: Vec<String> = sqlx::query_scalar(
        "SELECT tag_id FROM posts_tags WHERE post_id = ? ORDER BY sort_order",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    // Scheduling is settled here as well as at creation: a draft can be
    // scheduled later, and a scheduled post's date and delivery can both be
    // changed right up until it goes out.
    let schedule = scheduling(&form, &post_type, PublishAction::parse(&form.publish_action));
    if matches!(schedule, Scheduling::Undated) {
        form.status = "draft".into();
    }
    // A save with no body in it is a settings-only save; the content columns
    // keep what they already hold rather than being rendered from nothing.
    let content = form.has_content().then(|| form.rendered());
    let (html, plaintext, reading_time) = match &content {
        Some((html, plaintext, reading_time)) => (Some(html.clone()), Some(plaintext.clone()), Some(*reading_time)),
        None => (None, None, None),
    };
    // Editing never sends: a post goes out once, when it is created, or not
    // at all. `publish_action` is deliberately ignored here, so a replayed or
    // hand-crafted request cannot mail subscribers from an edit.
    let changes = repos::posts::PostChanges {
        title: Some(form.title.clone()),
        slug: form.slug.clone(),
        lexical: form.lexical.clone(),
        markdown: if form.is_koenig() { None } else { form.markdown.clone() },
        html,
        plaintext,
        custom_excerpt: form.custom_excerpt.clone(),
        feature_image: form.feature_image.clone(),
        featured: Some(form.featured),
        // A save that arrives without a status or a visibility — a body this
        // parser could not read, or a hand-made one — must not blank the
        // column: no value means "unchanged", not "empty".
        status: (!form.status.is_empty()).then(|| form.status.clone()),
        visibility: form.visibility.clone().filter(|v| !v.is_empty()),
        published_at: form.published_at.clone(),
        codeinjection_head: form.codeinjection_head.clone(),
        codeinjection_foot: form.codeinjection_foot.clone(),
        custom_template: form.custom_template.clone(),
        canonical_url: form.canonical_url.clone(),
        show_title_and_feature_image: Some(form.show_title_and_feature_image),
        meta_title: form.meta_title.clone(),
        meta_description: form.meta_description.clone(),
        og_title: form.og_title.clone(),
        og_description: form.og_description.clone(),
        og_image: form.og_image.clone(),
        twitter_title: form.twitter_title.clone(),
        twitter_description: form.twitter_description.clone(),
        twitter_image: form.twitter_image.clone(),
        email_subject: form.email_subject.clone(),
        feature_image_alt: form.feature_image_alt.clone(),
        feature_image_caption: form.feature_image_caption.clone(),
        reading_time,
        tag_ids: Some(form.tags.clone()),
        tier_ids: if form.tiers_present { Some(form.tiers.clone()) } else { None },
        // Authors are only rewritten when the form offered a choice;
        // emptying the list would otherwise orphan the post.
        author_ids: if form.authors.is_empty() { None } else { Some(form.authors.clone()) },
        clearing: editor_owned_fields(form.is_koenig(), form.has_content()),
    };
    let json_mode = wants_json(&headers);
    match repos::posts::update(&state.db, &id, &changes).await {
        Ok(post) => {
            if form.audience_present {if let Err(e)=ghost_api::newsletter_audience::save(&state,&post.id,&form.audience).await{return editor_error_json(&e.message(),StatusCode::UNPROCESSABLE_ENTITY)}}
            let _ = repos::actions::record(&state.db, &post_type, &post.id, &user.id, "edited").await;
            fire_post_webhooks_from(
                &state,
                &post_type,
                &post,
                "edited",
                was_published,
                &was_status,
                existing.as_ref(),
            );
            fire_tag_link_webhooks(&state, &post_type, &post, &tags_before, &form.tags).await;
            if !was_published && post.status == "published" {
                notify_slack_published(&state, &post).await;
            }
            let list = if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" };
            // What the script's banner should say, if anything: only a change
            // in the post's standing is worth interrupting the writing for.
            let note = |post: &ghost_core::models::Post| match post.status.as_str() {
                "scheduled" => format!(
                    "Scheduled for {} UTC.",
                    to_datetime_local(post.published_at.as_deref().unwrap_or("")).replace('T', " at ")
                ),
                _ if !was_published && post.status == "published" => "Published.".to_string(),
                _ => String::new(),
            };
            match schedule {
                Scheduling::At(delivery) => {
                    let _ = repos::posts::set_delivery(&state.db, &post.id, delivery).await;
                    if json_mode {
                        editor_saved_json(&state, &post, &note(&post), "ok").await
                    } else {
                        Redirect::to(list).into_response()
                    }
                }
                // Anything else means the post is no longer waiting on a
                // date, and a standing instruction to email itself must not
                // outlive the schedule that carried it — the post could be
                // scheduled again later by someone who never saw this form.
                // A post that was *already* emailed keeps its record: that
                // one is in `emails`, and this only clears the intent.
                other => {
                    if existing.as_ref().is_some_and(|p| p.status == "scheduled") {
                        let _ = repos::posts::set_delivery(&state.db, &post.id, Default::default()).await;
                    }
                    match other {
                        // The date was unusable, so this came back as a draft
                        // and the author needs to know before closing the tab.
                        Scheduling::Undated => {
                            if json_mode {
                                editor_saved_json(&state, &post, UNDATED, "error").await
                            } else {
                                Redirect::to(&with_flash(list, "error", UNDATED)).into_response()
                            }
                        }
                        _ => {
                            if json_mode {
                                editor_saved_json(&state, &post, &note(&post), "ok").await
                            } else {
                                Redirect::to(list).into_response()
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            if json_mode {
                editor_error_json(&e.message(), StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                page_u("Error", "", Some(&user), &format!("<div class='banner error'>{}</div>", esc(&e.message())), true).into_response()
            }
        }
    }
}

async fn delete_impl(state: &AppState, headers: &HeaderMap, id: &str, post_type: &str) -> Response {
    let actor = match require_actor(state, headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !may_edit_post(state, &actor, id).await {
        return forbidden_page(&actor, Role::Editor);
    }
    let user = actor.user;
    // Read it before it goes: a delivery that cannot name what was deleted is
    // not much use to a listener.
    let doomed = repos::posts::find_by_id(&state.db, id).await.ok().flatten();
    let _ = repos::posts::destroy(&state.db, id).await;
    let _ = repos::actions::record(&state.db, post_type, id, &user.id, "deleted").await;
    if let Some(p) = doomed {
        fire_post_webhooks(state, post_type, &p, "deleted", p.status == "published");
    }
    Redirect::to(if post_type == "page" { "/ghost/pages/" } else { "/ghost/posts/" }).into_response()
}

async fn post_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    delete_impl(&state, &headers, &id, "post").await
}

async fn page_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    delete_impl(&state, &headers, &id, "page").await
}

// ---------------------------------------------------------------------------
// tags
// ---------------------------------------------------------------------------

/// The visibility filters along the top. `None` is "everything"; internal
/// tags are the `#`-prefixed ones Ghost keeps out of the site's tag lists.
const TAG_FILTERS: &[(&str, &str, Option<&str>)] = &[
    ("all", "All Tags", None),
    ("public", "Public Tags", Some("public")),
    ("internal", "Internal Tags", Some("internal")),
];

async fn tags_list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Editor).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let (mut tags, _) = repos::tags::browse(&state.db, None, None, 100, 1, false, true).await.unwrap_or_default();
    // The tabs count the whole list, then the selected one narrows it, so a
    // tab's number does not change when you click a different tab.
    let mut counts: HashMap<String, i64> = HashMap::new();
    for t in &tags {
        *counts.entry(t.visibility.clone()).or_insert(0) += 1;
    }
    let selected = q.get("filter").map(String::as_str).unwrap_or("all");
    let selected = TAG_FILTERS
        .iter()
        .find(|(key, _, _)| *key == selected)
        .unwrap_or(&TAG_FILTERS[0]);
    if let Some(visibility) = selected.2 {
        tags.retain(|t| t.visibility == visibility);
    }
    let rows: String = tags
        .iter()
        .map(|t| {
            let count = t.count_posts.unwrap_or(0);
            let internal = t.visibility == "internal";
            format!(
                r#"<div class="row" data-filterable>
<span class="ic tint-{tint}">{ic}</span>
<a class="body" href="/ghost/tags/{id}/"><span class="title">{name}</span><span class="detail"><code>/{slug}/</code></span></a>
<div class="trailing">{internal_chip}<span class="chip tonal">{count} {post_word}</span>
<form method="post" action="/ghost/tags/{id}/delete/" data-confirm="Delete tag|&#8220;{name}&#8221; will be removed from every post.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>{chev}</div>
</div>"#,
                tint = if internal { "slate" } else { "amber" },
                ic = svg("tags"),
                name = esc(t.name.as_deref().unwrap_or("")),
                slug = esc(t.slug.as_deref().unwrap_or("")),
                internal_chip = if internal { r#"<span class="chip draft">internal</span>"# } else { "" },
                count = count,
                post_word = if count == 1 { "post" } else { "posts" },
                id = esc(&t.id),
                del = svg("delete"),
                chev = svg("chevron"),
            )
        })
        .collect();
    page_for_role(actor_role, 
        "Tags",
        "tags",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Tags</h1><p class="sub">{count} {tag_word}</p></div></header>
{filters}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/tags/" class="inline-form">
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="name" placeholder=" " required><span>New tag name</span></label>
<button class="btn tonal" type="submit">{add} Add tag</button></form></div>
<div class="section">{section}</div>
<div class="search rise" style="--i:2">{search}<input data-filter type="search" placeholder="Search tags" aria-label="Search tags"></div>
<div class="card rows rise" style="--i:3">{rows}</div>"#,
            filters = seg_filter_bar(TAG_FILTERS, "/ghost/tags/", selected.0, "", &counts),
            section = esc(selected.1),
            count = tags.len(),
            tag_word = if tags.len() == 1 { "tag" } else { "tags" },
            add = svg("add"),
            search = svg("search"),
            rows = rows,
        ),
        true,
    )
    .into_response()
}

/// Ghost's tag editor, field for field: the basics, then the metadata and
/// social cards that `ghost_head` emits on the tag's archive page, then code
/// injection.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TagEditForm {
    name: String,
    slug: String,
    description: String,
    accent_color: String,
    feature_image: String,
    meta_title: String,
    meta_description: String,
    canonical_url: String,
    og_title: String,
    og_description: String,
    og_image: String,
    twitter_title: String,
    twitter_description: String,
    twitter_image: String,
    codeinjection_head: String,
    codeinjection_foot: String,
}

/// An author's public archive URL, resolved through the router so it follows
/// routes.yaml (`/author/{slug}/` by default) instead of assuming a path.
async fn author_archive_url(state: &AppState, user: &ghost_core::models::User) -> String {
    let router = state.router.read().await.clone();
    let j = ghost_core::models::user_public_json(user, None);
    match router.permalink_for(&j) {
        Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, &j),
        None => format!("/author/{}/", user.slug.clone().unwrap_or_default()),
    }
}

/// The tag's public archive URL, resolved through the same router the
/// frontend uses so it always matches routes.yaml.
async fn tag_archive_url(state: &AppState, tag: &ghost_core::models::Tag) -> Option<String> {
    let router = state.router.read().await.clone();
    let j = ghost_core::models::tag_to_json(tag, None);
    let (_, permalink) = router.permalink_for(&j)?;
    Some(ghost_routing::tree::build_permalink(&permalink, &j))
}

fn tag_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/tags/{id}/?{encoded}")).into_response()
}

async fn tag_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Editor).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(tag)) = repos::tags::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That tag could not be found.");
    };
    let v = |s: Option<&str>| esc(s.unwrap_or(""));
    let internal = tag.visibility == "internal";
    let slug = tag.slug.clone().unwrap_or_default();
    let accent = tag.accent_color.clone().unwrap_or_default();

    page_for_role(actor_role, 
        "Tag",
        "tags",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/tags/" title="Back to tags">{back}</a>
<div><h1>{name}</h1><p class="sub">{sub}</p></div>
<div class="actions"><button class="btn" type="submit" form="tag-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/tags/{id}/" id="tag-form">
<div class="section">Basics</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{ic}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " required><span>Name</span></label>
<label class="field" style="margin:0"><input name="slug" value="{slug}" placeholder=" "><span>Slug</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="description" rows="2" placeholder=" " maxlength="500">{description}</textarea><span>Description · max 500 characters</span></label>
<div class="grid2" style="gap:12px;margin-top:12px">
<label class="field" style="margin:0"><input name="accent_color" value="{accent}" placeholder=" " pattern="^#?[0-9A-Fa-f]{{3,8}}$"><span>Colour · hex, e.g. #ff1a75</span></label>
<label class="field" style="margin:0"><input name="feature_image" value="{feature_image}" placeholder=" "><span>Tag image URL</span></label>
</div>
<p class="muted" style="margin:10px 0 0">A name starting with <code>#</code> makes the tag internal: it stays off the site and out of search, but still groups posts. {internal_note}</p>
</div></div>
</div>
<div class="section">Metadata</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{notes}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="meta_title" value="{meta_title}" placeholder=" "><span>Meta title</span></label>
<label class="field" style="margin:0"><input name="canonical_url" value="{canonical_url}" placeholder=" "><span>Canonical URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="meta_description" rows="2" placeholder=" " maxlength="500">{meta_description}</textarea><span>Meta description</span></label>
<p class="muted" style="margin:10px 0 0">Used by <code>{{{{ghost_head}}}}</code> on <code>/{slug}/</code>. Blank falls back to the tag name and description.</p>
</div></div>
</div>
<div class="section">Social cards</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<span class="title" style="display:block;margin-bottom:8px">Facebook / Open Graph</span>
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="og_title" value="{og_title}" placeholder=" "><span>Title</span></label>
<label class="field" style="margin:0"><input name="og_image" value="{og_image}" placeholder=" "><span>Image URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="og_description" rows="2" placeholder=" ">{og_description}</textarea><span>Description</span></label>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<span class="title" style="display:block;margin-bottom:8px">X (Twitter)</span>
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="twitter_title" value="{twitter_title}" placeholder=" "><span>Title</span></label>
<label class="field" style="margin:0"><input name="twitter_image" value="{twitter_image}" placeholder=" "><span>Image URL</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="twitter_description" rows="2" placeholder=" ">{twitter_description}</textarea><span>Description</span></label>
</div></div>
</div>
<div class="section">Code injection</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{code}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0 0 12px"><textarea name="codeinjection_head" class="code" rows="4" placeholder=" ">{cih}</textarea><span>Head · injected on this tag's archive page</span></label>
<label class="field" style="margin:0"><textarea name="codeinjection_foot" class="code" rows="4" placeholder=" ">{cif}</textarea><span>Foot</span></label>
</div></div>
</div>
</form>
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:5">
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Delete this tag</span><span class="detail">It is removed from every post; the posts themselves are kept.</span></div>
<div class="trailing">
<form method="post" action="/ghost/tags/{id}/delete/" data-confirm="Delete tag|&#8220;{name}&#8221; will be removed from every post.">
<button class="btn tonal small danger" type="submit">Delete</button></form></div></div>
</div>
<p class="muted" style="margin:14px 0 0 16px">{public_link}</p>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&id),
            ic = svg("tags"),
            notes = svg("notes"),
            link = svg("link"),
            code = svg("code"),
            del = svg("delete"),
            name = v(tag.name.as_deref()),
            name_v = v(tag.name.as_deref()),
            sub = if internal { "Internal tag".to_string() } else { format!("{} posts", tag.count_posts.unwrap_or(0)) },
            slug = esc(&slug),
            description = v(tag.description.as_deref()),
            accent = esc(&accent),
            feature_image = v(tag.feature_image.as_deref()),
            internal_note = if internal { "This tag is internal." } else { "" },
            meta_title = v(tag.meta_title.as_deref()),
            meta_description = v(tag.meta_description.as_deref()),
            canonical_url = v(tag.canonical_url.as_deref()),
            og_title = v(tag.og_title.as_deref()),
            og_description = v(tag.og_description.as_deref()),
            og_image = v(tag.og_image.as_deref()),
            twitter_title = v(tag.twitter_title.as_deref()),
            twitter_description = v(tag.twitter_description.as_deref()),
            twitter_image = v(tag.twitter_image.as_deref()),
            cih = v(tag.codeinjection_head.as_deref()),
            cif = v(tag.codeinjection_foot.as_deref()),
            public_link = if internal {
                "Internal tags have no public archive page.".to_string()
            } else {
                // Ask the router rather than assuming `/{slug}/`: the archive
                // path comes from routes.yaml, which defaults to /tag/{slug}/
                // and can be changed.
                match tag_archive_url(&state, &tag).await {
                    Some(u) => format!(
                        r#"Public archive: <a href="{u}" target="_blank" rel="noopener">{u}</a>"#,
                        u = esc(&u)
                    ),
                    None => "This tag has no public archive page.".to_string(),
                }
            },
        ),
        true,
    )
    .into_response()
}

async fn tag_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<TagEditForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    let name = form.name.trim();
    if name.is_empty() {
        return tag_flash(&id, "error", "A tag needs a name.");
    }
    // Ghost accepts a colour with or without the leading hash; store it the
    // way themes expect to read it.
    let accent = match form.accent_color.trim().trim_start_matches('#') {
        "" => String::new(),
        hex => format!("#{hex}"),
    };
    let t = |s: &str| Some(s.trim().to_string());
    let changes = repos::tags::TagChanges {
        name: Some(name.to_string()),
        slug: t(&form.slug),
        description: t(&form.description),
        feature_image: t(&form.feature_image),
        accent_color: Some(accent),
        meta_title: t(&form.meta_title),
        meta_description: t(&form.meta_description),
        canonical_url: t(&form.canonical_url),
        og_title: t(&form.og_title),
        og_description: t(&form.og_description),
        og_image: t(&form.og_image),
        twitter_title: t(&form.twitter_title),
        twitter_description: t(&form.twitter_description),
        twitter_image: t(&form.twitter_image),
        // code injection is markup — keep it byte for byte
        codeinjection_head: Some(form.codeinjection_head.clone()),
        codeinjection_foot: Some(form.codeinjection_foot.clone()),
        visibility: None,
    };
    match repos::tags::update(&state.db, &id, &changes).await {
        Ok(tag) => {
            fire_tag_webhook(&state, "tag.edited", &tag);
            tag_flash(&id, "ok", "Tag saved.")
        }
        Err(e) => tag_flash(&id, "error", &e.message()),
    }
}

#[derive(Deserialize)]
struct TagForm {
    name: String,
}

async fn tag_create(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<TagForm>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    if let Ok(tag) = repos::tags::insert(
        &state.db,
        &repos::tags::NewTag {
            name: form.name,
            ..Default::default()
        },
    )
    .await
    {
        fire_tag_webhook(&state, "tag.added", &tag);
    }
    Redirect::to("/ghost/tags/").into_response()
}

async fn tag_delete(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Editor).await {
        return r;
    }
    // Read it first: once the row is gone there is nothing to describe, and
    // a delete webhook that carries only an id is close to useless.
    let doomed = repos::tags::find_by_id(&state.db, &id).await.ok().flatten();
    let destroyed = repos::tags::destroy(&state.db, &id).await.is_ok();
    if let (true, Some(tag)) = (destroyed, doomed) {
        fire_tag_webhook(&state, "tag.deleted", &tag);
    }
    Redirect::to("/ghost/tags/").into_response()
}

// ---------------------------------------------------------------------------
// themes
// ---------------------------------------------------------------------------

/// Redirect back to the themes list carrying a one-shot banner.
fn themes_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/themes/?{encoded}")).into_response()
}

async fn themes_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
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

            format!(
                r#"<div class="row" data-filterable>
<span class="ic tint-purple">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{meta}</span></div>
<div class="trailing">{status}{customise}{edit}{activate}{delete}</div>
</div>"#,
                ic = svg("themes"),
                name = esc(&t.name),
                meta = meta,
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
<div class="section">Official themes</div>
<p class="muted" style="margin:-6px 0 14px">Free themes built by Ghost. Installing downloads the latest version from GitHub; it does not change the active theme.</p>
<div class="gallery rise" style="--i:4">{gallery}</div>"#,
            banner = banner,
            up = svg("upload"),
            rows = rows,
            gallery = gallery
        ),
        true,
    )
    .into_response()
}

/// Ghost's official theme gallery, as cards: a live preview of the demo site,
/// and one button whose job depends on whether the theme is already here.
///
/// `installed` is the themes already under `content/themes/`, so a gallery
/// entry that is installed offers Activate (or reads Active) instead of
/// downloading it a second time.
fn official_theme_gallery(installed: &[ghost_api::themes::InstalledTheme]) -> String {
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
<a class="shot" href="{preview}" target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true"><img src="/ghost/themes/official/{slug}/thumbnail.png" alt="" width="600" height="750" loading="lazy" decoding="async" onerror="this.remove()"></a>
<div class="meta"><div class="body"><span class="title">{name}</span><span class="detail">{category}</span></div></div>
<div class="actions"><a class="btn outlined small" href="{preview}" target="_blank" rel="noopener noreferrer">Preview</a>{action}</div>
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
fn upload_error_message(err: &axum::extract::multipart::MultipartError) -> String {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        format!(
            "That file is larger than the {} MB upload limit.",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )
    } else {
        format!("Upload failed: {err}")
    }
}

async fn theme_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
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
async fn theme_official_thumbnail(State(state): State<AppState>, Path(slug): Path<String>, headers: HeaderMap) -> Response {
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
async fn theme_install_official(State(state): State<AppState>, Path(slug): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::install_official(&state, &slug).await {
        Ok(name) => themes_flash("ok", &format!("Theme '{name}' installed. Activate it when you're ready.")),
        Err(e) => themes_flash("error", &format!("Could not install that theme: {}", e.message())),
    }
}

// --- theme code editor -------------------------------------------------------

fn theme_edit_flash(theme: &str, file: &str, kind: &str, message: &str) -> Response {
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
struct FileTree {
    /// Sub-directories by name, in path order.
    dirs: Vec<(String, FileTree)>,
    /// `(relative path, leaf name)` for the files directly in this directory.
    files: Vec<(String, String)>,
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

    fn build(paths: &[String]) -> Self {
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
fn render_file_tree(tree: &FileTree, theme: &str, open: &str, prefix: &str, depth: usize) -> String {
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
async fn theme_edit_page(
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
struct ThemeFileForm {
    file: String,
    contents: String,
}

async fn theme_file_save(
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
async fn reload_if_active(state: &AppState, name: &str) -> Option<String> {
    if ghost_api::themes::active_theme_name(state) != name {
        return None;
    }
    ghost_api::themes::activate(state, name).await.err().map(|e| e.message())
}

async fn theme_file_new(
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

async fn theme_file_delete(
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

async fn theme_activate(State(state): State<AppState>, Path(name): Path<String>, headers: HeaderMap) -> Response {
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

async fn theme_delete(State(state): State<AppState>, Path(name): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match ghost_api::themes::delete(&state, &name).await {
        Ok(()) => themes_flash("ok", &format!("Theme '{name}' deleted.")),
        Err(e) => themes_flash("error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// design (Ghost-style: site-wide brand settings + the active theme's
// declared custom settings, stored under custom_theme_settings and read by
// templates as @custom)
// ---------------------------------------------------------------------------

/// One `config.custom` declaration from the active theme's package.json.
struct CustomField {
    key: String,
    /// `select`, `color`, `boolean`, `text`, `image`, `collection`, …
    ftype: String,
    group: String,
    description: String,
    options: Vec<String>,
}

impl CustomField {
    /// `site_background_color` → `Site background color` (how Ghost labels
    /// the field in the Customize panel).
    fn label(&self) -> String {
        let mut s = self.key.replace('_', " ");
        if let Some(first) = s.get_mut(0..1) {
            let upper = first.to_uppercase();
            s.replace_range(0..1, &upper);
        }
        s
    }
}

fn custom_fields(decls: &serde_json::Map<String, serde_json::Value>) -> Vec<CustomField> {
    decls
        .iter()
        .map(|(key, def)| CustomField {
            key: key.clone(),
            ftype: def.get("type").and_then(|t| t.as_str()).unwrap_or("text").to_string(),
            group: def.get("group").and_then(|g| g.as_str()).unwrap_or("").to_string(),
            description: def.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            options: def
                .get("options")
                .and_then(|o| o.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// Coerce a posted form into override values for the declared fields,
/// mirroring Ghost's validation: only declared keys are addressable, selects
/// must be one of the theme's options, and an unticked boolean is `false`
/// (marked by the hidden `_<key>` companion input).
fn coerce_custom_values(
    decls: &serde_json::Map<String, serde_json::Value>,
    form: &HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    let defaults = ghost_render::ThemeState::custom_settings_defaults(decls);
    let mut out = serde_json::Map::new();
    for f in custom_fields(decls) {
        let value = match f.ftype.as_str() {
            "boolean" => serde_json::Value::Bool(
                form.contains_key(&f.key) && form.get(&f.key).map(String::as_str) == Some("1"),
            ),
            "select" => match form.get(&f.key) {
                Some(v) if f.options.iter().any(|o| o == v) => serde_json::Value::String(v.clone()),
                // an off-list select falls back to the theme's default
                _ => defaults.get(&f.key).cloned().unwrap_or(serde_json::Value::Null),
            },
            // text, image, color and anything else Ghost-style is a string
            _ => serde_json::Value::String(form.get(&f.key).cloned().unwrap_or_default()),
        };
        out.insert(f.key, value);
    }
    out
}

/// Redirect back to the design page carrying a one-shot banner.
fn design_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/design/?{encoded}")).into_response()
}

fn design_field_row(f: &CustomField, value: &serde_json::Value) -> String {
    let title = esc(&f.label());
    let desc = if f.description.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="detail">{}</span>"#, esc(&f.description))
    };
    let body = format!(r#"<span class="title">{title}</span>{desc}"#);
    match f.ftype.as_str() {
        "boolean" => format!(
            r#"<div class="row"><div class="body">{body}</div>
<div class="trailing"><input type="hidden" name="_{key}" value="1">
<label class="switch"><input type="checkbox" name="{key}" value="1" {checked}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            key = esc(&f.key),
            checked = if value.as_bool().unwrap_or(false) { "checked" } else { "" },
            body = body,
        ),
        "color" => {
            let v = value.as_str().unwrap_or("");
            format!(
                r#"<div class="row"><div class="body">{body}</div>
<div class="trailing color-field"><input type="color" name="{key}" value="{v}" aria-label="{title}"><code>{v}</code></div></div>"#,
                key = esc(&f.key),
                v = esc(v),
                title = title,
                body = body,
            )
        }
        "select" => {
            // A theme's options are plain strings, so each is its own label.
            let options: Vec<(&str, &str)> =
                f.options.iter().map(|o| (o.as_str(), o.as_str())).collect();
            format!(
                r#"<div class="row"><div class="body">{body}</div>
<div class="trailing">{control}</div></div>"#,
                control = selbox(&f.key, &f.label(), &options, value.as_str().unwrap_or("")),
                body = body,
            )
        }
        "image" => {
            let v = value.as_str().unwrap_or("");
            let preview = if v.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<img src="{v}" alt="" style="height:44px;width:88px;object-fit:cover;border-radius:10px;box-shadow:var(--shadow-1)">"#,
                    v = esc(v)
                )
            };
            format!(
                r#"<div class="row" style="align-items:flex-start"><div class="body" style="width:100%">
<div style="display:flex;gap:14px;align-items:center">{preview}
<label class="field" style="margin:0;flex:1"><input data-theme-image name="{key}" value="{v}" placeholder=" "><span>{title} · URL</span></label></div></div></div>"#,
                key = esc(&f.key),
                v = esc(v),
                title = title,
                preview = preview,
            )
        }
        // collection is not editable here (Ghost manages it through a
        // dedicated UI); everything else is a plain text field
        "collection" => format!(
            r#"<div class="row"><div class="body">{body}</div><div class="trailing"><span class="chip tonal">not editable</span></div></div>"#,
            body = body,
        ),
        _ => format!(
            r#"<div class="row"><div class="body">{body}</div>
<div class="trailing" style="flex:1;max-width:260px">{control}</div></div>"#,
            control = textbox(&f.key, &f.label(), value.as_str().unwrap_or("")),
            body = body,
        ),
    }
}

fn brand_image_field(key: &str, label: &str, value: &str) -> String {
    let preview_size = match key {
        "icon" => "max-width:48px;max-height:48px;",
        "logo" => "max-width:min(160px,100%);max-height:48px;",
        _ => "max-width:100%;max-height:120px;",
    };
    format!(
        r#"<div class="row" style="align-items:flex-start" data-brand-image data-upload-url="{upload_url}">
<span class="ic tint-pink">{image}</span>
<div class="body" style="min-width:0;width:100%">
<img alt="{label} preview" hidden style="{preview_size}width:auto;object-fit:contain;border-radius:8px;margin-bottom:12px">
<label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" " data-image-url><span>{label} · URL</span></label>
<div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:12px">
<input type="file" accept="image/jpeg,image/png,image/gif,image/webp,image/svg+xml,image/avif" aria-label="Upload {label}" hidden>
<button class="btn tonal small" type="button" data-image-upload>{upload} Upload image</button>
{unsplash}
<button class="btn text small" type="button" data-image-remove hidden>Remove</button></div>
<p class="muted" role="status" aria-live="polite" style="margin:8px 0 0"></p>
</div></div>"#,
        upload_url = esc(&ghost_core::url::base("/ghost/api/admin/images/upload/")),
        image = svg("image"),
        upload = svg("upload"),
        key = esc(key),
        label = esc(label),
        value = esc(&ghost_core::url::base_asset(value)),
        unsplash = if key == "cover_image" {
            format!(r#"<button class="btn tonal small" type="button" data-image-unsplash>{} Unsplash</button>"#, svg("search"))
        } else { String::new() },
    )
}

async fn design_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let theme = state.theme.read().await.clone();
    let banner = match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    };

    // effective values: theme defaults with the owner's overrides on top
    let overrides = state
        .settings
        .get("custom_theme_settings")
        .and_then(|v| v.get(&theme.name).cloned());
    let effective = theme.custom_settings(overrides.as_ref());

    let fields = custom_fields(&theme.pkg.config.custom);
    // Ghost's design page shows a theme's custom settings on three shelves —
    // Site-wide, Homepage, Post — the convention Casper established with its
    // `group` declarations. A field's declared group picks its shelf;
    // anything ungrouped or novel lands on Site-wide rather than inventing a
    // fourth shelf mid-page.
    let mut shelves: [(&str, Vec<String>); 3] =
        [("Site-wide", vec![]), ("Homepage", vec![]), ("Post", vec![])];
    for f in &fields {
        let group = f.group.to_lowercase();
        let shelf = if group.contains("home") {
            &mut shelves[1]
        } else if group.contains("post") {
            &mut shelves[2]
        } else {
            &mut shelves[0]
        };
        shelf.1.push(f.key.clone());
    }
    let custom_html: String = if fields.is_empty() {
        format!(
            r#"<div class="card padded rise" style="--i:3"><div style="display:flex;gap:14px;align-items:center"><span class="ic tint-purple">{ic}</span>
<p class="muted" style="margin:0">The <b>{name}</b> theme declares no customizable settings.</p></div></div>"#,
            ic = svg("tune"),
            name = esc(&theme.name),
        )
    } else {
        shelves
            .iter()
            .enumerate()
            .filter(|(_, shelf)| !shelf.1.is_empty())
            .map(|(i, (label, keys))| {
                let rows: String = keys
                    .iter()
                    .filter_map(|k| {
                        fields
                            .iter()
                            .find(|f| &f.key == k)
                            .map(|f| design_field_row(f, effective.get(k).unwrap_or(&serde_json::Value::Null)))
                    })
                    .collect();
                format!(
                    r#"<div class="section">{section}</div><div class="card rows rise" style="--i:{i}">{rows}</div>"#,
                    section = esc(label),
                    i = i + 3,
                    rows = rows,
                )
            })
            .collect()
    };

    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    page_for_role(actor_role,
        "Design & branding",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Design &amp; branding</h1><p class="sub">{name} theme</p></div>
<div class="actions"><button class="btn text danger" type="submit" form="reset-form">Reset to defaults</button>
<button class="btn" type="submit" form="design-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/design/" id="design-form" data-unsplash="{unsplash_headers}">
<div class="section">Brand</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-pink">{palette}</span>
<div class="body"><div class="color-field">
<input type="color" name="accent_color" value="{accent}" aria-label="Accent color">
<div><span class="title" style="display:block">Accent color</span><span class="detail">{accent}</span></div>
</div></div></div>
{icon_field}
{logo_field}
{cover_field}
<div class="row" style="align-items:center"><span class="ic tint-pink">{type_ic}</span>
<div class="body"><span class="title">Typography</span><span class="detail">Fonts used across the site</span></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px"><select name="heading_font">{heading_opts}</select><span>Heading font</span>{car}</label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{notes}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px"><select name="body_font">{body_opts}</select><span>Body font</span>{car}</label></div></div>
</div>
{custom_html}
</form>
<form method="post" action="/ghost/design/reset/" id="reset-form" data-confirm="Reset theme settings|Custom settings for {name} go back to the theme's defaults."></form>
<script src="{assets}/unsplash-picker.js" defer></script>"#,
            name = esc(&theme.name),
            assets = ASSETS,
            unsplash_headers = esc(&unsplash_headers(&state).to_string()),
            back = svg("chevron"),
            check = svg("check"),
            title_ic = svg("title"),
            palette = svg("themes"),
            type_ic = svg("tune"),
            notes = svg("notes"),
            car = SELECT_CARET,
            logo_field = brand_image_field("logo", "Publication logo", &state.settings.get_str("logo").unwrap_or_default()),
            icon_field = brand_image_field("icon", "Publication icon", &state.settings.get_str("icon").unwrap_or_default()),
            cover_field = brand_image_field("cover_image", "Publication cover", &state.settings.get_str("cover_image").unwrap_or_default()),
            accent = s("accent_color"),
            heading_opts = select_options(FONT_STACKS, &state.settings.get_str("heading_font").unwrap_or_default()),
            body_opts = select_options(FONT_STACKS, &state.settings.get_str("body_font").unwrap_or_default()),
            banner = banner,
            custom_html = custom_html,
        ),
        true,
    )
    .into_response()
}

/// The font choices the Brand card offers, when the theme declares none of
/// its own. The stored value is the CSS `font-family` stack itself, so a
/// theme can drop `@site.heading_font` straight into its stylesheet, and the
/// empty string is Ghost's seeded default — "let the theme decide".
const FONT_STACKS: &[(&str, &str)] = &[
    ("", "Theme default"),
    ("-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif", "Modern sans-serif"),
    ("'Helvetica Neue', Helvetica, Arial, sans-serif", "Clean sans-serif"),
    ("Georgia, 'Times New Roman', Times, serif", "Classic serif"),
    ("'Iowan Old Style', 'Palatino Linotype', 'Book Antiqua', Palatino, serif", "Elegant serif"),
    ("'SF Mono', 'Cascadia Code', Consolas, 'Courier New', monospace", "Monospace"),
];

async fn design_preview(State(state): State<AppState>, headers: HeaderMap, axum::Json(form): axum::Json<HashMap<String,String>>) -> Response {
    if let Err(r)=require_role(&state,&headers,Role::Administrator).await{return r}
    let theme=state.theme.read().await.clone();
    let mut values=Vec::new();
    for key in ["logo","icon","cover_image","accent_color","heading_font","body_font"] {
        if let Some(value)=form.get(key){values.push((key.into(),serde_json::json!(value)));}
    }
    let mut custom=state.settings.get("custom_theme_settings").and_then(|value|value.as_object().cloned()).unwrap_or_default();
    custom.insert(theme.name.clone(),serde_json::Value::Object(coerce_custom_values(&theme.pkg.config.custom,&form)));
    values.push(("custom_theme_settings".into(),serde_json::Value::Object(custom)));
    values.push(("web_analytics".into(),serde_json::json!(false)));
    values.push(("is_private".into(),serde_json::json!(false)));
    let mut preview=state.clone();preview.settings=std::sync::Arc::new(state.settings.with_overrides(values));
    use tower::ServiceExt;
    match ghost_front::site_router(preview).oneshot(axum::http::Request::builder().uri("/").body(axum::body::Body::empty()).unwrap()).await {
        Ok(mut response)=>{response.headers_mut().insert(header::CACHE_CONTROL,"private, no-store".parse().unwrap());response},
        Err(_)=>(StatusCode::INTERNAL_SERVER_ERROR,"Could not render preview").into_response()
    }
}

async fn design_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let theme = state.theme.read().await.clone();

    for key in ["logo", "icon", "cover_image", "accent_color", "heading_font", "body_font"] {
        let v = form.get(key).cloned().unwrap_or_default();
        let _ = state.settings.set(&state.db, key, &serde_json::json!(v)).await;
    }

    let values = coerce_custom_values(&theme.pkg.config.custom, &form);
    match state
        .settings
        .set(&state.db, "custom_theme_settings", &{
            let mut all = match state.settings.get("custom_theme_settings") {
                Some(serde_json::Value::Object(m)) => m,
                _ => serde_json::Map::new(),
            };
            all.insert(theme.name.clone(), serde_json::Value::Object(values));
            serde_json::Value::Object(all)
        })
        .await
    {
        Ok(()) => design_flash("ok", "Design settings saved."),
        Err(e) => design_flash("error", &e.message()),
    }
}

async fn design_reset(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let theme = state.theme.read().await.clone();
    let mut all = match state.settings.get("custom_theme_settings") {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    all.remove(&theme.name);
    match state
        .settings
        .set(&state.db, "custom_theme_settings", &serde_json::Value::Object(all))
        .await
    {
        Ok(()) => design_flash("ok", "Theme settings reset to defaults."),
        Err(e) => design_flash("error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// settings hub (Ghost's settings navigation. Every group has a page; the
// only deferred items are Growth → Network and Growth → Ghost Explore.)
// ---------------------------------------------------------------------------

/// One group in the settings navigation, in Ghost's sidebar order.
struct SettingsGroup {
    slug: &'static str,
    title: &'static str,
    icon: &'static str,
    tint: &'static str,
    /// Sub-pages inside the group, in Ghost's order (shown as the row detail).
    items: &'static [&'static str],
}

impl SettingsGroup {
    /// Where this group's row points.
    ///
    /// Most groups are a page under `/ghost/settings/`. Design and Navigation
    /// keep the top-level pages they already had and are linked from the hub
    /// rather than copied under it, so their URLs — and any bookmark or link
    /// into them — keep working.
    fn path(&self) -> String {
        match self.slug {
            "design" => "/ghost/design/".to_string(),
            "navigation" => "/ghost/navigation/".to_string(),
            other => format!("/ghost/settings/{other}/"),
        }
    }
}

const SETTINGS_GROUPS: &[SettingsGroup] = &[
    SettingsGroup { slug: "general", title: "General settings", icon: "settings", tint: "pink", items: &["Name, title & description", "Timezone", "Publication language", "Meta data", "Social accounts"] },
    SettingsGroup { slug: "staff", title: "Staff", icon: "ghost", tint: "teal", items: &["People", "Roles", "Invitations"] },
    SettingsGroup { slug: "analytics", title: "Analytics", icon: "chart", tint: "green", items: &["Web analytics", "Email opens", "Email clicks", "Member sources", "Outbound link tagging"] },
    SettingsGroup { slug: "design", title: "Design & branding", icon: "tune", tint: "purple", items: &["Brand", "Site-wide colors", "Theme settings"] },
    SettingsGroup { slug: "navigation", title: "Navigation", icon: "menu", tint: "red", items: &["Primary menu", "Secondary menu"] },
    SettingsGroup { slug: "code-injection", title: "Code injection", icon: "code", tint: "purple", items: &["Site head", "Site foot"] },
    SettingsGroup { slug: "site", title: "Announcement", icon: "flag", tint: "amber", items: &["Announcement bar", "Background", "Visibility"] },
    SettingsGroup { slug: "membership", title: "Membership", icon: "ghost", tint: "teal", items: &["Access", "Spam filters", "Tiers", "Signup portal", "Gift subscriptions", "Tips & donations", "Custom fields"] },
    SettingsGroup { slug: "email", title: "Email", icon: "mail", tint: "blue", items: &["Transactional email", "SMTP server", "Sender identity", "Send a test"] },
    SettingsGroup { slug: "newsletters", title: "Newsletters", icon: "send", tint: "purple", items: &["Newsletter sending", "Default recipients", "Mailgun", "Region"] },
    SettingsGroup { slug: "growth", title: "Growth", icon: "globe", tint: "slate", items: &["Network", "Ghost Explore", "Recommendations", "Signup forms", "Offers"] },
    SettingsGroup { slug: "features", title: "Features", icon: "tune", tint: "teal", items: &["Comments", "Automations", "Beta features", "Private features"] },
    SettingsGroup { slug: "integrations", title: "Integrations", icon: "integrations", tint: "red", items: &["Built-in apps", "Custom integrations", "Content API"] },
    SettingsGroup { slug: "migration", title: "Migration tools", icon: "import", tint: "blue", items: &["Import content", "Export all data", "Members", "Post analytics", "Themes"] },
    SettingsGroup { slug: "advanced", title: "Advanced", icon: "code", tint: "red", items: &["Redirects & routes"] },
    SettingsGroup { slug: "history", title: "History", icon: "history", tint: "slate", items: &["Site activity"] },
    SettingsGroup { slug: "danger", title: "Danger zone", icon: "delete", tint: "red", items: &["Delete all content", "Delete site"] },
];

/// The individual settings behind the hub's search: one row per field, with
/// the section it sits in and the page it lives on. The groups above cover
/// what Ghost's own navigation names; this covers what a user actually types
/// — "port", "webhook", "reply-to" — which none of the page titles contain.
/// The social accounts are folded in from `SOCIAL_ACCOUNTS` at render time
/// rather than repeated here.
const SETTINGS_INDEX: &[(&str, &str, &str)] = &[
    // (page slug, section, field)
    ("general", "Name, title & description", "Name"),
    ("general", "Name, title & description", "Title"),
    ("general", "Name, title & description", "Description"),
    ("general", "Timezone", "Site timezone"),
    ("general", "Publication language", "Publication language"),
    // The section names stay in the haystack: "X card" and "Facebook card"
    // are what these are called, and they are the headings on the page.
    ("general", "Meta data · search", "Meta title"),
    ("general", "Meta data · search", "Meta description"),
    ("general", "Meta data · search", "Structured data for LLMs"),
    ("general", "Meta data · X card", "X title"),
    ("general", "Meta data · X card", "X description"),
    ("general", "Meta data · X card", "X image"),
    ("general", "Meta data · Facebook card", "Facebook title"),
    ("general", "Meta data · Facebook card", "Facebook description"),
    ("general", "Meta data · Facebook card", "Facebook image"),
    ("analytics", "Analytics", "Web analytics"),
    ("analytics", "Analytics", "Email opens"),
    ("analytics", "Analytics", "Email clicks"),
    ("analytics", "Analytics", "Member sources"),
    ("analytics", "Analytics", "Outbound link tagging"),
    ("site", "Announcement bar", "Announcement"),
    ("site", "Announcement bar", "Background"),
    ("site", "Announcement bar", "Visibility"),
    ("design", "Brand", "Accent color"),
    ("design", "Brand", "Publication icon"),
    ("design", "Brand", "Publication logo"),
    ("design", "Brand", "Publication cover"),
    ("navigation", "Primary menu", "Primary navigation"),
    ("navigation", "Secondary menu", "Secondary navigation"),
    ("email", "Transactional email", "Transport"),
    ("email", "SMTP server", "Host"),
    ("email", "SMTP server", "Port"),
    ("email", "SMTP server", "Encryption"),
    ("email", "SMTP server", "Username"),
    ("email", "SMTP server", "Password"),
    ("email", "Sender identity", "From address"),
    ("email", "Sender identity", "From name"),
    ("email", "Sender identity", "Reply-to"),
    ("email", "Send a test", "Send to"),
    ("newsletters", "Newsletters", "Newsletter name"),
    ("newsletters", "Mailgun", "API key"),
    ("newsletters", "Mailgun", "Sending domain"),
    ("newsletters", "Mailgun", "Region"),
    ("newsletters", "Sender", "Sender name"),
    ("newsletters", "Sender", "Sender email"),
    ("newsletters", "Header", "Header image URL"),
    ("membership", "Stripe", "Publishable key"),
    ("membership", "Stripe", "Secret key"),
    ("membership", "Stripe", "Webhook signing secret"),
    ("membership", "Access", "Signup access"),
    ("membership", "Access", "Default post access"),
    ("membership", "Access", "Commenting"),
    ("membership", "Tiers", "Monthly price"),
    ("membership", "Tiers", "Yearly price"),
    ("membership", "Tiers", "Currency"),
    ("membership", "Tiers", "Free trial"),
    ("membership", "Tiers", "Benefits"),
    ("membership", "Signup portal", "Portal button signup text"),
    ("membership", "Signup portal", "Portal button icon"),
    ("membership", "Signup portal", "Portal button style"),
    ("membership", "Signup portal", "Signup terms"),
    ("membership", "Welcome emails", "Welcome page URL"),
    ("membership", "Access", "Site access"),
    ("membership", "Access", "Access code"),
    ("membership", "Spam filters", "Blocked email domains"),
    ("membership", "Custom fields", "Field name"),
    ("membership", "Custom fields", "Member access"),
    ("growth", "Offers", "Discount code"),
    ("growth", "Offers", "Free trial"),
    ("membership", "Gift subscriptions", "Shareable link"),
    ("membership", "Tips & donations", "Currency"),
    ("membership", "Tips & donations", "Suggested amount"),
    ("newsletters", "Newsletter sending", "Newsletter sending"),
    ("newsletters", "Default recipients", "Who gets a new post by default"),
    ("features", "Comments", "New comments"),
    ("features", "Comments", "Hold for review"),
    ("features", "Automations", "Automations"),
    ("features", "Beta features", "Feature flags"),
    ("features", "Private features", "Developer experiments"),
    ("code-injection", "Site head", "Head"),
    ("code-injection", "Site foot", "Foot"),
    ("advanced", "Redirects & routes", "Redirects"),
    ("advanced", "Redirects & routes", "Routes"),
    // What someone actually types when they are moving a site in or out.
    ("migration", "Import", "Import content"),
    ("migration", "Import", "Universal import"),
    ("migration", "Import", "Ghost export file"),
    ("migration", "Import", "WordPress"),
    ("migration", "Import", "Substack"),
    ("migration", "Import", "CSV"),
    ("migration", "Import", "Migrate from another platform"),
    ("migration", "Export", "Export all data"),
    ("migration", "Export", "Content & settings"),
    ("migration", "Export", "Members"),
    ("migration", "Export", "Post analytics"),
    ("migration", "Export", "Themes"),
    ("migration", "Export", "Media files"),
    ("migration", "Export", "Backup"),
];

/// Sub-pages and on-page headings the group rows do not already carry:
/// things with their own URL nested inside a group (the built-in apps
/// Integrations lists), and headings that are neither the page's own
/// name nor one of its listed sub-pages. Each row links straight to where
/// the thing actually lives.
const SETTINGS_PLACES: &[(&str, &str, &str)] = &[
    // (path under /ghost/settings/, what you are looking for, parent group slug)
    ("integrations/zapier/", "Zapier", "integrations"),
    ("integrations/slack/", "Slack", "integrations"),
    ("integrations/unsplash/", "Unsplash", "integrations"),
    ("membership/", "Stripe", "membership"),
    ("growth/", "Coming later", "growth"),
];

/// The editable social accounts, in Ghost's order. Keys are Ghost's settings
/// keys; the seed keeps Ghost's own defaults (facebook `ghost`, x `@ghost`).
const SOCIAL_ACCOUNTS: &[(&str, &str)] = &[
    ("facebook", "Facebook"),
    ("twitter", "X (Twitter)"),
    ("threads", "Threads"),
    ("bluesky", "Bluesky"),
    ("mastodon", "Mastodon"),
    ("tiktok", "TikTok"),
    ("youtube", "YouTube"),
    ("instagram", "Instagram"),
    ("linkedin", "LinkedIn"),
];

/// The Ghost release this port tracks. Reported on the About page and
/// stamped into the export file's meta.
///
/// Re-exported from `ghost-core` rather than declared here: `/members/api/site`
/// publishes the same version to Portal from another crate, and holding two
/// copies is what let them drift from 5.98 and 6.39 apart.
pub use ghost_core::GHOST_VERSION;

/// Ghost's beta feature flags, as its Beta features tab lists them
/// (Ghost 6.73's `PUBLIC_BETA_FEATURES`). Flag, title, description.
///
/// Stored the way Ghost stores them: keys on the `labs` settings object.
const BETA_FLAGS: &[(&str, &str, &str)] = &[
    (
        "superEditors",
        "Enhanced Editor role (beta)",
        "Allows newly-assigned editors to manage members and comments in addition to regular roles.",
    ),
    (
        "editorExcerpt",
        "Show post excerpt inline",
        "Adds the excerpt input below the post title in the editor.",
    ),
    (
        "additionalPaymentMethods",
        "Additional payment methods",
        "Enable support for CashApp, iDEAL, Bancontact, and others.",
    ),
    (
        "navigationIcons",
        "Navigation icons & visibility",
        "Add icons and member-visibility controls to navigation menu items. Requires theme support to render icons.",
    ),
];

/// Ghost's private feature flags — its Private features tab, which Ghost shows
/// only when `enableDeveloperExperiments` is configured.
///
/// Titles and descriptions are Ghost's own, so a flag reads here as it does
/// there. `automations` sat with the beta flags until 6.73 moved it here —
/// this port's own Automations system is switched on in Settings → Features
/// and has never been gated on the flag, so the move is cosmetic parity.
const PRIVATE_FLAGS: &[(&str, &str, &str)] = &[
    ("automations", "Automations", "Build flows that react to what members do."),
    ("automationsLifecycleTriggers", "Automations lifecycle triggers", "Triggers for upgrades and downgrades."),
    ("stripeAutomaticTax", "Stripe Automatic Tax (private beta)", "Use Stripe Automatic Tax at Stripe Checkout. Needs to be enabled in Stripe"),
    ("importMemberTier", "Import Member Tier", "Enables tier to be specified when importing members"),
    ("csvContentImporter", "CSV Content Importer", "Enables importing posts from CSV files in the Universal Importer"),
    ("admin7Design", "Admin 7 · Design", "Preview the floating Admin 7 sidebar and its navigation layout."),
    ("admin7CustomizeFromViewSite", "Admin 7 · Customize from View site", "Customize the site design from View site, with a live preview. Needs Admin 7 · Design."),
    ("emailUniqueid", "Email Unique ID", "Enables {uniqueid} variable in emails for unique image URLs to bypass ESP image caching"),
    ("themeTranslation", "Updated theme translation (beta)", "Enable theme translation using i18next instead of the old translation package."),
    ("pictureImageFormats", "Picture Element", "Use the HTML picture element to serve modern image formats (AVIF, WebP) with automatic fallbacks"),
    ("getHelperDeduplication", "Get helper deduplication", "Deduplicate identical {{#get}} helper queries within a single request to avoid redundant database calls"),
    ("membersCustomFields", "Member custom fields", "Let admins create and manage custom field definitions for members, and choose which field each Stripe checkout answer is stored in"),
    ("stripeCheckoutCollection", "Stripe checkout collection", "Let admins turn on shipping address, phone number and tax number collection for a tier, asked by Stripe checkout and stored against the member"),
    ("paywallImprovements", "Paywall improvements", "Enables paywall usability, discoverability and email customization improvements"),
    ("machinePayments", "Machine payments", "Let AI agents pay for access to paid-members markdown (.md) URLs via Stripe Machine Payments Protocol"),
    ("apps", "Apps", "Install and manage third-party apps that run on their own servers. Early and incomplete."),
];

/// Whether structured data for LLMs is published.
///
/// Ghost treats this as on unless explicitly `false`, so a site that has never
/// seen the setting is opted in rather than silently opted out.
fn llms_enabled(state: &AppState) -> bool {
    !matches!(state.settings.get("llms_enabled"), Some(serde_json::Value::Bool(false)))
        && state.settings.get_str("llms_enabled").as_deref() != Some("false")
}

/// Read the `labs` settings object, which is a flat map of flag → bool.
fn labs_flags(state: &AppState) -> serde_json::Map<String, serde_json::Value> {
    match state.settings.get("labs") {
        Some(serde_json::Value::Object(m)) => m,
        // Older rows hold the object as a JSON *string*; both shapes read the
        // same here.
        Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => serde_json::Map::new(),
    }
}

fn labs_flag_on(flags: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    flags.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false)
}

/// One switch per flag, posted as part of `form_id`.
///
/// Every switch writes one key on the `labs` settings object, which is how
/// Ghost stores them too — so a flag set here is the same flag Ghost would
/// have set, and reaches a theme as `{{#if labs.<flag>}}`. The hidden `shown`
/// field is what tells the save which flags this render actually offered, so
/// an unchecked box can be told apart from a flag that was never on screen.
fn labs_flag_rows(
    flags: &serde_json::Map<String, serde_json::Value>,
    set: &[(&str, &str, &str)],
    form_id: &str,
) -> String {
    set.iter()
        .map(|(key, title, detail)| {
            format!(
                r#"<div class="row toggle-row"><span class="ic tint-teal">{ic}</span>
<div class="label"><b>{title}</b><span>{detail}</span></div>
<input type="hidden" name="shown" value="{key}" form="{form}">
<label class="switch"><input type="checkbox" name="{key}" value="1"{on} form="{form}"><span class="track"><span class="thumb"></span></span></label></div>"#,
                ic = svg("star"),
                key = esc(key),
                form = esc(form_id),
                title = esc(title),
                detail = esc(detail),
                on = if labs_flag_on(flags, key) { " checked" } else { "" },
            )
        })
        .collect()
}

/// The words Ghost builds a private-site access code from
/// (server/services/settings/private-site-access-code.js). Short and
/// sayable, because this code gets shared by hand.
const ACCESS_CODE_WORDS: &[&str] = &[
    "anchor", "aurora", "beacon", "birch", "bright", "cedar", "cloud", "comet", "copper", "coral",
    "ember", "fern", "field", "forest", "golden", "green", "harbor", "hidden", "horizon",
    "juniper", "lagoon", "lunar", "maple", "meadow", "midnight", "north", "ocean", "olive",
    "paper", "pine", "quiet", "river", "sage", "signal", "silver", "solstice", "sparrow", "stone",
    "studio", "summit", "sunrise", "thistle", "valley", "violet", "willow", "window", "winter",
    "wild",
];

/// Ghost's `word###` access code, off the same CSPRNG the rest of the port
/// mints secrets with.
fn generate_access_code() -> String {
    let word = ACCESS_CODE_WORDS
        [ghost_core::ids::random_below(ACCESS_CODE_WORDS.len() as u32) as usize];
    format!("{word}{:03}", ghost_core::ids::random_below(1000))
}

/// Currencies offered for tips and donations. Ghost stores
/// `donations_currency` as an upper-case ISO code, unlike a tier's price
/// currency, which it keeps lower-case.
const CURRENCIES: &[(&str, &str)] = &[
    ("USD", "USD $"),
    ("EUR", "EUR €"),
    ("GBP", "GBP £"),
    ("AUD", "AUD $"),
    ("CAD", "CAD $"),
    ("INR", "INR ₹"),
];

/// Ghost's timezone picker (moment-timezone names with GMT offsets), west →
/// east. A stored value outside the list is still shown as a raw option.
const TIMEZONES: &[(&str, &str)] = &[
    ("Etc/GMT+12", "(GMT-12:00) GMT+12"),
    ("Pacific/Honolulu", "(GMT-10:00) Hawaii"),
    ("America/Anchorage", "(GMT-09:00) Alaska"),
    ("America/Los_Angeles", "(GMT-08:00) Pacific Time (US & Canada)"),
    ("America/Vancouver", "(GMT-08:00) Vancouver"),
    ("America/Denver", "(GMT-07:00) Mountain Time (US & Canada)"),
    ("America/Phoenix", "(GMT-07:00) Arizona"),
    ("America/Chicago", "(GMT-06:00) Central Time (US & Canada)"),
    ("America/Mexico_City", "(GMT-06:00) Mexico City"),
    ("America/Bogota", "(GMT-05:00) Bogotá"),
    ("America/Lima", "(GMT-05:00) Lima"),
    ("America/New_York", "(GMT-05:00) Eastern Time (US & Canada)"),
    ("America/Toronto", "(GMT-05:00) Toronto"),
    ("America/Halifax", "(GMT-04:00) Halifax"),
    ("America/Santiago", "(GMT-04:00) Santiago"),
    ("America/Argentina/Buenos_Aires", "(GMT-03:00) Buenos Aires"),
    ("America/Sao_Paulo", "(GMT-03:00) São Paulo"),
    ("Atlantic/Reykjavik", "(GMT+00:00) Reykjavik"),
    ("Europe/London", "(GMT+00:00) London"),
    ("Europe/Lisbon", "(GMT+00:00) Lisbon"),
    ("Etc/UTC", "(GMT+00:00) UTC"),
    ("UTC", "(GMT+00:00) UTC"),
    ("Europe/Dublin", "(GMT+01:00) Dublin"),
    ("Europe/Amsterdam", "(GMT+01:00) Amsterdam"),
    ("Europe/Berlin", "(GMT+01:00) Berlin"),
    ("Europe/Brussels", "(GMT+01:00) Brussels"),
    ("Europe/Madrid", "(GMT+01:00) Madrid"),
    ("Europe/Paris", "(GMT+01:00) Paris"),
    ("Europe/Rome", "(GMT+01:00) Rome"),
    ("Europe/Stockholm", "(GMT+01:00) Stockholm"),
    ("Europe/Vienna", "(GMT+01:00) Vienna"),
    ("Europe/Warsaw", "(GMT+01:00) Warsaw"),
    ("Europe/Zurich", "(GMT+01:00) Zurich"),
    ("Africa/Lagos", "(GMT+01:00) Lagos"),
    ("Europe/Athens", "(GMT+02:00) Athens"),
    ("Europe/Helsinki", "(GMT+02:00) Helsinki"),
    ("Europe/Kiev", "(GMT+02:00) Kyiv"),
    ("Africa/Cairo", "(GMT+02:00) Cairo"),
    ("Africa/Johannesburg", "(GMT+02:00) Johannesburg"),
    ("Europe/Istanbul", "(GMT+03:00) Istanbul"),
    ("Europe/Moscow", "(GMT+03:00) Moscow"),
    ("Africa/Nairobi", "(GMT+03:00) Nairobi"),
    ("Asia/Riyadh", "(GMT+03:00) Riyadh"),
    ("Asia/Tehran", "(GMT+03:30) Tehran"),
    ("Asia/Dubai", "(GMT+04:00) Dubai"),
    ("Asia/Karachi", "(GMT+05:00) Karachi"),
    ("Asia/Kolkata", "(GMT+05:30) Kolkata"),
    ("Asia/Kathmandu", "(GMT+05:45) Kathmandu"),
    ("Asia/Dhaka", "(GMT+06:00) Dhaka"),
    ("Asia/Yangon", "(GMT+06:30) Yangon"),
    ("Asia/Bangkok", "(GMT+07:00) Bangkok"),
    ("Asia/Jakarta", "(GMT+07:00) Jakarta"),
    ("Asia/Shanghai", "(GMT+08:00) Shanghai"),
    ("Asia/Hong_Kong", "(GMT+08:00) Hong Kong"),
    ("Asia/Kuala_Lumpur", "(GMT+08:00) Kuala Lumpur"),
    ("Asia/Manila", "(GMT+08:00) Manila"),
    ("Asia/Singapore", "(GMT+08:00) Singapore"),
    ("Asia/Taipei", "(GMT+08:00) Taipei"),
    ("Asia/Seoul", "(GMT+09:00) Seoul"),
    ("Asia/Tokyo", "(GMT+09:00) Tokyo"),
    ("Australia/Adelaide", "(GMT+09:30) Adelaide"),
    ("Australia/Brisbane", "(GMT+10:00) Brisbane"),
    ("Australia/Melbourne", "(GMT+10:00) Melbourne"),
    ("Australia/Sydney", "(GMT+10:00) Sydney"),
    ("Australia/Perth", "(GMT+08:00) Perth"),
    ("Pacific/Auckland", "(GMT+12:00) Auckland"),
    ("Pacific/Fiji", "(GMT+12:00) Fiji"),
];

/// Ghost's publication language picker: the setting stores the `locale`
/// code (Ghost has no separate `lang` setting).
///
/// Codes and coverage come from Ghost's own list, which matters beyond the
/// label: a theme's translations live in `<theme>/locales/<code>.json`, so a
/// locale saved under a code Ghost does not use (`zh-cn` rather than `zh`)
/// silently finds no translation file. Labels lead with the language's own
/// name, because that is what someone scans for when picking their own.
const LOCALES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("ar", "العربية (Arabic)"),
    ("az", "Azərbaycan (Azerbaijani)"),
    ("bg", "Български (Bulgarian)"),
    ("bn", "বাংলা (Bengali)"),
    ("bs", "Bosanski (Bosnian)"),
    ("ca", "Català (Catalan)"),
    ("cs", "Čeština (Czech)"),
    ("da", "Dansk (Danish)"),
    ("de", "Deutsch (German)"),
    ("de-CH", "Schweizerdeutsch (Swiss German)"),
    ("el", "Ελληνικά (Greek)"),
    ("en", "English"),
    ("eo", "Esperanto"),
    ("es", "Español (Spanish)"),
    ("et", "Eesti (Estonian)"),
    ("eu", "Euskara (Basque)"),
    ("fa", "فارسی (Persian/Farsi)"),
    ("fi", "Suomi (Finnish)"),
    ("fr", "Français (French)"),
    ("gd", "Gàidhlig (Gaelic (Scottish))"),
    ("he", "עברית (Hebrew)"),
    ("hi", "हिन्दी (Hindi)"),
    ("hr", "Hrvatski (Croatian)"),
    ("hu", "Magyar (Hungarian)"),
    ("id", "Bahasa Indonesia (Indonesian)"),
    ("is", "Íslenska (Icelandic)"),
    ("it", "Italiano (Italian)"),
    ("ja", "日本語 (Japanese)"),
    ("ko", "한국어 (Korean)"),
    ("kz", "Қазақша (Kazakh)"),
    ("lt", "Lietuvių (Lithuanian)"),
    ("lv", "Latviešu (Latvian)"),
    ("mk", "Македонски (Macedonian)"),
    ("mn", "Монгол (Mongolian)"),
    ("ms", "Bahasa Melayu (Malay)"),
    ("nb", "Norsk bokmål (Norwegian Bokmål)"),
    ("ne", "नेपाली (Nepali)"),
    ("nl", "Nederlands (Dutch)"),
    ("nn", "Norsk nynorsk (Norwegian Nynorsk)"),
    ("pa", "ਪੰਜਾਬੀ (Punjabi)"),
    ("pl", "Polski (Polish)"),
    ("pt", "Português (Portuguese)"),
    ("pt-BR", "Português (Brasil) (Portuguese (Brazil))"),
    ("ro", "Română (Romanian)"),
    ("ru", "Русский (Russian)"),
    ("si", "සිංහල (Sinhala)"),
    ("sk", "Slovenčina (Slovak)"),
    ("sl", "Slovenščina (Slovenian)"),
    ("sq", "Shqip (Albanian)"),
    ("sr", "Srpski (Serbian)"),
    ("sr-Cyrl", "Српски (Serbian (Cyrillic))"),
    ("sv", "Svenska (Swedish)"),
    ("sw", "Kiswahili (Swahili)"),
    ("ta", "தமிழ் (Tamil)"),
    ("th", "ไทย (Thai)"),
    ("tr", "Türkçe (Turkish)"),
    ("uk", "Українська (Ukrainian)"),
    ("ur", "اردو (Urdu)"),
    ("uz", "Oʻzbekcha (Uzbek)"),
    ("vi", "Tiếng Việt (Vietnamese)"),
    ("zh", "中文(简体) (Chinese)"),
    ("zh-Hant", "中文(繁體) (Traditional Chinese)"),
];
/// `<option>` list for a picker, marking `current`; a stored value outside
/// the list (e.g. a locale this list doesn't cover) still shows up raw.
fn select_options(list: &[(&str, &str)], current: &str) -> String {
    let opts: String = list
        .iter()
        .map(|(value, label)| {
            let selected = if *value == current { " selected" } else { "" };
            format!(r#"<option value="{value}"{selected}>{label}</option>"#, value = esc(value), selected = selected, label = esc(label))
        })
        .collect();
    if !current.is_empty() && !list.iter().any(|(v, _)| *v == current) {
        format!(r#"<option value="{c}" selected>{c}</option>{opts}"#, c = esc(current), opts = opts)
    } else {
        opts
    }
}

/// Dropdown caret used inside `.field select` rows.
const SELECT_CARET: &str = r#"<svg class="car" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 10l5 5 5-5z"/></svg>"#;

/// An M3 segmented button: the connected-chips control for a setting with a
/// handful of options, where every choice deserves to be visible at once.
/// Radios post exactly like the select they replace.
fn segmented(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let segs: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                r#"<label><input type="radio" name="{name}" value="{value}"{ck}><span class="ck">{check}</span><span>{label}</span></label>"#,
                name = esc(name),
                value = esc(value),
                ck = if *value == current { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    format!(r#"<div class="seg" role="radiogroup">{segs}</div>"#)
}

/// A compact dropdown for a settings row's trailing position, for option
/// lists too long for a segmented button.
///
/// `label` is the row's own title: the control has no visible label of its
/// own, so without it a screen reader announces the select by its current
/// value — "Bold" rather than "Title weight".
fn selbox(name: &str, label: &str, options: &[(&str, &str)], current: &str) -> String {
    format!(
        r#"<div class="selbox"><select name="{name}" aria-label="{label}">{opts}</select>{car}</div>"#,
        name = esc(name),
        label = esc(label),
        opts = select_options(options, current),
        car = SELECT_CARET,
    )
}

/// The text counterpart of [`selbox`], for a value that is typed rather than
/// chosen.
fn textbox(name: &str, label: &str, value: &str) -> String {
    format!(
        r#"<div class="textbox"><input name="{name}" value="{value}" aria-label="{label}"></div>"#,
        name = esc(name),
        value = esc(value),
        label = esc(label),
    )
}

/// Redirect back to a settings group page carrying a one-shot banner.
fn settings_flash(group: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/{group}/?{encoded}")).into_response()
}

/// The one-shot ok/error banner carried in the query string.
fn flash_banner(q: &HashMap<String, String>) -> String {
    match (q.get("ok"), q.get("error")) {
        (Some(m), _) => format!(r#"<div class="banner ok rise" style="--i:1">{check}{}</div>"#, esc(m), check = svg("check")),
        (_, Some(m)) => format!(r#"<div class="banner error rise" style="--i:1">{warn}{}</div>"#, esc(m), warn = svg("flag")),
        _ => String::new(),
    }
}

/// 404 for a caller that has no role to hand; renders the minimal rail.
fn not_found_page(user: &ghost_core::models::User, message: &str) -> Response {
    not_found_page_for(Role::Contributor, user, message)
}

/// A 404 rendered inside the admin shell, for a URL that is well-formed but
/// names something that isn't there (or that this user may not see — the two
/// are deliberately indistinguishable, so the page can't be used to probe
/// which staff accounts exist).
fn not_found_page_for(role: Role, user: &ghost_core::models::User, message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        page_for(
            "Not found",
            "",
            Some(user),
            role,
            &format!(
                r#"<header class="page-head rise"><div><h1>Not found</h1><p class="sub">{msg}</p></div></header>
<div class="card padded rise" style="--i:1;text-align:center">
<p class="muted" style="margin:8px 0 16px">{msg}</p>
<a class="btn tonal" href="/ghost/settings/">Back to settings</a></div>"#,
                msg = esc(message),
            ),
            true,
        ),
    )
        .into_response()
}

/// Shared header for a settings group page: back to the settings index, the
/// group title with its sub-pages as subtitle, and an optional Save button
/// bound to the page's form.
fn group_head(g: &SettingsGroup, form_id: Option<&str>) -> String {
    let save = match form_id {
        Some(id) => format!(
            r#"<button class="btn" type="submit" form="{id}">{check} Save</button>"#,
            id = esc(id),
            check = svg("check"),
        ),
        None => String::new(),
    };
    format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>{title}</h1><p class="sub">{sub}</p></div>
<div class="actions">{save}</div></header>"#,
        back = svg("chevron"),
        title = esc(g.title),
        sub = esc(&g.items.join(" · ")),
        save = save,
    )
}

/// A settings row that isn't built yet (kept visible so the hub shows
/// Ghost's full structure while pages are completed one by one).

async fn settings_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let rows: String = SETTINGS_GROUPS
        .iter()
        .map(|g| {
            let head = format!(
                r#"<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>"#,
                tint = g.tint,
                ic = svg(g.icon),
                title = esc(g.title),
                detail = esc(&g.items.join(" · ")),
            );
            format!(
                r#"<a class="row" href="{path}">{head}
<div class="trailing">{chev}</div></a>"#,
                path = g.path(),
                head = head,
                chev = svg("chevron"),
            )
        })
        .collect();
    // The searchable index, four kinds deep: the pages themselves, the
    // sub-page each group lists, the nested places and headings, and the
    // fields. A page's row answers to its own name and its sub-pages' names,
    // but everything below it matches only its own text — so a broad query
    // ("advanced") lands on the page, and a narrow one ("port", "zapier",
    // "built-in") lands on the thing itself without flooding the page in.
    let mut hit_rows: Vec<String> = Vec::new();
    let mut push_hit = |url: String, title: &str, detail: &str, haystack: &str, group: &SettingsGroup| {
        hit_rows.push(format!(
            r#"<a class="row" href="{url}" data-search="{haystack}">
<span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing">{chev}</div></a>"#,
            url = esc(&url),
            haystack = esc(haystack),
            tint = group.tint,
            ic = svg(group.icon),
            title = esc(title),
            detail = esc(detail),
            chev = svg("chevron"),
        ));
    };
    for g in SETTINGS_GROUPS {
        push_hit(
            g.path(),
            g.title,
            &g.items.join(" · "),
            &format!("{} {} {}", g.title, g.items.join(" "), g.slug),
            g,
        );
        for item in g.items {
            push_hit(g.path(), item, g.title, item, g);
        }
    }
    for (path, name, parent) in SETTINGS_PLACES {
        if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == *parent) {
            push_hit(format!("/ghost/settings/{path}"), name, g.title, name, g);
        }
    }
    let mut fields: Vec<(&str, &str, &str)> = SETTINGS_INDEX.to_vec();
    for (_, name) in SOCIAL_ACCOUNTS {
        fields.push(("general", "Social accounts", name));
    }
    for (slug, section, field) in &fields {
        if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == *slug) {
            push_hit(
                g.path(),
                field,
                &format!("{} · {}", g.title, section),
                &format!("{section} {field}"),
                g,
            );
        }
    }
    // Each feature flag answers to its own name, so "automatic tax" or
    // "navigation icons" finds the switch rather than only the page it sits
    // on. Folded in here rather than copied into `SETTINGS_INDEX` so the two
    // lists cannot drift, and the flag key joins the haystack because that is
    // what a developer reading a theme has in front of them.
    //
    // The private flags are indexed only where the Features page renders
    // them: on a production site, a hit would lead to a row that is not
    // there.
    if let Some(g) = SETTINGS_GROUPS.iter().find(|g| g.slug == "features") {
        let private: &[(&str, &str, &str)] =
            if state.config.env == "development" { PRIVATE_FLAGS } else { &[] };
        for (section, set) in [("Beta features", BETA_FLAGS), ("Private features", private)] {
            for (key, title, _) in set {
                push_hit(
                    g.path(),
                    title,
                    &format!("{} · {}", g.title, section),
                    &format!("{section} {title} {key}"),
                    g,
                );
            }
        }
    }
    let hits = hit_rows.join("\n");
    page_for_role(actor_role,
        "Settings",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><div><h1>Settings</h1><p class="sub">Site configuration</p></div></header>
<div class="search rise" style="--i:1">{search}<input id="settings-search" type="search" placeholder="Search all settings" aria-label="Search all settings"></div>
<div class="card rows rise" style="--i:2" id="settings-groups">{rows}</div>
<div class="card rows" id="settings-hits" style="display:none">{hits}
<div class="row" id="settings-empty" style="display:none"><div class="body"><span class="title">Nothing matches</span><span class="detail">Try another word, or clear the search to browse the pages.</span></div></div>
</div>"#,
            search = svg("search"),
            rows = rows,
            hits = hits,
        ),
        true,
    )
    .into_response()
}

// --- general: title & description, timezone, publication language, staff ---

async fn general_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "general").unwrap();
    page_for_role(actor_role,
        "General settings",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/general/" id="general-form" data-meta-previews data-site-title="{title}" data-site-description="{description}">
<div class="section">Name, title &amp; description</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="name" value="{name}" placeholder=" "><span>Name</span></label>
<p class="muted" style="margin:6px 0 0">Shown in the admin panel and available to themes as <code>{{{{@site.name}}}}</code>. Defaults to the title when blank.</p></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="title" value="{title}" placeholder=" " required><span>Title</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-pink">{notes}</span>
<div class="body"><label class="field" style="margin:0"><input name="description" value="{description}" placeholder=" "><span>Description</span></label></div></div>
</div>
<div class="section">Timezone</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:center"><span class="ic tint-pink">{globe}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="timezone">{tz_opts}</select><span>Site timezone</span>{car}</label></div></div>
</div>
<div class="section">Publication language</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:center"><span class="ic tint-pink">{lang}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="locale">{locale_opts}</select><span>Publication language</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Used as the site's <code>&lt;html lang&gt;</code> by themes.</p></div></div>
</div>
{metadata}
{social}
</form>"#,
            head = group_head(group, Some("general-form")),
            banner = flash_banner(&q),
            title_ic = svg("title"),
            notes = svg("notes"),
            globe = svg("globe"),
            lang = svg("notes"),
            car = SELECT_CARET,
            title = s("title"),
            name = s("name"),
            description = s("description"),
            tz_opts = select_options(TIMEZONES, &state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into())),
            locale_opts = select_options(LOCALES, &state.settings.get_str("locale").unwrap_or_else(|| "en".into())),
            metadata = metadata_sections(&state),
            social = social_sections(&state),
        ),
        true,
    )
    .into_response()
}

async fn general_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let get = |k: &str| form.get(k).cloned().unwrap_or_default();
    let title = get("title").trim().to_string();
    if title.is_empty() {
        return settings_flash("general", "error", "Title can't be empty.");
    }
    let non_empty = |v: String, fallback: &str| {
        let v = v.trim();
        if v.is_empty() { fallback.to_string() } else { v.to_string() }
    };
    let timezone = non_empty(get("timezone"), "Etc/UTC");
    let locale = non_empty(get("locale"), "en");
    let mut ok = state.settings.set(&state.db, "title", &serde_json::json!(title)).await.is_ok()
        && state.settings.set(&state.db, "description", &serde_json::json!(get("description"))).await.is_ok()
        && state.settings.set(&state.db, "timezone", &serde_json::json!(timezone)).await.is_ok()
        && state.settings.set(&state.db, "locale", &serde_json::json!(locale)).await.is_ok();
    if let Some(name) = form.get("name") {
        ok &= state.settings.set(&state.db, "name", &serde_json::json!(name.trim())).await.is_ok();
    }

    // Meta data and the social accounts post with this form now, so one Save
    // covers the page.
    for key in METADATA_KEYS.iter().chain(SOCIAL_ACCOUNTS.iter().map(|(k, _)| k)) {
        ok &= state.settings.set(&state.db, key, &serde_json::json!(get(key))).await.is_ok();
    }
    let llms = form.get("llms_enabled").map(|v| v == "1").unwrap_or(false);
    ok &= state.settings.set(&state.db, "llms_enabled", &serde_json::Value::Bool(llms)).await.is_ok();

    if ok {
        settings_saved(&state, "general", "General settings saved.")
    } else {
        settings_flash("general", "error", "Saving failed — check the logs.")
    }
}

// --- meta data: metadata, twitter card, facebook card ---

/// One meta-data text field row: label + input + recommended-length hint.
fn meta_field_row(icon_svg: &str, tint: &str, key: &str, label: &str, value: &str, hint: &str) -> String {
    format!(
        r#"<div class="row" style="align-items:center"><span class="ic tint-{tint}">{icon}</span>
<div class="body"><label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" "><span>{label}</span></label>
<p class="muted" style="margin:6px 0 0">{hint}</p></div></div>"#,
        tint = tint,
        icon = icon_svg,
        key = esc(key),
        label = esc(label),
        value = esc(value),
        hint = esc(hint),
    )
}

/// The mock search result Ghost shows above the metadata fields.
///
/// `title` and `description` carry the saved values already resolved against
/// their site-level fallbacks, so the preview reads the way a result actually
/// would. The elements are tagged with `data-prev` so the page script can
/// retitle them as the fields are typed into.
fn search_preview(site_url: &str, icon: &str, title: &str, description: &str) -> String {
    let trimmed = site_url.trim_end_matches('/');
    let host = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let favicon = if icon.is_empty() {
        String::new()
    } else {
        format!(r#" style="background-image:url('{}')""#, esc(icon))
    };
    format!(
        r#"<div class="seo-prev rise" style="--i:1"><span class="cap">Preview</span>
<div class="sheet">
<div class="site"><span class="favicon"{favicon}></span>
<div><div class="host">{host}</div><div class="addr">{url}</div></div></div>
<span class="g-title" data-prev="search-title">{title}</span>
<span class="g-desc" data-prev="search-description">{description}</span>
</div></div>"#,
        favicon = favicon,
        host = esc(host),
        url = esc(trimmed),
        title = esc(title),
        description = esc(description),
    )
}

/// The mock social post Ghost shows above the X and Facebook card fields.
///
/// `network` is the field-name stem (`twitter` or `og`), which is also what
/// the page script keys its live updates off.
fn social_card_preview(
    network: &str,
    site_url: &str,
    site_title: &str,
    icon: &str,
    image: &str,
    title: &str,
    description: &str,
) -> String {
    let trimmed = site_url.trim_end_matches('/');
    let host = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let avatar = if icon.is_empty() {
        String::new()
    } else {
        format!(r#" style="background-image:url('{}')""#, esc(icon))
    };
    let shot = if image.is_empty() {
        format!(r#"<span class="shot empty" data-prev="{network}-shot">No image set</span>"#)
    } else {
        format!(
            r#"<span class="shot" data-prev="{network}-shot"><img src="{src}" alt=""></span>"#,
            src = esc(image)
        )
    };
    format!(
        r#"<div class="seo-prev social rise" style="--i:1"><span class="cap">Preview</span>
<div class="sheet">
<div class="who"><span class="avatar"{avatar}></span>
<div><div class="name">{site_title}</div><div class="when">2h</div></div></div>
<div class="bar"></div><div class="bar short"></div>
<div class="card-mock">{shot}
<div class="body"><span class="t" data-prev="{network}-title">{title}</span>
<span class="d" data-prev="{network}-description">{description}</span>
<span class="u">{host}</span></div></div>
</div></div>"#,
        avatar = avatar,
        site_title = esc(site_title),
        network = esc(network),
        shot = shot,
        title = esc(title),
        description = esc(description),
        host = esc(host),
    )
}

/// One meta-data image row: preview + URL input (like the design page).
fn meta_image_row(key: &str, label: &str, value: &str) -> String {
    let preview = if value.is_empty() {
        String::new()
    } else {
        format!(
            r#"<img src="{v}" alt="" style="height:44px;width:88px;object-fit:cover;border-radius:10px;box-shadow:var(--shadow-1)">"#,
            v = esc(value)
        )
    };
    format!(
        r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{img}</span>
<div class="body" style="width:100%">
<div style="display:flex;gap:14px;align-items:center">{preview}
<label class="field" style="margin:0;flex:1"><input name="{key}" value="{value}" placeholder=" "><span>{label} · URL</span></label></div></div></div>"#,
        img = svg("image"),
        key = esc(key),
        value = esc(value),
        label = esc(label),
        preview = preview,
    )
}

/// The Meta data sections — search result, X card, Facebook card.
///
/// Built as a fragment rather than a page of its own: these live inside
/// General settings, under the title and description they fall back to, so
/// the thing being overridden is on screen beside the override.
fn metadata_sections(state: &AppState) -> String {
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let raw = |k: &str| state.settings.get_str(k).unwrap_or_default();

    // Every card falls back the way the front end does — the site title and
    // description, then the meta overrides — so the previews show what a
    // crawler would actually read rather than an empty box.
    let site_title = raw("title");
    let site_description = raw("description");
    let icon = raw("icon");
    let site_url = state.site_url();
    let fallback = |specific: &str, meta: &str, site: &str| {
        let specific = raw(specific);
        if !specific.is_empty() {
            return specific;
        }
        let meta = raw(meta);
        if !meta.is_empty() { meta } else { site.to_string() }
    };

    format!(
            r#"<div class="section">Meta data · search</div>
{search_preview}
<div class="card rows rise" style="--i:4">
{meta_title}{meta_description}
{llms_row}
</div>
<div class="section">Meta data · X card</div>
{x_preview}
<div class="card rows rise" style="--i:5">
{twitter_title}{twitter_description}{twitter_image}
</div>
<div class="section">Meta data · Facebook card</div>
{fb_preview}
<div class="card rows rise" style="--i:6">
{og_title}{og_description}{og_image}
</div>"#,
            // Ghost reads this as "on unless explicitly false", so a site that
            // has never touched it is opted in.
            llms_row = format!(
                r#"<div class="row toggle-row"><span class="ic tint-purple">{ic}</span>
<div class="label"><b>Enable structured data for LLMs and AI search engines</b>
<span>Publishes <code>/llms.txt</code>, and lets any post or page URL be fetched as Markdown by appending <code>.md</code>.</span></div>
<label class="switch"><input type="checkbox" name="llms_enabled" value="1"{on} form="general-form"><span class="track"><span class="thumb"></span></span></label></div>"#,
                ic = svg("globe"),
                on = if llms_enabled(state) { " checked" } else { "" },
            ),
            search_preview = search_preview(
                &site_url,
                &icon,
                &fallback("meta_title", "title", &site_title),
                &fallback("meta_description", "description", &site_description),
            ),
            x_preview = social_card_preview(
                "twitter",
                &site_url,
                &site_title,
                &icon,
                &raw("twitter_image"),
                &fallback("twitter_title", "meta_title", &site_title),
                &fallback("twitter_description", "meta_description", &site_description),
            ),
            fb_preview = social_card_preview(
                "og",
                &site_url,
                &site_title,
                &icon,
                &raw("og_image"),
                &fallback("og_title", "meta_title", &site_title),
                &fallback("og_description", "meta_description", &site_description),
            ),
            meta_title = meta_field_row(&svg("title"), "purple", "meta_title", "Meta title", &s("meta_title"), "Recommended: under 70 characters. Falls back to the site title."),
            meta_description = meta_field_row(&svg("notes"), "purple", "meta_description", "Meta description", &s("meta_description"), "Recommended: under 156 characters. Falls back to the site description."),
            // The settings keys stay `twitter_*`: Ghost renamed the label to X
            // but kept the keys, and they are what `twitter:` meta tags read.
            twitter_title = meta_field_row(&svg("title"), "purple", "twitter_title", "X title", &s("twitter_title"), "Recommended: under 70 characters."),
            twitter_description = meta_field_row(&svg("notes"), "purple", "twitter_description", "X description", &s("twitter_description"), "Recommended: under 200 characters."),
            twitter_image = meta_image_row("twitter_image", "X image", &s("twitter_image")),
            og_title = meta_field_row(&svg("title"), "purple", "og_title", "Facebook title", &s("og_title"), "Recommended: under 70 characters."),
            og_description = meta_field_row(&svg("notes"), "purple", "og_description", "Facebook description", &s("og_description"), "Recommended: under 200 characters."),
            og_image = meta_image_row("og_image", "Facebook image", &s("og_image")),
    )
}

/// The text keys the Meta data sections own.
const METADATA_KEYS: &[&str] = &[
    "meta_title", "meta_description",
    "twitter_title", "twitter_description", "twitter_image",
    "og_title", "og_description", "og_image",
];

// --- email: the transactional transport that carries member magic links ---

/// Settings whose value is a credential. They are never rendered back into
/// the page (the field shows a "saved" placeholder instead), are masked by
/// the Admin API, and are absent from the theme/Content API allowlist — so a
/// password typed here cannot leak back out through any read path.
pub const SECRET_SETTINGS: &[&str] = &["mail_smtp_pass", "mailgun_api_key", "mailgun_webhook_signing_key"];

/// A posted credential field. An empty box means "leave what is stored"
/// rather than "clear it", because the form never shows the current value —
/// without this, saving any other field on the page would wipe the password.
async fn save_secret(state: &AppState, key: &str, posted: Option<&String>) -> bool {
    match posted.map(|s| s.trim()) {
        None | Some("") => true,
        // an explicit sentinel is how you deliberately clear one
        Some("-") => state.settings.set(&state.db, key, &serde_json::json!("")).await.is_ok(),
        Some(v) => state.settings.set(&state.db, key, &serde_json::json!(v)).await.is_ok(),
    }
}

/// Whether a credential is stored, phrased for the helper text under the
/// field. Short enough not to clip the floating label it sits beside.
fn secret_state(state: &AppState, key: &str) -> &'static str {
    if state.settings.get_str(key).unwrap_or_default().trim().is_empty() {
        "Nothing stored yet."
    } else {
        "A value is stored. Leave the box empty to keep it, type to replace it, or enter a single “-” to clear it."
    }
}

async fn email_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "email").unwrap();
    // Blank means Direct, the same fallback the mailer itself applies.
    let transport = state
        .settings
        .get_str("mail_transport")
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "Direct".into());
    let security = state
        .settings
        .get_str("mail_smtp_security")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "starttls".into());
    let site_url = state.site_url();
    let cfg = ghost_mail::MailConfig::from_settings(
        &state.settings,
        &state.settings.get_str("title").unwrap_or_else(|| "Ghost".into()),
        &site_url,
    );
    let status = if cfg.uses_relay() {
        format!(
            r#"<div class="banner ok rise" style="--i:1">{check}<span class="msg">Sending as <code>{from}</code> over SMTP</span></div>"#,
            check = svg("check"),
            from = esc(&cfg.from_address),
        )
    } else {
        format!(
            r#"<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Using the <strong>Direct</strong> transport, sending as <code>{from}</code>. Direct delivers straight to each recipient's real mail server — it needs outbound port 25, which many networks block, and mail from an unauthenticated address is usually rejected or marked as spam. <strong>A local mail catcher will not see Direct mail</strong>: to test with MailHog or Mailpit, switch to SMTP below. Configure SMTP for anything real, too.</span></div>"#,
            bulb = svg("mail"),
            from = esc(&cfg.from_address),
        )
    };

    page_for_role(actor_role, 
        "Email",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}{status}
<form method="post" action="/ghost/settings/email/" id="email-form">
<div class="section">Transactional email</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:center"><span class="ic tint-blue">{mail}</span>
<div class="body"><label class="field fixed" style="margin:0"><select name="mail_transport">{transport_opts}</select><span>Transport</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Carries member signup and signin magic links, and password resets. Ghost sets this as <code>mail.transport</code> in <code>config.production.json</code>; here it lives in Settings. <strong>Direct</strong> is Ghost’s default and needs no credentials.</p></div></div>
</div>
<div class="section">SMTP server</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{code}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mail_smtp_host" value="{host}" placeholder=" " autocomplete="off"><span>Host · e.g. smtp.mailgun.org</span></label>
<label class="field" style="margin:0"><input name="mail_smtp_port" value="{port}" placeholder=" " inputmode="numeric" autocomplete="off"><span>Port · 587 STARTTLS, 465 TLS</span></label>
<label class="field fixed" style="margin:0"><select name="mail_smtp_security">{security_opts}</select><span>Encryption</span>{car}</label>
<label class="field" style="margin:0"><input name="mail_smtp_user" value="{user}" placeholder=" " autocomplete="off"><span>Username</span></label>
<label class="field" style="margin:0"><input name="mail_smtp_pass" type="password" value="" placeholder=" " autocomplete="new-password"><span>Password</span></label>
</div>
<p class="muted" style="margin:10px 0 0">The password is never shown again after it is saved. {pass_hint}</p>
<p class="muted" style="margin:6px 0 0">Testing locally? A mail catcher takes everything and delivers nothing: MailHog and Mailpit both listen on <code>127.0.0.1</code> port <code>1025</code> with encryption <strong>None</strong> and no username or password.</p>
</div></div>
</div>
<div class="section">Sender identity</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{person}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mail_from_address" value="{from_address}" placeholder=" "><span>From address</span></label>
<label class="field" style="margin:0"><input name="mail_from_name" value="{from_name}" placeholder=" "><span>From name</span></label>
<label class="field" style="margin:0"><input name="mail_reply_to" value="{reply_to}" placeholder=" "><span>Reply-to · optional</span></label>
</div>
<p class="muted" style="margin:10px 0 0">Defaults to <code>{default_from}</code>. Most providers reject a From address on a domain you have not verified.</p>
</div></div>
</div>
</form>
<div class="section">Send a test</div>
<div class="card rows rise" style="--i:5">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{send}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Check the settings work</span>
<form method="post" action="/ghost/settings/email/test/" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<label class="field" style="margin:0;flex:1;min-width:220px"><input name="to" type="email" value="{owner_email}" placeholder=" " required><span>Send to</span></label>
<button class="btn tonal small" type="submit">Send test email</button>
</form>
<p class="muted" style="margin:10px 0 0">Save your changes first. Unlike magic links, this reports the transport error directly so you can debug credentials here.</p>
</div></div>
</div>"#,
            head = group_head(group, Some("email-form")),
            banner = flash_banner(&q),
            status = status,
            mail = svg("mail"),
            code = svg("code"),
            person = svg("ghost"),
            send = svg("send"),
            car = SELECT_CARET,
            transport_opts = select_options(&[("Direct", "Direct · no relay, straight to the recipient"), ("SMTP", "SMTP · send through a relay")], &transport),
            security_opts = select_options(
                &[("starttls", "STARTTLS · port 587"), ("tls", "TLS/SSL · port 465"), ("none", "None · local relay only")],
                &security
            ),
            host = s("mail_smtp_host"),
            port = s("mail_smtp_port"),
            user = s("mail_smtp_user"),
            pass_hint = secret_state(&state, "mail_smtp_pass"),
            from_address = s("mail_from_address"),
            from_name = s("mail_from_name"),
            reply_to = s("mail_reply_to"),
            default_from = esc(&cfg.from_address),
            owner_email = esc(user.email.as_deref().unwrap_or("")),
        ),
        true,
    )
    .into_response()
}

async fn email_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in [
        "mail_transport", "mail_smtp_host", "mail_smtp_port", "mail_smtp_security",
        "mail_smtp_user", "mail_from_address", "mail_from_name", "mail_reply_to",
    ] {
        let value = form.get(key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    ok &= save_secret(&state, "mail_smtp_pass", form.get("mail_smtp_pass")).await;
    if ok {
        settings_flash("email", "ok", "Email settings saved.")
    } else {
        settings_flash("email", "error", "Saving failed — check the logs.")
    }
}

/// Send a test message with the saved settings, reporting the transport's
/// own error. This is the one place a mail failure is surfaced to a human —
/// magic-link sends stay silent on purpose.
async fn email_test(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let to = form.get("to").cloned().unwrap_or_default().trim().to_string();
    if !to.contains('@') {
        return settings_flash("email", "error", "Enter a valid address to send the test to.");
    }
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let (subject, html, text) = ghost_mail::templates::test_email(&title, &site_url);
    let msg = ghost_mail::Message { to: to.clone(), subject, html, text };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => settings_flash("email", "ok", &format!("Test email sent to {to}.")),
        Err(e) => settings_flash("email", "error", &format!("Send failed: {}", e.message())),
    }
}

// --- newsletters: the newsletters themselves, and the Mailgun transport
//     that carries them. Ghost keeps both on this one settings screen. ---

/// Redirect back to one newsletter's editor with a one-shot banner.
fn newsletter_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/newsletters/{id}/?{encoded}")).into_response()
}

/// A colour setting, as a mode `<select>` plus a swatch for a custom value.
///
/// Every colour column in `newsletters` holds *either* a sentinel — `accent`,
/// `light`, `transparent` — *or* a literal `#rrggbb`, so one control cannot
/// express it. The select picks the meaning; the swatch only matters when the
/// meaning is "a specific colour".
///
/// `modes` are `(stored value, label)` pairs, where an empty stored value
/// means the column's "unset" state (NULL, or the column default).
/// A colour setting as Ghost-style swatch chips: one per named mode plus a
/// custom picker. Posts `{key}_mode` and `{key}_custom`, which
/// [`colour_from_form`] folds back into the stored value.
///
/// Each mode carries the colour its chip should show — `""` for a mode with
/// no single colour (Automatic), `"none"` for transparent, else a hex.
fn colour_row(
    key: &str,
    label: &str,
    detail: &str,
    modes: &[(&str, &str, &str)],
    stored: Option<&str>,
    default_mode: &str,
) -> String {
    let custom = stored.filter(|v| ghost_mail::newsletter::is_hex_color(v));
    let mode = match custom {
        Some(_) => "custom",
        None => stored
            .map(|v| v.trim())
            .filter(|v| modes.iter().any(|(m, _, _)| m == v))
            .unwrap_or(default_mode),
    };

    let chips: String = modes
        .iter()
        .map(|(value, name, swatch)| {
            let dot = match *swatch {
                "" => String::new(),
                "none" => r#"<span class="dot none"></span>"#.to_string(),
                hex => format!(r#"<span class="dot" style="background:{hex}"></span>"#),
            };
            format!(
                r#"<label class="swatch"><input type="radio" name="{key}_mode" value="{value}"{ck}>{dot}<span>{name}</span></label>"#,
                key = esc(key),
                value = esc(value),
                ck = if *value == mode { " checked" } else { "" },
                name = esc(name),
            )
        })
        .collect();

    // A swatch has to be a valid hex for the colour input to accept it, so an
    // unset value falls back to Ghost's own dark ink.
    let custom_swatch = custom.unwrap_or("#15212a");

    format!(
        r#"<div class="row"><span class="ic tint-pink">{palette}</span>
<div class="body"><span class="title">{label}</span><span class="detail">{detail}</span>
<div class="swatches under">{chips}
<label class="swatch custom"><input type="radio" name="{key}_mode" value="custom"{ck_custom}><input type="color" name="{key}_custom" value="{custom_swatch}" aria-label="{label} custom colour"><span>Custom</span></label>
</div></div></div>"#,
        palette = svg("themes"),
        label = esc(label),
        detail = esc(detail),
        chips = chips,
        key = esc(key),
        ck_custom = if mode == "custom" { " checked" } else { "" },
        custom_swatch = esc(custom_swatch),
    )
}

/// Collapse a `colour_row`'s two inputs back into the single stored value.
fn colour_from_form(form: &HashMap<String, String>, key: &str) -> Option<String> {
    let mode = form.get(&format!("{key}_mode")).map(String::as_str).unwrap_or("");
    match mode {
        "custom" => form
            .get(&format!("{key}_custom"))
            .map(|v| v.trim().to_lowercase())
            .filter(|v| ghost_mail::newsletter::is_hex_color(v)),
        // The "unset" option, which the nullable colour columns store as NULL
        // and the renderer reads as "derive it".
        "" => None,
        m => Some(m.to_string()),
    }
}

/// The per-newsletter design editor. Every setting Ghost exposes for a
/// newsletter is here, grouped the way Ghost groups them.
async fn newsletter_edit_page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return settings_flash("newsletters", "error", "That newsletter no longer exists.");
    };
    let subscribers = repos::newsletters::subscriber_count(&state.db, &n.id).await;

    // The site accent colour, for the swatch chips that preview it. Ghost's
    // own fallback when a site never set one is the dark ink.
    let accent = state
        .settings
        .get_str("accent_color")
        .filter(|c| ghost_mail::newsletter::is_hex_color(c))
        .unwrap_or_else(|| "#15212A".into());
    let accent_chip: (&str, &str, &str) = ("accent", "Accent", &accent);
    let contrast_chip: (&str, &str, &str) = ("", "Contrast", "");

    // A settings row: leading tinted icon, title over a supporting line, and
    // the control in the trailing slot — the Material list item the whole
    // editor is built from.
    let setting = |tint: &str, icon: &str, title: &str, hint: &str, control: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{hint}</span></div>
<div class="trailing">{control}</div></div>"#,
            tint = tint,
            ic = svg(icon),
            title = esc(title),
            hint = esc(hint),
            control = control,
        )
    };

    let body = format!(
        r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/newsletters/" title="Back to newsletters">{back}</a>
<div><h1>{name}</h1><p class="sub">{subscribers} subscriber{plural} · {status}</p></div></header>
{banner}
<form method="post" action="/ghost/settings/newsletters/{id}/">
<div class="settings">

<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{title_ic}</span>
<div class="body" style="width:100%">{name_field}{desc_field}</div></div>
{subscribe_on_signup}
{visibility_row}
{status_row}
</div>

<div class="section">Sender</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{send_ic}</span>
<div class="body" style="width:100%"><div class="grid2">{sender_name}{sender_email}</div>
<p class="muted" style="margin:8px 0 0">Leave the name empty to send as the site title, and the address empty to use the one from <a href="/ghost/settings/email/">Settings → Email</a>.</p></div></div>
{reply_to}
</div>

<div class="section">Header</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{image_ic}</span>
<div class="body" style="width:100%">{header_image}
<p class="muted" style="margin:8px 0 0">A banner across the top of every email. Use a full URL or an uploaded image path.</p></div></div>
{show_header_icon}
{show_header_title}
{show_header_name}
{header_background}
</div>

<div class="section">Post title</div>
<div class="card rows rise" style="--i:4">
{show_post_title_section}
{title_font}
{title_weight}
{title_alignment}
{post_title_colour}
{show_excerpt}
{show_feature_image}
</div>

<div class="section">Body</div>
<div class="card rows rise" style="--i:5">
{body_font}
{background_colour}
{section_title_colour}
{divider_colour}
{image_corners}
</div>

<div class="section">Links &amp; buttons</div>
<div class="card rows rise" style="--i:6">
{link_colour}
{link_style}
{button_colour}
{button_style}
{button_corners}
</div>

<div class="section">Footer</div>
<div class="card rows rise" style="--i:7">
<div class="row" style="align-items:flex-start"><span class="ic tint-slate">{notes_ic}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="footer_content" rows="3" placeholder=" ">{footer_content}</textarea><span>Footer · HTML, shown above the unsubscribe line</span></label></div></div>
{show_badge}
{show_latest_posts}
{show_comment_cta}
{show_share_button}
{feedback_enabled}
{show_subscription_details}
</div>

</div>
<div style="margin:18px 4px 8px"><button class="btn" type="submit">Save newsletter</button></div>
</form>

<div class="section">Preview</div>
<div class="card padded rise" style="--i:8">
<p class="muted" style="margin:0 0 12px">Rendered with the saved settings, using your most recent post. Save to see a change here.</p>
<iframe src="/ghost/settings/newsletters/{id}/preview/" title="Newsletter preview" style="width:100%;height:760px;border:1px solid var(--outline-variant);border-radius:16px;background:#fff"></iframe>
</div>"#,
        back = svg("chevron"),
        name = esc(&n.name),
        subscribers = subscribers,
        plural = if subscribers == 1 { "" } else { "s" },
        status = esc(&n.status),
        banner = flash_banner(&q),
        id = esc(&n.id),
        title_ic = svg("title"),
        send_ic = svg("send"),
        image_ic = svg("upload"),
        notes_ic = svg("notes"),
        name_field = text_field("name", "Newsletter name", &n.name),
        desc_field = format!(
            r#"<label class="field" style="margin:12px 0 0"><textarea name="description" rows="2" placeholder=" ">{}</textarea><span>Description · shown in the signup portal</span></label>"#,
            esc(n.description.as_deref().unwrap_or(""))
        ),
        subscribe_on_signup = toggle_row(
            "subscribe_on_signup",
            "Subscribe new members",
            "New members are added to this newsletter when they sign up",
            n.subscribe_on_signup
        ),
        // Ghost's newsletter `visibility`: who may subscribe to it at all.
        visibility_row = setting(
            "purple",
            "ghost",
            "Visibility",
            "Who is offered this newsletter in the signup portal",
            &segmented(
                "visibility",
                &[("members", "All members"), ("paid", "Paid-members only")],
                &n.visibility,
            ),
        ),
        status_row = setting(
            "purple",
            "flag",
            "Status",
            "Archived newsletters are hidden from the signup portal",
            &segmented("status", &[("active", "Active"), ("archived", "Archived")], &n.status),
        ),
        sender_name = text_field("sender_name", "Sender name", n.sender_name.as_deref().unwrap_or("")),
        sender_email = text_field("sender_email", "Sender email", n.sender_email.as_deref().unwrap_or("")),
        reply_to = setting(
            "teal",
            "mail",
            "Replies go to",
            "The address readers' replies are delivered to",
            &segmented(
                "sender_reply_to",
                &[("newsletter", "Newsletter address"), ("support", "Support address")],
                &n.sender_reply_to,
            ),
        ),
        header_image = text_field("header_image", "Header image URL", n.header_image.as_deref().unwrap_or("")),
        show_header_icon = toggle_row(
            "show_header_icon",
            "Publication icon",
            "Show the site icon at the top of the email",
            n.show_header_icon
        ),
        show_header_title = toggle_row(
            "show_header_title",
            "Publication title",
            "Show the site title at the top of the email",
            n.show_header_title
        ),
        show_header_name = toggle_row(
            "show_header_name",
            "Newsletter name",
            "Show this newsletter's name under the title",
            n.show_header_name
        ),
        header_background = colour_row(
            "header_background_color",
            "Header background",
            "A colour behind the header and post title.",
            &[
                ("transparent", "Transparent", "none"),
                ("accent", "Accent", &accent),
            ],
            Some(&n.header_background_color),
            "transparent",
        ),
        show_post_title_section = toggle_row(
            "show_post_title_section",
            "Post title",
            "Show the title, byline and date above the content",
            n.show_post_title_section
        ),
        title_font = setting(
            "amber",
            "title",
            "Title font",
            "The typeface of the post title",
            &segmented(
                "title_font_category",
                &[("sans_serif", "Sans-serif"), ("serif", "Serif")],
                &n.title_font_category,
            ),
        ),
        title_weight = setting(
            "amber",
            "title",
            "Title weight",
            "How heavy the post title's strokes are",
            &selbox(
                "title_font_weight",
                "Title weight",
                &[
                    ("normal", "Normal"),
                    ("medium", "Medium"),
                    ("semibold", "Semibold"),
                    ("bold", "Bold"),
                ],
                &n.title_font_weight,
            ),
        ),
        title_alignment = setting(
            "amber",
            "title",
            "Title alignment",
            "Centered under the header, or to the left",
            &segmented("title_alignment", &[("center", "Centered"), ("left", "Left")], &n.title_alignment),
        ),
        post_title_colour = colour_row(
            "post_title_color",
            "Post title colour",
            "Automatic picks black or white, whichever reads on the background.",
            &[("", "Automatic", ""), accent_chip],
            n.post_title_color.as_deref(),
            "",
        ),
        show_excerpt = toggle_row(
            "show_excerpt",
            "Excerpt",
            "Show the post's custom excerpt under the title",
            n.show_excerpt
        ),
        show_feature_image = toggle_row(
            "show_feature_image",
            "Feature image",
            "Show the post's feature image",
            n.show_feature_image
        ),
        body_font = setting(
            "green",
            "notes",
            "Body font",
            "The typeface of the post content",
            &segmented(
                "body_font_category",
                &[("sans_serif", "Sans-serif"), ("serif", "Serif")],
                &n.body_font_category,
            ),
        ),
        background_colour = colour_row(
            "background_color",
            "Background",
            "Light is white. A dark colour flips the text and borders to suit it.",
            &[("light", "Light", "#ffffff")],
            Some(&n.background_color),
            "light",
        ),
        section_title_colour = colour_row(
            "section_title_color",
            "Heading colour",
            "Applies to headings inside the post content.",
            &[("", "Automatic", ""), accent_chip],
            n.section_title_color.as_deref(),
            "",
        ),
        divider_colour = colour_row(
            "divider_color",
            "Divider colour",
            "Horizontal rules and the lines between sections.",
            &[("", "Default grey", "#e0e7eb"), accent_chip],
            n.divider_color.as_deref(),
            "",
        ),
        image_corners = setting(
            "green",
            "upload",
            "Image corners",
            "The corners of images inside the post",
            &segmented("image_corners", &[("square", "Square"), ("rounded", "Rounded")], &n.image_corners),
        ),
        link_colour = colour_row(
            "link_color",
            "Link colour",
            "Contrast uses black or white, whichever reads on the background.",
            &[accent_chip, contrast_chip],
            n.link_color.as_deref(),
            "accent",
        ),
        link_style = setting(
            "pink",
            "link",
            "Link style",
            "How links stand out from the surrounding text",
            &segmented(
                "link_style",
                &[("underline", "Underlined"), ("regular", "Regular"), ("bold", "Bold")],
                &n.link_style,
            ),
        ),
        button_colour = colour_row(
            "button_color",
            "Button colour",
            "Used by button and CTA cards in the post content.",
            &[accent_chip, contrast_chip],
            n.button_color.as_deref(),
            "accent",
        ),
        button_style = setting(
            "pink",
            "themes",
            "Button style",
            "Filled with the button colour, or outlined by it",
            &segmented("button_style", &[("fill", "Filled"), ("outline", "Outlined")], &n.button_style),
        ),
        button_corners = setting(
            "pink",
            "themes",
            "Button corners",
            "The corners of buttons in the post content",
            &segmented(
                "button_corners",
                &[("square", "Square"), ("rounded", "Rounded"), ("pill", "Pill")],
                &n.button_corners,
            ),
        ),
        footer_content = esc(n.footer_content.as_deref().unwrap_or("")),
        show_badge = toggle_row(
            "show_badge",
            "Powered by Ghost badge",
            "Show the badge at the very bottom of the email",
            n.show_badge
        ),
        show_latest_posts = toggle_row(
            "show_latest_posts",
            "Latest posts",
            "Add a \"Keep reading\" list of the three most recent posts",
            n.show_latest_posts
        ),
        show_comment_cta = toggle_row(
            "show_comment_cta",
            "Comments link",
            "Invite readers to comment on the post",
            n.show_comment_cta
        ),
        show_share_button = toggle_row(
            "show_share_button",
            "Share button",
            "Let readers share the post",
            n.show_share_button
        ),
        feedback_enabled = toggle_row(
            "feedback_enabled",
            "Audience feedback",
            "Add \"More like this\" and \"Less like this\" buttons",
            n.feedback_enabled
        ),
        show_subscription_details = toggle_row(
            "show_subscription_details",
            "Subscription details",
            "Print each member's own subscription details in the footer",
            n.show_subscription_details
        ),
    );

    page_for_role(actor_role, &n.name, "settings", Some(&user), &body, true).into_response()
}

async fn newsletter_edit_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(mut n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return settings_flash("newsletters", "error", "That newsletter no longer exists.");
    };

    let get = |k: &str| form.get(k).cloned().unwrap_or_default().trim().to_string();
    let keep = |k: &str| Some(get(k)).filter(|v| !v.is_empty());
    let on = |k: &str| form.get(k).map(|v| v == "1").unwrap_or(false);
    // An unrecognised value is never written: these columns are enums in
    // Ghost, and the renderer's `match` arms assume it.
    let one_of = |k: &str, allowed: &[&str], fallback: &str| {
        let v = get(k);
        if allowed.contains(&v.as_str()) { v } else { fallback.to_string() }
    };

    let name = get("name");
    if name.is_empty() {
        return newsletter_flash(&id, "error", "A newsletter needs a name.");
    }
    n.name = name;
    n.description = keep("description");
    n.sender_name = keep("sender_name");
    n.sender_email = keep("sender_email");
    n.sender_reply_to = one_of("sender_reply_to", &["newsletter", "support"], "newsletter");
    n.status = one_of("status", &["active", "archived"], "active");
    n.visibility = one_of("visibility", &["members", "paid"], "members");
    n.subscribe_on_signup = on("subscribe_on_signup");

    n.header_image = keep("header_image");
    n.show_header_icon = on("show_header_icon");
    n.show_header_title = on("show_header_title");
    n.show_header_name = on("show_header_name");
    n.show_post_title_section = on("show_post_title_section");
    n.show_excerpt = on("show_excerpt");
    n.show_feature_image = on("show_feature_image");

    n.title_font_category = one_of("title_font_category", &["serif", "sans_serif"], "sans_serif");
    n.title_alignment = one_of("title_alignment", &["center", "left"], "center");
    n.title_font_weight = one_of(
        "title_font_weight",
        &["normal", "medium", "semibold", "bold"],
        "bold",
    );
    n.body_font_category = one_of("body_font_category", &["serif", "sans_serif"], "sans_serif");

    // The two NOT NULL colour columns keep their sentinel when nothing else
    // was chosen; the nullable ones are allowed to go back to NULL.
    n.background_color = colour_from_form(&form, "background_color").unwrap_or_else(|| "light".into());
    n.header_background_color =
        colour_from_form(&form, "header_background_color").unwrap_or_else(|| "transparent".into());
    n.post_title_color = colour_from_form(&form, "post_title_color");
    n.section_title_color = colour_from_form(&form, "section_title_color");
    n.divider_color = colour_from_form(&form, "divider_color");
    n.link_color = colour_from_form(&form, "link_color");
    n.button_color = colour_from_form(&form, "button_color");

    n.link_style = one_of("link_style", &["underline", "regular", "bold"], "underline");
    n.button_style = one_of("button_style", &["fill", "outline"], "fill");
    n.button_corners = one_of("button_corners", &["square", "rounded", "pill"], "rounded");
    n.image_corners = one_of("image_corners", &["square", "rounded"], "square");

    n.footer_content = keep("footer_content");
    n.show_badge = on("show_badge");
    n.show_latest_posts = on("show_latest_posts");
    n.show_comment_cta = on("show_comment_cta");
    n.show_share_button = on("show_share_button");
    n.feedback_enabled = on("feedback_enabled");
    n.show_subscription_details = on("show_subscription_details");

    match repos::newsletters::update(&state.db, &n).await {
        Ok(()) => newsletter_flash(&id, "ok", "Newsletter saved."),
        Err(e) => newsletter_flash(&id, "error", &e.message()),
    }
}

async fn newsletter_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let name = form.get("name").cloned().unwrap_or_default();
    match repos::newsletters::create(&state.db, &name).await {
        Ok(n) => Redirect::to(&format!("/ghost/settings/newsletters/{}/", n.id)).into_response(),
        Err(e) => settings_flash("newsletters", "error", &e.message()),
    }
}

/// The design editor's preview pane: the real renderer, on a real post.
///
/// Rendering the actual template rather than an approximation is the point —
/// a preview that agreed with a mock-up instead of the email would be worse
/// than none.
async fn newsletter_preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(n)) = repos::newsletters::find_by_id(&state.db, &id).await else {
        return (StatusCode::NOT_FOUND, "No such newsletter").into_response();
    };
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    // The most recent published post, so the preview shows real content and
    // real card styling. A brand-new site has none, hence the placeholder.
    let sample_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE status IN ('published', 'sent') AND type = 'post' \
         ORDER BY published_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let sample = match sample_id {
        Some(id) => repos::posts::find_by_id(&state.db, &id).await.ok().flatten(),
        None => None,
    };

    let (post, post_title, html, plaintext, feature_image, published_at) = match sample {
        Some(p) => {
            let title = p.title.clone().unwrap_or_else(|| "Untitled".into());
            let html = p.html.clone().unwrap_or_default();
            let plaintext = p.plaintext.clone().unwrap_or_default();
            let feature_image = p.feature_image.clone();
            let published_at = p.published_at.clone().unwrap_or_default();
            (p, title, html, plaintext, feature_image, published_at)
        }
        None => (
            ghost_core::models::Post::default(),
            "Your next newsletter".to_string(),
            "<p>Publish a post and its content will appear here, styled exactly as subscribers will see it.</p>".to_string(),
            "Publish a post and its content will appear here.".to_string(),
            None,
            ghost_core::dates::now_db(),
        ),
    };
    // The permalink goes through the router for the same reason the send path
    // does it: a dated or prefixed structure is not /{slug}/.
    let post_json = ghost_core::models::post_to_json(&post, &[], &[], "");
    let router = state.router.read().await.clone();
    let path = match router.permalink_for(&post_json) {
        Some((_, p)) => ghost_routing::tree::build_permalink(&p, &post_json),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let post_url = format!("{site_url}{path}");

    let site_ctx = ghost_mail::newsletter::SiteContext {
        title: title.clone(),
        url: site_url.clone(),
        icon_url: state
            .settings
            .get_str("icon")
            .filter(|i| !i.is_empty())
            .map(|i| ghost_core::url::absolute(&site_url, &ghost_core::url::base_asset(&i))),
        locale: state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
        direction: "ltr".into(),
        accent_color: state.settings.get_str("accent_color").unwrap_or_default(),
    };
    let post_ctx = ghost_mail::newsletter::PostContext {
        id: post.id.clone(),
        title: post_title,
        url: post_url,
        html: ghost_core::url::absolutize_html(&site_url, &html),
        plaintext,
        custom_excerpt: post.custom_excerpt.clone().filter(|e| !e.is_empty()),
        published_at: ghost_core::dates::short_date_in_zone(&published_at, &timezone),
        authors: title.clone(),
        feature_image,
        email_only: post.status == "sent",
        ..Default::default()
    };

    let rendered = ghost_mail::newsletter::NewsletterEmail {
        site: &site_ctx,
        newsletter: &n,
        post: &post_ctx,
        latest_posts: &[],
        comments_enabled: state
            .settings
            .get_str("comments_enabled")
            .map(|v| v != "off")
            .unwrap_or(false),
    };
    let html = match rendered.render() {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.message()).into_response(),
    };
    // The preview is not addressed to anybody, so the member tokens get
    // sample values rather than being left as `%%{…}%%` literals.
    let preview_member = ghost_mail::newsletter::Recipient {
        uuid: "preview".into(),
        email: "subscriber@example.com".into(),
        name: Some("Subscriber".into()),
        status: "free".into(),
        created_at: ghost_core::dates::short_date_in_zone(&ghost_core::dates::now_db(), &timezone),
        unsubscribe_url: "#".into(),
        manage_account_url: "#".into(),
        key: "preview".into(),
    };
    Html(ghost_mail::newsletter::personalize(&html, &preview_member)).into_response()
}

async fn newsletters_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "newsletters").unwrap();
    let base = state
        .settings
        .get_str("mailgun_base_url")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.mailgun.net/v3".into());
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_url = state.site_url();
    let mail_cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let configured = ghost_mail::MailgunConfig::from_settings(&state.settings, &mail_cfg).is_some();
    let status = if configured {
        format!(
            r#"<div class="banner ok rise" style="--i:1">{check}<span class="msg">Mailgun is configured for <code>{domain}</code></span></div>"#,
            check = svg("check"),
            domain = s("mailgun_domain"),
        )
    } else {
        format!(
            r#"<div class="banner info rise" style="--i:1">{send}Not configured. Newsletters need a bulk provider — Mailgun is what Ghost uses, because sending thousands of messages through a normal SMTP relay gets a domain blocked.</div>"#,
            send = svg("send"),
        )
    };

    // The newsletters themselves. Each row opens the design editor, which is
    // where every setting that shapes the email lives.
    let mut newsletter_rows = String::new();
    match repos::newsletters::list(&state.db).await {
        Ok(list) if !list.is_empty() => {
            for nl in &list {
                let count = repos::newsletters::subscriber_count(&state.db, &nl.id).await;
                newsletter_rows.push_str(&format!(
                    r#"<a class="row" href="/ghost/settings/newsletters/{id}/"><span class="ic tint-purple">{mail}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{count} subscriber{plural}{desc}</span></div>
<div class="trailing"><span class="chip {chip}">{status}</span>{chev}</div></a>"#,
                    id = esc(&nl.id),
                    mail = svg("mail"),
                    name = esc(&nl.name),
                    count = count,
                    plural = if count == 1 { "" } else { "s" },
                    desc = match nl.description.as_deref().filter(|d| !d.is_empty()) {
                        Some(d) => format!(" · {}", esc(d)),
                        None => String::new(),
                    },
                    chip = if nl.status == "active" { "published" } else { "draft" },
                    status = esc(&nl.status),
                    chev = svg("chevron"),
                ));
            }
        }
        _ => {
            newsletter_rows = r#"<div class="row"><div class="body"><span class="muted">No newsletters yet — add one below and posts will have somewhere to go.</span></div></div>"#.into();
        }
    }

    // --- newsletter sending + default recipients (Ghost's Email area) -----
    //
    // Ghost encodes both in the same pair of settings:
    // `editor_default_email_recipients` is `disabled` when sending is off, and
    // otherwise names the default audience — with `filter` deferring to
    // `editor_default_email_recipients_filter` for the segment cases.
    let recipients = state
        .settings
        .get_str("editor_default_email_recipients")
        .unwrap_or_else(|| "visibility".into());
    let recipients_filter = state.settings.get_str("editor_default_email_recipients_filter");
    let sending_on = recipients != "disabled";
    // With nobody able to sign up there is no one to email, so Ghost greys the
    // switch out and points at the setting that is really in the way.
    let signup_closed = state.settings.get_str("members_signup_access").as_deref() == Some("none");

    let sending_card = format!(
        r#"<div class="section">Newsletter sending</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-purple">{mail}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="newsletter_sending" form="newsletters-form" value="1"{ck}{dis}><span class="track"><span class="thumb"></span></span></label></div></div>
</div>
{note}"#,
        mail = svg("mail"),
        title = if sending_on && !signup_closed { "Enabled" } else { "Disabled" },
        detail = if sending_on && !signup_closed {
            "Newsletter features are active, posts can be sent by email"
        } else {
            "Posts cannot be sent by email while this is off"
        },
        ck = if sending_on && !signup_closed { " checked" } else { "" },
        dis = if signup_closed { " disabled" } else { "" },
        note = if signup_closed {
            r#"<p class="muted" style="margin:10px 2px 0">Your <a href="/ghost/settings/membership/">subscription access</a> is set to
&ldquo;Nobody can sign up&rdquo;, so there is no audience to email. Allow signups to turn newsletter sending back on.</p>"#
        } else {
            ""
        },
    );

    // The five audiences Ghost offers, flattened from the two settings.
    let recipients_value = match (recipients.as_str(), recipients_filter.as_deref()) {
        ("filter", Some("status:free,status:-free")) => "all-members",
        ("filter", Some("status:-free")) => "paid-only",
        ("filter", None) | ("filter", Some("")) => "none",
        ("filter", _) => "segment",
        (other, _) => other,
    };
    let recipients_card = if sending_on && !signup_closed {
        format!(
            r#"<div class="section">Default recipients</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{people}</span>
<div class="body" style="width:100%">
<label class="field fixed" style="margin:0;max-width:360px"><select name="editor_default_email_recipients" form="newsletters-form">{opts}</select><span>Who gets a new post by default</span>{car}</label>
<p class="muted" style="margin:8px 0 0">Set per post in the editor; this only picks the starting point. {segment_note}</p>
</div></div>
</div>"#,
            people = svg("ghost"),
            car = SELECT_CARET,
            opts = select_options(
                &{
                    let mut o = vec![
                        ("visibility", "Whoever has access to the post"),
                        ("all-members", "All members"),
                        ("paid-only", "Paid-members only"),
                        ("none", "Usually nobody"),
                    ];
                    // Only offered when it is already in use, so the saved
                    // segment stays selectable instead of the select falling
                    // back to its first option.
                    if recipients_value == "segment" {
                        o.push(("segment", "Specific people"));
                    }
                    o
                },
                recipients_value,
            ),
            // "Specific people" is a tier/label filter Ghost builds with a
            // picker this page has no equivalent of, so an existing one is
            // reported and preserved rather than silently rewritten.
            segment_note = if recipients_value == "segment" {
                format!(
                    "This site currently sends to a saved segment (<code>{}</code>); choosing another option replaces it.",
                    esc(recipients_filter.as_deref().unwrap_or("")),
                )
            } else {
                String::new()
            },
        )
    } else {
        String::new()
    };

    page_for_role(actor_role,
        "Newsletters",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}{status}
{sending_card}
{recipients_card}
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:2">{newsletter_rows}</div>
<div class="card padded rise" style="--i:3"><form method="post" action="/ghost/settings/newsletters/new/" class="inline-form">
<div style="flex:1;min-width:220px">{new_name}</div>
<button class="btn tonal" type="submit">{add} Add newsletter</button>
</form>
<p class="muted" style="margin:10px 0 0">A new newsletter starts from Ghost's defaults. Open it to set the design — colours, typography, header and footer.</p></div>
<form method="post" action="/ghost/settings/newsletters/" id="newsletters-form">
<div class="section">Mailgun</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{send}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="mailgun_api_key" type="password" value="" placeholder=" " autocomplete="off"><span>API key</span></label>
<label class="field" style="margin:0"><input name="mailgun_webhook_signing_key" type="password" value="" placeholder=" " autocomplete="off"><span>Webhook signing key</span></label>
<label class="field" style="margin:0"><input name="mailgun_domain" value="{domain}" placeholder=" " autocomplete="off"><span>Sending domain · e.g. mg.example.com</span></label>
<label class="field fixed" style="margin:0"><select name="mailgun_base_url">{region_opts}</select><span>Region</span>{car}</label>
<label class="field" style="margin:0"><input name="bulk_email_from_address" value="{bulk_from}" placeholder=" "><span>From address · defaults to the transactional one</span></label>
</div>
<p class="muted" style="margin:10px 0 0">{key_hint} Use the <strong>Sending</strong> API key, not the private account key. The region must match where the domain was created — an EU domain on the US endpoint fails with a 401 that looks like a bad key.</p>
</div></div>
</div>
</form>
<div class="banner info rise" style="--i:4">{bulb}Newsletters are sent from the editor: choose <b>Publish and email</b> when publishing a post. Without Mailgun, a configured SMTP relay is used instead, one message per subscriber.</div>"#,
            head = group_head(group, Some("newsletters-form")),
            banner = flash_banner(&q),
            status = status,
            sending_card = sending_card,
            recipients_card = recipients_card,
            send = svg("send"),
            bulb = svg("mail"),
            car = SELECT_CARET,
            newsletter_rows = newsletter_rows,
            new_name = text_field("name", "Newsletter name", ""),
            add = svg("add"),
            key_hint = secret_state(&state, "mailgun_api_key"),
            domain = s("mailgun_domain"),
            region_opts = select_options(
                &[
                    ("https://api.mailgun.net/v3", "US · api.mailgun.net"),
                    ("https://api.eu.mailgun.net/v3", "EU · api.eu.mailgun.net"),
                ],
                &base
            ),
            bulk_from = s("bulk_email_from_address"),
        ),
        true,
    )
    .into_response()
}

async fn newsletters_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in ["mailgun_domain", "mailgun_base_url", "bulk_email_from_address"] {
        let value = form.get(key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    ok &= save_secret(&state, "mailgun_api_key", form.get("mailgun_api_key")).await;
    ok &= save_secret(&state, "mailgun_webhook_signing_key", form.get("mailgun_webhook_signing_key")).await;

    // Newsletter sending and the default audience share two settings, the way
    // Ghost encodes them: `disabled` turns sending off, anything else names
    // the default audience, and the segment cases defer to the filter.
    let sending_on = form.get("newsletter_sending").map(|v| v == "1").unwrap_or(false);
    let (recipients, filter): (&str, Option<&str>) = if !sending_on {
        ("disabled", None)
    } else {
        match form.get("editor_default_email_recipients").map(String::as_str) {
            Some("all-members") => ("filter", Some("status:free,status:-free")),
            Some("paid-only") => ("filter", Some("status:-free")),
            Some("none") => ("filter", None),
            // An untouched saved segment: leave both settings exactly as they
            // are rather than flattening it to something this page can name.
            Some("segment") => ("", None),
            _ => ("visibility", None),
        }
    };
    if !recipients.is_empty() {
        ok &= state
            .settings
            .set(&state.db, "editor_default_email_recipients", &serde_json::json!(recipients))
            .await
            .is_ok();
        ok &= state
            .settings
            .set(
                &state.db,
                "editor_default_email_recipients_filter",
                &match filter {
                    Some(f) => serde_json::json!(f),
                    None => serde_json::Value::Null,
                },
            )
            .await
            .is_ok();
    }

    if ok {
        settings_flash("newsletters", "ok", "Bulk email settings saved.")
    } else {
        settings_flash("newsletters", "error", "Saving failed — check the logs.")
    }
}

// --- social accounts ---

/// The Social accounts section, for the same form as everything else on
/// General settings.
fn social_sections(state: &AppState) -> String {
    let rows: String = SOCIAL_ACCOUNTS
        .iter()
        .map(|(key, label)| {
            let value = state.settings.get_str(key).unwrap_or_default();
            format!(
                r#"<div class="row" style="align-items:center"><span class="ic tint-blue">{link}</span>
<div class="body"><label class="field" style="margin:0"><input name="{key}" value="{value}" placeholder=" "><span>{label}</span></label></div></div>"#,
                link = svg("link"),
                key = esc(key),
                value = esc(&value),
                label = esc(label),
            )
        })
        .collect();
    format!(
        r#"<div class="section">Social accounts</div>
<div class="card rows rise" style="--i:7">{rows}</div>
<p class="muted" style="margin:4px 0 0 16px">Full profile URLs, e.g. <code>https://x.com/ghost</code>. Themes expose these as <code>@site.facebook</code> and friends.</p>"#,
        rows = rows,
    )
}

// --- analytics: web analytics, outbound link tagging (Ghost's `analytics`
// settings group is exactly these two keys) ---

async fn analytics_settings_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let toggle = |key: &str, title: &str, detail: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-green">{chart}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="{key}" value="1" {ck}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            chart = svg("chart"),
            title = esc(title),
            detail = esc(detail),
            key = esc(key),
            ck = if state.settings.get_bool(key) { "checked" } else { "" },
        )
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "analytics").unwrap();
    page_for_role(actor_role,
        "Analytics",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/analytics/" id="analytics-form">
<div class="section">Analytics</div>
<div class="card rows rise" style="--i:1">
{web}{opens}{clicks}{sources}{outbound}
</div>
<p class="muted" style="margin:10px 2px 0">Email tracking needs clicks and opens to be recorded before a post's
analytics can attribute them. Turning opens off also stops click tracking.</p>
</form>"#,
            head = group_head(group, Some("analytics-form")),
            banner = flash_banner(&q),
            // Same switches, wording and order as Ghost's Analytics panel.
            web = toggle("web_analytics", "Web analytics", "Cookie-free, first party traffic analytics for your site"),
            opens = toggle("email_track_opens", "Email opens", "Record when a member opens an email"),
            clicks = toggle("email_track_clicks", "Email clicks", "Record when a member clicks on any link in an email"),
            sources = toggle("members_track_sources", "Member sources", "Track the traffic sources and posts that drive the most member growth"),
            outbound = toggle("outbound_link_tagging", "Outbound link tagging", "Make it easier for other sites to track the traffic you send them in their analytics"),
        ),
        true,
    )
    .into_response()
}

async fn analytics_settings_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = |key: &str| form.get(key).map(|v| v == "1").unwrap_or(false);
    // Ghost disables the clicks switch while opens are off, because a click is
    // only attributable to a member whose open was recorded.
    let track_opens = on("email_track_opens");
    let mut ok = true;
    for (key, value) in [
        ("web_analytics", on("web_analytics")),
        ("email_track_opens", track_opens),
        ("email_track_clicks", track_opens && on("email_track_clicks")),
        ("members_track_sources", on("members_track_sources")),
        ("outbound_link_tagging", on("outbound_link_tagging")),
    ] {
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(value)).await.is_ok();
    }
    if ok {
        settings_flash("analytics", "ok", "Analytics settings saved.")
    } else {
        settings_flash("analytics", "error", "Saving failed — check the logs.")
    }
}

// --- features: the system switches for Comments and Automations ---

async fn features_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "features").unwrap();
    let moderation = state.settings.get_str("comments_moderation").unwrap_or_else(|| "immediate".into());
    // get_str + "false" rather than get_bool: a missing row must read as on,
    // and get_bool calls missing false
    let automations_on = state.settings.get_str("automations_enabled").as_deref() != Some("false");

    // The feature flags live here rather than under Advanced: they are
    // features, and Advanced is where a site's plumbing lives — import,
    // redirects, routes. One page, one Save, so the flags post as part of the
    // same form as the switches above them.
    let flags = labs_flags(&state);
    let beta_rows = labs_flag_rows(&flags, BETA_FLAGS, "features-form");

    // Said plainly, once: a switch here sets the same flag Ghost would set,
    // and your theme can read it — but none of this port's own features are
    // gated on one yet, so turning a flag on will not by itself change how
    // the admin or the site behaves. Better stated than implied.
    let honoured_note = "Each switch sets the flag Ghost stores under the same name, and your theme can read any of \
         them as <code>{{#if labs.flagName}}</code>. None of this port's own features are gated on a \
         flag yet, so turning one on changes what your theme sees rather than how the admin behaves.";

    // Ghost keeps these behind `enableDeveloperExperiments`; the equivalent
    // here is running in development, so a production site is not offered
    // switches whose features are unfinished by Ghost's own account.
    let private_section = if state.config.env == "development" {
        format!(
            r#"<div class="section">Private features</div>
<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted" style="margin:10px 2px 0">Ghost hides these unless developer experiments are switched on, and shows them here
because this site is running in development.</p>"#,
            rows = labs_flag_rows(&flags, PRIVATE_FLAGS, "features-form"),
        )
    } else {
        String::new()
    };

    page_for_role(actor_role,
        "Features",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/features/" id="features-form">
<div class="section">Comments</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-purple">{comment}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:340px">
<select name="comments_moderation">{mod_opts}</select><span>New comments</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Held comments arrive hidden and wait in the moderation screen's Hidden filter — restoring one publishes it.</p></div></div>
</div>
<div class="section">Automations</div>
<div class="card rows rise" style="--i:2">
{automations_toggle}
</div>
</form>
<div class="section">Beta features</div>
<div class="card rows rise" style="--i:3">{beta_rows}</div>
<p class="muted" style="margin:10px 2px 0">A testing ground for new or experimental features. They may change, break or
inexplicably disappear at any time. {honoured}</p>
{private_section}"#,
            head = group_head(group, Some("features-form")),
            banner = flash_banner(&q),
            comment = svg("comment"),
            car = SELECT_CARET,
            mod_opts = crate::select_options(
                &[("immediate", "Publish immediately"), ("review", "Hold for review")],
                &moderation,
            ),
            automations_toggle = toggle_row(
                "automations_enabled",
                "Automations",
                "Runs the email sequences that start when members join. Stopping the system pauses every automation — nothing is enrolled, nothing is sent, and runs in flight wait exactly where they are.",
                automations_on,
            ),
            beta_rows = beta_rows,
            honoured = honoured_note,
            private_section = private_section,
        ),
        true,
    )
    .into_response()
}

/// `shown` repeats once per feature flag on the page, so the body is read as
/// pairs; the map is only for the fields that appear once.
async fn features_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let form: HashMap<String, String> = pairs.iter().cloned().collect();
    let moderation = form.get("comments_moderation").map(String::as_str).unwrap_or("immediate");
    if !matches!(moderation, "immediate" | "review") {
        return settings_flash("features", "error", "New comments must publish immediately or be held for review.");
    }
    let automations_on = form.get("automations_enabled").map(|v| v == "1").unwrap_or(false);
    let saved_moderation = state
        .settings
        .set(&state.db, "comments_moderation", &serde_json::json!(moderation))
        .await;
    let saved_automations = state
        .settings
        .set(&state.db, "automations_enabled", &serde_json::Value::Bool(automations_on))
        .await;

    // Only the flags this page actually rendered are touched: on a production
    // site the private ones are never shown, and reading their absence as
    // "off" would clear flags the publisher never saw.
    let shown: Vec<&str> =
        pairs.iter().filter(|(k, _)| k == "shown").map(|(_, v)| v.as_str()).collect();
    let mut flags = labs_flags(&state);
    for key in shown {
        // A flag that is off is removed rather than stored as `false`, which
        // is the shape Ghost's own object takes.
        if pairs.iter().any(|(k, v)| k == key && v == "1") {
            flags.insert(key.to_string(), serde_json::Value::Bool(true));
        } else {
            flags.remove(key);
        }
    }
    let saved_flags =
        state.settings.set(&state.db, "labs", &serde_json::Value::Object(flags)).await;

    // Turning the system back on should not wait for the next tick to notice
    if automations_on {
        ghost_api::automations_engine::request_poll();
    }
    if saved_moderation.is_ok() && saved_automations.is_ok() && saved_flags.is_ok() {
        settings_flash("features", "ok", "Feature settings saved.")
    } else {
        settings_flash("features", "error", "Saving failed — check the logs.")
    }
}

// --- site: theme (the existing design page) + announcement bar ---

/// Ghost's announcement visibility audiences (AnnouncementVisibilityValues).
const ANNOUNCEMENT_VISIBILITIES: &[(&str, &str)] = &[
    ("visitors", "Visitors"),
    ("free_members", "Free members"),
    ("paid_members", "Paid members"),
];

/// Read `announcement_visibility` (a JSON array) into the checked set.
fn announcement_visibility_set(v: Option<serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(a)) => a
            .into_iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str::<Vec<String>>(&s).unwrap_or_default()
        }
        _ => vec![],
    }
}

async fn site_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let checked: Vec<String> = announcement_visibility_set(state.settings.get("announcement_visibility"));
    let vis_chips: String = ANNOUNCEMENT_VISIBILITIES
        .iter()
        .map(|(value, label)| {
            let ck = if checked.iter().any(|c| c == value) { "checked" } else { "" };
            format!(
                r#"<label class="fchip"><input type="checkbox" name="visibility" value="{value}" {ck}><span class="ck">{check}</span>{label}</label>"#,
                value = esc(value),
                ck = ck,
                check = svg("check"),
                label = esc(label),
            )
        })
        .collect();
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "site").unwrap();
    page_for_role(actor_role,
        "Announcement",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/site/" id="site-form">
<div class="section">Announcement bar</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{megaphone}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="announcement_content" rows="3" placeholder=" ">{content}</textarea><span>Announcement · Markdown supported</span></label>
<p class="muted" style="margin:8px 0 0">Leave empty to hide the bar. Shown at the top of every page.</p></div></div>
<div class="row"><span class="ic tint-amber">{palette}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px"><select name="announcement_background">{bg_opts}</select><span>Background</span>{car}</label></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{visibility}</span>
<div class="body"><span class="title" style="display:block;margin-bottom:8px">Visibility</span>
<div class="chips">{vis_chips}</div>
<p class="muted" style="margin:8px 0 0">At least one audience must be selected for the bar to appear. Member audiences take effect once members are available.</p></div></div>
</div>
</form>"#,
            head = group_head(group, Some("site-form")),
            banner = flash_banner(&q),
            megaphone = svg("flag"),
            palette = svg("themes"),
            visibility = svg("globe"),
            car = SELECT_CARET,
            content = s("announcement_content"),
            bg_opts = select_options(&[("dark", "Dark"), ("light", "Light"), ("accent", "Accent color")], &state.settings.get_str("announcement_background").unwrap_or_else(|| "dark".into())),
            vis_chips = vis_chips,
        ),
        true,
    )
    .into_response()
}

/// The announcement-bar form: the visibility checkboxes all post under
/// `visibility`, so this needs the same hand-rolled parsing as the editor's
/// tag checkboxes.
#[derive(Default)]
struct SiteForm {
    announcement_content: String,
    announcement_background: String,
    visibility: Vec<String>,
}

impl SiteForm {
    fn parse(body: &str) -> Self {
        let mut form = SiteForm::default();
        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match key.as_ref() {
                "announcement_content" => form.announcement_content = value.into_owned(),
                "announcement_background" => form.announcement_background = value.into_owned(),
                "visibility" => form.visibility.push(value.into_owned()),
                _ => {}
            }
        }
        form
    }
}

impl<S: Send + Sync> axum::extract::FromRequest<S> for SiteForm {
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body = String::from_utf8_lossy(&bytes);
        Ok(SiteForm::parse(&body))
    }
}

async fn site_save(State(state): State<AppState>, headers: HeaderMap, form: SiteForm) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // only Ghost's backgrounds and audiences are addressable; anything else
    // falls back to the stored value
    let background = if ["dark", "light", "accent"].contains(&form.announcement_background.as_str()) {
        form.announcement_background
    } else {
        state.settings.get_str("announcement_background").unwrap_or_else(|| "dark".into())
    };
    let visibility: Vec<String> = form
        .visibility
        .into_iter()
        .filter(|v| ANNOUNCEMENT_VISIBILITIES.iter().any(|(val, _)| val == v))
        .collect();
    let ok = state
        .settings
        .set(&state.db, "announcement_content", &serde_json::json!(form.announcement_content))
        .await
        .is_ok()
        && state.settings.set(&state.db, "announcement_background", &serde_json::json!(background)).await.is_ok()
        && state
            .settings
            .set(&state.db, "announcement_visibility", &serde_json::json!(visibility))
            .await
            .is_ok();
    if ok {
        settings_saved(&state, "site", "Site settings saved.")
    } else {
        settings_flash("site", "error", "Saving failed — check the logs.")
    }
}

// --- membership: access, tiers, signup portal, welcome emails, newsletters ---

/// Ghost's price display: cents with whole-dollar collapsing ("$5").
fn tier_price(cents: Option<i64>) -> String {
    match cents {
        None => "—".into(),
        Some(c) if c % 100 == 0 => format!("${}", c / 100),
        Some(c) => format!("${:.2}", c as f64 / 100.0),
    }
}

// --- membership tiers ---

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TierForm {
    name: String,
    description: String,
    /// Entered in whole currency units; Stripe works in the smallest unit.
    monthly_price: String,
    yearly_price: String,
    currency: String,
    welcome_page_url: String,
    trial_days: String,
    visibility: String,
    active: Option<String>,
    /// One per line.
    benefits: String,
}

fn tier_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/membership/tiers/{id}/?{encoded}")).into_response()
}

/// Parse a price typed as "5", "5.00" or "£5" into cents. Returns `None` for
/// an empty field, which means "this interval is not offered".
fn parse_price_cents(raw: &str) -> Option<i64> {
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().map(|v| (v * 100.0).round() as i64).filter(|c| *c > 0)
}

fn format_price_input(cents: Option<i64>) -> String {
    match cents {
        Some(c) if c > 0 => format!("{:.2}", c as f64 / 100.0),
        _ => String::new(),
    }
}

async fn tier_benefits(state: &AppState, tier_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT b.name FROM benefits b JOIN products_benefits pb ON pb.benefit_id = b.id \
         WHERE pb.product_id = ? ORDER BY pb.sort_order",
    )
    .bind(tier_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

/// Replace a tier's benefits with the lines given, creating any that are new.
async fn set_tier_benefits(state: &AppState, tier_id: &str, lines: &str) {
    let _ = sqlx::query("DELETE FROM products_benefits WHERE product_id = ?")
        .bind(tier_id)
        .execute(&state.db)
        .await;
    let now = ghost_core::dates::now_db();
    for (i, raw) in lines.lines().enumerate() {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM benefits WHERE name = ?")
            .bind(name)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
        let bid = match existing {
            Some(b) => b,
            None => {
                let bid = ghost_core::ids::new_id();
                // benefits.slug is unique, so suffix with the id to avoid a
                // collision between two similarly-named benefits
                let slug = format!("{}-{}", ghost_core::slug::safe_string(name), &bid[..6]);
                if sqlx::query("INSERT INTO benefits (id, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
                    .bind(&bid).bind(name).bind(&slug).bind(&now).bind(&now)
                    .execute(&state.db).await.is_err()
                {
                    continue;
                }
                bid
            }
        };
        let _ = sqlx::query("INSERT INTO products_benefits (id, product_id, benefit_id, sort_order) VALUES (?, ?, ?, ?)")
            .bind(ghost_core::ids::new_id()).bind(tier_id).bind(&bid).bind(i as i64)
            .execute(&state.db).await;
    }
}

async fn tier_create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
    let slug = format!("tier-{}", &id[..8]);
    let res = sqlx::query(
        "INSERT INTO products (id, name, slug, active, visibility, type, currency, created_at, updated_at) \
         VALUES (?, 'New tier', ?, 0, 'public', 'paid', 'usd', ?, ?)",
    )
    .bind(&id).bind(&slug).bind(&now).bind(&now)
    .execute(&state.db).await;
    match res {
        Ok(_) => Redirect::to(&format!("/ghost/settings/membership/tiers/{id}/")).into_response(),
        Err(e) => membership_flash("error", &format!("Could not create the tier: {e}")),
    }
}

type TierRowData = (String, String, Option<String>, String, bool, Option<String>, Option<i64>, Option<i64>, Option<String>, i64, Option<String>);

async fn load_tier(state: &AppState, id: &str) -> Option<TierRowData> {
    sqlx::query_as::<_, TierRowData>(
        "SELECT id, name, description, type, active, currency, monthly_price, yearly_price, \
         welcome_page_url, trial_days, visibility FROM products WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

async fn tier_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Some((tid, name, description, ttype, active, currency, monthly, yearly, welcome, trial, visibility)) =
        load_tier(&state, &id).await
    else {
        return not_found_page(&user, "That tier could not be found.");
    };
    let is_free = ttype == "free";
    let benefits = tier_benefits(&state, &tid).await.join("\n");
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();

    // A paid tier is only sellable once Stripe can take the money.
    let price_note = if is_free {
        "The free tier has no price. Its benefits are what members see when they sign up for free.".to_string()
    } else if stripe_on {
        "Saving a changed price creates a new price in Stripe. Existing subscribers stay on the price they signed up at, which is how Stripe works.".to_string()
    } else {
        "Connect Stripe on the Membership screen before setting prices — without it a paid tier cannot be sold.".to_string()
    };

    let price_fields = if is_free {
        String::new()
    } else {
        format!(
            r#"<div class="section">Pricing</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{coin}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="monthly_price" value="{monthly}" placeholder=" " inputmode="decimal"><span>Monthly price</span></label>
<label class="field" style="margin:0"><input name="yearly_price" value="{yearly}" placeholder=" " inputmode="decimal"><span>Yearly price</span></label>
<label class="field fixed" style="margin:0"><select name="currency">{cur_opts}</select><span>Currency</span>{car}</label>
<label class="field" style="margin:0"><input name="trial_days" value="{trial}" placeholder=" " inputmode="numeric"><span>Free trial · days, 0 for none</span></label>
</div>
<p class="muted" style="margin:10px 0 0">{price_note}</p>
</div></div>
</div>"#,
            coin = svg("chart"),
            monthly = esc(&format_price_input(monthly)),
            yearly = esc(&format_price_input(yearly)),
            cur_opts = select_options(
                &[("usd", "USD $"), ("eur", "EUR €"), ("gbp", "GBP £"), ("aud", "AUD $"), ("cad", "CAD $"), ("inr", "INR ₹")],
                &currency.clone().unwrap_or_else(|| "usd".into())
            ),
            car = SELECT_CARET,
            trial = trial,
            price_note = price_note,
        )
    };

    page_for_role(actor_role, 
        "Tier",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/membership/" title="Back to membership">{back}</a>
<div><h1>{name}</h1><p class="sub">{sub}</p></div>
<div class="actions"><button class="btn" type="submit" form="tier-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/settings/membership/tiers/{id}/" id="tier-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{ghost_ic}</span>
<div class="body" style="width:100%">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " required><span>Name</span></label>
<label class="field fixed" style="margin:0"><select name="visibility">{vis_opts}</select><span>Visibility</span>{car}</label>
</div>
<label class="field" style="margin:12px 0 0"><input name="description" value="{description}" placeholder=" "><span>Description · shown in Portal</span></label>
<label class="field" style="margin:12px 0 0"><input name="welcome_page_url" value="{welcome}" placeholder=" "><span>Welcome page URL · where new members land</span></label>
<label class="check" style="display:flex;align-items:center;gap:10px;margin:14px 0 0">
<label class="switch"><input type="checkbox" name="active" value="1" {act}><span class="track"><span class="thumb"></span></span></label>
<span class="muted">Active · inactive tiers are hidden from Portal</span></label>
</div></div>
</div>
{price_fields}
<div class="section">Benefits</div>
<div class="card rows rise" style="--i:3">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{check_ic}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><textarea name="benefits" rows="5" placeholder=" ">{benefits}</textarea><span>One benefit per line</span></label>
<p class="muted" style="margin:10px 0 0">Listed under the tier in Portal's signup window.</p>
</div></div>
</div>
</form>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&tid),
            ghost_ic = svg("ghost"),
            check_ic = svg("check"),
            car = SELECT_CARET,
            name = esc(&name),
            name_v = esc(&name),
            sub = if is_free { "Free tier".to_string() } else { "Paid tier".to_string() },
            description = esc(description.as_deref().unwrap_or("")),
            welcome = esc(welcome.as_deref().unwrap_or("")),
            vis_opts = select_options(&[("public", "Public"), ("none", "Hidden")], visibility.as_deref().unwrap_or("public")),
            act = if active { "checked" } else { "" },
            price_fields = price_fields,
            benefits = esc(&benefits),
        ),
        true,
    )
    .into_response()
}

async fn tier_edit_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<TierForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some((tid, _name, _d, ttype, _a, old_currency, old_monthly, old_yearly, _w, _t, _v)) =
        load_tier(&state, &id).await
    else {
        return membership_flash("error", "That tier could not be found.");
    };
    let name = form.name.trim();
    if name.is_empty() {
        return tier_flash(&tid, "error", "A tier needs a name.");
    }
    let is_free = ttype == "free";
    let currency = if form.currency.trim().is_empty() { "usd".to_string() } else { form.currency.trim().to_lowercase() };
    let monthly = if is_free { None } else { parse_price_cents(&form.monthly_price) };
    let yearly = if is_free { None } else { parse_price_cents(&form.yearly_price) };
    let trial: i64 = form.trial_days.trim().parse().unwrap_or(0).max(0);

    let now = ghost_core::dates::now_db();
    let res = sqlx::query(
        "UPDATE products SET name = ?, description = ?, welcome_page_url = ?, visibility = ?, \
         active = ?, currency = ?, monthly_price = ?, yearly_price = ?, trial_days = ?, updated_at = ? WHERE id = ?",
    )
    .bind(name)
    .bind(form.description.trim())
    .bind(form.welcome_page_url.trim())
    .bind(if form.visibility == "none" { "none" } else { "public" })
    .bind(form.active.is_some())
    .bind(&currency)
    .bind(monthly)
    .bind(yearly)
    .bind(trial)
    .bind(&now)
    .bind(&tid)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        return tier_flash(&tid, "error", &format!("Could not save: {e}"));
    }
    if is_free {
        let mut plans: Vec<String> = state.settings.get("portal_plans").and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
        plans.retain(|plan| plan != "free");
        if form.active.is_some() && form.visibility != "none" { plans.insert(0, "free".into()); }
        if let Err(error) = state.settings.set(&state.db, "portal_plans", &serde_json::json!(plans)).await {
            return tier_flash(&tid, "error", &error.message());
        }
    }
    set_tier_benefits(&state, &tid, &form.benefits).await;

    // Mirror the prices into Stripe. Only when something actually changed —
    // Stripe prices are immutable, so every save would otherwise pile up new
    // ones.
    if !is_free {
        let changed = monthly != old_monthly
            || yearly != old_yearly
            || currency != old_currency.unwrap_or_default();
        let missing_prices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ? AND ((monthly_price > 0 AND monthly_price_id IS NULL) OR (yearly_price > 0 AND yearly_price_id IS NULL))")
            .bind(&tid).fetch_one(&state.db).await.unwrap_or(0);
        if changed || missing_prices > 0 {
            if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
                if let Err(e) = sync_tier_to_stripe(&state, &cfg, &tid, name, &currency, monthly, yearly).await {
                    return tier_flash(&tid, "error", &format!("Saved, but Stripe was not updated: {}", e.message()));
                }
            }
        }
    }
    tier_flash(&tid, "ok", "Tier saved.")
}

/// Create the Stripe product and prices for a tier and record their ids.
async fn sync_tier_to_stripe(
    state: &AppState, cfg: &ghost_stripe::StripeConfig, tier_id: &str, name: &str,
    currency: &str, monthly: Option<i64>, yearly: Option<i64>,
) -> ghost_core::Result<()> {
    ghost_api::stripe_tiers::sync_tier(state, cfg, tier_id, name, currency, monthly, yearly).await
}

// --- Stripe connection (Ghost's "Connect to Stripe", self-hosted flavour) ---

fn membership_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/membership/?{encoded}")).into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct StripeConnectForm {
    secret_key: String,
    publishable_key: String,
}

/// Ghost's official Connect broker returns a secure key after Stripe onboarding.
/// Its state is bound to this verified admin session, just as in Ghost 6.73.
fn stripe_oauth_url(mode: &str, state: &str) -> String {
    let client = if mode == "test" { "ca_8LBum4Ctv3mmJ1oD0ZRmxjdAhNrrBUy3" } else { "ca_8LBuZWhYshxF0A55KgCXu8PRTquCKC5x" };
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code").append_pair("scope", "read_write")
        .append_pair("client_id", client).append_pair("redirect_uri", "https://stripe.ghost.org")
        .append_pair("state", state).finish();
    format!("https://connect.stripe.com/oauth/authorize?{query}")
}

async fn stripe_oauth_start(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    use base64::Engine;
    if let Err(response) = require_role(&state, &headers, Role::Administrator).await { return response; }
    if state.config.env == "production" && !state.site_url().starts_with("https://") {
        return membership_flash("error", "Stripe Connect requires HTTPS in production.");
    }
    let Some(sid) = session_id_from(&headers) else { return membership_flash("error", "Sign in again before connecting Stripe."); };
    let mode = if form.get("mode").is_some_and(|mode| mode == "test") { "test" } else { "live" };
    let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::json!({"mode": mode, "randomState": ghost_core::ids::random_hex(16)}).to_string());
    let updated = sqlx::query("UPDATE sessions SET session_data = json_set(session_data, '$.stripe-connect-state', ?, '$.stripe-connect-started-at', ?) WHERE session_id = ?")
        .bind(&encoded).bind(now_unix_seconds()).bind(&sid).execute(&state.db).await;
    match updated {
        Ok(result) if result.rows_affected() == 1 => Redirect::to(&stripe_oauth_url(mode, &encoded)).into_response(),
        _ => membership_flash("error", "Could not start the Stripe connection. Sign in again and retry."),
    }
}

fn now_unix_seconds() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

#[derive(Deserialize)]
struct StripeSecureKeyForm { secure_key: String }

#[derive(Deserialize)]
struct StripeSecureKey { a: String, p: String, s: String, l: bool, i: String }

fn decode_stripe_secure_key(encoded: &str, expected: &str) -> Result<StripeSecureKey, &'static str> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|_| "The Stripe secure key could not be parsed.")?;
    let key: StripeSecureKey = serde_json::from_slice(&raw).map_err(|_| "The Stripe secure key could not be parsed.")?;
    if expected.is_empty() || key.s != expected { return Err("The Stripe connection belongs to another session. Start the connection again."); }
    if !key.i.starts_with("acct_") || !key.a.starts_with("sk_") || !key.p.starts_with("pk_") {
        return Err("The Stripe secure key is incomplete.");
    }
    if key.l != key.a.starts_with("sk_live_") || key.l != key.p.starts_with("pk_live_") {
        return Err("The Stripe secure key has mismatched test and live modes.");
    }
    Ok(key)
}

async fn stripe_oauth_finish(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<StripeSecureKeyForm>) -> Response {
    if let Err(response) = require_role(&state, &headers, Role::Administrator).await { return response; }
    let Some(sid) = session_id_from(&headers) else { return membership_flash("error", "Sign in again before connecting Stripe."); };
    let saved: Option<(Option<String>, Option<i64>)> = sqlx::query_as("SELECT json_extract(session_data, '$.stripe-connect-state'), json_extract(session_data, '$.stripe-connect-started-at') FROM sessions WHERE session_id = ?")
        .bind(&sid).fetch_optional(&state.db).await.ok().flatten();
    let Some((Some(expected), Some(started))) = saved else { return membership_flash("error", "Start Connect with Stripe before pasting its secure key."); };
    if now_unix_seconds().saturating_sub(started) > 30 * 60 {
        return membership_flash("error", "The Stripe connection expired. Start Connect with Stripe again.");
    }
    let key = match decode_stripe_secure_key(&form.secure_key, &expected) {
        Ok(key) => key,
        Err(message) => return membership_flash("error", message),
    };
    if let Err(error) = sqlx::query("UPDATE sessions SET session_data = json_remove(session_data, '$.stripe-connect-state', '$.stripe-connect-started-at') WHERE session_id = ?")
        .bind(&sid).execute(&state.db).await {
        return membership_flash("error", &format!("Could not complete the Stripe connection: {error}"));
    }
    stripe_connect(State(state), headers, Form(StripeConnectForm {secret_key: key.a, publishable_key: key.p})).await
}

/// Store Stripe keys after proving they work.
///
/// Ghost(Pro) uses Stripe Connect's OAuth; a self-hosted site pastes its own
/// keys, which is what this does. The key is verified against `/v1/account`
/// before anything is saved — a typo should fail here, not at a visitor's
/// first checkout.
async fn stripe_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<StripeConnectForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let secret = form.secret_key.trim().to_string();
    let publishable = form.publishable_key.trim().to_string();
    if !secret.starts_with("sk_") && !secret.starts_with("rk_") {
        return membership_flash("error", "That does not look like a Stripe secret key — it should start with sk_ or rk_.");
    }
    if !publishable.is_empty() && !publishable.starts_with("pk_") {
        return membership_flash("error", "The publishable key should start with pk_.");
    }
    let account = match ghost_stripe::verify_account(&secret).await {
        Ok(a) => a,
        Err(e) => return membership_flash("error", &e.message()),
    };

    let old_account = state.settings.get_str("stripe_connect_account_id").unwrap_or_default();
    let old_live = ghost_stripe::StripeConfig::from_settings(&state.settings).map(|cfg| cfg.is_live());
    let account_changed = old_account != account.id || old_live != Some(account.livemode);
    if account_changed {
        let reset = async {
            let mut tx = state.db.begin().await?;
            sqlx::query("UPDATE products SET monthly_price_id = NULL, yearly_price_id = NULL WHERE type = 'paid'").execute(&mut *tx).await?;
            sqlx::query("DELETE FROM stripe_prices").execute(&mut *tx).await?;
            sqlx::query("DELETE FROM stripe_products").execute(&mut *tx).await?;
            tx.commit().await
        }.await;
        if let Err(error) = reset { return membership_flash("error", &format!("Could not reset Stripe price references: {error}")); }
    }

    // Keep a working endpoint or local Stripe CLI secret when reconnecting
    // the same account and mode. A different account must not inherit it.
    if account_changed {
        if let Some(old_cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
            if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|id| !id.is_empty()) {
                let _ = ghost_stripe::delete_webhook(&old_cfg, &id).await;
            }
        }
        for key in ["members_stripe_webhook_id", "members_stripe_webhook_secret", "stripe_checkout_setup_error"] {
            let _ = state.settings.set(&state.db, key, &serde_json::json!("")).await;
        }
    }

    let set = |k: &'static str, v: String| {
        let state = state.clone();
        async move { state.settings.set(&state.db, k, &serde_json::json!(v)).await.is_ok() }
    };
    let mut ok = set("stripe_secret_key", secret.clone()).await;
    ok &= set("stripe_publishable_key", publishable).await;
    ok &= set("stripe_connect_account_id", account.id.clone()).await;
    ok &= set("stripe_connect_display_name", account.display_name.clone()).await;
    ok &= set("stripe_connect_livemode", account.livemode.to_string()).await;
    // Paid tiers only become offerable once Stripe is attached, and Portal
    // reads this to decide whether to show them at all.
    ok &= state.settings.set(&state.db, "paid_members_enabled", &serde_json::json!(true)).await.is_ok();
    if !ok {
        return membership_flash("error", "Could not save the Stripe settings — check the logs.");
    }

    // Register the webhook so subscription changes reach us. A site on
    // localhost is unreachable from Stripe, so this is allowed to fail
    // without failing the connection itself.
    let cfg = ghost_stripe::StripeConfig {
        secret_key: secret,
        publishable_key: String::new(),
        webhook_secret: String::new(),
        checkout_design: None,
    };
    let price_note = match ghost_api::stripe_tiers::sync_active_tiers(&state, &cfg).await {
        Ok(()) => String::new(),
        Err(error) => format!(" Tier price setup failed: {}. Checkout will retry setup when needed.", error.message()),
    };
    let hook_url = format!("{}/members/webhooks/stripe/", state.site_url());
    let hook_note = if state.settings.get_str("members_stripe_webhook_secret").is_some_and(|secret| !secret.is_empty()) {
        String::new()
    } else { match ghost_stripe::create_webhook(&cfg, &hook_url).await {
        Ok((id, whsec)) => {
            let _ = state.settings.set(&state.db, "members_stripe_webhook_id", &serde_json::json!(id)).await;
            let _ = state.settings.set(&state.db, "members_stripe_webhook_secret", &serde_json::json!(whsec)).await;
            String::new()
        }
        Err(e) => format!(
            " Webhooks could not be registered ({}) — subscriptions will not update automatically until {hook_url} is reachable from the internet.",
            e.message()
        ),
    }};

    let who = if account.display_name.is_empty() { account.id } else { account.display_name };
    let mode = if account.livemode { "live" } else { "test" };
    membership_flash("ok", &format!("Connected to Stripe account “{who}” in {mode} mode.{price_note}{hook_note}"))
}

/// The webhook signing-secret form, shown both when nothing is set and when
/// rotating an existing secret.
fn stripe_webhook_secret_form(site_url: &str) -> String {
    format!(
        r#"<form method="post" action="/ghost/settings/membership/stripe/webhook/" class="inline-form">
<label class="field" style="flex:1;min-width:240px;margin:0"><input name="webhook_secret" type="password" placeholder=" " autocomplete="off" required><span>Webhook signing secret · whsec_…</span></label>
<button class="btn tonal small" type="submit">Save secret</button></form>
<p class="muted" style="margin:10px 0 0">Testing locally? Run this and paste the <code>whsec_…</code> it prints:</p>
<p class="muted" style="margin:6px 0 0"><code style="display:block;white-space:pre-wrap;word-break:break-all">stripe listen --events {events} --forward-to {url}/members/webhooks/stripe/</code></p>
<p class="muted" style="margin:6px 0 0">Recent Stripe CLI versions require <code>--events</code> (or <code>--all-snapshot</code>).</p>"#,
        events = ghost_stripe::WEBHOOK_EVENTS.join(","),
        url = esc(site_url),
    )
}

/// Direct API key setup, shown when no account is connected.
/// Hosted onboarding stays hidden until we provide our own Stripe Connect flow.
fn stripe_key_form(heading: &str, blurb: &str, button: &str) -> String {
    format!(
        r#"<span class="title" style="display:block;margin-bottom:6px">{heading}</span>
<p class="muted" style="margin:0 0 12px">{blurb}</p>
<form method="post" action="/ghost/settings/membership/stripe/connect/">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="secret_key" type="password" placeholder=" " autocomplete="off" required><span>Secret key · sk_live_… or sk_test_…</span></label>
<label class="field" style="margin:0"><input name="publishable_key" placeholder=" " autocomplete="off"><span>Publishable key · pk_…</span></label>
</div>
<button class="btn tonal" type="submit" style="margin-top:12px">{coin} {button}</button>
</form>
<p class="muted" style="margin:12px 0 0">The key is checked against Stripe before it is stored, and is never shown again afterwards. Stripe Checkout still requires the account business details to be complete.</p>"#,
        heading = heading,
        blurb = blurb,
        button = button,
        coin = svg("chart"),
    )
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct WebhookSecretForm {
    webhook_secret: String,
}

/// Set the webhook signing secret by hand.
///
/// Auto-registration only works when Stripe can reach the site, so a local
/// install never gets a secret — and without one every webhook is refused,
/// which makes paid membership impossible to exercise. `stripe listen` prints
/// a `whsec_…` for exactly this case.
async fn stripe_set_webhook_secret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<WebhookSecretForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let secret = form.webhook_secret.trim().to_string();
    if !secret.is_empty() && !secret.starts_with("whsec_") {
        return membership_flash("error", "A webhook signing secret starts with whsec_.");
    }
    match state.settings.set(&state.db, "members_stripe_webhook_secret", &serde_json::json!(secret.clone())).await {
        Ok(()) if secret.is_empty() => membership_flash("ok", "Webhook secret cleared. Webhooks will be refused until one is set."),
        Ok(()) => membership_flash("ok", "Webhook signing secret saved."),
        Err(e) => membership_flash("error", &e.message()),
    }
}

/// Turn the webhook off: drop the signing secret, and remove the endpoint at
/// Stripe when we registered one.
///
/// Clearing the secret is what actually deactivates it — every incoming
/// webhook is refused without one — so this is also the safe way to retire a
/// secret that has leaked, without touching the Stripe connection itself.
async fn stripe_deactivate_webhook(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut note = String::new();
    if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            if let Err(e) = ghost_stripe::delete_webhook(&cfg, &id).await {
                // Local-only secrets (from `stripe listen`) have no endpoint
                // to delete, and a deleted one is already gone — neither is
                // worth failing over, but say so rather than implying the
                // Stripe side was tidied up.
                note = format!(" The endpoint at Stripe could not be removed ({}) — delete it in the dashboard if it still exists.", e.message());
            }
        }
    }
    for k in ["members_stripe_webhook_id", "members_stripe_webhook_secret"] {
        let _ = state.settings.set(&state.db, k, &serde_json::json!("")).await;
    }
    membership_flash("ok", &format!("Webhook deactivated. Subscription changes will not be accepted until a new secret is set.{note}"))
}

/// Disconnect. The tiers and their Stripe ids are left alone: reconnecting
/// the same account should not mean rebuilding every price, and deleting
/// local price ids would orphan live subscriptions.
async fn stripe_disconnect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Remove the registered webhook first, while the key still works.
    if let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) {
        if let Some(id) = state.settings.get_str("members_stripe_webhook_id").filter(|i| !i.is_empty()) {
            let _ = ghost_stripe::delete_webhook(&cfg, &id).await;
        }
    }
    for k in [
        "stripe_secret_key", "stripe_publishable_key", "stripe_connect_account_id",
        "stripe_connect_display_name", "stripe_connect_livemode",
        "members_stripe_webhook_id", "members_stripe_webhook_secret",
    ] {
        let _ = state.settings.set(&state.db, k, &serde_json::json!("")).await;
    }
    let _ = state.settings.set(&state.db, "paid_members_enabled", &serde_json::json!(false)).await;
    membership_flash("ok", "Disconnected from Stripe. Existing subscriptions are untouched on Stripe's side.")
}

/// One row of the Portal links reference: a name, and the same destination in
/// both of the spellings Portal accepts.
///
/// `link` is shown first because that is the form a theme author reaches for;
/// the attribute is the one to use on an element that is not an anchor, or
/// where the href has to stay a real URL for visitors without JavaScript. Both
/// are rendered into `data-` attributes and the visible one is chosen in the
/// browser, so flipping the switch costs no round trip.
fn portal_link_row(name: &str, link: &str, attr: &str) -> String {
    format!(
        r#"<div class="plink" data-plink data-link="{link}" data-attr="{attr}">
<span class="name">{name}</span>
<span class="key"><code>{link}</code><button type="button" data-copy="{link}" title="Copy" aria-label="Copy {name}">{copy}</button></span></div>"#,
        name = esc(name),
        link = esc(link),
        attr = esc(attr),
        copy = svg("copy"),
    )
}

/// Every Portal destination the site answers, as a link and as a data
/// attribute — Ghost's Settings → Portal → Links, which this port had nowhere
/// to show.
///
/// Rows are gated on what the site can actually do: with no Stripe connection
/// there are no paid signups, no gifts and no tips, so listing those links
/// would advertise screens Portal refuses to open. `paid_tiers` drives the
/// picker, since a tier-specific signup URL needs a real tier id to work.
fn portal_links_card(
    site_url: &str,
    paid_tiers: &[(String, String)],
    paid_on: bool,
    recommendations_on: bool,
) -> String {
    let base = site_url.trim_end_matches('/');
    let row = |name: &str, path: &str| {
        let attr = path.trim_start_matches('/');
        let attr = if attr.is_empty() {
            "data-portal".to_string()
        } else {
            format!(r#"data-portal="{attr}""#)
        };
        portal_link_row(name, &format!("{base}/#/portal{path}"), &attr)
    };

    let mut generic = String::new();
    generic.push_str(&row("Default", ""));
    generic.push_str(&row("Sign in", "/signin"));
    generic.push_str(&row("Sign up", "/signup"));
    generic.push_str(&row("Sign up / Free", "/signup/free"));
    if paid_on {
        generic.push_str(&row("Sign up / Monthly", "/signup/monthly"));
        generic.push_str(&row("Sign up / Yearly", "/signup/yearly"));
        generic.push_str(&row("Gift subscriptions", "/gift"));
        generic.push_str(&row("Tips & donations", "/support"));
    }
    if recommendations_on {
        generic.push_str(&row("Recommendations", "/recommendations"));
    }

    // The tier block only makes sense once there is a paid tier to name: the
    // id is part of the path, and `#/portal/signup//monthly` opens nothing.
    let tiers = if paid_on && !paid_tiers.is_empty() {
        let opts: String = paid_tiers
            .iter()
            .enumerate()
            .map(|(i, (id, name))| {
                format!(
                    r#"<option value="{id}"{sel}>{name}</option>"#,
                    id = esc(id),
                    name = esc(name),
                    sel = if i == 0 { " selected" } else { "" },
                )
            })
            .collect();
        format!(
            r#"<h3>Tiers</h3>
<div class="plink"><span class="name">Tier</span>
<label class="field fixed" style="margin:0;max-width:260px"><select data-plink-tier>{opts}</select><span>Tier</span>{car}</label></div>
{monthly}{yearly}"#,
            opts = opts,
            car = SELECT_CARET,
            monthly = row("Signup / Monthly", "/signup/{tier}/monthly"),
            yearly = row("Signup / Yearly", "/signup/{tier}/yearly"),
        )
    } else {
        String::new()
    };

    let mut account = String::new();
    account.push_str(&row("Account", "/account"));
    if paid_on {
        account.push_str(&row("Account / Plans", "/account/plans"));
    }
    account.push_str(&row("Account / Profile", "/account/profile"));
    account.push_str(&row("Account / Newsletters", "/account/newsletters"));
    account.push_str(&row("Account / Newsletter help", "/account/newsletters/help"));
    account.push_str(&row("Account / Email disabled", "/account/newsletters/disabled"));

    format!(
        r#"<div class="section">Portal links</div>
<div class="card padded rise plinks" style="--i:7" data-plinks>
<div class="inline-form" style="justify-content:space-between;align-items:center;margin-bottom:6px">
<p class="muted" style="margin:0;max-width:460px">Use these <span data-plink-note>links</span> anywhere in your theme, your navigation or an email to open a page of Portal. Copy a row to use it as it is.</p>
{seg}
</div>
<h3>Generic</h3>
{generic}
{tiers}
<h3>Account</h3>
{account}
<p class="muted" style="margin:18px 0 0">An offer has a link of its own, <code>{base}/&lt;offer code&gt;</code>, which sends the visitor to <code>#/portal/offers/&lt;id&gt;</code> — find it on the offer in <a href="/ghost/settings/growth/">Settings → Growth</a>.{gift_note}</p>
</div>"#,
        seg = segmented(
            "portal_link_mode",
            &[("links", "Links"), ("attrs", "Data attributes")],
            "links",
        ),
        generic = generic,
        tiers = tiers,
        account = account,
        base = esc(base),
        // A redemption link only exists once gifts can be bought, which needs
        // the same Stripe connection as the Gift row above.
        gift_note = if paid_on {
            " A gift email carries <code>#/portal/gift/redeem/&lt;token&gt;</code>, which is minted per gift."
        } else {
            ""
        },
    )
}

async fn membership_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let get = |k: &str, d: &str| state.settings.get_str(k).unwrap_or_else(|| d.to_string());

    let mut tier_rows = String::new();
    // The paid, active tiers also feed the Portal links reference below, whose
    // tier-specific signup URLs name a tier by id.
    let mut paid_tiers: Vec<(String, String)> = Vec::new();
    if let Ok(tiers) = sqlx::query_as::<_, (String, String, String, Option<i64>, Option<i64>, bool)>(
        "SELECT id, name, type, monthly_price, yearly_price, active FROM products ORDER BY type DESC, created_at",
    )
    .fetch_all(&state.db)
    .await
    {
        for (id, name, tier_type, monthly, yearly, active) in tiers {
            if active && tier_type != "free" {
                paid_tiers.push((id.clone(), name.clone()));
            }
            let price = if tier_type == "free" {
                "Free".to_string()
            } else {
                format!("{} / month · {} / year", tier_price(monthly), tier_price(yearly))
            };
            tier_rows.push_str(&format!(
                r#"<a class="row" href="/ghost/settings/membership/tiers/{id}/"><span class="ic tint-teal">{ghost}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{price}</span></div>
<div class="trailing">{inactive}<span class="chip tonal">{typ}</span>{chev}</div></a>"#,
                id = esc(&id),
                ghost = svg("ghost"),
                name = esc(&name),
                price = esc(&price),
                inactive = if active { "" } else { r#"<span class="chip draft">inactive</span>"# },
                typ = esc(&tier_type),
                chev = svg("chevron"),
            ));
        }
    }

    let mut newsletter_rows = String::new();
    // the primary newsletter's editable fields: (id, name, description, sender_name, sender_email)
    let mut newsletter_edit: Option<(String, String, String, String, String)> = None;
    if let Ok(rows) = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, name, sender_name, sender_email, status, description FROM newsletters ORDER BY sort_order, created_at",
    )
    .fetch_all(&state.db)
    .await
    {
        for (i, (id, name, sender_name, sender_email, status, description)) in rows.iter().enumerate() {
            if i == 0 {
                newsletter_edit = Some((
                    id.clone(),
                    name.clone(),
                    description.clone().unwrap_or_default(),
                    sender_name.clone().unwrap_or_default(),
                    sender_email.clone().unwrap_or_default(),
                ));
            }
            let edit = if i == 0 {
                r#"<span class="chip published">editable below</span>"#.to_string()
            } else {
                String::new()
            };
            let sender = [sender_name.clone().unwrap_or_default(), sender_email.clone().unwrap_or_default()]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" <");
            newsletter_rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-teal">{mail}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{sender}{status}</span></div>
<div class="trailing">{edit}<span class="chip tonal">{id_short}</span></div></div>"#,
                mail = svg("notes"),
                name = esc(name),
                sender = if sender.is_empty() { String::new() } else { format!("{} · ", esc(&sender)) },
                status = esc(status.as_deref().unwrap_or("active")),
                edit = edit,
                id_short = esc(&id[..8.min(id.len())]),
            ));
        }
    }

    // Stripe connection card. Ghost(Pro) does this over Stripe Connect OAuth;
    // a self-hosted site pastes its own keys, which is what this offers.
    let stripe_card = match ghost_stripe::StripeConfig::from_settings(&state.settings) {
        Some(cfg) => {
            let who = state.settings.get_str("stripe_connect_display_name").unwrap_or_default();
            let account = state.settings.get_str("stripe_connect_account_id").unwrap_or_default();
            let live = cfg.is_live();
            let hook = state.settings.get_str("members_stripe_webhook_secret").unwrap_or_default();
            format!(
                r#"<div class="section">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-green">{check_ic}</span>
<div class="body"><span class="title">Connected{named}</span><span class="detail">{account} · {mode} mode</span></div>
<div class="trailing"><span class="chip {chip}">{mode}</span>
<form method="post" action="/ghost/settings/membership/stripe/disconnect/" data-confirm="Disconnect from Stripe|New paid signups will stop. Existing subscriptions stay active on Stripe and resume if you reconnect the same account.">
<button class="btn tonal small danger" type="submit">Disconnect from Stripe</button></form></div></div>
{checkout_notice}
{hook_row}
</div>"#,
                checkout_notice = {
                    let error = state.settings.get_str("stripe_checkout_setup_error").unwrap_or_default();
                    if error.is_empty() { String::new() } else {
                        format!(r#"<div class="row"><span class="ic tint-amber">{}</span><div class="body"><span class="title">Checkout setup required</span><span class="detail">{}</span><a href="https://dashboard.stripe.com/settings/business-details" target="_blank" rel="noopener">Open Stripe business details</a></div></div>"#, svg("flag"), esc(&error))
                    }
                },
                check_ic = svg("check"),
                named = if who.is_empty() { String::new() } else { format!(" · {}", esc(&who)) },
                account = esc(&account),
                mode = if live { "live" } else { "test" },
                chip = if live { "published" } else { "draft" },
                hook_row = if !hook.trim().is_empty() {
                    // Active: deactivating comes first, the way disconnecting
                    // precedes connecting above. Setting a replacement is
                    // still possible directly, for rotating a secret.
                    format!(
                        r#"<div class="row"><span class="ic tint-green">{link_ic}</span>
<div class="body"><span class="title">Webhooks active</span><span class="detail">Subscription changes update members automatically.</span></div>
<div class="trailing"><span class="chip published">verified</span>
<form method="post" action="/ghost/settings/membership/stripe/webhook/deactivate/" data-confirm="Deactivate webhook|Subscription changes will stop being accepted until a new signing secret is set. Payments still go through at Stripe.">
<button class="btn tonal small danger" type="submit">Deactivate webhook</button></form></div></div>"#,
                        link_ic = svg("link"),
                    )
                } else {
                    format!(
                        r#"<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{flag}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:4px">Webhooks not active</span>
<p class="muted" style="margin:0 0 10px">Without a signing secret every incoming webhook is refused, so subscriptions will never update. Stripe could not register one automatically — that needs a publicly reachable site.</p>
{secret_form}
</div></div>"#,
                        flag = svg("flag"),
                        secret_form = stripe_webhook_secret_form(&state.site_url()),
                    )
                },
            )
        }
        None => format!(
            r#"<div class="section">Stripe</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{coin}</span>
<div class="body" style="width:100%">{key_form}</div></div>
</div>"#,
            coin = svg("chart"),
            key_form = stripe_key_form(
                "Connect Stripe",
                "Enter your Stripe secret and publishable keys to offer paid tiers. Use test keys until you are ready to accept payments.",
                "Save API keys",
            ),
        ),
    };

    let toggle = |key: &str, title: &str, detail: &str| {
        format!(
            r#"<div class="row"><span class="ic tint-teal">{ghost}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" form="membership-form" name="{key}" value="1" {ck}><span class="track"><span class="thumb"></span></span></label></div></div>"#,
            ghost = svg("ghost"),
            title = esc(title),
            detail = esc(detail),
            key = esc(key),
            ck = if state.settings.get_bool(key) { "checked" } else { "" },
        )
    };

    // The access code only matters while the site is private, so Ghost only
    // shows it then. It is deliberately readable rather than masked: it is
    // shared with readers, not a credential anyone types in private.
    let private_code = if state.settings.get_bool("is_private") {
        let site_url = state.site_url();
        format!(
            r#"<div style="display:flex;gap:10px;align-items:flex-end;flex-wrap:wrap;margin-top:12px">
<label class="field" style="margin:0;max-width:320px;flex:1"><input name="password" form="membership-form" value="{code}" placeholder=" " maxlength="191"><span>Access code</span></label>
<button class="btn tonal small" type="submit" form="membership-form" name="regenerate_code" value="1" title="Generate a new code">{refresh} Regenerate</button>
</div>
<p class="muted" style="margin:6px 0 0">Everyone who visits <a href="{url}/private/" target="_blank" rel="noopener">{url}/private/</a> needs this.
Changing it — including regenerating — signs out everyone who had the old one.</p>"#,
            code = s("password"),
            refresh = svg("history"),
            url = esc(site_url.trim_end_matches('/')),
        )
    } else {
        String::new()
    };

    // Ghost's fourth default-access option: specific tiers. The chips only
    // mean anything while that option is chosen, so they are rendered with the
    // row rather than as a separate setting.
    let visibility_now = get("default_content_visibility", "public");
    let tier_options: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM products WHERE active = 1 ORDER BY type DESC, created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect();
    let selected_tiers: Vec<String> = match state.settings.get("default_content_visibility_tiers") {
        Some(serde_json::Value::Array(a)) => {
            a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
        _ => Vec::new(),
    };
    let visibility_tiers = if visibility_now == "tiers" {
        format!(
            r#"<div style="margin-top:10px">{chips}</div>
<p class="muted" style="margin:8px 0 0">New posts are visible to members on any of the ticked tiers.</p>"#,
            chips = chip_set(
                "default_content_visibility_tiers",
                &tier_options,
                &selected_tiers,
                "No tiers yet — add one under Tiers below.",
            ),
        )
    } else {
        String::new()
    };

    // --- welcome emails -----------------------------------------------------
    //
    // These are the two built-in welcome automations, seeded on every install.
    // Ghost surfaces them here as well as under Automations, because this is
    // where a publisher looks for them.
    let welcome_rows = {
        let flows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT id, name, status, slug FROM automations WHERE slug IN (?, ?) ORDER BY slug",
        )
        .bind("member-welcome-email-free")
        .bind("member-welcome-email-paid")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        let mut rows = String::new();
        for (id, name, status, slug) in &flows {
            let active = status == "active";
            let is_free = slug.as_deref() == Some("member-welcome-email-free");
            // An empty flow cannot be turned on — the automation engine
            // refuses it — so offer the editor instead of a switch that is
            // guaranteed to bounce.
            let written: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM automation_actions WHERE automation_id = ?")
                    .bind(id)
                    .fetch_one(&state.db)
                    .await
                    .unwrap_or(0);
            let trailing = if written == 0 {
                format!(
                    r#"<span class="chip draft">not written</span><a class="btn tonal small" href="/ghost/automations/{id}/">Write it</a>"#,
                    id = esc(id),
                )
            } else {
                format!(
                    r#"<span class="chip {chip}">{state_label}</span>
<form method="post" action="/ghost/automations/{id}/status/">
<input type="hidden" name="status" value="{next}">
<button class="btn tonal small" type="submit">{button}</button></form>"#,
                    id = esc(id),
                    chip = if active { "published" } else { "draft" },
                    state_label = if active { "on" } else { "off" },
                    next = if active { "inactive" } else { "active" },
                    button = if active { "Turn off" } else { "Turn on" },
                )
            };
            rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-teal">{ic}</span>
<div class="body"><a class="title" href="/ghost/automations/{id}/">{title}</a><span class="detail">{detail}</span></div>
<div class="trailing">{trailing}</div></div>"#,
                ic = svg("mail"),
                id = esc(id),
                title = esc(name),
                detail = if is_free {
                    "Sent when someone signs up for a free account"
                } else {
                    "Sent when someone starts a paid subscription"
                },
                trailing = trailing,
            ));
        }
        if rows.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">The welcome flows are seeded at startup and will appear here.</span></div></div>"#.into();
        }
        format!(
            r#"<div class="card rows rise" style="--i:4">{rows}</div>
<p class="muted" style="margin:10px 0 0 16px">Open one to write it. Signup and signin magic links are separate — they always send, using the
transport in <a href="/ghost/settings/email/">Settings → Email</a>.</p>"#
        )
    };

    // --- custom fields: Ghost's Membership → Custom fields ------------------
    //
    // Each row is its own form: a field's key and type are fixed once it
    // exists (values reference the key and are shaped by the type), so only
    // the name and the member's access can be edited here.
    let fields = repos::metafields::list_definitions(&state.db).await.unwrap_or_default();
    let last = fields.len().saturating_sub(1);
    let mut field_rows = String::new();
    for (i, f) in fields.iter().enumerate() {
        let held = repos::metafields::value_count(&state.db, &f.key).await;
        let type_label = match f.field_type.as_str() {
            "short_text" => "Short text",
            "long_text" => "Long text",
            "address" => "Address",
            other => other,
        };
        field_rows.push_str(&format!(
            r#"<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{ic}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/membership/fields/{id}/" class="inline-form" style="align-items:flex-end">
<label class="field" style="flex:1;min-width:180px;margin:0"><input name="name" value="{name}" placeholder=" " maxlength="191" required><span>Name</span></label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="member_access">{access_opts}</select><span>Member access</span>{car}</label>
<label class="field fixed" style="min-width:150px;margin:0"><select name="status">{status_opts}</select><span>Status</span>{car}</label>
<button class="btn tonal small" type="submit">Save</button>
</form>
<p class="muted" style="margin:8px 0 0"><code>{identity}</code> · {type_label} · {held} member{plural} with a value</p>
</div>
<div class="trailing" style="align-items:flex-start">
<form method="post" action="/ghost/settings/membership/fields/{id}/move/"><input type="hidden" name="dir" value="up">
<button class="icon-btn" style="transform:rotate(-90deg)" title="Move up" aria-label="Move up"{up_dis}>{up}</button></form>
<form method="post" action="/ghost/settings/membership/fields/{id}/move/"><input type="hidden" name="dir" value="down">
<button class="icon-btn" style="transform:rotate(90deg)" title="Move down" aria-label="Move down"{down_dis}>{down}</button></form>
<form method="post" action="/ghost/settings/membership/fields/{id}/delete/" data-confirm="Delete custom field|&#8220;{name}&#8221; and the values {held} member{plural} hold for it will be deleted.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form>
</div></div>"#,
            ic = svg("notes"),
            id = esc(&f.id),
            name = esc(&f.name),
            identity = esc(&f.identity()),
            type_label = type_label,
            held = held,
            plural = if held == 1 { "" } else { "s" },
            car = SELECT_CARET,
            access_opts = select_options(
                &[
                    ("none", "Staff only"),
                    ("read", "Members can view"),
                    ("write", "Members can edit"),
                ],
                &f.member_access,
            ),
            status_opts = select_options(&[("active", "Active"), ("archived", "Archived")], &f.status),
            up = svg("chevron"),
            down = svg("chevron"),
            up_dis = if i == 0 { " disabled" } else { "" },
            down_dis = if i == last { " disabled" } else { "" },
            del = svg("delete"),
        ));
    }
    if field_rows.is_empty() {
        field_rows = r#"<div class="row"><div class="body"><span class="muted">No custom fields yet — add one to collect something extra about your members.</span></div></div>"#.into();
    }
    let fields_card = format!(
        r#"<div class="section">Custom fields</div>
<div class="card rows rise" style="--i:7">{field_rows}</div>
<div class="card padded rise" style="--i:7">
<form method="post" action="/ghost/settings/membership/fields/new/" class="inline-form">
<label class="field" style="flex:1;min-width:200px;margin:0"><input name="name" placeholder=" " maxlength="191" required><span>Field name</span></label>
<label class="field fixed" style="min-width:170px;margin:0"><select name="type">{type_opts}</select><span>Type</span>{car}</label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="member_access">{new_access_opts}</select><span>Member access</span>{car}</label>
<button class="btn tonal" type="submit">{add} Add field</button>
</form>
<p class="muted" style="margin:12px 0 0">The key is taken from the name and cannot change afterwards — values are stored against it.
An <b>address</b> collects six parts (line 1 and 2, city, state, postal code, country).
<b>Staff only</b> keeps a field out of the member's own account page entirely.</p>
</div>"#,
        field_rows = field_rows,
        car = SELECT_CARET,
        add = svg("add"),
        type_opts = select_options(
            &[("short_text", "Short text"), ("long_text", "Long text"), ("address", "Address")],
            "short_text",
        ),
        new_access_opts = select_options(
            &[("none", "Staff only"), ("read", "Members can view"), ("write", "Members can edit")],
            "none",
        ),
    );

    // --- spam filters: Ghost puts this immediately after Access ------------
    // `blocked_email_domains` is stored as a JSON array; the textarea is the
    // readable form of it, one domain per line.
    let blocked_domains = state.settings.blocked_email_domains().join("\n");
    let spam_card = format!(
        r#"<div class="section">Spam filters</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-amber">{shield}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><textarea name="blocked_email_domains" form="membership-form" rows="3" placeholder=" ">{domains}</textarea><span>Blocked email domains</span></label>
<p class="muted" style="margin:6px 0 0">Protect your member signups from spam. One domain per line, e.g. <code>spam.xyz</code> to block
addresses like <code>hello@spam.xyz</code>. Members who already signed up are not removed.</p>
</div></div>
</div>"#,
        shield = svg("flag"),
        domains = esc(&blocked_domains),
    );

    // Stripe is what unlocks paid members, and gifts and tips both bill
    // through it — Ghost hides each of these sections until it is connected.
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();
    let site_url = state.site_url();
    let site_url = site_url.trim_end_matches('/');

    let gift_card = if stripe_on {
        format!(
            r#"<div class="section">Gift subscriptions</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{gift}</span>
<div class="body" style="width:100%">
<span class="title">Shareable link</span>
<p class="muted" style="margin:6px 0 10px">Allow your readers to share your work by purchasing a gift subscription for a friend or colleague.</p>
{link}
</div></div>
</div>"#,
            gift = svg("ghost"),
            link = key_chip("Gift", &format!("{site_url}/#/portal/gift")),
        )
    } else {
        String::new()
    };

    // Ghost stores the suggested tip in the currency's minor unit (cents), but
    // asks for it in the major one — so it is divided here and multiplied back
    // on save.
    let tips_card = if stripe_on {
        let cents = state.settings.get_str("donations_suggested_amount").unwrap_or_default();
        let cents: i64 = cents.trim().parse().unwrap_or(0);
        format!(
            r#"<div class="section">Tips &amp; donations</div>
<div class="card rows rise" style="--i:6">
<div class="row"><span class="ic tint-green">{coin}</span>
<div class="body"><div class="grid2" style="gap:12px">
<label class="field fixed" style="margin:0"><select name="donations_currency" form="membership-form">{currency_opts}</select><span>Currency</span>{car}</label>
<label class="field" style="margin:0"><input name="donations_suggested_amount" form="membership-form" type="number" min="0" max="10000" step="1" value="{amount}" placeholder=" "><span>Suggested amount</span></label>
</div>
<p class="muted" style="margin:8px 0 0">One-time payments, no account needed. Stripe caps a preset amount at 10,000.</p></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{linkic}</span>
<div class="body" style="width:100%"><span class="title">Shareable link</span>
<p class="muted" style="margin:6px 0 10px">Send readers straight to the tip page, or add it to your navigation.</p>
{link}</div></div>
</div>"#,
            coin = svg("ghost"),
            linkic = svg("link"),
            car = SELECT_CARET,
            currency_opts = select_options(CURRENCIES, &get("donations_currency", "USD")),
            amount = cents / 100,
            link = key_chip("Tips", &format!("{site_url}/#/portal/support")),
        )
    } else {
        String::new()
    };

    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "membership").unwrap();
    page_for_role(actor_role, 
        "Membership",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/membership/" id="membership-form"></form>
<div class="card"><a class="row" href="/ghost/settings/checkout/"><div class="body"><span class="title">Checkout design</span><span class="desc">Customize colors, typography and button corners for paid signups.</span></div></a></div>
<div class="section">Access</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-amber">{private_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="is_private" form="membership-form">{private_opts}</select><span>Site access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who is allowed to browse the site at all. A private site is hidden from search
engines and asks every visitor for one shared access code.</p>
{private_code}</div></div>
<div class="row"><span class="ic tint-teal">{key}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="members_signup_access" form="membership-form">{access_opts}</select><span>Signup access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who is allowed to create an account on your site.</p></div></div>
<div class="row"><span class="ic tint-teal">{lock}</span>
<div class="body" style="width:100%"><label class="field fixed" style="margin:0;max-width:320px"><select name="default_content_visibility" form="membership-form" onchange="this.form.requestSubmit()">{visibility_opts}</select><span>Default post access</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Access used by new posts when none is set in the editor.</p>{visibility_tiers}</div></div>
<div class="row"><span class="ic tint-teal">{comment_ic}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:320px"><select name="comments_enabled" form="membership-form">{comment_opts}</select><span>Commenting</span>{car}</label>
<p class="muted" style="margin:6px 0 0">Who can comment on posts. Readers always need an account to comment. Existing comments stay visible when you turn commenting off — <a href="/ghost/comments/">moderate them here</a>.</p></div></div>
</div>
{spam_card}
{stripe_card}
<div class="section">Tiers</div>
<div class="card rows rise" style="--i:2">{tier_rows}</div>
<div style="padding:2px 4px 8px"><form method="post" action="/ghost/settings/membership/tiers/new/">
<button class="btn tonal small" type="submit">{add} Add tier</button></form></div>
<p class="muted" style="margin:4px 0 0 16px">Open a tier to change its name, price, benefits and visibility. Paid tiers need Stripe connected above.</p>
<div class="section">Signup portal</div>
<div class="card rows rise" style="--i:3">
{portal_button}
<div class="row" style="align-items:center"><span class="ic tint-teal">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="portal_button_signup_text" form="membership-form" value="{signup_text}" placeholder=" "><span>Portal button signup text</span></label></div></div>
<div class="row" style="align-items:center"><span class="ic tint-teal">{title_ic}</span>
<div class="body"><label class="field" style="margin:0"><input name="portal_button_icon" form="membership-form" value="{icon}" placeholder=" "><span>Portal button icon</span></label></div></div>
<div class="row"><span class="ic tint-teal">{palette}</span>
<div class="body"><label class="field fixed" style="margin:0;max-width:260px"><select name="portal_button_style" form="membership-form">{style_opts}</select><span>Portal button style</span>{car}</label></div></div>
{portal_name}
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{notes}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="portal_signup_terms_html" form="membership-form" rows="3" placeholder=" ">{terms}</textarea><span>Signup terms · HTML shown on signup</span></label></div></div>
{terms_required}
<div class="row" style="align-items:flex-start"><span class="ic tint-teal">{plans_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:10px">Plans shown at signup</span>
{plan_chips}
<div style="max-width:260px;margin-top:12px"><label class="field fixed" style="margin:0"><select name="portal_default_plan" form="membership-form">{default_plan_opts}</select><span>Default price at signup</span>{car}</label></div>
<p class="muted" style="margin:8px 0 0">Which prices the portal offers, and which one is preselected. A default that is not on offer falls back to one that is.</p>
</div></div>
<div class="row" style="align-items:center"><span class="ic tint-teal">{mail_ic}</span>
<div class="body"><label class="field" style="margin:0;max-width:360px"><input name="members_support_address" form="membership-form" type="email" value="{support_address}" placeholder=" "><span>Support email address</span></label>
<p class="muted" style="margin:6px 0 0">Shown on the member's account page and used as the reply-to on member emails. Blank uses the sender address from <a href="/ghost/settings/email/">Settings → Email</a>.</p></div></div>
{gift_signup}
{gift_account}
</div>
{gift_card}
<div class="section">Welcome emails</div>
{welcome_deferred}
{tips_card}
{fields_card}
<div class="section">Newsletters</div>
<div class="card rows rise" style="--i:5">{newsletter_rows}</div>
{newsletter_edit}
<p class="muted" style="margin:4px 0 0 16px">The design of each newsletter — colours, typography, header and footer — is set in <a href="/ghost/settings/newsletters/">Settings → Newsletters</a>.</p>
{portal_links}
"#,
            head = group_head(group, Some("membership-form")),
            banner = flash_banner(&q),
            spam_card = spam_card,
            gift_card = gift_card,
            tips_card = tips_card,
            fields_card = fields_card,
            private_ic = svg("lock"),
            private_opts = select_options(
                &[("false", "Public"), ("true", "Private")],
                if state.settings.get_bool("is_private") { "true" } else { "false" },
            ),
            private_code = private_code,
            key = svg("ghost"),
            lock = svg("flag"),
            car = SELECT_CARET,
            access_opts = select_options(&[("all", "Anybody can sign up"), ("invite", "People can only be invited"), ("none", "Nobody can sign up")], &get("members_signup_access", "all")),
            visibility_opts = select_options(
                &[
                    ("public", "Public"),
                    ("members", "Members only"),
                    ("paid", "Paid-members only"),
                    ("tiers", "Specific tiers"),
                ],
                &visibility_now,
            ),
            visibility_tiers = visibility_tiers,
            comment_ic = svg("comment"),
            // Ghost's three levels, same values: `off`, `all`, `paid`.
            comment_opts = select_options(&[("off", "Nobody"), ("all", "All members"), ("paid", "Paid members only")], &get("comments_enabled", "off")),
            tier_rows = tier_rows,
            stripe_card = stripe_card,
            add = svg("add"),
            portal_button = toggle("portal_button", "Show portal button", "Display the floating portal button on the site"),
            portal_name = toggle("portal_name", "Show portal name", "Show the site name inside the portal"),
            terms_required = toggle("portal_signup_checkbox_required", "Require signup terms", "Members must tick the terms checkbox to sign up"),
            plans_ic = svg("tags"),
            mail_ic = svg("mail"),
            plan_chips = {
                // Ghost stores this as a JSON array of plan names.
                let current: Vec<String> = match state.settings.get("portal_plans") {
                    Some(serde_json::Value::Array(a)) => {
                        a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
                    }
                    Some(serde_json::Value::String(s)) => {
                        serde_json::from_str(&s).unwrap_or_default()
                    }
                    _ => vec!["free".to_string()],
                };
                chip_set(
                    "portal_plans",
                    &[
                        ("free".to_string(), "Free".to_string()),
                        ("monthly".to_string(), "Monthly".to_string()),
                        ("yearly".to_string(), "Yearly".to_string()),
                    ],
                    &current,
                    "",
                )
            },
            default_plan_opts = select_options(
                &[("monthly", "Monthly"), ("yearly", "Yearly")],
                &get("portal_default_plan", "monthly"),
            ),
            support_address = s("members_support_address"),
            gift_signup = toggle(
                "portal_signup_gift_promotion",
                "Promote gifts at signup",
                "Offer a gift subscription on the signup screen",
            ),
            gift_account = toggle(
                "portal_account_gift_promotion",
                "Promote gifts on the account page",
                "Offer a gift subscription from a member's own account",
            ),
            title_ic = svg("title"),
            palette = svg("themes"),
            notes = svg("notes"),
            signup_text = s("portal_button_signup_text"),
            icon = s("portal_button_icon"),
            style_opts = select_options(&[("icon-and-text", "Icon and text"), ("icon", "Icon only"), ("text", "Text only")], &get("portal_button_style", "icon-and-text")),
            terms = s("portal_signup_terms_html"),
            welcome_deferred = welcome_rows,
            newsletter_rows = newsletter_rows,
            newsletter_edit = newsletter_edit
                .map(|(id, name, description, sender_name, sender_email)| {
                    format!(
                        r#"<div class="card padded rise" style="--i:6">
<p class="muted" style="margin:0 0 12px">Edit <b>{name}</b></p>
<label class="field"><input name="newsletter_name" form="membership-form" value="{name}" placeholder=" "><span>Newsletter name</span></label>
<label class="field"><input name="newsletter_description" form="membership-form" value="{description}" placeholder=" "><span>Description · optional</span></label>
<div class="grid2">
<label class="field"><input name="newsletter_sender_name" form="membership-form" value="{sender_name}" placeholder=" "><span>Sender name</span></label>
<label class="field"><input name="newsletter_sender_email" form="membership-form" value="{sender_email}" placeholder=" "><span>Sender email</span></label>
</div>
<input type="hidden" name="newsletter_id" form="membership-form" value="{id}">
</div>"#,
                        name = esc(&name),
                        description = esc(&description),
                        sender_name = esc(&sender_name),
                        sender_email = esc(&sender_email),
                        id = esc(&id),
                    )
                })
                .unwrap_or_default(),
            // Gated on the same connection as the Gift and Tips cards above,
            // so the page never offers a link to a screen it has just said is
            // unavailable.
            portal_links = portal_links_card(
                site_url,
                &paid_tiers,
                stripe_on,
                state.settings.get_bool("recommendations_enabled"),
            ),
        ),
        true,
    )
    .into_response()
}

async fn membership_save(State(state): State<AppState>, headers: HeaderMap, Form(pairs): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Checkbox chips repeat under one name, so the body is read as pairs; the
    // map is only for the fields that appear once.
    let many = |k: &str| -> Vec<String> {
        pairs.iter().filter(|(n, _)| n == k).map(|(_, v)| v.clone()).collect()
    };
    let form: HashMap<String, String> = pairs.iter().cloned().collect();
    let get = |k: &str| form.get(k).cloned().unwrap_or_default();
    let one_of = |v: String, allowed: &[&str], fallback: String| {
        if allowed.contains(&v.as_str()) { v } else { fallback }
    };
    let access = one_of(get("members_signup_access"), &["all", "invite", "none"], "all".into());
    let visibility = one_of(
        get("default_content_visibility"),
        &["public", "members", "paid", "tiers"],
        "public".into(),
    );
    let style = one_of(get("portal_button_style"), &["icon-and-text", "icon", "text"], "icon-and-text".into());
    let commenting = one_of(get("comments_enabled"), &["off", "all", "paid"], "off".into());

    let mut ok = true;
    ok &= state.settings.set(&state.db, "members_signup_access", &serde_json::json!(access)).await.is_ok();
    ok &= state.settings.set(&state.db, "default_content_visibility", &serde_json::json!(visibility)).await.is_ok();
    ok &= state.settings.set(&state.db, "comments_enabled", &serde_json::json!(commenting)).await.is_ok();
    ok &= state.settings.set(&state.db, "portal_button_style", &serde_json::json!(style)).await.is_ok();
    for key in ["portal_button_signup_text", "portal_button_icon", "portal_signup_terms_html"] {
        ok &= state.settings.set(&state.db, key, &serde_json::json!(get(key))).await.is_ok();
    }
    for key in [
        "portal_button",
        "portal_name",
        "portal_signup_checkbox_required",
        "portal_signup_gift_promotion",
        "portal_account_gift_promotion",
    ] {
        let on = form.get(key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(on)).await.is_ok();
    }
    ok &= state
        .settings
        .set(&state.db, "members_support_address", &serde_json::json!(get("members_support_address").trim()))
        .await
        .is_ok();

    // Which tiers a "Specific tiers" default applies to. Only meaningful at
    // that setting, so it is cleared otherwise rather than left to go stale.
    let tiers = if visibility == "tiers" { many("default_content_visibility_tiers") } else { vec![] };
    ok &= state
        .settings
        .set(&state.db, "default_content_visibility_tiers", &serde_json::json!(tiers))
        .await
        .is_ok();

    // Portal plans. Ghost keeps a JSON array, and guarantees the default is
    // one of the offered prices — a default nobody can pick would leave the
    // portal preselecting a price it does not show.
    let plans: Vec<String> = many("portal_plans")
        .into_iter()
        .filter(|p| ["free", "monthly", "yearly"].contains(&p.as_str()))
        .collect();
    let default_plan = one_of(get("portal_default_plan"), &["monthly", "yearly"], "monthly".into());
    let default_plan = if plans.contains(&default_plan) {
        default_plan
    } else if plans.iter().any(|p| p == "monthly") {
        "monthly".to_string()
    } else if plans.iter().any(|p| p == "yearly") {
        "yearly".to_string()
    } else {
        default_plan
    };
    ok &= ghost_db::repos::tiers::set_free_signup_visibility(&state.db, plans.iter().any(|plan| plan == "free")).await.is_ok();
    ok &= state.settings.set(&state.db, "portal_plans", &serde_json::json!(plans)).await.is_ok();
    ok &= state
        .settings
        .set(&state.db, "portal_default_plan", &serde_json::json!(default_plan))
        .await
        .is_ok();

    // Private site. Turning it on with no code yet generates one, the way
    // Ghost does — a private site with an empty code would be one nobody,
    // including its owner, could get into.
    let private = get("is_private") == "true";
    ok &= state.settings.set(&state.db, "is_private", &serde_json::Value::Bool(private)).await.is_ok();
    if private {
        // "Regenerate" wins over whatever is in the box: the button means
        // "give me a new one", so a stale typed value must not override it.
        let regenerate = get("regenerate_code") == "1";
        let typed = get("password");
        let code = if regenerate {
            generate_access_code()
        } else {
            match typed.trim() {
                "" => state
                    .settings
                    .get_str("password")
                    .filter(|c| !c.trim().is_empty())
                    .unwrap_or_else(generate_access_code),
                t => t.to_string(),
            }
        };
        ok &= state.settings.set(&state.db, "password", &serde_json::json!(code)).await.is_ok();
    }

    // Spam filters. Stored as a JSON array, so the textarea is parsed rather
    // than saved verbatim.
    let domains = ghost_db::settings_cache::parse_blocked_email_domains(&get("blocked_email_domains"));
    ok &= state
        .settings
        .set(&state.db, "blocked_email_domains", &serde_json::json!(domains))
        .await
        .is_ok();

    // Tips & donations. Only written when the section was on screen — it is
    // hidden until Stripe is connected, and a hidden form posts nothing, which
    // would otherwise read back as "clear the amount".
    if form.contains_key("donations_currency") {
        let currency = one_of(
            get("donations_currency"),
            &CURRENCIES.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            "USD".into(),
        );
        // Shown in the major unit, stored in the minor one, and capped where
        // Stripe caps a preset amount.
        let amount = get("donations_suggested_amount").trim().parse::<i64>().unwrap_or(0);
        let cents = amount.clamp(0, 10_000) * 100;
        ok &= state.settings.set(&state.db, "donations_currency", &serde_json::json!(currency)).await.is_ok();
        ok &= state
            .settings
            .set(&state.db, "donations_suggested_amount", &serde_json::json!(cents))
            .await
            .is_ok();
    }

    // the primary newsletter's editable fields
    let nid = get("newsletter_id");
    if !nid.is_empty() {
        let now = ghost_core::dates::now_db();
        let res = sqlx::query(
            "UPDATE newsletters SET name = ?, description = ?, sender_name = ?, sender_email = ?, updated_at = ? WHERE id = ?",
        )
        .bind(get("newsletter_name"))
        .bind(get("newsletter_description"))
        .bind(get("newsletter_sender_name"))
        .bind(get("newsletter_sender_email"))
        .bind(&now)
        .bind(&nid)
        .execute(&state.db)
        .await;
        ok &= res.map(|r| r.rows_affected() > 0).unwrap_or(false);
    }

    if ok {
        settings_flash("membership", "ok", "Membership settings saved.")
    } else {
        settings_flash("membership", "error", "Saving failed — check the logs.")
    }
}

// --- custom fields (Ghost's member metafields) ------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct MetafieldNewForm {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
    member_access: String,
}

async fn metafield_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<MetafieldNewForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return settings_flash("membership", "error", "A custom field needs a name.");
    }
    // The key is minted from the name and then frozen: values are stored
    // against it, so it cannot follow a later rename.
    let key = ghost_core::metafields::key_from_name(&name);
    if !ghost_core::metafields::is_valid_key(&key) {
        return settings_flash(
            "membership",
            "error",
            "That name has no letters or digits to build a key from.",
        );
    }
    if !ghost_core::metafields::is_known_type(&form.field_type) {
        return settings_flash("membership", "error", "Choose a field type.");
    }
    let access = if ghost_core::metafields::is_member_access(&form.member_access) {
        form.member_access.as_str()
    } else {
        "none"
    };
    match repos::metafields::definition_conflict(&state.db, &key, &name, None).await {
        Ok(Some("key")) => {
            return settings_flash(
                "membership",
                "error",
                &format!("Another field already uses the key “{key}” — pick a different name."),
            )
        }
        Ok(Some(_)) => {
            return settings_flash("membership", "error", "Another field already has that name.")
        }
        Err(e) => return settings_flash("membership", "error", &e.message()),
        Ok(None) => {}
    }
    match repos::metafields::create_definition(&state.db, &key, &name, &form.field_type, access)
        .await
    {
        Ok(f) => settings_flash("membership", "ok", &format!("Added “{}”.", f.name)),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct MetafieldSaveForm {
    name: String,
    member_access: String,
    status: String,
}

async fn metafield_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<MetafieldSaveForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Ok(Some(_)) = repos::metafields::find_definition(&state.db, &id).await else {
        return settings_flash("membership", "error", "That custom field no longer exists.");
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return settings_flash("membership", "error", "A custom field needs a name.");
    }
    if let Ok(Some(_)) =
        repos::metafields::definition_conflict(&state.db, "", &name, Some(&id)).await
    {
        return settings_flash("membership", "error", "Another field already has that name.");
    }
    let access = if ghost_core::metafields::is_member_access(&form.member_access) {
        form.member_access.as_str()
    } else {
        "none"
    };
    let status = if form.status == "archived" { "archived" } else { "active" };
    match repos::metafields::update_definition(&state.db, &id, &name, access, status).await {
        Ok(f) => settings_flash("membership", "ok", &format!("Saved “{}”.", f.name)),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

async fn metafield_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::metafields::delete_definition(&state.db, &id).await {
        // The values cascade with it, which the confirmation said.
        Ok(()) => settings_flash("membership", "ok", "Custom field deleted."),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct MetafieldMoveForm {
    dir: String,
}

async fn metafield_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<MetafieldMoveForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::metafields::reorder_definition(&state.db, &id, form.dir == "up").await {
        Ok(()) => settings_flash("membership", "ok", "Order saved."),
        Err(e) => settings_flash("membership", "error", &e.message()),
    }
}

// --- growth: recommendations + signup forms (network & explore come later) ---

/// Everything the embeddable signup form is built from.
struct SignupFormEmbed<'a> {
    site_url: &'a str,
    /// `all-in-one` or `minimal`.
    layout: &'a str,
    background: &'a str,
    button: &'a str,
    title: &'a str,
    description: &'a str,
    icon: &'a str,
    locale: &'a str,
    labels: &'a [&'a str],
    /// A preview is the same embed sized to fill its frame instead of
    /// sitting in a page.
    preview: bool,
}

/// The `<script>` embed for the signup form, as Ghost's `generateCode` writes
/// it.
///
/// Three things here are load-bearing and were all wrong before. The script
/// comes from the CDN — the previous snippet pointed at
/// `{site}/public/signup-form.min.js`, which this server does not serve, so
/// every embed handed out was a 404 and the form never appeared. The script
/// is wrapped in a sized `<div>`, because the form renders into its container
/// and an unsized one collapses to nothing. And the two computed contrast
/// colours are sent: without `text-color` the copy on a dark background is
/// drawn dark on dark.
///
/// Attribute order follows Ghost's `preferredOrder` so a snippet from this
/// screen and one from Ghost diff cleanly.
fn signup_form_snippet(e: SignupFormEmbed<'_>) -> String {
    let all_in_one = e.layout != "minimal";
    let mut attrs: Vec<(String, String)> = Vec::new();

    if all_in_one {
        attrs.push(("background-color".into(), e.background.to_string()));
        attrs.push(("text-color".into(), ghost_render::color::contrast_text_color(e.background)));
    }
    attrs.push(("button-color".into(), e.button.to_string()));
    attrs.push(("button-text-color".into(), ghost_render::color::contrast_text_color(e.button)));
    if all_in_one {
        // Ghost sends these even when blank, so the form can tell "no title"
        // from "title not configured by this embed".
        attrs.push(("title".into(), e.title.to_string()));
        attrs.push(("description".into(), e.description.to_string()));
        if !e.icon.is_empty() {
            // Ghost asks the image server for a 192px square rather than
            // shipping the full-size icon to every embed.
            attrs.push(("icon".into(), icon_at_192(&ghost_core::url::base_asset(e.icon))));
        }
    }
    attrs.push(("site".into(), format!("{}/", e.site_url)));
    attrs.push(("locale".into(), e.locale.to_string()));
    for (i, label) in e.labels.iter().enumerate() {
        attrs.push((format!("label-{}", i + 1), (*label).to_string()));
    }

    let style = match (e.preview, all_in_one) {
        (false, true) => "height: 40vmin;min-height: 360px",
        (false, false) => "min-height: 58px;max-width: 440px;margin: 0 auto;width: 100%",
        (true, true) => "height: 100vh",
        (true, false) => {
            "min-height:58px;width:calc(100% - 48px);position:absolute;left:50%;top:50%;transform:translate(-50%, -50%)"
        }
    };

    let data: String =
        attrs.iter().map(|(k, v)| format!(r#" data-{k}="{}""#, esc(v))).collect();
    format!(
        r#"<div style="{style}"><script src="{src}"{data} async></script></div>"#,
        style = esc(style),
        src = ghost_render::apps::signup_form_script_url(),
        data = data,
    )
}

/// Rewrite a `/content/images/…` URL to ask for a 192px square, the way
/// Ghost's embed code does before handing the icon to the signup form.
///
/// Ghost's regex is `/content/images/(?:size/[^/]+/)*` — any size segment
/// already on the URL is dropped before the wanted one goes in, so asking
/// twice cannot produce `size/w192h192/size/w192h192/`. Written out by hand
/// because this workspace pulls in no regex crate, and this is the one
/// substitution that would need it.
fn icon_at_192(url: &str) -> String {
    const MARKER: &str = "/content/images/";
    let Some(at) = url.find(MARKER) else {
        return url.to_string();
    };
    let (head, mut rest) = url.split_at(at + MARKER.len());
    while let Some(stripped) = rest.strip_prefix("size/") {
        match stripped.find('/') {
            Some(slash) => rest = &stripped[slash + 1..],
            None => break,
        }
    }
    format!("{head}size/w192h192/{rest}")
}

async fn growth_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Each row is its own editing form, the way the custom-fields list works:
    // a recommendation is three short fields, so a separate page would be more
    // navigation than content.
    let mut rec_rows = String::new();
    if let Ok(recs) = sqlx::query_as::<_, (String, String, String, Option<String>, bool)>(
        "SELECT id, url, title, description, one_click_subscribe FROM recommendations ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    {
        if recs.is_empty() {
            rec_rows = r#"<div class="row"><div class="body"><span class="muted">No recommended sites yet — add one below.</span></div></div>"#.into();
        }
        for (id, url, title, description, one_click) in recs {
            rec_rows.push_str(&format!(
                r#"<div class="row" style="align-items:flex-start"><span class="ic tint-slate">{globe}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/growth/recommendations/{id}/" class="inline-form" style="align-items:flex-end">
<label class="field" style="flex:1;min-width:160px;margin:0"><input name="title" value="{title}" placeholder=" " maxlength="191" required><span>Title</span></label>
<label class="field" style="flex:1.4;min-width:200px;margin:0"><input name="url" value="{url}" placeholder=" " maxlength="2000" required><span>URL</span></label>
<button class="btn tonal small" type="submit">Save</button>
</form>
<label class="field" style="margin:10px 0 0"><input name="description" value="{desc}" placeholder=" " maxlength="2000" form="rec-{id}"><span>Short description</span></label>
<label class="fchip" style="margin-top:10px"><input type="checkbox" name="one_click_subscribe" value="1"{oc} form="rec-{id}"><span class="ck">{check}</span>One-click subscribe</label>
<form method="post" action="/ghost/settings/growth/recommendations/{id}/" id="rec-{id}"></form>
</div>
<div class="trailing" style="align-items:flex-start">
<form method="post" action="/ghost/settings/growth/recommendations/{id}/delete/" data-confirm="Remove recommendation|&#8220;{title}&#8221; will no longer be recommended to your members.">
<button class="icon-btn danger" title="Remove" aria-label="Remove">{del}</button></form></div></div>"#,
                globe = svg("globe"),
                id = esc(&id),
                title = esc(&title),
                url = esc(&url),
                desc = esc(description.as_deref().unwrap_or("")),
                oc = if one_click { " checked" } else { "" },
                check = svg("check"),
                del = svg("delete"),
            ));
        }
    }
    let rec_add = format!(
        r#"<div class="card padded rise" style="--i:2">
<form method="post" action="/ghost/settings/growth/recommendations/new/" class="inline-form">
<label class="field" style="flex:1;min-width:200px;margin:0"><input name="url" placeholder="https://example.com" maxlength="2000" required><span>URL</span></label>
<label class="field" style="flex:1;min-width:160px;margin:0"><input name="title" placeholder=" " maxlength="191"><span>Title · optional</span></label>
<button class="btn tonal" type="submit">{add} Add recommendation</button>
</form>
<p class="muted" style="margin:12px 0 0">Leave the title blank to use the site's domain. Recommendations appear to members when they subscribe.</p>
</div>"#,
        add = svg("add"),
    );
    let origin = state.site_url();

    // --- signup form builder -----------------------------------------------
    //
    // Server-rendered, so the controls are a GET form that re-renders this
    // page with the snippet rebuilt — there is no client-side state to keep in
    // step with what gets copied, and the preview below is literally the
    // snippet above it running.
    let sf_layout = q.get("sf_layout").cloned().unwrap_or_else(|| "all-in-one".into());
    let sf_layout = if sf_layout == "minimal" { "minimal" } else { "all-in-one" };
    let accent = state
        .settings
        .get_str("accent_color")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "#15171A".into());
    let sf_bg = q.get("sf_bg").cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "#F1F3F4".into());
    let sf_btn = q.get("sf_btn").cloned().filter(|c| !c.is_empty()).unwrap_or(accent);
    let sf_labels = q.get("sf_labels").cloned().unwrap_or_default();

    let signup_builder = {
        let site_title = state.settings.get_str("title").unwrap_or_default();
        let site_desc = state.settings.get_str("description").unwrap_or_default();
        let icon = state.settings.get_str("icon").unwrap_or_default();
        let locale = state.settings.get_str("locale").unwrap_or_else(|| "en".into());
        let labels: Vec<&str> =
            sf_labels.split(',').map(str::trim).filter(|l| !l.is_empty()).take(10).collect();

        let snippet = signup_form_snippet(SignupFormEmbed {
            site_url: origin.trim_end_matches('/'),
            layout: sf_layout,
            background: &sf_bg,
            button: &sf_btn,
            title: &site_title,
            description: &site_desc,
            icon: &icon,
            locale: &locale,
            labels: &labels,
            preview: false,
        });
        let preview_snippet = signup_form_snippet(SignupFormEmbed {
            site_url: origin.trim_end_matches('/'),
            layout: sf_layout,
            background: &sf_bg,
            button: &sf_btn,
            title: &site_title,
            description: &site_desc,
            icon: &icon,
            locale: &locale,
            labels: &labels,
            preview: true,
        });

        format!(
            r#"<div class="card padded rise" style="--i:3">
<form method="get" action="/ghost/settings/growth/" class="inline-form" style="align-items:flex-end">
<label class="field fixed" style="min-width:180px;margin:0"><select name="sf_layout" onchange="this.form.submit()">{layout_opts}</select><span>Layout</span>{car}</label>
{bg_field}
<label class="field" style="min-width:140px;margin:0"><input name="sf_btn" type="text" value="{btn}" placeholder=" "><span>Button color</span></label>
<label class="field" style="flex:1;min-width:180px;margin:0"><input name="sf_labels" value="{labels}" placeholder=" "><span>Labels at signup · comma separated</span></label>
<button class="btn tonal" type="submit">Update</button>
</form>
<p class="muted" style="margin:14px 0 8px">Preview — this is the snippet below, running:</p>
<iframe class="sf-preview{minimal}" title="Signup form preview" sandbox="allow-scripts allow-popups allow-forms" srcdoc="{preview_doc}"></iframe>
<p class="muted" style="margin:16px 0 8px">Embed this on any external site to collect members for <b>{origin_disp}</b>:</p>
<div class="key" style="align-items:flex-start"><span class="chip tonal">embed</span><pre style="margin:0;white-space:pre-wrap;word-break:break-all;font:inherit">{snippet_disp}</pre>
<button type="button" data-copy="{snippet_copy}" title="Copy" aria-label="Copy snippet">{copy}</button></div>
<p class="muted" style="margin:12px 0 0">{layout_note} Anyone signing up through this form is tagged with the labels above, so you can tell one embed's members from another's.</p>
</div>"#,
            layout_opts = select_options(
                &[("all-in-one", "All-in-one · title, description and icon"), ("minimal", "Minimal · just the form")],
                sf_layout,
            ),
            car = SELECT_CARET,
            // Ghost only offers a background colour for the all-in-one
            // layout; the minimal one is meant to sit on whatever is behind
            // it, so the embed never sets a background at all.
            bg_field = if sf_layout == "minimal" {
                String::new()
            } else {
                format!(
                    r#"<label class="field" style="min-width:140px;margin:0"><input name="sf_bg" type="text" value="{bg}" placeholder=" "><span>Background color</span></label>"#,
                    bg = esc(&sf_bg),
                )
            },
            btn = esc(&sf_btn),
            labels = esc(&sf_labels),
            origin_disp = esc(&origin),
            minimal = if sf_layout == "minimal" { " minimal" } else { "" },
            // The preview runs the real CDN script against this site, inside
            // a sandboxed frame: same-origin is withheld, so it cannot reach
            // the admin session that rendered it.
            preview_doc = esc(&format!(
                r#"<!doctype html><html><head><meta charset="utf-8"><style>html,body{{margin:0;height:100%}}</style></head><body>{preview_snippet}</body></html>"#,
            )),
            snippet_disp = esc(&snippet),
            snippet_copy = esc(&snippet),
            copy = svg("copy"),
            layout_note = if sf_layout == "minimal" {
                "The minimal layout renders the email field and button alone, for dropping into your own section."
            } else {
                "The all-in-one layout carries your site title, description and icon, taken from Settings → General and Design."
            },
        )
    };


    // --- offers: Ghost only shows these once Stripe is connected, since an
    // offer is a discount on a paid tier and nothing else ---------------------
    let offers_card = if ghost_stripe::StripeConfig::from_settings(&state.settings).is_some() {
        let offers = repos::offers::list(&state.db).await.unwrap_or_default();
        let mut rows = String::new();
        for o in &offers {
            let redemptions = repos::offers::redemption_count(&state.db, &o.id).await;
            rows.push_str(&format!(
                r#"<a class="row" href="/ghost/settings/growth/offers/{id}/"><span class="ic tint-green">{tag}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{summary} · code <code>{code}</code> · {redemptions} redemption{plural}</span></div>
<div class="trailing">{archived}{chev}</div></a>"#,
                id = esc(&o.id),
                tag = svg("tags"),
                name = esc(&o.name),
                summary = esc(&o.summary()),
                code = esc(&o.code),
                redemptions = redemptions,
                plural = if redemptions == 1 { "" } else { "s" },
                archived = if o.active {
                    String::new()
                } else {
                    r#"<span class="chip draft">archived</span>"#.to_string()
                },
                chev = svg("chevron"),
            ));
        }
        if rows.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">No offers yet — add one to give readers a discount link.</span></div></div>"#.into();
        }
        format!(
            r#"<div class="section">Offers</div>
<div class="card rows rise" style="--i:3">{rows}</div>
<div style="padding:2px 4px 8px"><form method="post" action="/ghost/settings/growth/offers/new/">
<button class="btn tonal small" type="submit">{add} Add offer</button></form></div>
<p class="muted" style="margin:4px 0 0 16px">An offer is a discount on one paid tier, redeemed through a link you share. Archiving one stops new redemptions; members already on it keep their price.</p>"#,
            rows = rows,
            add = svg("add"),
        )
    } else {
        String::new()
    };

    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "growth").unwrap();
    page_for_role(actor_role, 
        "Growth",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/growth/" id="growth-form">
<div class="section">Recommendations</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-slate">{heart}</span>
<div class="body"><span class="title">Enable recommendations</span><span class="detail">Show recommended publications to your members and accept incoming recommendations</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="recommendations_enabled" value="1" {rec_ck}><span class="track"><span class="thumb"></span></span></label></div></div>
</div>
<div class="card rows rise" style="--i:2">{rec_rows}</div>
</form>
{rec_add}
<div class="section">Signup forms</div>
{signup_builder}
{offers_card}
<div class="section">Ghost Explore</div>
<form method="post" action="/ghost/settings/growth/explore/" id="explore-form">
<div class="card rows rise" style="--i:5">{explore_rows}</div>
<div style="padding:10px 4px 0"><button class="btn tonal small" type="submit">Save</button></div>
</form>
<p class="muted" style="margin:10px 2px 0">Ghost Explore is a directory of publications at <a href="https://ghost.org/explore/" target="_blank" rel="noopener">ghost.org/explore</a>.
When enabled, the publication synchronizes public publication details with Explore. Growth statistics are shared only when separately enabled. Private sites and local addresses are excluded.</p>
<div class="section">Social web</div>
<div class="card rows rise" style="--i:6">
{network}
</div>"#,
            head = group_head(group, Some("growth-form")),
            banner = flash_banner(&q),
            offers_card = offers_card,
            heart = svg("star"),
            rec_ck = if state.settings.get_bool("recommendations_enabled") { "checked" } else { "" },
            rec_rows = rec_rows,
            rec_add = rec_add,
            signup_builder = signup_builder,
            network = format!(r#"<a class="row" href="/ghost/social/"><span class="ic tint-teal">{}</span><div class="body"><span class="title">Social web</span><span class="detail">Reader, notes, follows and publication federation</span></div>{}</a>"#,svg("share"),svg("chevron")),
            explore_rows = format!(
                "{}{}",
                toggle_row(
                    "explore_ping",
                    "List this site on Ghost Explore",
                    "Announce the site to Ghost's public directory of publications",
                    state.settings.get_bool("explore_ping"),
                ),
                toggle_row(
                    "explore_ping_growth",
                    "Share growth data to rank higher?",
                    "Uses your revenue and member growth to rank the site more highly. Total member count is shown publicly; the rest is not.",
                    state.settings.get_bool("explore_ping_growth"),
                ),
            ),
        ),
        true,
    )
    .into_response()
}

async fn growth_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.get("recommendations_enabled").map(|v| v == "1").unwrap_or(false);
    match state.settings.set(&state.db, "recommendations_enabled", &serde_json::Value::Bool(on)).await {
        Ok(()) => settings_flash("growth", "ok", "Growth settings saved."),
        Err(e) => settings_flash("growth", "error", &e.message()),
    }
}

/// Ghost Explore's two opt-ins, saved on their own so they cannot clear the
/// recommendations switch that lives in a different form on the same page.
async fn growth_explore_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in ["explore_ping", "explore_ping_growth"] {
        let on = form.get(key).map(|v| v == "1").unwrap_or(false);
        ok &= state.settings.set(&state.db, key, &serde_json::Value::Bool(on)).await.is_ok();
    }
    if ok {
        settings_flash("growth", "ok", "Ghost Explore settings saved.")
    } else {
        settings_flash("growth", "error", "Saving failed — check the logs.")
    }
}

// --- recommendations ---------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RecommendationForm {
    url: String,
    title: String,
    description: String,
    one_click_subscribe: String,
}

/// Normalise a recommended site's URL, and refuse one that is not a site.
///
/// A bare domain gets `https://`, because that is what someone types.
fn recommendation_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let with_scheme = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    // A host with a dot in it is the bar: `https://localhost` is a typo here,
    // not a publication worth recommending to members.
    let host = with_scheme.split("//").nth(1)?.split('/').next()?;
    (host.contains('.') && host.len() > 3).then_some(with_scheme)
}

/// Fall back to the domain, the way Ghost titles a recommendation it could
/// not fetch a name for.
fn recommendation_title(title: &str, url: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    url.split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .map(|h| h.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| "Recommended site".into())
}

async fn recommendation_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<RecommendationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(url) = recommendation_url(&form.url) else {
        return settings_flash("growth", "error", "Enter the full address of the site to recommend.");
    };
    // `url` is unique in Ghost — recommending the same site twice says nothing
    // the first one did not.
    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recommendations WHERE url = ?")
        .bind(&url)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    if taken > 0 {
        return settings_flash("growth", "error", "That site is already recommended.");
    }
    let now = ghost_core::dates::now_db();
    let res = sqlx::query(
        "INSERT INTO recommendations (id, url, title, description, one_click_subscribe, featured, created_at, updated_at) \
         VALUES (?, ?, ?, '', 0, 0, ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(&url)
    .bind(recommendation_title(&form.title, &url))
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => settings_flash("growth", "ok", "Recommendation added."),
        Err(e) => settings_flash("growth", "error", &format!("Could not add it: {e}")),
    }
}

async fn recommendation_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<RecommendationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // The row carries two forms — one for title/url, one for the description
    // and the subscribe flag — so a field the submitted form did not include
    // keeps its stored value rather than being blanked.
    let existing: Option<(String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT url, title, description, one_click_subscribe FROM recommendations WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((old_url, old_title, old_desc, old_oc)) = existing else {
        return settings_flash("growth", "error", "That recommendation no longer exists.");
    };

    let url = if form.url.trim().is_empty() {
        old_url
    } else {
        match recommendation_url(&form.url) {
            Some(u) => u,
            None => return settings_flash("growth", "error", "That does not look like a site address."),
        }
    };
    let title = if form.title.trim().is_empty() {
        old_title
    } else {
        recommendation_title(&form.title, &url)
    };
    // Only the form that carries the description may change it.
    let description = if form.description.trim().is_empty() && form.one_click_subscribe.is_empty() {
        old_desc.unwrap_or_default()
    } else {
        form.description.trim().to_string()
    };
    let one_click = if form.description.trim().is_empty() && form.one_click_subscribe.is_empty() {
        old_oc
    } else {
        form.one_click_subscribe == "1"
    };

    let res = sqlx::query(
        "UPDATE recommendations SET url = ?, title = ?, description = ?, one_click_subscribe = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&url)
    .bind(&title)
    .bind(&description)
    .bind(one_click)
    .bind(ghost_core::dates::now_db())
    .bind(&id)
    .execute(&state.db)
    .await;
    match res {
        Ok(_) => settings_flash("growth", "ok", "Recommendation saved."),
        Err(e) => settings_flash("growth", "error", &format!("Could not save it: {e}")),
    }
}

async fn recommendation_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let _ = sqlx::query("DELETE FROM recommendations WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await;
    settings_flash("growth", "ok", "Recommendation removed.")
}

// --- offers -----------------------------------------------------------------

fn offer_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/growth/offers/{id}/?{encoded}")).into_response()
}

/// Ghost's own slugification of an offer code: lower case, spaces to dashes,
/// nothing else — the code ends up in a URL.
fn offer_code(raw: &str) -> String {
    ghost_core::slug::safe_string(raw)
}

/// Create a blank offer and open it, the way "Add tier" works.
///
/// A new offer starts archived so a half-filled code is never live, and
/// carries no Stripe coupon until its terms are saved.
async fn offer_new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // An offer discounts a tier, so there has to be a paid one to discount.
    let tier: Option<String> = sqlx::query_scalar(
        "SELECT id FROM products WHERE type = 'paid' AND active = 1 ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(tier) = tier else {
        return settings_flash(
            "growth",
            "error",
            "Add a paid tier first — an offer is a discount on one.",
        );
    };

    // Unique by construction: the name and code both carry a short suffix, so
    // adding several in a row never collides.
    let suffix = ghost_core::ids::random_below(100_000);
    let input = repos::offers::OfferInput {
        name: format!("New offer {suffix}"),
        code: format!("offer-{suffix}"),
        product_id: tier,
        interval: "month".into(),
        discount_type: "percent".into(),
        discount_amount: 10,
        duration: "once".into(),
        duration_in_months: None,
        currency: None,
        portal_title: "Special offer".into(),
        portal_description: String::new(),
        stripe_coupon_id: None,
        active: false,
    };
    match repos::offers::create(&state.db, &input).await {
        Ok(o) => Redirect::to(&format!("/ghost/settings/growth/offers/{}/", o.id)).into_response(),
        Err(e) => settings_flash("growth", "error", &e.message()),
    }
}

async fn offer_edit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(offer)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };
    let redemptions = repos::offers::redemption_count(&state.db, &id).await;
    // Terms are frozen once anyone has redeemed: the discount lives in a
    // Stripe coupon their subscription already carries, so changing it here
    // would describe something different from what they are being charged.
    let locked = redemptions > 0;

    let tiers: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM products WHERE type = 'paid' ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let tier_opts: String = tiers
        .iter()
        .map(|(tid, name)| {
            format!(
                r#"<option value="{v}"{sel}>{n}</option>"#,
                v = esc(tid),
                n = esc(name),
                sel = if *tid == offer.product_id { " selected" } else { "" },
            )
        })
        .collect();

    let site_url = state.site_url();
    let share = format!("{}/{}", site_url.trim_end_matches('/'), offer.code);

    let dis = if locked { " disabled" } else { "" };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "growth").unwrap();
    let _ = group;

    page_for_role(actor_role,
        "Offer",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/growth/" title="Back to growth">{back}</a>
<div><h1>{name}</h1><p class="sub">{summary} · {status}</p></div>
<div class="actions"><button class="btn" type="submit" form="offer-form">{check} Save</button></div></header>
{banner}
<form method="post" action="/ghost/settings/growth/offers/{id}/" id="offer-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{tag}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " maxlength="191" required><span>Internal name · only you see this</span></label>
<label class="field" style="margin:0"><input name="code" value="{code}" placeholder=" " maxlength="191" required><span>Code · the end of the share link</span></label>
</div>
<p class="muted" style="margin:10px 0 0">Share link: <a href="{share}" target="_blank" rel="noopener">{share}</a></p>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{notes}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><input name="portal_title" value="{portal_title}" placeholder=" " maxlength="191"><span>Display title · shown in the portal</span></label>
<label class="field" style="margin:12px 0 0"><textarea name="portal_description" rows="2" placeholder=" " maxlength="2000">{portal_description}</textarea><span>Display description</span></label>
</div></div>
</div>
<div class="section">Discount</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-green">{coin}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field fixed" style="margin:0"><select name="product_id"{dis}>{tier_opts}</select><span>Tier</span>{car}</label>
<label class="field fixed" style="margin:0"><select name="interval"{dis}>{cadence_opts}</select><span>Cadence</span>{car}</label>
<label class="field fixed" style="margin:0"><select name="discount_type"{dis}>{type_opts}</select><span>Type</span>{car}</label>
<label class="field" style="margin:0"><input name="discount_amount" type="number" min="1" step="1" value="{amount}" placeholder=" "{dis}><span>{amount_label}</span></label>
<label class="field fixed" style="margin:0"><select name="duration"{dis}>{duration_opts}</select><span>Duration</span>{car}</label>
<label class="field" style="margin:0"><input name="duration_in_months" type="number" min="1" step="1" value="{months}" placeholder=" "{dis}><span>Months · repeating only</span></label>
</div>
<p class="muted" style="margin:10px 0 0">{terms_note}</p>
</div></div>
</div>
</form>
<div class="section">Status</div>
<div class="card rows rise" style="--i:3">
<div class="row"><span class="ic tint-amber">{flag}</span>
<div class="body"><span class="title">{status_title}</span><span class="detail">{status_detail}</span></div>
<div class="trailing"><form method="post" action="/ghost/settings/growth/offers/{id}/status/">
<input type="hidden" name="active" value="{next}">
<button class="btn tonal small" type="submit">{status_btn}</button></form></div></div>
<div class="row"><span class="ic tint-slate">{people}</span>
<div class="body"><span class="title">Redemptions</span><span class="detail">{redemptions} member{plural} signed up through this offer</span></div></div>
</div>"#,
            back = svg("chevron"),
            check = svg("check"),
            banner = flash_banner(&q),
            id = esc(&id),
            name = esc(&offer.name),
            name_v = esc(&offer.name),
            summary = esc(&offer.summary()),
            status = offer.status(),
            code = esc(&offer.code),
            share = esc(&share),
            tag = svg("tags"),
            notes = svg("notes"),
            coin = svg("ghost"),
            flag = svg("flag"),
            people = svg("person"),
            car = SELECT_CARET,
            dis = dis,
            tier_opts = tier_opts,
            cadence_opts = select_options(
                &[("month", "Monthly"), ("year", "Yearly")],
                offer.interval.as_deref().unwrap_or("month"),
            ),
            type_opts = select_options(
                &[("percent", "Percentage off"), ("fixed", "Amount off"), ("trial", "Free trial")],
                offer.discount_type.as_deref().unwrap_or("percent"),
            ),
            amount = offer.discount_amount.unwrap_or(0),
            amount_label = match offer.discount_type.as_deref() {
                Some("trial") => "Trial days",
                Some("fixed") => "Amount off · in cents",
                _ => "Percent off · 1-100",
            },
            duration_opts = select_options(
                &[("once", "Once"), ("repeating", "Repeating"), ("forever", "Forever")],
                offer.duration.as_deref().unwrap_or("once"),
            ),
            months = offer.duration_in_months.map(|m| m.to_string()).unwrap_or_default(),
            portal_title = esc(offer.portal_title.as_deref().unwrap_or("")),
            portal_description = esc(offer.portal_description.as_deref().unwrap_or("")),
            terms_note = if locked {
                "Someone has already redeemed this offer, so its terms are fixed — their subscription is priced by the Stripe coupon behind it. Create a new offer to change the discount."
            } else {
                "A free trial ignores the duration. Saving mints a fresh Stripe coupon for these terms."
            },
            status_title = if offer.active { "Active" } else { "Archived" },
            status_detail = if offer.active {
                "The share link works and new members can redeem it."
            } else {
                "The share link no longer redeems. Members already on it keep their price."
            },
            next = if offer.active { "0" } else { "1" },
            status_btn = if offer.active { "Archive" } else { "Activate" },
            redemptions = redemptions,
            plural = if redemptions == 1 { "" } else { "s" },
        ),
        true,
    )
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct OfferForm {
    name: String,
    code: String,
    product_id: String,
    interval: String,
    discount_type: String,
    discount_amount: String,
    duration: String,
    duration_in_months: String,
    portal_title: String,
    portal_description: String,
}

async fn offer_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<OfferForm>,
) -> Response {
    let user = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a.user,
        Err(r) => return r,
    };
    let Ok(Some(existing)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return offer_flash(&id, "error", "An offer needs a name.");
    }
    let code = offer_code(&form.code);
    if code.is_empty() {
        return offer_flash(&id, "error", "An offer needs a code — it becomes the share link.");
    }
    // Both are unique in Ghost, and the code is a public URL, so a clash is
    // reported rather than left to the database constraint.
    match repos::offers::conflict(&state.db, &name, &code, Some(&id)).await {
        Ok(Some("name")) => return offer_flash(&id, "error", "Another offer already uses that name."),
        Ok(Some(_)) => return offer_flash(&id, "error", "Another offer already uses that code."),
        Err(e) => return offer_flash(&id, "error", &e.message()),
        Ok(None) => {}
    }

    let locked = repos::offers::redemption_count(&state.db, &id).await > 0;
    let mut input = repos::offers::OfferInput {
        name,
        code,
        portal_title: form.portal_title.trim().to_string(),
        portal_description: form.portal_description.trim().to_string(),
        active: existing.active,
        // Carried over; only replaced below when a new coupon is minted.
        stripe_coupon_id: None,
        product_id: existing.product_id.clone(),
        interval: existing.interval.clone().unwrap_or_else(|| "month".into()),
        discount_type: existing.discount_type.clone().unwrap_or_else(|| "percent".into()),
        discount_amount: existing.discount_amount.unwrap_or(0),
        duration: existing.duration.clone().unwrap_or_else(|| "once".into()),
        duration_in_months: existing.duration_in_months,
        currency: existing.currency.clone(),
    };

    if locked {
        // Presentation only — the terms inputs were disabled on the page.
        return match repos::offers::update(&state.db, &id, &input).await {
            Ok(_) => offer_flash(&id, "ok", "Offer saved."),
            Err(e) => offer_flash(&id, "error", &e.message()),
        };
    }

    let one_of = |v: &str, allowed: &[&str], fallback: &str| {
        if allowed.contains(&v) { v.to_string() } else { fallback.to_string() }
    };
    input.product_id = form.product_id.trim().to_string();
    input.interval = one_of(form.interval.trim(), &["month", "year"], "month");
    input.discount_type = one_of(form.discount_type.trim(), &["percent", "fixed", "trial"], "percent");
    input.duration = one_of(form.duration.trim(), &["once", "repeating", "forever"], "once");
    let amount: i64 = form.discount_amount.trim().parse().unwrap_or(0);
    input.duration_in_months = form.duration_in_months.trim().parse::<i64>().ok().filter(|m| *m > 0);

    // The tier has to exist and be paid — the select is built from that list,
    // but the form is a POST like any other.
    let tier: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, currency FROM products WHERE id = ? AND type = 'paid' LIMIT 1",
    )
    .bind(&input.product_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((_, tier_currency)) = tier else {
        return offer_flash(&id, "error", "Choose a paid tier for this offer.");
    };

    match input.discount_type.as_str() {
        "percent" if !(1..=100).contains(&amount) => {
            return offer_flash(&id, "error", "A percentage discount must be between 1 and 100.")
        }
        "trial" if amount < 1 => {
            return offer_flash(&id, "error", "A free trial needs at least one day.")
        }
        "fixed" if amount < 1 => {
            return offer_flash(&id, "error", "An amount discount must be more than zero.")
        }
        _ => {}
    }
    if input.discount_type == "repeating" && input.duration_in_months.is_none() {
        return offer_flash(&id, "error", "A repeating offer needs a number of months.");
    }
    input.discount_amount = amount;

    // A trial is not a coupon at all: Stripe applies it as the subscription's
    // trial period, so the duration is its own thing.
    if input.discount_type == "trial" {
        input.duration = "trial".into();
        input.duration_in_months = None;
        input.currency = None;
    } else if input.discount_type == "fixed" {
        // Stripe requires a currency on an amount-off coupon, and it has to be
        // the one the tier charges in or the discount cannot apply.
        input.currency = tier_currency.clone().or_else(|| Some("usd".into()));
    } else {
        input.currency = None;
    }
    if input.duration != "repeating" {
        input.duration_in_months = None;
    }

    // Mint a coupon for the new terms. Stripe coupons cannot be edited, so a
    // changed discount is always a new one; the old is left in place for the
    // subscriptions already carrying it.
    if input.discount_type != "trial" {
        let Some(cfg) = ghost_stripe::StripeConfig::from_settings(&state.settings) else {
            return offer_flash(&id, "error", "Connect Stripe before saving an offer's discount.");
        };
        let discount = if input.discount_type == "fixed" {
            ghost_stripe::Discount::Fixed {
                amount_cents: input.discount_amount,
                currency: input.currency.as_deref().unwrap_or("usd"),
            }
        } else {
            ghost_stripe::Discount::Percent(input.discount_amount)
        };
        match ghost_stripe::create_coupon(
            &cfg,
            &input.name,
            discount,
            &input.duration,
            input.duration_in_months,
        )
        .await
        {
            Ok(coupon) => input.stripe_coupon_id = Some(coupon),
            Err(e) => return offer_flash(&id, "error", &format!("Stripe: {}", e.message())),
        }
    }

    // The terms live outside `update`, which only touches presentation, so
    // they are written here in one statement alongside it.
    let now = ghost_core::dates::now_db();
    let wrote = sqlx::query(
        "UPDATE offers SET product_id = ?, interval = ?, discount_type = ?, discount_amount = ?, \
         duration = ?, duration_in_months = ?, currency = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&input.product_id)
    .bind(&input.interval)
    .bind(&input.discount_type)
    .bind(input.discount_amount)
    .bind(&input.duration)
    .bind(input.duration_in_months)
    .bind(&input.currency)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await
    .is_ok();
    if !wrote {
        return offer_flash(&id, "error", "Saving the discount failed — check the logs.");
    }
    match repos::offers::update(&state.db, &id, &input).await {
        Ok(_) => offer_flash(&id, "ok", "Offer saved."),
        Err(e) => offer_flash(&id, "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct OfferStatusForm {
    active: String,
}

async fn offer_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<OfferStatusForm>,
) -> Response {
    let user = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a.user,
        Err(r) => return r,
    };
    let Ok(Some(offer)) = repos::offers::find_by_id(&state.db, &id).await else {
        return not_found_page(&user, "That offer could not be found.");
    };
    let active = form.active == "1";
    // An offer with no coupon has never had its terms saved, so there is
    // nothing for a redeemer to actually get.
    if active && offer.discount_type.as_deref() != Some("trial") && offer.stripe_coupon_id.is_none()
    {
        return offer_flash(&id, "error", "Save the discount first — this offer has no Stripe coupon yet.");
    }
    match repos::offers::set_active(&state.db, &id, active).await {
        Ok(()) => offer_flash(&id, "ok", if active { "Offer activated." } else { "Offer archived." }),
        Err(e) => offer_flash(&id, "error", &e.message()),
    }
}

// --- advanced: integrations (existing page), import/export, code injection, labs ---

async fn advanced_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    // Beta and private feature flags used to sit here; they live on the
    // Features page now, where the rest of what a publisher can switch on and
    // off already is. What is left on this page is plumbing — import, export,
    // redirects, routes — and none of it posts a form, so the header carries
    // no Save.
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "advanced").unwrap();
    page_for_role(actor_role,
        "Advanced",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="section">Redirects &amp; routes</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{redirects_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Redirects</span>
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/settings/advanced/redirects/">Download</a>
<form method="post" action="/ghost/settings/advanced/redirects/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0">
<input type="file" name="file" accept=".yaml,.yml,.json" required class="filefield">
<button class="btn tonal small" type="submit">Upload redirects</button></form></div>
<p class="muted" style="margin:10px 0 0">Ghost-format redirects, YAML or JSON — <code>301: /old/ -> /new/</code>. The file is validated before anything takes effect.</p></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{routes_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Routes</span>
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/settings/advanced/routes/">Download</a>
<form method="post" action="/ghost/settings/advanced/routes/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0">
<input type="file" name="file" accept=".yaml,.yml" required class="filefield">
<button class="btn tonal small" type="submit">Upload routes</button></form></div>
<p class="muted" style="margin:10px 0 0">routes.yaml defines the site's URL structure — collections, channels and custom templates. Uploading rebuilds routing and the active theme immediately.</p></div></div>
</div>
<p class="muted" style="margin:14px 2px 0">Looking for something else? Beta and private features are in <a href="/ghost/settings/features/">Settings → Features</a>, and importing and exporting moved to <a href="/ghost/settings/migration/">Settings → Migration tools</a>.</p>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            redirects_ic = svg("link"),
            routes_ic = svg("layout"),
        ),
        true,
    )
    .into_response()
}

// --- migration tools: import and export ------------------------------------

/// The platforms a publisher most often arrives from, and what to feed the
/// universal importer with in each case.
///
/// Ghost answers this with a hosted service at `migrate.ghost.org`, embedded
/// in an iframe and handed an Admin API key. A self-hosted port cannot stand
/// that service up, and handing a third party an owner-level key to do it
/// would be a poor trade — so these are directions to the file each platform
/// exports, which the universal importer below reads.
const MIGRATION_SOURCES: &[(&str, &str)] = &[
    ("Ghost", "Settings → Migration tools → Export on the old site gives the JSON or zip this screen reads directly."),
    ("WordPress", "Install the Ghost plugin on WordPress and use its export, which writes a Ghost-format zip."),
    ("Substack", "Settings → Exports → New export. Unzip it; the posts CSV imports here, the rest is already HTML."),
    ("Medium", "Settings → Account → Download your information. The archive holds one HTML file per post."),
    ("Squarespace", "Settings → Import & Export → Export, which produces a WordPress-format file."),
];

async fn migration_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "migration").unwrap();

    // What the archive would actually contain, so the checkbox list can say
    // how much there is rather than only what there is.
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM members").fetch_one(&state.db).await.unwrap_or(0);
    let post_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE type = 'post'")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let page_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE type = 'page'")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let theme_count = ghost_api::themes::list(&state).len();

    let components: String = EXPORT_COMPONENTS
        .iter()
        .map(|(key, label, detail, default_on)| {
            format!(
                r#"<label class="fchip exp"><input type="checkbox" name="{key}" value="1"{on}><span class="ck">{check}</span>
<span class="expl"><b>{label}</b><span>{detail}</span></span></label>"#,
                key = esc(key),
                on = if *default_on { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
                detail = esc(detail),
            )
        })
        .collect();

    let sources: String = MIGRATION_SOURCES
        .iter()
        .map(|(platform, how)| {
            format!(
                r#"<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{platform}</span><span class="detail">{how}</span></div></div>"#,
                ic = svg("globe"),
                platform = esc(platform),
                how = esc(how),
            )
        })
        .collect();

    page_for_role(
        actor_role,
        "Migration tools",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="mtabs">
<input type="radio" name="mtab" id="mtab-import" checked>
<input type="radio" name="mtab" id="mtab-export">
<div class="mtab-bar"><label for="mtab-import">Import</label><label for="mtab-export">Export</label></div>

<div class="mtab-panel" data-tab="import">
<div class="section">Universal import</div>
<div class="card padded rise" style="--i:1">
<form method="post" action="/ghost/settings/migration/import/" enctype="multipart/form-data" class="inline-form" style="align-items:center">
<input type="file" name="file" accept=".json,.zip,application/json,application/zip" required class="filefield">
<button class="btn tonal" type="submit">{upload} Import</button>
</form>
<p class="muted" style="margin:12px 0 0">A Ghost export — either the <code>.json</code> file or a <code>.zip</code> holding one.
A zip's <code>content/images</code> and <code>content/files</code> are restored alongside the content, so posts keep their pictures.</p>
<p class="muted" style="margin:8px 0 0">Posts, pages and tags are added; Markdown is re-rendered on the way in. Nothing already on this
site is replaced or removed, so importing the same file twice gives you two copies rather than one updated one.</p>
</div>
<div class="section">Members</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-blue">{people}</span>
<div class="body"><span class="title">Import members from CSV</span><span class="detail">Map the columns of any CSV onto member fields, labels and tiers</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/">Go to Members</a></div></div>
</div>
<div class="section">Coming from another platform</div>
<div class="card rows rise" style="--i:3">{sources}</div>
<p class="muted" style="margin:10px 2px 0">Ghost(Pro) runs a hosted migration service for these. This is a self-hosted site, so
the route is the same but manual: export from the old platform, then feed the file to the importer above.</p>
</div>

<div class="mtab-panel" data-tab="export">
<div class="section">Export all data</div>
<form class="card padded rise" style="--i:1" method="get" action="/ghost/settings/migration/export/">
<p class="muted" style="margin:0 0 14px">Pick what to put in the archive. This is the one to take before an upgrade, or when moving the site somewhere else.</p>
<div class="expgrid">{components}</div>
<div style="margin-top:16px"><button class="btn" type="submit">{download} Download archive</button></div>
</form>
<div class="section">Individual files</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-blue">{notes}</span>
<div class="body"><span class="title">Content &amp; settings</span><span class="detail">{posts} posts and {pages} pages, with their tags · JSON</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/content/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{people}</span>
<div class="body"><span class="title">Members</span><span class="detail">{members} members with labels, tiers and Stripe ids · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/export/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{chart}</span>
<div class="body"><span class="title">Post analytics</span><span class="detail">Sends, opens, views and feedback, one row per post · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/analytics/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{themes_ic}</span>
<div class="body"><span class="title">Themes</span><span class="detail">{themes} installed, including anything edited in the browser · ZIP</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/themes/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{link}</span>
<div class="body"><span class="title">Redirects &amp; routes</span><span class="detail">The two YAML files, downloadable one at a time</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/advanced/">Go to Advanced</a></div></div>
</div>
<p class="muted" style="margin:10px 2px 0">An export is a copy, not a backup of the database itself — it holds your content, members and
files, not sessions, API keys or password hashes.</p>
</div>
</div>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            upload = svg("upload"),
            download = svg("import"),
            people = svg("person"),
            notes = svg("notes"),
            chart = svg("chart"),
            themes_ic = svg("themes"),
            link = svg("link"),
            sources = sources,
            components = components,
            posts = post_count,
            pages = page_count,
            members = member_count,
            themes = theme_count,
        ),
        true,
    )
    .into_response()
}

// --- code injection: a page of its own under Settings ------------------

async fn code_injection_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "code-injection").unwrap();
    page_for_role(actor_role,
        "Code injection",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/code-injection/" id="codeinjection-form">
<div class="section">Site head</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{code}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="codeinjection_head" class="code" rows="8" placeholder=" ">{cih}</textarea><span>Injected into &lt;head&gt; on every page</span></label></div></div>
</div>
<div class="section">Site foot</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{code}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="codeinjection_foot" class="code" rows="8" placeholder=" ">{cif}</textarea><span>Injected before &lt;/body&gt; on every page</span></label></div></div>
</div>
</form>
<p class="muted" style="margin:10px 2px 0">Styles, scripts or analytics for the whole site — pasted as-is, so tags and quotes
stay exactly as written.</p>"#,
            head = group_head(group, Some("codeinjection-form")),
            banner = flash_banner(&q),
            code = svg("code"),
            cih = s("codeinjection_head"),
            cif = s("codeinjection_foot"),
        ),
        true,
    )
    .into_response()
}

async fn code_injection_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in ["codeinjection_head", "codeinjection_foot"] {
        let value = form.get(key).cloned().unwrap_or_default();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    if ok {
        settings_saved(&state, "code-injection", "Code injection saved.")
    } else {
        settings_flash("code-injection", "error", "Saving failed — check the logs.")
    }
}

/// Save the feature flags, stored as keys on the `labs` settings object —
/// the same object Ghost writes, so a flag set here is the flag a Ghost
/// theme or export expects.
///
/// Unchecked switches post nothing, so each rendered row carries a `shown`
/// marker and only those are rewritten — a flag whose row was not on the page
/// (the private ones outside development) keeps whatever it held, and a flag
/// set by a newer Ghost that this build has no row for survives untouched.
// --- export / import (Ghost-format JSON) ---

/// Ghost export shape: `{db: [{meta: {…}, data: {posts, tags, users, …}}]}`.
/// Kept as a pure function over the fetched rows so it can be unit-tested.
fn build_ghost_export(posts: &[ghost_core::models::Post], tags: &[ghost_core::models::Tag], site_url: &str) -> serde_json::Value {
    let post_rows: Vec<serde_json::Value> = posts
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "uuid": p.uuid,
                "title": p.title,
                "slug": p.slug,
                "markdown": p.markdown,
                "html": p.html,
                "plaintext": p.plaintext,
                "feature_image": p.feature_image,
                "featured": p.featured,
                "type": p.post_type,
                "status": p.status,
                "visibility": p.visibility,
                "custom_excerpt": p.custom_excerpt,
                "reading_time": p.reading_time,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
                "published_at": p.published_at,
            })
        })
        .collect();
    let tag_rows: Vec<serde_json::Value> = tags
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "slug": t.slug,
                "description": t.description,
                "visibility": t.visibility,
            })
        })
        .collect();
    serde_json::json!({
        "db": [{
            "meta": {
                "exported_on": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                "version": GHOST_VERSION,
            },
            "data": {
                "posts": post_rows,
                "tags": tag_rows,
                "site_url": site_url,
            }
        }]
    })
}

/// The content export as a JSON string: posts, pages and tags in Ghost's
/// export shape. Shared by the standalone download and the backup bundle.
async fn build_content_export(state: &AppState) -> String {
    let q = repos::posts::BrowsePosts {
        filter: None,
        order: None,
        limit: 100_000,
        page: 1,
        post_type: None,
        public: false,
        include_authors: false,
        include_tags: false,
    };
    let posts = repos::posts::browse(&state.db, &q)
        .await
        .map(|p| p.posts)
        .unwrap_or_default();
    let (tags, _) =
        repos::tags::browse(&state.db, None, None, 100_000, 1, false, true).await.unwrap_or_default();
    let export = build_ghost_export(&posts, &tags, &state.site_url());
    serde_json::to_string_pretty(&export).unwrap_or_default()
}

async fn migration_export_content(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"ghost-export.{}.json\"", export_date_stamp()),
            ),
        ],
        build_content_export(&state).await,
    )
        .into_response()
}

/// Ghost's post-analytics columns, less the ones this port has no source for.
///
/// Ghost drops a column rather than filling it with zeroes when the site does
/// not track that number — an empty `clicks` column reads as "nobody clicked",
/// which is a different and wrong claim. The same rule applies here: there is
/// no per-post click or signup attribution in this schema, so `clicks`,
/// `signups` and `paid_conversions` are absent rather than blank. `views` is
/// in their place because this port's own analytics does count those.
const POST_ANALYTICS_COLUMNS: &[&str] = &[
    "id", "title", "url", "author", "status", "created_at", "updated_at", "published_at",
    "featured", "tags", "post_access", "email_recipients", "newsletter_name", "sends", "opens",
    "views", "feedback_more_like_this", "feedback_less_like_this",
];

/// Per-post sends, opens, views and feedback, as a CSV.
async fn build_post_analytics_csv(state: &AppState) -> Option<Vec<u8>> {
    let q = repos::posts::BrowsePosts {
        filter: None,
        order: Some("published_at desc".into()),
        limit: 100_000,
        page: 1,
        post_type: Some("post".into()),
        public: false,
        include_authors: true,
        include_tags: true,
    };
    let posts = repos::posts::browse(&state.db, &q).await.map(|p| p.posts).unwrap_or_default();
    let site_url = state.site_url();
    let site_url = site_url.trim_end_matches('/');

    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(POST_ANALYTICS_COLUMNS).ok()?;
    for p in &posts {
        let slug = p.slug.clone().unwrap_or_default();
        // The email for a post, if one went out. A post reverted to draft
        // keeps its email row, so the status check below is what stops a
        // draft reporting last month's sends.
        let email = sqlx::query_as::<_, (i64, i64, Option<String>, Option<String>)>(
            "SELECT email_count, opened_count, recipient_filter, newsletter_id \
             FROM emails WHERE post_id = ? LIMIT 1",
        )
        .bind(&p.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .filter(|_| p.status == "published" || p.status == "sent");

        let newsletter_name = match email.as_ref().and_then(|e| e.3.clone()) {
            Some(nid) => sqlx::query_scalar::<_, String>("SELECT name FROM newsletters WHERE id = ?")
                .bind(&nid)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .unwrap_or_default(),
            None => String::new(),
        };

        // Views are counted against the rendered path, which is the slug.
        let views: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM page_views WHERE path = ? OR path = ?")
            .bind(format!("/{slug}/"))
            .bind(format!("/{slug}"))
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

        let feedback = |score: i64| async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM members_feedback WHERE post_id = ? AND score = ?",
            )
            .bind(&p.id)
            .bind(score)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0)
        };
        let (more, less) = (feedback(1).await, feedback(0).await);

        let record = [
            p.id.clone(),
            p.title.clone().unwrap_or_default(),
            format!("{site_url}/{slug}/"),
            post_author_names(state, &p.id).await,
            p.status.clone(),
            p.created_at.clone().unwrap_or_default(),
            p.updated_at.clone().unwrap_or_default(),
            p.published_at.clone().unwrap_or_default(),
            if p.featured { "true".into() } else { "false".into() },
            post_tag_names(state, &p.id).await,
            p.visibility.clone(),
            email.as_ref().and_then(|e| e.2.clone()).unwrap_or_default(),
            newsletter_name,
            email.as_ref().map(|e| e.0.to_string()).unwrap_or_default(),
            email.as_ref().map(|e| e.1.to_string()).unwrap_or_default(),
            views.to_string(),
            more.to_string(),
            less.to_string(),
        ];
        w.write_record(&record).ok()?;
    }
    w.into_inner().ok()
}

/// The names behind a post's authors and tags, comma-joined as Ghost writes
/// them. Queried per post rather than taken from the browse include, because
/// the join only populates when the caller asked for it and a missing name
/// would silently export an empty column.
async fn post_author_names(state: &AppState, post_id: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT u.name FROM users u JOIN posts_authors pa ON pa.author_id = u.id \
         WHERE pa.post_id = ? ORDER BY pa.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .join(", ")
}

async fn post_tag_names(state: &AppState, post_id: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT t.name FROM tags t JOIN posts_tags pt ON pt.tag_id = t.id \
         WHERE pt.post_id = ? ORDER BY pt.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .join(", ")
}

async fn migration_export_analytics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match build_post_analytics_csv(&state).await {
        Some(body) => csv_download(&format!("posts.analytics.{}.csv", export_date_stamp()), body),
        None => settings_flash("migration", "error", "Could not build the analytics export."),
    }
}

// --- the export bundle -------------------------------------------------------

/// The pieces an export may carry, in the order they are offered.
///
/// `(key, label, detail, on by default)`. Media is off by default for the
/// reason Ghost gives: it is usually the bulk of the archive, and someone
/// moving content between sites rarely wants to wait for it.
const EXPORT_COMPONENTS: &[(&str, &str, &str, bool)] = &[
    ("content", "Content & settings", "Posts, pages and tags, in Ghost's JSON export format", true),
    ("members", "Members", "Every member with their labels and tiers (CSV)", true),
    ("analytics", "Post analytics", "Sends, opens, views and feedback per post (CSV)", true),
    ("themes", "Themes", "Every installed theme, including anything you edited", true),
    ("routes", "Redirects & routes", "routes.yaml and redirects.yaml as they stand", true),
    (
        "media",
        "Media files",
        "Every uploaded image and file. Can make the archive very much larger",
        false,
    ),
];

/// Add a directory to the archive, recursively, under `prefix`.
///
/// Walked breadth-first with an explicit stack rather than by recursion
/// because `ZipWriter` is borrowed mutably throughout, and an async recursive
/// helper around it would need boxing for no gain.
fn zip_add_dir<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &std::path::Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> std::io::Result<()> {
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Dotfiles and dependency trees are never part of a backup.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(base) else { continue };
            let Ok(bytes) = std::fs::read(&path) else { continue };
            zip.start_file(format!("{prefix}/{}", rel.to_string_lossy().replace('\\', "/")), opts)?;
            std::io::Write::write_all(zip, &bytes)?;
        }
    }
    Ok(())
}

/// Build the export archive from the components the operator ticked.
///
/// Everything is assembled in memory: these are a publisher's own posts and
/// members, which is small next to the media they may also have asked for —
/// and media is the one component that is off by default.
async fn build_export_archive(
    state: &AppState,
    want: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        if want("content") {
            zip.start_file("ghost-export.json", opts).ok()?;
            zip.write_all(build_content_export(state).await.as_bytes()).ok()?;
        }
        if want("members") {
            if let Some(csv) = build_members_csv(state).await {
                zip.start_file("members.csv", opts).ok()?;
                zip.write_all(&csv).ok()?;
            }
        }
        if want("analytics") {
            if let Some(csv) = build_post_analytics_csv(state).await {
                zip.start_file("posts.analytics.csv", opts).ok()?;
                zip.write_all(&csv).ok()?;
            }
        }
        if want("themes") {
            zip_add_dir(&mut zip, &state.config.themes_dir(), "themes", opts).ok()?;
        }
        if want("routes") {
            for file in ["routes.yaml", "redirects.yaml"] {
                if let Ok(bytes) = std::fs::read(state.config.content_path().join(file)) {
                    zip.start_file(format!("settings/{file}"), opts).ok()?;
                    zip.write_all(&bytes).ok()?;
                }
            }
        }
        if want("media") {
            zip_add_dir(&mut zip, &state.config.images_dir(), "content/images", opts).ok()?;
            zip_add_dir(&mut zip, &state.config.files_dir(), "content/files", opts).ok()?;
        }
        zip.finish().ok()?;
    }
    Some(buf.into_inner())
}

fn zip_download(filename: &str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        body,
    )
        .into_response()
}

/// `?content=1&members=1&…` — one archive of whatever was ticked.
async fn migration_export_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let want = |key: &str| q.get(key).map(|v| v == "1").unwrap_or(false);
    if !EXPORT_COMPONENTS.iter().any(|(k, _, _, _)| want(k)) {
        return settings_flash("migration", "error", "Pick at least one thing to export.");
    }
    match build_export_archive(&state, &want).await {
        Some(body) => {
            zip_download(&format!("ghost-backup.{}.zip", export_date_stamp()), body)
        }
        None => settings_flash("migration", "error", "Could not build the archive."),
    }
}

async fn migration_export_themes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let only_themes = |key: &str| key == "themes";
    match build_export_archive(&state, &only_themes).await {
        Some(body) => zip_download(&format!("themes.{}.zip", export_date_stamp()), body),
        None => settings_flash("migration", "error", "Could not build the themes archive."),
    }
}

/// Result of walking an import file, reported back through the flash.
#[derive(Default)]
struct ImportCounts {
    posts: usize,
    pages: usize,
    tags: usize,
}

impl ImportCounts {
    fn message(&self) -> String {
        format!(
            "Imported {} {}, {} {} and {}.",
            self.posts,
            if self.posts == 1 { "post" } else { "posts" },
            self.tags,
            if self.tags == 1 { "tag" } else { "tags" },
            if self.pages == 1 { "1 page".to_string() } else { format!("{} pages", self.pages) },
        )
    }
}

/// Insert posts/pages/tags from a Ghost export file's `data` object.
async fn run_import(state: &AppState, owner_id: &str, data: &serde_json::Value) -> Result<ImportCounts, ghost_core::GhostError> {
    let mut counts = ImportCounts::default();

    // tags first (keyed by export id so posts can map them over)
    let mut tag_ids: HashMap<String, String> = HashMap::new();
    if let Some(tags) = data.get("tags").and_then(|t| t.as_array()) {
        for t in tags {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let existing: Option<String> = sqlx::query_scalar("SELECT id FROM tags WHERE name = ?")
                .bind(&name)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();
            if let Some(id) = existing {
                tag_ids.insert(t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(), id);
                continue;
            }
            if let Ok(tag) = repos::tags::insert(
                &state.db,
                &repos::tags::NewTag {
                    name: name.clone(),
                    description: t.get("description").and_then(|v| v.as_str()).map(str::to_string),
                    ..Default::default()
                },
            )
            .await
            {
                counts.tags += 1;
                tag_ids.insert(t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(), tag.id);
            }
        }
    }

    if let Some(posts) = data.get("posts").and_then(|p| p.as_array()) {
        for p in posts {
            let title = p.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").trim().to_string();
            // A Ghost export carries `lexical`; this server's own export
            // carries `markdown`. Keep whichever arrived, so the editor
            // reopens the real source rather than a rendering of it.
            let markdown = p
                .get("markdown")
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .map(str::to_string);
            let lexical = p
                .get("lexical")
                .and_then(|v| v.as_str())
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string);
            let post_type = match p.get("type").and_then(|v| v.as_str()) {
                Some("page") => "page",
                _ => "post",
            };
            let tag_ids: Vec<String> = p
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.get("id").and_then(|v| v.as_str()))
                        .filter_map(|id| tag_ids.get(id).cloned())
                        .collect()
                })
                .unwrap_or_default();
            let (html, plaintext, reading_time) = match (&markdown, &lexical) {
                (Some(md), _) => ghost_render::markdown::render_markdown(md),
                (None, source) => {
                    // The export's own `html` is Ghost's rendering of the
                    // lexical and is preferred; rendering it here is the
                    // fallback for an export that omitted it.
                    let html = p
                        .get("html")
                        .and_then(|v| v.as_str())
                        .filter(|h| !h.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| source.as_deref().map(ghost_render::lexical::to_html).unwrap_or_default());
                    let plain = ghost_render::markdown::html_to_plaintext(&html);
                    let rt = ghost_render::markdown::reading_time(&plain);
                    (html, plain, rt)
                }
            };
            let new = repos::posts::NewPost {
                tier_ids: Vec::new(),
                title,
                slug: p.get("slug").and_then(|v| v.as_str()).map(str::to_string),
                lexical,
                markdown,
                html: Some(html),
                plaintext: Some(plaintext),
                custom_excerpt: p.get("custom_excerpt").and_then(|v| v.as_str()).map(str::to_string),
                feature_image: p.get("feature_image").and_then(|v| v.as_str()).map(str::to_string),
                featured: p.get("featured").and_then(|v| v.as_bool()).unwrap_or(false),
                post_type: post_type.to_string(),
                status: if p.get("visibility").and_then(|v| v.as_str()) == Some("tiers") { "draft".to_string() } else { p.get("status").and_then(|v| v.as_str()).unwrap_or("draft").to_string() },
                visibility: p.get("visibility").and_then(|v| v.as_str()).unwrap_or("public").to_string(),
                published_at: None,
                codeinjection_head: p.get("codeinjection_head").and_then(|v| v.as_str()).map(str::to_string),
                codeinjection_foot: p.get("codeinjection_foot").and_then(|v| v.as_str()).map(str::to_string),
                custom_template: p.get("custom_template").and_then(|v| v.as_str()).map(str::to_string),
                canonical_url: p.get("canonical_url").and_then(|v| v.as_str()).map(str::to_string),
                show_title_and_feature_image: p.get("show_title_and_feature_image").and_then(|v| v.as_bool()),
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                twitter_title: None,
                twitter_description: None,
                twitter_image: None,
                email_subject: None,
                feature_image_alt: None,
                feature_image_caption: None,
                author_ids: vec![],
                tag_ids,
                reading_time,
            };
            if repos::posts::insert(&state.db, &new, owner_id).await.is_ok() {
                if post_type == "page" {
                    counts.pages += 1;
                } else {
                    counts.posts += 1;
                }
            }
        }
    }
    Ok(counts)
}

/// What came out of an uploaded archive.
struct ArchiveContents {
    /// The Ghost export JSON found inside, if there was one.
    export: Option<Vec<u8>>,
    /// Media restored to `content/`, counted for the report.
    media_restored: usize,
}

/// Unpack an uploaded zip: find the export JSON, restore any media beside it.
///
/// Both shapes are accepted — the archive this screen produces
/// (`ghost-export.json` + `content/images/…`) and Ghost's own, which nests
/// everything one level down inside a named folder. Rather than guess at the
/// layout, the first `.json` that parses as an export wins and any path under
/// a `content/images` or `content/files` segment is treated as media.
///
/// Entry paths are the untrusted part: a zip may name `../../etc/passwd`, so
/// every restored path is rebuilt from its own segments and anything with a
/// traversal or an absolute root is skipped rather than sanitised.
fn unpack_import_archive(
    state: &AppState,
    data: &[u8],
) -> Result<ArchiveContents, ghost_core::GhostError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| ghost_core::GhostError::BadRequest(format!("Not a readable zip: {e}")))?;
    let mut export = None;
    let mut media_restored = 0;

    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else { continue };
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` is the zip crate's own traversal check: it returns
        // None for an absolute path or one that climbs out of the archive.
        let Some(path) = entry.enclosed_name() else { continue };
        let rel = path.to_string_lossy().replace('\\', "/");
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut bytes).is_err() {
            continue;
        }

        if export.is_none() && rel.to_lowercase().ends_with(".json") {
            // A theme's package.json is also a .json; only something shaped
            // like an export counts.
            if serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("db").cloned())
                .is_some()
            {
                export = Some(bytes);
                continue;
            }
        }

        let target = if let Some(rest) = after_segment(&rel, "content/images/") {
            Some(state.config.images_dir().join(rest))
        } else {
            after_segment(&rel, "content/files/").map(|rest| state.config.files_dir().join(rest))
        };
        if let Some(target) = target {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&target, &bytes).is_ok() {
                media_restored += 1;
            }
        }
    }
    Ok(ArchiveContents { export, media_restored })
}

/// The part of `path` after `marker`, if the marker is a whole-segment match.
///
/// Used to find media inside an archive whatever it is nested under, without
/// matching a directory that merely ends in the same letters.
fn after_segment(path: &str, marker: &str) -> Option<String> {
    let at = path.find(marker)?;
    if at != 0 && !path[..at].ends_with('/') {
        return None;
    }
    let rest = &path[at + marker.len()..];
    // Rebuild from segments so nothing odd survives into a filesystem path.
    if rest.is_empty() || rest.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return None;
    }
    Some(rest.to_string())
}

/// The universal importer: a Ghost export as JSON, or a zip holding one.
async fn migration_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or_default().to_ascii_lowercase();
        let data = match field.bytes().await {
            Ok(d) => d.to_vec(),
            Err(e) => return settings_flash("migration", "error", &format!("Upload failed: {e}")),
        };
        if data.is_empty() {
            return settings_flash("migration", "error", "That file is empty.");
        }

        // A zip starts "PK\x03\x04". Sniffing the bytes rather than trusting
        // the extension means a correctly-named file that is not a zip fails
        // with "not valid JSON" instead of a confusing zip error.
        let is_zip = data.starts_with(b"PK\x03\x04") || filename.ends_with(".zip");
        let (json, media_note) = if is_zip {
            match unpack_import_archive(&state, &data) {
                Ok(contents) => {
                    let note = match contents.media_restored {
                        0 => String::new(),
                        1 => " 1 media file was restored.".to_string(),
                        n => format!(" {n} media files were restored."),
                    };
                    match contents.export {
                        Some(bytes) => (bytes, note),
                        None if contents.media_restored > 0 => {
                            return settings_flash(
                                "migration",
                                "ok",
                                &format!("No export file in that archive, but{note}"),
                            )
                        }
                        None => {
                            return settings_flash(
                                "migration",
                                "error",
                                "That archive holds no Ghost export file and no media.",
                            )
                        }
                    }
                }
                Err(e) => return settings_flash("migration", "error", &e.message()),
            }
        } else {
            (data, String::new())
        };

        let parsed: serde_json::Value = match serde_json::from_slice(&json) {
            Ok(v) => v,
            Err(e) => {
                return settings_flash("migration", "error", &format!("Not a valid JSON file: {e}"))
            }
        };
        let data = parsed
            .get("db")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|entry| entry.get("data"));
        return match data {
            Some(data) => match run_import(&state, &user.id, data).await {
                Ok(counts) => {
                    settings_flash("migration", "ok", &format!("{}{media_note}", counts.message()))
                }
                Err(e) => settings_flash("migration", "error", &e.message()),
            },
            None => settings_flash(
                "migration",
                "error",
                "That file doesn't look like a Ghost export (no db[0].data).",
            ),
        };
    }
    settings_flash("migration", "error", "No file was uploaded.")
}

// --- labs: the redirects and routes files, uploadable and downloadable ---

/// The current redirects.yaml, as Ghost's Labs screen offers it. A site that
/// has never uploaded any gets the neutral document rather than an error.
async fn redirects_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let body = tokio::fs::read_to_string(state.config.content_path().join("redirects.yaml"))
        .await
        .unwrap_or_else(|_| "# No redirects configured yet.\n".to_string());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"redirects.yaml\""),
        ],
        body,
    )
        .into_response()
}

async fn redirects_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return settings_flash("advanced", "error", &format!("Upload failed: {e}")),
        };
        // Parse before writing anything: a malformed file never reaches the
        // disk, let alone the live redirect table.
        let parsed = match ghost_routing::redirects::parse_redirects_yaml(&text) {
            Ok(r) => r,
            Err(e) => return settings_flash("advanced", "error", &format!("Not a valid redirects file: {e}")),
        };
        let path = state.config.content_path().join("redirects.yaml");
        if let Err(e) = tokio::fs::write(&path, &text).await {
            return settings_flash("advanced", "error", &format!("Could not write redirects.yaml: {e}"));
        }
        let count = ghost_routing::redirects::rules_map(&parsed).len();
        *state.redirects.write().await = Arc::new(parsed);
        fire_site_changed(&state);
        return settings_flash("advanced", "ok", &format!("Redirects uploaded — {count} rule{s} in force.", s = if count == 1 { "" } else { "s" }));
    }
    settings_flash("advanced", "error", "No file was uploaded.")
}

async fn routes_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let body = tokio::fs::read_to_string(state.config.content_path().join("routes.yaml"))
        .await
        .unwrap_or_else(|_| ghost_routing::DEFAULT_ROUTES_YAML.to_string());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"routes.yaml\""),
        ],
        body,
    )
        .into_response()
}

/// Upload a routes.yaml. Routing is live configuration, so the file is
/// parsed *and* built into a router tree before anything is written — then
/// the tree is swapped in and the active theme is recompiled against it,
/// because a theme's resolvers and template choices are derived from routes.
async fn routes_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return settings_flash("advanced", "error", &format!("Upload failed: {e}")),
        };
        let settings = match ghost_routing::routes_yaml::parse_routes_yaml(&text) {
            Ok(s) => s,
            Err(e) => return settings_flash("advanced", "error", &format!("Not a valid routes file: {e}")),
        };
        let tree = match ghost_routing::RouterTree::build(&settings) {
            Ok(t) => t,
            Err(e) => return settings_flash("advanced", "error", &format!("These routes cannot be routed: {e}")),
        };
        let path = state.config.content_path().join("routes.yaml");
        if let Err(e) = tokio::fs::write(&path, &text).await {
            return settings_flash("advanced", "error", &format!("Could not write routes.yaml: {e}"));
        }
        *state.router.write().await = Arc::new(tree);
        // The theme was compiled against the old routes; recompile it against
        // the new ones. A failure here leaves routing updated but the old
        // theme in place — said out loud rather than half-reported.
        let theme = ghost_api::themes::active_theme_name(&state);
        if let Err(e) = ghost_api::themes::activate(&state, &theme).await {
            return settings_flash(
                "advanced",
                "error",
                &format!("Routes are in force, but the theme could not be rebuilt against them: {}", e.message()),
            );
        }
        fire_site_changed(&state);
        return settings_flash("advanced", "ok", "Routes uploaded — routing and the theme have been rebuilt.");
    }
    settings_flash("advanced", "error", "No file was uploaded.")
}

// --- history: the actions audit log ---

/// Which resource types each of Ghost's history filter groups covers. The
/// port records a few names Ghost does not, so each group names its own.
const HISTORY_RESOURCES: &[(&str, &str, &[&str])] = &[
    ("post", "Posts", &["post"]),
    ("page", "Pages", &["page"]),
    ("tag", "Tags", &["tag"]),
    ("tier", "Tiers & offers", &["tier", "product", "offer"]),
    ("setting", "Settings & staff", &["user", "users", "setting", "settings", "integrations", "members", "db"]),
];

const HISTORY_EVENTS: &[(&str, &str)] =
    &[("added", "Added"), ("edited", "Edited"), ("deleted", "Deleted"), ("published", "Published")];

async fn history_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Ghost's history filters are exclusions: everything shows until you
    // untick something. A submitted form is marked, so a first load — where
    // no box has been touched — is not read as "everything excluded".
    let filtered = q.contains_key("f");
    let shown_event = |v: &str| !filtered || q.contains_key(&format!("e_{v}"));
    let shown_resource = |k: &str| !filtered || q.contains_key(&format!("r_{k}"));

    let excluded_events: Vec<&str> =
        HISTORY_EVENTS.iter().map(|(v, _)| *v).filter(|v| !shown_event(v)).collect();
    // Expand the excluded groups to the resource names they actually cover.
    let excluded_names: Vec<&str> = HISTORY_RESOURCES
        .iter()
        .filter(|(k, _, _)| !shown_resource(k))
        .flat_map(|(_, _, names)| names.iter().copied())
        .collect();

    let mut sql = String::from(
        "SELECT a.event, a.resource_type, a.resource_id, a.context, a.created_at, COALESCE(u.name, 'System') \
         FROM actions a LEFT JOIN users u ON u.id = a.actor_id WHERE 1=1",
    );
    for _ in &excluded_events {
        sql.push_str(" AND a.event != ?");
    }
    for _ in &excluded_names {
        sql.push_str(" AND a.resource_type != ?");
    }
    sql.push_str(" ORDER BY a.created_at DESC LIMIT 100");

    let mut query =
        sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String)>(&sql);
    for e in &excluded_events {
        query = query.bind(*e);
    }
    for r in &excluded_names {
        query = query.bind(*r);
    }

    let chip = |name: String, label: &str, on: bool| {
        format!(
            r#"<label class="fchip"><input type="checkbox" name="{name}" value="1"{ck} onchange="this.form.submit()"><span class="ck">{check}</span>{label}</label>"#,
            name = esc(&name),
            label = esc(label),
            ck = if on { " checked" } else { "" },
            check = svg("check"),
        )
    };
    let filter_bar = format!(
        r#"<form method="get" action="/ghost/settings/history/" class="card padded rise" style="--i:1">
<input type="hidden" name="f" value="1">
<span class="detail" style="display:block;margin-bottom:8px">Event</span>
<div class="chips">{events}</div>
<span class="detail" style="display:block;margin:14px 0 8px">Resource</span>
<div class="chips">{resources}</div>
<noscript><div style="margin-top:12px"><button class="btn tonal small" type="submit">Apply</button></div></noscript>
</form>"#,
        events = HISTORY_EVENTS
            .iter()
            .map(|(v, l)| chip(format!("e_{v}"), l, shown_event(v)))
            .collect::<String>(),
        resources = HISTORY_RESOURCES
            .iter()
            .map(|(k, l, _)| chip(format!("r_{k}"), l, shown_resource(k)))
            .collect::<String>(),
    );

    let mut rows = String::new();
    if let Ok(events) = query.fetch_all(&state.db).await {
        if events.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">Nothing has happened yet — edits and publishes show up here.</span></div></div>"#.into();
        }
        for (event, resource_type, resource_id, _context, created_at, actor) in events {
            rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-slate">{ic}</span>
<div class="body"><span class="title">{actor} {event} {resource_type}</span><span class="detail">{when}</span></div>
<div class="trailing"><span class="chip tonal">{rid}</span></div></div>"#,
                ic = svg("history"),
                actor = esc(&actor),
                event = esc(&event),
                resource_type = esc(&resource_type),
                when = esc(&created_at),
                rid = esc(resource_id.as_deref().map(|r| &r[..8.min(r.len())]).unwrap_or("—")),
            ));
        }
    }
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "history").unwrap();
    page_for_role(actor_role, 
        "History",
        "settings",
        Some(&user),
        &format!(
            r#"{head}
<div class="section">Filter</div>
{filter_bar}
<div class="section">Site activity</div>
<div class="card rows rise" style="--i:2">{rows}</div>
<p class="muted" style="margin:4px 0 0 16px">The last 100 matching events from the actions audit log.</p>"#,
            head = group_head(group, None),
            filter_bar = filter_bar,
            rows = rows,
        ),
        true,
    )
    .into_response()
}

// --- danger zone: delete all content / delete the whole site ---

/// Tables wiped by "delete all content" (Ghost's danger zone keeps users and
/// settings) — everything post/tag-shaped plus their join and revision tables.
const CONTENT_TABLES: &[&str] = &[
    "posts", "posts_meta", "posts_tags", "posts_authors", "posts_products",
    "post_revisions", "mobiledoc_revisions", "tags", "snippets", "emails",
    "email_batches", "email_recipients", "email_recipient_failures",
];

/// Tables wiped by "delete site" on top of the content set — identity, members
/// and commerce data. Settings, roles and permissions survive so first-boot
/// seeding can run again.
const SITE_TABLES: &[&str] = &[
    "users", "sessions", "actions", "invites", "roles_users",
    "members", "members_products", "members_newsletters", "members_labels",
    "members_created_events", "members_stripe_customers", "subscriptions",
    "products", "products_benefits", "benefits", "offers",
    "stripe_products", "stripe_prices",
    "newsletters", "recommendations", "recommendation_click_events",
    "recommendation_subscribe_events", "labels",
    "comments", "comment_likes", "comment_reports",
    "integrations", "api_keys", "webhooks", "custom_theme_settings",
    "brute", "tokens", "milestones",
];

async fn wipe_tables(db: &sqlx::SqlitePool, tables: &[&str]) -> usize {
    let mut wiped = 0;
    for table in tables {
        if sqlx::query(&format!("DELETE FROM {table}")).execute(db).await.is_ok() {
            wiped += 1;
        }
    }
    wiped
}

async fn danger_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "danger").unwrap();
    page_for_role(actor_role, 
        "Danger zone",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-red">{posts}</span>
<div class="body"><span class="title">Delete all content</span><span class="detail">Permanently remove every post, page, tag and email draft. Users and settings are kept.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/content/" data-confirm="Delete all content|Every post, page and tag will be permanently deleted. This cannot be undone.">
<button class="btn text danger" type="submit">Delete…</button></form></div></div>
<div class="row"><span class="ic tint-red">{lock}</span>
<div class="body"><span class="title">Reset all authentication</span><span class="detail">Sign every staff member and member out everywhere, and invalidate every outstanding magic link. Nobody loses their account.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/auth/" data-confirm="Reset all authentication|Everyone — staff and members — will be signed out on every device, and any magic links already emailed will stop working. You will be signed out too.">
<button class="btn text danger" type="submit">Reset…</button></form></div></div>
<div class="row"><span class="ic tint-red">{gift}</span>
<div class="body"><span class="title">Reset all gift links</span><span class="detail">Invalidate every gift subscription link that has been shared but not yet redeemed.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/gifts/" data-confirm="Reset all gift links|Every unredeemed gift link stops working. Gifts already redeemed are unaffected.">
<button class="btn text danger" type="submit">Reset…</button></form></div></div>
<div class="row"><span class="ic tint-red">{ghost}</span>
<div class="body"><span class="title">Delete site</span><span class="detail">Wipe everything — content, users, members and integrations — and restart at the setup screen. Settings return to their defaults.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/site/" data-confirm="Delete this site|ALL content, users and integrations will be permanently deleted and you will be signed out. This cannot be undone.">
<button class="btn text danger" type="submit">Delete…</button></form></div></div>
</div>
<p class="muted" style="margin:14px 0 0 16px">Ghost version {version} · {env} · SQLite · <a href="/ghost/settings/about/">About this site</a></p>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            posts = svg("delete"),
            lock = svg("lock"),
            gift = svg("mail"),
            ghost = svg("ghost"),
            version = GHOST_VERSION,
            env = esc(&state.config.env),
        ),
        true,
    )
    .into_response()
}

async fn danger_delete_content(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let _ = wipe_tables(&state.db, CONTENT_TABLES).await;
    let _ = repos::actions::record(&state.db, "db", "content", &user.id, "deleted").await;
    settings_flash("danger", "ok", "All content deleted.")
}

async fn danger_delete_site(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let _ = wipe_tables(&state.db, CONTENT_TABLES).await;
    let _ = wipe_tables(&state.db, SITE_TABLES).await;
    state.refresh_setup_flag().await;
    // the session row is gone too — straight back to setup
    Redirect::to("/ghost/setup/").into_response()
}

/// Sign everyone out, everywhere.
///
/// Ghost's "Reset all authentication": staff sessions, member sessions and
/// every unredeemed magic-link token go at once. Nobody loses their account —
/// they sign in again. The caller is signed out too, which is the point: an
/// operator reaching for this has lost track of who holds a session, and an
/// exception for themselves would leave the one session they cannot vouch for.
async fn danger_reset_auth(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let _ = sqlx::query("DELETE FROM sessions").execute(&state.db).await;
    // Members authenticate by a signed `transient_id`, so rolling it is what
    // actually ends their sessions — deleting rows would not.
    let members: Vec<String> = sqlx::query_scalar("SELECT id FROM members")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for id in members {
        let _ = sqlx::query("UPDATE members SET transient_id = ? WHERE id = ?")
            .bind(ghost_core::ids::uuid_v4())
            .bind(&id)
            .execute(&state.db)
            .await;
    }
    // Outstanding magic links, including any already in someone's inbox.
    let _ = sqlx::query("DELETE FROM tokens").execute(&state.db).await;

    // No session of our own survives, so there is nowhere to flash to.
    Redirect::to("/ghost/signin/?reset=1").into_response()
}

/// Invalidate every unredeemed gift link.
///
/// A gift link is a token like a magic link; redeemed gifts are subscriptions
/// and are untouched.
async fn danger_reset_gifts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let removed = sqlx::query("DELETE FROM tokens WHERE data LIKE '%\"gift\"%'")
        .execute(&state.db)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
    settings_flash(
        "danger",
        "ok",
        &match removed {
            0 => "No unredeemed gift links to reset.".to_string(),
            1 => "1 gift link reset.".to_string(),
            n => format!("{n} gift links reset."),
        },
    )
}

/// Ghost's About dialog: what is running, and where.
async fn about_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let db_path = state.config.database.connection.filename.clone();
    let mail = match state.settings.get_str("mail_transport").as_deref() {
        Some("smtp") => format!(
            "SMTP · {}",
            state.settings.get_str("mail_smtp_host").unwrap_or_else(|| "not configured".into())
        ),
        _ => "Direct".to_string(),
    };
    let row = |label: &str, value: &str| {
        format!(
            r#"<div class="row"><div class="body"><span class="title">{label}</span><span class="detail">{value}</span></div></div>"#,
            label = esc(label),
            value = esc(value),
        )
    };

    page_for_role(actor_role,
        "About",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>About</h1><p class="sub">What is running on this site</p></div></header>
<div class="card rows rise" style="--i:1">
{version_row}{port_row}{env_row}{db_row}{mail_row}{theme_row}
</div>
<p class="muted" style="margin:14px 0 0 16px">Ghost-rust is an independent reimplementation of Ghost in Rust. It tracks the Ghost
release above for its admin, Content API and theme behaviour; it is not affiliated with the Ghost Foundation.</p>"#,
            back = svg("chevron"),
            version_row = row("Ghost version", GHOST_VERSION),
            port_row = row("Ghost-rust", env!("CARGO_PKG_VERSION")),
            env_row = row("Environment", &state.config.env),
            db_row = row("Database", &format!("{} · {}", state.config.database.client, db_path)),
            mail_row = row("Mail", &mail),
            theme_row = row(
                "Active theme",
                &state.settings.get_str("active_theme").unwrap_or_else(|| "casper".into()),
            ),
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// navigation (Ghost-style: `navigation` + `secondary_navigation` settings,
// edited as ordered label/URL rows and rendered by themes via {{navigation}})
// ---------------------------------------------------------------------------

/// One menu entry (`{"label": …, "url": …}` in the stored JSON array).
#[derive(Default, Debug, PartialEq)]
struct NavItem {
    label: String,
    url: String,
}

/// Cap on menu length so `add` can't run away; Ghost has no fixed limit but
/// menus are single-digit in practice.
const NAV_MAX_ITEMS: usize = 50;

/// The posted navigation form. Every row posts a `…_label` and a `…_url`
/// input under the same names, so this needs the same hand-rolled parsing as
/// the editor's tag checkboxes (`axum::Form` would collapse the repeats).
#[derive(Default, Debug)]
struct NavForm {
    primary: Vec<NavItem>,
    secondary: Vec<NavItem>,
    /// Which submit button was pressed: `save`, `add-{kind}` or
    /// `{up|down|remove}-{kind}-{index}`.
    action: String,
}

impl NavForm {
    /// Labels and URLs arrive in row order, so the i-th of each belongs to
    /// the same row — zip them (padding with blanks if counts disagree).
    fn parse(body: &str) -> Self {
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
fn nav_apply_action(form: &mut NavForm, action: &str) -> bool {
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
fn nav_clean(items: &[NavItem]) -> Vec<serde_json::Value> {
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
fn nav_items_from_setting(v: Option<serde_json::Value>) -> Vec<NavItem> {
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
fn navigation_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/navigation/?{encoded}")).into_response()
}

/// One editable menu: a card of label/URL rows plus an add button. The
/// up/down/remove buttons post back with `action=…` (and move instantly via
/// JS when available); rows post in DOM order, which is the saved order.
fn nav_list_html(items: &[NavItem], kind: &str, tint: &str) -> String {
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

fn navigation_html(primary: &[NavItem], secondary: &[NavItem], banner: &str) -> String {
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

async fn navigation_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
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

async fn navigation_save(State(state): State<AppState>, headers: HeaderMap, form: NavForm) -> Response {
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

// ---------------------------------------------------------------------------
// staff profile (Ghost's Settings → Staff → <user>, which the account block
// at the foot of the rail opens for the signed-in user)
// ---------------------------------------------------------------------------

// --- Settings → Staff: the team list, role changes and invitations ---

fn staff_flash(kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/staff/?{encoded}")).into_response()
}

/// Whether `user` may administer other staff. Ghost allows Owner and
/// Administrator; everyone else can edit only their own profile.
async fn can_manage_staff(state: &AppState, user: &ghost_core::models::User) -> bool {
    role_of(state, user).await.can_manage_staff()
}

async fn role_of(state: &AppState, user: &ghost_core::models::User) -> Role {
    let name = repos::users::role_name(&state.db, &user.id).await.ok().flatten().unwrap_or_default();
    Role::from_name(&name)
}

/// Which roles `actor` may hand out.
///
/// Ghost lets Editors invite and manage Authors and Contributors, but not
/// other Editors or Administrators — an Editor must not be able to promote
/// themselves a peer with settings access, or grant a role above their own.
fn assignable_by(actor: Role) -> Vec<&'static (&'static str, &'static str)> {
    repos::staff::ASSIGNABLE_ROLES
        .iter()
        .filter(|(name, _)| {
            let r = Role::from_name(name);
            if actor.is_admin() { r <= Role::Administrator } else { r < Role::Editor }
        })
        .collect()
}

/// Whether `actor` may act on a staff member currently holding `target`.
fn may_act_on(actor: Role, target: Role) -> bool {
    if target == Role::Owner {
        return false;
    }
    if actor.is_admin() {
        return true;
    }
    // Editors reach only below themselves.
    actor.can_manage_staff() && target < Role::Editor
}

/// `<option>`s for the role picker, with each role's description as a
/// tooltip so the difference between them is discoverable.
fn role_options_for(actor: Role, selected: &str) -> String {
    assignable_by(actor)
        .into_iter()
        .map(|(name, desc)| {
            format!(
                r#"<option value="{n}"{sel} title="{d}">{n}</option>"#,
                n = esc(name),
                d = esc(desc),
                sel = if *name == selected { " selected" } else { "" },
            )
        })
        .collect()
}

async fn staff_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    let manage = actor_role.can_manage_staff();
    let staff = repos::staff::list_staff(&state.db).await.unwrap_or_default();
    let invites = if manage { repos::staff::list_invites(&state.db).await.unwrap_or_default() } else { vec![] };

    let rows: String = staff
        .iter()
        .map(|m| {
            let initial = m.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string();
            let suspended = m.status.as_deref() == Some("inactive");
            let is_owner = m.role == "Owner";
            let is_me = m.id == user.id;
            // You can open your own profile, or anyone's if you manage staff.
            let openable = manage || is_me;
            let body = format!(
                r#"<span class="title">{name}{you}</span><span class="detail">{email}</span>"#,
                name = esc(m.name.as_deref().unwrap_or("")),
                you = if is_me { r#" <span class="muted">· you</span>"# } else { "" },
                email = esc(m.email.as_deref().unwrap_or("")),
            );
            let body = if openable {
                format!(
                    r#"<a class="body" href="/ghost/settings/staff/{slug}/">{body}</a>"#,
                    slug = esc(m.slug.as_deref().unwrap_or("")),
                    body = body,
                )
            } else {
                format!(r#"<div class="body">{body}</div>"#)
            };
            format!(
                r#"<div class="row" data-filterable><span class="avatar">{initial}</span>{body}
<div class="trailing">{suspended_chip}<span class="chip {chip}">{role}</span>{chev}</div></div>"#,
                initial = esc(&initial),
                body = body,
                suspended_chip = if suspended { r#"<span class="chip draft">suspended</span>"# } else { "" },
                chip = if is_owner { "published" } else { "tonal" },
                role = esc(&m.role),
                chev = if openable { svg("chevron") } else { String::new() },
            )
        })
        .collect();

    let invite_rows: String = invites
        .iter()
        .map(|i| {
            let expired = i.is_expired();
            format!(
                r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{email}</span><span class="detail">Invited as {role} · {state_label}</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/staff/invite/{id}/resend/"><button class="btn tonal small" type="submit">Resend</button></form>
<form method="post" action="/ghost/settings/staff/invite/{id}/revoke/" data-confirm="Revoke invitation|The link emailed to {email} will stop working.">
<button class="icon-btn danger" title="Revoke" aria-label="Revoke">{del}</button></form></div></div>"#,
                ic = svg("mail"),
                email = esc(&i.email),
                role = esc(&i.role),
                state_label = if expired { "expired".to_string() } else { format!("{} · expires in a week", i.status) },
                id = esc(&i.id),
                del = svg("delete"),
            )
        })
        .collect();

    let invite_card = if manage {
        format!(
            r#"<div class="section">Invite people</div>
<div class="card padded rise" style="--i:2">
<form method="post" action="/ghost/settings/staff/invite/" class="inline-form">
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="email" type="email" placeholder=" " required><span>Email address</span></label>
<label class="field fixed" style="min-width:190px;margin:0"><select name="role">{roles}</select><span>Role</span>{car}</label>
<button class="btn tonal" type="submit">{add} Send invitation</button>
</form>
<p class="muted" style="margin:12px 0 0">{roles_help}</p>
</div>
{invite_list}"#,
            roles = role_options_for(actor_role, "Contributor"),
            car = SELECT_CARET,
            add = svg("mail"),
            roles_help = repos::staff::ASSIGNABLE_ROLES
                .iter()
                .map(|(n, d)| format!("<strong>{}</strong> — {}", esc(n), esc(d)))
                .collect::<Vec<_>>()
                .join("<br>"),
            invite_list = if invites.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<div class="section">Pending invitations</div><div class="card rows rise" style="--i:3">{invite_rows}</div>"#
                )
            },
        )
    } else {
        format!(
            r#"<div class="banner info rise" style="--i:2">{ic}<span class="msg">Only the owner and administrators can invite people or change roles.</span></div>"#,
            ic = svg("ghost"),
        )
    };

    // Only the owner and administrators can change how everyone else signs in.
    let security_card = if manage {
        format!(
            r#"<div class="section">Security settings</div>
<div class="card rows rise" style="--i:4">
<form method="post" action="/ghost/settings/staff/security/" id="staff-security">
<div class="row"><span class="ic tint-amber">{lock}</span>
<div class="body"><span class="title">Require email 2FA codes on staff logins</span>
<span class="detail">Require email 2FA codes to be used on all staff logins</span></div>
<div class="trailing"><label class="switch"><input type="checkbox" name="require_email_mfa" value="1" {checked} onchange="this.form.requestSubmit()"><span class="track"><span class="thumb"></span></span></label></div></div>
</form></div>
<p class="muted" style="margin:10px 2px 0">With this on, a correct password is not enough: everyone gets a six-digit
code by email and has five minutes to enter it. Sessions already signed in stay signed in.</p>"#,
            lock = svg("lock"),
            checked = if state.settings.get_bool("require_email_mfa") { "checked" } else { "" },
        )
    } else {
        String::new()
    };

    page_for(
        "Staff",
        "settings",
        Some(&user),
        actor_role,
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Staff</h1><p class="sub">{count} {word}</p></div></header>
{banner}
<div class="section">Active people</div>
<div class="card rows rise" style="--i:1">{rows}</div>
{invite_card}
{security_card}"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            count = staff.len(),
            word = if staff.len() == 1 { "person" } else { "people" },
            rows = rows,
            invite_card = invite_card,
            security_card = security_card,
        ),
        true,
    )
    .into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct InviteForm {
    email: String,
    role: String,
}

/// Build the invitation link and email it. Returns the link so the caller can
/// surface it when there is no mail transport to carry it.
async fn send_invite_email(
    state: &AppState,
    invite: &repos::staff::Invite,
    inviter: &ghost_core::models::User,
) -> (String, Option<String>) {
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let link = format!("{site_url}/ghost/invite/{}/", invite.token);
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    let (subject, html, text) = ghost_mail::templates::staff_invite(
        &title,
        &site_url,
        inviter.name.as_deref().unwrap_or("A colleague"),
        &invite.role,
        &link,
    );
    let msg = ghost_mail::Message { to: invite.email.clone(), subject, html, text };
    match ghost_mail::send(&cfg, &msg).await {
        Ok(()) => {
            let _ = repos::staff::mark_invite_sent(&state.db, &invite.id).await;
            (link, None)
        }
        Err(e) => (link, Some(e.message())),
    }
}

/// Turn the email-2FA requirement on or off.
///
/// Administrators only, like the rest of the staff page's controls — this
/// decides how every other person on the site signs in.
async fn staff_security_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.get("require_email_mfa").map(|v| v == "1").unwrap_or(false);
    let saved = state
        .settings
        .set(&state.db, "require_email_mfa", &serde_json::Value::Bool(on))
        .await
        .is_ok();
    let message = match (saved, on) {
        (false, _) => "Saving failed — check the logs.",
        (true, true) => "Staff logins now require an emailed 2FA code.",
        (true, false) => "Staff logins no longer require a 2FA code.",
    };
    let kind = if saved { "ok" } else { "error" };
    let encoded: String = form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/staff/?{encoded}")).into_response()
}

async fn staff_invite(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<InviteForm>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return staff_flash("error", "Only editors and administrators can invite people.");
    }
    // An Editor must not be able to invite a peer or an administrator —
    // that would be a route to the settings they cannot reach themselves.
    if !assignable_by(actor_role).iter().any(|(n, _)| *n == form.role) {
        return staff_flash("error", &format!("As {}, you cannot invite someone as {}.", actor_role.label(), form.role));
    }
    let email = form.email.trim().to_lowercase();
    if !email.contains('@') {
        return staff_flash("error", "Enter a valid email address.");
    }
    let invite = match repos::staff::create_invite(&state.db, &email, &form.role).await {
        Ok(i) => i,
        Err(e) => return staff_flash("error", &e.message()),
    };
    match send_invite_email(&state, &invite, &user).await {
        (_, None) => staff_flash("ok", &format!("Invitation sent to {email}.")),
        // The invite is valid either way, so say so and hand over the link
        // rather than leaving the inviter thinking nothing happened.
        (link, Some(err)) => {
            tracing::warn!("staff: invite email to {email} failed: {err}");
            staff_flash(
                "error",
                &format!("Invitation created, but the email could not be sent ({err}). Share this link instead: {link}"),
            )
        }
    }
}

async fn staff_invite_resend(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if !can_manage_staff(&state, &user).await {
        return staff_flash("error", "Only the owner and administrators can manage invitations.");
    }
    let Some(old) = repos::staff::list_invites(&state.db).await.unwrap_or_default().into_iter().find(|i| i.id == id) else {
        return staff_flash("error", "That invitation no longer exists.");
    };
    // Reissue rather than resend: a fresh token means an old forwarded email
    // cannot still be redeemed.
    let invite = match repos::staff::create_invite(&state.db, &old.email, &old.role).await {
        Ok(i) => i,
        Err(e) => return staff_flash("error", &e.message()),
    };
    match send_invite_email(&state, &invite, &user).await {
        (_, None) => staff_flash("ok", &format!("Invitation resent to {}.", invite.email)),
        (link, Some(err)) => staff_flash(
            "error",
            &format!("Could not send the email ({err}). Share this link instead: {link}"),
        ),
    }
}

async fn staff_invite_revoke(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if !can_manage_staff(&state, &user).await {
        return staff_flash("error", "Only the owner and administrators can manage invitations.");
    }
    match repos::staff::revoke_invite(&state.db, &id).await {
        Ok(()) => staff_flash("ok", "Invitation revoked."),
        Err(e) => staff_flash("error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RoleForm {
    role: String,
}

async fn staff_set_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<RoleForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can change roles.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let current = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if current == "Owner" {
        return profile_flash(&slug, "error", "The owner's role cannot be changed.");
    }
    if !may_act_on(actor_role, Role::from_name(&current))
        || !assignable_by(actor_role).iter().any(|(n, _)| *n == form.role)
    {
        return profile_flash(&slug, "error", &format!("As {}, you cannot set that role.", actor_role.label()));
    }
    // Demoting the last administrator would leave nobody able to administer
    // the site, including reversing the change.
    if current == "Administrator"
        && form.role != "Administrator"
        && repos::staff::admin_count(&state.db).await.unwrap_or(0) <= 1
    {
        return profile_flash(&slug, "error", "This is the last administrator — promote someone else first.");
    }
    match repos::staff::set_role(&state.db, &target.id, &form.role).await {
        Ok(()) => profile_flash(&slug, "ok", &format!("Role changed to {}.", form.role)),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct StatusForm {
    status: String,
}

async fn staff_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<StatusForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can suspend people.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    if target.id == user.id {
        return profile_flash(&slug, "error", "You cannot suspend your own account.");
    }
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if !may_act_on(actor_role, Role::from_name(&role)) {
        return profile_flash(&slug, "error", "You cannot suspend that person.");
    }
    match repos::staff::set_user_status(&state.db, &target.id, &form.status).await {
        Ok(()) => profile_flash(
            &slug,
            "ok",
            if form.status == "inactive" { "Account suspended." } else { "Account reactivated." },
        ),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

async fn staff_delete(State(state): State<AppState>, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let actor_role = role_of(&state, &user).await;
    if !actor_role.can_manage_staff() {
        return profile_flash(&slug, "error", "Only editors and administrators can remove people.");
    }
    let Ok(Some(target)) = repos::users::find_by_slug(&state.db, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    if target.id == user.id {
        return profile_flash(&slug, "error", "You cannot delete your own account.");
    }
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    if !may_act_on(actor_role, Role::from_name(&role)) {
        return profile_flash(&slug, "error", "You cannot remove that person.");
    }
    match repos::staff::delete_staff(&state.db, &target.id).await {
        Ok(()) => staff_flash("ok", "Staff member removed."),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

// --- redeeming an invitation (signed out) ---

async fn invite_accept_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let invite = repos::staff::find_invite_by_token(&state.db, &token).await.ok().flatten();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let body = match &invite {
        Some(i) if !i.is_expired() => format!(
            r#"<div class="auth"><div class="blob b1"></div><div class="blob b2"></div><div class="blob b3"></div>
<div class="auth-box">
<div class="auth-logo">{logo}</div>
<h1>Join {title}</h1>
<p class="sub">You were invited as {role}. Choose a name and password to finish.</p>
<div class="card"><form method="post" action="/ghost/invite/{token}/">
<label class="field"><input value="{email}" disabled><span>Email</span></label>
<label class="field"><input name="name" placeholder=" " required autocomplete="name" autofocus><span>Your name</span></label>
<label class="field"><input name="password" type="password" minlength="10" required autocomplete="new-password"><span>Password · min 10 characters</span></label>
<button class="btn" type="submit">Create account</button>
</form></div></div></div>"#,
            logo = svg("ghost"),
            title = esc(&title),
            role = esc(&i.role),
            token = esc(&token),
            email = esc(&i.email),
        ),
        _ => format!(
            r#"<div class="auth"><div class="auth-box"><div class="auth-logo">{logo}</div>
<h1>Invitation not valid</h1>
<p class="sub">This invitation has expired or has already been used. Ask whoever invited you to send a new one.</p>
<div class="card"><a class="btn tonal" href="/ghost/signin/">Go to sign in</a></div></div></div>"#,
            logo = svg("ghost"),
        ),
    };
    page_u("Accept invitation", "", None, &body, false).into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AcceptInviteForm {
    name: String,
    password: String,
}

async fn invite_accept_submit(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Form(form): Form<AcceptInviteForm>,
) -> Response {
    let Some(invite) = repos::staff::find_invite_by_token(&state.db, &token).await.ok().flatten() else {
        return Redirect::to("/ghost/signin/").into_response();
    };
    if invite.is_expired() {
        return Redirect::to("/ghost/signin/").into_response();
    }
    let name = form.name.trim();
    if name.is_empty() || form.password.chars().count() < 10 {
        return Redirect::to(&format!("/ghost/invite/{token}/")).into_response();
    }
    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return Redirect::to(&format!("/ghost/invite/{token}/")).into_response(),
    };
    let created = repos::users::insert_staff(&state.db, name, &invite.email, &hash, &invite.role).await;
    match created {
        Ok(user) => {
            // Single use: the invite is spent whether or not signin succeeds.
            let _ = repos::staff::delete_invite_by_email(&state.db, &invite.email).await;
            match repos::sessions::create(&state.db, &user.id).await {
                Ok(sid) => {
                    let mut resp = Redirect::to("/ghost/posts/").into_response();
                    if let Ok(v) = format!(
                        "ghost-admin-api-session={sid}; Path={}/ghost; HttpOnly; SameSite=Lax; Max-Age={}",
                        ghost_core::url::subdir(),
                        60 * 60 * 24 * 7
                    )
                    .parse()
                    {
                        resp.headers_mut().insert(header::SET_COOKIE, v);
                    }
                    resp
                }
                Err(_) => Redirect::to("/ghost/signin/").into_response(),
            }
        }
        Err(e) => {
            tracing::error!("staff: could not create invited user {}: {}", invite.email, e.message());
            Redirect::to(&format!("/ghost/invite/{token}/")).into_response()
        }
    }
}

/// The editable profile fields, in Ghost's own order on that screen.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ProfileForm {
    name: String,
    slug: String,
    email: String,
    location: String,
    bio: String,
    profile_image: String,
    cover_image: String,
    meta_title: String,
    meta_description: String,
}

/// The Social Links tab posts the website plus every social handle. All inputs
/// always render, so an empty field means "cleared", never "untouched".
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SocialForm {
    website: String,
    twitter: String,
    facebook: String,
    linkedin: String,
    bluesky: String,
    threads: String,
    mastodon: String,
    tiktok: String,
    youtube: String,
    instagram: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

/// The staff user page's tabs, keyed by the `?tab=` the seg-bar carries.
/// Ghost's own user screen splits Profile / Social Links / Email Notifications
/// the same way.
const PROFILE_TABS: &[(&str, &str)] = &[
    ("profile", "Profile"),
    ("social-links", "Social Links"),
    ("email-notifications", "Email Notifications"),
];

fn profile_tab(q: &HashMap<String, String>) -> &'static str {
    let tab = q.get("tab").map(String::as_str).unwrap_or("profile");
    PROFILE_TABS
        .iter()
        .find(|(key, _)| *key == tab)
        .map(|(key, _)| *key)
        .unwrap_or("profile")
}

fn profile_flash(slug: &str, kind: &str, message: &str) -> Response {
    profile_flash_tab(slug, "profile", kind, message)
}

fn profile_flash_tab(slug: &str, tab: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/staff/{slug}/?tab={tab}&{encoded}")).into_response()
}

/// Ghost lets an owner/admin open any staff member's profile, and everyone
/// open their own. There are no non-owner roles wired up here yet, so the
/// rule is simply: your own profile, or any profile if you are the owner.
async fn profile_target(
    state: &AppState,
    viewer: &ghost_core::models::User,
    slug: &str,
) -> Option<ghost_core::models::User> {
    let target = repos::users::find_by_slug(&state.db, slug).await.ok().flatten()?;
    if target.id == viewer.id {
        return Some(target);
    }
    let role = repos::users::role_name(&state.db, &viewer.id).await.ok().flatten().unwrap_or_default();
    matches!(role.as_str(), "Owner" | "Administrator").then_some(target)
}

async fn profile_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let is_self = target.id == user.id;
    let role = repos::users::role_name(&state.db, &target.id).await.ok().flatten().unwrap_or_default();
    let v = |s: Option<&str>| esc(s.unwrap_or(""));
    let initial = target.name.as_deref().unwrap_or("?").chars().next().unwrap_or('?').to_uppercase().to_string();
    let avatar = match target.profile_image.as_deref().filter(|s| !s.is_empty()) {
        Some(img) => format!(
            r#"<img class="avatar lg" src="{}" alt="" onerror="this.replaceWith(Object.assign(document.createElement('span'),{{className:'avatar lg',textContent:'{}'}}))">"#,
            esc(img),
            esc(&initial)
        ),
        None => format!(r#"<span class="avatar lg">{}</span>"#, esc(&initial)),
    };

    // Role, suspension and removal are staff administration, so they are
    // gated on the viewer rather than on whose profile this is — and the
    // owner is exempt from all three.
    let viewer_role = role_of(&state, &user).await;
    let manage = may_act_on(viewer_role, Role::from_name(&role));
    let tab = profile_tab(&q);
    // Ghost's Email notifications tab gates the growth toggles on the edited
    // user being an administrator, and half of them on Stripe being set up.
    let target_is_admin = Role::from_name(&role).is_admin();
    let stripe_on = ghost_stripe::StripeConfig::from_settings(&state.settings).is_some();
    let admin_card = if manage && role != "Owner" {
        let suspended = target.status.as_deref() == Some("inactive");
        let self_note = if is_self {
            r#"<p class="muted" style="margin:10px 0 0">You cannot suspend or remove your own account.</p>"#
        } else {
            ""
        };
        format!(
            r#"<div class="section">Role &amp; access</div>
<div class="card rows rise" style="--i:5">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{people}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/staff/{slug}/role/" class="inline-form">
<label class="field fixed" style="min-width:200px;margin:0"><select name="role">{role_opts}</select><span>Role</span>{car}</label>
<button class="btn tonal small" type="submit">Change role</button>
</form>
<p class="muted" style="margin:10px 0 0">{role_help}</p>
</div></div>
{status_row}
<div class="row"><span class="ic tint-red">{del}</span>
<div class="body"><span class="title">Remove from the team</span><span class="detail">Their posts stay; the account and its sessions are deleted.</span></div>
<div class="trailing">{delete_btn}</div></div>
</div>{self_note}"#,
            people = svg("ghost"),
            slug = esc(&slug),
            role_opts = role_options_for(viewer_role, &role),
            car = SELECT_CARET,
            role_help = repos::staff::ASSIGNABLE_ROLES
                .iter()
                .find(|(n, _)| *n == role)
                .map(|(_, d)| esc(d))
                .unwrap_or_default(),
            del = svg("delete"),
            status_row = if is_self {
                String::new()
            } else {
                format!(
                    r#"<div class="row"><span class="ic tint-amber">{flag}</span>
<div class="body"><span class="title">{title}</span><span class="detail">{detail}</span></div>
<div class="trailing"><form method="post" action="/ghost/settings/staff/{slug}/status/">
<input type="hidden" name="status" value="{next}">
<button class="btn tonal small" type="submit">{label}</button></form></div></div>"#,
                    flag = svg("flag"),
                    slug = esc(&slug),
                    title = if suspended { "Reactivate account" } else { "Suspend account" },
                    detail = if suspended {
                        "They can sign in again."
                    } else {
                        "Blocks sign in without deleting anything."
                    },
                    next = if suspended { "active" } else { "inactive" },
                    label = if suspended { "Reactivate" } else { "Suspend" },
                )
            },
            delete_btn = if is_self {
                r#"<button class="btn tonal small" disabled>Delete</button>"#.to_string()
            } else {
                format!(
                    r#"<form method="post" action="/ghost/settings/staff/{slug}/delete/" data-confirm="Remove staff member|&#8220;{name}&#8221; will lose access to this site.">
<button class="btn tonal small danger" type="submit">Delete</button></form>"#,
                    slug = esc(&slug),
                    name = esc(target.name.as_deref().unwrap_or("")),
                )
            },
            self_note = self_note,
        )
    } else {
        String::new()
    };

    // Changing your own password needs the current one; an owner or admin
    // resetting somebody else's does not — Ghost only asks the old password
    // of the account it belongs to.
    let password_card = if is_self || manage {
        format!(
            r#"<div class="section">Password</div>
<div class="card rows rise" style="--i:4">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{lock}</span>
<div class="body" style="width:100%">
<form method="post" action="/ghost/settings/staff/{slug}/password/" class="grid2" style="gap:12px">
{current_row}
<label class="field" style="margin:0"><input type="password" name="new_password" placeholder=" " autocomplete="new-password" minlength="10" required><span>New password · min 10 characters</span></label>
<label class="field" style="margin:0"><input type="password" name="confirm_password" placeholder=" " autocomplete="new-password" minlength="10" required><span>Confirm new password</span></label>
<div><button class="btn tonal small" type="submit">{label}</button></div>
</form>
<p class="muted" style="margin:10px 0 0">{note}</p>
</div></div>
</div>"#,
            lock = svg("logout"),
            slug = esc(&slug),
            current_row = if is_self {
                r#"<label class="field" style="margin:0"><input type="password" name="current_password" placeholder=" " autocomplete="current-password" required><span>Current password</span></label>
<span></span>"#
            } else {
                r#"<span></span><span></span>"#
            },
            label = if is_self { "Change password" } else { "Set new password" },
            note = if is_self {
                "Changing it signs this account out of every other session."
            } else {
                "Their sessions are all ended and they sign in with the new password."
            },
        )
    } else {
        String::new()
    };

    // --- the tab bar (Ghost splits this screen the same three ways) -------
    let tab_bar = format!(
        r#"<div class="seg-bar">{}</div>"#,
        PROFILE_TABS
            .iter()
            .map(|(key, label)| {
                format!(
                    r#"<a class="seg{on}" href="/ghost/settings/staff/{slug}/?tab={key}">{label}</a>"#,
                    on = if *key == tab { " on" } else { "" },
                    slug = esc(&slug),
                    key = key,
                    label = label,
                )
            })
            .collect::<String>()
    );
    let save_form = match tab {
        "social-links" => "social-form",
        "email-notifications" => "notify-form",
        _ => "profile-form",
    };

    // --- Profile: Ghost's field order (name, slug, email, location, bio) --
    let profile_body = if tab == "profile" {
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/" id="profile-form">
<div class="section">Details</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-pink">{person}</span>
<div class="body" style="width:100%">
<div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="name" value="{name_v}" placeholder=" " maxlength="191" required><span>Full name</span></label>
<label class="field" style="margin:0"><input name="slug" value="{slug_v}" placeholder=" " maxlength="191"><span>Slug</span></label>
<label class="field" style="margin:0"><input name="email" type="email" value="{email_v}" placeholder=" " maxlength="191" required><span>Email · used for notifications</span></label>
<label class="field" style="margin:0"><input name="location" value="{location}" placeholder=" " maxlength="150"><span>Location</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="bio" rows="4" placeholder=" " maxlength="2000">{bio}</textarea><span>Bio</span></label>
<p class="muted" style="margin:6px 0 0">Recommended: 250 characters. You&rsquo;ve used {bio_len}. Themes read these as <code>{{{{author.bio}}}}</code> and friends.</p>
</div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{image}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="profile_image" value="{profile_image}" placeholder=" "><span>Profile image URL</span></label>
<label class="field" style="margin:0"><input name="cover_image" value="{cover_image}" placeholder=" "><span>Cover image URL</span></label>
</div></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-slate">{seo}</span>
<div class="body" style="width:100%"><div class="grid2" style="gap:12px">
<label class="field" style="margin:0"><input name="meta_title" value="{meta_title}" placeholder=" " maxlength="300"><span>Meta title</span></label>
<label class="field" style="margin:0"><input name="meta_description" value="{meta_description}" placeholder=" " maxlength="500"><span>Meta description</span></label>
</div></div></div>
</div>
</form>
<p class="muted" style="margin:14px 0 0 16px">Public author page: <a href="{author_url}" target="_blank" rel="noopener">{author_url}</a></p>
{password_card}
{admin_card}"#,
            slug = esc(&slug),
            name_v = v(target.name.as_deref()),
            slug_v = v(target.slug.as_deref()),
            email_v = v(target.email.as_deref()),
            location = v(target.location.as_deref()),
            bio = v(target.bio.as_deref()),
            bio_len = target.bio.as_deref().map(|b| b.chars().count()).unwrap_or(0),
            profile_image = v(target.profile_image.as_deref()),
            cover_image = v(target.cover_image.as_deref()),
            meta_title = v(target.meta_title.as_deref()),
            meta_description = v(target.meta_description.as_deref()),
            person = svg("ghost"),
            image = svg("image"),
            seo = svg("history"),
            author_url = esc(&author_archive_url(&state, &target).await),
            password_card = password_card,
            admin_card = admin_card,
        )
    } else {
        String::new()
    };

    // --- Social Links: Website plus the platforms Ghost offers, in its order
    let social_body = if tab == "social-links" {
        let social = |name: &str, label: &str, placeholder: &str, value: &str| {
            format!(
                r#"<label class="field" style="margin:0"><input name="{name}" value="{value}" placeholder="{placeholder}" maxlength="191"><span>{label}</span></label>"#,
                name = name,
                value = esc(value),
                placeholder = esc(placeholder),
                label = label,
            )
        };
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/social/" id="social-form">
<div class="section">Social links</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{link}</span>
<div class="body" style="width:100%">
<label class="field" style="margin:0"><input name="website" value="{website}" placeholder="https://example.com" maxlength="2000"><span>Website</span></label>
<div class="grid2" style="gap:12px;margin-top:12px">
{twitter}
{facebook}
{linkedin}
{bluesky}
{threads}
{mastodon}
{tiktok}
{youtube}
{instagram}
</div>
<p class="muted" style="margin:12px 0 0">Shown on your public author page. Full profile URLs, or just the username.</p>
</div></div>
</div>
</form>"#,
            slug = esc(&slug),
            link = svg("link"),
            website = v(target.website.as_deref()),
            twitter = social("twitter", "X", "https://x.com/username", target.twitter.as_deref().unwrap_or("")),
            facebook = social("facebook", "Facebook", "https://www.facebook.com/username", target.facebook.as_deref().unwrap_or("")),
            linkedin = social("linkedin", "LinkedIn", "https://www.linkedin.com/in/username", target.linkedin.as_deref().unwrap_or("")),
            bluesky = social("bluesky", "Bluesky", "https://bsky.app/profile/username", target.bluesky.as_deref().unwrap_or("")),
            threads = social("threads", "Threads", "https://threads.net/@username", target.threads.as_deref().unwrap_or("")),
            mastodon = social("mastodon", "Mastodon", "https://mastodon.social/@username", target.mastodon.as_deref().unwrap_or("")),
            tiktok = social("tiktok", "TikTok", "https://www.tiktok.com/@username", target.tiktok.as_deref().unwrap_or("")),
            youtube = social("youtube", "YouTube", "https://www.youtube.com/@channel", target.youtube.as_deref().unwrap_or("")),
            instagram = social("instagram", "Instagram", "https://www.instagram.com/username", target.instagram.as_deref().unwrap_or("")),
        )
    } else {
        String::new()
    };

    // --- Email Notifications: the same groups and wordings as Ghost's screen.
    // `shown` markers tell the save handler which switches were actually
    // offered, so a toggle hidden by role or Stripe stays untouched.
    let notifications_body = if tab == "email-notifications" {
        let toggle = |key: &str, title: &str, detail: &str, on: bool| {
            format!(
                r#"{row}<input type="hidden" name="shown" value="{key}">"#,
                row = toggle_row(key, title, detail, on),
                key = key,
            )
        };
        let engagement = format!(
            r#"<div class="section">Engagement</div>
<div class="card rows rise" style="--i:2">{comments}{recommendations}</div>"#,
            comments = toggle(
                "comment_notifications",
                "Comments",
                "Every time a member comments on one of your posts",
                target.comment_notifications,
            ),
            recommendations = if target_is_admin {
                toggle(
                    "recommendation_notifications",
                    "Recommendations",
                    "Every time another publisher recommends you to their audience",
                    target.recommendation_notifications,
                )
            } else {
                String::new()
            },
        );
        let growth = if target_is_admin {
            format!(
                r#"<div class="section">Members</div>
<div class="card rows rise" style="--i:3">{signups}{paid_started}{paid_canceled}</div>
<div class="section">Revenue</div>
<div class="card rows rise" style="--i:4">{milestones}{donations}{gifts}</div>"#,
                signups = toggle(
                    "free_member_signup_notification",
                    "New signups",
                    "Every time a new free member signs up",
                    target.free_member_signup_notification,
                ),
                paid_started = if stripe_on {
                    toggle(
                        "paid_subscription_started_notification",
                        "New paid members",
                        "Every time a member starts a new paid subscription",
                        target.paid_subscription_started_notification,
                    )
                } else {
                    String::new()
                },
                paid_canceled = if stripe_on {
                    toggle(
                        "paid_subscription_canceled_notification",
                        "Paid member cancellations",
                        "Every time a member cancels their paid subscription",
                        target.paid_subscription_canceled_notification,
                    )
                } else {
                    String::new()
                },
                milestones = toggle(
                    "milestone_notifications",
                    "Milestones",
                    if stripe_on {
                        "Occasional summaries of your audience & revenue growth"
                    } else {
                        "Occasional summaries of your audience growth"
                    },
                    target.milestone_notifications,
                ),
                donations = if stripe_on {
                    toggle(
                        "donation_notifications",
                        "Tips & donations",
                        "Every time you receive a one-time payment",
                        target.donation_notifications,
                    )
                } else {
                    String::new()
                },
                gifts = if stripe_on {
                    toggle(
                        "gift_subscription_notifications",
                        "Gift subscriptions",
                        "Every time someone purchases or redeems a gift subscription",
                        target.gift_subscription_notifications,
                    )
                } else {
                    String::new()
                },
            )
        } else {
            String::new()
        };
        format!(
            r#"<form method="post" action="/ghost/settings/staff/{slug}/notifications/" id="notify-form">{engagement}{growth}</form>"#,
            slug = esc(&slug),
            engagement = engagement,
            growth = growth,
        )
    } else {
        String::new()
    };

    page_for(
        "Profile",
        "settings",
        Some(&user),
        viewer_role,
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/staff/" title="Back to staff">{back}</a>
<div><h1>{name}</h1><p class="sub">{role_label}</p></div>
<div class="actions"><button class="btn" type="submit" form="{save_form}">{check} Save</button></div></header>
{banner}
<div class="card padded rise" style="--i:1;display:flex;align-items:center;gap:16px;flex-wrap:wrap">
{avatar}
<div style="min-width:0"><b style="font-size:17px;display:block">{name}</b>
<span class="muted">{email}</span></div>
<div class="trailing"><span class="chip tonal">{role_label}</span></div>
</div>
{tab_bar}
{profile_body}
{social_body}
{notifications_body}"#,
            back = svg("chevron"),
            check = svg("check"),
            save_form = save_form,
            banner = flash_banner(&q),
            avatar = avatar,
            name = v(target.name.as_deref()),
            email = v(target.email.as_deref()),
            role_label = esc(&role),
            tab_bar = tab_bar,
            profile_body = profile_body,
            social_body = social_body,
            notifications_body = notifications_body,
        ),
        true,
    )
    .into_response()
}

async fn profile_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let name = form.name.trim();
    if name.is_empty() {
        return profile_flash(&slug, "error", "Name cannot be empty.");
    }
    let email = form.email.trim().to_lowercase();
    if !email.contains('@') {
        return profile_flash(&slug, "error", "Please enter a valid email address.");
    }
    // Emails identify a staff account at signin, so a collision would lock
    // somebody out of theirs.
    if let Ok(Some(other)) = repos::users::find_by_email(&state.db, &email).await {
        if other.id != target.id {
            return profile_flash(&slug, "error", "Another staff member already uses that email.");
        }
    }

    let changes = repos::users::UserChanges {
        name: Some(name.to_string()),
        email: Some(email),
        bio: Some(form.bio.trim().to_string()),
        location: Some(form.location.trim().to_string()),
        profile_image: Some(form.profile_image.trim().to_string()),
        cover_image: Some(form.cover_image.trim().to_string()),
        meta_title: Some(form.meta_title.trim().to_string()),
        meta_description: Some(form.meta_description.trim().to_string()),
        slug: Some(form.slug.trim().to_string()),
        ..Default::default()
    };
    match repos::users::update(&state.db, &target.id, &changes).await {
        // Editing the slug moves the user, so redirect to wherever they now
        // live rather than to a URL that no longer resolves. A rename alone
        // leaves the slug (and this URL) as it was.
        Ok(updated) => profile_flash_tab(
            updated.slug.as_deref().unwrap_or(&slug),
            "profile",
            "ok",
            "Profile saved.",
        ),
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

/// The Social Links tab — every field always renders there, so each one that
/// posts is meant, and an empty value clears it.
async fn profile_social_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<SocialForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if profile_target(&state, &user, &slug).await.is_none() {
        return not_found_page(&user, "That staff member could not be found.");
    }
    let t = |s: &str| Some(s.trim().to_string());
    let changes = repos::users::UserChanges {
        website: t(&form.website),
        twitter: t(&form.twitter),
        facebook: t(&form.facebook),
        linkedin: t(&form.linkedin),
        bluesky: t(&form.bluesky),
        threads: t(&form.threads),
        mastodon: t(&form.mastodon),
        tiktok: t(&form.tiktok),
        youtube: t(&form.youtube),
        instagram: t(&form.instagram),
        ..Default::default()
    };
    match repos::users::update_by_slug(&state.db, &slug, &changes).await {
        Ok(_) => profile_flash_tab(&slug, "social-links", "ok", "Social links saved."),
        Err(e) => profile_flash_tab(&slug, "social-links", "error", &e.message()),
    }
}

/// The Email Notifications tab. Unchecked switches post nothing, so each
/// rendered toggle carries a `shown` marker and only those are written back —
/// the ones a role or a missing Stripe setup hid keep their value.
async fn profile_notifications(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    if profile_target(&state, &user, &slug).await.is_none() {
        return not_found_page(&user, "That staff member could not be found.");
    }
    let shown: Vec<&str> =
        form.iter().filter(|(k, _)| k == "shown").map(|(_, v)| v.as_str()).collect();
    let on = |key: &str| form.iter().any(|(k, v)| k == key && v == "1");
    let flag = |key: &str| shown.contains(&key).then(|| on(key));
    let changes = repos::users::UserChanges {
        comment_notifications: flag("comment_notifications"),
        free_member_signup_notification: flag("free_member_signup_notification"),
        paid_subscription_started_notification: flag("paid_subscription_started_notification"),
        paid_subscription_canceled_notification: flag("paid_subscription_canceled_notification"),
        recommendation_notifications: flag("recommendation_notifications"),
        milestone_notifications: flag("milestone_notifications"),
        donation_notifications: flag("donation_notifications"),
        gift_subscription_notifications: flag("gift_subscription_notifications"),
        ..Default::default()
    };
    match repos::users::update_by_slug(&state.db, &slug, &changes).await {
        Ok(_) => profile_flash_tab(&slug, "email-notifications", "ok", "Email notifications saved."),
        Err(e) => profile_flash_tab(&slug, "email-notifications", "error", &e.message()),
    }
}

async fn profile_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(form): Form<PasswordForm>,
) -> Response {
    let user = match require_session(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r.into_response(),
    };
    let Some(target) = profile_target(&state, &user, &slug).await else {
        return not_found_page(&user, "That staff member could not be found.");
    };
    let is_self = target.id == user.id;
    // Ghost's password endpoint asks the old password only of the account it
    // belongs to; an administrator resetting someone else's just needs edit
    // rights — which profile_target has already checked.
    if !is_self && !role_of(&state, &user).await.is_admin() {
        return profile_flash(&slug, "error", "Only you can change your own password.");
    }
    if form.new_password != form.confirm_password {
        return profile_flash(&slug, "error", "The new passwords do not match.");
    }
    if form.new_password.chars().count() < 10 {
        return profile_flash(&slug, "error", "Password must be at least 10 characters.");
    }
    if is_self {
        let email = user.email.clone().unwrap_or_default();
        match repos::users::verify_password(&state.db, &email, &form.current_password).await {
            Ok(Some(_)) => {}
            _ => return profile_flash(&slug, "error", "Current password is incorrect."),
        }
    }
    let pw = form.new_password.clone();
    let hash = match tokio::task::spawn_blocking(move || bcrypt::hash(pw.as_str(), 12)).await {
        Ok(Ok(h)) => h,
        _ => return profile_flash(&slug, "error", "Could not set that password."),
    };
    let changes = repos::users::UserChanges { password_hash: Some(hash), ..Default::default() };
    match repos::users::update(&state.db, &target.id, &changes).await {
        Ok(_) => {
            // A password change invalidates every session the account holds —
            // a cookie cloned before the change stops working. This session
            // survives its own change; nothing else does.
            let keep = is_self.then(|| session_id_from(&headers)).flatten();
            let _ = repos::sessions::destroy_for_user(&state.db, &target.id, keep.as_deref()).await;
            profile_flash(
                &slug,
                "ok",
                if is_self { "Password changed." } else { "Password set — their other sessions are signed out." },
            )
        }
        Err(e) => profile_flash(&slug, "error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// integrations (shows the content API keys) — a Settings → Advanced sub-page
// ---------------------------------------------------------------------------

/// Integrations used to be a top-level rail item, then lived under Settings →
/// Advanced before becoming a settings page of its own; this keeps links from
/// both eras working.
async fn integrations_redirect() -> Response {
    Redirect::permanent("/ghost/settings/integrations/").into_response()
}

/// The three apps Ghost ships with. Slug, label, blurb, icon, tint.
///
/// Zapier is an integration row in the database because it authenticates with
/// an API key; Slack and Unsplash are configuration only, which is the same
/// split Ghost makes — so the hub lists all three but each page works
/// differently underneath.
const BUILTIN_APPS: &[(&str, &str, &str, &str, &str)] = &[
    ("zapier", "Zapier", "Automate Ghost with 6,000+ apps", "share", "amber"),
    ("slack", "Slack", "Post to a channel when you publish", "send", "purple"),
    ("unsplash", "Unsplash", "Free photography inside the editor", "image", "green"),
    ("firstpromoter", "FirstPromoter", "Referral and affiliate tracking for members", "share", "blue"),
    ("pintura", "Pintura", "Edit images without leaving the editor", "image", "pink"),
    ("transistor", "Transistor.fm", "Private podcast feeds for your members", "send", "teal"),
];

fn integrations_flash(sub: &str, kind: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair(kind, message)
        .finish();
    Redirect::to(&format!("/ghost/settings/integrations/{sub}?{encoded}")).into_response()
}

/// A secret with a copy button. Long keys are shown in full — they have to be
/// copied somewhere, and hiding them behind a reveal only adds a click.
fn key_chip(key_type: &str, secret: &str) -> String {
    format!(
        r#"<span class="key"><span class="chip tonal">{}</span>{}<button type="button" data-copy="{secret}" title="Copy" aria-label="Copy key">{copy}</button></span>"#,
        esc(key_type),
        esc(secret),
        secret = esc(secret),
        copy = svg("copy"),
    )
}

async fn integrations_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    // --- built-in apps -----------------------------------------------------
    let slack_on = ghost_api::slack::SlackConfig::from_settings(&state.settings).is_some();
    let unsplash_on = state.settings.get_bool("unsplash");
    let mut apps = String::new();
    for (slug, name, blurb, icon, tint) in BUILTIN_APPS {
        let status = match *slug {
            "slack" if slack_on => r#"<span class="chip ok">Connected</span>"#,
            "unsplash" if unsplash_on => r#"<span class="chip ok">On</span>"#,
            "unsplash" => r#"<span class="chip tonal">Off</span>"#,
            s if state.settings.get_bool(s) => r#"<span class="chip ok">On</span>"#,
            "firstpromoter" | "pintura" | "transistor" => r#"<span class="chip tonal">Off</span>"#,
            "zapier" => r#"<span class="chip tonal">API key ready</span>"#,
            _ => r#"<span class="chip tonal">Not set up</span>"#,
        };
        apps.push_str(&format!(
            r#"<a class="row" href="/ghost/settings/integrations/{slug}/"><span class="ic tint-{tint}">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{blurb}</span></div>
<div class="trailing">{status}</div></a>"#,
            slug = slug,
            tint = tint,
            ic = svg(icon),
            name = esc(name),
            blurb = esc(blurb),
            status = status,
        ));
    }

    // --- custom integrations ----------------------------------------------
    let customs = repos::integrations::list_custom(&state.db).await.unwrap_or_default();
    let mut custom_rows = String::new();
    for i in &customs {
        let detail = i
            .description
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| format!("/{}", i.slug.clone().unwrap_or_default()));
        custom_rows.push_str(&format!(
            r#"<a class="row" href="/ghost/settings/integrations/custom/{id}/"><span class="ic tint-teal">{ic}</span>
<div class="body"><span class="title">{name}</span><span class="detail">{detail}</span></div>
<span class="chip tonal">2 keys</span></a>"#,
            id = esc(&i.id),
            ic = svg("integrations"),
            name = esc(&i.name),
            detail = esc(&detail),
        ));
    }
    if custom_rows.is_empty() {
        custom_rows = r#"<div class="row"><div class="body"><span class="detail">No custom integrations yet. Add one to get a Content API and Admin API key for your own scripts.</span></div></div>"#.to_string();
    }

    // --- the frontend's own key -------------------------------------------
    let mut internal = String::new();
    if let Ok(Some(key)) = repos::integrations::internal_content_key(&state.db).await {
        internal = format!(
            r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-green">{ic}</span>
<div class="body"><span class="title">Ghost Core Content API</span>
<div style="margin-top:6px">{chip}</div>
<span class="detail">curl "{site}/ghost/api/content/posts/?key={key}"</span></div></div>"#,
            ic = svg("code"),
            chip = key_chip("content", &key),
            site = esc(&state.site_url()),
            key = esc(&key),
        );
    }

    page_for_role(
        actor_role,
        "Integrations",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>Integrations</h1><p class="sub">Built-in apps, API keys &amp; your own scripts</p></div>
<div class="actions"><a class="btn text" href="https://ghost.org/integrations/" target="_blank" rel="noopener">Browse integrations</a></div></header>
{banner}
<div class="section">Built-in apps</div>
<div class="card rows rise" style="--i:1">{apps}</div>
<div class="section">Custom integrations</div>
<div class="card rows rise" style="--i:2">{custom_rows}</div>
<div class="card padded rise" style="--i:3"><form method="post" action="/ghost/settings/integrations/">
<div class="section" style="margin-top:0">Add a custom integration</div>
<p class="muted" style="margin:0 0 12px">Creates a Content API key and an Admin API key you can use from your own code.</p>
<div class="inline-form">
{name_field}{desc_field}
<button class="btn" type="submit">{add} Add integration</button></div></form></div>
<div class="section">Content API</div>
<div class="card rows rise" style="--i:4">{internal}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            apps = apps,
            custom_rows = custom_rows,
            name_field = text_field("name", "Name", ""),
            desc_field = text_field("description", "Description (optional)", ""),
            add = svg("add"),
            internal = internal,
        ),
        true,
    )
    .into_response()
}

#[derive(Deserialize)]
struct IntegrationForm {
    name: String,
    #[serde(default)]
    description: String,
}

async fn integration_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<IntegrationForm>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match repos::integrations::create_custom(&state.db, &form.name, Some(&form.description)).await {
        Ok(i) => {
            let _ = repos::actions::record(&state.db, "integrations", &i.id, &actor.user.id, "added").await;
            integrations_flash(&format!("custom/{}/", i.id), "ok", "Integration created.")
        }
        Err(e) => integrations_flash("", "error", &e.message()),
    }
}

/// One custom integration: rename it, read its keys, delete it.
async fn custom_integration_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let Ok(Some(i)) = repos::integrations::find_by_id(&state.db, &id).await else {
        return not_found_page_for(actor_role, &user, "That integration does not exist.");
    };

    let mut keys = String::new();
    if let Ok(list) = repos::api_keys::find_by_integration(&state.db, &i.id).await {
        for k in list {
            // an admin key is an `id:secret` pair; both halves sign the JWT
            let shown = match k.key_type.as_str() {
                "admin" => format!("{}:{}", k.id, k.secret),
                _ => k.secret.clone(),
            };
            keys.push_str(&format!(
                r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-teal">{ic}</span>
<div class="body"><span class="title">{label}</span><div style="margin-top:6px">{chip}</div></div></div>"#,
                ic = svg("code"),
                label = if k.key_type == "admin" { "Admin API key" } else { "Content API key" },
                chip = key_chip(&k.key_type, &shown),
            ));
        }
    }

    // Each webhook row reports its last delivery, which is the only way to
    // tell a listener that is working from one quietly failing.
    let hooks = repos::webhooks::list_for_integration(&state.db, &i.id).await.unwrap_or_default();
    let mut webhook_rows = String::new();
    for h in &hooks {
        let last = match (&h.last_triggered_at, &h.last_triggered_status) {
            (Some(at), Some(status)) => {
                let ok = status.starts_with('2');
                format!(
                    r#"<span class="chip {chip}">{status}</span><span class="detail" style="margin-left:8px">{at}{err}</span>"#,
                    chip = if ok { "published" } else { "draft" },
                    status = esc(status),
                    at = esc(&at[..16.min(at.len())]),
                    err = match h.last_triggered_error.as_deref().filter(|e| !e.is_empty()) {
                        Some(e) => format!(" · {}", esc(e)),
                        None => String::new(),
                    },
                )
            }
            _ => r#"<span class="detail">never fired</span>"#.to_string(),
        };
        webhook_rows.push_str(&format!(
            r#"<div class="row"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{label}{name}</span><span class="detail">{url}</span>
<div style="margin-top:6px">{last}</div></div>
<div class="trailing">
<form method="post" action="/ghost/settings/integrations/custom/{iid}/webhooks/{wid}/delete/" data-confirm="Delete webhook|Nothing will be sent to {url} when this event fires.">
<button class="icon-btn danger" title="Delete" aria-label="Delete">{del}</button></form></div></div>"#,
            ic = svg("share"),
            label = esc(ghost_api::webhooks::event_label(&h.event)),
            name = match h.name.as_deref().filter(|n| !n.is_empty()) {
                Some(n) => format!(" · {}", esc(n)),
                None => String::new(),
            },
            url = esc(&h.target_url),
            last = last,
            iid = esc(&i.id),
            wid = esc(&h.id),
            del = svg("delete"),
        ));
    }
    if webhook_rows.is_empty() {
        webhook_rows = r#"<div class="row"><div class="body"><span class="muted">No webhooks yet — add one below to be told when something changes.</span></div></div>"#.into();
    }

    page_for_role(
        actor_role,
        &i.name,
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>{name}</h1><p class="sub">Custom integration</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/custom/{id}/">
{name_field}{desc_field}
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>
<div class="section">API keys</div>
<div class="card rows rise" style="--i:2">{keys}</div>
<div class="card rows rise" style="--i:3"><div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-blue">{globe}</span>
<div class="body"><span class="title">API URL</span><div style="margin-top:6px">{api_chip}</div></div></div></div>
<div class="section">Webhooks</div>
<div class="card rows rise" style="--i:4">{webhook_rows}</div>
<div class="card padded rise" style="--i:4">
<form method="post" action="/ghost/settings/integrations/custom/{id}/webhooks/new/" class="inline-form">
<label class="field" style="min-width:160px;margin:0"><input name="name" placeholder=" " maxlength="191"><span>Name · optional</span></label>
<label class="field fixed" style="min-width:230px;margin:0"><select name="event" required>{event_opts}</select><span>Event</span>{car}</label>
<label class="field" style="flex:1;min-width:220px;margin:0"><input name="target_url" type="url" placeholder="https://example.com/hook" maxlength="2000" required><span>Target URL</span></label>
<label class="field" style="min-width:150px;margin:0"><input name="secret" placeholder=" " maxlength="191"><span>Secret · optional</span></label>
<button class="btn tonal" type="submit">{add} Add webhook</button>
</form>
<p class="muted" style="margin:12px 0 0">Ghost POSTs the changed resource as JSON when the event fires. Give a secret and each
delivery carries an <code>X-Ghost-Signature</code> header — HMAC-SHA256 of the body — so your endpoint can tell a real
delivery from anyone who learned the URL.</p>
</div>
<div class="section">Danger zone</div>
<div class="card padded rise" style="--i:4">
<form method="post" action="/ghost/settings/integrations/custom/{id}/delete/" data-confirm="Delete integration|&#8220;{name}&#8221; and its API keys will be removed. Anything using them will stop working.">
<button class="btn danger" type="submit">{del} Delete integration</button></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            name = esc(&i.name),
            id = esc(&i.id),
            name_field = text_field("name", "Name", &i.name),
            desc_field = text_field("description", "Description", i.description.as_deref().unwrap_or("")),
            keys = keys,
            globe = svg("globe"),
            api_chip = key_chip("url", &format!("{}/ghost/api", state.site_url())),
            webhook_rows = webhook_rows,
            event_opts = ghost_api::webhooks::EVENT_GROUPS
                .iter()
                .map(|(group, events)| format!(
                    r#"<optgroup label="{g}">{opts}</optgroup>"#,
                    g = esc(group),
                    opts = events
                        .iter()
                        .map(|(v, l)| format!(
                            r#"<option value="{v}">{l}</option>"#,
                            v = esc(v),
                            l = esc(l)
                        ))
                        .collect::<String>(),
                ))
                .collect::<String>(),
            car = SELECT_CARET,
            add = svg("add"),
            del = svg("delete"),
        ),
        true,
    )
    .into_response()
}

/// Fire the webhooks a post or page change should raise.
///
/// Ghost sends the resource under `{post: {current, previous}}`, and raises
/// both the generic `edited` and whichever publish transition happened — a
/// listener for "post published" should hear about it without also having to
/// subscribe to every edit and diff the status itself.
fn fire_post_webhooks(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    change: &str,
    was_published: bool,
) {
    fire_post_webhooks_from(state, post_type, post, change, was_published, "", None)
}

/// As above, but told what the status *was*, so the scheduling transitions
/// can be worked out rather than guessed.
///
/// `previous_status` empty means "not known", which is how the delete and
/// create paths call in — neither can schedule anything.
fn fire_post_webhooks_from(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    change: &str,
    was_published: bool,
    previous_status: &str,
    before: Option<&ghost_core::models::Post>,
) {
    ghost_api::federation::post_changed(state,&post.id,change=="deleted");
    let is_page = post_type == "page";
    let now_published = post.status == "published";
    let body = |p: &ghost_core::models::Post| {
        serde_json::json!({
            "id": p.id,
            "uuid": p.uuid,
            "title": p.title,
            "slug": p.slug,
            "status": p.status,
            "visibility": p.visibility,
            "published_at": p.published_at,
            "updated_at": p.updated_at,
            "excerpt": p.custom_excerpt,
            "url": format!("{}/{}/", state.site_url().trim_end_matches('/'), p.slug.clone().unwrap_or_default()),
        })
    };
    let key = if is_page { "page" } else { "post" };
    // Ghost sends the prior value of each field that changed, and nothing
    // for the fields that did not. A receiver diffing an edit needs that —
    // "the title changed, and here is what it was" is a different message
    // from "something about this post changed".
    let previous = match before {
        Some(old) => changed_fields(&body(old), &body(post)),
        None => serde_json::json!({}),
    };
    let payload = serde_json::json!({ key: { "current": body(post), "previous": previous } });

    let mut events: Vec<&'static str> = Vec::new();
    match change {
        "added" => events.push(if is_page { "page.added" } else { "post.added" }),
        "deleted" => events.push(if is_page { "page.deleted" } else { "post.deleted" }),
        _ => events.push(if is_page { "page.edited" } else { "post.edited" }),
    }
    if change != "deleted" {
        match (was_published, now_published) {
            (false, true) => events.push(if is_page { "page.published" } else { "post.published" }),
            (true, true) if change == "edited" => {
                events.push(if is_page { "page.published.edited" } else { "post.published.edited" })
            }
            (true, false) => {
                events.push(if is_page { "page.unpublished" } else { "post.unpublished" })
            }
            _ => {}
        }
        // Scheduling, as Ghost splits it. The old code raised a bare
        // `post.scheduled` for pages too, so a page listener heard nothing
        // and a post listener heard about pages — and it fired again on
        // every edit of an already-scheduled post, which is what
        // `rescheduled` is for.
        let was_scheduled = previous_status == "scheduled";
        let now_scheduled = post.status == "scheduled";
        match (was_scheduled, now_scheduled) {
            (false, true) if change != "added" => {
                events.push(if is_page { "page.scheduled" } else { "post.scheduled" })
            }
            (true, true) => {
                events.push(if is_page { "page.rescheduled" } else { "post.rescheduled" })
            }
            (true, false) => {
                events.push(if is_page { "page.unscheduled" } else { "post.unscheduled" })
            }
            _ => {}
        }
    }
    for event in events {
        ghost_api::webhooks::fire(state, event, payload.clone());
    }
}

/// The entries of `before` whose value differs in `after`.
///
/// Ghost's `previous` is exactly this: the old values of the fields that
/// moved, not a whole second copy of the resource. Sending the lot would
/// make every edit look like every field changed.
fn changed_fields(before: &serde_json::Value, after: &serde_json::Value) -> serde_json::Value {
    let (Some(b), Some(a)) = (before.as_object(), after.as_object()) else {
        return serde_json::json!({});
    };
    let mut out = serde_json::Map::new();
    for (k, old) in b {
        if a.get(k) != Some(old) {
            out.insert(k.clone(), old.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// `post.tag.attached` / `post.tag.detached`, and the page equivalents.
///
/// Ghost raises one event per tag that joined or left, carrying both the
/// entry and the tag — a listener wants to know *which* tag moved, which a
/// bare `post.edited` cannot tell it. These four were in the picker and had
/// no code behind them at all.
async fn fire_tag_link_webhooks(
    state: &AppState,
    post_type: &str,
    post: &ghost_core::models::Post,
    before: &[String],
    after: &[String],
) {
    let attached: Vec<&String> = after.iter().filter(|t| !before.contains(t)).collect();
    let detached: Vec<&String> = before.iter().filter(|t| !after.contains(t)).collect();
    if attached.is_empty() && detached.is_empty() {
        return;
    }
    let is_page = post_type == "page";
    let key = if is_page { "page" } else { "post" };
    for (tag_ids, event) in [
        (attached, if is_page { "page.tag.attached" } else { "post.tag.attached" }),
        (detached, if is_page { "page.tag.detached" } else { "post.tag.detached" }),
    ] {
        for tag_id in tag_ids {
            // A detached tag still exists, so it can be described in full.
            let Ok(Some(tag)) = repos::tags::find_by_id(&state.db, tag_id).await else {
                continue;
            };
            ghost_api::webhooks::fire(
                state,
                event,
                serde_json::json!({
                    key: { "current": { "id": post.id, "title": post.title, "slug": post.slug } },
                    "tag": { "current": { "id": tag.id, "name": tag.name, "slug": tag.slug } },
                }),
            );
        }
    }
}

/// `site.changed` — Ghost's whole-site invalidation event.
///
/// Ghost raises it whenever a response carries `X-Cache-Invalidate: /*`,
/// which is its way of saying the rendered site is now different. This port
/// has no such header, so it is raised from the saves that mean the same
/// thing: settings, navigation, code injection, the active theme and the
/// routing files. Cache purges and static rebuilds hang off this event, so a
/// site that never fires it serves stale pages until something else evicts
/// them.
fn fire_site_changed(state: &AppState) {
    ghost_api::webhooks::fire(state, "site.changed", serde_json::json!({}));
}

/// A settings save that changed the rendered site: flash as usual, and tell
/// the integrations.
fn settings_saved(state: &AppState, group: &str, message: &str) -> Response {
    fire_site_changed(state);
    settings_flash(group, "ok", message)
}

/// Tags, in Ghost's `{tag: {current, previous}}` shape.
///
/// The three tag events were in the picker from the start and never fired,
/// so a listener set up against one simply never heard anything.
fn fire_tag_webhook(state: &AppState, event: &'static str, tag: &ghost_core::models::Tag) {
    ghost_api::webhooks::fire(
        state,
        event,
        serde_json::json!({
            "tag": {
                "current": {
                    "id": tag.id,
                    "name": tag.name,
                    "slug": tag.slug,
                    "description": tag.description,
                    "visibility": tag.visibility,
                },
                "previous": {}
            }
        }),
    );
}

/// The members' equivalent, delegating to the shared builder so the admin's
/// form and Portal signup emit the identical shape.
fn fire_member_webhook(state: &AppState, event: &'static str, id: &str, email: &str, name: &str) {
    ghost_api::webhooks::fire_member(state, event, id, email, name, serde_json::json!({}));
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct WebhookForm {
    name: String,
    event: String,
    target_url: String,
    secret: String,
}

fn integration_flash(id: &str, kind: &str, message: &str) -> Response {
    let encoded: String =
        form_urlencoded::Serializer::new(String::new()).append_pair(kind, message).finish();
    Redirect::to(&format!("/ghost/settings/integrations/custom/{id}/?{encoded}"))
        .into_response()
}

async fn webhook_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<WebhookForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // An event this build cannot fire would be registered and never heard
    // from, which is worse than refusing it.
    if !ghost_api::webhooks::is_known_event(form.event.trim()) {
        return integration_flash(&id, "error", "Choose an event from the list.");
    }
    let url = form.target_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return integration_flash(&id, "error", "The target URL must start with http:// or https://.");
    }
    match repos::webhooks::create(
        &state.db,
        &id,
        form.event.trim(),
        url,
        form.name.trim(),
        form.secret.trim(),
    )
    .await
    {
        Ok(_) => integration_flash(&id, "ok", "Webhook added."),
        Err(e) => integration_flash(&id, "error", &e.message()),
    }
}

async fn webhook_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, wid)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match repos::webhooks::delete(&state.db, &wid).await {
        Ok(()) => integration_flash(&id, "ok", "Webhook deleted."),
        Err(e) => integration_flash(&id, "error", &e.message()),
    }
}

async fn custom_integration_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<IntegrationForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let sub = format!("custom/{id}/");
    match repos::integrations::update_custom(&state.db, &id, &form.name, Some(&form.description)).await {
        Ok(()) => integrations_flash(&sub, "ok", "Integration saved."),
        Err(e) => integrations_flash(&sub, "error", &e.message()),
    }
}

async fn custom_integration_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let actor = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    match repos::integrations::delete_custom(&state.db, &id).await {
        Ok(()) => {
            let _ = repos::actions::record(&state.db, "integrations", &id, &actor.user.id, "deleted").await;
            integrations_flash("", "ok", "Integration deleted.")
        }
        Err(e) => integrations_flash(&format!("custom/{id}/"), "error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// built-in app: Zapier
// ---------------------------------------------------------------------------

async fn zapier_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let key = repos::integrations::admin_key_for_slug(&state.db, "zapier")
        .await
        .ok()
        .flatten();
    let key_block = match &key {
        Some(k) => format!(
            r#"<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-amber">{ic}</span>
<div class="body"><span class="title">Admin API key</span><div style="margin-top:6px">{chip}</div></div></div>
<div class="row" style="align-items:flex-start;flex-wrap:wrap"><span class="ic tint-blue">{globe}</span>
<div class="body"><span class="title">API URL</span><div style="margin-top:6px">{url}</div></div></div>"#,
            ic = svg("lock"),
            chip = key_chip("admin", k),
            globe = svg("globe"),
            url = key_chip("url", &format!("{}/ghost/api", state.site_url())),
        ),
        None => r#"<div class="row"><div class="body"><span class="detail">The Zapier key is missing from this database. Restart the server to have it recreated.</span></div></div>"#.to_string(),
    };

    page_for_role(
        actor_role,
        "Zapier",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Zapier</h1><p class="sub">Automate Ghost with 6,000+ apps</p></div></header>
{banner}
<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Paste these into Zapier when it asks to connect to Ghost. The Admin API key lets a Zap create and edit content, so treat it like a password.</span></div>
<div class="section">Credentials</div>
<div class="card rows rise" style="--i:2">{key_block}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            bulb = svg("integrations"),
            key_block = key_block,
        ),
        true,
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// built-in app: Slack
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SlackForm {
    #[serde(default)]
    slack_url: String,
    #[serde(default)]
    slack_username: String,
}

async fn slack_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let url = state.settings.get_str("slack_url").unwrap_or_default();
    let username = state
        .settings
        .get_str("slack_username")
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "Ghost".into());
    let configured = !url.trim().is_empty();
    let test = match configured {
        true => r#"<form method="post" action="/ghost/settings/integrations/slack/test/" style="margin-top:14px"><button class="btn tonal" type="submit">Send test notification</button></form>"#,
        false => "",
    };

    page_for_role(
        actor_role,
        "Slack",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Slack</h1><p class="sub">Post to a channel when you publish</p></div></header>
{banner}
<div class="banner info rise" style="--i:1">{bulb}<span class="msg">Create an <b>Incoming Webhook</b> in Slack (Slack → Apps → Incoming WebHooks), pick a channel, and paste the URL it gives you. The URL is a credential — anyone with it can post to that channel.</span></div>
<div class="card padded rise" style="--i:2"><form method="post" action="/ghost/settings/integrations/slack/">
{url_field}{name_field}
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form>
{test}</div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            bulb = svg("send"),
            url_field = text_field("slack_url", "Webhook URL", &url),
            name_field = text_field("slack_username", "Username shown in Slack", &username),
            test = test,
        ),
        true,
    )
    .into_response()
}

async fn slack_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<SlackForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    if let Err(e) = ghost_api::slack::validate_url(&form.slack_url) {
        return integrations_flash("slack/", "error", &e.message());
    }
    let url = form.slack_url.trim().to_string();
    let username = match form.slack_username.trim() {
        "" => "Ghost".to_string(),
        u => u.to_string(),
    };
    for (key, value) in [("slack_url", url), ("slack_username", username)] {
        if let Err(e) = state.settings.set(&state.db, key, &serde_json::json!(value)).await {
            return integrations_flash("slack/", "error", &e.message());
        }
    }
    integrations_flash("slack/", "ok", "Slack settings saved.")
}

async fn slack_test(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let Some(cfg) = ghost_api::slack::SlackConfig::from_settings(&state.settings) else {
        return integrations_flash("slack/", "error", "Add a webhook URL first.");
    };
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    match ghost_api::slack::send_test(&cfg, &title, &state.site_url()).await {
        Ok(()) => integrations_flash("slack/", "ok", "Test notification sent to Slack."),
        Err(e) => integrations_flash("slack/", "error", &e.message()),
    }
}

// ---------------------------------------------------------------------------
// built-in app: Unsplash
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UnsplashForm {
    #[serde(default)]
    unsplash: Option<String>,
    #[serde(default)]
    unsplash_access_key: String,
}

/// FirstPromoter: referral tracking. Ghost injects their script with the
/// account id, so the id is what makes the toggle do anything.
async fn firstpromoter_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("firstpromoter");
    let id = state.settings.get_str("firstpromoter_id").unwrap_or_default();
    page_for_role(
        actor_role,
        "FirstPromoter",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>FirstPromoter</h1><p class="sub">Referral and affiliate tracking for members</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/firstpromoter/">
{toggle}
<div style="margin-top:14px">{id_field}</div>
<p class="muted" style="margin:10px 0 0">{note}</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row(
                "firstpromoter",
                "Enable FirstPromoter",
                "Adds their tracking script to your public pages",
                on
            ),
            id_field = text_field("firstpromoter_id", "FirstPromoter account ID", &id),
            note = if id.trim().is_empty() {
                "The account ID comes from your FirstPromoter dashboard. Without it the script has nothing to report to, so the toggle does nothing."
            } else {
                "Your pages carry the FirstPromoter script under this account ID."
            },
        ),
        true,
    )
    .into_response()
}

/// Pintura: an image editor in the admin. Ghost loads it from URLs you host,
/// since the library is commercial and not bundled.
async fn pintura_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("pintura");
    let js = state.settings.get_str("pintura_js_url").unwrap_or_default();
    let css = state.settings.get_str("pintura_css_url").unwrap_or_default();
    page_for_role(
        actor_role,
        "Pintura",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Pintura</h1><p class="sub">Edit images without leaving the editor</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/pintura/">
{toggle}
<div class="grid2" style="gap:12px;margin-top:14px">{js_field}{css_field}</div>
<p class="muted" style="margin:10px 0 0">Pintura is a commercial library Ghost does not ship. Buy a licence, host the two files
yourself, and point these at them.</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row("pintura", "Enable Pintura", "Adds an edit button to image cards", on),
            js_field = text_field("pintura_js_url", "Script URL (pintura.umd.js)", &js),
            css_field = text_field("pintura_css_url", "Stylesheet URL (pintura.css)", &css),
        ),
        true,
    )
    .into_response()
}

/// Transistor.fm: private podcast feeds for members. The URL template carries
/// `{uuid}`, which Ghost swaps for the member's own so each gets a feed only
/// they can use.
async fn transistor_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("transistor");
    let g = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    page_for_role(
        actor_role,
        "Transistor.fm",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Transistor.fm</h1><p class="sub">Private podcast feeds for your members</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/transistor/">
{toggle}
<label class="field" style="margin:14px 0 0"><input name="transistor_portal_url_template" value="{url_template}" placeholder="https://partner.transistor.fm/ghost/{{uuid}}"><span>Private feed URL template</span></label>
<p class="muted" style="margin:6px 0 0">Use <code>{{uuid}}</code> where the member's own identifier goes — that is what makes each feed private to them.</p>
<div class="grid2" style="gap:12px;margin-top:14px">
<label class="field" style="margin:0"><input name="transistor_portal_heading" value="{heading}" placeholder=" "><span>Heading in the account page</span></label>
<label class="field" style="margin:0"><input name="transistor_portal_button_text" value="{button}" placeholder=" "><span>Button text</span></label>
</div>
<label class="field" style="margin:12px 0 0"><textarea name="transistor_portal_description" rows="2" placeholder=" ">{description}</textarea><span>Description</span></label>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row(
                "transistor",
                "Enable Transistor integration",
                "Offer members their private podcast feed from the account page",
                on
            ),
            url_template = g("transistor_portal_url_template"),
            heading = g("transistor_portal_heading"),
            button = g("transistor_portal_button_text"),
            description = g("transistor_portal_description"),
        ),
        true,
    )
    .into_response()
}

/// One save for all three: a toggle plus its text settings.
///
/// The app is named by the caller rather than taken from the path — these are
/// three fixed routes, so there is no segment to extract.
async fn builtin_app_save(
    state: &AppState,
    headers: &HeaderMap,
    app: &str,
    form: &HashMap<String, String>,
) -> Response {
    if let Err(r) = require_role(state, headers, Role::Administrator).await {
        return r;
    }
    let fields: &[&str] = match app {
        "firstpromoter" => &["firstpromoter_id"],
        "pintura" => &["pintura_js_url", "pintura_css_url"],
        "transistor" => &[
            "transistor_portal_url_template",
            "transistor_portal_heading",
            "transistor_portal_button_text",
            "transistor_portal_description",
        ],
        _ => return integrations_flash("", "error", "Unknown app."),
    };
    if form.get(app).is_some_and(|value| value == "1") {
        let nonempty = |key: &str| form.get(key).is_some_and(|value| !value.trim().is_empty());
        let valid_url = |key: &str| form.get(key).and_then(|value| reqwest::Url::parse(value.trim()).ok())
            .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
        let message = match app {
            "firstpromoter" if !nonempty("firstpromoter_id") => Some("Enter your FirstPromoter account ID before enabling the integration."),
            "pintura" if !valid_url("pintura_js_url") || !valid_url("pintura_css_url") => Some("Enter valid HTTP or HTTPS URLs for both Pintura assets before enabling the integration."),
            "transistor" if !valid_url("transistor_portal_url_template") || !form.get("transistor_portal_url_template").is_some_and(|value| value.contains("{uuid}")) => Some("Enter a valid private feed URL containing {uuid} before enabling Transistor."),
            _ => None,
        };
        if let Some(message) = message { return integrations_flash(&format!("{app}/"), "error", message); }
    }
    let mut ok = state
        .settings
        .set(&state.db, app, &serde_json::Value::Bool(form.get(app).map(|v| v == "1").unwrap_or(false)))
        .await
        .is_ok();
    for key in fields {
        let value = form.get(*key).cloned().unwrap_or_default().trim().to_string();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    if ok {
        integrations_flash(&format!("{app}/"), "ok", "Saved.")
    } else {
        integrations_flash(&format!("{app}/"), "error", "Saving failed — check the logs.")
    }
}

async fn firstpromoter_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "firstpromoter", &form).await
}

async fn pintura_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "pintura", &form).await
}

async fn transistor_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    builtin_app_save(&state, &headers, "transistor", &form).await
}

async fn unsplash_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let on = state.settings.get_bool("unsplash");
    let key = state.settings.get_str("unsplash_access_key").unwrap_or_default();
    let note = match key.trim().is_empty() {
        true => "Without an access key the picker shows a built-in sample gallery. Add a key from unsplash.com/developers to search the whole library.",
        false => "Searching the full Unsplash library with your access key.",
    };

    page_for_role(
        actor_role,
        "Unsplash",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/integrations/" title="Back to integrations">{back}</a>
<div><h1>Unsplash</h1><p class="sub">Free photography inside the editor</p></div></header>
{banner}
<div class="card padded rise" style="--i:1"><form method="post" action="/ghost/settings/integrations/unsplash/">
{toggle}
<div style="margin-top:14px">{key_field}</div>
<p class="muted" style="margin:10px 0 0">{note}</p>
<div style="margin-top:14px"><button class="btn" type="submit">Save</button></div></form></div>"#,
            back = svg("chevron"),
            banner = flash_banner(&q),
            toggle = toggle_row("unsplash", "Unsplash in the editor", "Adds an Unsplash button to image cards", on),
            key_field = text_field("unsplash_access_key", "Unsplash access key (optional)", &key),
            note = esc(note),
        ),
        true,
    )
    .into_response()
}

async fn unsplash_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<UnsplashForm>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let on = form.unsplash.as_deref() == Some("1");
    if let Err(e) = state.settings.set(&state.db, "unsplash", &serde_json::json!(on)).await {
        return integrations_flash("unsplash/", "error", &e.message());
    }
    let key = form.unsplash_access_key.trim().to_string();
    if let Err(e) = state
        .settings
        .set(&state.db, "unsplash_access_key", &serde_json::json!(key))
        .await
    {
        return integrations_flash("unsplash/", "error", &e.message());
    }
    integrations_flash("unsplash/", "ok", "Unsplash settings saved.")
}

#[cfg(test)]
mod tests {
    use super::{recommendation_title, recommendation_url};

    #[tokio::test]
    async fn tier_newsletters_require_a_current_matching_entitlement() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        ghost_db::seed::seed_if_empty(&pool).await.unwrap();
        let owner = ghost_db::repos::users::setup_owner(&pool, "Test owner", "owner@example.test", "unused-test-hash")
            .await.unwrap();
        sqlx::query("INSERT INTO newsletters (id, name, slug, created_at) VALUES ('newsletter', 'Test', 'test', datetime('now'))")
            .execute(&pool).await.unwrap();
        for id in ["selected", "other"] {
            sqlx::query("INSERT INTO products (id, name, slug, created_at) VALUES (?, ?, ?, datetime('now'))")
                .bind(id).bind(id).bind(id).execute(&pool).await.unwrap();
        }
        let post = ghost_db::repos::posts::insert(&pool, &ghost_db::repos::posts::NewPost {
            title: "Tier newsletter".into(), status: "published".into(),
            visibility: "tiers".into(), post_type: "post".into(),
            tier_ids: vec!["selected".into()], ..Default::default()
        }, &owner.id).await.unwrap();
        for (id, status, tier, expiry, disabled, subscribed) in [
            ("matching", "paid", "selected", None, false, true),
            ("comped", "comped", "selected", None, false, true),
            ("wrong", "paid", "other", None, false, true),
            ("expired", "paid", "selected", Some("2000-01-01"), false, true),
            ("free", "free", "selected", None, false, true),
            ("disabled", "paid", "selected", None, true, true),
            ("unsubscribed", "paid", "selected", None, false, false),
        ] {
            sqlx::query("INSERT INTO members (id, transient_id, email, status, email_disabled, created_at) VALUES (?, ?, ?, ?, ?, datetime('now'))")
                .bind(id).bind(id).bind(format!("{id}@example.test")).bind(status).bind(disabled)
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO members_products (id, member_id, product_id, expiry_at) VALUES (?, ?, ?, ?)")
                .bind(id).bind(id).bind(tier).bind(expiry).execute(&pool).await.unwrap();
            if subscribed {
                sqlx::query("INSERT INTO members_newsletters (id, member_id, newsletter_id) VALUES (?, ?, 'newsletter')")
                    .bind(id).bind(id).execute(&pool).await.unwrap();
            }
        }
        let mut emails: Vec<_> = super::newsletter_recipients(&pool, "newsletter", "tiers", &post.id)
            .await.into_iter().map(|member| member.email).collect();
        emails.sort();
        assert_eq!(emails, ["comped@example.test", "matching@example.test"]);
        assert!(super::newsletter_recipients(&pool, "newsletter", "tiers", "missing-post").await.is_empty());
    }

    #[test]
    fn module_pages_share_one_admin_only_sidebar_entry() {
        for active in ["modules", "analytics", "comments", "automations"] {
            let nav = super::rail(active, None, super::Role::Administrator);
            assert!(nav.contains(r#"class="nav-item active" href="/ghost/modules/""#));
            for href in ["analytics", "comments", "automations"] {
                assert!(!nav.contains(&format!(r#"class="nav-item" href="/ghost/{href}/""#)));
                assert!(!nav.contains(&format!(r#"class="nav-item active" href="/ghost/{href}/""#)));
            }
        }
        for role in [super::Role::Contributor, super::Role::Author, super::Role::Editor] {
            assert!(!super::rail("posts", None, role).contains("/ghost/modules/"));
        }
    }

    #[tokio::test]
    async fn admin_branding_uses_the_current_site_name_and_escapes_html() {
        let settings = std::sync::Arc::new(ghost_db::SettingsCache::from_values([
            ("title".into(), serde_json::json!("A different title")),
            ("name".into(), serde_json::json!("News <&>")),
        ]));
        super::ADMIN_SETTINGS.scope(settings, async {
            let html = super::page_u("Posts", "posts", None, "", true).0;
            assert!(html.contains("<b>News &lt;&amp;&gt;</b>"));
            assert!(html.contains("<title>Posts — News &lt;&amp;&gt;</title>"));
            assert!(!html.contains("<b>Admin</b>"));
            assert!(!html.contains("<span>Ghost</span>"));
        }).await;
    }

    #[test]
    fn stripe_connect_secure_keys_are_bound_to_the_starting_session_and_mode() {
        use base64::Engine;
        let encode = |value: serde_json::Value| base64::engine::general_purpose::STANDARD.encode(value.to_string());
        let key = encode(serde_json::json!({"a":"sk_test_fixture", "p":"pk_test_fixture", "i":"acct_fixture", "l":false, "s":"session-state", "n":"A site title"}));
        assert!(super::decode_stripe_secure_key(&key, "session-state").is_ok());
        assert!(super::decode_stripe_secure_key(&key, "another-session").is_err());
        assert!(super::decode_stripe_secure_key(&key, "").is_err());
        let wrong_mode = encode(serde_json::json!({"a":"sk_test_fixture", "p":"pk_live_fixture", "i":"acct_fixture", "l":false, "s":"session-state"}));
        assert!(super::decode_stripe_secure_key(&wrong_mode, "session-state").is_err());
        assert!(super::decode_stripe_secure_key("not-base64", "session-state").is_err());
    }

    #[test]
    fn stripe_connect_uses_the_same_provider_and_mode_as_ghost_673() {
        let test = reqwest::Url::parse(&super::stripe_oauth_url("test", "bound-state")).unwrap();
        assert_eq!(test.host_str(), Some("connect.stripe.com"));
        let params: std::collections::HashMap<_, _> = test.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], "ca_8LBum4Ctv3mmJ1oD0ZRmxjdAhNrrBUy3");
        assert_eq!(params["redirect_uri"], "https://stripe.ghost.org");
        assert_eq!(params["state"], "bound-state");
        assert_eq!(params["scope"], "read_write");
    }

    #[test]
    fn a_recommended_url_gets_a_scheme_but_must_be_a_real_host() {
        assert_eq!(recommendation_url("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(
            recommendation_url(" http://example.com/blog ").as_deref(),
            Some("http://example.com/blog")
        );
        // a host with no dot is a typo here, not a publication
        assert!(recommendation_url("localhost").is_none());
        assert!(recommendation_url("").is_none());
    }

    #[test]
    fn a_blank_title_falls_back_to_the_domain() {
        assert_eq!(recommendation_title("", "https://www.example.com/blog"), "example.com");
        assert_eq!(recommendation_title("  ", "https://example.com"), "example.com");
        assert_eq!(recommendation_title(" Real Name ", "https://example.com"), "Real Name");
    }

    use super::*;

    #[test]
    fn editor_form_collects_repeated_tag_fields() {
        // what the browser posts with three boxes ticked
        let form = EditorForm::parse(
            "title=Hello&slug=hello&markdown=Body&status=published\
             &tags=aaa111&tags=bbb222&tags=ccc333",
        );
        assert_eq!(form.tags, vec!["aaa111", "bbb222", "ccc333"]);
        assert_eq!(form.title, "Hello");
        assert_eq!(form.slug.as_deref(), Some("hello"));
    }

    #[test]
    fn editor_form_accepts_one_tag_or_none() {
        // a single ticked box is what used to fail to deserialize
        let one = EditorForm::parse("title=T&markdown=&status=draft&tags=6ad3d43d22c4d20da6000002");
        assert_eq!(one.tags, vec!["6ad3d43d22c4d20da6000002"]);

        let none = EditorForm::parse("title=T&markdown=&status=draft");
        assert!(none.tags.is_empty());
    }

    #[test]
    fn editor_form_decodes_and_blanks_empty_optionals() {
        let form = EditorForm::parse(
            "title=Caf%C3%A9+%26+Bar&slug=&markdown=A+%2B+B&status=draft\
             &feature_image=&custom_excerpt=++&featured=1",
        );
        assert_eq!(form.title, "Café & Bar");
        assert_eq!(form.markdown.as_deref(), Some("A + B"));
        // an empty or whitespace-only field is absent, not an empty string
        assert_eq!(form.slug, None);
        assert_eq!(form.feature_image, None);
        assert_eq!(form.custom_excerpt, None);
        // a ticked checkbox posts at all; an unticked one posts nothing
        assert!(form.featured);
        assert!(!form.show_title_and_feature_image);
    }

    #[test]
    fn koenig_html_is_preferred_over_rendering_the_lexical() {
        let lexical = r#"{"root":{"children":[{"type":"paragraph","version":1,"children":[{"text":"hi","type":"text","format":0,"version":1}]}],"type":"root","version":1}}"#;
        let body = format!(
            "title=T&status=draft&lexical={}&html={}",
            form_urlencoded::byte_serialize(lexical.as_bytes()).collect::<String>(),
            form_urlencoded::byte_serialize(b"<p>from the browser</p>").collect::<String>(),
        );
        let form = EditorForm::parse(&body);
        assert!(form.is_koenig());
        let (html, plaintext, _) = form.rendered();
        assert_eq!(html, "<p>from the browser</p>");
        assert_eq!(plaintext, "from the browser");
    }

    #[test]
    fn lexical_without_browser_html_is_rendered_server_side() {
        let lexical = r#"{"root":{"children":[{"type":"paragraph","version":1,"children":[{"text":"hi","type":"text","format":0,"version":1}]}],"type":"root","version":1}}"#;
        let body = format!(
            "title=T&status=draft&lexical={}",
            form_urlencoded::byte_serialize(lexical.as_bytes()).collect::<String>()
        );
        let (html, plaintext, reading_time) = EditorForm::parse(&body).rendered();
        assert_eq!(html, "<p>hi</p>");
        assert_eq!(plaintext, "hi");
        assert_eq!(reading_time, 1);
    }

    #[test]
    fn markdown_fallback_still_renders_when_koenig_did_not_run() {
        let form = EditorForm::parse("title=T&status=draft&markdown=%23+Heading");
        assert!(!form.is_koenig());
        let (html, _, _) = form.rendered();
        assert!(html.contains("<h1>Heading</h1>"), "{html}");
    }

    #[test]
    fn each_editor_may_only_clear_the_other_source_it_replaces() {
        // A Koenig save blanks any leftover markdown, and a Markdown save
        // blanks any leftover lexical — never the one it did not write.
        assert!(editor_owned_fields(true, true).contains(&"markdown"));
        assert!(!editor_owned_fields(true, true).contains(&"lexical"));
        assert!(editor_owned_fields(false, true).contains(&"lexical"));
        assert!(!editor_owned_fields(false, true).contains(&"markdown"));
    }

    #[test]
    fn a_save_carrying_no_body_leaves_the_content_alone() {
        // Every settings field posts, but neither editor did — the content
        // columns must not be cleared or re-rendered from nothing.
        let form = EditorForm::parse("title=T&status=draft&slug=t&meta_title=M");
        assert!(!form.has_content());
        for field in ["html", "plaintext", "lexical", "markdown"] {
            assert!(!editor_owned_fields(form.is_koenig(), form.has_content()).contains(&field), "{field} would be cleared");
        }
        // the settings on the same request are still applied
        assert!(editor_owned_fields(false, false).contains(&"meta_title"));
    }

    #[test]
    fn publish_dates_round_trip_through_the_datetime_input() {
        assert_eq!(to_datetime_local("2026-10-19 14:30:00"), "2026-10-19T14:30");
        assert_eq!(from_datetime_local("2026-10-19T14:30").as_deref(), Some("2026-10-19 14:30:00"));
        // an empty or half-filled input leaves the date alone
        assert_eq!(to_datetime_local(""), "");
        assert_eq!(from_datetime_local(""), None);
        assert_eq!(from_datetime_local("2026-10-19"), None);
    }

    #[test]
    fn editor_form_ignores_unknown_fields() {
        let form = EditorForm::parse("title=T&markdown=M&status=draft&csrf=xyz&_method=put");
        assert_eq!(form.title, "T");
        assert_eq!(form.status, "draft");
    }

    #[test]
    fn nav_form_pairs_labels_with_urls_in_row_order() {
        // two primary rows and one secondary row, as the browser posts them
        let form = NavForm::parse(
            "primary_label=Home&primary_url=%2F&primary_label=About&primary_url=%2Fabout%2F\
             &secondary_label=Sign+up&secondary_url=%23%2Fportal%2F&action=save",
        );
        assert_eq!(
            form.primary,
            vec![NavItem { label: "Home".into(), url: "/".into() }, NavItem { label: "About".into(), url: "/about/".into() }]
        );
        assert_eq!(
            form.secondary,
            vec![NavItem { label: "Sign up".into(), url: "#/portal/".into() }]
        );
        assert_eq!(form.action, "save");
    }

    #[test]
    fn nav_form_pads_uneven_label_url_counts() {
        // a row with a label but no matching url still shows up, url blank
        let form = NavForm::parse("primary_label=Home&primary_url=%2F&primary_label=Blog");
        assert_eq!(
            form.primary,
            vec![NavItem { label: "Home".into(), url: "/".into() }, NavItem { label: "Blog".into(), url: String::new() }]
        );
    }

    #[test]
    fn nav_row_actions_move_add_and_remove() {
        let mut form = NavForm {
            primary: vec![
                NavItem { label: "A".into(), url: "/a/".into() },
                NavItem { label: "B".into(), url: "/b/".into() },
                NavItem { label: "C".into(), url: "/c/".into() },
            ],
            ..Default::default()
        };

        // down moves B below C
        assert!(nav_apply_action(&mut form, "down-primary-1"));
        assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["A", "C", "B"]);
        // up on the first row is a no-op
        assert!(nav_apply_action(&mut form, "up-primary-0"));
        assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["A", "C", "B"]);
        // remove takes out the row at the index
        assert!(nav_apply_action(&mut form, "remove-primary-0"));
        assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["C", "B"]);
        // add appends an empty row; save is not a row action
        assert!(nav_apply_action(&mut form, "add-primary"));
        assert_eq!(form.primary.last(), Some(&NavItem::default()));
        assert!(!nav_apply_action(&mut form, "save"));
        // out-of-range and unknown actions re-render without mutating
        assert!(nav_apply_action(&mut form, "remove-primary-99"));
        assert!(nav_apply_action(&mut form, "teleport-primary-0"));
        assert_eq!(form.primary.len(), 3);
    }

    #[test]
    fn publish_actions_map_to_status_and_sending() {
        use PublishAction::*;
        // "Publish only" leaves the requested status alone and mails nobody
        assert_eq!(PublishAction::parse("publish_only"), PublishOnly);
        assert!(!PublishOnly.sends_email());
        assert_eq!(PublishOnly.status_for("published"), "published");

        // "Publish and email" publishes *and* sends
        assert_eq!(PublishAction::parse("publish_email"), PublishAndEmail);
        assert!(PublishAndEmail.sends_email());
        assert_eq!(PublishAndEmail.status_for("published"), "published");

        // "Email only" overrides the status to `sent`, so the post goes to
        // subscribers without appearing in the site's listings
        assert_eq!(PublishAction::parse("email_only"), EmailOnly);
        assert!(EmailOnly.sends_email());
        assert_eq!(EmailOnly.status_for("published"), "sent");
        assert_eq!(EmailOnly.status_for("draft"), "sent");

        // anything unrecognised must not send — failing closed matters when
        // the alternative is mailing every subscriber by accident
        assert_eq!(PublishAction::parse(""), PublishOnly);
        assert_eq!(PublishAction::parse("nonsense"), PublishOnly);
    }

    /// A scheduled save is the one case where the decision is stored instead
    /// of acted on, so the storing has to be right: a bad date must never
    /// become "now", and an email intent must survive to the delivery row.
    #[test]
    fn scheduling_reads_the_date_and_the_delivery() {
        // The comparison is textual, so fixed dates either side of any
        // plausible "now" make this independent of the clock.
        const FUTURE: &str = "2099-01-01 09:00:00";
        const PAST: &str = "2000-01-01 09:00:00";
        let form = |status: &str, when: &str, action: &str| {
            let mut f = EditorForm::parse("title=T&markdown=body");
            f.status = status.into();
            f.published_at = (!when.is_empty()).then(|| when.to_string());
            f.publish_action = action.into();
            f
        };

        // not scheduled: nothing to arrange, whatever the date says
        let f = form("published", FUTURE, "publish_email");
        assert!(matches!(scheduling(&f, "post", PublishAction::PublishAndEmail), Scheduling::No));

        // scheduled into the future, with and without an email
        let f = form("scheduled", FUTURE, "publish_only");
        let Scheduling::At(d) = scheduling(&f, "post", PublishAction::PublishOnly) else {
            panic!("a future date is a schedule");
        };
        assert!(!d.email && !d.email_only);
        assert_eq!(d.status(), "published");

        let f = form("scheduled", FUTURE, "publish_email");
        let Scheduling::At(d) = scheduling(&f, "post", PublishAction::PublishAndEmail) else {
            panic!("a future date is a schedule");
        };
        assert!(d.email && !d.email_only);
        assert_eq!(d.status(), "published");

        let f = form("scheduled", FUTURE, "email_only");
        let Scheduling::At(d) = scheduling(&f, "post", PublishAction::EmailOnly) else {
            panic!("a future date is a schedule");
        };
        assert!(d.email && d.email_only);
        // email-only posts land in Ghost's `sent`, off the site's listings
        assert_eq!(d.status(), "sent");

        // a page has no newsletter, so the choice cannot make it send one
        let f = form("scheduled", FUTURE, "publish_email");
        let Scheduling::At(d) = scheduling(&f, "page", PublishAction::PublishAndEmail) else {
            panic!("a page can be scheduled too");
        };
        assert!(!d.email);

        // a date in the past, or none at all, is not a schedule — and must
        // not fall through to publishing immediately
        for when in [PAST, ""] {
            let f = form("scheduled", &when, "publish_email");
            assert!(
                matches!(scheduling(&f, "post", PublishAction::PublishAndEmail), Scheduling::Undated),
                "{when:?} was accepted as a schedule"
            );
        }
    }

    #[test]
    fn editor_form_defaults_to_not_sending() {
        // a plain draft save posts no publish_action at all
        let f = EditorForm::parse("title=T&markdown=body&status=draft");
        assert_eq!(PublishAction::parse(&f.publish_action), PublishAction::PublishOnly);
    }

    #[test]
    fn roles_match_ghosts_permission_fixtures() {
        use Role::*;
        // Ghost grants no `member`, `integration`, `api_key` or `db`
        // permission below Administrator, and `setting` is browse/read only
        // for Editor and under — so settings, members, themes and
        // integrations are admin territory.
        for r in [Contributor, Author, Editor] {
            assert!(!r.is_admin(), "{r:?} must not reach admin-only screens");
        }
        for r in [Administrator, Owner] {
            assert!(r.is_admin());
        }

        // tag = all for Editor and above; Authors may only add while writing
        assert!(!Contributor.can_manage_tags());
        assert!(!Author.can_manage_tags());
        assert!(Editor.can_manage_tags());

        // post = all for Editor; Authors and Contributors are scoped to their own
        assert!(!Contributor.can_edit_any_post());
        assert!(!Author.can_edit_any_post());
        assert!(Editor.can_edit_any_post());

        // publishing is the line Ghost draws for Contributors
        assert!(!Contributor.can_publish());
        assert!(Author.can_publish());

        // Editors manage Authors and Contributors
        assert!(!Author.can_manage_staff());
        assert!(Editor.can_manage_staff());
    }

    #[test]
    fn an_unknown_role_gets_the_least_privilege() {
        // a renamed or missing role must never be read as trusted
        assert_eq!(Role::from_name(""), Role::Contributor);
        assert_eq!(Role::from_name("Nonsense"), Role::Contributor);
        assert_eq!(Role::from_name("Admin Integration"), Role::Contributor);
        // and the real names still map
        assert_eq!(Role::from_name("Owner"), Role::Owner);
        assert_eq!(Role::from_name("Administrator"), Role::Administrator);
        assert_eq!(Role::from_name("Editor"), Role::Editor);
        assert_eq!(Role::from_name("Author"), Role::Author);
        assert_eq!(Role::from_name("Contributor"), Role::Contributor);
    }

    #[test]
    fn roles_are_ordered_by_privilege() {
        use Role::*;
        assert!(Owner > Administrator && Administrator > Editor && Editor > Author && Author > Contributor);
    }

    #[test]
    fn member_csv_columns_match_ghosts() {
        // Ghost's @tryghost/members-csv DEFAULT_COLUMNS, in order. A file
        // exported here has to import into Ghost unchanged, so drift in this
        // list silently breaks interoperability.
        assert_eq!(
            MEMBER_CSV_COLUMNS,
            [
                "id", "email", "name", "note", "subscribed_to_emails", "complimentary_plan",
                "stripe_customer_id", "created_at", "deleted_at", "labels", "tiers",
            ]
        );
        // email is what rows are matched on, so it must always be present
        assert!(MEMBER_CSV_COLUMNS.contains(&"email"));
    }

    #[test]
    fn member_edit_form_collects_repeated_newsletter_boxes() {
        // every ticked newsletter posts under the same name, which a typed
        // Form extractor would collapse to the last one
        let f = MemberEditForm::parse(
            "name=Ada&email=ada%40example.com&note=VIP&labels=a%2C+b&status=comped\
             &newsletters=n1&newsletters=n2&newsletters=n3",
        );
        assert_eq!(f.newsletters, vec!["n1", "n2", "n3"]);
        assert_eq!(f.email, "ada@example.com");
        assert_eq!(f.status, "comped");
        assert_eq!(f.labels, "a, b");
    }

    #[test]
    fn member_edit_form_with_no_newsletters_means_unsubscribed() {
        // the absence of the field is the signal, so it must parse as empty
        // rather than being treated as "unchanged"
        let f = MemberEditForm::parse("name=Ada&email=ada%40example.com&status=free");
        assert!(f.newsletters.is_empty());
    }

    #[test]
    fn nav_saves_when_no_action_was_posted() {
        // A submit button contributes its name/value only when it is the
        // element that activated the form, so the toolbar Save button and
        // Enter-in-a-field both post with no `action` at all. Treating that
        // as an unknown row action silently discarded the whole edit.
        let mut form = NavForm {
            primary: vec![NavItem { label: "Home".into(), url: "/".into() }],
            ..Default::default()
        };
        assert!(!nav_apply_action(&mut form, ""), "an absent action must save, not re-render");
        assert_eq!(form.primary.len(), 1, "and must not mutate the rows on the way");
    }

    #[test]
    fn nav_save_button_posts_an_explicit_action() {
        // belt and braces for the above: the rendered toolbar button carries
        // the field, so the save path does not depend on the empty-string
        // fallback alone
        let html = navigation_html(&[], &[], "");
        assert!(
            html.contains(r#"type="submit" form="nav-form" name="action" value="save""#),
            "toolbar Save must submit action=save"
        );
    }

    #[test]
    fn nav_round_trips_a_posted_form() {
        // the exact body the browser sends when Save is clicked
        let form = NavForm::parse(
            "primary_label=Home&primary_url=%2F\
             &primary_label=About&primary_url=%2Fabout%2F\
             &secondary_label=Privacy&secondary_url=%2Fprivacy%2F&action=save",
        );
        assert_eq!(form.action, "save");
        let primary = nav_clean(&form.primary);
        assert_eq!(primary.len(), 2);
        assert_eq!(primary[1]["label"], serde_json::json!("About"));
        assert_eq!(primary[1]["url"], serde_json::json!("/about/"));
        assert_eq!(nav_clean(&form.secondary).len(), 1);
    }

    #[test]
    fn nav_clean_drops_blank_rows_and_defaults_url() {
        let items = vec![
            NavItem { label: "Home".into(), url: "".into() },
            NavItem { label: "  ".into(), url: "/gone/".into() }, // blank label → deleted
            NavItem { label: " About ".into(), url: " /about/ ".into() },
        ];
        assert_eq!(
            nav_clean(&items),
            vec![
                serde_json::json!({"label": "Home", "url": "/"}),
                serde_json::json!({"label": "About", "url": "/about/"})
            ]
        );
    }

    #[test]
    fn nav_items_read_back_from_array_or_json_string() {
        // the settings cache stores arrays; the Admin API may store the same
        // value as a JSON-encoded string
        let arr = nav_items_from_setting(Some(serde_json::json!([{"label": "Home", "url": "/"}])));
        let s = nav_items_from_setting(Some(serde_json::json!("[{\"label\":\"Home\",\"url\":\"/\"}]")));
        assert_eq!(arr, vec![NavItem { label: "Home".into(), url: "/".into() }]);
        assert_eq!(arr, s);
        assert!(nav_items_from_setting(None).is_empty());
    }

    #[test]
    fn money_minor_groups_and_drops_whole_cents() {
        // the subscriptions.mrr column is in minor units, like Stripe's
        assert_eq!(money_minor(0), "0");
        assert_eq!(money_minor(500), "5");
        assert_eq!(money_minor(1250), "12.50");
        assert_eq!(money_minor(123456), "1,234.56");
        assert_eq!(money_minor(100000000), "1,000,000");
        assert_eq!(money_minor(-2550), "-25.50");
    }

    #[test]
    fn countable_agrees_with_its_noun() {
        assert_eq!(countable(1, "visitor"), "1 visitor");
        assert_eq!(countable(0, "visitor"), "0 visitors");
        assert_eq!(countable(2, "view"), "2 views");
        assert_eq!(countable(2500, "member"), "2.5K members");
    }

    #[test]
    fn audience_clause_only_ever_emits_known_segments() {
        // the value reaches here from the query string, so it must never be
        // interpolated — it is matched against a fixed set instead
        assert_eq!(audience_clause("paid"), " AND audience = 'paid'");
        assert_eq!(audience_clause("all"), "");
        assert_eq!(audience_clause("'; DROP TABLE page_views;--"), "");
    }

    #[test]
    fn analytics_range_falls_back_to_thirty_days() {
        let any = |v: &str| {
            let mut q = HashMap::new();
            q.insert("range".to_string(), v.to_string());
            AnalyticsRange::from_query(&q)
        };
        assert_eq!(any("7").cutoff, "-7 days");
        assert_eq!(any("today").cutoff, "start of day");
        assert_eq!(any("all").days, None);
        // an unknown range is the default rather than an error page
        assert_eq!(any("bogus").slug, "30");
        assert_eq!(AnalyticsRange::from_query(&HashMap::new()).slug, "30");
    }

    #[test]
    fn settings_groups_match_ghosts_navigation() {
        // Ghost's settings sidebar, in order. Email and Bulk email sit after
        // Membership: Ghost keeps both in config.json, this port exposes them
        // in the UI, and they belong next to the membership features that
        // send the mail. Features is this port's own group — the Comments
        // and Automations system switches have no Ghost page to mirror.
        //
        // Design & branding and Navigation moved here out of the left rail,
        // which is also where Ghost keeps them; Announcement is what is left
        // of the old Site group once its Theme row — a second door onto
        // Design — was taken away. Integrations and Code injection are
        // Ghost's own settings pages too; they lived under Advanced here
        // until each grew a Save button of its own.
        //
        // Meta data and Social accounts are no longer groups: they are
        // sections of General settings, beside the title and description
        // every one of their fields falls back to. Migration tools is a group
        // in its place, holding the import and export that were under
        // Advanced — which is Ghost's own arrangement.
        let titles: Vec<&str> = SETTINGS_GROUPS.iter().map(|g| g.title).collect();
        assert_eq!(
            titles,
            ["General settings", "Staff", "Analytics",
             "Design & branding", "Navigation", "Code injection", "Announcement",
             "Membership", "Email", "Newsletters", "Growth", "Features", "Integrations",
             "Migration tools", "Advanced", "History", "Danger zone"]
        );
        // every group has a working page at its slug
        assert!(SETTINGS_GROUPS.iter().all(|g| !g.slug.is_empty() && !g.items.is_empty()));
        // the two groups that keep their own top-level page are the only ones
        // that do not live under /ghost/settings/
        for g in SETTINGS_GROUPS {
            let expected_top_level = matches!(g.slug, "design" | "navigation");
            assert_eq!(
                !g.path().starts_with("/ghost/settings/"),
                expected_top_level,
                "{} points at {}",
                g.title,
                g.path()
            );
        }
    }

    #[test]
    fn every_searchable_setting_lands_on_a_real_page() {
        // a typo'd slug in the index would render nowhere — the row is
        // silently dropped — so the index is checked against the groups
        for (slug, section, field) in SETTINGS_INDEX {
            assert!(
                SETTINGS_GROUPS.iter().any(|g| g.slug == *slug),
                "SETTINGS_INDEX points at unknown settings page: {slug}"
            );
            assert!(!section.is_empty() && !field.is_empty(), "empty section or field for {slug}");
        }
        for (_, name, parent) in SETTINGS_PLACES {
            assert!(
                SETTINGS_GROUPS.iter().any(|g| g.slug == *parent),
                "SETTINGS_PLACES points at unknown settings page: {parent} ({name})"
            );
        }
    }

    #[test]
    fn announcement_visibility_reads_array_or_json_string() {
        let arr = announcement_visibility_set(Some(serde_json::json!(["visitors", "paid_members"])));
        assert_eq!(arr, vec!["visitors", "paid_members"]);
        let s = announcement_visibility_set(Some(serde_json::json!("[\"free_members\"]")));
        assert_eq!(s, vec!["free_members"]);
        assert!(announcement_visibility_set(None).is_empty());
        assert!(announcement_visibility_set(Some(serde_json::json!("not json"))).is_empty());
    }

    #[test]
    fn ghost_export_has_ghosts_shape() {
        let post = ghost_core::models::Post {
            id: "p1".into(),
            title: Some("Hello".into()),
            slug: Some("hello".into()),
            markdown: Some("Body".into()),
            post_type: "post".into(),
            status: "published".into(),
            ..Default::default()
        };
        let tag = ghost_core::models::Tag {
            id: "t1".into(),
            name: Some("News".into()),
            ..Default::default()
        };
        let export = build_ghost_export(std::slice::from_ref(&post), std::slice::from_ref(&tag), "http://x/");
        let entry = &export["db"][0];
        assert!(entry["meta"]["exported_on"].is_u64());
        assert_eq!(entry["data"]["posts"][0]["slug"], "hello");
        assert_eq!(entry["data"]["tags"][0]["name"], "News");
    }

    #[test]
    fn import_counts_message_pluralises() {
        let one = ImportCounts { posts: 1, pages: 1, tags: 1 };
        assert_eq!(one.message(), "Imported 1 post, 1 tag and 1 page.");
        let many = ImportCounts { posts: 3, pages: 2, tags: 4 };
        assert_eq!(many.message(), "Imported 3 posts, 4 tags and 2 pages.");
    }

    #[test]
    fn tier_prices_collapse_whole_dollars() {
        assert_eq!(tier_price(None), "—");
        assert_eq!(tier_price(Some(500)), "$5");
        assert_eq!(tier_price(Some(499)), "$4.99");
    }

    #[test]
    fn pickers_contain_the_seeded_defaults() {
        // a fresh site stores Etc/UTC / en — both must be selectable
        assert!(TIMEZONES.iter().any(|(v, _)| *v == "Etc/UTC"));
        assert!(LOCALES.iter().any(|(v, _)| *v == "en"));
    }

    #[test]
    fn select_options_mark_current_and_admit_unknown_values() {
        let list = [("a", "Alpha"), ("b", "Beta")];
        let opts = select_options(&list, "a");
        assert!(opts.contains(r#"<option value="a" selected>Alpha</option>"#));
        assert!(!opts.contains("selected>Beta"));

        // a stored value outside the list still shows up as a raw option
        let opts = select_options(&list, "zz");
        assert!(opts.starts_with(r#"<option value="zz" selected>zz</option>"#));
        assert!(opts.contains(r#"<option value="a">Alpha</option>"#));
    }

    #[test]
    fn compact_numbers_and_clean_axes() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1284), "1.3K");
        assert_eq!(compact(2_400_000), "2.4M");
        assert_eq!(clean_max(1), 4);
        assert_eq!(clean_max(6), 8);
        assert_eq!(clean_max(70), 80);
        assert_eq!(clean_max(100), 100);
        assert_eq!(clean_max(1234), 2000);
    }

    #[test]
    fn views_chart_draws_bars_grid_and_a_max_label() {
        let days = vec![
            DayViews { day: "2026-10-16".into(), views: 4 },
            DayViews { day: "2026-10-17".into(), views: 0 },
            DayViews { day: "2026-10-18".into(), views: 12 },
        ];
        let svg = views_chart(&days);
        assert!(svg.contains("aria-label=\"Views per day\""));
        assert!(svg.contains("2026-10-18: 12 views"), "every bar carries a tooltip");
        assert!(svg.contains(">12</text>"), "the max value is labeled");
        assert_eq!(svg.matches("class=\"bar\"").count(), 2, "zero days draw no bar");
        assert!(svg.contains(">10-17</text>"), "day labels are MM-DD");

        // empty window renders a note, not a broken chart
        assert!(views_chart(&[]).contains("No views recorded yet"));
    }

    #[test]
    fn member_uuids_are_v4_shaped() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
        assert_eq!(&id[14..15], "4", "version nibble");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "variant nibble");
    }

    #[test]
    fn custom_values_coerce_by_declared_type() {
        // a theme declaring one field of each supported type
        let decls: serde_json::Map<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
            "site_background_color": {"type": "color", "default": "#ffffff"},
            "sticky_header": {"type": "boolean", "default": true},
            "navigation_layout": {"type": "select", "options": ["A", "B"], "default": "A"},
            "hero_title": {"type": "text", "default": "Hi"}
        }))
        .unwrap();

        // booleans absent from the post (unticked) and selects off-list fall
        // back; colors/texts take the posted strings
        let mut form = HashMap::new();
        form.insert("site_background_color".into(), "#123456".into());
        form.insert("navigation_layout".into(), "NOT-A-CHOICE".into());
        form.insert("hero_title".into(), "Hello".into());
        let values = coerce_custom_values(&decls, &form);
        assert_eq!(values["site_background_color"], serde_json::json!("#123456"));
        assert_eq!(values["sticky_header"], serde_json::json!(false));
        assert_eq!(values["navigation_layout"], serde_json::json!("A"));
        assert_eq!(values["hero_title"], serde_json::json!("Hello"));
        assert!(!values.contains_key("not_declared"));

        // a ticked boolean posts "1" alongside its hidden companion marker
        let mut form = HashMap::new();
        form.insert("_sticky_header".into(), "1".into());
        form.insert("sticky_header".into(), "1".into());
        let values = coerce_custom_values(&decls, &form);
        assert_eq!(values["sticky_header"], serde_json::json!(true));
    }

    #[test]
    fn portal_links_carry_both_spellings_of_each_destination() {
        let tiers = vec![("tier1".to_string(), "Gold".to_string())];
        let html = portal_links_card("https://site.test/", &tiers, true, true);

        // The link and the attribute ride on the same row, because the switch
        // between them happens in the browser with no round trip.
        assert!(html.contains(r#"data-link="https://site.test/#/portal/signup""#));
        assert!(html.contains(r#"data-attr="data-portal=&quot;signup&quot;""#));
        // The bare default has no value on the attribute at all.
        assert!(html.contains(r#"data-link="https://site.test/#/portal" data-attr="data-portal""#));
        // A trailing slash on the site URL must not double up.
        assert!(!html.contains("site.test//#"));
        // Tier rows keep the placeholder; the picker fills it client-side.
        assert!(html.contains("#/portal/signup/{tier}/monthly"));
        assert!(html.contains(r#"<option value="tier1" selected>Gold</option>"#));
    }

    #[test]
    fn portal_links_hide_what_the_site_cannot_do() {
        // No Stripe: no paid signup, no gifts, no tips, no plan switching —
        // Portal refuses to open those, so offering the links would mislead.
        let html = portal_links_card("https://site.test", &[], false, false);
        for absent in [
            "signup/monthly",
            "gift",
            "support",
            "account/plans",
            "recommendations",
        ] {
            let row = format!(r#"data-link="https://site.test/#/portal/{absent}""#);
            assert!(!html.contains(&row), "{absent} should be hidden");
        }
        assert!(!html.contains("<h3>Tiers</h3>"));
        // What survives is what a free members site really answers.
        assert!(html.contains("#/portal/signup/free"));
        assert!(html.contains("#/portal/account/newsletters"));
    }

    #[test]
    fn the_tier_block_needs_a_tier_to_name() {
        // `#/portal/signup//monthly` opens nothing, so with Stripe connected
        // but no paid tier priced yet the whole group stays away.
        let html = portal_links_card("https://site.test", &[], true, false);
        assert!(!html.contains("<h3>Tiers</h3>"));
        assert!(html.contains(r#"data-link="https://site.test/#/portal/gift""#));
    }

    #[test]
    fn the_file_tree_mirrors_the_paths_it_was_built_from() {
        let files: Vec<String> = [
            "assets/css/screen.css",
            "assets/screenshot.jpg",
            "default.hbs",
            "partials/icons/arrow.hbs",
            "partials/card.hbs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let tree = FileTree::build(&files);

        // Directories nest; files sit in the directory that owns them.
        let names: Vec<&str> = tree.dirs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["assets", "partials"]);
        assert_eq!(tree.files, vec![("default.hbs".to_string(), "default.hbs".to_string())]);

        let assets = &tree.dirs[0].1;
        assert_eq!(assets.dirs.len(), 1, "assets holds css");
        assert_eq!(
            assets.files,
            vec![("assets/screenshot.jpg".to_string(), "screenshot.jpg".to_string())],
            "a leaf keeps its full path for the link and its bare name for the label"
        );
        assert_eq!(
            assets.dirs[0].1.files,
            vec![("assets/css/screen.css".to_string(), "screen.css".to_string())]
        );
    }

    #[test]
    fn only_the_open_files_folders_start_expanded() {
        let files: Vec<String> =
            ["assets/css/screen.css", "partials/card.hbs"].iter().map(|s| s.to_string()).collect();
        let html = render_file_tree(&FileTree::build(&files), "casper", "assets/css/screen.css", "", 0);

        // Both directories on the path to the open file are open; the
        // unrelated one is not, or a deep assets tree buries the templates.
        assert!(html.contains(r#"<details class="fdir" open><summary style="--d:0"><span class="tw">"#));
        assert_eq!(html.matches("<details class=\"fdir\" open>").count(), 2);
        assert_eq!(html.matches("<details class=\"fdir\">").count(), 1);
        // The open file is the marked row, and links by full path.
        assert!(html.contains(r#"class="frow active""#));
        assert!(html.contains("file=assets%2Fcss%2Fscreen.css"));
    }

    #[test]
    fn a_binary_file_is_listed_but_marked() {
        let files: Vec<String> = vec!["logo.png".to_string(), "index.hbs".to_string()];
        let html = render_file_tree(&FileTree::build(&files), "casper", "index.hbs", "", 0);
        assert!(html.contains("logo.png"), "binaries are still listed");
        assert_eq!(html.matches(r#"<span class="chip draft">binary</span>"#).count(), 1);
    }

    #[test]
    fn the_export_bundle_offers_ghosts_components() {
        // Media is the one that defaults off: it is usually the bulk of the
        // archive, and a content move rarely wants to wait for it.
        let keys: Vec<&str> = EXPORT_COMPONENTS.iter().map(|(k, _, _, _)| *k).collect();
        assert_eq!(keys, ["content", "members", "analytics", "themes", "routes", "media"]);
        let off: Vec<&str> = EXPORT_COMPONENTS
            .iter()
            .filter(|(_, _, _, on)| !on)
            .map(|(k, _, _, _)| *k)
            .collect();
        assert_eq!(off, ["media"]);
    }

    #[test]
    fn post_analytics_columns_only_promise_what_is_measured() {
        // Ghost drops a column rather than filling it with zeroes when the
        // site does not track it; an empty `clicks` would read as "nobody
        // clicked", which is a different and wrong claim.
        for absent in ["clicks", "signups", "paid_conversions"] {
            assert!(
                !POST_ANALYTICS_COLUMNS.contains(&absent),
                "{absent} has no source in this schema and must not be a column"
            );
        }
        for present in ["id", "title", "url", "sends", "opens", "views", "feedback_more_like_this"] {
            assert!(POST_ANALYTICS_COLUMNS.contains(&present), "{present} is missing");
        }
    }

    #[test]
    fn archive_paths_cannot_escape_the_content_directory() {
        // An uploaded zip names its own entries, so this is the check that
        // keeps a crafted archive from writing outside content/.
        assert_eq!(after_segment("content/images/2026/01/a.png", "content/images/"),
                   Some("2026/01/a.png".to_string()));
        // Nested one level down, as Ghost's own export is.
        assert_eq!(after_segment("backup/content/images/a.png", "content/images/"),
                   Some("a.png".to_string()));
        for bad in [
            "content/images/../../../etc/passwd",
            "content/images/..",
            "content/images/a/../../b",
            "content/images/",
        ] {
            assert_eq!(after_segment(bad, "content/images/"), None, "{bad} must be refused");
        }
        // A directory that merely ends in the same letters is not a match.
        assert_eq!(after_segment("mycontent/images/a.png", "content/images/"), None);
    }

    #[test]
    fn metadata_and_social_keys_are_all_saved_by_general() {
        // The merged page owns these; a key here that general_save does not
        // write is a field that silently resets on every save.
        assert!(METADATA_KEYS.contains(&"meta_title"));
        assert!(METADATA_KEYS.contains(&"og_image"));
        assert!(METADATA_KEYS.contains(&"twitter_description"));
        assert_eq!(METADATA_KEYS.len(), 8);
        // Social keys come from the same table the index and the form use.
        assert!(SOCIAL_ACCOUNTS.iter().any(|(k, _)| *k == "bluesky"));
    }

    #[test]
    fn no_ga_flag_is_offered_as_a_switch() {
        // Ghost forces its GA flags true and drops them from the writable
        // allowlist, so a toggle for one would be a control that does
        // nothing — and that reads as a broken setting, not a no-op.
        let offered: Vec<&str> = BETA_FLAGS
            .iter()
            .chain(PRIVATE_FLAGS.iter())
            .map(|(k, _, _)| *k)
            .collect();
        for ga in ghost_core::LABS_GA_FLAGS {
            assert!(!offered.contains(ga), "{ga} went GA and must not be a switch");
        }
    }

    #[test]
    fn labs_flags_match_ghost_6_73() {
        // Ghost 6.73's PUBLIC_BETA_FEATURES, in its order.
        let beta: Vec<&str> = BETA_FLAGS.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(
            beta,
            ["superEditors", "editorExcerpt", "additionalPaymentMethods", "navigationIcons"]
        );
        // `automations` moved out of the beta tab and into the private one.
        assert!(PRIVATE_FLAGS.iter().any(|(k, _, _)| *k == "automations"));
        assert!(!beta.contains(&"automations"));
        // No flag is offered twice across the two tabs.
        let mut all: Vec<&str> =
            BETA_FLAGS.iter().chain(PRIVATE_FLAGS.iter()).map(|(k, _, _)| *k).collect();
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "a flag is listed in both tabs");
    }

    fn embed(layout: &str, labels: &[&str]) -> String {
        signup_form_snippet(SignupFormEmbed {
            site_url: "https://site.test",
            layout,
            background: "#ffffff",
            button: "#15171A",
            title: "My Site",
            description: "A description",
            icon: "/content/images/2026/01/icon.png",
            locale: "en",
            labels,
            preview: false,
        })
    }

    #[test]
    fn the_signup_embed_loads_the_cdn_script_not_a_local_path() {
        // The whole point: the snippet used to point at
        // `{site}/public/signup-form.min.js`, which this server does not
        // serve, so every embed handed out was a 404.
        let html = embed("all-in-one", &[]);
        assert!(html.contains(&ghost_render::apps::signup_form_script_url()));
        assert!(!html.contains("/public/signup-form"));
        // The script has to be in a sized box or the form renders into
        // nothing, and `async` is how Ghost loads it.
        assert!(html.starts_with(r#"<div style="height: 40vmin;min-height: 360px">"#));
        assert!(html.contains(" async></script></div>"));
    }

    #[test]
    fn the_embed_sends_both_computed_contrast_colours() {
        // Without these the copy is drawn dark on a dark background.
        let html = embed("all-in-one", &[]);
        assert!(html.contains(r##"data-background-color="#ffffff""##));
        assert!(html.contains(r##"data-text-color="#000000""##), "white needs dark text");
        assert!(html.contains(r##"data-button-text-color="#FFFFFF""##), "a dark button needs light text");
    }

    #[test]
    fn the_minimal_layout_drops_the_copy_and_the_background() {
        // Minimal is meant to sit on whatever is behind it, so Ghost sends
        // no background at all — and no title, description or icon.
        let html = embed("minimal", &["a", "b"]);
        for absent in ["data-background-color", "data-text-color", "data-title", "data-description", "data-icon"] {
            assert!(!html.contains(absent), "{absent} does not belong on the minimal layout");
        }
        assert!(html.contains(r##"data-button-color="#15171A""##));
        assert!(html.contains(r#"data-label-1="a""#) && html.contains(r#"data-label-2="b""#));
        assert!(html.contains(r#"data-site="https://site.test/""#), "the site URL carries its trailing slash");
    }

    #[test]
    fn embed_attributes_follow_ghosts_order() {
        // Ghost sorts by a fixed preferred order, so a snippet from here and
        // one from Ghost diff cleanly rather than line-by-line.
        let html = embed("all-in-one", &["x"]);
        let order = [
            "data-background-color", "data-text-color", "data-button-color",
            "data-button-text-color", "data-title", "data-description", "data-icon",
            "data-site", "data-locale", "data-label-1",
        ];
        let mut last = 0;
        for key in order {
            let at = html.find(key).unwrap_or_else(|| panic!("{key} missing"));
            assert!(at > last, "{key} is out of order");
            last = at;
        }
    }

    #[test]
    fn the_embed_icon_is_asked_for_at_192_square() {
        assert_eq!(
            icon_at_192("https://s.test/content/images/2026/01/icon.png"),
            "https://s.test/content/images/size/w192h192/2026/01/icon.png"
        );
        // Any size already on the URL is replaced, not stacked.
        assert_eq!(
            icon_at_192("https://s.test/content/images/size/w256h256/2026/01/icon.png"),
            "https://s.test/content/images/size/w192h192/2026/01/icon.png"
        );
        // A URL that is not a content image is left exactly as it is.
        assert_eq!(icon_at_192("https://cdn.test/logo.svg"), "https://cdn.test/logo.svg");
    }

    #[test]
    fn the_preview_embed_fills_its_frame() {
        // Same attributes, different box: the preview is the real snippet
        // sized to the frame rather than to a page.
        let page = embed("all-in-one", &[]);
        let preview = signup_form_snippet(SignupFormEmbed {
            site_url: "https://site.test", layout: "all-in-one", background: "#ffffff",
            button: "#15171A", title: "My Site", description: "A description",
            icon: "/content/images/2026/01/icon.png", locale: "en", labels: &[], preview: true,
        });
        assert!(preview.contains(r#"style="height: 100vh""#));
        assert_ne!(page, preview);
        // The data attributes are identical — only the wrapper differs.
        let strip = |s: &str| s[s.find("<script").unwrap()..].to_string();
        assert_eq!(strip(&page), strip(&preview));
    }
}

async fn forgot_password_page() -> Response {
    page_u("Reset password", "", None, r#"<div class="auth-box"><h1>Reset password</h1><div class="card padded"><form id="password-recovery"><div id="recovery-fields"></div><button class="btn" type="submit">Continue</button><p role="status" id="recovery-message"></p></form><a href="/ghost/signin/">Back to sign in</a></div></div>"#,false).into_response()
}

async fn social_page(State(state):State<AppState>,headers:HeaderMap)->Response{
    let actor=match require_role(&state,&headers,Role::Editor).await{Ok(actor)=>actor,Err(response)=>return response};
    page_for_role(actor.role,"Social web","modules",Some(&actor.user),r#"<header class="page-head"><div><h1>Social web</h1><p class="sub">Your publication on the fediverse</p></div></header><div id="native-social" class="card padded"><p role="status">Loading your social web…</p></div><script src="/ghost/assets/1.8.14/native-social.js" defer></script>"#,true).into_response()
}

async fn machine_payments_page(State(state):State<AppState>,headers:HeaderMap)->Response{
    let actor=match require_role(&state,&headers,Role::Administrator).await{Ok(actor)=>actor,Err(response)=>return response};
    page_for_role(actor.role,"Machine payments","modules",Some(&actor.user),r#"<header class="page-head"><div><h1>Machine payments</h1><p class="sub">Paid access to individual articles for AI agents</p></div></header><div class="card padded" id="machine-payments"><p role="status">Loading payment settings…</p></div><script src="/ghost/assets/1.8.14/machine-payments.js" defer></script>"#,true).into_response()
}

async fn checkout_design_page(State(state):State<AppState>,headers:HeaderMap)->Response {
    let actor=match require_role(&state,&headers,Role::Administrator).await {Ok(actor)=>actor,Err(response)=>return response};
    page_for_role(actor.role,"Checkout design","settings",Some(&actor.user),r#"<header class="page-head"><div><h1>Checkout design</h1><p class="sub">Style the payment page for paid memberships</p></div></header><div class="card padded" id="checkout-design"><p role="status">Loading checkout design…</p></div><script src="/ghost/assets/1.8.14/checkout-design.js" defer></script>"#,true).into_response()
}
