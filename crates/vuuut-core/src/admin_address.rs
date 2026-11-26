//! The configurable public address of the built-in admin.
pub const DEFAULT: &str = "vuuut";
pub const PATH_KEY: &str = "admin_path";
pub const REDIRECT_KEY: &str = "admin_default_redirect";

pub fn clean(value: &str) -> Option<&str> {
    let slug = value.trim().trim_matches('/');
    if slug.is_empty() || slug.len() > 64
        || !slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || slug.starts_with('-') || slug.ends_with('-')
        || RESERVED.contains(&slug)
    {
        return None;
    }
    Some(slug)
}

const RESERVED: &[&str] = &[
    "ghost", "shop", "directory", "courses", "book", "podcast", "portfolio", "members", "assets", "content", "public", "p", "private",
    "tag", "author", "rss", "page", "email", "webmentions", "unsubscribe", "r", "search",
    "forms", "cookies", "ads", "popups",
];

/// Strip a prefix only at a URL path boundary.
pub fn suffix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    path.strip_prefix(prefix).filter(|tail| tail.is_empty() || tail.starts_with(['/', '?', '#']))
}
