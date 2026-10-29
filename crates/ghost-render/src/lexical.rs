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
//! this is the *fallback* path, and the one that matters whenever the HTML did
//! not come with the document: a post created through the Admin API with only
//! `lexical`, a Ghost export imported into the schema, an automation email
//! body. Podcast player identities are personalized at the theme boundary;
//! email rendering turns those players into per-recipient listening links.
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
            // `tk` is a TextNode subclass — the editor's "to komment" marker
            // is a highlight on the text, not a wrapper around it, so it
            // exports as the text itself with its formats.
            "text" | "extended-text" | "tk" => out.push_str(&text_html(child)),
            // Also a TextNode, but `getTextContent()` is empty: it exists to
            // give Lexical a cursor position and carries nothing to render.
            "zwnj" => {}
            // An at-link in progress exports as an empty span, so the
            // half-typed mention does not reach the published post.
            "at-link" | "at-link-search" => {}
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
        "audio" => audio_html(node),
        "video" => video_html(node),
        "product" => product_html(node),
        "header" => header_html(node),
        "signup" => signup_html(node),
        "call-to-action" => cta_html(node),
        "paywall" => "<!--members-only-->".into(),
        // email-only content is dropped from the web rendering, as in Ghost
        "email" | "email-cta" => String::new(),
        "transistor" => transistor_html(node),
        // Unknown cards are omitted.
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

// ---------------------------------------------------------------------------
// media and layout cards
//
// Ghost assembles these in `@tryghost/kg-default-nodes`, some from DOM calls
// and some from template literals, so its own output carries whatever
// indentation the template it came from had. These follow the cards above and
// are written compact — same tags, classes, attributes and order, without the
// incidental whitespace, which no theme can select on.
//
// The players here are inert markup on purpose: Ghost's audio and video cards
// are driven by the theme's own `cards.min.js`, which binds to these class
// names. Emitting them is what gives that script something to bind to.
// ---------------------------------------------------------------------------

/// The play triangle, shared by the audio and video players.
const PLAY_PATH: &str = "M23.14 10.608 2.253.164A1.559 1.559 0 0 0 0 1.557v20.887a1.558 1.558 0 0 0 2.253 1.392L23.14 13.393a1.557 1.557 0 0 0 0-2.785Z";
/// The two bars of the pause button, likewise shared.
const PAUSE_RECTS: &str = r#"<rect x="3" y="1" width="7" height="22" rx="1.5" ry="1.5"></rect><rect x="14" y="1" width="7" height="22" rx="1.5" ry="1.5"></rect>"#;
const UNMUTE_PATH: &str = "M15.189 2.021a9.728 9.728 0 0 0-7.924 4.85.249.249 0 0 1-.221.133H5.25a3 3 0 0 0-3 3v2a3 3 0 0 0 3 3h1.794a.249.249 0 0 1 .221.133 9.73 9.73 0 0 0 7.924 4.85h.06a1 1 0 0 0 1-1V3.02a1 1 0 0 0-1.06-.998Z";
const MUTE_PATH: &str = "M16.177 4.3a.248.248 0 0 0 .073-.176v-1.1a1 1 0 0 0-1.061-1 9.728 9.728 0 0 0-7.924 4.85.249.249 0 0 1-.221.133H5.25a3 3 0 0 0-3 3v2a3 3 0 0 0 3 3h.114a.251.251 0 0 0 .177-.073ZM23.707 1.706A1 1 0 0 0 22.293.292l-22 22a1 1 0 0 0 0 1.414l.009.009a1 1 0 0 0 1.405-.009l6.63-6.631A.251.251 0 0 1 8.515 17a.245.245 0 0 1 .177.075 10.081 10.081 0 0 0 6.5 2.92 1 1 0 0 0 1.061-1V9.266a.247.247 0 0 1 .073-.176Z";

