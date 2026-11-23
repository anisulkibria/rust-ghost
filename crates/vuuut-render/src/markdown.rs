//! Markdown pipeline (comrak, GFM) — Ghost's md → html/plaintext/reading_time.
use comrak::{html::collect_text, markdown_to_html, parse_document, Options};

fn options() -> Options<'static> {
    let mut opts = Options::default();
    opts.extension.autolink = true;
    opts.extension.table = true;
    opts.extension.strikethrough = true;
    opts.extension.tasklist = true;
    opts.extension.footnotes = true;
    opts.render.unsafe_ = true; // Ghost content trusts authors' raw HTML
    opts
}

/// Render markdown to HTML (GFM + raw HTML).
pub fn to_html(md: &str) -> String {
    markdown_to_html(md, &options())
}

/// Markdown as a Koenig markdown *card* renders slightly differently from
/// markdown as a whole post: Ghost runs it through markdown-it with soft
/// breaks on and an anchor id on every heading. comrak is not markdown-it,
/// so exotic input can still diverge, but the shape matches.
pub fn to_card_html(md: &str) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let mut opts = options();
    opts.render.hardbreaks = true;
    opts.extension.superscript = true;
    // comrak's own `header_ids` nests an anchor element inside the heading;
    // Ghost puts a plain `id` on the heading itself, so the attribute is
    // added afterwards instead.
    add_heading_ids(&markdown_to_html(md, &opts))
}

/// Give every heading the anchor id Ghost's markdown renderer would.
fn add_heading_ids(html: &str) -> String {
    let mut used = std::collections::HashMap::new();
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<h") {
        let after_h = &rest[start + 2..];
        let level = after_h.chars().next().filter(|c| ('1'..='6').contains(c));
        let (Some(level), Some(close)) = (level, after_h.find('>')) else {
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        };
        let open_end = start + 2 + close;
        let end_tag = format!("</h{level}>");
        let Some(text_end) = rest[open_end + 1..].find(&end_tag) else {
            out.push_str(&rest[..open_end + 1]);
            rest = &rest[open_end + 1..];
            continue;
        };
        let inner = &rest[open_end + 1..open_end + 1 + text_end];
        out.push_str(&rest[..open_end]);
        out.push_str(&format!(r#" id="{}">"#, crate::lexical::heading_anchor(&strip_tags(inner), &mut used)));
        out.push_str(inner);
        rest = &rest[open_end + 1 + text_end..];
    }
    out.push_str(rest);
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Plain-text rendering (for plaintext column + excerpts).
pub fn to_plaintext(md: &str) -> String {
    let arena = comrak::Arena::new();
    let root = parse_document(&arena, md, &options());
    collect_text(root)
}

/// Plain text from rendered HTML, for content that never passed through
/// markdown — a Koenig document arrives as HTML, and the `plaintext` column,
/// excerpts and reading time all still need words.
///
/// Tags are dropped, `<script>`/`<style>` bodies with them, and block
/// boundaries become whitespace so words either side do not run together.
pub fn html_to_plaintext(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        // A tag name tells us whether to skip the element's contents too.
        let name: String = after.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let name = name.to_ascii_lowercase();
        let Some(end) = after.find('>') else { break };
        if matches!(name.as_str(), "script" | "style") && !after.starts_with('/') {
            let close = format!("</{name}");
            rest = match after[end + 1..].find(&close) {
                Some(i) => &after[end + 1 + i..],
                None => "",
            };
            continue;
        }
        // Inline tags join their neighbours; everything else is a break.
        if !matches!(name.as_str(), "a" | "b" | "strong" | "i" | "em" | "u" | "s" | "code" | "span" | "mark" | "sub" | "sup" | "") {
            out.push(' ');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ghost's reading time: words / 213, minimum 1.
pub fn reading_time(plaintext: &str) -> i64 {
    let words = plaintext.split_whitespace().count() as i64;
    ((words + 212) / 213).max(1)
}

/// Derive (html, plaintext, reading_time) from markdown input.
pub fn render_markdown(md: &str) -> (String, String, i64) {
    let html = to_html(md);
    let plaintext = to_plaintext(md);
    let rt = reading_time(&plaintext);
    (html, plaintext, rt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_html() {
        let html = to_html("# Title\n\nHello **world**");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>world</strong>"));
    }

    #[test]
    fn plain_and_reading_time() {
        let (_, plain, rt) = render_markdown(&format!("word {}", "w ".repeat(300)));
        assert!(rt >= 2);
        assert!(plain.contains("word"));
    }

    #[test]
    fn plaintext_from_html() {
        assert_eq!(html_to_plaintext("<p>one</p><p>two</p>"), "one two");
        assert_eq!(html_to_plaintext("<p>a <strong>bold</strong> word</p>"), "a bold word");
        assert_eq!(html_to_plaintext("<p>hi</p><script>var x = 1 < 2;</script>"), "hi");
        assert_eq!(html_to_plaintext("<p>Tom &amp; Jerry</p>"), "Tom & Jerry");
    }

    #[test]
    fn raw_html_passes() {
        let html = to_html("<div class=\"x\">hi</div>");
        assert!(html.contains("<div class=\"x\">"));
    }
}
