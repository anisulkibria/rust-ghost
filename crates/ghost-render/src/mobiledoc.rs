//! Mobiledoc → Lexical conversion, for content written before Ghost 5.
//!
//! Ported from `@tryghost/kg-converters` (`mobiledoc-to-lexical.js`, 1.2.8),
//! which Ghost calls from `Post.onSaving`. Ghost no longer renders Mobiledoc
//! at all: a post still stored that way is converted on its next save and the
//! `mobiledoc` column is cleared. Without this, an imported or legacy post
//! whose `html` column happens to be empty has no renderable content — the
//! source is there, and nothing can read it.
//!
//! Mobiledoc's shape, for the code below:
//!
//! ```text
//! { "version": "0.3.1",
//!   "markups": [["strong"], ["a", ["href", "/x/"]]],
//!   "atoms":   [["soft-return", "", {}]],
//!   "cards":   [["image", {"src": "/a.jpg"}]],
//!   "sections": [[1, "p", [[0, [0], 1, "bold text"]]]] }
//! ```
//!
//! A section is `[type, ...]`: 1 is markup (text), 3 a list, 10 a card. A
//! marker inside one is `[textType, openMarkupIndexes, closeCount, value]`,
//! where `textType` 0 is text and 1 an atom reference.
use serde_json::{json, Map, Value};

/// The document Lexical renders as an empty post.
///
/// Ghost substitutes this for an empty Mobiledoc, because Lexical refuses to
/// render a root with no children.
pub fn blank_document() -> Value {
    json!({
        "root": {
            "children": [],
            "direction": Value::Null,
            "format": "",
            "indent": 0,
            "type": "root",
            "version": 1,
        }
    })
}

/// Mobiledoc tag → the Lexical node it becomes.
fn node_for_tag(tag: &str) -> Option<Value> {
    Some(match tag {
        "p" => json!({ "type": "paragraph" }),
        "h1" => json!({ "type": "heading", "tag": "h1" }),
        "h2" => json!({ "type": "heading", "tag": "h2" }),
        "h3" => json!({ "type": "heading", "tag": "h3" }),
        "h4" => json!({ "type": "heading", "tag": "h4" }),
        "h5" => json!({ "type": "heading", "tag": "h5" }),
        "h6" => json!({ "type": "heading", "tag": "h6" }),
        "blockquote" => json!({ "type": "quote" }),
        "aside" => json!({ "type": "aside" }),
        "a" => json!({ "type": "link", "rel": Value::Null, "target": Value::Null,
                       "title": Value::Null, "url": Value::Null }),
        _ => return None,
    })
}

/// Lexical stores text formatting as a bitmask; Mobiledoc as open tags.
fn format_bit(tag: &str) -> u32 {
    match tag {
        "strong" | "b" => 1,
        "em" | "i" => 1 << 1,
        "s" => 1 << 2,
        "u" => 1 << 3,
        "code" => 1 << 4,
        "sub" => 1 << 5,
        "sup" => 1 << 6,
        _ => 0,
    }
}

/// Cards Koenig renamed between the two formats.
fn card_name(name: &str) -> &str {
    match name {
        "code" => "codeblock",
        "hr" => "horizontalrule",
        other => other,
    }
}

/// `{{serialize}}` the way Ghost does: a JSON string, ready for the column.
pub fn to_lexical(serialized_mobiledoc: Option<&str>) -> String {
    convert(serialized_mobiledoc).to_string()
}

/// Convert serialized Mobiledoc into a Lexical document.
///
/// Anything unreadable — absent, empty, not JSON, or carrying no `sections` —
/// becomes the blank document rather than an error, which is upstream's
/// behaviour for everything except a structurally broken parse.
pub fn convert(serialized_mobiledoc: Option<&str>) -> Value {
    let Some(source) = serialized_mobiledoc.map(str::trim).filter(|s| !s.is_empty()) else {
        return blank_document();
    };
    let Ok(mobiledoc) = serde_json::from_str::<Value>(source) else {
        return blank_document();
    };
    let Some(sections) = mobiledoc.get("sections").and_then(Value::as_array) else {
        return blank_document();
    };

    let mut children: Vec<Value> = Vec::new();
    let mut direction = Value::Null;
    for section in sections {
        let Some(section) = section.as_array() else { continue };
        match section.first().and_then(Value::as_i64) {
            // Markup (text) section.
            Some(1) => {
                let child = markup_section(section, &mobiledoc);
                // Ghost only marks the root as left-to-right once some text is
                // actually in it.
                if child.get("children").and_then(Value::as_array).is_some_and(|c| !c.is_empty()) {
                    direction = json!("ltr");
                }
                children.push(child);
            }
            // Image section: Ghost never wrote these, so neither does this.
            Some(2) => {}
            Some(3) => {
                children.push(list_section(section, &mobiledoc));
                // Mobiledoc only ever supported left-to-right.
                direction = json!("ltr");
            }
            Some(10) => {
                if let Some(card) = card_section(section, &mobiledoc) {
                    children.push(card);
                }
            }
            _ => {}
        }
    }

    json!({
        "root": {
            "children": children,
            "direction": direction,
            "format": "",
            "indent": 0,
            "type": "root",
            "version": 1,
        }
    })
}

