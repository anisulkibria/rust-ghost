//! Port of `@tryghost/social-urls` plus the canonical platform table that
//! `{{social_url}}` / `{{#social_accounts}}` iterate.
//!
//! Ghost stores a *handle*, not a URL, for every platform except mastodon
//! (which stores a hostname-qualified handle without the scheme). Each
//! builder below mirrors the matching function in social-urls/lib/index.js.

/// Theme-facing type, the settings/schema field it reads, and the display
/// name — the canonical order is the admin settings UI's SOCIAL_PLATFORM_KEYS.
///
/// `x` is the only entry whose type differs from its source key: the DB
/// column is still `twitter`.
pub const SOCIAL_PLATFORMS: &[(&str, &str, &str)] = &[
    // (type, source_key, name)
    ("x", "twitter", "X"),
    ("facebook", "facebook", "Facebook"),
    ("linkedin", "linkedin", "LinkedIn"),
    ("bluesky", "bluesky", "Bluesky"),
    ("threads", "threads", "Threads"),
    ("mastodon", "mastodon", "Mastodon"),
    ("tiktok", "tiktok", "TikTok"),
    ("youtube", "youtube", "YouTube"),
    ("instagram", "instagram", "Instagram"),
];

/// Build the profile URL for `platform` from a stored handle.
///
/// `platform` accepts both the theme-facing type and the source key, so
/// `{{social_url type="twitter"}}` and `type="x"` both resolve.
pub fn url_for(platform: &str, username: &str) -> Option<String> {
    let u = username;
    // Ghost's admin normalises input down to a handle before storing, so it
    // never passes a full URL here. Ours may not have, and prepending a
    // prefix to one would produce a doubled, broken link.
    if u.starts_with("http://") || u.starts_with("https://") {
        return Some(u.to_string());
    }
    Some(match platform {
        "twitter" | "x" => format!("https://x.com/{}", u.trim_start_matches('@')),
        "facebook" => format!("https://www.facebook.com/{}", u.trim_start_matches('/')),
        "threads" => format!("https://www.threads.net/@{}", u.trim_start_matches('@')),
        "bluesky" => format!("https://bsky.app/profile/{u}"),
        // mastodon handles carry their own host, so only the scheme is added
        "mastodon" => format!("https://{u}"),
        "tiktok" => {
            if u.starts_with('@') {
                format!("https://www.tiktok.com/{u}")
            } else {
                format!("https://www.tiktok.com/@{u}")
            }
        }
        "youtube" => format!("https://www.youtube.com/{u}"),
        "instagram" => format!("https://www.instagram.com/{u}"),
        "linkedin" => {
            // a bare handle is a personal profile; the other path types are
            // kept as written
            let known = ["in/", "company/", "school/", "pub/"];
            if known.iter().any(|p| u.starts_with(p)) {
                format!("https://www.linkedin.com/{u}")
            } else {
                format!("https://www.linkedin.com/in/{u}")
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_each_platform_url() {
        let cases = [
            ("twitter", "@ghost", "https://x.com/ghost"),
            ("x", "ghost", "https://x.com/ghost"),
            ("facebook", "/ghost", "https://www.facebook.com/ghost"),
            ("threads", "@ghost", "https://www.threads.net/@ghost"),
            ("bluesky", "ghost.org", "https://bsky.app/profile/ghost.org"),
            ("mastodon", "mastodon.social/@ghost", "https://mastodon.social/@ghost"),
            ("tiktok", "ghost", "https://www.tiktok.com/@ghost"),
            ("tiktok", "@ghost", "https://www.tiktok.com/@ghost"),
            ("youtube", "@ghost", "https://www.youtube.com/@ghost"),
            ("instagram", "ghost", "https://www.instagram.com/ghost"),
            ("linkedin", "ghost", "https://www.linkedin.com/in/ghost"),
            ("linkedin", "company/ghost", "https://www.linkedin.com/company/ghost"),
        ];
        for (platform, handle, want) in cases {
            assert_eq!(url_for(platform, handle).as_deref(), Some(want), "{platform}/{handle}");
        }
    }

    #[test]
    fn unknown_platform_has_no_url() {
        assert_eq!(url_for("myspace", "ghost"), None);
    }
}
