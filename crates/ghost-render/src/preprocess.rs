//! Template preprocessing.
//!
//! 1. Layout declaration: `{{!< default}}` at the top of a template names
//!    its layout (express-hbs semantics; default "default").
//! 2. `@site`/`@blog`/`@config`/`@member`/`@labs`/`@custom` are rewritten to
//!    `@root.<name>` — handlebars-rust only resolves `@root` from data, and
//!    its grammar accepts arbitrary `@foo` but cannot seed them via the
//!    public render API. Loop vars (`@index`, `@first`, …) are untouched.
//! 3. Quoted partial names (`{{> "icons/facebook"}}`) are unquoted.
use once_cell::sync::Lazy;
use regex::Regex;

static LAYOUT_DECL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)^\s*\{\{!\s*<\s*([A-Za-z0-9_-]+)\s*\}\}").unwrap());

static QUOTED_PARTIAL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(\{\{~?[#>]?>\s*)["']([^"'}]+)["']"#).unwrap());

/// Apply every source rewrite a theme template needs before compiling.
pub fn prepare(src: &str) -> String {
    let src = rewrite_partial_names(src);
    let src = rewrite_dynamic_partial_blocks(&src);
    rewrite_at_vars(&rewrite_inverted_sections(&src))
}

/// Byte spans of every `{{…}}` tag, with the text between the braces.
fn tags(src: &str) -> Vec<(usize, usize, &str)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"{{") {
            // long comments may contain `}}`, so they end at `--}}`
            let (close, close_len): (&[u8], usize) =
                if b[i..].starts_with(b"{{!--") { (b"--}}", 4) } else { (b"}}", 2) };
            let mut j = i + 2;
            let end = loop {
                if j >= b.len() {
                    return out;
                }
                if b[j..].starts_with(close) {
                    break j;
                }
                j += 1;
            };
            out.push((i, end + close_len, &src[i + 2..end]));
            i = end + close_len;
            continue;
        }
        i += 1;
    }
    out
}

/// handlebars.js allows a dynamic partial block to be closed by a name that
/// does not match its sub-expression open tag — Ghost's Casper and Source
/// write `{{#> (concat "icons/" type)}} … {{/undefined}}`. handlebars-rust
/// insists the names match, but accepts the nameless `{{/}}` close for any
/// block, so rewrite the matching close of each sub-expression partial block.
pub fn rewrite_dynamic_partial_blocks(src: &str) -> String {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    // one entry per open block: is it a sub-expression partial block?
    let mut stack: Vec<bool> = Vec::new();

    for (start, end, inner) in tags(src) {
        let tag = inner.trim_start_matches('~').trim_start();
        if let Some(name) = tag.strip_prefix("#>") {
            stack.push(name.trim_start().starts_with('('));
        } else if tag.starts_with('#') || tag.starts_with('^') {
            stack.push(false);
        } else if tag.starts_with('/') && stack.pop() == Some(true) {
            spans.push((start, end));
        }
    }

    if spans.is_empty() {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (start, end) in spans {
        out.push_str(&src[last..start]);
        out.push_str("{{/}}");
        last = end;
    }
    out.push_str(&src[last..]);
    out
}

/// Ghost themes quote partial paths — `{{> "icons/facebook"}}` — which
/// handlebars.js accepts but handlebars-rust's grammar rejects. The bare
/// form (`{{> icons/facebook}}`) parses and resolves identically.
pub fn rewrite_partial_names(src: &str) -> String {
    QUOTED_PARTIAL.replace_all(src, "${1}${2}").into_owned()
}

/// At-vars set by our own foreach/each (must NOT be rewritten).
const LOOP_VARS: &[&str] = &[
    "index", "first", "last", "key", "number", "even", "odd", "rowStart", "rowEnd", "root", "root.",
    "log", "data",
];

/// Names that get rewritten to @root.<name>.
const ROOT_VARS: &[&str] = &["site", "blog", "config", "member", "labs", "custom", "page", "context", "site.url"];

/// Extract `{{!< layout}}` (returns None when the template declares no
/// layout — the caller decides the default) and strip it from the source.
pub fn extract_layout(src: &str) -> (Option<String>, String) {
    if let Some(m) = LAYOUT_DECL.captures(src) {
        let name = m.get(1).map(|n| n.as_str().to_string()).unwrap_or_else(|| "default".into());
        let rest = src[m.get(0).unwrap().end()..].to_string();
        (Some(name), rest)
    } else {
        (None, src.to_string())
    }
}

/// Rewrite `{{^helper args}}` inverted sections into
/// `{{#helper args}}{{else}}` — handlebars.js supports `^` on any block
/// helper; handlebars-rust only supports `{{else}}`/`{{^}}` inside blocks.
pub fn rewrite_inverted_sections(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < src.len() {
        // match "{{^" followed by an identifier char
        if bytes[i..].starts_with(b"{{^") {
            let rest = &src[i + 3..];
            let name_len = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .count();
            if name_len > 0 {
                // find end of the opening tag
                if let Some(end) = src[i..].find("}}") {
                    let tag_end = i + end + 2;
                    let inner = &src[i + 3..i + end]; // name + args
                    // find matching {{/name}} — track nested same-name opens
                    let close = format!("{{{{/{}}}", &inner.split_whitespace().next().unwrap_or(""));
                    let open_name = &inner.split_whitespace().next().unwrap_or("").to_string();
                    let nested_open = format!("{{{{#{}}}", open_name);
                    let mut depth = 1;
                    let mut j = tag_end;
                    let mut close_pos = None;
                    // scan over bytes, not `src[j..]`: this advances one byte
                    // at a time, so `j` lands inside multi-byte characters
                    // and str slicing there would panic
                    while j < bytes.len() {
                        if bytes[j..].starts_with(nested_open.as_bytes()) {
                            depth += 1;
                            j += nested_open.len();
                        } else if bytes[j..].starts_with(close.as_bytes()) {
                            depth -= 1;
                            if depth == 0 {
                                close_pos = Some(j);
                                break;
                            }
                            j += close.len();
                        } else {
                            j += 1;
                        }
                    }
                    if close_pos.is_some() {
                        // {{^name args}} → {{#name args}}{{else}}
                        out.push_str("{{#");
                        out.push_str(inner);
                        out.push_str("}}{{else}}");
                        i = tag_end;
                        continue;
                    }
                }
            }
        }
        // copy up to next {{
        match src[i..].find("{{") {
            Some(next) if next > 0 => {
                out.push_str(&src[i..i + next]);
                i += next;
            }
            Some(_) => {
                out.push_str("{{");
                i += 2;
            }
            None => {
                out.push_str(&src[i..]);
                break;
            }
        }
    }
    out
}
pub fn rewrite_at_vars(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let bytes = src.as_bytes();
    while i < src.len() {
        if bytes[i] == b'{' && i + 1 < src.len() && bytes[i + 1] == b'{' {
            // comment start?
            if src[i..].starts_with("{{!") {
                // copy through to closing }}
                if let Some(end) = src[i..].find("}}") {
                    let _seg = &src[i..i + end + 2];
                    // long comments {{!-- --}} may contain }} inside; find real end
                    let seg_end = if src[i..].starts_with("{{!--") {
                        src[i..].find("--}}").map(|e| i + e + 4)
                    } else {
                        Some(i + end + 2)
                    };
                    if let Some(e) = seg_end {
                        out.push_str(&src[i..e]);
                        i = e;
                        continue;
                    }
                }
                out.push_str(&src[i..]);
                break;
            }
            // find end of this mustache (no nested mustaches inside)
            if let Some(end) = src[i..].find("}}") {
                let seg = &src[i..i + end + 2];
                out.push_str(&rewrite_segment(seg));
                i = i + end + 2;
                continue;
            }
            out.push_str(&src[i..]);
            break;
        }
        // copy non-mustache region up to next {{
        match src[i..].find("{{") {
            Some(next) => {
                out.push_str(&src[i..i + next]);
                i += next;
            }
            None => {
                out.push_str(&src[i..]);
                break;
            }
        }
    }
    out
}

/// Inside a single mustache segment, rewrite @var → @root.var.
fn rewrite_segment(seg: &str) -> String {
    let mut out = seg.to_string();
    for name in ROOT_VARS {
        for pat in [format!("@{name}."), format!("@{name}=")] {
            let rep = format!("@root.{name}.");
            if out.contains(&pat) {
                out = out.replace(&pat, &rep);
            }
        }
        // bare @name at segment end or before }} / | / space / ) / ,
        // handled by checking with a small boundary set below
        let bare = format!("@{name}{{}}");
        let _ = bare;
    }
    // second pass for bare occurrences: @site}} / @site |x| / @site space
    let re_candidates = ["site", "blog", "config", "member", "labs", "custom", "page", "context"];
    let mut result = String::with_capacity(out.len());
    let mut i = 0;
    let bytes = out.as_bytes();
    while i < out.len() {
        if bytes[i] == b'@' {
            let rest = &out[i + 1..];
            if let Some(name) = re_candidates.iter().find(|n| rest.starts_with(**n)) {
                let after = &rest[name.len()..];
                let boundary = after
                    .chars()
                    .next()
                    .map(|c| !c.is_alphanumeric() && c != '_' && c != '-')
                    .unwrap_or(true);
                if boundary && !LOOP_VARS.contains(name) {
                    result.push_str("@root.");
                    result.push_str(name);
                    i += 1 + name.len();
                    continue;
                }
            }
        }
        // copy one whole character — stepping a byte at a time would split
        // multi-byte characters into mojibake
        let ch = out[i..].chars().next().unwrap_or('\0');
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_layout() {
        let (layout, rest) = extract_layout("{{!< default}}\n<h1>hi</h1>");
        assert_eq!(layout.as_deref(), Some("default"));
        assert_eq!(rest.trim(), "<h1>hi</h1>");

        let (layout, rest) = extract_layout("<h1>hi</h1>");
        assert!(layout.is_none());
        assert_eq!(rest, "<h1>hi</h1>");
    }

    #[test]
    fn rewrites_at_site() {
        let src = "{{@site.title}} {{@site.url}} {{@config.posts_per_page}}";
        let out = rewrite_at_vars(src);
        assert_eq!(out, "{{@root.site.title}} {{@root.site.url}} {{@root.config.posts_per_page}}");
    }

    #[test]
    fn preserves_loop_vars() {
        let src = "{{@index}} {{@first}} {{@last}} {{@number}}";
        assert_eq!(rewrite_at_vars(src), src);
    }

    #[test]
    fn skips_comments() {
        let src = "{{!-- @site.title --}}{{@site.url}}";
        assert_eq!(rewrite_at_vars(src), "{{!-- @site.title --}}{{@root.site.url}}");
    }

    #[test]
    fn closes_dynamic_partial_blocks_namelessly() {
        // the shape Casper and Source use for per-platform icons
        let src = r#"{{#> (concat "icons/" type)}}<span>{{name}}</span>{{/undefined}}"#;
        assert_eq!(
            rewrite_dynamic_partial_blocks(src),
            r#"{{#> (concat "icons/" type)}}<span>{{name}}</span>{{/}}"#
        );
    }

    #[test]
    fn leaves_ordinary_blocks_alone() {
        for src in [
            "{{#if x}}a{{/if}}",
            "{{#foreach posts}}{{title}}{{/foreach}}",
            "{{#> layout}}a{{/layout}}",
            "{{> icons/rss}}",
            "{{^if x}}a{{/if}}",
        ] {
            assert_eq!(rewrite_dynamic_partial_blocks(src), src, "{src}");
        }
    }

    #[test]
    fn rewrites_only_the_matching_close_when_nested() {
        let src = r#"{{#> (concat "icons/" type)}}{{#if name}}<span>{{name}}</span>{{/if}}{{/undefined}}"#;
        let out = rewrite_dynamic_partial_blocks(src);
        assert!(out.ends_with("{{/if}}{{/}}"), "{out}");
        assert_eq!(out.matches("{{/}}").count(), 1);
    }

    #[test]
    fn close_scan_skips_long_comments() {
        // a `}}` inside {{!-- --}} must not be mistaken for a tag end
        let src = r#"{{#> (concat "icons/" type)}}{{!-- a }} brace --}}x{{/undefined}}"#;
        let out = rewrite_dynamic_partial_blocks(src);
        assert!(out.ends_with("x{{/}}"), "{out}");
    }

    #[test]
    fn inverted_section_spanning_multibyte_text() {
        // the scan for the closing tag steps byte-by-byte; a multi-byte
        // character in between used to panic on a str slice boundary
        let src = "{{^if featured}}<span>✦ é 日本</span>{{/if}}";
        let out = rewrite_inverted_sections(src);
        assert_eq!(out, "{{#if featured}}{{else}}<span>✦ é 日本</span>{{/if}}");
    }

    #[test]
    fn preserves_multibyte_text_verbatim() {
        for src in [
            "<div class=\"divider-mark\"><span>✦</span></div>",
            "{{#if x}}✦{{/if}}",
            "{{t \"Lire la suite →\"}}",
            "日本語 {{@site.title}} ✦",
        ] {
            let out = prepare(src);
            for ch in ['✦', '→', 'é', '日'] {
                assert_eq!(src.matches(ch).count(), out.matches(ch).count(), "{src} -> {out}");
            }
        }
    }

    #[test]
    fn unquotes_partial_names() {
        assert_eq!(rewrite_partial_names(r#"{{> "icons/facebook"}}"#), "{{> icons/facebook}}");
        assert_eq!(rewrite_partial_names("{{> 'icons/rss'}}"), "{{> icons/rss}}");
        assert_eq!(rewrite_partial_names("{{>\"header\"}}"), "{{>header}}");
        // unquoted and block partials are left alone
        assert_eq!(rewrite_partial_names("{{> footer}}"), "{{> footer}}");
        assert_eq!(rewrite_partial_names(r#"{{#> "layout"}}x{{/layout}}"#), "{{#> layout}}x{{/layout}}");
        // a quoted helper argument is not a partial name
        let helper = r#"{{social_url type="facebook"}}"#;
        assert_eq!(rewrite_partial_names(helper), helper);
    }

    #[test]
    fn handles_subexpressions() {
        let src = "{{tags prefix=(t @site.title)}}";
        assert!(rewrite_at_vars(src).contains("@root.site.title"));
    }
}