/// A Lexical container node for a Mobiledoc tag, before its children.
fn empty_node(tag: &str, extra: &[(&str, Value)]) -> Value {
    let params = node_for_tag(tag);
    let mut node = Map::new();
    node.insert("children".into(), json!([]));
    node.insert("direction".into(), json!("ltr"));
    node.insert("format".into(), json!(""));
    node.insert("indent".into(), json!(0));
    if let Some(Value::Object(params)) = &params {
        for (k, v) in params {
            node.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in extra {
        node.insert((*k).to_string(), v.clone());
    }
    // An unmapped tag becomes a node of its own name, as upstream does — it
    // keeps an unknown section visible rather than dropping the text in it.
    let node_type = params
        .as_ref()
        .and_then(|p| p.get("type").cloned())
        .or_else(|| extra.iter().find(|(k, _)| *k == "type").map(|(_, v)| v.clone()))
        .unwrap_or_else(|| json!(tag));
    node.insert("type".into(), node_type);
    node.insert("version".into(), json!(1));
    Value::Object(node)
}

fn text_node(text: &str, format: u32) -> Value {
    json!({
        "detail": 0,
        "format": format,
        "mode": "normal",
        "style": "",
        "text": text,
        "type": "text",
        "version": 1,
    })
}

/// Append a child, and mark the parent left-to-right once it holds real text.
fn embed(parent: &mut Value, child: Value) {
    let has_text = child.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty());
    if let Some(children) = parent.get_mut("children").and_then(Value::as_array_mut) {
        children.push(child);
    }
    if has_text {
        if let Value::Object(m) = parent {
            m.insert("direction".into(), json!("ltr"));
        }
    }
}

fn markup_section(section: &[Value], mobiledoc: &Value) -> Value {
    let tag = section.get(1).and_then(Value::as_str).unwrap_or("p");
    let mut node = empty_node(tag, &[]);
    if let Some(markers) = section.get(2).and_then(Value::as_array) {
        populate(&mut node, markers, mobiledoc);
    }
    node
}

/// Walk a section's markers, rebuilding the text runs and their formatting.
///
/// The state carried across markers is what makes this awkward: Mobiledoc
/// opens markups on one marker and closes them a few markers later, so bold
/// spanning three runs is three markers with one `strong` open across them.
/// Links are the special case — their text collects into a link node that is
/// only attached to the parent when the `a` markup closes.
fn populate(node: &mut Value, markers: &[Value], mobiledoc: &Value) {
    let markups = mobiledoc.get("markups").and_then(Value::as_array);
    let atoms = mobiledoc.get("atoms").and_then(Value::as_array);

    // The markups open at this point, innermost last.
    let mut open: Vec<Value> = Vec::new();
    let mut link_node: Option<Value> = None;
    let mut href: Option<String> = None;
    let mut rel: Option<String> = None;
    let mut link_open = false;

    for marker in markers {
        let Some(marker) = marker.as_array() else { continue };
        let text_type = marker.first().and_then(Value::as_i64).unwrap_or(0);
        let open_indexes = marker.get(1).and_then(Value::as_array).cloned().unwrap_or_default();
        let close_count = marker.get(2).and_then(Value::as_i64).unwrap_or(0);
        let value = marker.get(3);

        // textType 1 is an atom reference. Ghost only ever used `soft-return`.
        if text_type != 0 {
            let atom = value
                .and_then(Value::as_u64)
                .and_then(|index| atoms.and_then(|a| a.get(index as usize)))
                .and_then(Value::as_array);
            let name = atom.and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("");
            if name == "soft-return" {
                embed(node, json!({ "type": "linebreak", "version": 1 }));
            }
            continue;
        }

        for index in &open_indexes {
            let Some(markup) = index
                .as_u64()
                .and_then(|i| markups.and_then(|m| m.get(i as usize)))
                .and_then(Value::as_array)
            else {
                continue;
            };
            if markup.first().and_then(Value::as_str) == Some("a") {
                link_open = true;
                // Attributes are a flat `[name, value, name, value]` list.
                let attrs = markup.get(1).and_then(Value::as_array);
                if let Some(attrs) = attrs {
                    if attrs.first().and_then(Value::as_str) == Some("href") {
                        href = attrs.get(1).and_then(Value::as_str).map(str::to_string);
                    }
                    if attrs.get(2).and_then(Value::as_str) == Some("rel") {
                        rel = attrs.get(3).and_then(Value::as_str).map(str::to_string);
                    }
                }
            }
            open.push(Value::Array(markup.clone()));
        }

        if let Some(text) = value.and_then(Value::as_str) {
            let format = open
                .iter()
                .filter_map(|m| m.get(0).and_then(Value::as_str))
                .fold(0u32, |acc, tag| acc | format_bit(tag));
            if link_open {
                let mut link = link_node.take().unwrap_or_else(|| {
                    empty_node(
                        "a",
                        &[
                            ("url", href.clone().map(Value::String).unwrap_or(Value::Null)),
                            ("rel", rel.clone().map(Value::String).unwrap_or(Value::Null)),
                        ],
                    )
                });
                embed(&mut link, text_node(text, format));
                link_node = Some(link);
            } else {
                embed(node, text_node(text, format));
            }
        }

        for _ in 0..close_count {
            let closed = open.pop();
            if closed.as_ref().and_then(|m| m.get(0)).and_then(Value::as_str) == Some("a") {
                if let Some(link) = link_node.take() {
                    embed(node, link);
                }
                link_open = false;
                href = None;
                rel = None;
            }
        }
    }

    // A link left open at the end of the section still has to be attached, or
    // its text vanishes. Mobiledoc from the wild is not always balanced.
    if let Some(link) = link_node.take() {
        embed(node, link);
    }
}

fn list_section(section: &[Value], mobiledoc: &Value) -> Value {
    let tag = section.get(1).and_then(Value::as_str).unwrap_or("ul");
    let list_type = if tag == "ul" { "bullet" } else { "number" };
    let mut list = empty_node(
        tag,
        &[
            ("tag", json!(tag)),
            ("type", json!("list")),
            ("listType", json!(list_type)),
            ("start", json!(1)),
            ("direction", json!("ltr")),
        ],
    );
    if let Some(items) = section.get(2).and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            let mut item_node = empty_node(
                "li",
                &[("type", json!("listitem")), ("value", json!(index + 1)), ("direction", json!("ltr"))],
            );
            if let Some(markers) = item.as_array() {
                populate(&mut item_node, markers, mobiledoc);
            }
            if let Some(children) = list.get_mut("children").and_then(Value::as_array_mut) {
                children.push(item_node);
            }
        }
    }
    list
}

