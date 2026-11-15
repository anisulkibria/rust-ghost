//! `/robots.txt` — the first file a crawler asks for.
//!
//! This port served one only on a private site, where the gate in
//! [`crate::private`] answers with `Disallow: /` before anything else runs. A
//! public site returned the theme's 404 page: HTML, 404, and no `Sitemap:`
//! line anywhere, so the sitemap index was discoverable only by guessing or
//! by submitting it to a search console by hand.
//!
//! Two representations, as with everything else here:
//!
//! * **The default** ([`ghost_core::seo::DEFAULT_ROBOTS`]) — Ghost's own
//!   file, with the `Sitemap:` line filled in with this install's absolute
//!   URL. Nothing in it blocks content.
//! * **A custom file**, from Settings → SEO. It replaces the default
//!   entirely, which is the point: a site that needs `Crawl-delay`, a
//!   specific agent blocked, or a second sitemap listed cannot express that
//!   by editing a template.
//!
//! The custom file is bounded and stripped of control characters on the way
//! in (Settings → SEO does that), and is written out as `text/plain` with no
//! interpolation at all, so there is nothing in it for a crawler to mistake
//! for a header.
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;

/// An hour. Long enough that a crawler does not refetch it on every pass,
/// short enough that editing the file in the admin takes effect while
/// somebody is still watching.
const CACHE: &str = "public, max-age=3600";

/// The file this site serves, custom or default.
///
/// Public so the SEO dashboard can show exactly what a crawler would read
/// rather than describing it — a report that paraphrases the file is a report
/// that can be wrong about it.
pub fn body(state: &AppState) -> String {
    let custom = state
        .settings
        .get_str("seo_robots_txt")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match custom {
        Some(custom) => {
            // A file that has been edited is served as written. One trailing
            // newline is added if it is missing, because a directive on an
            // unterminated last line is ignored by some crawlers.
            if custom.ends_with('\n') {
                custom
            } else {
                format!("{custom}\n")
            }
        }
        None => ghost_core::seo::DEFAULT_ROBOTS.replace("{{sitemap}}", &sitemap_url(state)),
    }
}

/// The absolute URL of the sitemap index, base path included exactly once.
///
/// `site_url` already carries the install's base path, so it cannot simply be
/// concatenated with the prefixed path — that is what `url::absolute` is for.
pub fn sitemap_url(state: &AppState) -> String {
    ghost_core::url::absolute(&state.site_url(), &ghost_core::url::base("/sitemap.xml"))
}

pub async fn robots_txt(State(state): State<AppState>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, CACHE),
        ],
        body(&state),
    )
        .into_response()
}
