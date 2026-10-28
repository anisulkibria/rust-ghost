//! Comment domain rules: what HTML a comment may contain, and what a
//! comment reduces to as plain text.
//!
//! Ghost runs every comment body through `sanitize-html` in
//! `core/server/models/comment.js` before it is stored, with a deliberately
//! tiny allow-list — four tags, and links forced to open in a new tab with
//! `rel="ugc noopener noreferrer nofollow"`. The comment form is a rich-text
//! editor that only produces those tags anyway; the sanitizer is there
//! because anything that can speak HTTP can POST to the comments endpoint,
//! so the stored HTML is attacker-controlled until this function has run.
//!
//! The plain-text conversions mirror `@tryghost/html-to-plaintext`'s
//! `comment` and `commentSnippet` profiles. `commentSnippet` is what the API
//! exposes as `in_reply_to_snippet`, so its output is visitor-facing.

use crate::{GhostError, Result};

/// Ghost's `allowedTags` for comment HTML.
const ALLOWED_TAGS: &[&str] = &["p", "br", "a", "blockquote"];

/// `sanitize-html`'s `nonTextTags` default: these are dropped *with* their
/// contents, rather than being unwrapped down to their text. Without this a
/// `<script>alert(1)</script>` would survive as the literal text `alert(1)`
/// — harmless, but noise nobody meant to publish.
const NON_TEXT_TAGS: &[&str] = &["script", "style", "textarea", "option"];

/// `sanitize-html`'s `allowedSchemes` default. A `javascript:` or `data:`
/// href fails this and the attribute is dropped, leaving an inert `<a>`.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "ftp", "mailto", "tel"];

/// The `rel` Ghost forces onto every comment link. `ugc` marks it as
/// user-generated, and the rest stop a comment from passing authority or
/// handing the linked page a reference to this one.
const LINK_REL: &str = "ugc noopener noreferrer nofollow";

// ---------------------------------------------------------------------------
// tokenising
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Text(&'a str),
    /// `<name attrs>` — `self_closing` is the trailing `/`.
    Open { name: String, attrs: &'a str, self_closing: bool },
    Close { name: String },
    /// Comments, doctypes and processing instructions, which are discarded.
    Ignored,
}

/// Walk `html` as a flat token stream.
///
/// Deliberately not a full HTML5 parser: the only shapes that need to come
/// out intact are the four allowed tags, and everything else is either
/// unwrapped or dropped. What matters is that no `<` or `>` can ever reach
/// the output except as one this function emitted — which is why an
/// unterminated `<` at the end of the input is treated as text and escaped
/// rather than hopefully skipped.
fn tokenize(html: &str) -> Vec<Token<'_>> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // `<` not followed by a tag name, `/` or `!` is literal text in
        // HTML5 ("a < b"), so leave it in the text run.
        let next = bytes.get(i + 1).copied();
        let starts_tag = matches!(next, Some(c) if c.is_ascii_alphabetic() || c == b'/' || c == b'!' || c == b'?');
        if !starts_tag {
            i += 1;
            continue;
        }

        // `<!-- … -->` may contain `>`, so it ends on the comment close.
        let (end, is_comment) = if html[i..].starts_with("<!--") {
            match html[i + 4..].find("-->") {
                Some(p) => (i + 4 + p + 3, true),
                // an unclosed comment swallows the rest of the document
                None => (bytes.len(), true),
            }
        } else {
            match html[i..].find('>') {
                Some(p) => (i + p + 1, false),
                // an unclosed tag is not a tag: keep it as text so it gets
                // escaped instead of silently truncating the comment
                None => {
                    i += 1;
                    continue;
                }
            }
        };

        if text_start < i {
            out.push(Token::Text(&html[text_start..i]));
        }

        if is_comment || matches!(next, Some(b'!') | Some(b'?')) {
            out.push(Token::Ignored);
        } else {
            let inner = &html[i + 1..end - 1];
            let inner = inner.strip_suffix('/').unwrap_or(inner);
            let self_closing = html[i + 1..end - 1].ends_with('/');
            match inner.strip_prefix('/') {
                Some(close) => out.push(Token::Close { name: tag_name(close) }),
                None => {
                    let name = tag_name(inner);
                    let attrs = inner.get(name.len()..).unwrap_or("");
                    out.push(Token::Open { name, attrs, self_closing });
                }
            }
        }

        i = end;
        text_start = end;
    }

    if text_start < html.len() {
        out.push(Token::Text(&html[text_start..]));
    }
    out
}

