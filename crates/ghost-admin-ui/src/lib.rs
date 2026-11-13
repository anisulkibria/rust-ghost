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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ghost_api::AppState;

pub mod analytics;
mod click;
pub mod automations;
pub mod comments;
pub mod forms;
pub mod posts;
pub mod social;
pub mod rest;
pub mod schedule;
mod preview;

// The crate's public surface, unchanged by the split: every one of these was
// an item of this file before the modules above were carved out of it.
pub use auth::index;
pub use roles::{Actor, Role};
pub use saving::{retry_email, send_newsletter_for_test};
pub use settings::email::SECRET_SETTINGS;
use analytics::*;
use social::*;
mod apps;
use apps::*;
mod assets;
use assets::*;
mod auth;
use auth::*;
mod design;
use design::*;
mod editor;
use editor::*;
mod integrations;
use integrations::*;
mod media;
use media::*;
mod members;
use members::*;
mod modules;
use modules::*;
mod navigation;
use navigation::*;
mod roles;
use roles::*;
mod saving;
use saving::*;
mod settings;
use settings::*;
mod shell;
use shell::*;
mod staff;
use staff::*;
mod tags;
use tags::*;
mod themes;
use themes::*;

/// Upload ceiling for theme zips and content imports. Ghost's own limit is
/// also in this range; it exists to bound memory, since multipart bodies are
/// buffered before being written out.
const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
/// Redirects and routes files are a few kilobytes; a megabyte is already
/// generous and keeps a mistaken upload of something else from being read.
const ONE_MB_BYTES: usize = 1024 * 1024;
/// Body limit for the media library's own upload routes: the largest of the
/// three family ceilings, since one post may carry files of any family and
/// the body is read before anything knows which. The family's real limit is
/// then applied per file by `ghost_api::images::store`.
const MAX_MEDIA_BYTES: usize = 200 * 1024 * 1024;

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

/// The platform's own mark: the italic `v` on a tile with a north-east arrow,
/// as the Click dashboard draws it, so the three products share one glyph.
const BRAND_GLYPH: &str = "v<span>↗</span>";

