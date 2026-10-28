//! `/llms.txt` — the public index AI tooling reads.
//!
//! Ghost's `frontend/services/llms` (6.72), gated on Settings → Meta data's
//! "Enable structured data for LLMs and AI search engines". The same switch
//! turns off the `.md` suffix on post URLs, because the file's whole premise
//! is that those URLs are fetchable.
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;
use ghost_db::repos;

/// Ghost's own wording, kept so the file reads the same to a crawler.
const INTRO: &str = "Public Ghost content for AI and LLM tooling.";
const MARKDOWN_DISCOVERABILITY: &str =
    "Append `.md` to any post or page URL to get the content in Markdown (for example, `/example-post.md`).";
const EMPTY_SECTION: &str = "_No public content available._";

/// Ghost caps the index at 50 KiB so a crawler is not handed an unbounded
/// file; posts are trimmed to fit once the fixed sections are counted.
const BUDGET_BYTES: usize = 50 * 1024;
/// Ghost lists at most this many pages.
const PAGES_LIMIT: usize = 20;
/// How much of a description to keep on one line.
const DESCRIPTION_CHARS: usize = 200;

/// Whether the LLM surfaces are published.
///
/// On unless explicitly `false`, matching Ghost, and never on a private site —
/// a file that indexes every post would undo the access code.
pub fn enabled(state: &AppState) -> bool {
    if crate::private::is_private(state) {
        return false;
    }
    !matches!(state.settings.get("llms_enabled"), Some(serde_json::Value::Bool(false)))
        && state.settings.get_str("llms_enabled").as_deref() != Some("false")
}

/// One line per entry, as Ghost writes it: a Markdown link to the `.md` URL,
/// and the excerpt after a dash when there is one.
fn index_line(title: &str, url: &str, description: &str) -> String {
    let link = format!("- [{}]({})", title.replace(']', ""), markdown_url(url));
    if description.is_empty() {
        link
    } else {
        format!("{link} - {description}")
    }
}

/// `/my-post/` → `/my-post.md`. Ghost appends the suffix to the path, so the
/// trailing slash goes first.
fn markdown_url(url: &str) -> String {
    format!("{}.md", url.trim_end_matches('/'))
}

/// Collapse an excerpt to one bounded line — newlines would break the list.
fn one_line(raw: &str) -> String {
    let flat: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= DESCRIPTION_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(DESCRIPTION_CHARS).collect();
    // Break on a word so the line does not end mid-word.
    match cut.rfind(' ') {
        Some(i) => format!("{}…", &cut[..i]),
        None => format!("{cut}…"),
    }
}

pub async fn llms_txt(State(state): State<AppState>) -> Response {
    if !enabled(&state) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let site_url = state.site_url();
    let origin = site_url.trim_end_matches('/').to_string();
    let title = state
        .settings
        .get_str("title")
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| origin.clone());
    let description = state
        .settings
        .get_str("meta_description")
        .filter(|d| !d.is_empty())
        .or_else(|| state.settings.get_str("description").filter(|d| !d.is_empty()))
        .unwrap_or_else(|| "Public Ghost publication content.".into());

    let header_block = format!("# {title}\n> {description}");

    let entries = |rows: &[ghost_core::models::Post]| -> Vec<String> {
        rows.iter()
            .map(|p| {
                let url = format!("{origin}/{}/", p.slug.clone().unwrap_or_default());
                let desc = p
                    .custom_excerpt
                    .clone()
                    .filter(|e| !e.is_empty())
                    .or_else(|| Some(ghost_render::markdown::html_to_plaintext(&ghost_api::content::public_html(p))))
                    .unwrap_or_default();
                index_line(p.title.as_deref().unwrap_or("Untitled"), &url, &one_line(&desc))
            })
            .collect()
    };

    let browse = |post_type: &'static str, limit: i64| {
        let db = state.db.clone();
        async move {
            let q = repos::posts::BrowsePosts {
                filter: None,
                order: Some("published_at desc".into()),
                limit,
                page: 1,
                post_type: Some(post_type.into()),
                public: true,
                include_authors: false,
                include_tags: false,
            };
            repos::posts::browse(&db, &q).await.map(|r| r.posts).unwrap_or_default()
        }
    };

    let pages = browse("page", PAGES_LIMIT as i64).await;
    // Fetched generously and then trimmed to the byte budget below, the way
    // Ghost spends whatever the fixed sections leave over.
    let posts = browse("post", 500).await;

    let pages_section = {
        let lines = entries(&pages);
        if lines.is_empty() { EMPTY_SECTION.to_string() } else { lines.join("\n") }
    };

    let optional = {
        let mut links = vec![format!("- [RSS Feed]({origin}/rss/)")];
        links.push(format!("- [Sitemap]({origin}/sitemap.xml)"));
        links.join("\n")
    };

    let scaffold = |posts_section: &str| {
        format!(
            "{header_block}\n{INTRO}\n{MARKDOWN_DISCOVERABILITY}\n\n## Pages\n{pages_section}\n\n## Posts\n{posts_section}\n\n## Optional\n{optional}"
        )
    };

    // Spend the remaining budget on posts, a whole line at a time.
    let fixed = scaffold("").len();
    let mut used = fixed;
    let mut post_lines: Vec<String> = Vec::new();
    for line in entries(&posts) {
        let cost = line.len() + 1;
        if used + cost > BUDGET_BYTES {
            break;
        }
        used += cost;
        post_lines.push(line);
    }
    let posts_section =
        if post_lines.is_empty() { EMPTY_SECTION.to_string() } else { post_lines.join("\n") };

    let body = format!("{}\n", scaffold(&posts_section).trim_end());
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=UTF-8"),
            (header::CACHE_CONTROL, "public, max-age=900"),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_markdown_url_replaces_the_trailing_slash() {
        assert_eq!(markdown_url("https://x.test/my-post/"), "https://x.test/my-post.md");
        assert_eq!(markdown_url("https://x.test/my-post"), "https://x.test/my-post.md");
    }

    #[test]
    fn an_index_line_reads_as_ghost_writes_it() {
        assert_eq!(
            index_line("Hello", "https://x.test/hello/", "About things"),
            "- [Hello](https://x.test/hello.md) - About things"
        );
        // no excerpt, no dash
        assert_eq!(
            index_line("Hello", "https://x.test/hello/", ""),
            "- [Hello](https://x.test/hello.md)"
        );
    }

    #[test]
    fn descriptions_are_flattened_and_bounded() {
        assert_eq!(one_line("one\ntwo   three"), "one two three");
        let long = "word ".repeat(100);
        let out = one_line(&long);
        assert!(out.chars().count() <= DESCRIPTION_CHARS + 1, "got {}", out.chars().count());
        assert!(out.ends_with('…'));
        // and it does not break mid-word
        assert!(!out.trim_end_matches('…').ends_with("wor"));
    }
}
