//! Template preprocessing.
//!
//! 1. Layout declaration: `{{!< default}}` at the top of a template names
//!    its layout (express-hbs semantics; default "default").
//! 2. `@site`/`@blog`/`@config`/`@member`/`@labs`/`@custom` are rewritten to
//!    `@root.<name>` — handlebars-rust only resolves `@root` from data, and
//!    its grammar accepts arbitrary `@foo` but cannot seed them via the
//!    public render API. Loop vars (`@index`, `@first`, …) are untouched.
//! 3. Quoted partial names (`{{> "icons/facebook"}}`) are unquoted.
//! 4. `{{^helper}}` inverted sections become `#` blocks with their branches
//!    arranged to match, and `posts.length` paths become `length` helper
//!    calls — both are things handlebars.js inherits from JavaScript that
//!    handlebars-rust rejects outright.
use once_cell::sync::Lazy;
use regex::Regex;

static LAYOUT_DECL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)^\s*\{\{!\s*<\s*([A-Za-z0-9_-]+)\s*\}\}").unwrap());

static QUOTED_PARTIAL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(\{\{~?[#>]?>\s*)["']([^"'}]+)["']"#).unwrap());

/// Apply every source rewrite a theme template needs before compiling.
pub fn prepare(src: &str) -> String {
    let src = rewrite_feature_image_captions(src);
    let src = rewrite_partial_names(&src);
    let src = rewrite_dynamic_partial_blocks(&src);
    // before the inverted-section pass, so a `{{^../x}}` arrives there as an
    // ordinary named block
    let src = rewrite_path_blocks(&src);
    let src = rewrite_length_paths(&src);
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

/// Ghost exposes feature image captions as HTML SafeStrings. serde_json
/// cannot carry that marker, so render caption field expressions unescaped
/// while keeping ordinary fields and helper arguments escaped as usual.
fn rewrite_feature_image_captions(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (start, end, inner) in tags(src) {
        let path = inner.trim_matches('~').trim();
        if path.rsplit(['.', '/']).next() != Some("feature_image_caption")
            || path.chars().any(|c| !(c.is_alphanumeric() || "_./".contains(c)))
        {
            continue;
        }
        out.push_str(&src[last..start]);
        out.push_str("{{{");
        out.push_str(inner);
        out.push_str("}}}");
        last = end;
    }
    out.push_str(&src[last..]);
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

/// What a `{{…}}` tag does to block nesting.
#[derive(PartialEq)]
enum Kind {
    /// `{{#x}}`, `{{#> x}}` or `{{^x}}` — opens a block.
    Open,
    /// `{{/x}}` — closes one.
    Close,
    /// `{{else}}` or `{{^}}` — splits the block it sits in.
    Else,
    Other,
}

fn classify(inner: &str) -> Kind {
    let t = inner.trim_matches('~').trim();
    if t.starts_with('#') {
        return Kind::Open;
    }
    if t.starts_with('/') {
        return Kind::Close;
    }
    if t == "^" || t == "else" {
        return Kind::Else;
    }
    if inverted_open(inner).is_some() {
        return Kind::Open;
    }
    Kind::Other
}

/// Byte offset of the `^` in an `{{^name …}}` inverted-section open, or `None`
/// for anything else — including the bare `{{^}}`, which is an else.
fn inverted_open(inner: &str) -> Option<usize> {
    let caret = inner.find('^')?;
    if inner[..caret].chars().any(|c| c != '~' && !c.is_whitespace()) {
        return None;
    }
    let named = inner[caret + 1..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    named.then_some(caret)
}

/// Rewrite `{{^helper args}}` inverted sections into the `{{#helper args}}`
/// form with the branches arranged to match — handlebars.js supports `^` on
/// any block helper; handlebars-rust only supports `{{else}}`/`{{^}}` inside
/// a `#` block.
///
/// Without an else, the body is simply the else branch:
///
/// ```text
/// {{^is "paged"}}A{{/is}}   →  {{#is "paged"}}{{else}}A{{/is}}
/// ```
///
/// With one, the two branches **swap**, because `^` inverts the test and
/// nothing else:
///
/// ```text
/// {{^is "paged"}}A{{else}}B{{/is}}  →  {{#is "paged"}}B{{else}}A{{/is}}
/// ```
///
/// Stacking an extra `{{else}}` in front of a branch that already has one
/// would leave the block with two, which is a parse error — the shape that
/// stopped Journal and Wave compiling at all.
pub fn rewrite_inverted_sections(src: &str) -> String {
    let tags = tags(src);
    let Some(idx) = tags.iter().position(|(_, _, inner)| inverted_open(inner).is_some()) else {
        return src.to_string();
    };
    let (open_start, open_end, open_inner) = tags[idx];
    let caret = inverted_open(open_inner).expect("position matched on this tag");

    // Walk to the matching close, noting the block's own `{{else}}` on the way.
    let mut depth = 1usize;
    let mut else_span: Option<(usize, usize)> = None;
    let mut close_span: Option<(usize, usize)> = None;
    for &(start, end, inner) in &tags[idx + 1..] {
        match classify(inner) {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    close_span = Some((start, end));
                    break;
                }
            }
            Kind::Else if depth == 1 && else_span.is_none() => else_span = Some((start, end)),
            _ => {}
        }
    }

    // An unbalanced tag is left exactly as it was, so one malformed block
    // cannot stop the rest of the template being rewritten. The compiler
    // reports it far better than we could.
    let Some((close_start, close_end)) = close_span else {
        let mut out = src[..open_end].to_string();
        out.push_str(&rewrite_inverted_sections(&src[open_end..]));
        return out;
    };

    let mut out = String::with_capacity(src.len() + 16);
    out.push_str(&src[..open_start]);
    // `^` → `#`, keeping any whitespace-control tilde and the arguments as they
    // were written
    out.push_str("{{");
    out.push_str(&open_inner[..caret]);
    out.push('#');
    out.push_str(&open_inner[caret + 1..]);
    out.push_str("}}");
    match else_span {
        Some((else_start, else_end)) => {
            out.push_str(&rewrite_inverted_sections(&src[else_end..close_start]));
            out.push_str("{{else}}");
            out.push_str(&rewrite_inverted_sections(&src[open_end..else_start]));
        }
        None => {
            out.push_str("{{else}}");
            out.push_str(&rewrite_inverted_sections(&src[open_end..close_start]));
        }
    }
    out.push_str(&src[close_start..close_end]);
    out.push_str(&rewrite_inverted_sections(&src[close_end..]));
    out
}
/// Rewrite a block opened on a path handlebars-rust will not parse as a block
/// name into an explicit `blockHelperMissing` call.
///
/// handlebars.js lets any path open a block — `{{#../primary_tag}}` is how
/// Lyra reaches past a `{{#foreach}}` to the post it is iterating from — and
/// routes it to `blockHelperMissing`. handlebars-rust's grammar wants an
/// identifier there and rejects the whole template otherwise.
///
/// ```text
/// {{#../primary_tag}}A{{/../primary_tag}}
///   →  {{#blockHelperMissing ../primary_tag}}A{{/blockHelperMissing}}
/// ```
///
/// The close has to be renamed to match: handlebars-rust accepts the nameless
/// `{{/}}` for a partial block but insists a helper block closes on its own
/// name.
pub fn rewrite_path_blocks(src: &str) -> String {
    let tags = tags(src);
    let Some(idx) = tags.iter().position(|(_, _, inner)| path_block_open(inner).is_some()) else {
        return src.to_string();
    };
    let (open_start, open_end, open_inner) = tags[idx];
    let (sigil, path) = path_block_open(open_inner).expect("position matched on this tag");

    let mut depth = 1usize;
    let mut close_span = None;
    for &(start, end, inner) in &tags[idx + 1..] {
        match classify(inner) {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    close_span = Some((start, end));
                    break;
                }
            }
            _ => {}
        }
    }

    let Some((close_start, close_end)) = close_span else {
        let mut out = src[..open_end].to_string();
        out.push_str(&rewrite_path_blocks(&src[open_end..]));
        return out;
    };

    let mut out = String::with_capacity(src.len() + 24);
    out.push_str(&src[..open_start]);
    out.push_str(&format!("{{{{{sigil}blockHelperMissing {path}}}}}"));
    out.push_str(&rewrite_path_blocks(&src[open_end..close_start]));
    out.push_str("{{/blockHelperMissing}}");
    out.push_str(&rewrite_path_blocks(&src[close_end..]));
    out
}

/// Split `{{#../tag.name}}` into its sigil and the path it opens on, or `None`
/// when the tag opens on a plain helper name (or is not a block open at all).
fn path_block_open(inner: &str) -> Option<(char, &str)> {
    let body = inner.trim_matches('~').trim();
    let sigil = body.chars().next().filter(|c| *c == '#' || *c == '^')?;
    let rest = body[sigil.len_utf8()..].trim_start();
    // `{{#> partial}}` and sub-expressions are somebody else's business
    if rest.starts_with('>') || rest.starts_with('(') {
        return None;
    }
    // only a lone path: arguments mean it really is a helper call
    let path = rest.split_whitespace().next()?;
    if path != rest || path.is_empty() {
        return None;
    }
    // a bare identifier parses fine as a block name; a path does not
    let is_identifier = path
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && path.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
    (!is_identifier).then_some((sigil, path))
}

/// Rewrite `posts.length` and friends into calls to the `length` helper.
///
/// In handlebars.js `{{#match posts.length ">" 1}}` works because `posts` is a
/// JavaScript array and `.length` is a real member of it. handlebars-rust
/// resolves paths against JSON, where indexing an array by name is an error —
/// not a miss — so the whole template fails to render. Headline and Taste both
/// open their home page with one.
///
/// In argument position the path becomes a sub-expression; on its own it
/// becomes the call itself:
///
/// ```text
/// {{#match posts.length ">" 1}}  →  {{#match (length posts) ">" 1}}
/// {{posts.length}}               →  {{length posts}}
/// ```
pub fn rewrite_length_paths(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (start, end, inner) in tags(src) {
        // comments, partials and close tags carry no resolvable paths
        let sigil = inner.trim_start_matches('~').trim_start();
        if sigil.starts_with('!') || sigil.starts_with('>') || sigil.starts_with('/') || !inner.contains(".length") {
            continue;
        }
        let Some(rewritten) = rewrite_length_in_tag(inner) else {
            continue;
        };
        out.push_str(&src[last..start]);
        out.push_str("{{");
        out.push_str(&rewritten);
        out.push_str("}}");
        last = end;
    }
    if last == 0 {
        return src.to_string();
    }
    out.push_str(&src[last..]);
    out
}

/// Rewrite every `….length` path inside one tag's body, or `None` when it has
/// none to rewrite.
fn rewrite_length_in_tag(inner: &str) -> Option<String> {
    let is_path_char = |c: char| c.is_alphanumeric() || "_-./@[]".contains(c);

    let mut out = String::with_capacity(inner.len() + 8);
    let mut rest = inner;
    // The first bare word is the helper name; a path there is the whole
    // expression rather than an argument to one.
    let mut seen_word = false;
    let mut changed = false;

    while !rest.is_empty() {
        // string literals are opaque: `{{t "4.length"}}` is just text
        if let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let end = rest[1..].find(quote).map_or(rest.len(), |i| i + 2);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            seen_word = true;
            continue;
        }
        let word_len = rest.chars().take_while(|c| is_path_char(*c)).map(char::len_utf8).sum::<usize>();
        if word_len == 0 {
            let ch = rest.chars().next().expect("rest is non-empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
            continue;
        }
        let word = &rest[..word_len];
        rest = &rest[word_len..];
        match word.strip_suffix(".length").filter(|path| !path.is_empty()) {
            Some(path) if seen_word => {
                out.push_str(&format!("(length {path})"));
                changed = true;
            }
            Some(path) => {
                out.push_str(&format!("length {path}"));
                changed = true;
            }
            None => out.push_str(word),
        }
        seen_word = true;
    }
    changed.then_some(out)
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
    fn inverted_section_without_else_becomes_the_else_branch() {
        assert_eq!(
            rewrite_inverted_sections(r#"{{^is "paged"}}A{{/is}}"#),
            r#"{{#is "paged"}}{{else}}A{{/is}}"#
        );
    }

    #[test]
    fn inverted_section_with_else_swaps_its_branches() {
        // Journal and Wave both write this; stacking an else in front of the
        // first branch left the block with two and it would not parse
        let out = rewrite_inverted_sections(r#"{{^is "paged"}}A{{else}}B{{/is}}"#);
        assert_eq!(out, r#"{{#is "paged"}}B{{else}}A{{/is}}"#);
        assert_eq!(out.matches("{{else}}").count(), 1);
    }

    #[test]
    fn inverted_section_ignores_a_nested_blocks_else() {
        // the inner {{else}} belongs to {{#if}}, so the outer block has none
        let out = rewrite_inverted_sections("{{^is \"paged\"}}{{#if x}}A{{else}}B{{/if}}{{/is}}");
        assert_eq!(out, "{{#is \"paged\"}}{{else}}{{#if x}}A{{else}}B{{/if}}{{/is}}");
    }

    #[test]
    fn rewrites_nested_and_sequential_inverted_sections() {
        let out = rewrite_inverted_sections("{{^is \"a\"}}{{^is \"b\"}}X{{/is}}{{/is}}{{^if y}}Z{{/if}}");
        assert_eq!(
            out,
            "{{#is \"a\"}}{{else}}{{#is \"b\"}}{{else}}X{{/is}}{{/is}}{{#if y}}{{else}}Z{{/if}}"
        );
    }

    #[test]
    fn unbalanced_inverted_section_is_left_alone() {
        let src = "{{^is \"paged\"}}A";
        assert_eq!(rewrite_inverted_sections(src), src);
    }

    #[test]
    fn rewrites_parent_scope_block_sections() {
        // Lyra's read-next card reaches past a {{#foreach}} like this
        assert_eq!(
            rewrite_path_blocks("{{#../primary_tag}}<a>{{name}}</a>{{/../primary_tag}}"),
            "{{#blockHelperMissing ../primary_tag}}<a>{{name}}</a>{{/blockHelperMissing}}"
        );
        assert_eq!(
            rewrite_path_blocks("{{^../primary_tag}}none{{/../primary_tag}}"),
            "{{^blockHelperMissing ../primary_tag}}none{{/blockHelperMissing}}"
        );
    }

    #[test]
    fn leaves_helper_blocks_and_partials_alone() {
        for src in [
            "{{#if x}}a{{/if}}",
            "{{#foreach posts}}a{{/foreach}}",
            "{{#primary_tag}}a{{/primary_tag}}",
            "{{#get \"posts\" limit=\"3\"}}a{{/get}}",
            "{{#> layout}}a{{/layout}}",
            r#"{{#> (concat "icons/" type)}}a{{/}}"#,
            "{{#match posts.length \">\" 1}}a{{/match}}",
        ] {
            assert_eq!(rewrite_path_blocks(src), src, "{src}");
        }
    }

    #[test]
    fn parent_scope_block_survives_the_full_pipeline() {
        // the inverted pass runs after and must see a well-formed named block
        let out = prepare("{{^../primary_tag}}none{{/../primary_tag}}");
        assert_eq!(out, "{{#blockHelperMissing ../primary_tag}}{{else}}none{{/blockHelperMissing}}");
    }

    #[test]
    fn rewrites_length_paths_to_helper_calls() {
        // the exact shapes Headline, Taste and Journal open their pages with
        assert_eq!(
            rewrite_length_paths(r#"{{#match posts.length ">" 1}}"#),
            r#"{{#match (length posts) ">" 1}}"#
        );
        assert_eq!(rewrite_length_paths("{{posts.length}}"), "{{length posts}}");
        assert_eq!(rewrite_length_paths("{{#if tags.length}}"), "{{#if (length tags)}}");
        assert_eq!(rewrite_length_paths("{{plural n empty=posts.length}}"), "{{plural n empty=(length posts)}}");
    }

    #[test]
    fn leaves_length_alone_where_it_is_not_a_path() {
        for src in [
            "<p>the length of it</p>",
            r#"{{t "the .length of it"}}"#,
            "{{!-- posts.length --}}",
            "{{> partials/x.length}}",
            "{{length posts}}",
            // a bare `length` is a member name, not the `.length` idiom
            "{{post.lengthy}}",
        ] {
            assert_eq!(rewrite_length_paths(src), src, "{src}");
        }
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
