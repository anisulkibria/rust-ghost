//! What a stored layout cannot know until a page is viewed.
//!
//! Layout HTML is rendered once, on save, and stored. Three things in it are
//! left as markers for `ghost_front` to resolve on each view:
//!
//! * **Display conditions** — `<!--vb-if:ID:who=paid;from=…-->…<!--vb-end:ID-->`
//!   around an element; [`apply_conditions`] keeps or drops the block for
//!   the visitor in front of it.
//! * **Dynamic tags** — `[[post.title]]` typed into a text becomes the
//!   placeholder `%%vb:post.title%%` ([`mark_tags`]); [`fill_tags`] puts the
//!   escaped value in.
//! * **Dynamic widgets** — posts, menus, breadcrumbs… are an empty
//!   `<div class="vb-dyn" data-vb-dyn="kind" …>` carrying their settings;
//!   [`replace_dyn`] hands each to a closure that draws it.
//!
//! None of these markers can be produced by a visitor or by a text field: the
//! renderer escapes `<`, so `<!--` never comes from content, and `%%vb:` is
//! only written for a tag name on the allow-list.

use super::esc;
use serde_json::Value;

/// Tags that may appear in text, with their labels for the editor's picker.
pub const TEXT_TAGS: &[(&str, &str)] = &[
    ("post.title", "Post title"),
    ("post.excerpt", "Post excerpt"),
    ("post.date", "Publish date"),
    ("post.updated", "Last updated"),
    ("post.reading_time", "Reading time"),
    ("post.author", "Author name"),
    ("post.author_bio", "Author bio"),
    ("post.tags", "Tags"),
    ("post.primary_tag", "Primary tag"),
    ("site.title", "Site title"),
    ("site.description", "Site description"),
    ("member.name", "Member's first name"),
    ("year", "Current year"),
    ("today", "Today's date"),
];

/// Tags that may stand for a whole link or image address.
pub const URL_TAGS: &[&str] = &[
    "post.url",
    "post.feature_image",
    "post.author_url",
    "post.author_image",
    "post.primary_tag_url",
    "site.url",
    "site.logo",
    "site.icon",
    "site.cover",
];

pub fn placeholder(name: &str) -> String {
    format!("%%vb:{name}%%")
}

fn known(name: &str) -> bool {
    TEXT_TAGS.iter().any(|(k, _)| *k == name) || URL_TAGS.contains(&name)
}

