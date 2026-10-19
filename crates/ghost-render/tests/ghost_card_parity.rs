//! Pins the Lexical renderer to Ghost's own output, card for card.
//!
//! `ghost-card-output.json` holds one Lexical document per Koenig card type
//! together with the HTML Ghost 6.39.0 produced for it, captured by running
//! each document through `@tryghost/kg-lexical-html-renderer` itself. Any
//! drift from Ghost — a changed class, a dropped attribute, a different
//! nesting — fails here rather than on someone's published site.
//!
//! The `unsupported` list names the cards this renderer deliberately leaves
//! alone: several kilobytes of templated markup each, with branches for
//! layout, colour and thumbnail state. They are written by the editor, which
//! runs Koenig's own renderer, so the only way to reach this path with one of
//! them is content imported from elsewhere. A test asserts they stay empty
//! rather than becoming a half-built card that themes would style as real.
use serde_json::Value;

fn golden() -> Value {
    serde_json::from_str(include_str!("ghost-card-output.json")).expect("fixture parses")
}

/// Whitespace between block-level tags does not change the rendered page, and
/// Ghost's own templates are indented; compare without it.
fn normalise(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '>' {
            out.push('>');
            let mut seen_space = false;
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
                seen_space = true;
            }
            if seen_space && chars.peek() != Some(&'<') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[test]
fn every_supported_card_matches_ghost_exactly() {
    let golden = golden();
    let cases = golden["cases"].as_object().expect("cases");
    let supported: Vec<&str> = golden["meta"]["supported"].as_array().unwrap().iter().filter_map(Value::as_str).collect();

    let mut failures = Vec::new();
    for name in &supported {
        let case = &cases[*name];
        let ours = ghost_render::lexical::to_html(case["lexical"].as_str().unwrap());
        let theirs = case["html"].as_str().unwrap();
        if normalise(&ours) != normalise(theirs) {
            failures.push(format!("\n--- {name}\n ghost: {}\n ours : {}", normalise(theirs), normalise(&ours)));
        }
    }
    assert!(failures.is_empty(), "{} card(s) drifted from Ghost:{}", failures.len(), failures.concat());
}

#[test]
fn unsupported_cards_are_left_out_rather_than_guessed_at() {
    let golden = golden();
    let cases = golden["cases"].as_object().expect("cases");
    for name in golden["meta"]["unsupported"].as_array().unwrap().iter().filter_map(Value::as_str) {
        let ours = ghost_render::lexical::to_html(cases[name]["lexical"].as_str().unwrap());
        assert_eq!(ours, "", "{name} produced markup this renderer cannot get right");
    }
}

#[test]
fn the_fixture_covers_every_card_ghost_ships() {
    // The node types in Ghost 6.39.0's DEFAULT_NODES, minus the ones that are
    // text-level rather than cards (extended-text and friends) and the two
    // internal editor nodes (at-link, zwnj, tk).
    const CARDS: &[&str] = &[
        "codeblock", "image", "markdown", "video", "audio", "callout", "call-to-action", "horizontalrule", "html",
        "file", "toggle", "button", "header", "bookmark", "paywall", "product", "embed", "email", "gallery",
        "email-cta", "signup",
    ];
    let golden = golden();
    let documents: String = golden["cases"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| c["lexical"].as_str().unwrap_or_default())
        .collect();
    for card in CARDS {
        assert!(
            documents.contains(&format!(r#""type":"{card}""#)),
            "no fixture exercises the {card} card — add one before trusting this suite"
        );
    }
}
