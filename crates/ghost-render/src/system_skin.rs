//! The site's system surfaces — Portal and Search — in the admin's design.
//!
//! Both are Ghost's apps, from a CDN, rendered in same-origin iframes. Rather
//! than rewrite either, `{{ghost_head}}` adds one small script that puts a
//! stylesheet into their iframes as they appear (see `assets/system.js`). It
//! also hides Portal's "Powered by Ghost".
//!
//! The three files are versioned by one hash of all three, so changing any of
//! them is a new URL for every one, and the script can ask for the
//! stylesheets with the same `?v=` it was loaded with.

use sha2::{Digest, Sha256};

pub const SCRIPT: &str = include_str!("../assets/system.js");
pub const PORTAL_CSS: &str = include_str!("../assets/portal.css");
pub const SEARCH_CSS: &str = include_str!("../assets/search.css");

pub fn version() -> &'static str {
    static VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut hash = Sha256::new();
        for part in [SCRIPT, PORTAL_CSS, SEARCH_CSS] {
            hash.update(part.as_bytes());
        }
        hex::encode(hash.finalize())[..12].to_string()
    });
    &VERSION
}

/// The tag `{{ghost_head}}` writes beside Portal and Search.
pub fn head_tag() -> String {
    let src = ghost_core::url::base(&format!("/public/vuuut-system.js?v={}", version()));
    format!(r#"<script defer src="{src}"></script>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_carries_the_version_the_script_reads_back() {
        let tag = head_tag();
        assert!(tag.contains(&format!("vuuut-system.js?v={}", version())), "{tag}");
        // The script splits its own src on `?v=` to ask for the stylesheets.
        assert!(SCRIPT.contains("split('?v=')"));
    }

    #[test]
    fn the_badge_is_hidden_and_portal_is_only_styled_by_its_own_classes() {
        assert!(PORTAL_CSS.contains(".gh-portal-powered { display: none !important; }"));
        // Nothing in the Portal sheet reaches outside Portal's own classes or
        // the iframe's root — it is injected only into Portal's iframes, but
        // a bare `div {}` here would still be a surprise to read.
        for line in PORTAL_CSS.lines().map(str::trim) {
            if line.ends_with('{') && !line.starts_with('@') {
                assert!(
                    line.starts_with(".gh-portal") || line.starts_with(":root"),
                    "unscoped Portal rule: {line}"
                );
            }
        }
    }
}
