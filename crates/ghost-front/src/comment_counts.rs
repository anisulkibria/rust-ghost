//! `comment-counts.min.js` — the script that fills in `{{comment_count}}`.
//!
//! `{{comment_count}}` renders a `<script data-ghost-comment-count="<id>">`
//! placeholder rather than a number, because a cached index page cannot know
//! how many comments each post has by the time someone reads it. This script,
//! linked from `ghost_head`, collects every placeholder on the page, fetches
//! all their counts in one request to `/members/api/comments/counts/`, and
//! replaces each placeholder with the rendered text.
//!
//! It also watches for placeholders added later, so a theme with infinite
//! scroll gets counts on the posts it appends.
//!
//! Vendored from Ghost's `core/frontend/src/comment-counts/comment-counts.js`
//! and served from the binary rather than `content/public/`, so there is
//! nothing to build or write at boot — the same arrangement as the Koenig
//! card assets next door.
use axum::http::header;
use axum::response::{IntoResponse, Response};

const JS: &str = include_str!("../assets/comment-counts.js");

/// Guard against the vendored file going missing in a refactor: an empty one
/// would leave every comment count on the site blank, and only on pages
/// nobody tests.
const _: () = {
    assert!(!JS.is_empty());
};

/// Served at Ghost's own URL (`/public/comment-counts.min.js`) so a theme
/// that hardcodes it keeps working.
///
/// Unlike the card assets this URL carries no version token — Ghost's does
/// not either — so it is revalidated rather than cached hard. The file is
/// 3 KB and conditional requests on it are cheap.
pub async fn comment_counts_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=0, must-revalidate"),
        ],
        JS,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_script_still_speaks_the_same_protocol() {
        // the endpoint it reads off `ghost_head`'s script tag
        assert!(JS.contains("data-ghost-comments-counts-api"));
        // every attribute `{{comment_count}}` emits
        for attr in [
            "ghostCommentCount",
            "ghostCommentCountEmpty",
            "ghostCommentCountSingular",
            "ghostCommentCountPlural",
            "ghostCommentCountTag",
            "ghostCommentCountClassName",
            "ghostCommentCountAutowrap",
        ] {
            assert!(JS.contains(attr), "the counts script no longer reads {attr}");
        }
        // and the observer that catches placeholders added after load
        assert!(JS.contains("MutationObserver"));
    }
}
