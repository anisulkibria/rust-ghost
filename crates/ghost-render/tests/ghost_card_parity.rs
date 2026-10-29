//! Pins the Lexical renderer to Ghost's own output, card for card.
//!
//! `ghost-card-output.json` holds one Lexical document per Koenig card type
//! together with the HTML Ghost produced for it, captured by running each
//! document through `@tryghost/kg-lexical-html-renderer` itself —
//! `ghost-card-output.mjs` beside it is the script that does the capturing,
//! so the fixture can be refreshed against a newer Ghost. Any drift from
//! Ghost — a changed class, a dropped attribute, a different nesting — fails
//! here rather than on someone's published site.
//!
//! The `unsupported` list names cards this renderer deliberately leaves
//! alone, and a test asserts they stay empty rather than becoming a
//! half-built card themes would style as real. It is currently empty: every
//! card in the fixture is rendered, including the Transistor player. Member
//! and publication identity are substituted at the final render boundary.
use serde_json::Value;

fn golden() -> Value {
    serde_json::from_str(include_str!("ghost-card-output.json")).expect("fixture parses")
}

/// Ghost's card templates are indented source, so its output carries runs of
/// newlines and spaces that no browser renders and no theme can select on.
/// Every run collapses to one space, and a run sitting between two tags goes
/// altogether — which is what the browser does with it.
///
/// Deliberately not a full trim: one space between two words is still one
/// space, so a renderer that loses a real space inside a sentence still fails.
fn normalise(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.replace("> <", "><").trim().to_string()
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
        "email-cta", "signup", "transistor",
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