/// Ghost's audio card.
///
/// Both thumbnails are always in the markup with one of them hidden, so the
/// player can swap them without re-rendering. The duration goes out as the
/// raw second count Ghost stores — `cards.min.js` formats it once the file's
/// metadata loads, and `0:00` beside it is the static placeholder until then.
fn audio_html(node: &Value) -> String {
    let src = str_of(node, "src");
    if src.trim().is_empty() {
        return String::new();
    }
    let thumbnail = str_of(node, "thumbnailSrc");
    let (thumb_class, empty_class) = if thumbnail.is_empty() {
        ("kg-audio-thumbnail kg-audio-hide", "kg-audio-thumbnail placeholder")
    } else {
        ("kg-audio-thumbnail", "kg-audio-thumbnail placeholder kg-audio-hide")
    };
    format!(
        r#"<div class="kg-card kg-audio-card"><img src="{thumbnail}" alt="audio-thumbnail" class="{thumb_class}"><div class="{empty_class}"><svg width="24" height="24" fill="none"><path fill-rule="evenodd" clip-rule="evenodd" d="M7.5 15.33a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm-2.25.75a2.25 2.25 0 1 1 4.5 0 2.25 2.25 0 0 1-4.5 0ZM15 13.83a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm-2.25.75a2.25 2.25 0 1 1 4.5 0 2.25 2.25 0 0 1-4.5 0Z"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M14.486 6.81A2.25 2.25 0 0 1 17.25 9v5.579a.75.75 0 0 1-1.5 0v-5.58a.75.75 0 0 0-.932-.727.755.755 0 0 1-.059.013l-4.465.744a.75.75 0 0 0-.544.72v6.33a.75.75 0 0 1-1.5 0v-6.33a2.25 2.25 0 0 1 1.763-2.194l4.473-.746Z"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M3 1.5a.75.75 0 0 0-.75.75v19.5a.75.75 0 0 0 .75.75h18a.75.75 0 0 0 .75-.75V5.133a.75.75 0 0 0-.225-.535l-.002-.002-3-2.883A.75.75 0 0 0 18 1.5H3ZM1.409.659A2.25 2.25 0 0 1 3 0h15a2.25 2.25 0 0 1 1.568.637l.003.002 3 2.883a2.25 2.25 0 0 1 .679 1.61V21.75A2.25 2.25 0 0 1 21 24H3a2.25 2.25 0 0 1-2.25-2.25V2.25c0-.597.237-1.169.659-1.591Z"></path></svg></div><div class="kg-audio-player-container"><audio src="{src}" preload="metadata"></audio><div class="kg-audio-title">{title}</div><div class="kg-audio-player"><button class="kg-audio-play-icon" aria-label="Play audio"><svg viewBox="0 0 24 24"><path d="{PLAY_PATH}"></path></svg></button><button class="kg-audio-pause-icon kg-audio-hide" aria-label="Pause audio"><svg viewBox="0 0 24 24">{PAUSE_RECTS}</svg></button><span class="kg-audio-current-time">0:00</span><div class="kg-audio-time">/<span class="kg-audio-duration">{duration}</span></div><input type="range" class="kg-audio-seek-slider" max="100" value="0"><button class="kg-audio-playback-rate" aria-label="Adjust playback speed">1×</button><button class="kg-audio-unmute-icon" aria-label="Unmute"><svg viewBox="0 0 24 24"><path d="{UNMUTE_PATH}"></path></svg></button><button class="kg-audio-mute-icon kg-audio-hide" aria-label="Mute"><svg viewBox="0 0 24 24"><path d="{MUTE_PATH}"></path></svg></button><input type="range" class="kg-audio-volume-slider" max="100" value="100"></div></div></div>"#,
        thumbnail = attr(&thumbnail),
        src = attr(&src),
        title = esc(&str_of(node, "title")),
        duration = raw_number(node, "duration"),
    )
}