fn tag_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Parse `name="value"` / `name='value'` / `name=value` / `name` pairs out of
/// a tag's attribute text.
fn attributes(attrs: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = attrs.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        let name = attrs[start..i].to_ascii_lowercase();
        // skip whitespace before a possible `=`
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let vstart = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                value = attrs[vstart..i].to_string();
                i = (i + 1).min(b.len());
            } else {
                let vstart = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = attrs[vstart..i].to_string();
            }
        }
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// entities
// ---------------------------------------------------------------------------

/// `sanitize-html`'s `escapeHtml`: `&` is only escaped when it does not
/// already open an entity, so text that arrives correctly escaped is not
/// double-escaped on the way back out.
fn escape_text(s: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(['&', '<', '>', '"']) {
        out.push_str(&rest[..pos]);
        let c = rest.as_bytes()[pos];
        rest = &rest[pos + 1..];
        match c {
            b'&' if looks_like_entity(rest) => out.push('&'),
            b'&' => out.push_str("&amp;"),
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            b'"' if in_attribute => out.push_str("&quot;"),
            b'"' => out.push('"'),
            _ => {}
        }
    }
    out.push_str(rest);
    out
}

/// Whether what follows an `&` is an entity reference — `[a-zA-Z0-9#]{1,20};`,
/// the same lookahead `sanitize-html` uses.
fn looks_like_entity(after_amp: &str) -> bool {
    for (n, c) in after_amp.chars().enumerate() {
        if c == ';' {
            return n > 0;
        }
        if !(c.is_ascii_alphanumeric() || c == '#') || n == 20 {
            return false;
        }
    }
    false
}

/// Decode the entities a comment body can realistically carry. Used only on
/// the plain-text paths — the sanitizer leaves entities alone, because
/// re-encoding them is `escape_text`'s job.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let Some(semi) = rest[1..].find(';').map(|p| p + 1).filter(|p| *p <= 21) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "ldquo" => Some('\u{201c}'),
            "rdquo" => Some('\u{201d}'),
            "bull" => Some('•'),
            other => other
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// sanitising
// ---------------------------------------------------------------------------

/// Validate and normalise the HTML body of a comment, exactly as Ghost's
/// `Comment.onSaving` does: sanitize, trim empty paragraphs, reject empty.
///
/// Returns a [`GhostError::ValidationError`] carrying Ghost's own message
/// when nothing survives, because that string reaches the commenter.
pub fn prepare_html(html: &str) -> Result<String> {
    let cleaned = trim_paragraphs(&sanitize_html(html)).trim().to_string();
    if cleaned.is_empty() {
        return Err(GhostError::ValidationError {
            message: "The body of a comment cannot be empty".into(),
            context: None,
        });
    }
    Ok(cleaned)
}

/// Ghost's `sanitize-html` configuration for comments.
pub fn sanitize_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    // tags whose contents are being discarded, innermost last
    let mut suppress: Vec<String> = Vec::new();
    // allowed tags currently open, so a stray `</p>` cannot close nothing
    let mut open: Vec<String> = Vec::new();

    for token in tokenize(html) {
        match token {
            Token::Ignored => {}
            Token::Text(t) => {
                if suppress.is_empty() {
                    out.push_str(&escape_text(t, false));
                }
            }
            Token::Open { name, attrs, self_closing } => {
                if !suppress.is_empty() {
                    // a nested non-text tag has to be popped by its own
                    // close, so track it even while suppressing
                    if NON_TEXT_TAGS.contains(&name.as_str()) && !self_closing {
                        suppress.push(name);
                    }
                    continue;
                }
                if NON_TEXT_TAGS.contains(&name.as_str()) {
                    if !self_closing {
                        suppress.push(name);
                    }
                    continue;
                }
                if !ALLOWED_TAGS.contains(&name.as_str()) {
                    // unwrapped: the tag goes, the contents stay
                    continue;
                }
                out.push_str(&open_tag(&name, attrs));
                if name != "br" && !self_closing {
                    open.push(name);
                }
            }
            Token::Close { name } => {
                if let Some(pos) = suppress.iter().rposition(|n| *n == name) {
                    suppress.truncate(pos);
                    continue;
                }
                if !suppress.is_empty() || name == "br" {
                    continue;
                }
                if let Some(pos) = open.iter().rposition(|n| *n == name) {
                    // anything left open inside is closed too, which is what
                    // a browser's parser would do
                    for inner in open.split_off(pos).into_iter().rev() {
                        out.push_str(&format!("</{inner}>"));
                    }
                }
            }
        }
    }

    for name in open.into_iter().rev() {
        out.push_str(&format!("</{name}>"));
    }
    out
}

