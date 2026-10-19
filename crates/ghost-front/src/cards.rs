//! Koenig card assets — the CSS and JS that make `kg-*` markup look like a
//! card rather than a stack of bare divs.
//!
//! Ghost keeps these out of themes: the editor can insert a gallery, a
//! toggle or an audio player into any post, so the styles that go with them
//! belong to the platform. Ghost builds `cards.min.css`/`cards.min.js` from
//! one file per card and links them from `ghost_head`; the files here are
//! those same sources, concatenated in the order Ghost's glob produces.
//!
//! They are served at Ghost's own URLs so a theme that already hardcodes
//! them keeps working, and from the binary rather than `content/public/` so
//! there is nothing to build, write or invalidate at boot.
use axum::http::header;
use axum::response::{IntoResponse, Response};
use std::sync::LazyLock;

const CSS: &str = include_str!("../assets/cards.css");
const JS: &str = include_str!("../assets/cards.js");

/// Cache-busting token for the asset URLs. The contents only change when the
/// binary does, so hashing them once at first use is enough — and it means
/// an upgrade invalidates the browser's copy without anyone remembering to
/// bump a number.
pub fn version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(CSS.as_bytes());
        hasher.update(JS.as_bytes());
        hex::encode(hasher.finalize())[..10].to_string()
    });
    &VERSION
}

fn asset(body: &'static str, mime: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            // Safe to cache hard: the URL carries a hash of the contents.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}

pub async fn cards_css() -> Response {
    asset(CSS, "text/css; charset=utf-8")
}

pub async fn cards_js() -> Response {
    asset(JS, "text/javascript; charset=utf-8")
}

/// Guard against the vendored files going missing in a refactor: a card
/// stylesheet that silently became empty would only show up as unstyled
/// posts on the live site.
const _: () = {
    assert!(!CSS.is_empty());
    assert!(!JS.is_empty());
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_assets_cover_the_cards_that_need_styling() {
        for card in ["kg-gallery", "kg-bookmark", "kg-callout", "kg-toggle", "kg-audio", "kg-video", "kg-file", "kg-button"] {
            assert!(CSS.contains(card), "{card} has no styles in the vendored card CSS");
        }
        // the three cards that also need behaviour
        for behaviour in ["kg-gallery", "toggle", "audio"] {
            assert!(JS.contains(behaviour), "{behaviour} has no script in the vendored card JS");
        }
    }

    #[test]
    fn the_version_is_stable_and_short() {
        assert_eq!(version(), version());
        assert_eq!(version().len(), 10);
    }
}
