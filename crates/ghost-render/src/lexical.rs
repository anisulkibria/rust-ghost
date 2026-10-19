//! Lexical (Koenig) → HTML.
//!
//! Ghost stores Koenig documents as a Lexical editor state and renders them
//! with `@tryghost/kg-lexical-html-renderer`, which drives a headless Lexical
//! editor and asks every node for its `exportDOM`. There is no Rust Lexical,
//! so this walks the serialised JSON directly, mirroring that package's
//! `$convertToHtmlString` (transformers for the element nodes, `TextContent`
//! for runs of text/link/linebreak) and the card renderers from
//! `@tryghost/kg-default-nodes`.
//!
//! The admin editor posts the HTML Koenig itself produced in the browser, so
//! this is the *fallback* path: content that arrived some other way — a Ghost
//! database imported into the schema, a post written with JavaScript off —
//! still renders. Cards it does not know are skipped rather than guessed at;
//! the node list below covers the ones Ghost's own editor inserts most.
use serde_json::Value;
use std::collections::HashMap;

/// Render a serialised Lexical editor state to Ghost-flavoured HTML.
///
/// Returns an empty string for anything that is not a Lexical document, so a
/// caller can treat "no lexical" and "unparseable lexical" alike.
pub fn to_html(lexical: &str) -> String {
    let Ok(doc) = serde_json::from_str::<Value>(lexical) else {
        return String::new();
    };
    let Some(children) = doc.pointer("/root/children").and_then(Value::as_array) else {
        return String::new();
    };

    let mut ids = HashMap::new();
    let mut out: Vec<String> = Vec::new();
    for child in children {
        out.push(top_level(child, &mut ids));
    }
    // Koenig keeps a trailing empty paragraph in the document so there is
    // always somewhere to put the cursor. It is editor furniture, not content.
    if let Some(last) = children.last() {
        if kind(last) == "paragraph" && text_content(last).trim().is_empty() {
            out.pop();
        }
    }
    out.concat()
}