/// Render one allowed open tag with its surviving attributes.
///
/// Only `<a>` keeps any, and Ghost's `simpleTransform` overwrites `target`
/// and `rel` regardless of what was sent — so a comment can never ask a link
/// to open in place, or to pass PageRank.
fn open_tag(name: &str, attrs: &str) -> String {
    if name == "br" {
        return "<br />".into();
    }
    if name != "a" {
        return format!("<{name}>");
    }
    let href = attributes(attrs)
        .into_iter()
        .find(|(k, _)| k == "href")
        .map(|(_, v)| v)
        .filter(|v| scheme_allowed(v));
    match href {
        Some(href) => format!(
            r#"<a href="{}" target="_blank" rel="{LINK_REL}">"#,
            escape_text(href.trim(), true)
        ),
        None => format!(r#"<a target="_blank" rel="{LINK_REL}">"#),
    }
}

/// `sanitize-html`'s URL check: a relative or protocol-relative URL is fine,
/// an absolute one must use an allowed scheme.
fn scheme_allowed(url: &str) -> bool {
    let url = decode_entities(url.trim());
    // control characters are how `java\nscript:` smuggles a scheme past a
    // naive prefix test
    let stripped: String = url.chars().filter(|c| !c.is_control() && *c != '\u{feff}').collect();
    if stripped.starts_with("//") {
        return true; // allowProtocolRelative, on by default
    }
    match stripped.split_once(':') {
        // a `:` after a `/`, `?` or `#` is part of a path, not a scheme
        Some((scheme, _))
            if !scheme.contains('/') && !scheme.contains('?') && !scheme.contains('#') =>
        {
            ALLOWED_SCHEMES.contains(&scheme.trim().to_ascii_lowercase().as_str())
        }
        _ => true,
    }
}

/// Ghost's `trimParagraphs`: drop empty paragraphs from each end, then
/// collapse the first run of them that remains down to a single blank line.
///
/// The one-run-only behaviour is not an oversight to fix: Ghost's `replace`
/// has no `g` flag, and this port matches what Ghost stores.
fn trim_paragraphs(s: &str) -> String {
    let runs = empty_paragraph_runs(s);
    let mut s = s.to_string();

    // trailing run first, so the leading removal does not shift its offsets
    if let Some((start, end)) = runs.last().copied().filter(|(_, end)| *end == s.len()) {
        s.replace_range(start..end, "");
    }
    let runs = empty_paragraph_runs(&s);
    if let Some((start, end)) = runs.first().copied().filter(|(start, _)| *start == 0) {
        s.replace_range(start..end, "");
    }
    let runs = empty_paragraph_runs(&s);
    if let Some((start, end)) = runs.first().copied() {
        s.replace_range(start..end, "<p></p>");
    }
    s
}

/// Byte spans of every maximal run of `<p>`-whitespace-`</p>`.
fn empty_paragraph_runs(s: &str) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while let Some(rel) = s[i..].find("<p>") {
        let start = i + rel;
        let mut j = start + 3;
        while s[j..].starts_with(char::is_whitespace) {
            j += s[j..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
        if !s[j..].starts_with("</p>") {
            i = start + 3;
            continue;
        }
        let end = j + 4;
        match runs.last_mut() {
            Some(last) if last.1 == start => last.1 = end,
            _ => runs.push((start, end)),
        }
        i = end;
    }
    runs
}

// ---------------------------------------------------------------------------
// plain text
// ---------------------------------------------------------------------------

/// `htmlToPlaintext.commentSnippet` — what the API exposes as
/// `in_reply_to_snippet`.
///
/// Blockquotes are skipped entirely: a reply that quotes the comment it is
/// answering would otherwise produce a snippet of that other comment.
pub fn snippet(html: &str) -> String {
    let mut out = String::new();
    let mut suppress = 0usize;
    for token in tokenize(html) {
        match token {
            Token::Ignored => {}
            Token::Text(t) if suppress == 0 => out.push_str(&decode_entities(t)),
            Token::Text(_) => {}
            Token::Open { name, self_closing, .. } => {
                if name == "blockquote" && !self_closing {
                    suppress += 1;
                } else if suppress == 0 && is_block_break(&name) {
                    out.push(' ');
                }
            }
            Token::Close { name } => {
                if name == "blockquote" {
                    suppress = suppress.saturating_sub(1);
                } else if suppress == 0 && is_block_break(&name) {
                    out.push(' ');
                }
            }
        }
    }
    collapse_whitespace(&out)
}

/// `htmlToPlaintext.comment` — the plain-text alternative for the report
/// email, where blockquotes are kept and marked with `>` and links carry
/// their href in brackets.
pub fn to_plaintext(html: &str) -> String {
    let mut out = String::new();
    // href of the link currently open, to append after its text
    let mut link: Vec<String> = Vec::new();
    let mut quote_depth = 0usize;

    for token in tokenize(html) {
        match token {
            Token::Ignored => {}
            Token::Text(t) => out.push_str(&decode_entities(t)),
            Token::Open { ref name, attrs, self_closing } => match name.as_str() {
                "br" => out.push('\n'),
                "p" => push_break(&mut out, 1),
                "blockquote" => {
                    push_break(&mut out, 1);
                    if !self_closing {
                        quote_depth += 1;
                    }
                }
                "a" => link.push(
                    attributes(attrs)
                        .into_iter()
                        .find(|(k, _)| k == "href")
                        .map(|(_, v)| decode_entities(&v))
                        .unwrap_or_default(),
                ),
                _ => {}
            },
            Token::Close { ref name } => match name.as_str() {
                "p" => push_break(&mut out, 1),
                "blockquote" => {
                    quote_depth = quote_depth.saturating_sub(1);
                    push_break(&mut out, 2);
                }
                "a" => {
                    if let Some(href) = link.pop() {
                        // `hideLinkHrefIfSameAsText`: a bare URL as its own
                        // text does not get repeated in brackets
                        let text_is_href = out.trim_end().ends_with(href.as_str());
                        if !href.is_empty() && !text_is_href {
                            out.push_str(&format!(" [{href}]"));
                        }
                    }
                }
                _ => {}
            },
        }
        // mark quoted lines as they are produced
        if quote_depth > 0 {
            if let Some(stripped) = out.strip_suffix('\n') {
                let mut s = stripped.to_string();
                s.push('\n');
                out = s;
            }
        }
    }

    // apply the `> ` prefix to blockquote lines in a second pass: doing it
    // inline would have to guess where a line starts before its text arrives
    let marked = mark_blockquotes(html, &out);
    marked.trim().to_string()
}

/// Prefix the lines that came from blockquote content with `> `.
fn mark_blockquotes(html: &str, text: &str) -> String {
    if !html.contains("<blockquote") {
        return text.to_string();
    }
    // the quoted text, flattened the same way the body was, is what to match
    let mut quoted: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut buf = String::new();
    for token in tokenize(html) {
        match token {
            Token::Open { ref name, self_closing, .. } if name == "blockquote" && !self_closing => {
                depth += 1
            }
            Token::Close { ref name } if name == "blockquote" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    for line in collapse_whitespace(&buf).lines() {
                        if !line.trim().is_empty() {
                            quoted.push(line.trim().to_string());
                        }
                    }
                    buf.clear();
                }
            }
            Token::Text(t) if depth > 0 => buf.push_str(&decode_entities(t)),
            Token::Open { ref name, .. } | Token::Close { ref name }
                if depth > 0 && is_block_break(name) =>
            {
                buf.push('\n')
            }
            _ => {}
        }
    }
    if quoted.is_empty() {
        return text.to_string();
    }
    text.lines()
        .map(|line| {
            if quoted.iter().any(|q| q == line.trim()) && !line.trim().is_empty() {
                format!("> {}", line.trim())
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_block_break(name: &str) -> bool {
    matches!(name, "p" | "br" | "blockquote" | "div" | "li" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// Append newlines without stacking more than `n` in a row.
fn push_break(out: &mut String, n: usize) {
    let existing = out.chars().rev().take_while(|c| *c == '\n').count();
    if out.trim().is_empty() {
        return;
    }
    for _ in existing..n {
        out.push('\n');
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// member access
// ---------------------------------------------------------------------------

/// Ghost's `contentGating.checkPostAccess`, which decides both whether a
/// member may read a post and — via the comments service — whether they may
/// comment on it.
///
/// Tier-restricted posts require explicit entitlements; callers without them
/// fail closed. Use `check_post_tier_access` when the tier lists are available.
pub fn check_post_access(visibility: &str, member_status: Option<&str>) -> bool {
    if visibility == "public" {
        return true;
    }
    let Some(status) = member_status else {
        return false;
    };
    match visibility {
        "members" => true,
        "paid" => matches!(status, "paid" | "comped" | "gift"),
        _ => false,
    }
}

pub fn check_post_tier_access(visibility: &str, member_status: Option<&str>, post_tiers: &[String], member_tiers: &[String]) -> bool {
    if visibility != "tiers" { return check_post_access(visibility, member_status); }
    check_post_access("paid", member_status) && post_tiers.iter().any(|id| member_tiers.contains(id))
}

/// The initials Ghost puts in the avatar bubble of a notification email:
/// first and last initial for a multi-word name, otherwise just the first.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(' ').filter(|w| !w.is_empty()).collect();
    let picks: Vec<&str> = match words.len() {
        0 => vec![],
        1 => vec![words[0]],
        n => vec![words[0], words[n - 1]],
    };
    picks
        .iter()
        .filter_map(|w| w.chars().next())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_allowed_tags() {
        assert_eq!(sanitize_html("<p>hi</p>"), "<p>hi</p>");
        assert_eq!(sanitize_html("<blockquote><p>q</p></blockquote>"), "<blockquote><p>q</p></blockquote>");
        assert_eq!(sanitize_html("a<br>b"), "a<br />b");
    }

    #[test]
    fn unwraps_disallowed_tags_but_keeps_their_text() {
        assert_eq!(sanitize_html("<p><b>bold</b></p>"), "<p>bold</p>");
        assert_eq!(sanitize_html("<div>x</div>"), "x");
        assert_eq!(sanitize_html("<img src=x onerror=alert(1)>"), "");
    }

    #[test]
    fn drops_non_text_tags_with_their_contents() {
        assert_eq!(sanitize_html("<p>a</p><script>alert(1)</script>"), "<p>a</p>");
        assert_eq!(sanitize_html("<style>p{color:red}</style>ok"), "ok");
    }

    #[test]
    fn forces_target_and_rel_on_links() {
        assert_eq!(
            sanitize_html(r#"<a href="https://x.test" target="_self" rel="dofollow">x</a>"#),
            format!(r#"<a href="https://x.test" target="_blank" rel="{LINK_REL}">x</a>"#)
        );
    }

    #[test]
    fn drops_hrefs_with_a_disallowed_scheme() {
        for bad in [
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<a href="data:text/html,<script>">x</a>"#,
            "<a href=\"java\nscript:alert(1)\">x</a>",
        ] {
            let out = sanitize_html(bad);
            assert!(!out.contains("href="), "{bad} -> {out}");
        }
        assert!(sanitize_html(r#"<a href="/relative">x</a>"#).contains(r#"href="/relative""#));
        assert!(sanitize_html(r#"<a href="mailto:a@b.test">x</a>"#).contains("mailto:"));
        assert!(sanitize_html(r#"<a href="//cdn.test/x">x</a>"#).contains("//cdn.test/x"));
    }

    #[test]
    fn escapes_text_without_double_escaping_entities() {
        assert_eq!(sanitize_html("<p>a & b</p>"), "<p>a &amp; b</p>");
        assert_eq!(sanitize_html("<p>a &amp; b</p>"), "<p>a &amp; b</p>");
        assert_eq!(sanitize_html("<p>1 < 2</p>"), "<p>1 &lt; 2</p>");
        // an unterminated tag must not be able to escape as markup
        assert_eq!(sanitize_html("<p>a</p><x"), "<p>a</p>&lt;x");
    }

    #[test]
    fn closes_tags_left_open() {
        assert_eq!(sanitize_html("<p>a"), "<p>a</p>");
        assert_eq!(sanitize_html("<blockquote><p>a"), "<blockquote><p>a</p></blockquote>");
        // a close with no matching open is dropped rather than emitted
        assert_eq!(sanitize_html("a</p>"), "a");
    }

    #[test]
    fn trims_empty_paragraphs() {
        assert_eq!(trim_paragraphs("<p></p><p>a</p>"), "<p>a</p>");
        assert_eq!(trim_paragraphs("<p>a</p><p></p><p></p>"), "<p>a</p>");
        assert_eq!(trim_paragraphs("<p></p><p></p><p>a</p>"), "<p>a</p>");
        // an interior run collapses to one blank line
        assert_eq!(trim_paragraphs("<p>a</p><p></p><p></p><p>b</p>"), "<p>a</p><p></p><p>b</p>");
        assert_eq!(trim_paragraphs("<p>  </p><p>a</p>"), "<p>a</p>");
    }

    #[test]
    fn rejects_a_body_that_sanitises_to_nothing() {
        assert!(prepare_html("<p></p>").is_err());
        assert!(prepare_html("<script>alert(1)</script>").is_err());
        assert_eq!(prepare_html("  <p>hi</p> ").unwrap(), "<p>hi</p>");
    }

    /// Expectations taken from running `@tryghost/html-to-plaintext`
    /// against Ghost 6.39.
    #[test]
    fn snippets_match_ghost() {
        assert_eq!(snippet("<p>Hello world</p>"), "Hello world");
        assert_eq!(snippet("<p>One</p><p>Two</p>"), "One Two");
        assert_eq!(
            snippet(r#"<p>Link <a href="https://example.com" target="_blank">here</a> ok</p>"#),
            "Link here ok"
        );
        assert_eq!(snippet("<blockquote>quoted</blockquote><p>after</p>"), "after");
        assert_eq!(snippet("<p>Line<br>break</p>"), "Line break");
        assert_eq!(snippet("<p>  spaced   out  </p>"), "spaced out");
        assert_eq!(snippet("<p>&amp; &lt; &gt; &quot;</p>"), "& < > \"");
    }

    #[test]
    fn plaintext_matches_ghost() {
        assert_eq!(to_plaintext("<p>Hello world</p>"), "Hello world");
        assert_eq!(to_plaintext("<p>One</p><p>Two</p>"), "One\nTwo");
        assert_eq!(
            to_plaintext(r#"<p>Link <a href="https://example.com">here</a> ok</p>"#),
            "Link here [https://example.com] ok"
        );
        assert_eq!(to_plaintext("<blockquote>quoted</blockquote><p>after</p>"), "> quoted\n\nafter");
        assert_eq!(to_plaintext("<p>Line<br>break</p>"), "Line\nbreak");
    }

    #[test]
    fn gates_posts_the_way_ghost_does() {
        assert!(check_post_access("public", None));
        assert!(check_post_access("public", Some("free")));
        assert!(!check_post_access("members", None));
        assert!(check_post_access("members", Some("free")));
        assert!(!check_post_access("paid", Some("free")));
        assert!(check_post_access("paid", Some("comped")));
        assert!(check_post_access("paid", Some("paid")));
        assert!(!check_post_access("tiers", Some("free")));
    }

    #[test]
    fn initials_take_the_first_and_last_word() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("Ada"), "A");
        assert_eq!(initials("Ada B Lovelace"), "AL");
        assert_eq!(initials(""), "");
    }
}