/// A card becomes a Lexical decorator node: its payload flattened onto a node
/// named after the card.
fn card_section(section: &[Value], mobiledoc: &Value) -> Option<Value> {
    let index = section.get(1).and_then(Value::as_u64)? as usize;
    let card = mobiledoc.get("cards").and_then(Value::as_array)?.get(index)?.as_array()?;
    let name = card_name(card.first().and_then(Value::as_str)?);
    let mut payload = match card.get(1) {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };

    // Properties Koenig renamed.
    if name == "embed" {
        if let Some(embed_type) = payload.remove("type") {
            payload.insert("embedType".into(), embed_type);
        }
    }
    // A callout's background colour is written into a class name, so a value
    // that is not a bare token would break the markup. Upstream drops it to
    // white, and so does the Lexical renderer in this port.
    if name == "callout" {
        let bad = payload
            .get("backgroundColor")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'));
        if bad {
            payload.insert("backgroundColor".into(), json!("white"));
        }
    }
    // `type` names the node, so a payload carrying one of its own would
    // rename the card.
    payload.remove("type");

    let mut node = Map::new();
    node.insert("type".into(), json!(name));
    for (k, v) in payload {
        node.insert(k, v);
    }
    Some(Value::Object(node))
}

/// Migrate every post still stored as Mobiledoc to Lexical.
///
/// Ghost converts a post on its next save, which means a legacy post nobody
/// edits is never migrated — and if its `html` column is also empty, it has no
/// renderable content at all. Ghost covers that with a one-off migration
/// (`2022-05-21-00-00-regenerate-posts-html.js`); this is the equivalent, run
/// once at startup.
///
/// Deliberately conservative about `html`: a post whose HTML already renders
/// keeps it, and only the editable source is migrated. Regenerating markup
/// that currently works would risk changing live pages for no gain.
///
/// Returns how many posts were converted, for the startup log. A row that
/// cannot be converted is left exactly as it was rather than failing the boot.
pub async fn backfill_legacy_posts(pool: &sqlx::SqlitePool) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT id, mobiledoc, html FROM posts \
         WHERE mobiledoc IS NOT NULL AND TRIM(mobiledoc) NOT IN ('', '{}') \
           AND (lexical IS NULL OR TRIM(lexical) = '')",
    )
    .fetch_all(pool)
    .await?;

    let mut converted = 0u64;
    for (id, mobiledoc, html) in rows {
        let doc = convert(Some(&mobiledoc));
        // An empty conversion means there was nothing in the Mobiledoc after
        // all; writing a blank document over the row would lose nothing but
        // gains nothing either.
        if doc["root"]["children"].as_array().is_none_or(|c| c.is_empty()) {
            continue;
        }
        let lexical = doc.to_string();
        let needs_html = html.as_deref().unwrap_or("").trim().is_empty();
        let result = if needs_html {
            let rendered = crate::lexical::to_html(&lexical);
            let plaintext = crate::markdown::html_to_plaintext(&rendered);
            let reading_time = crate::markdown::reading_time(&plaintext);
            sqlx::query(
                "UPDATE posts SET lexical = ?, html = ?, plaintext = ?, reading_time = ? WHERE id = ?",
            )
            .bind(&lexical)
            .bind(&rendered)
            .bind(&plaintext)
            .bind(reading_time)
            .bind(&id)
            .execute(pool)
            .await
        } else {
            sqlx::query("UPDATE posts SET lexical = ? WHERE id = ?").bind(&lexical).bind(&id).execute(pool).await
        };
        if result.is_ok() {
            converted += 1;
        }
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_children(doc: &Value) -> &Vec<Value> {
        doc["root"]["children"].as_array().unwrap()
    }

    #[test]
    fn nothing_readable_becomes_the_blank_document() {
        for source in [None, Some(""), Some("   "), Some("not json"), Some("{}")] {
            assert_eq!(convert(source), blank_document(), "{source:?} has no content to convert");
        }
    }

    #[test]
    fn a_blank_document_has_a_null_direction() {
        // Lexical treats direction as unset until there is text; Ghost's
        // editor writes null here and a bare "" would not round-trip.
        assert_eq!(blank_document()["root"]["direction"], Value::Null);
    }

    #[test]
    fn a_paragraph_becomes_a_paragraph_with_a_text_node() {
        let doc = convert(Some(
            r#"{"version":"0.3.1","markups":[],"atoms":[],"cards":[],
                "sections":[[1,"p",[[0,[],0,"Hello world"]]]]}"#,
        ));
        let children = root_children(&doc);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["type"], json!("paragraph"));
        assert_eq!(children[0]["children"][0]["text"], json!("Hello world"));
        assert_eq!(children[0]["children"][0]["format"], json!(0));
        assert_eq!(doc["root"]["direction"], json!("ltr"), "text makes the root ltr");
    }

    #[test]
    fn headings_keep_their_level() {
        let doc = convert(Some(
            r#"{"sections":[[1,"h2",[[0,[],0,"Title"]]],[1,"h6",[[0,[],0,"Small"]]]]}"#,
        ));
        let children = root_children(&doc);
        assert_eq!(children[0]["type"], json!("heading"));
        assert_eq!(children[0]["tag"], json!("h2"));
        assert_eq!(children[1]["tag"], json!("h6"));
    }

    #[test]
    fn a_blockquote_becomes_a_quote() {
        let doc = convert(Some(r#"{"sections":[[1,"blockquote",[[0,[],0,"Said"]]]]}"#));
        assert_eq!(root_children(&doc)[0]["type"], json!("quote"));
    }

    #[test]
    fn markups_become_a_lexical_format_bitmask() {
        // strong + em, which is how Ghost wrote bold italic text.
        let doc = convert(Some(
            r#"{"markups":[["strong"],["em"]],
                "sections":[[1,"p",[[0,[0,1],2,"both"]]]]}"#,
        ));
        assert_eq!(root_children(&doc)[0]["children"][0]["format"], json!(1 | 2));
    }

    #[test]
    fn every_markup_tag_maps_to_its_upstream_bit() {
        assert_eq!(format_bit("strong"), 1);
        assert_eq!(format_bit("b"), 1, "b and strong are the same format");
        assert_eq!(format_bit("em"), 2);
        assert_eq!(format_bit("i"), 2);
        assert_eq!(format_bit("s"), 4);
        assert_eq!(format_bit("u"), 8);
        assert_eq!(format_bit("code"), 16);
        assert_eq!(format_bit("sub"), 32);
        assert_eq!(format_bit("sup"), 64);
        assert_eq!(format_bit("a"), 0, "a link is a node, not a format");
    }

    #[test]
    fn a_format_opened_across_several_runs_stays_open() {
        // One `strong` opened on the first marker and closed on the last: both
        // runs are bold, which is what the open-markup stack is for.
        let doc = convert(Some(
            r#"{"markups":[["strong"]],
                "sections":[[1,"p",[[0,[0],0,"one "],[0,[],1,"two"]]]]}"#,
        ));
        let runs = root_children(&doc)[0]["children"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["format"], json!(1));
        assert_eq!(runs[1]["format"], json!(1), "the markup was still open");
    }

    #[test]
    fn a_link_becomes_a_link_node_carrying_its_text() {
        let doc = convert(Some(
            r#"{"markups":[["a",["href","/about/"]]],
                "sections":[[1,"p",[[0,[0],1,"About us"]]]]}"#,
        ));
        let link = &root_children(&doc)[0]["children"][0];
        assert_eq!(link["type"], json!("link"));
        assert_eq!(link["url"], json!("/about/"));
        assert_eq!(link["children"][0]["text"], json!("About us"));
    }

    #[test]
    fn a_links_rel_is_carried_over() {
        let doc = convert(Some(
            r#"{"markups":[["a",["href","https://x.test/","rel","nofollow"]]],
                "sections":[[1,"p",[[0,[0],1,"x"]]]]}"#,
        ));
        assert_eq!(root_children(&doc)[0]["children"][0]["rel"], json!("nofollow"));
    }

    #[test]
    fn text_around_a_link_stays_outside_it() {
        let doc = convert(Some(
            r#"{"markups":[["a",["href","/x/"]]],
                "sections":[[1,"p",[[0,[],0,"see "],[0,[0],1,"this"],[0,[],0," now"]]]]}"#,
        ));
        let children = root_children(&doc)[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0]["text"], json!("see "));
        assert_eq!(children[1]["type"], json!("link"));
        assert_eq!(children[2]["text"], json!(" now"));
    }

    #[test]
    fn an_unclosed_link_is_still_attached() {
        // Mobiledoc from old imports is not always balanced; the text in a
        // link that never closes must not disappear.
        let doc = convert(Some(
            r#"{"markups":[["a",["href","/x/"]]],
                "sections":[[1,"p",[[0,[0],0,"dangling"]]]]}"#,
        ));
        let children = root_children(&doc)[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["children"][0]["text"], json!("dangling"));
    }

    #[test]
    fn a_soft_return_atom_becomes_a_linebreak() {
        let doc = convert(Some(
            r#"{"atoms":[["soft-return","",{}]],
                "sections":[[1,"p",[[0,[],0,"one"],[1,[],0,0],[0,[],0,"two"]]]]}"#,
        ));
        let children = root_children(&doc)[0]["children"].as_array().unwrap();
        assert_eq!(children[1]["type"], json!("linebreak"));
        assert_eq!(children[2]["text"], json!("two"));
    }

    #[test]
    fn a_bullet_list_becomes_a_lexical_list() {
        let doc = convert(Some(
            r#"{"sections":[[3,"ul",[[[0,[],0,"first"]],[[0,[],0,"second"]]]]]}"#,
        ));
        let list = &root_children(&doc)[0];
        assert_eq!(list["type"], json!("list"));
        assert_eq!(list["listType"], json!("bullet"));
        assert_eq!(list["tag"], json!("ul"));
        let items = list["children"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], json!("listitem"));
        assert_eq!(items[0]["value"], json!(1));
        assert_eq!(items[1]["value"], json!(2), "listitem values are 1-based");
        assert_eq!(items[0]["children"][0]["text"], json!("first"));
    }

    #[test]
    fn an_ordered_list_is_numbered() {
        let doc = convert(Some(r#"{"sections":[[3,"ol",[[[0,[],0,"a"]]]]]}"#));
        assert_eq!(root_children(&doc)[0]["listType"], json!("number"));
    }

    #[test]
    fn a_card_becomes_a_decorator_node_with_its_payload_inlined() {
        let doc = convert(Some(
            r#"{"cards":[["image",{"src":"/content/images/a.jpg","caption":"A"}]],
                "sections":[[10,0]]}"#,
        ));
        let card = &root_children(&doc)[0];
        assert_eq!(card["type"], json!("image"));
        assert_eq!(card["src"], json!("/content/images/a.jpg"));
        assert_eq!(card["caption"], json!("A"));
    }

    #[test]
    fn renamed_cards_use_their_lexical_names() {
        let doc = convert(Some(r#"{"cards":[["code",{"code":"x=1"}],["hr",{}]],
                                   "sections":[[10,0],[10,1]]}"#));
        assert_eq!(root_children(&doc)[0]["type"], json!("codeblock"));
        assert_eq!(root_children(&doc)[1]["type"], json!("horizontalrule"));
    }

    #[test]
    fn an_embeds_type_property_is_renamed() {
        // `type` names the Lexical node, so the card's own `type` has to move
        // out of the way or the node would be called "video".
        let doc = convert(Some(
            r#"{"cards":[["embed",{"type":"video","url":"https://x.test/v"}]],
                "sections":[[10,0]]}"#,
        ));
        let card = &root_children(&doc)[0];
        assert_eq!(card["type"], json!("embed"));
        assert_eq!(card["embedType"], json!("video"));
        assert_eq!(card["url"], json!("https://x.test/v"));
    }

    #[test]
    fn a_callouts_unusable_background_colour_falls_back_to_white() {
        let doc = convert(Some(
            r#"{"cards":[["callout",{"backgroundColor":"rgba(0,0,0,.5)","calloutText":"x"}]],
                "sections":[[10,0]]}"#,
        ));
        assert_eq!(root_children(&doc)[0]["backgroundColor"], json!("white"));
    }

    #[test]
    fn a_usable_background_colour_is_kept() {
        let doc = convert(Some(
            r#"{"cards":[["callout",{"backgroundColor":"blue-100"}]],"sections":[[10,0]]}"#,
        ));
        assert_eq!(root_children(&doc)[0]["backgroundColor"], json!("blue-100"));
    }

    #[test]
    fn a_card_index_with_no_card_behind_it_is_skipped() {
        let doc = convert(Some(r#"{"cards":[],"sections":[[10,4],[1,"p",[[0,[],0,"kept"]]]]}"#));
        let children = root_children(&doc);
        assert_eq!(children.len(), 1, "the dangling card reference is dropped, the paragraph is not");
        assert_eq!(children[0]["type"], json!("paragraph"));
    }

    #[test]
    fn image_sections_are_ignored_as_upstream_ignores_them() {
        let doc = convert(Some(r#"{"sections":[[2,"/a.jpg"],[1,"p",[[0,[],0,"x"]]]]}"#));
        assert_eq!(root_children(&doc).len(), 1);
    }

    #[test]
    fn a_whole_legacy_post_converts_in_order() {
        let doc = convert(Some(
            r#"{"version":"0.3.1",
                "markups":[["strong"],["a",["href","/next/"]]],
                "atoms":[],
                "cards":[["hr",{}]],
                "sections":[
                  [1,"h2",[[0,[],0,"Heading"]]],
                  [1,"p",[[0,[0],1,"Bold"],[0,[],0," then "],[0,[1],1,"a link"]]],
                  [10,0],
                  [3,"ul",[[[0,[],0,"item"]]]]
                ]}"#,
        ));
        let children = root_children(&doc);
        assert_eq!(children.len(), 4);
        assert_eq!(children[0]["type"], json!("heading"));
        assert_eq!(children[1]["type"], json!("paragraph"));
        assert_eq!(children[1]["children"][0]["format"], json!(1));
        assert_eq!(children[1]["children"][2]["type"], json!("link"));
        assert_eq!(children[2]["type"], json!("horizontalrule"));
        assert_eq!(children[3]["type"], json!("list"));
    }

    #[test]
    fn sections_with_no_sections_key_convert_to_blank() {
        assert_eq!(convert(Some(r#"{"version":"0.3.1","markups":[]}"#)), blank_document());
    }

    #[test]
    fn to_lexical_returns_a_serialized_document() {
        let serialized = to_lexical(Some(r#"{"sections":[[1,"p",[[0,[],0,"x"]]]]}"#));
        let parsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["root"]["children"][0]["children"][0]["text"], json!("x"));
    }
}