/// Ghost's video card.
///
/// The `poster` is a transparent pixel rather than the thumbnail: the real
/// thumbnail is a CSS background on the `<video>`, which the explicit aspect
/// ratio sizes, and a poster image would cover it.
fn video_html(node: &Value) -> String {
    const TRANSPARENT_PIXEL: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    let src = str_of(node, "src");
    if src.trim().is_empty() {
        return String::new();
    }
    let caption = str_of(node, "caption");
    // Unlike the other cards, Ghost emits `kg-width-regular` here rather than
    // leaving the default width off, so this cannot use `card_class`.
    let mut class = "kg-card kg-video-card".to_string();
    match str_or(node, "cardWidth", "regular") {
        w if !w.is_empty() => class.push_str(&format!(" kg-width-{}", attr(&w))),
        _ => {}
    }
    if !caption.is_empty() {
        class.push_str(" kg-card-hascaption");
    }
    let thumbnail = str_of(node, "thumbnailSrc");
    let custom_thumbnail = str_of(node, "customThumbnailSrc");
    let shown_thumbnail = if custom_thumbnail.is_empty() { &thumbnail } else { &custom_thumbnail };
    let (width, height) = (positive_int(node, "width"), positive_int(node, "height"));
    let aspect_ratio = match (width, height) {
        (Some(w), Some(h)) => format!("{w} / {h}"),
        _ => "16 / 9".into(),
    };
    // Ghost interpolates these straight into the tag, which writes
    // `width="null"` for a video whose dimensions were never stored. Left off
    // instead, the way the image card above leaves them off.
    let dims = match (width, height) {
        (Some(w), Some(h)) => format!(r#" width="{w}" height="{h}""#),
        _ => String::new(),
    };
    let loop_video = node.get("loop").and_then(Value::as_bool).unwrap_or(false);
    let autoplay = if loop_video { " loop autoplay muted" } else { "" };
    // A looping video plays by itself, so its transport controls start hidden.
    let hide_controls = if loop_video { " kg-video-hide" } else { "" };
    format!(
        r#"<figure class="{class}" data-kg-thumbnail="{thumbnail}" data-kg-custom-thumbnail="{custom_thumbnail}"><div class="kg-video-container"><video src="{src}" poster="{TRANSPARENT_PIXEL}"{dims}{autoplay} playsinline="" preload="metadata" style="aspect-ratio: {aspect_ratio}; background: transparent url('{shown_thumbnail}') 50% 50% / cover no-repeat;"></video><div class="kg-video-overlay"><button class="kg-video-large-play-icon" aria-label="Play video"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{PLAY_PATH}"></path></svg></button></div><div class="kg-video-player-container{hide_controls}"><div class="kg-video-player"><button class="kg-video-play-icon" aria-label="Play video"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{PLAY_PATH}"></path></svg></button><button class="kg-video-pause-icon kg-video-hide" aria-label="Pause video"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{PAUSE_RECTS}</svg></button><span class="kg-video-current-time">0:00</span><div class="kg-video-time"> /<span class="kg-video-duration">{duration}</span></div><input type="range" class="kg-video-seek-slider" max="100" value="0"><button class="kg-video-playback-rate" aria-label="Adjust playback speed">1×</button><button class="kg-video-unmute-icon" aria-label="Unmute"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{UNMUTE_PATH}"></path></svg></button><button class="kg-video-mute-icon kg-video-hide" aria-label="Mute"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="{MUTE_PATH}"></path></svg></button><input type="range" class="kg-video-volume-slider" max="100" value="100"></div></div></div>{figcaption}</figure>"#,
        src = attr(&src),
        thumbnail = attr(&thumbnail),
        custom_thumbnail = attr(&custom_thumbnail),
        shown_thumbnail = attr(shown_thumbnail),
        duration = formatted_duration(node.get("duration").and_then(Value::as_f64).unwrap_or(0.0)),
        figcaption = figcaption(&caption),
    )
}

/// `m:ss`, Ghost's `formattedDuration`.
fn formatted_duration(duration: f64) -> String {
    let minutes = (duration / 60.0).floor();
    let seconds = (duration - minutes * 60.0).floor();
    format!("{minutes:.0}:{seconds:02.0}")
}

/// Ghost's product card.
fn product_html(node: &Value) -> String {
    const STAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12.729,1.2l3.346,6.629,6.44.638a.805.805,0,0,1,.5,1.374l-5.3,5.253,1.965,7.138a.813.813,0,0,1-1.151.935L12,19.934,5.48,23.163a.813.813,0,0,1-1.151-.935L6.294,15.09.99,9.837a.805.805,0,0,1,.5-1.374l6.44-.638L11.271,1.2A.819.819,0,0,1,12.729,1.2Z"></path></svg>"##;
    let (title, description) = (str_of(node, "productTitle"), str_of(node, "productDescription"));
    let image = str_of(node, "productImageSrc");
    let url = str_of(node, "productUrl");
    let button_text = str_of(node, "productButton");
    let button_enabled = node.get("productButtonEnabled").and_then(Value::as_bool).unwrap_or(false);
    let rating_enabled = node.get("productRatingEnabled").and_then(Value::as_bool).unwrap_or(false);
    let button_filled = button_enabled && !url.is_empty() && !button_text.is_empty();
    // Ghost's `isEmpty`: a card with nothing in any of its slots renders as
    // nothing rather than as an empty container.
    if title.is_empty() && description.is_empty() && !button_filled && image.is_empty() && !rating_enabled {
        return String::new();
    }
    let img = if image.is_empty() {
        String::new()
    } else {
        let width = positive_int(node, "productImageWidth").map(|w| format!(r#" width="{w}""#)).unwrap_or_default();
        let height = positive_int(node, "productImageHeight").map(|h| format!(r#" height="{h}""#)).unwrap_or_default();
        format!(
            r#"<img src="{src}"{width}{height} class="kg-product-card-image" loading="lazy">"#,
            src = attr(&image),
        )
    };
    let rating = if rating_enabled {
        let stars = node.get("productStarRating").and_then(Value::as_i64).unwrap_or(5);
        let star_spans: String = (1..=5)
            .map(|i| {
                // The active class is interpolated ahead of the base one, so
                // an inactive star keeps Ghost's leading space in `class`.
                let active = if stars >= i { "kg-product-card-rating-active" } else { "" };
                format!(r#"<span class="{active} kg-product-card-rating-star">{STAR}</span>"#)
            })
            .collect();
        format!(r#"<div class="kg-product-card-rating">{star_spans}</div>"#)
    } else {
        String::new()
    };
    let button = if button_enabled {
        format!(
            r#"<a href="{url}" class="kg-product-card-button kg-product-card-btn-accent" target="_blank" rel="noopener noreferrer"><span>{text}</span></a>"#,
            url = attr(&url),
            text = esc(&button_text),
        )
    } else {
        String::new()
    };
    // Title and description are rich text in the editor, so they pass through
    // as the HTML Ghost stores rather than being escaped.
    format!(
        r#"<div class="kg-card kg-product-card"><div class="kg-product-card-container">{img}<div class="kg-product-card-title-container"><h4 class="kg-product-card-title">{title}</h4></div>{rating}<div class="kg-product-card-description">{description}</div>{button}</div></div>"#,
    )
}

/// Ghost's header card, which has two incompatible generations. `version` is
/// on the node and defaults to 1, so an older document keeps the markup its
/// theme was written against.
fn header_html(node: &Value) -> String {
    match node.get("version").and_then(Value::as_i64).unwrap_or(1) {
        2 => header_v2_html(node),
        _ => header_v1_html(node),
    }
}

fn header_v1_html(node: &Value) -> String {
    let (header, subheader) = (str_of(node, "header"), str_of(node, "subheader"));
    let (button_url, button_text) = (str_of(node, "buttonUrl"), str_of(node, "buttonText"));
    let button = node.get("buttonEnabled").and_then(Value::as_bool).unwrap_or(false)
        && !button_url.is_empty()
        && !button_text.is_empty();
    if header.is_empty() && subheader.is_empty() && !button {
        return String::new();
    }
    let background = str_of(node, "backgroundImageSrc");
    let style = str_or(node, "style", "dark");
    let background_style =
        if style == "image" { format!("background-image: url({background})") } else { String::new() };
    let header_html = if header.is_empty() {
        String::new()
    } else {
        format!(r#"<h2 class="kg-header-card-header" id="{}"> {header} </h2>"#, slugify(&header))
    };
    // A subheader of nothing but trailing line breaks is the editor leaving
    // the field open, not a subheading.
    let has_subheader = !subheader.is_empty() && !strip_trailing_breaks(&subheader).trim().is_empty();
    let subheader_html = if has_subheader {
        format!(r#"<h3 class="kg-header-card-subheader" id="{}"> {subheader} </h3>"#, slugify(&subheader))
    } else {
        String::new()
    };
    let button_html = if button {
        format!(r#"<a class="kg-header-card-button" href="{}"> {} </a>"#, attr(&button_url), esc(&button_text))
    } else {
        String::new()
    };
    format!(
        r#"<div class="kg-card kg-header-card kg-width-full kg-size-{size} kg-style-{style}" data-kg-background-image="{background}" style="{background_style}">{header_html}{subheader_html}{button_html}</div>"#,
        size = attr(&str_or(node, "size", "small")),
        style = attr(&style),
        background = attr(&background),
        background_style = attr(&background_style),
    )
}

fn header_v2_html(node: &Value) -> String {
    let layout = str_or(node, "layout", "full");
    let split = layout == "split";
    let background_color = str_or(node, "backgroundColor", "#000000");
    let background_image = str_of(node, "backgroundImageSrc");
    let mut class = "kg-card kg-header-card kg-v2".to_string();
    if !layout.is_empty() && !split {
        class.push_str(&format!(" kg-width-{}", attr(&layout)));
    }
    if split {
        class.push_str(" kg-layout-split kg-width-full");
        if node.get("swapped").and_then(Value::as_bool).unwrap_or(false) {
            class.push_str(" kg-swapped");
        }
    }
    if layout == "full" {
        class.push_str(" kg-content-wide");
    }
    if split && str_or(node, "backgroundSize", "cover") == "contain" {
        class.push_str(" kg-content-wide");
    }
    let accent = if background_color == "accent" { "kg-style-accent" } else { "" };
    // The split layout shows an image *and* a background colour, so it keeps
    // the colour where the other layouts drop it behind the image.
    let wrapper_style = if background_color != "accent" && (background_image.is_empty() || split) {
        format!(r#" style="background-color: {};""#, attr(&background_color))
    } else {
        String::new()
    };
    let img = if background_image.is_empty() {
        String::new()
    } else {
        format!(
            r#"<picture><img class="kg-header-card-image" src="{}" loading="lazy" alt=""></picture>"#,
            attr(&background_image),
        )
    };
    let text_color = str_or(node, "textColor", "#FFFFFF");
    let (header, subheader) = (str_of(node, "header"), str_of(node, "subheader"));
    let header_html = if header.is_empty() {
        String::new()
    } else {
        format!(
            r#"<h2 id="{id}" class="kg-header-card-heading" style="color: {color};" data-text-color="{color}">{header}</h2>"#,
            id = slugify(&header),
            color = attr(&text_color),
        )
    };
    let subheader_html = if subheader.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p id="{id}" class="kg-header-card-subheading" style="color: {color};" data-text-color="{color}">{subheader}</p>"#,
            id = slugify(&subheader),
            color = attr(&text_color),
        )
    };
    let button_url = str_of(node, "buttonUrl");
    let button_html = if node.get("buttonEnabled").and_then(Value::as_bool).unwrap_or(false)
        && !button_url.trim().is_empty()
    {
        let button_color = str_or(node, "buttonColor", "#ffffff");
        let button_accent = if button_color == "accent" { "kg-style-accent" } else { "" };
        let button_style =
            if button_color == "accent" { String::new() } else { format!("background-color: {button_color};") };
        let button_text_color = str_or(node, "buttonTextColor", "#000000");
        format!(
            r#"<a href="{url}" class="kg-header-card-button {button_accent}" style="{button_style}color: {text_color};" data-button-color="{button_color}" data-button-text-color="{text_color}">{text}</a>"#,
            url = attr(&button_url),
            button_style = attr(&button_style),
            text_color = attr(&button_text_color),
            button_color = attr(&button_color),
            text = esc(&str_of(node, "buttonText")),
        )
    } else {
        String::new()
    };
    let alignment = if str_or(node, "alignment", "center") == "center" { "kg-align-center" } else { "" };
    format!(
        r#"<div class="{class} {accent}"{wrapper_style} data-background-color="{background_color}">{outer_img}<div class="kg-header-card-content">{inner_img}<div class="kg-header-card-text {alignment}">{header_html}{subheader_html}{button_html}</div></div></div>"#,
        background_color = attr(&background_color),
        outer_img = if split { "" } else { &img },
        inner_img = if split { &img } else { "" },
    )
}

/// Ghost's `slugify` for card anchors: tags out, symbols out, spaces to
/// dashes, lowercased. Deliberately not the heading-anchor slug above — that
/// one percent-encodes and deduplicates, and these ids do neither.
fn slugify(str: &str) -> String {
    let mut out = String::with_capacity(str.len());
    let mut in_tag = false;
    for c in str.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            // JavaScript's `\w` is ASCII-only; `\s` covers the whitespace run
            // that collapses to a single dash.
            c if c.is_ascii_alphanumeric() || c == '_' => out.extend(c.to_lowercase()),
            c if c.is_whitespace() => out.push(' '),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join("-")
}

fn strip_trailing_breaks(html: &str) -> String {
    static BREAKS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(<br\s*/?>)+$").unwrap());
    BREAKS.replace(html, "").into_owned()
}

/// Ghost's signup card. Portal binds to the `data-members-*` attributes, and
/// the card ships hidden — the same script reveals it once Portal has loaded,
/// so a visitor never sees a form that cannot submit.
fn signup_html(node: &Value) -> String {
    const LOADING_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" height="24" width="24" viewBox="0 0 24 24"><g stroke-linecap="round" stroke-width="2" fill="currentColor" stroke="none" stroke-linejoin="round" class="nc-icon-wrapper"><g class="nc-loop-dots-4-24-icon-o"><circle cx="4" cy="12" r="3"></circle><circle cx="12" cy="12" r="3"></circle><circle cx="20" cy="12" r="3"></circle></g><style data-cap="butt"> .nc-loop-dots-4-24-icon-o{--animation-duration:0.8s} .nc-loop-dots-4-24-icon-o *{opacity:.4;transform:scale(.75);animation:nc-loop-dots-4-anim var(--animation-duration) infinite} .nc-loop-dots-4-24-icon-o :nth-child(1){transform-origin:4px 12px;animation-delay:-.3s;animation-delay:calc(var(--animation-duration)/-2.666)} .nc-loop-dots-4-24-icon-o :nth-child(2){transform-origin:12px 12px;animation-delay:-.15s;animation-delay:calc(var(--animation-duration)/-5.333)} .nc-loop-dots-4-24-icon-o :nth-child(3){transform-origin:20px 12px} @keyframes nc-loop-dots-4-anim{0%,100%{opacity:.4;transform:scale(.75)}50%{opacity:1;transform:scale(1)}} </style></g></svg>"##;
    let layout = str_or(node, "layout", "wide");
    let split = layout == "split";
    let background_color = str_or(node, "backgroundColor", "#F0F0F0");
    let background_image = str_of(node, "backgroundImageSrc");
    let mut class = "kg-card kg-signup-card".to_string();
    if !layout.is_empty() && !split {
        class.push_str(&format!(" kg-width-{}", attr(&layout)));
    }
    if split {
        class.push_str(" kg-layout-split kg-width-full");
        if node.get("swapped").and_then(Value::as_bool).unwrap_or(false) {
            class.push_str(" kg-swapped");
        }
    }
    if layout == "full" {
        class.push_str(" kg-content-wide");
    }
    if split && str_or(node, "backgroundSize", "cover") == "contain" {
        class.push_str(" kg-content-wide");
    }
    // An accent background is a class, not an inline colour — except in the
    // split layout, which paints a colour beside its image.
    let accent = if (split && background_color == "accent")
        || (!split && background_image.is_empty() && background_color == "accent")
    {
        "kg-style-accent"
    } else {
        ""
    };
    let background_style = if background_color != "accent" && (background_image.is_empty() || split) {
        format!("background-color: {background_color}")
    } else {
        String::new()
    };
    let img = if background_image.is_empty() {
        String::new()
    } else {
        format!(r#"<picture><img class="kg-signup-card-image" src="{}" alt=""></picture>"#, attr(&background_image))
    };
    // The signup card decides its own text colour rather than taking the
    // generated default: on a transparent background the text inherits the
    // theme's colour, unless a background image is covering that background.
    let text_color = if background_color == "transparent" && (split || background_image.is_empty()) {
        String::new()
    } else {
        str_or(node, "textColor", "#000000")
    };
    let text_style =
        if text_color.is_empty() { String::new() } else { format!(r#" style="color: {};""#, attr(&text_color)) };
    let button_color = str_or(node, "buttonColor", "accent");
    let button_accent = if button_color == "accent" { "kg-style-accent" } else { "" };
    let button_style =
        if button_color == "accent" { String::new() } else { format!("background-color: {button_color};") };
    let labels: String = node
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .map(|label| format!(r#"<input data-members-label type="hidden" value="{}">"#, attr(label)))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let button_text = str_or(node, "buttonText", "Subscribe");
    let success = str_or(node, "successMessage", "Email sent! Check your inbox to complete your signup.");
    let form = format!(
        r#"<form class="kg-signup-card-form" data-members-form="signup">{labels}<div class="kg-signup-card-fields"><input class="kg-signup-card-input" id="email" data-members-email="" type="email" required="true" placeholder="jamie@example.com"><button class="kg-signup-card-button {button_accent}" style="{button_style}color: {button_text_color};" type="submit"><span class="kg-signup-card-button-default">{button_text}</span><span class="kg-signup-card-button-loading">{LOADING_ICON}</span></button></div><div class="kg-signup-card-success"{text_style}> {success} </div><div class="kg-signup-card-error"{text_style} data-members-error=""></div></form>"#,
        button_style = attr(&button_style),
        button_text_color = attr(&str_or(node, "buttonTextColor", "#FFFFFF")),
        button_text = esc(&button_text),
        success = esc(&success),
    );
    // Heading, subheading and disclaimer are dropped rather than left empty,
    // so a theme's spacing rules do not reserve room for nothing.
    let (header, subheader, disclaimer) =
        (str_of(node, "header"), str_of(node, "subheader"), str_of(node, "disclaimer"));
    let header_html = if header.is_empty() {
        String::new()
    } else {
        format!(r#"<h2 class="kg-signup-card-heading"{text_style}>{header}</h2>"#)
    };
    let subheader_html = if subheader.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="kg-signup-card-subheading"{text_style}>{subheader}</p>"#)
    };
    let disclaimer_html = if disclaimer.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="kg-signup-card-disclaimer"{text_style}>{disclaimer}</p>"#)
    };
    let alignment = if str_or(node, "alignment", "left") == "center" { "kg-align-center" } else { "" };
    format!(
        r#"<div class="{class} {accent}" data-lexical-signup-form="" style="{background_style}; display: none;">{outer_img}<div class="kg-signup-card-content">{inner_img}<div class="kg-signup-card-text {alignment}">{header_html}{subheader_html}{form}{disclaimer_html}</div></div></div>"#,
        background_style = attr(&background_style),
        outer_img = if split { "" } else { &img },
        inner_img = if split { &img } else { "" },
    )
}

/// Ghost's call-to-action card — the sponsored-content block.
fn cta_html(node: &Value) -> String {
    let layout = str_or(node, "layout", "minimal");
    let text_value = str_of(node, "textValue");
    let image_url = str_of(node, "imageUrl");
    let (button_url, button_text) = (str_of(node, "buttonUrl"), str_or(node, "buttonText", "Learn more"));
    let show_button = bool_or(node, "showButton", true)
        && !button_url.is_empty()
        && !button_text.is_empty();
    // Both colours are interpolated into a class name and a style, so a value
    // that is not a bare word or hex code falls back rather than escaping
    // into the attribute.
    let background_color = match str_or(node, "backgroundColor", "grey") {
        c if is_colour_token(&c) => c,
        _ => "white".to_string(),
    };
    let button_color = match str_or(node, "buttonColor", "#000000") {
        c if is_colour_token(&c) => c,
        _ => "accent".to_string(),
    };
    let button_text_color = str_or(node, "buttonTextColor", "#ffffff");
    let button_accent = if button_color == "accent" { "kg-style-accent" } else { "" };
    let button_style = if button_color == "accent" {
        format!(r#"style="color: {};""#, attr(&button_text_color))
    } else {
        format!(r#"style="background-color: {}; color: {};""#, attr(&button_color), attr(&button_text_color))
    };
    let has_sponsor_label = bool_or(node, "hasSponsorLabel", true);
    let sponsor_label = if has_sponsor_label {
        format!(
            r#"<div class="kg-cta-sponsor-label-wrapper"><div class="kg-cta-sponsor-label"> {} </div></div>"#,
            keep_inline_tags(&str_of(node, "sponsorLabel")),
        )
    } else {
        String::new()
    };
    let link = |content: String| {
        if show_button {
            format!(r#"<a href="{}">{content}</a>"#, attr(&button_url))
        } else {
            content
        }
    };
    let image = if image_url.is_empty() {
        String::new()
    } else {
        let dimensions = match (positive_int(node, "imageWidth"), positive_int(node, "imageHeight")) {
            (Some(w), Some(h)) => format!(r#" data-image-dimensions="{w}x{h}""#),
            _ => String::new(),
        };
        format!(
            r#"<div class="kg-cta-image-container">{}</div>"#,
            link(format!(r#"<img src="{}" alt="CTA Image"{dimensions}>"#, attr(&image_url))),
        )
    };
    let inner = if text_value.is_empty() && !show_button {
        String::new()
    } else {
        let text = if text_value.is_empty() {
            String::new()
        } else {
            format!(r#"<div class="kg-cta-text"> {text_value} </div>"#)
        };
        let button = if show_button {
            format!(
                r#"<a href="{url}" class="kg-cta-button {button_accent}" {button_style}> {text} </a>"#,
                url = attr(&button_url),
                text = esc(&button_text),
            )
        } else {
            String::new()
        };
        format!(r#"<div class="kg-cta-content-inner">{text}{button}</div>"#)
    };
    // Ghost leaves the gaps where these conditional classes are empty, and
    // themes match on the individual names, so the runs of spaces are kept.
    format!(
        r#"<div class="kg-card kg-cta-card kg-cta-bg-{background_color} kg-cta-{layout} {no_dividers} {has_image} {link_accent} {centered}" data-layout="{layout}">{sponsor_label}<div class="kg-cta-content">{image}{inner}</div></div>"#,
        background_color = attr(&background_color),
        layout = attr(&layout),
        no_dividers = if bool_or(node, "showDividers", true) {
            ""
        } else {
            "kg-cta-no-dividers"
        },
        has_image = if image_url.is_empty() { "" } else { "kg-cta-has-img" },
        link_accent = if str_or(node, "linkColor", "text") == "accent" { "kg-cta-link-accent" } else { "" },
        centered = if str_or(node, "alignment", "left") == "center" { "kg-cta-centered" } else { "" },
    )
}

/// A bare word or a 3/6-digit hex code — Ghost's guard on the colour fields
/// that reach a class name or an inline style.
fn is_colour_token(colour: &str) -> bool {
    if let Some(hex) = colour.strip_prefix('#') {
        return matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    !colour.is_empty() && colour.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A stored number as Ghost writes it into markup: `200`, not `200.0`.
fn raw_number(node: &Value, key: &str) -> String {
    node.get(key).filter(|v| v.is_number()).map(Value::to_string).unwrap_or_else(|| "0".into())
}

fn positive_int(node: &Value, key: &str) -> Option<i64> {
    node.get(key).and_then(Value::as_i64).filter(|n| *n > 0)
}

/// A card property with the default its Koenig node declares.
///
/// Ghost builds the node from the serialised document and fills the gaps from
/// the property defaults — and for string properties it does so with `||`,
/// not `??`, so a stored empty string takes the default too. That is not a
/// detail to tidy up: Koenig writes `"textColor": ""` into the document for a
/// header left on its defaults, and reading it literally renders `color: ;`.
fn str_or(node: &Value, key: &str, default: &str) -> String {
    match node.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => default.to_string(),
    }
}

fn bool_or(node: &Value, key: &str, default: bool) -> bool {
    node.get(key).and_then(Value::as_bool).unwrap_or(default)
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
    fn audio_card_renders_the_player_ghosts_script_binds_to() {
        let html = to_html(&doc(
            r#"{"type":"audio","version":1,"src":"/content/media/a.mp3","title":"Episode 1","duration":214,"thumbnailSrc":""}"#,
        ));
        assert!(html.starts_with(r#"<div class="kg-card kg-audio-card">"#), "{html}");
        assert!(html.contains(r#"<audio src="/content/media/a.mp3" preload="metadata"></audio>"#), "{html}");
        assert!(html.contains(r#"<div class="kg-audio-title">Episode 1</div>"#), "{html}");
        // no thumbnail: the real one is hidden and the placeholder is not
        assert!(html.contains(r#"class="kg-audio-thumbnail kg-audio-hide""#), "{html}");
        assert!(html.contains(r#"<div class="kg-audio-thumbnail placeholder">"#), "{html}");
        // the raw second count, which the player formats on load
        assert!(html.contains(r#"<span class="kg-audio-duration">214</span>"#), "{html}");
        assert_eq!(to_html(&doc(r#"{"type":"audio","version":1,"src":"  "}"#)), "");
    }

    #[test]
    fn video_card_carries_the_thumbnail_as_a_background() {
        let html = to_html(&doc(
            r#"{"type":"video","version":1,"src":"/content/media/v.mp4","thumbnailSrc":"/t.jpg","width":1920,"height":1080,"duration":65,"cardWidth":"regular","loop":false}"#,
        ));
        // Ghost emits the default width class here rather than omitting it
        assert!(html.contains(r#"class="kg-card kg-video-card kg-width-regular""#), "{html}");
        assert!(html.contains(r#"width="1920" height="1080""#), "{html}");
        assert!(html.contains("aspect-ratio: 1920 / 1080;"), "{html}");
        assert!(html.contains("url('/t.jpg') 50% 50% / cover no-repeat;"), "{html}");
        assert!(html.contains(r#"<span class="kg-video-duration">1:05</span>"#), "{html}");
        // not looping, so the transport controls are not pre-hidden
        assert!(html.contains(r#"<div class="kg-video-player-container">"#), "{html}");

        let looping = to_html(&doc(
            r#"{"type":"video","version":1,"src":"/v.mp4","loop":true,"thumbnailSrc":"/t.jpg","customThumbnailSrc":"/c.jpg"}"#,
        ));
        assert!(looping.contains(" loop autoplay muted playsinline"), "{looping}");
        assert!(looping.contains(r#"<div class="kg-video-player-container kg-video-hide">"#), "{looping}");
        // a custom thumbnail wins for display, and both are kept as data
        assert!(looping.contains("url('/c.jpg')"), "{looping}");
        assert!(looping.contains(r#"data-kg-thumbnail="/t.jpg" data-kg-custom-thumbnail="/c.jpg""#), "{looping}");
        // no stored dimensions: the attributes are left off, not written "null"
        assert!(!looping.contains("null"), "{looping}");
        assert!(looping.contains("aspect-ratio: 16 / 9;"), "{looping}");
    }

    #[test]
    fn product_card_marks_the_stars_up_to_the_rating() {
        let html = to_html(&doc(
            r#"{"type":"product","version":1,"productTitle":"<b>Thing</b>","productDescription":"<p>Good</p>","productRatingEnabled":true,"productStarRating":3,"productButtonEnabled":true,"productButton":"Buy","productUrl":"https://example.com/buy"}"#,
        ));
        // title and description are rich text, so they are not re-escaped
        assert!(html.contains(r#"<h4 class="kg-product-card-title"><b>Thing</b></h4>"#), "{html}");
        assert!(html.contains(r#"<div class="kg-product-card-description"><p>Good</p></div>"#), "{html}");
        assert_eq!(html.matches("kg-product-card-rating-active").count(), 3, "{html}");
        // an inactive star keeps Ghost's leading space in the class attribute
        assert!(html.contains(r#"<span class=" kg-product-card-rating-star">"#), "{html}");
        assert!(html.contains(r#"<a href="https://example.com/buy" class="kg-product-card-button kg-product-card-btn-accent" target="_blank" rel="noopener noreferrer"><span>Buy</span></a>"#), "{html}");
        // Ghost's isEmpty: every slot empty renders nothing at all
        assert_eq!(to_html(&doc(r#"{"type":"product","version":1}"#)), "");
    }

    #[test]
    fn header_card_renders_per_version() {
        // v1 is the default, and keys its classes off size and style
        let v1 = to_html(&doc(
            r#"{"type":"header","version":1,"size":"large","style":"image","backgroundImageSrc":"/bg.jpg","header":"Big news","subheader":"<br>","buttonEnabled":true,"buttonUrl":"/x","buttonText":"Go"}"#,
        ));
        assert!(v1.contains(r#"class="kg-card kg-header-card kg-width-full kg-size-large kg-style-image""#), "{v1}");
        assert!(v1.contains(r#"style="background-image: url(/bg.jpg)""#), "{v1}");
        assert!(v1.contains(r#"<h2 class="kg-header-card-header" id="big-news"> Big news </h2>"#), "{v1}");
        // a subheader of only line breaks is an empty field, not a subheading
        assert!(!v1.contains("kg-header-card-subheader"), "{v1}");

        let v2 = to_html(&doc(
            r##"{"type":"header","version":2,"layout":"full","alignment":"center","backgroundColor":"#000000","textColor":"#FFFFFF","header":"Hello there","subheader":"Sub","buttonEnabled":true,"buttonUrl":"/x","buttonText":"Go","buttonColor":"accent","buttonTextColor":"#000000"}"##,
        ));
        assert!(v2.contains(r#"class="kg-card kg-header-card kg-v2 kg-width-full kg-content-wide ""#), "{v2}");
        assert!(v2.contains(r#"style="background-color: #000000;""#), "{v2}");
        assert!(v2.contains(r##"<h2 id="hello-there" class="kg-header-card-heading" style="color: #FFFFFF;" data-text-color="#FFFFFF">Hello there</h2>"##), "{v2}");
        assert!(v2.contains(r#"<div class="kg-header-card-text kg-align-center">"#), "{v2}");
        assert!(v2.contains(r#"class="kg-header-card-button kg-style-accent" style="color: #000000;""#), "{v2}");
        assert_eq!(to_html(&doc(r#"{"type":"header","version":1}"#)), "");
    }

    #[test]
    fn signup_card_carries_portals_form_attributes() {
        let html = to_html(&doc(
            r##"{"type":"signup","version":1,"layout":"wide","backgroundColor":"accent","header":"Join","subheader":"Sub","disclaimer":"","buttonText":"Sign up","buttonColor":"accent","buttonTextColor":"#FFFFFF","textColor":"#111111","labels":["vip","beta"]}"##,
        ));
        assert!(html.contains(r#"class="kg-card kg-signup-card kg-width-wide kg-style-accent""#), "{html}");
        assert!(html.contains("data-lexical-signup-form"), "{html}");
        // shipped hidden; Portal's script reveals it once it can submit
        assert!(html.contains(r#"style="; display: none;""#), "{html}");
        assert!(html.contains(r#"<form class="kg-signup-card-form" data-members-form="signup">"#), "{html}");
        assert!(html.contains(r#"<input data-members-label type="hidden" value="vip">"#), "{html}");
        assert!(html.contains(r#"<input data-members-label type="hidden" value="beta">"#), "{html}");
        assert!(html.contains(r#"data-members-email="" type="email" required="true""#), "{html}");
        assert!(html.contains(r#"data-members-error="""#), "{html}");
        assert!(html.contains(r#"<h2 class="kg-signup-card-heading" style="color: #111111;">Join</h2>"#), "{html}");
        // an empty disclaimer is dropped rather than left as an empty <p>
        assert!(!html.contains("kg-signup-card-disclaimer"), "{html}");
        // the node's own default for the string the card always shows
        assert!(html.contains("Email sent! Check your inbox to complete your signup."), "{html}");
        // a valueless attribute still serialises with an empty value
        assert!(html.contains(r#"data-lexical-signup-form="""#), "{html}");
    }

    #[test]
    fn cta_card_keeps_the_class_gaps_themes_match_on() {
        let html = to_html(&doc(
            r##"{"type":"call-to-action","version":1,"layout":"minimal","alignment":"center","backgroundColor":"grey","textValue":"<p>Read this</p>","showButton":true,"showDividers":true,"buttonText":"Learn more","buttonUrl":"https://sponsor.example","buttonColor":"#112233","buttonTextColor":"#ffffff","hasSponsorLabel":true,"sponsorLabel":"<p><span style=\"white-space: pre-wrap;\">SPONSORED</span></p>","linkColor":"accent","imageUrl":"/i.png","imageWidth":600,"imageHeight":400}"##,
        ));
        assert!(html.contains(r#"class="kg-card kg-cta-card kg-cta-bg-grey kg-cta-minimal  kg-cta-has-img kg-cta-link-accent kg-cta-centered""#), "{html}");
        assert!(html.contains(r#"data-layout="minimal""#), "{html}");
        // the sponsor label is rich text reduced to inline tags, as Ghost does
        assert!(html.contains(r#"<div class="kg-cta-sponsor-label"> SPONSORED </div>"#), "{html}");
        // with a button, the image links to the same place
        assert!(html.contains(r#"<a href="https://sponsor.example"><img src="/i.png" alt="CTA Image" data-image-dimensions="600x400"></a>"#), "{html}");
        assert!(html.contains(r#"class="kg-cta-button " style="background-color: #112233; color: #ffffff;"> Learn more </a>"#), "{html}");

        // a colour that is neither a word nor a hex code cannot escape the
        // attribute it is interpolated into
        let injected = to_html(&doc(
            r#"{"type":"call-to-action","version":1,"layout":"immersive","backgroundColor":"white\" onload=\"alert(1)","buttonColor":"x\"","textValue":"hi","showButton":false,"showDividers":false,"hasSponsorLabel":false}"#,
        ));
        assert!(injected.contains("kg-cta-bg-white kg-cta-immersive kg-cta-no-dividers"), "{injected}");
        assert!(!injected.contains("onload"), "{injected}");
    }

    #[test]
    fn tk_renders_as_text_and_cursor_nodes_render_as_nothing() {
        let html = to_html(&doc(
            r#"{"type":"paragraph","version":1,"children":[{"type":"zwnj","text":"","version":1},{"type":"tk","text":"TK","format":1,"version":1},{"type":"text","text":" check this","format":0,"version":1}]}"#,
        ));
        assert_eq!(html, "<p><strong>TK</strong> check this</p>");
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

fn transistor_html(node:&Value)->String {
    let url="https://partner.transistor.fm/ghost/embed/%7Buuid%7D";
    let attrs=r#"width="100%" height="370" frameborder="no" scrolling="no" seamless="""#;
    let inner=format!(r#"<iframe {attrs} title="Transistor podcasts" data-src="{url}" data-kg-transistor-embed=""></iframe><script>{}</script><noscript><iframe {attrs} src="{url}"></iframe></noscript>"#,include_str!("transistor.js"));
    let web=node.pointer("/visibility/web");
    let non_member=web.and_then(|v|v["nonMember"].as_bool()).unwrap_or(false);
    let segment=web.and_then(|v|v["memberSegment"].as_str()).unwrap_or("status:free,status:-free");
    if !non_member&&segment.is_empty(){return String::new()}
    if non_member&&segment=="status:free,status:-free"{return inner}
    let segment=segment.replace('"',"&quot;").replace("-->","--&gt;");
    format!("\n<!--kg-gated-block:begin nonMember:{non_member} memberSegment:\"{segment}\" -->{inner}<!--kg-gated-block:end-->\n")
}
/// Replace per-publication and per-member podcast placeholders at the output boundary.
pub fn personalize_transistor(html:&str,uuid:Option<&str>,site_uuid:Option<&str>)->String {
    let id=uuid.unwrap_or("");
    let mut output=html.replace("/ghost/embed/%7Buuid%7D",&format!("/ghost/embed/{id}")).replace("/ghost/embed/{uuid}",&format!("/ghost/embed/{id}"));
    if let Some(site)=site_uuid.filter(|s|!s.is_empty()){
        let site=percent_encoding::utf8_percent_encode(site,percent_encoding::NON_ALPHANUMERIC);
        output=output.replace(&format!("/ghost/embed/{id}\""),&format!("/ghost/embed/{id}?ctx={site}\""));
    }
    output
}