/// In already-escaped text, turn each `[[known.tag]]` into its placeholder.
/// Unknown names are left as typed.
pub fn mark_tags(escaped: &str) -> String {
    if !escaped.contains("[[") {
        return escaped.to_string();
    }
    let mut out = String::with_capacity(escaped.len());
    let mut rest = escaped;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("]]") {
            Some(j) if known(after[..j].trim()) => {
                out.push_str(&placeholder(after[..j].trim()));
                rest = &after[j + 2..];
            }
            _ => {
                out.push_str("[[");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replace every placeholder with `get(name)`, escaped. A name with no value
/// becomes empty.
pub fn fill_tags(html: &str, get: impl Fn(&str) -> Option<String>) -> String {
    if !html.contains("%%vb:") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find("%%vb:") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 5..];
        match after.find("%%") {
            Some(j) if j <= 40 && known(&after[..j]) => {
                out.push_str(&esc(&get(&after[..j]).unwrap_or_default()));
                rest = &after[j + 2..];
            }
            _ => {
                out.push_str("%%vb:");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// display conditions
// ---------------------------------------------------------------------------

/// The condition an element's settings ask for, encoded for its marker, or
/// `None` when it shows to everyone always.
pub fn cond_of(s: &Value) -> Option<String> {
    let mut parts = Vec::new();
    match s["if_who"].as_str() {
        Some("any") | None => {}
        Some(w) => parts.push(format!("who={w}")),
    }
    for (k, key) in [("from", "if_from"), ("until", "if_until")] {
        if let Some(v) = s[key].as_str().filter(|v| !v.is_empty()) {
            parts.push(format!("{k}={v}"));
        }
    }
    if let Some(d) = s["if_days"].as_str() {
        let d: String = d.chars().filter(|c| ('1'..='7').contains(c)).collect();
        if !d.is_empty() && d.len() < 7 {
            parts.push(format!("days={d}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(";"))
}

pub fn open_marker(id: &str, cond: &str) -> String {
    format!("<!--vb-if:{id}:{cond}-->")
}

pub fn close_marker(id: &str) -> String {
    format!("<!--vb-end:{id}-->")
}

/// Who is looking and when.
pub struct Facts<'a> {
    /// `""` for a visitor, else the member's status: `free`, `paid`, `comped`.
    pub member: &'a str,
    /// Now, UTC, `YYYY-MM-DDTHH:MM`.
    pub now: String,
    /// 1 = Monday … 7 = Sunday.
    pub weekday: u32,
}

impl Facts<'_> {
    pub fn now(member: &str) -> Facts<'_> {
        use chrono::Datelike;
        let now = chrono::Utc::now();
        Facts { member, now: now.format("%Y-%m-%dT%H:%M").to_string(), weekday: now.weekday().number_from_monday() }
    }
}

pub fn holds(cond: &str, f: &Facts) -> bool {
    for part in cond.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let ok = match k {
            "who" => match v {
                "visitors" => f.member.is_empty(),
                "members" => !f.member.is_empty(),
                "free" => f.member == "free",
                "paid" => matches!(f.member, "paid" | "comped"),
                _ => true,
            },
            // ISO strings compare in order; a bare date means its start
            "from" => f.now.as_str() >= v,
            "until" => {
                if v.len() == 10 {
                    &f.now[..10.min(f.now.len())] <= v
                } else {
                    f.now.as_str() <= v
                }
            }
            "days" => v.contains(char::from_digit(f.weekday, 10).unwrap_or('0')),
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Keep or drop every conditional block for these facts.
pub fn apply_conditions(html: &str, f: &Facts) -> String {
    if !html.contains("<!--vb-if:") {
        return html.to_string();
    }
    let mut out = html.to_string();
    let mut from = 0;
    while let Some(rel) = out[from..].find("<!--vb-if:") {
        let start = from + rel;
        let Some(head_end) = out[start..].find("-->").map(|e| start + e + 3) else { break };
        let head = &out[start + 10..head_end - 3];
        let Some((id, cond)) = head.split_once(':') else {
            from = head_end;
            continue;
        };
        let close = close_marker(id);
        let Some(end) = out[head_end..].find(&close).map(|e| head_end + e) else {
            from = head_end;
            continue;
        };
        if holds(cond, f) {
            // keep the content, drop the two markers; inner blocks are
            // handled as the scan continues from here
            let inner = out[head_end..end].to_string();
            out.replace_range(start..end + close.len(), &inner);
            from = start;
        } else {
            out.replace_range(start..end + close.len(), "");
            from = start;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// dynamic widgets
// ---------------------------------------------------------------------------

/// The marker a dynamic widget leaves in stored HTML.
pub fn dyn_marker(kind: &str, id: &str, settings: &Value) -> String {
    format!(
        r#"<div class="vb-dyn" data-vb-dyn="{}" data-vb-dyn-id="{}" data-vb-q="{}"></div>"#,
        esc(kind),
        esc(id),
        esc(&settings.to_string())
    )
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Replace each dynamic-widget marker with what `draw(kind, id, settings)`
/// returns.
pub fn replace_dyn(html: &str, mut draw: impl FnMut(&str, &str, &Value) -> String) -> String {
    const OPEN: &str = r#"<div class="vb-dyn" data-vb-dyn=""#;
    if !html.contains(OPEN) {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find("></div>") else {
            out.push_str(tail);
            return out;
        };
        let tag = &tail[..end];
        let attr = |name: &str| -> String {
            let key = format!(r#"{name}=""#);
            tag.find(&key).and_then(|p| tag[p + key.len()..].find('"').map(|q| tag[p + key.len()..p + key.len() + q].to_string())).unwrap_or_default()
        };
        let kind = attr("data-vb-dyn");
        let id = attr("data-vb-dyn-id");
        let q: Value = serde_json::from_str(&unescape(&attr("data-vb-q"))).unwrap_or(Value::Null);
        out.push_str(&draw(&kind, &id, &q));
        rest = &tail[end + "></div>".len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tags_are_marked_and_filled_escaped() {
        let m = mark_tags("Hi [[post.title]] and [[nope]] [[year]]");
        assert_eq!(m, "Hi %%vb:post.title%% and [[nope]] %%vb:year%%");
        let out = fill_tags(&m, |k| (k == "post.title").then(|| "<b>T</b>".to_string()));
        assert_eq!(out, "Hi &lt;b&gt;T&lt;/b&gt; and [[nope]] ");
    }

    #[test]
    fn conditions_keep_and_drop() {
        let html = format!("a{}secret{}b{}{}x{}{}c", open_marker("w1", "who=paid"), close_marker("w1"), open_marker("w2", "who=visitors"), open_marker("w3", "days=1"), close_marker("w3"), close_marker("w2"));
        let visitor_monday = Facts { member: "", now: "2026-11-23T10:00".into(), weekday: 1 };
        assert_eq!(apply_conditions(&html, &visitor_monday), "abxc");
        let paid_tuesday = Facts { member: "paid", now: "2026-11-24T10:00".into(), weekday: 2 };
        assert_eq!(apply_conditions(&html, &paid_tuesday), "asecretbc");
        assert!(holds("from=2026-01-01;until=2026-12-31", &paid_tuesday));
        assert!(!holds("until=2026-11-23", &paid_tuesday));
        assert!(holds("until=2026-11-24", &paid_tuesday));
    }

    #[test]
    fn dynamic_markers_round_trip() {
        let html = format!("x{}y", dyn_marker("posts", "w9", &json!({"count": 3, "tag": "a\"b"})));
        let out = replace_dyn(&html, |k, id, q| format!("[{k}:{id}:{}:{}]", q["count"], q["tag"].as_str().unwrap()));
        assert_eq!(out, "x[posts:w9:3:a\"b]y");
    }

    #[test]
    fn cond_of_reads_settings() {
        assert_eq!(cond_of(&json!({})), None);
        assert_eq!(cond_of(&json!({"if_who": "any"})), None);
        assert_eq!(cond_of(&json!({"if_who": "paid", "if_days": "135x"})).as_deref(), Some("who=paid;days=135"));
    }
}