/// Plain text of a document, for the `plaintext` column and excerpts.
pub fn to_plaintext(lexical: &str) -> String {
    let Ok(doc) = serde_json::from_str::<Value>(lexical) else {
        return String::new();
    };
    let Some(children) = doc.pointer("/root/children").and_then(Value::as_array) else {
        return String::new();
    };
    children
        .iter()
        .map(text_content)
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Wrap existing content in a one-card Lexical document, so a post written
/// before Koenig opens in it with nothing lost.
///
/// Converting markdown or HTML into paragraphs, headings and cards would mean
/// a second parser and a second set of judgement calls about what maps to
/// what — and any of them getting it wrong would quietly damage a published
/// post the first time its author opened it. A Koenig markdown card holds the
/// source verbatim and renders it exactly as before, which is also how Ghost
/// itself carries markdown written in its older editor. The author can split
/// the card up into rich text whenever they want to.
pub fn wrap_existing(markdown: Option<&str>, html: Option<&str>) -> Option<String> {
    let card = match (markdown.filter(|m| !m.trim().is_empty()), html.filter(|h| !h.trim().is_empty())) {
        (Some(markdown), _) => serde_json::json!({ "type": "markdown", "version": 1, "markdown": markdown }),
        (None, Some(html)) => serde_json::json!({ "type": "html", "version": 1, "html": html }),
        (None, None) => return None,
    };
    Some(
        serde_json::json!({
            "root": {
                "children": [card],
                "direction": null,
                "format": "",
                "indent": 0,
                "type": "root",
                "version": 1,
            }
        })
        .to_string(),
    )
}

// ---------------------------------------------------------------------------
// elements
// ---------------------------------------------------------------------------

fn top_level(node: &Value, ids: &mut HashMap<String, usize>) -> String {
    match kind(node) {
        "paragraph" => format!("<p>{}</p>", children_html(node)),
        "heading" | "extended-heading" => {
            let tag = node.get("tag").and_then(Value::as_str).unwrap_or("h2");
            let tag = if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") { tag } else { "h2" };
            format!(r#"<{tag} id="{id}">{inner}</{tag}>"#, id = heading_id(&text_content(node), ids), inner = children_html(node))
        }
        "quote" | "extended-quote" => format!("<blockquote>{}</blockquote>", children_html(node)),
        "aside" => format!(r#"<blockquote class="kg-blockquote-alt">{}</blockquote>"#, children_html(node)),
        "list" => list_html(node),
        "" => String::new(),
        _ => card_html(node),
    }
}

/// Ghost's list export: a nested list is emitted *inside* the `<li>` that is
/// still open, so `<ol><li>one<ol><li>one.two</li></ol></li></ol>` comes out
/// the way a browser would nest it rather than as a sibling list.
fn list_html(node: &Value) -> String {
    let tag = match node.get("tag").and_then(Value::as_str) {
        Some("ol") => "ol",
        _ => "ul",
    };
    let mut out = String::new();
    let mut li_open = false;
    for item in child_nodes(node) {
        if kind(item) != "listitem" {
            continue;
        }
        let inner = child_nodes(item);
        match inner.first() {
            // a bare nested list: it belongs to the <li> we already opened
            Some(first) if kind(first) == "list" => {
                out.push_str(&list_html(first));
                if li_open {
                    out.push_str("</li>");
                    li_open = false;
                }
            }
            _ => {
                if li_open {
                    out.push_str("</li>");
                }
                out.push_str(&format!("<li>{}", children_html(item)));
                li_open = true;
            }
        }
    }
    if li_open {
        out.push_str("</li>");
    }
    match node.get("start").and_then(Value::as_i64) {
        Some(start) if start != 1 => format!(r#"<{tag} start="{start}">{out}</{tag}>"#),
        _ => format!("<{tag}>{out}</{tag}>"),
    }
}

/// Children of an element: runs of text/link/linebreak render inline, nested
/// elements recurse, and a card nested in an element renders as itself.
fn children_html(node: &Value) -> String {
    let mut out = String::new();
    for child in child_nodes(node) {
        match kind(child) {
            "text" | "extended-text" => out.push_str(&text_html(child)),
            "linebreak" => out.push_str("<br>"),
            "tab" => out.push('\t'),
            "link" | "extended-link" => {
                let href = child.get("url").and_then(Value::as_str).unwrap_or("");
                let rel = child.get("rel").and_then(Value::as_str).filter(|r| !r.is_empty());
                let rel = rel.map(|r| format!(r#" rel="{}""#, attr(r))).unwrap_or_default();
                // Ghost omits href entirely for an empty URL rather than
                // emitting a link back to the current page.
                let href = if href.is_empty() { String::new() } else { format!(r#" href="{}""#, attr(href)) };
                out.push_str(&format!("<a{href}{rel}>{}</a>", children_html(child)));
            }
            "paragraph" | "heading" | "extended-heading" | "quote" | "extended-quote" | "aside" | "list" | "listitem" => {
                let mut ids = HashMap::new();
                out.push_str(&top_level(child, &mut ids));
            }
            _ => out.push_str(&card_html(child)),
        }
    }
    out
}

/// Lexical's format bitmask, innermost-last so the tags nest in Ghost's order.
const FORMATS: &[(i64, &str)] = &[
    (1, "strong"),
    (2, "em"),
    (4, "s"),
    (8, "u"),
    (16, "code"),
    (32, "sub"),
    (64, "sup"),
    (128, "mark"),
];

fn text_html(node: &Value) -> String {
    let text = esc(node.get("text").and_then(Value::as_str).unwrap_or(""));
    let format = node.get("format").and_then(Value::as_i64).unwrap_or(0);
    if format == 0 {
        return text;
    }
    let open: String = FORMATS.iter().filter(|(bit, _)| format & bit != 0).map(|(_, t)| format!("<{t}>")).collect();
    let close: String = FORMATS.iter().rev().filter(|(bit, _)| format & bit != 0).map(|(_, t)| format!("</{t}>")).collect();
    format!("{open}{text}{close}")
}

// ---------------------------------------------------------------------------
// cards
// ---------------------------------------------------------------------------

fn card_html(node: &Value) -> String {
    match kind(node) {
        "horizontalrule" => "<hr>".into(),
        // Ghost brackets raw-HTML and markdown cards with comments so the
        // boundary survives a round-trip back into the editor.
        "html" => {
            let html = str_of(node, "html");
            if html.trim().is_empty() {
                String::new()
            } else {
                format!("<!--kg-card-begin: html-->{html}<!--kg-card-end: html-->")
            }
        }
        // Ghost renders the markdown card's output inline, with no wrapper
        // and no boundary comments — those belong to the mobiledoc era.
        "markdown" => crate::markdown::to_card_html(&str_of(node, "markdown")),
        "codeblock" => codeblock_html(node),
        "image" => image_html(node),
        "embed" => {
            let html = str_of(node, "html");
            if html.trim().is_empty() {
                return String::new();
            }
            let caption = str_of(node, "caption");
            let class = card_class("kg-embed-card", &caption, node);
            format!(r#"<figure class="{class}">{html}{}</figure>"#, figcaption(&caption))
        }
        "callout" => {
            // Anything that is not a valid class name — `rgba(0,0,0,0)` can
            // survive an old mobiledoc paste — falls back to white, as Ghost
            // does rather than emitting a broken class.
            let colour = node.get("backgroundColor").and_then(Value::as_str).unwrap_or("");
            let colour = if !colour.is_empty() && colour.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                colour
            } else {
                "white"
            };
            let emoji = str_of(node, "calloutEmoji");
            let emoji_html = if emoji.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="kg-callout-emoji">{}</div>"#, esc(&emoji))
            };
            format!(
                r#"<div class="kg-card kg-callout-card kg-callout-card-{colour}">{emoji_html}<div class="kg-callout-text">{text}</div></div>"#,
                colour = attr(colour),
                text = keep_inline_tags(&str_of(node, "calloutText")),
            )
        }
        "button" => {
            let url = str_of(node, "buttonUrl");
            let text = str_of(node, "buttonText");
            if url.is_empty() || text.is_empty() {
                return String::new();
            }
            let align = node.get("alignment").and_then(Value::as_str).unwrap_or("center");
            format!(
                r#"<div class="kg-card kg-button-card kg-align-{align}"><a href="{url}" class="kg-btn kg-btn-accent">{text}</a></div>"#,
                align = attr(align),
                url = attr(&url),
                text = esc(&text),
            )
        }
        "toggle" => format!(
            r#"<div class="kg-card kg-toggle-card" data-kg-toggle-state="close"><div class="kg-toggle-heading"><h4 class="kg-toggle-heading-text">{heading}</h4><button class="kg-toggle-card-icon" aria-label="Expand toggle to read content"><svg id="Regular" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path class="cls-1" d="M23.25,7.311,12.53,18.03a.749.749,0,0,1-1.06,0L.75,7.311"></path></svg></button></div><div class="kg-toggle-content">{content}</div></div>"#,
            heading = str_of(node, "heading"),
            content = str_of(node, "content"),
        ),
        "bookmark" => bookmark_html(node),
        "gallery" => gallery_html(node),
        "file" => file_html(node),
        "paywall" => "<!--members-only-->".into(),
        // email-only content is dropped from the web rendering, as in Ghost
        "email" | "email-cta" => String::new(),
        // The audio, video, product, signup, header and call-to-action cards
        // are several kilobytes of templated markup each, with branches for
        // layout, colour and thumbnail state. Reproducing them here would be
        // guesswork that themes style as if it were the real thing, so they
        // are left out: the editor writes these to `html` itself, and that
        // is the path they actually take.
        _ => String::new(),
    }
}

fn codeblock_html(node: &Value) -> String {
    let code = esc(&str_of(node, "code"));
    let language = str_of(node, "language");
    let caption = str_of(node, "caption");
    let class = if language.is_empty() { String::new() } else { format!(r#" class="language-{}""#, attr(&language)) };
    let pre = format!("<pre><code{class}>{code}</code></pre>");
    if caption.is_empty() {
        pre
    } else {
        format!(r#"<figure class="kg-card kg-code-card">{pre}{}</figure>"#, figcaption(&caption))
    }
}

fn image_html(node: &Value) -> String {
    let src = str_of(node, "src");
    if src.trim().is_empty() {
        return String::new();
    }
    let caption = str_of(node, "caption");
    let class = card_class("kg-image-card", &caption, node);
    let alt = attr(&str_of(node, "alt"));
    let title = match str_of(node, "title") {
        t if t.is_empty() => String::new(),
        t => format!(r#" title="{}""#, attr(&t)),
    };
    let dims = match (node.get("width").and_then(Value::as_i64), node.get("height").and_then(Value::as_i64)) {
        (Some(w), Some(h)) => format!(r#" width="{w}" height="{h}""#),
        _ => String::new(),
    };
    let img = format!(r#"<img src="{src}" class="kg-image" alt="{alt}" loading="lazy"{title}{dims}>"#, src = attr(&src));
    // An image with a link wraps the <img>, matching Ghost's image card.
    let img = match str_of(node, "href") {
        h if h.is_empty() => img,
        h => format!(r#"<a href="{}">{img}</a>"#, attr(&h)),
    };
    format!(r#"<figure class="{class}">{img}{}</figure>"#, figcaption(&caption))
}

fn bookmark_html(node: &Value) -> String {
    let url = str_of(node, "url");
    if url.is_empty() {
        return String::new();
    }
    let meta = node.get("metadata").cloned().unwrap_or(Value::Null);
    let m = |key: &str| meta.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let icon = match m("icon") {
        i if i.is_empty() => String::new(),
        i => format!(r#"<img class="kg-bookmark-icon" src="{}" alt="">"#, attr(&i)),
    };
    // Ghost puts the publisher in `kg-bookmark-author` and the author in
    // `kg-bookmark-publisher`. That looks like a bug and is not one — the
    // classes were swapped years ago and themes style them that way round,
    // so matching Ghost means matching the swap.
    let author_span = match m("publisher") {
        p if p.is_empty() => String::new(),
        p => format!(r#"<span class="kg-bookmark-author">{}</span>"#, esc(&p)),
    };
    let publisher_span = match m("author") {
        a if a.is_empty() => String::new(),
        a => format!(r#"<span class="kg-bookmark-publisher">{}</span>"#, esc(&a)),
    };
    let thumbnail = match m("thumbnail") {
        t if t.is_empty() => String::new(),
        t => format!(r#"<div class="kg-bookmark-thumbnail"><img src="{}" alt="" onerror="this.style.display = 'none'"></div>"#, attr(&t)),
    };
    let caption = str_of(node, "caption");
    let mut class = "kg-card kg-bookmark-card".to_string();
    if !caption.is_empty() {
        class.push_str(" kg-card-hascaption");
    }
    format!(
        r#"<figure class="{class}"><a class="kg-bookmark-container" href="{url}"><div class="kg-bookmark-content"><div class="kg-bookmark-title">{title}</div><div class="kg-bookmark-description">{desc}</div><div class="kg-bookmark-metadata">{icon}{author_span}{publisher_span}</div></div>{thumbnail}</a>{figcaption}</figure>"#,
        url = attr(&url),
        title = esc(&m("title")),
        desc = esc(&m("description")),
        figcaption = figcaption(&caption),
    )
}

/// Ghost lays a gallery out in rows of up to three. The one wrinkle: when the
/// last row would hold a single image, the one before it is pushed down so
/// the tail reads as a pair rather than a widow.
fn gallery_html(node: &Value) -> String {
    let images: Vec<&Value> = node
        .get("images")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|i| valid_gallery_image(i)).collect())
        .unwrap_or_default();
    if images.is_empty() {
        return String::new();
    }

    let count = images.len();
    let mut rows: Vec<Vec<&Value>> = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let mut row = image.get("row").and_then(Value::as_i64).unwrap_or(0) as usize;
        if count > 1 && count % 3 == 1 && index == count - 2 {
            row += 1;
        }
        while rows.len() <= row {
            rows.push(Vec::new());
        }
        rows[row].push(image);
    }

    let body: String = rows
        .iter()
        .filter(|row| !row.is_empty())
        .map(|row| {
            let cells: String = row
                .iter()
                .map(|image| {
                    let title = match str_of(image, "title") {
                        t if t.is_empty() => String::new(),
                        t => format!(r#" title="{}""#, attr(&t)),
                    };
                    let img = format!(
                        r#"<img src="{src}" width="{w}" height="{h}" loading="lazy" alt="{alt}"{title}>"#,
                        src = attr(&str_of(image, "src")),
                        w = image.get("width").and_then(Value::as_i64).unwrap_or(0),
                        h = image.get("height").and_then(Value::as_i64).unwrap_or(0),
                        alt = attr(&str_of(image, "alt")),
                    );
                    let img = match str_of(image, "href") {
                        href if href.is_empty() => img,
                        href => format!(r#"<a href="{}">{img}</a>"#, attr(&href)),
                    };
                    format!(r#"<div class="kg-gallery-image">{img}</div>"#)
                })
                .collect();
            format!(r#"<div class="kg-gallery-row">{cells}</div>"#)
        })
        .collect();

    let caption = str_of(node, "caption");
    let mut class = "kg-card kg-gallery-card kg-width-wide".to_string();
    if !caption.is_empty() {
        class.push_str(" kg-card-hascaption");
    }
    format!(
        r#"<figure class="{class}"><div class="kg-gallery-container">{body}</div>{figcaption}</figure>"#,
        figcaption = figcaption(&caption),
    )
}

fn valid_gallery_image(image: &Value) -> bool {
    !str_of(image, "fileName").trim().is_empty()
        && !str_of(image, "src").trim().is_empty()
        && image.get("width").and_then(Value::as_i64).is_some_and(|w| w > 0)
        && image.get("height").and_then(Value::as_i64).is_some_and(|h| h > 0)
        && image.get("row").and_then(Value::as_i64).is_some_and(|r| r >= 0)
}

fn file_html(node: &Value) -> String {
    let src = str_of(node, "src");
    if src.trim().is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="kg-card kg-file-card"><a class="kg-file-card-container" href="{src}" title="Download" download=""><div class="kg-file-card-contents"><div class="kg-file-card-title">{title}</div><div class="kg-file-card-caption">{caption}</div><div class="kg-file-card-metadata"><div class="kg-file-card-filename">{name}</div><div class="kg-file-card-filesize">{size}</div></div></div><div class="kg-file-card-icon"><svg viewBox="0 0 24 24"><defs><style>.a{{fill:none;stroke:currentColor;stroke-linecap:round;stroke-linejoin:round;stroke-width:1.5px;}}</style></defs><title>download-circle</title><polyline class="a" points="8.25 14.25 12 18 15.75 14.25"></polyline><line class="a" x1="12" y1="6.75" x2="12" y2="18"></line><circle class="a" cx="12" cy="12" r="11.25"></circle></svg></div></a></div>"#,
        src = attr(&src),
        title = esc(&str_of(node, "fileTitle")),
        caption = esc(&str_of(node, "fileCaption")),
        name = esc(&str_of(node, "fileName")),
        size = esc(&bytes_to_size(node.get("fileSize").and_then(Value::as_f64).unwrap_or(0.0))),
    )
}

/// Ghost's `bytesToSize`: binary units, rounded to the nearest whole unit.
fn bytes_to_size(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["Bytes", "KB", "MB", "GB", "TB"];
    if bytes <= 0.0 {
        return "0 Byte".into();
    }
    let index = (bytes.ln() / 1024_f64.ln()).floor().clamp(0.0, 4.0) as usize;
    format!("{} {}", (bytes / 1024_f64.powi(index as i32)).round(), UNITS[index])
}

/// Ghost's `cleanDOM`: unwrap every tag outside the inline allowlist, keeping
/// the text inside it. Used for callout text, which is rich text in the
/// editor but only permits a handful of inline tags in the output.
fn keep_inline_tags(html: &str) -> String {
    const ALLOWED: &[&str] = &["a", "strong", "em", "b", "i", "br", "code", "mark", "s", "del", "u", "sup", "sub"];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let tag = &after[..end];
        let name: String =
            tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if ALLOWED.contains(&name.as_str()) {
            out.push('<');
            out.push_str(tag);
            out.push('>');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `kg-card kg-<name>-card` plus the width and caption modifiers Ghost adds.
fn card_class(name: &str, caption: &str, node: &Value) -> String {
    let mut class = format!("kg-card {name}");
    match node.get("cardWidth").and_then(Value::as_str) {
        Some(w) if w != "regular" && !w.is_empty() => class.push_str(&format!(" kg-width-{}", attr(w))),
        _ => {}
    }
    if !caption.is_empty() {
        class.push_str(" kg-card-hascaption");
    }
    class
}

fn figcaption(caption: &str) -> String {
    if caption.trim().is_empty() {
        String::new()
    } else {
        // Captions are themselves HTML (bold, links) in Koenig, so they are
        // passed through rather than escaped.
        format!("<figcaption>{caption}</figcaption>")
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn kind(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn child_nodes(node: &Value) -> &[Value] {
    node.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn str_of(node: &Value, key: &str) -> String {
    node.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The text of a node and everything under it.
fn text_content(node: &Value) -> String {
    if let Some(t) = node.get("text").and_then(Value::as_str) {
        return t.to_string();
    }
    child_nodes(node).iter().map(text_content).collect()
}

/// Ghost 4+ heading anchors: lowercased, symbols dropped, spaces to dashes,
/// percent-encoded, and deduplicated across the document.
///
/// Shared with the markdown card, whose headings are slugged the same way.
pub(crate) fn heading_anchor(text: &str, used: &mut HashMap<String, usize>) -> String {
    heading_id(text, used)
}

fn heading_id(text: &str, used: &mut HashMap<String, usize>) -> String {
    const SYMBOLS: &[char] = &[
        '[', ']', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '.', '/', ':', ';', '<', '=', '>', '?',
        '@', '\\', '^', '_', '{', '|', '}', '~',
    ];
    let cleaned: String = text.trim().to_lowercase().chars().filter(|c| !SYMBOLS.contains(c)).collect();
    let dashed = cleaned.split_whitespace().collect::<Vec<_>>().join("-");
    let base = dashed.trim_matches('-').to_string();
    let id = percent_encode(&base);
    match used.get_mut(&id) {
        Some(n) => {
            let deduped = format!("{id}-{n}");
            *n += 1;
            deduped
        }
        None => {
            used.insert(id.clone(), 1);
            id
        }
    }
}

/// `encodeURIComponent`: everything outside the unreserved set is escaped.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(children: &str) -> String {
        format!(r#"{{"root":{{"children":[{children}],"direction":"ltr","format":"","indent":0,"type":"root","version":1}}}}"#)
    }

    #[test]
    fn renders_paragraphs_and_formats() {
        let html = to_html(&doc(
            r#"{"children":[{"detail":0,"format":1,"mode":"normal","style":"","text":"bold","type":"text","version":1}],"type":"paragraph","version":1}"#,
        ));
        assert_eq!(html, "<p><strong>bold</strong></p>");
    }

    #[test]
    fn heading_ids_are_slugged_and_deduplicated() {
        let heading = r#"{"children":[{"text":"Hello, World!","type":"text","format":0,"version":1}],"tag":"h2","type":"heading","version":1}"#;
        let html = to_html(&doc(&format!("{heading},{heading}")));
        assert!(html.contains(r#"<h2 id="hello-world">"#), "{html}");
        assert!(html.contains(r#"<h2 id="hello-world-1">"#), "{html}");
    }

    #[test]
    fn nests_lists_inside_the_open_list_item() {
        let nested = r#"{"type":"list","tag":"ul","start":1,"version":1,"children":[
            {"type":"listitem","version":1,"children":[{"text":"one","type":"text","format":0,"version":1}]},
            {"type":"listitem","version":1,"children":[{"type":"list","tag":"ul","start":1,"version":1,"children":[
                {"type":"listitem","version":1,"children":[{"text":"one.two","type":"text","format":0,"version":1}]}]}]}]}"#;
        assert_eq!(to_html(&doc(nested)), "<ul><li>one<ul><li>one.two</li></ul></li></ul>");
    }

    #[test]
    fn links_carry_href_and_rel() {
        let html = to_html(&doc(
            r#"{"type":"paragraph","version":1,"children":[{"type":"link","url":"https://example.com","rel":"noopener","version":1,"children":[{"text":"go","type":"text","format":0,"version":1}]}]}"#,
        ));
        assert_eq!(html, r#"<p><a href="https://example.com" rel="noopener">go</a></p>"#);
    }

    #[test]
    fn image_card_matches_ghost_markup() {
        let html = to_html(&doc(
            r#"{"type":"image","version":1,"src":"/content/images/a.png","alt":"A","width":800,"height":600,"cardWidth":"wide","caption":"Hi"}"#,
        ));
        assert!(html.contains(r#"class="kg-card kg-image-card kg-width-wide kg-card-hascaption""#), "{html}");
        assert!(html.contains(r#"<img src="/content/images/a.png" class="kg-image" alt="A" loading="lazy" width="800" height="600">"#), "{html}");
        assert!(html.contains("<figcaption>Hi</figcaption>"), "{html}");
    }

    #[test]
    fn html_card_keeps_ghosts_boundary_comments() {
        let html = to_html(&doc(r#"{"type":"html","version":1,"html":"<div>raw</div>"}"#));
        assert_eq!(html, "<!--kg-card-begin: html--><div>raw</div><!--kg-card-end: html-->");
    }

    #[test]
    fn trailing_empty_paragraph_is_dropped() {
        let para = r#"{"type":"paragraph","version":1,"children":[{"text":"body","type":"text","format":0,"version":1}]}"#;
        let empty = r#"{"type":"paragraph","version":1,"children":[]}"#;
        assert_eq!(to_html(&doc(&format!("{para},{empty}"))), "<p>body</p>");
    }

    #[test]
    fn unknown_cards_are_skipped_not_guessed() {
        let para = r#"{"type":"paragraph","version":1,"children":[{"text":"body","type":"text","format":0,"version":1}]}"#;
        let unknown = r#"{"type":"some-future-card","version":1}"#;
        assert_eq!(to_html(&doc(&format!("{unknown},{para}"))), "<p>body</p>");
    }

    #[test]
    fn not_lexical_renders_nothing() {
        assert_eq!(to_html(""), "");
        assert_eq!(to_html("# just markdown"), "");
        assert_eq!(to_html(r#"{"foo":1}"#), "");
    }

    #[test]
    fn existing_content_is_wrapped_losslessly_and_round_trips() {
        let wrapped = wrap_existing(Some("# Hi\n\n![](/a.png)"), None).unwrap();
        // the source survives verbatim …
        let doc: Value = serde_json::from_str(&wrapped).unwrap();
        assert_eq!(doc["root"]["children"][0]["type"], "markdown");
        assert_eq!(doc["root"]["children"][0]["markdown"], "# Hi\n\n![](/a.png)");
        // … and renders the way Ghost renders a markdown card, anchor ids
        // on the headings and no wrapper around the whole thing
        let html = to_html(&wrapped);
        assert!(html.contains(r#"<h1 id="hi">Hi</h1>"#), "{html}");
        assert!(html.contains(r#"<img src="/a.png""#), "{html}");
        assert!(!html.contains("kg-card-begin"), "{html}");
    }

    #[test]
    fn html_only_content_is_wrapped_in_an_html_card() {
        let wrapped = wrap_existing(None, Some("<p>seeded</p>")).unwrap();
        assert_eq!(to_html(&wrapped), "<!--kg-card-begin: html--><p>seeded</p><!--kg-card-end: html-->");
        assert_eq!(wrap_existing(None, None), None);
        assert_eq!(wrap_existing(Some("  "), Some("   ")), None);
    }

    #[test]
    fn plaintext_joins_blocks() {
        let para = r#"{"type":"paragraph","version":1,"children":[{"text":"one","type":"text","format":0,"version":1}]}"#;
        let other = r#"{"type":"paragraph","version":1,"children":[{"text":"two","type":"text","format":0,"version":1}]}"#;
        assert_eq!(to_plaintext(&doc(&format!("{para},{other}"))), "one\n\ntwo");
    }
}