/// What the rail's brand link shows. A logo already carries the publication's
/// name in it, so setting one next to the site name read as the same title
/// twice: the logo stands alone, as large as the rail allows. With no logo the
/// platform's tile leads the name (or, failing that, the title), and a site
/// that has set neither shows the tile with the platform's own name.
fn admin_brand_mark() -> String {
    let (logo, name) = ADMIN_SETTINGS.try_with(|settings| {
        let pick = |key: &str| {
            settings.get_str(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        (pick("logo"), pick("name").or_else(|| pick("title")))
    }).unwrap_or((None, None));
    if let Some(url) = logo {
        // The name goes in `alt` so the link still announces the publication
        // when the image is missing or images are off.
        return format!(
            r#"<span class="brand-logo"><img src="{}" alt="{}"></span>"#,
            esc(&ghost_core::url::base_asset(&url)),
            esc(name.as_deref().unwrap_or("")),
        );
    }
    // The Click dashboard's lockup — tile, "vuuut", and a tracked-out caption
    // beneath — with the publication's name as the caption, so the rail says
    // which site this is without a second title.
    format!(
        r#"<span class="brand-mark" aria-hidden="true">{glyph}</span><span class="brand-name"><b>vuuut</b>{tag}</span>"#,
        glyph = BRAND_GLYPH,
        tag = name
            .map(|n| format!(r#"<span class="tag">{}</span>"#, esc(&n)))
            .unwrap_or_default(),
    )
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
        .route("/appearance/", post(appearance_save))
        .route("/analytics/", get(analytics_page))
        .route("/analytics/click/", get(click::page).post(click::action))
        .route("/modules/", get(modules_page))
        .route("/social/", get(social_page))
        .route("/social/thread/", get(social_thread_page))
        .route("/social/account/", get(social_account_page))
        .route("/social/notes/", post(social_note_create))
        .route("/social/notes/delete/", post(social_note_delete))
        .route("/social/follow/", post(social_follow))
        .route("/social/interact/", post(social_interact))
        .route("/social/block/", post(social_block))
        .route("/social/enabled/", post(social_set_enabled))
        .route("/social/read/", post(social_mark_read))
        // Machine payments was a module card before it moved in with the other
        // payment settings; the old path keeps any bookmark working.
        .route("/machine-payments/", get(|| async { Redirect::to("/ghost/settings/machine-payments/") }))
        .route("/settings/checkout/", get(checkout_design_page))
        .route("/members/", get(members_page).post(member_create))
        .route("/members/bulk/", post(members_bulk))
        .route("/members/export/", get(members_export))
        .route("/members/import/", post(members_import).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/members/import/run/", post(members_import_run))
        // Static `new` ahead of `{id}`, as the posts list's `bulk` is, so the
        // new-member page is never read as a member whose id is "new".
        .route("/members/new/", get(member_new_page).post(member_create))
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
        // Forms: the builder and the responses. `new` and `responses` are
        // static segments and must precede `{id}`, as must `export` under
        // `responses`.
        .route("/forms/", get(forms::forms_page))
        .route("/forms/new/", post(forms::form_create))
        .route("/forms/responses/", get(forms::responses_page))
        .route("/forms/responses/export/", get(forms::responses_export))
        .route("/forms/responses/{id}/", get(forms::response_page))
        .route("/forms/responses/{id}/status/", post(forms::response_status))
        .route("/forms/responses/{id}/delete/", post(forms::response_delete))
        .route("/forms/{id}/", get(forms::form_page).post(forms::form_save))
        .route("/forms/{id}/delete/", post(forms::form_delete))
        .route("/settings/forms/", get(forms::settings_page).post(forms::settings_save))
        // Media: the file library. No `{id}` anywhere in it — a stored path
        // has slashes and dots in it, so a file is named by a `path=`
        // parameter that `ghost_core::media::parse_location` checks, rather
        // than by a path segment that would have to be a wildcard capture.
        // The upload and replace routes carry the largest of the three family
        // ceilings; `store` then enforces the real one for the family a file
        // is filed under.
        .route("/media/", get(media::library_page))
        .route(
            "/media/upload/",
            post(media::upload).layer(axum::extract::DefaultBodyLimit::max(MAX_MEDIA_BYTES)),
        )
        .route("/media/delete/", post(media::bulk_delete))
        .route("/media/item/", get(media::item_page))
        .route("/media/item/save/", post(media::item_save))
        .route("/media/item/rename/", post(media::item_rename))
        .route(
            "/media/item/replace/",
            post(media::item_replace).layer(axum::extract::DefaultBodyLimit::max(MAX_MEDIA_BYTES)),
        )
        .route("/media/item/delete/", post(media::item_delete))
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
        .route("/posts/", get(posts::posts_list))
        .route("/posts/new/", get(editor_new).post(editor_create))
        // Static `bulk` ahead of `{id}`, the same arrangement every other
        // fixed segment in these lists relies on.
        .route("/posts/bulk/", post(posts_bulk))
        .route("/posts/{id}/", get(editor_page).post(editor_save))
        .route("/posts/{id}/preview/", post(preview::render))
        .route("/posts/{id}/preview/test/", post(preview::send_test))
        .route("/posts/{id}/delete/", post(post_delete))
        .route("/posts/{id}/analytics/", get(post_analytics_page))
        .route("/pages/", get(posts::pages_list))
        .route("/pages/new/", get(page_new).post(page_create))
        .route("/pages/bulk/", post(pages_bulk))
        .route("/pages/{id}/", get(editor_page).post(editor_save))
        .route("/pages/{id}/preview/", post(preview::render))
        .route("/pages/{id}/preview/test/", post(preview::send_test))
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
        .route("/settings/machine-payments/", get(machine_payments_page))
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

/// `PUT /emails/{id}/retry`, for merging into the Admin API.
///
/// It belongs with the rest of the emails family in `ghost-api`, but retrying
/// means rendering and sending the newsletter again, which lives here. The
/// binary has both crates in scope and merges this into the same `/admin`
/// nest, so the route sits at `/ghost/api/admin/emails/{id}/retry` and under
/// the versioned prefixes alongside its siblings.
pub fn email_retry_router() -> Router<AppState> {
    Router::new()
        .route("/emails/{id}/retry", axum::routing::put(email_retry))
        .route("/emails/{id}/retry/", axum::routing::put(email_retry))
}

async fn email_retry(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(response) = ghost_api::permissions::require(&state, &headers, ghost_api::permissions::Role::Editor).await {
        return response;
    }
    match retry_email(&state, &id).await {
        Ok(sent) => axum::Json(serde_json::json!({"emails":[{"id":id,"retried_count":sent}]})).into_response(),
        Err(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({"errors":[{"message":message,"errorType":"ValidationError"}]})),
        )
            .into_response(),
    }
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

#[cfg(test)]
mod tests;

