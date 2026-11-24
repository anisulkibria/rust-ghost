//! Built-in server-rendered admin UI (setup wizard, login, posts editor,
//! tags, settings, integrations). Forms post to themselves and call the
//! repos directly.
use axum::extract::{Form, Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use vuuut_db::repos;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vuuut_api::AppState;

pub mod analytics;
mod click;
pub mod automations;
pub mod comments;
pub mod dashboard;
pub mod forms;
pub mod history;
pub mod languages;
pub mod memberships;
pub mod newsletters;
pub mod posts;
pub mod seo;
pub mod social;
pub mod rest;
pub mod schedule;
pub mod security;
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
// Tools → Search and Filtering: how the public site's search works. Not the
// staff palette, which is `search` and answers at `/omnisearch/`.
pub mod site_search;
// Tools → Advertising & Ad Management. The public half is `vuuut_api::ads`;
// the rules are `vuuut_core::ads`.
pub mod ads;
// Tools → Workflow & Editorial Management. Rules in `vuuut_core::workflows`.
pub mod workflows;
// Tools → Social → Social media: accounts on other platforms, shares to them,
// share buttons. The other half of Social is `social` (the fediverse).
pub mod social_media;
// Tools → Caching & Performance. Runtime half `vuuut_api::caching`.
pub mod caching;
// Tools → Directory & Classified Listings. The public directory is
// `vuuut_front::directory`; the rules are `vuuut_core::directory`.
pub mod directories;
// Tools → Dummy content: demonstration content, generated and removed.
// Rules and catalogue in `vuuut_core::dummies`; the record in
// `vuuut_db::repos::dummies`.
pub mod dummies;
// Tools → E-commerce & Online Stores. The storefront is `vuuut_front::shop`;
// the rules are `vuuut_core::ecom`.
pub mod ecom;
mod payments;
// Tools → Page Builder & Visual Editor. Rules and renderer in
// `vuuut_core::builder`; the public swap in `vuuut_front::routes`.
pub mod builder;
// What is inside each tool, for the Tools hub's search box and the palette.
mod tool_finder;
// Cookie consent & privacy compliance: the overview, the cookie declaration,
// the consent log and Settings → Cookies. The visitor-facing half is
// `vuuut_api::cookies`; the rules are `vuuut_core::cookies`.
pub mod cookies;
// Permissions: the Owner's role × resource matrix, the middleware that
// enforces it, and the catalogue of what can be granted. See `permissions.rs`
// for why enforcement is one layer rather than 147 edited handlers.
pub mod permissions;
// Popups & lead generation: the admin screens. The visitor-facing half is
// `vuuut_api::popups`; the rules are `vuuut_core::popups`.
pub mod popups;
// The admin-wide search behind the rail's box. Its own module because it
// reads from every other one — see `search.rs` for what it indexes and who
// is allowed to see each part.
mod search;
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
pub(crate) mod tools;
use tools::*;

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
/// then applied per file by `vuuut_api::images::store`.
const MAX_MEDIA_BYTES: usize = 200 * 1024 * 1024;
/// Body limit for uploading a backup taken elsewhere: it holds the whole
/// media library and every theme, not one file.
/// Tools → E-commerce: one downloadable product file, plus multipart overhead.
const ECOM_FILE_BYTES: usize = 2 * 1024 * 1024 * 1024 + 1024 * 1024;
/// The product form, which carries any number of those files (each still
/// capped at 2 GB while it streams).
const ECOM_FORM_BYTES: usize = 8 * 1024 * 1024 * 1024;
const MAX_BACKUP_BYTES: usize = 2 * 1024 * 1024 * 1024;

tokio::task_local! {
    static ADMIN_SETTINGS: Arc<vuuut_db::SettingsCache>;
}

async fn with_admin_settings(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    ADMIN_SETTINGS.scope(state.settings, next.run(req)).await
}

/// Put the Security module's response headers on every admin response.
///
/// One middleware rather than a helper each screen remembers to call: a
/// security header that is only on the pages somebody thought about is not a
/// security header. The set itself, and the argument for each member of it,
/// is [`vuuut_core::security::response_headers`] — this only decides *where*
/// it is applied, which is everything under `/ghost/`.
///
/// Three things worth knowing about the placement:
///
/// * It runs **outside** `apply_base_path`, so it is the last thing to touch
///   the response and a screen cannot accidentally be served without it.
/// * It **does not overwrite** a header a handler set deliberately. Nothing
///   under `/ghost/` currently sets one of these — the SVG route that serves
///   `Content-Security-Policy: sandbox` is on the public router, not this one
///   — but a handler that does will have a reason, and a blanket middleware
///   silently replacing a stronger policy with a weaker one is a downgrade
///   dressed as hardening.
/// * `cache-control: no-store` is in the set, and the one thing in the admin
///   that genuinely wants to be cached is the versioned editor runtime under
///   `/ghost/assets/`, which sets its own `immutable` and is therefore left
///   alone by the same rule.
async fn with_security_headers(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let headers = vuuut_api::security_settings::response_headers(
        &state,
        vuuut_core::security::Surface::Admin,
    );
    let mut resp = next.run(req).await;
    for (name, value) in headers {
        let Ok(name) = header::HeaderName::from_bytes(name.as_bytes()) else { continue };
        // A value built from a setting, so it is parsed rather than trusted —
        // `vuuut_core::security` reduces every one of these to a value it
        // offered, but a header value that would not parse must never panic
        // a request.
        let Ok(value) = value.parse::<header::HeaderValue>() else { continue };
        if !resp.headers().contains_key(&name) {
            resp.headers_mut().insert(name, value);
        }
    }
    resp
}

fn admin_site_name() -> String {
    ADMIN_SETTINGS.try_with(|settings| settings.site_name()).unwrap_or_else(|_| vuuut_core::DEFAULT_SITE_TITLE.into())
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
            esc(&vuuut_core::url::base_asset(&url)),
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
        // The admin home: where `/ghost/` and every sign-in land. Open to
        // every signed-in role, with its sections gated inside.
        .route("/dashboard/", get(dashboard::dashboard_page))
        // The rail's search box. Returns an HTML fragment for the modal, open
        // to every signed-in role with each source gated inside.
        .route("/omnisearch/", get(search::search_results))
        // Permissions: Owner only. Not catalogued as a grantable resource —
        // the screen that decides access cannot be one of the things access
        // is decided about.
        .route(
            "/permissions/",
            get(permissions::permissions_page).post(permissions::permissions_save),
        )
        .route("/analytics/", get(analytics_page))
        .route("/analytics/click/", get(click::page).post(click::action))
        .route("/modules/", get(modules::modules_page))
        // Tools → Directory & Classified Listings. Literals before `{id}`.
        // Tools → Page Builder & Visual Editor. Literals before `{id}`.
        .route("/builder/", get(builder::builder_page))
        .route("/builder/new/", post(builder::builder_new))
        .route("/builder/open/", get(builder::builder_open))
        .route("/builder/theme/", get(builder::theme_page))
        .route("/builder/import/", post(builder::builder_import).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::elementor::MAX_IMPORT_BODY)))
        .route("/builder/{id}/export/", get(builder::builder_export))
        .route("/builder/templates/", get(builder::templates_page))
        .route("/builder/templates/new/", post(builder::template_new))
        .route("/builder/templates/{id}/edit/", get(builder::template_editor))
        .route("/builder/templates/{id}/save/", post(builder::template_save).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::spec::MAX_LAYOUT_BODY)))
        .route("/builder/templates/{id}/catalogue/", get(builder::template_catalogue))
        .route("/builder/templates/{id}/import/", post(builder::template_upload).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::elementor::MAX_IMPORT_BODY)))
        .route("/builder/templates/{id}/interchange/", post(builder::template_interchange).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::elementor::MAX_INTERCHANGE_BODY)))
        .route("/builder/templates/{id}/render/", post(builder::template_render).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::spec::MAX_LAYOUT_BODY)))
        .route("/builder/templates/{id}/active/", post(builder::template_active))
        .route("/builder/templates/{id}/rename/", post(builder::template_rename))
        .route("/builder/templates/{id}/delete/", post(builder::template_delete))
        .route("/builder/settings/", get(builder::settings_page).post(builder::settings_save))
        .route("/builder/{id}/active/", post(builder::builder_active))
        .route("/builder/{id}/delete/", post(builder::builder_delete))
        // Tools → E-commerce & Online Stores (static segments before {id}).
        .route("/ecom/", get(ecom::ecom_home))
        .route("/ecom/analytics/", get(ecom::ecom_analytics))
        .route("/ecom/products/", get(ecom::products_page))
        .route("/ecom/products/export.csv", get(ecom::products_export))
        .route("/ecom/products/bulk/", post(ecom::products_bulk))
        .route("/ecom/products/import/", get(ecom::products_import_page).post(ecom::products_import).layer(axum::extract::DefaultBodyLimit::max(26 * 1024 * 1024)))
        .route("/ecom/products/new/", get(ecom::product_new).post(ecom::product_create).layer(axum::extract::DefaultBodyLimit::max(ECOM_FORM_BYTES)))
        .route("/ecom/products/{id}/", get(ecom::product_edit).post(ecom::product_save).layer(axum::extract::DefaultBodyLimit::max(ECOM_FORM_BYTES)))
        .route("/ecom/products/{id}/delete/", post(ecom::product_delete))
        .route("/ecom/products/{id}/duplicate/", post(ecom::product_duplicate))
        .route("/ecom/products/{id}/files/", post(ecom::product_file_upload).layer(axum::extract::DefaultBodyLimit::max(ECOM_FILE_BYTES)))
        .route("/ecom/products/{id}/files/{fid}/", get(ecom::product_file_download))
        .route("/ecom/products/{id}/files/{fid}/delete/", post(ecom::product_file_delete))
        .route("/ecom/collections/", get(ecom::collections_page))
        .route("/ecom/collections/new/", get(ecom::collection_new).post(ecom::collection_create))
        .route("/ecom/collections/{id}/", get(ecom::collection_edit).post(ecom::collection_save))
        .route("/ecom/collections/{id}/products/", post(ecom::collection_products))
        .route("/ecom/collections/{id}/delete/", post(ecom::collection_delete))
        .route("/ecom/inventory/", get(ecom::inventory_page))
        .route("/ecom/inventory/{id}/", post(ecom::inventory_set))
        .route("/ecom/orders/", get(ecom::orders_page))
        .route("/ecom/orders/export.csv", get(ecom::orders_export))
        .route("/ecom/orders/abandoned/", get(ecom::abandoned_page))
        .route("/ecom/orders/new/", get(ecom::order_new).post(ecom::order_create))
        .route("/ecom/orders/{id}/", get(ecom::order_page))
        .route("/ecom/orders/{id}/print/", get(ecom::order_print))
        .route("/ecom/orders/{id}/paid/", post(ecom::order_paid))
        .route("/ecom/orders/{id}/fulfil/", post(ecom::order_fulfil))
        .route("/ecom/orders/{id}/unfulfil/", post(ecom::order_unfulfil))
        .route("/ecom/orders/{id}/refund/", post(ecom::order_refund))
        .route("/ecom/orders/{id}/cancel/", post(ecom::order_cancel))
        .route("/ecom/orders/{id}/archive/", post(ecom::order_archive))
        .route("/ecom/orders/{id}/note/", post(ecom::order_note))
        .route("/ecom/orders/{id}/comment/", post(ecom::order_comment))
        .route("/ecom/orders/{id}/contact/", post(ecom::order_contact))
        .route("/ecom/orders/{id}/resend/", post(ecom::order_resend))
        .route("/ecom/orders/{id}/delete/", post(ecom::order_delete))
        .route("/ecom/orders/{id}/downloads/", post(ecom::order_downloads))
        .route("/ecom/customers/", get(ecom::customers_page))
        .route("/ecom/customers/export.csv", get(ecom::customers_export))
        .route("/ecom/customers/new/", get(ecom::customer_new).post(ecom::customer_create))
        .route("/ecom/customers/{id}/", get(ecom::customer_page).post(ecom::customer_save))
        .route("/ecom/customers/{id}/delete/", post(ecom::customer_delete))
        .route("/ecom/discounts/", get(ecom::discounts_page).post(ecom::discount_create))
        .route("/ecom/discounts/{id}/", post(ecom::discount_save))
        .route("/ecom/discounts/{id}/delete/", post(ecom::discount_delete))
        .route("/ecom/settings/", get(ecom::ecom_settings_page).post(ecom::ecom_settings_save))
        .route("/ecom/settings/shipping/", post(ecom::shipping_create))
        .route("/ecom/settings/shipping/{id}/", post(ecom::shipping_save))
        .route("/ecom/settings/shipping/{id}/delete/", post(ecom::shipping_delete))
        .route("/ecom", get(|| async { Redirect::permanent("/ghost/ecom/") }))
        .route("/directories/", get(directories::directories_page))
        .route("/directories/export.csv", get(directories::export_csv))
        .route("/directories/listings/", get(directories::listings_page))
        .route("/directories/listings/new/", get(directories::listing_new).post(directories::listing_create))
        .route("/directories/listings/{id}/", get(directories::listing_edit).post(directories::listing_save))
        .route("/directories/listings/{id}/status/", post(directories::listing_status))
        .route("/directories/listings/{id}/delete/", post(directories::listing_delete))
        .route("/directories/categories/", get(directories::categories_page).post(directories::category_create))
        .route("/directories/categories/{id}/", post(directories::category_save))
        .route("/directories/categories/{id}/delete/", post(directories::category_delete))
        .route("/directories/settings/", get(directories::dir_settings_page).post(directories::dir_settings_save))
        // Tools → Caching & Performance.
        // Tools → Dummy content.
        .route("/dummies/", get(dummies::dummies_page))
        .route("/dummies/full/", post(dummies::generate_full))
        .route("/dummies/custom/", post(dummies::generate_custom))
        .route("/dummies/remove/", post(dummies::remove))
        .route("/caching/", get(caching::caching_page))
        .route("/caching/purge/", post(caching::purge_all))
        .route("/caching/stats/reset/", post(caching::reset_stats))
        .route("/caching/pages/", get(caching::pages_page))
        .route("/caching/pages/purge/", post(caching::purge_page))
        .route("/caching/settings/", get(caching::cp_settings_page).post(caching::cp_settings_save))
        .route("/caching/cdn/disconnect/", post(caching::cdn_disconnect))
        // Social → Social media. Literals before `{id}`.
        .route("/social/media/", get(social_media::accounts_page))
        .route("/social/media/accounts/", post(social_media::account_create))
        .route("/social/media/compose/", get(social_media::compose_page).post(social_media::compose_send))
        .route("/social/media/history/", get(social_media::history_page))
        .route("/social/media/settings/", get(social_media::sm_settings_page).post(social_media::sm_settings_save))
        .route("/social/media/accounts/{id}/", get(social_media::account_page).post(social_media::account_save))
        .route("/social/media/accounts/{id}/test/", post(social_media::account_test))
        .route("/social/media/accounts/{id}/auto/", post(social_media::account_auto))
        .route("/social/media/accounts/{id}/delete/", post(social_media::account_delete))
        .route("/social/media/shares/{id}/retry/", post(social_media::share_retry))
        .route("/social/media/shares/{id}/cancel/", post(social_media::share_cancel))
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
        .route("/machine-payments/", get(machine_payments_page))
        .route("/memberships/settings/checkout/", get(checkout_design_page))
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
        // memberships: the funnel, its gaps, and the tier and offer
        // inventories. The members list above is this module's Members tab
        // and deliberately keeps its own URL — see the module docs. Nothing
        // here is addressed by id and nothing here writes.
        .route("/memberships/", get(memberships::memberships_page))
        .route("/memberships/tiers/", get(memberships::tiers_page))
        .route("/memberships/offers/", get(memberships::offers_page))
        // newsletters: the sending programme, and every edition that went
        // out. `retry` is a static segment under `editions` and nothing here
        // is addressed by id — the one id the module takes comes back in a
        // posted body rather than in a path.
        .route("/newsletters/", get(newsletters::newsletters_page))
        .route("/newsletters/editions/", get(newsletters::editions_page))
        .route("/newsletters/editions/retry/", post(newsletters::edition_retry))
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
        .route("/forms/settings/", get(forms::settings_page).post(forms::settings_save))
        // security: the posture report, the live sessions and lockouts, and
        // the switches behind them. Static segments only — nothing here is
        // addressed by id, because the two things that are (a session, a
        // lockout key) are posted in a body rather than named in a path.
        .route("/security/", get(security::security_page))
        .route("/security/sessions/", get(security::sessions_page))
        .route("/security/sessions/revoke/", post(security::session_revoke))
        .route("/security/sessions/revoke-all/", post(security::sessions_revoke_all))
        .route("/security/lockouts/clear/", post(security::lockout_clear))
        .route("/security/lockouts/clear-all/", post(security::lockouts_clear_all))
        .route("/security/firewall/", get(security::firewall_page))
        .route("/security/firewall/add/", post(security::firewall_add))
        .route("/security/firewall/delete/", post(security::firewall_delete))
        .route("/security/firewall/clear/", post(security::firewall_clear))
        .route("/security/firewall/toggle/", post(security::firewall_toggle))
        .route("/security/settings/", get(security::settings_page).post(security::settings_save))
        // seo: the report, and the switches behind it. Static `content`
        // before any `{id}` this module might grow.
        .route("/seo/", get(seo::seo_page))
        .route("/seo/content/", get(seo::content_page))
        .route("/seo/settings/", get(seo::settings_page).post(seo::settings_save))
        .route("/languages/", get(languages::languages_page))
        .route("/languages/settings/", get(languages::settings_page).post(languages::settings_save))
        // Media: the file library. No `{id}` anywhere in it — a stored path
        // has slashes and dots in it, so a file is named by a `path=`
        // parameter that `vuuut_core::media::parse_location` checks, rather
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
        // Image optimisation and the Media tool’s settings.
        .route("/media/optimize/", post(media::images::optimize_library))
        .route("/media/settings/", get(media::images::images_settings_page).post(media::images::images_settings_save))
        .route("/media/item/", get(media::item_page))
        .route("/media/item/save/", post(media::item_save))
        .route("/media/item/rename/", post(media::item_rename))
        .route(
            "/media/item/replace/",
            post(media::item_replace).layer(axum::extract::DefaultBodyLimit::max(MAX_MEDIA_BYTES)),
        )
        .route("/media/item/delete/", post(media::item_delete))
        // An image's private original, which the public site never serves.
        .route("/media/item/original/", get(media::images::original_page))
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
        // The visual editor: under /posts/ so the right to edit the post is
        // what opens it, not a grant on the Page Builder tool.
        .route("/posts/{id}/builder/", get(builder::editor_page))
        .route("/posts/{id}/builder/save/", post(builder::editor_save).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::spec::MAX_LAYOUT_BODY)))
        .route("/posts/{id}/builder/catalogue/", get(builder::editor_catalogue))
        .route("/posts/{id}/builder/import/", post(builder::editor_upload).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::elementor::MAX_IMPORT_BODY)))
        .route("/posts/{id}/builder/interchange/", post(builder::editor_interchange).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::elementor::MAX_INTERCHANGE_BODY)))
        .route("/posts/{id}/builder/render/", post(builder::editor_render).layer(axum::extract::DefaultBodyLimit::max(vuuut_core::builder::spec::MAX_LAYOUT_BODY)))
        .route("/posts/{id}/builder/revisions/", get(builder::editor_revisions))
        .route("/posts/{id}/builder/revisions/{rid}/", get(builder::editor_revision))
        .route("/posts/{id}/builder/templates/", post(builder::editor_template_save))
        .route("/posts/{id}/preview/test/", post(preview::send_test))
        .route("/posts/{id}/delete/", post(post_delete))
        .route("/posts/{id}/analytics/", get(post_analytics_page))
        // Post history and Page history, reached from the editor's own panel.
        .route("/posts/{id}/history/", get(history::history_page))
        .route("/posts/{id}/history/{revision}/restore/", post(history::restore))
        // multilingual: linking an entry to its translations, from the
        // editor's Language panel.
        .route("/posts/{id}/translations/link/", post(languages::link))
        .route("/posts/{id}/translations/unlink/", post(languages::unlink))
        .route("/posts/{id}/translations/new/", post(languages::create))
        .route("/posts/{id}/translations/search/", get(languages::search))
        .route("/pages/", get(posts::pages_list))
        .route("/pages/new/", get(page_new).post(page_create))
        .route("/pages/bulk/", post(pages_bulk))
        .route("/pages/{id}/", get(editor_page).post(editor_save))
        .route("/pages/{id}/preview/", post(preview::render))
        .route("/pages/{id}/preview/test/", post(preview::send_test))
        .route("/pages/{id}/delete/", post(page_delete))
        .route("/pages/{id}/history/", get(history::history_page))
        .route("/pages/{id}/history/{revision}/restore/", post(history::restore))
        .route("/pages/{id}/translations/link/", post(languages::link))
        .route("/pages/{id}/translations/unlink/", post(languages::unlink))
        .route("/pages/{id}/translations/new/", post(languages::create))
        .route("/pages/{id}/translations/search/", get(languages::search))
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
        // Tools: the site-building screens, each at a top-level URL. Their
        // old homes (mostly under Settings) redirect, so bookmarks still land.
        .route("/tools/", get(tools_page))
        // Popups & lead generation. Static segments before `{id}`, so
        // `/popups/new/` and `/popups/leads/` are never read as an id.
        .route("/popups/settings/", get(popups::popups_settings_page).post(popups::popups_settings_save))
        .route("/popups/", get(popups::popups_page))
        .route("/popups/new/", get(popups::popup_new_page).post(popups::popup_create))
        .route("/popups/leads/", get(popups::leads_page))
        .route("/popups/leads.csv", get(popups::leads_csv))
        .route("/popups/{id}/", get(popups::popup_edit_page).post(popups::popup_save))
        .route("/popups/{id}/delete/", post(popups::popup_delete))
        .route("/popups/{id}/leads/", get(popups::popup_leads_page))
        .route("/popups/{id}/leads.csv", get(popups::popup_leads_csv))
        // Tools → Workflow & Editorial Management. Literals before `{id}`.
        .route("/workflows/", get(workflows::board_page))
        .route("/workflows/calendar/", get(workflows::calendar_page))
        .route("/workflows/ideas/", post(workflows::create_idea))
        .route("/workflows/settings/", get(workflows::wf_settings_page).post(workflows::wf_settings_save))
        .route("/workflows/items/{id}/", get(workflows::wf_item_page).post(workflows::wf_item_save))
        .route("/workflows/items/{id}/move/", post(workflows::move_item))
        .route("/workflows/items/{id}/check/", post(workflows::wf_item_check))
        .route("/workflows/items/{id}/notes/", post(workflows::wf_note_add))
        .route("/workflows/items/{id}/notes/{note}/delete/", post(workflows::wf_note_delete))
        // Tools → Advertising & Ad Management. Every literal segment is
        // routed before `{id}`, so `/ads/new/` is never read as an ad id.
        .route("/ads/", get(ads::ads_page))
        .route("/ads/list/", get(ads::list_page))
        .route("/ads/new/", get(ads::new_page).post(ads::create))
        .route("/ads/report.csv", get(ads::report_csv))
        .route("/ads/settings/", get(ads::ads_settings_page).post(ads::ads_settings_save))
        .route("/ads/settings/switch/", post(ads::ads_settings_switch))
        .route("/ads/advertisers/", get(ads::advertisers_page).post(ads::advertiser_create))
        .route("/ads/advertisers/{id}/", get(ads::advertiser_page).post(ads::advertiser_save))
        .route("/ads/advertisers/{id}/delete/", post(ads::advertiser_delete))
        .route("/ads/advertisers/{id}/report.csv", get(ads::advertiser_csv))
        .route("/ads/{id}/", get(ads::edit_page).post(ads::save))
        .route("/ads/{id}/duplicate/", post(ads::duplicate))
        .route("/ads/{id}/delete/", post(ads::delete))
        // Tools → Search and Filtering (the public site's search).
        .route("/search/", get(site_search::search_page).post(site_search::search_save))
        .route("/search/try/", get(site_search::try_page))
        .route("/search/insights/", get(site_search::insights_page))
        .route("/search/insights/clear/", post(site_search::insights_clear))
        // Cookie consent & privacy compliance. All static segments; the one
        // `{id}` route is below the literals it could otherwise swallow.
        .route("/cookies/settings/", get(cookies::cookies_settings_page).post(cookies::cookies_settings_save))
        .route("/cookies/", get(cookies::cookies_page))
        .route("/cookies/switch/", post(cookies::cookies_switch))
        .route("/cookies/renew/", post(cookies::cookies_renew))
        .route("/cookies/declarations/", get(cookies::declarations_page).post(cookies::declaration_create))
        .route("/cookies/declarations/import/", post(cookies::declaration_import))
        .route("/cookies/declarations/{id}/delete/", post(cookies::declaration_delete))
        .route("/cookies/log/", get(cookies::log_page))
        .route("/cookies/log.csv", get(cookies::log_csv))
        .route("/cookies/log/purge/", post(cookies::log_purge))
        .route("/customizations/", get(design_page).post(design_save))
        .route("/customizations/preview/", post(design_preview))
        .route("/customizations/reset/", post(design_reset))
        .route("/design/", get(|| async { Redirect::permanent("/ghost/customizations/") }))
        .route("/navigations/", get(navigation_page).post(navigation_save))
        .route("/navigation/", get(|| async { Redirect::permanent("/ghost/navigations/") }))
        .route("/code-injection/", get(code_injection_page).post(code_injection_save))
        .route("/settings/code-injection/", get(|| async { Redirect::permanent("/ghost/code-injection/") }))
        .route("/announcement/", get(site_page).post(site_save))
        .route("/settings/site/", get(|| async { Redirect::permanent("/ghost/announcement/") }))
        .route("/redirects/", get(redirects_page).post(redirects_save))
        .route("/redirects/download/", get(redirects_download))
        .route("/redirects/upload/", post(redirects_upload).layer(axum::extract::DefaultBodyLimit::max(ONE_MB_BYTES)))
        .route("/routes/", get(routes_page).post(routes_save))
        .route("/routes/download/", get(routes_download))
        .route("/routes/upload/", post(routes_upload).layer(axum::extract::DefaultBodyLimit::max(ONE_MB_BYTES)))
        .route("/settings/advanced/", get(|| async { Redirect::permanent("/ghost/tools/") }))
        .route("/settings/staff/", get(|| async { Redirect::permanent("/ghost/staffs/") }))
        .route(
            "/settings/staff/{slug}/",
            get(|Path(slug): Path<String>| async move { Redirect::permanent(&format!("/ghost/staffs/{slug}/")) }),
        )
        .route("/settings/", get(settings_page))
        .route("/settings/general/", get(general_page).post(general_save))
        .route("/settings/general/admin-address/", post(admin_address_save))
        // Meta data and Social accounts are sections of General settings now.
        // The old paths stay as redirects: they are what an existing bookmark,
        // or a link from elsewhere in this admin, still points at.
        .route("/settings/metadata/", get(|| async { Redirect::to("/ghost/settings/general/") }))
        .route("/settings/social/", get(|| async { Redirect::to("/ghost/settings/general/") }))
        .route("/analytics/settings/", get(analytics_settings_page).post(analytics_settings_save))
        .route("/settings/features/", get(features_page).post(features_save))
        .route("/memberships/settings/", get(membership_page).post(membership_save))
        .route("/settings/machine-payments/", get(|| async { Redirect::permanent("/ghost/machine-payments/") }))
        .route("/settings/payments/", get(payments::page))
        .route("/settings/payments/store/", post(payments::save_store))
        .route("/settings/payments/tips/", post(payments::save_tips))
        .route("/settings/payments/polar/", post(payments::save_polar))
        .route("/settings/payments/gumroad/", post(payments::save_gumroad))
        .route("/settings/payments/products/", post(payments::save_product))
        .route("/settings/payments/stripe/connect/", post(payments::stripe_connect))
        .route("/settings/payments/stripe/oauth/start/", post(payments::stripe_oauth_start))
        .route("/settings/payments/stripe/oauth/finish/", post(payments::stripe_oauth_finish))
        .route("/settings/payments/stripe/disconnect/", post(payments::stripe_disconnect))
        .route("/settings/payments/stripe/webhook/", post(payments::stripe_set_webhook_secret))
        .route("/settings/payments/stripe/webhook/deactivate/", post(payments::stripe_deactivate_webhook))
        .route("/payments/", axum::routing::any(payments::legacy))
        .route("/payments/{*tail}", axum::routing::any(payments::legacy))
        .route("/memberships/settings/stripe/{*tail}", axum::routing::any(payments::legacy))
        .route("/memberships/settings/tiers/{id}/", get(tier_edit_page).post(tier_edit_save))
        .route("/memberships/settings/tiers/new/", post(tier_create))
        .route("/memberships/settings/fields/new/", post(metafield_new))
        .route("/memberships/settings/fields/{id}/", post(metafield_save))
        .route("/memberships/settings/fields/{id}/delete/", post(metafield_delete))
        .route("/memberships/settings/fields/{id}/move/", post(metafield_move))
        .route("/settings/email/", get(|| async { Redirect::permanent("/ghost/settings/emails/") }))
        .route("/settings/emails/", get(email_page).post(email_save))
        .route("/settings/emails/test/", post(email_test))
        .route("/newsletters/settings/", get(newsletters_settings_page).post(newsletters_settings_save))
        // The static segment is matched ahead of `{id}`, the same way the
        // tier routes above rely on.
        .route("/newsletters/settings/new/", post(newsletter_create))
        .route("/newsletters/settings/{id}/", get(newsletter_edit_page).post(newsletter_edit_save))
        .route("/newsletters/settings/{id}/preview/", get(newsletter_preview))
        // Growth used to be a settings group; it is a Tool now.
        .route("/settings/growth/", get(|| async { Redirect::permanent("/ghost/growth/") }))
        .route("/growth/", get(growth_page).post(growth_save))
        .route("/growth/explore/", post(growth_explore_save))
        .route("/growth/recommendations/new/", post(recommendation_new))
        .route("/growth/recommendations/{id}/", post(recommendation_save))
        .route("/growth/recommendations/{id}/delete/", post(recommendation_delete))
        .route("/growth/offers/new/", post(offer_new))
        .route("/growth/offers/{id}/", get(offer_edit_page).post(offer_save))
        .route("/growth/offers/{id}/status/", post(offer_set_status))
        .route("/history/", get(history_page))
        .route("/settings/history/", get(|| async { Redirect::permanent("/ghost/history/") }))
        .route("/settings/danger/", get(danger_page))
        .route("/settings/danger/content/", post(danger_delete_content))
        .route("/settings/danger/site/", post(danger_delete_site))
        .route("/settings/danger/auth/", post(danger_reset_auth))
        .route("/settings/danger/gifts/", post(danger_reset_gifts))
        .route("/settings/about/", get(about_page))
        // Migration tools. Import and export used to live under Advanced;
        // Ghost gives them a screen of their own, and so does this.
        // Importing is Tools → Migrations; exporting and backups are
        // Settings → Backups. The old Migration tools URLs land on the right one.
        .route("/migrations/", get(migrations_page))
        .route(
            "/migrations/import/",
            post(migration_import).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/settings/migration/", get(|| async { Redirect::permanent("/ghost/migrations/") }))
        .route(
            "/settings/migration/import/",
            post(migration_import).layer(axum::extract::DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        // Backups used to be a settings group; it is a Tool now.
        .route("/settings/backups/", get(|| async { Redirect::permanent("/ghost/backups/") }))
        .route("/backups/", get(backups_page))
        .route("/backups/schedule/", post(backups_schedule_save))
        .route("/backups/create/", post(backups_create))
        .route("/backups/restore/", post(backups_restore))
        .route("/backups/delete/", post(backups_delete))
        .route("/backups/download/{name}", get(backups_download))
        // a backup carries every image and theme, so it is allowed to be big
        .route("/backups/upload/", post(backups_upload).layer(axum::extract::DefaultBodyLimit::max(MAX_BACKUP_BYTES)))
        .route("/backups/export/", get(migration_export_all))
        .route("/backups/export/content/", get(migration_export_content))
        .route("/backups/export/ghost/", get(migration_export_for_ghost))
        .route("/backups/export/analytics/", get(migration_export_analytics))
        .route("/backups/export/themes/", get(migration_export_themes))
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
        .route("/staffs/", get(staff_page))
        .route("/staffs/security/", post(staff_security_save))
        .route("/staffs/invite/", post(staff_invite))
        .route("/staffs/invite/{id}/resend/", post(staff_invite_resend))
        .route("/staffs/invite/{id}/revoke/", post(staff_invite_revoke))
        .route("/staffs/{slug}/", get(profile_page).post(profile_save))
        .route("/staffs/{slug}/social/", post(profile_social_save))
        .route("/staffs/{slug}/notifications/", post(profile_notifications))
        .route("/staffs/{slug}/password/", post(profile_password))
        .route("/staffs/{slug}/role/", post(staff_set_role))
        .route("/staffs/{slug}/status/", post(staff_set_status))
        .route("/staffs/{slug}/delete/", post(staff_delete))
        // redeeming an invitation is the one staff route a signed-out person
        // must reach
        .route("/invite/{token}/", get(invite_accept_page).post(invite_accept_submit))
        .route("/integrations/", get(integrations_page).post(integration_create))
        // the built-in apps. Static segments, so they cannot be shadowed by a
        // custom integration's id — which is why those live under /custom/.
        .route("/integrations/zapier/", get(zapier_page))
        .route("/integrations/slack/", get(slack_page).post(slack_save))
        .route("/integrations/slack/test/", post(slack_test))
        .route("/integrations/unsplash/", get(unsplash_page).post(unsplash_save))
        .route("/integrations/firstpromoter/", get(firstpromoter_page).post(firstpromoter_save))
        .route("/integrations/pintura/", get(pintura_page).post(pintura_save))
        .route("/integrations/transistor/", get(transistor_page).post(transistor_save))
        .route(
            "/integrations/custom/{id}/",
            get(custom_integration_page).post(custom_integration_save),
        )
        .route(
            "/integrations/custom/{id}/webhooks/new/",
            post(webhook_new),
        )
        .route(
            "/integrations/custom/{id}/webhooks/{wid}/delete/",
            post(webhook_delete),
        )
        .route(
            "/integrations/custom/{id}/delete/",
            post(custom_integration_delete),
        )
        // bookmarks and in-page links from before Integrations became a
        // settings page of its own still resolve, wherever they pointed
        .route("/settings/integrations/", get(|| async { Redirect::permanent("/ghost/integrations/") }))
        .route(
            "/settings/integrations/{*rest}",
            get(|Path(rest): Path<String>| async move {
                Redirect::permanent(&format!("/ghost/integrations/{rest}"))
            }),
        )
        .route(
            "/settings/advanced/integrations/",
            get(|| async { Redirect::permanent("/ghost/integrations/") }),
        )
        .route(
            "/settings/advanced/integrations/{*rest}",
            get(|Path(rest): Path<String>| async move {
                Redirect::permanent(&format!("/ghost/integrations/{rest}"))
            }),
        )
        // the editor's vendored runtime (React + Koenig), served from the
        // binary and cached hard behind the version in the path
        .route("/assets/{version}/{file}", get(admin_asset))
        .merge(legacy_tool_settings_routes())
        .layer(axum::middleware::from_fn(apply_base_path))
        .layer(axum::middleware::from_fn_with_state(state.clone(), with_security_headers))
        .layer(axum::middleware::from_fn_with_state(state.clone(), with_admin_settings))
        // Permissions, inside the settings layer so the matrix is read from a
        // warm `SettingsCache`, and outside every handler so a role that may
        // not reach a screen is refused before the handler that would have
        // trusted it runs.
        .layer(axum::middleware::from_fn_with_state(state.clone(), permissions::enforce))
        // Outermost: an address the firewall refuses is refused before the
        // session cookie is read, so a barred address is barred whether or
        // not it holds a valid login.
        .layer(axum::middleware::from_fn_with_state(
            state,
            vuuut_api::firewall::admin_guard,
        ))
}

/// `PUT /emails/{id}/retry`, for merging into the Admin API.
///
/// It belongs with the rest of the emails family in `vuuut-api`, but retrying
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
    if let Err(response) = vuuut_api::permissions::require(&state, &headers, vuuut_api::permissions::Role::Editor).await {
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
    if !vuuut_core::url::has_subdir() {
        return resp;
    }

    if let Some(location) = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()) {
        if let Ok(value) = vuuut_core::url::base(location).parse() {
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
    let rewritten = vuuut_core::url::rewrite_html(html);
    // the length changed, so a stale Content-Length would truncate the page
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(rewritten))
}

#[cfg(test)]
mod tests;

