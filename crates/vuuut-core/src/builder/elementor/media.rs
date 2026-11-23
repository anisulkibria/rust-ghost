//! Match missing Elementor image URLs to the images in a companion HTML file.
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Default)]
struct Images(RefCell<Vec<Image>>);

struct Image {
    src: String,
    alt: String,
}

impl TokenSink for Images {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }
        match tag.name.as_ref() {
            "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                return TokenSinkResult::RawData(RawKind::Rawtext);
            }
            "title" | "textarea" => return TokenSinkResult::RawData(RawKind::Rcdata),
            "plaintext" => return TokenSinkResult::Plaintext,
            "img" => {}
            _ => return TokenSinkResult::Continue,
        }
        let attr = |name: &str| {
            tag.attrs
                .iter()
                .find(|item| &*item.name.local == name)
                .map(|item| item.value.as_ref())
                .unwrap_or_default()
        };
        let url = attr("src").trim();
        let scheme = url
            .split_once("://")
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        // Keep relative and non-HTTP references in the list so they cannot
        // shift the positional match for the surrounding images.
        let src = if matches!(scheme.as_deref(), Some("http" | "https")) {
            super::super::safe_url(url).unwrap_or_default()
        } else {
            String::new()
        };
        self.0.borrow_mut().push(Image {
            src,
            alt: attr("alt").trim().to_owned(),
        });
        TokenSinkResult::Continue
    }
}

fn image_slots<'a>(node: &'a mut Value, slots: &mut Vec<&'a mut Map<String, Value>>) {
    if matches!(node["t"].as_str(), Some("image" | "image-box"))
        && node["elementor"]["source"]["settings"]["image"].is_object()
    {
        if let Some(settings) = node["s"].as_object_mut() {
            slots.push(settings);
        }
    } else if let Some(fields) = node.as_object_mut() {
        for (key, value) in fields {
            if matches!(key.as_str(), "sections" | "cols" | "items") {
                for child in value.as_array_mut().into_iter().flatten() {
                    image_slots(child, slots);
                }
            }
        }
    }
}

fn label(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn existing_src(slot: &Map<String, Value>) -> Option<&str> {
    slot.get("src")
        .and_then(Value::as_str)
        .filter(|src| !src.is_empty())
}

/// Fill empty slots only. Exact, unique descriptions take precedence.
/// Position is used when the files have the same count and all known
/// descriptions or URLs agree with that order.
pub(super) fn restore(layout: &mut Value, html: Option<&str>) -> usize {
    let mut slots = Vec::new();
    image_slots(layout, &mut slots);
    let missing = slots
        .iter()
        .filter(|slot| existing_src(slot).is_none())
        .count();
    let Some(html) = html.filter(|_| missing > 0) else {
        return missing;
    };

    let queue = BufferQueue::default();
    queue.push_back(html.into());
    let tokenizer = Tokenizer::new(Images::default(), Default::default());
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    let images = tokenizer.sink.0.into_inner();

    let labels: Vec<_> = slots
        .iter()
        .map(|slot| {
            ["alt", "caption"]
                .into_iter()
                .find_map(|key| {
                    slot.get(key)
                        .and_then(Value::as_str)
                        .map(label)
                        .filter(|text| !text.is_empty())
                })
                .unwrap_or_default()
        })
        .collect();
    let mut label_count = HashMap::new();
    for text in &labels {
        *label_count.entry(text.as_str()).or_insert(0) += 1;
    }
    let mut by_alt = HashMap::new();
    for (index, image) in images.iter().enumerate() {
        by_alt
            .entry(label(&image.alt))
            .and_modify(|same| *same = None)
            .or_insert(Some(index));
    }
    let by_label: Vec<_> = labels
        .iter()
        .map(|text| {
            (!text.is_empty() && label_count[text.as_str()] == 1)
                .then(|| by_alt.get(text).copied().flatten())
                .flatten()
        })
        .collect();
    let by_position = slots.len() == images.len()
        && slots.iter().enumerate().all(|(index, slot)| {
            let photo = &images[index];
            by_label[index].is_none_or(|found| found == index)
                && (labels[index].is_empty() || labels[index] == label(&photo.alt))
                && existing_src(slot).is_none_or(|src| src == photo.src)
        });

    let mut unrecovered = missing;
    for (index, slot) in slots.into_iter().enumerate() {
        if existing_src(slot).is_some() {
            continue;
        }
        let Some(photo) = by_label[index]
            .or_else(|| by_position.then_some(index))
            .and_then(|found| images.get(found))
            .filter(|photo| !photo.src.is_empty())
        else {
            continue;
        };
        slot.insert("src".into(), json!(photo.src));
        if slot
            .get("caption")
            .and_then(Value::as_str)
            .is_some_and(|text| label(text) == label(&photo.alt))
        {
            slot.remove("caption");
        }
        if slot
            .get("alt")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            && !photo.alt.is_empty()
        {
            slot.insert(
                "alt".into(),
                json!(photo.alt.chars().take(300).collect::<String>()),
            );
        }
        unrecovered -= 1;
    }
    unrecovered
}
