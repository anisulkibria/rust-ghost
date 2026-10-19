//! Ghost-compatible slug generation ("generateSlug" plugin port).
use once_cell::sync::Lazy;
use regex::Regex;

/// Routing-reserved slugs that no model may claim verbatim.
pub const PROTECTED_SLUGS: &[&str] = &["ghost", "rss", "amp", "feed"];

/// Extra protected slugs per model (Ghost: tags may not be "new").
pub fn protected_for(model: &str) -> Vec<&'static str> {
    match model {
        "tags" => vec!["new"],
        "users" => vec!["ghost", "ghost-admin"],
        _ => vec![],
    }
}

static NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-zA-Z0-9-_]").unwrap());
static MULTI_DASH: Lazy<Regex> = Lazy::new(|| Regex::new(r"-+").unwrap());

/// Ghost's `string.safe`: downcase, dash-separate non-alphanumerics, trim dashes.
/// Keeps unicode letters/digits by transliterating the simple cases via the
/// `slug` crate, which matches Ghost's downsize behaviour closely enough for
/// latin titles; other scripts fall back to pinyin-less ASCII stripping.
pub fn safe_string(input: &str) -> String {
    let s = input.trim();
    // slug::slugify handles spaces/punctuation and keeps unicode alphanumerics
    let slugged = slug::slugify(s);
    let cleaned = NON_ALNUM.replace_all(&slugged, "-");
    let deduped = MULTI_DASH.replace_all(&cleaned, "-");
    deduped.trim_matches('-').to_string()
}

/// Generate a unique slug for `model` (table name) from `base`.
///
/// `exists` must report whether some row already uses the slug; callers
/// editing an existing row must exclude that row's own id inside `exists`.
/// Posts are unique per (slug, type) — caller handles that by passing
/// a model string like "posts:page".
pub fn generate_slug(model: &str, base: &str, exists: &mut dyn FnMut(&str) -> bool) -> String {
    let mut slug = safe_string(base);
    if slug.len() > 185 {
        slug.truncate(185);
        slug = slug.trim_matches('-').to_string();
    }

    let mut all_protected: Vec<&str> = PROTECTED_SLUGS.to_vec();
    all_protected.extend(protected_for(model));

    if slug.is_empty() {
        slug = format!("{}-{}", model.split(':').next().unwrap_or(model), "1");
    } else if all_protected.contains(&slug.as_str()) {
        let model_name = model.split(':').next().unwrap_or(model);
        let singular = model_name.trim_end_matches('s');
        slug = format!("{}-{}", slug, singular);
    }

    if !exists(&slug) {
        return slug;
    }

    // uniquify: -2, -3, ...
    let mut i = 2u32;
    loop {
        let candidate = format!("{}-{}", slug, i);
        if !exists(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_titles() {
        assert_eq!(safe_string("Hello, World!"), "hello-world");
        assert_eq!(safe_string("  My  Fancy   Post  "), "my-fancy-post");
    }

    #[test]
    fn protected_slug_gets_suffix() {
        let slug = generate_slug("tags", "rss", &mut |_| false);
        assert_eq!(slug, "rss-tag");
    }

    #[test]
    fn uniquifies_with_counter() {
        let mut taken = 2;
        let slug = generate_slug("posts", "my post", &mut |s| {
            if s == "my-post" || (s == "my-post-2" && taken > 0) {
                taken -= 1;
                true
            } else {
                false
            }
        });
        assert_eq!(slug, "my-post-3");
    }

    #[test]
    fn empty_base_falls_back() {
        let slug = generate_slug("posts", "!!!", &mut |_| false);
        assert!(!slug.is_empty());
    }
}
