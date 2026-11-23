//! The language switcher dropdown `{{ghost_head}}` injects, like Portal.
//!
//! A theme draws the switcher as one ordinary menu link (see
//! `context::Switcher`); this script turns that link into a dropdown of
//! every language, so no theme has to change. Served from
//! `/public/language-switcher.js`.

use sha2::{Digest, Sha256};

/// The script itself.
pub const SCRIPT: &str = include_str!("../assets/language-switcher.js");

/// A short hash of [`SCRIPT`], for the `?v=` on its URL: a changed script is
/// a new URL by construction, so no browser keeps a stale copy.
pub fn version() -> &'static str {
    static VERSION: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| hex::encode(Sha256::digest(SCRIPT.as_bytes()))[..12].to_string());
    &VERSION
}

/// The tags `{{ghost_head}}` writes for a page whose menu carries the
/// switcher: the page's languages as JSON, and the script that reads them.
///
/// The JSON is escaped for an inline `<script>`: a language label or a URL
/// containing `</script>` must not be able to end the block early.
pub fn head_tags(switcher: &serde_json::Value) -> Option<String> {
    if !switcher.is_object() {
        return None;
    }
    let json = serde_json::to_string(switcher)
        .ok()?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let src = vuuut_core::url::base(&format!("/public/language-switcher.js?v={}", version()));
    Some(format!(
        r#"<script id="vuuut-language-data" type="application/json">{json}</script>
    <script defer src="{src}"></script>"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nothing_is_injected_without_a_switcher() {
        assert_eq!(head_tags(&serde_json::Value::Null), None);
    }

    #[test]
    fn the_data_cannot_break_out_of_its_script_block() {
        let tags = head_tags(&json!({"trigger": {"label": "</script><script>alert(1)</script>", "url": "/"}})).unwrap();
        assert_eq!(tags.matches("</script>").count(), 2, "{tags}");
        assert!(tags.contains(r#"</script>"#), "{tags}");
        assert!(tags.contains(&format!("/public/language-switcher.js?v={}", version())));
    }
}
