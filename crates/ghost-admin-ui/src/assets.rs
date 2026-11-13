//! The admin's own assets, served from the binary: the stylesheet and the vendored editor runtime.

use super::*;

// ---------------------------------------------------------------------------
// editor assets
// ---------------------------------------------------------------------------

/// Koenig's version, used to bust caches. The bundle under `assets/` is
/// immutable for a given build, so the URL carries the version and the
/// response can then be cached for as long as the browser likes. The bridge
/// script changes with this admin rather than with Koenig, so the number
/// moves for either: a browser holding a stale bridge would keep a broken
/// editor (and a word count that never counts) until its cache expired.
macro_rules! koenig_version {
    () => {
        "1.8.29"
    };
}
pub(crate) const KOENIG_VERSION: &str = koenig_version!();

/// Where the editor's scripts live, version-stamped.
///
/// Built from [`KOENIG_VERSION`] rather than repeating the number: the two
/// were separate literals and had drifted, so bumping the version no longer
/// changed any URL and a browser kept serving the stale bridge from cache —
/// which is the one thing the version exists to prevent.
pub(crate) const ASSETS: &str = concat!("/ghost/assets/", koenig_version!());

pub(crate) fn pintura_asset_tags(config: &serde_json::Value) -> String {
    match (config["jsUrl"].as_str(), config["cssUrl"].as_str()) {
        (Some(js), Some(css)) => format!(r#"<link rel="stylesheet" href="{}"><script src="{}" defer></script>"#, esc(css), esc(js)),
        _ => String::new(),
    }
}

/// The admin's own assets: its stylesheet, and the vendored editor runtime.
/// Serving these from the binary keeps the admin a single self-contained
/// process — no CDN, and no network at all once the page is open.
pub(crate) async fn admin_asset(Path((version, file)): Path<(String, String)>) -> Response {
    if version != KOENIG_VERSION {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let body: &'static str = match file.as_str() {
        // The whole of the admin's styling. Requested with `?v=` a hash of
        // this file (`admin_css_hash`), so the immutable cache below never
        // outlives a change to it.
        "admin.css" => CSS,
        "react.js" => include_str!("../assets/react.production.min.js"),
        "react-jsx.js" => include_str!("../assets/react-jsx-runtime.js"),
        "react-dom.js" => include_str!("../assets/react-dom.production.min.js"),
        // Ships its own stylesheet, which it injects into <head> on load.
        "koenig-lexical.js" => include_str!("../assets/koenig-lexical.umd.js"),
        "koenig-bridge.js" => include_str!("../assets/koenig-bridge.js"),
        "koenig-integrations.js" => include_str!("../assets/koenig-integrations.js"),
        "koenig-forms.js" => include_str!("../assets/koenig-forms.js"),
        "forms-builder.js" => include_str!("../assets/forms-builder.js"),
        "media-library.js" => include_str!("../assets/media-library.js"),
        "unsplash-picker.js" => include_str!("../assets/unsplash-picker.js"),
        "publishing-workflows.js" => include_str!("../assets/publishing-workflows.js"),
        "automation-editor.js" => include_str!("../assets/automation-editor.js"),
        "machine-payments.js" => include_str!("../assets/machine-payments.js"),
        "checkout-design.js" => include_str!("../assets/checkout-design.js"),
        "analytics-map.js" => include_str!("../assets/analytics-map.js"),
        // Country outlines for the Locations choropleth: Natural Earth
        // v5.1.2 (public domain), projected and simplified to the size this
        // map is drawn at. Big enough to be worth fetching once — which the
        // immutable cache below makes it — and far too big to inline in every
        // Analytics page.
        "world-map.json" => include_str!("../assets/world-map.json"),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    (
        [
            (
                header::CONTENT_TYPE,
                if file.ends_with(".json") {
                    "application/json; charset=utf-8"
                } else if file.ends_with(".css") {
                    "text/css; charset=utf-8"
                } else {
                    "text/javascript; charset=utf-8"
                },
            ),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}
