//! Forms: what a form is made of, and what a response to one must look like.
//!
//! This module is the trust boundary for the forms feature and is pure on
//! purpose — no database, no HTTP — so every rule below can be tested
//! directly.
//!
//! Two kinds of untrusted input arrive here:
//!
//! * a **form definition**, posted by staff from the builder. Staff are
//!   trusted to design a form, not to store arbitrary JSON, so
//!   [`normalize_fields`] rebuilds the definition from scratch: unknown
//!   field types are dropped, every string is bounded and stripped of
//!   control characters, and field keys are regenerated unless they match
//!   the shape this module issues.
//! * a **response**, posted by anyone on the internet. [`validate`] reads
//!   only the fields the form defines — anything else in the request is
//!   never looked at, let alone stored — and checks each value against its
//!   field's type. A choice field accepts only the options the form offers.
//!
//! Nothing here produces HTML. Values are stored as plain text and escaped
//! where they are displayed.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Upper bounds. Generous for a contact form, small enough that a response
/// can never be used to park megabytes in the database.
pub const MAX_FIELDS: usize = 30;
pub const MAX_OPTIONS: usize = 50;
pub const MAX_NAME: usize = 120;
pub const MAX_LABEL: usize = 120;
pub const MAX_HELP: usize = 300;
pub const MAX_OPTION: usize = 120;
pub const MAX_SUBMIT_LABEL: usize = 60;
pub const MAX_SUCCESS_MESSAGE: usize = 400;
/// A single-line answer.
pub const MAX_SHORT: usize = 500;
/// A multi-line answer.
pub const MAX_LONG: usize = 5000;
pub const MAX_EMAIL: usize = 254;
pub const MAX_URL: usize = 2000;
/// Addresses one form (or the site) may notify.
pub const MAX_RECIPIENTS: usize = 10;

pub const MAX_LINK_LABEL: usize = 80;
pub const DEFAULT_LINK_LABEL: &str = "Open link";

pub const DEFAULT_SUBMIT_LABEL: &str = "Send";
pub const DEFAULT_SUCCESS_MESSAGE: &str = "Thank you. Your message has been sent.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Email,
    Phone,
    Url,
    Number,
    Date,
    Textarea,
    Select,
    Radio,
    Checkboxes,
    /// A single must-tick box: "I agree to be contacted".
    Consent,
}

impl FieldKind {
    pub const ALL: &'static [FieldKind] = &[
        FieldKind::Text,
        FieldKind::Email,
        FieldKind::Phone,
        FieldKind::Url,
        FieldKind::Number,
        FieldKind::Date,
        FieldKind::Textarea,
        FieldKind::Select,
        FieldKind::Radio,
        FieldKind::Checkboxes,
        FieldKind::Consent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Email => "email",
            FieldKind::Phone => "phone",
            FieldKind::Url => "url",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Textarea => "textarea",
            FieldKind::Select => "select",
            FieldKind::Radio => "radio",
            FieldKind::Checkboxes => "checkboxes",
            FieldKind::Consent => "consent",
        }
    }

    pub fn parse(s: &str) -> Option<FieldKind> {
        FieldKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// What the builder calls it.
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Text => "Short text",
            FieldKind::Email => "Email",
            FieldKind::Phone => "Phone",
            FieldKind::Url => "Web address",
            FieldKind::Number => "Number",
            FieldKind::Date => "Date",
            FieldKind::Textarea => "Long text",
            FieldKind::Select => "Dropdown",
            FieldKind::Radio => "Single choice",
            FieldKind::Checkboxes => "Multiple choice",
            FieldKind::Consent => "Consent checkbox",
        }
    }

    /// Whether the field offers a fixed list of answers.
    pub fn has_options(self) -> bool {
        matches!(self, FieldKind::Select | FieldKind::Radio | FieldKind::Checkboxes)
    }
}

/// One question on a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// The input's `name`. Issued by [`new_key`], never chosen by a person,
    /// so it is safe to use in markup and cannot collide with the form's own
    /// control fields.
    pub key: String,
    pub kind: FieldKind,
    pub label: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// One answer, as stored: the question travels with it, so editing or
/// deleting the field later does not change what an old response says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub value: String,
}

/// A fresh field key: `f_` and eight hex characters.
pub fn new_key() -> String {
    format!("f_{}", crate::ids::random_hex(4))
}

/// Whether `key` has the shape [`new_key`] issues.
pub fn is_field_key(key: &str) -> bool {
    key.len() == 10
        && key.starts_with("f_")
        && key[2..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The fields a brand-new form starts with: the contact form most sites want.
pub fn default_fields() -> Vec<Field> {
    let field = |kind, label: &str, required| Field {
        key: new_key(),
        kind,
        label: label.into(),
        required,
        placeholder: String::new(),
        help: String::new(),
        options: Vec::new(),
    };
    vec![
        field(FieldKind::Text, "Name", true),
        field(FieldKind::Email, "Email", true),
        field(FieldKind::Textarea, "Message", true),
    ]
}

/// Remove control characters and trim. With `multiline`, line breaks and
/// tabs survive (and `\r\n` becomes `\n`); otherwise every run of
/// whitespace collapses to one space, so a "single line" really is one.
///
/// Stripping control characters is what keeps a value from smuggling a
/// header break into an email subject or an escape sequence into a log.
pub fn clean(raw: &str, multiline: bool) -> String {
    if multiline {
        let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
        let kept: String = unified.chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control()).collect();
        kept.trim().to_string()
    } else {
        let kept: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        kept.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// `clean`, then cut to `max` characters (not bytes).
pub fn clean_bounded(raw: &str, multiline: bool, max: usize) -> String {
    let cleaned = clean(raw, multiline);
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        cleaned.chars().take(max).collect::<String>().trim_end().to_string()
    }
}

/// A deliberately conservative address check: one `@`, a dotted domain, and
/// only characters that cannot break out of a header or an HTML attribute.
/// It rejects some exotic-but-legal addresses, which is the right trade for
/// a public form.
pub fn is_email(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_EMAIL {
        return false;
    }
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    if local.is_empty() || local.len() > 64 || local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    if !local.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~.-".contains(&b)) {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Parse a comma-, semicolon- or whitespace-separated list of addresses.
/// Returns the valid ones (deduplicated, lower-cased, capped) and the
/// entries that were refused, so a settings page can say which.
pub fn parse_recipients(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut good: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for part in raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_lowercase();
        if is_email(&lower) {
            if !good.contains(&lower) && good.len() < MAX_RECIPIENTS {
                good.push(lower);
            }
        } else {
            bad.push(clean_bounded(part, false, 80));
        }
    }
    (good, bad)
}

fn is_http_url(s: &str) -> bool {
    if s.len() > MAX_URL || s.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`')) {
        return false;
    }
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    match rest {
        Some(rest) => {
            let host = rest.split(['/', '?', '#']).next().unwrap_or("");
            !host.is_empty() && !host.starts_with('.') && !host.contains('@')
        }
        None => false,
    }
}

fn is_phone(s: &str) -> bool {
    let digits = s.bytes().filter(u8::is_ascii_digit).count();
    (3..=20).contains(&digits)
        && s.len() <= 32
        && s.bytes().all(|b| b.is_ascii_digit() || b" +-().".contains(&b))
}

fn is_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn is_number(s: &str) -> bool {
    s.len() <= 32
        && s.bytes().all(|b| b.is_ascii_digit() || b"+-.".contains(&b))
        && s.parse::<f64>().map(f64::is_finite).unwrap_or(false)
}

/// What a form does once a response has been accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessAction {
    /// Show the success message.
    Message,
    /// Show the message with a link to a URL or a file.
    Link,
    /// Send the visitor on to a URL.
    Redirect,
}

impl SuccessAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SuccessAction::Message => "message",
            SuccessAction::Link => "link",
            SuccessAction::Redirect => "redirect",
        }
    }

    /// Anything unrecognised is the plain message — the action that sends
    /// nobody anywhere.
    pub fn parse(s: &str) -> SuccessAction {
        match s {
            "link" => SuccessAction::Link,
            "redirect" => SuccessAction::Redirect,
            _ => SuccessAction::Message,
        }
    }
}

/// Check the address a form links or redirects to after a response.
///
/// Accepted: a full `http://` or `https://` URL, or a path on this site
/// (`/thanks/`, `/content/files/guide.pdf`). Refused: every other scheme —
/// `javascript:` and `data:` would run in the visitor's page — and
/// `//host`, which looks like a path and is another site. The value is set
/// by staff, never taken from a request, so a response cannot choose where a
/// visitor is sent.
pub fn normalize_success_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let refuse = || Err("The link must be a full address starting with https://, or a path on this site starting with /.".to_string());
    if url.chars().count() > MAX_URL
        || url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '\\'))
    {
        return refuse();
    }
    if url.starts_with('/') {
        return if url.starts_with("//") { refuse() } else { Ok(url.to_string()) };
    }
    if is_http_url(url) {
        return Ok(url.to_string());
    }
    refuse()
}

/// Rebuild a form definition from what the builder posted.
///
/// `raw` is the builder's JSON array. The result is safe to store and to
/// render: bounded, free of control characters, with unique issued keys.
/// An `Err` is a sentence to show the person editing the form.
pub fn normalize_fields(raw: &str) -> Result<Vec<Field>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "The form's fields could not be read. Reload the page and try again.".to_string())?;
    let Some(items) = parsed.as_array() else {
        return Err("The form's fields could not be read. Reload the page and try again.".into());
    };
    if items.is_empty() {
        return Err("A form needs at least one field.".into());
    }
    if items.len() > MAX_FIELDS {
        return Err(format!("A form can have at most {MAX_FIELDS} fields."));
    }

    let text = |item: &serde_json::Value, key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut seen: HashSet<String> = HashSet::new();
    let mut fields = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let n = index + 1;
        let Some(kind) = FieldKind::parse(&text(item, "kind")) else {
            return Err(format!("Field {n} has a type this form builder does not know."));
        };
        let label = clean_bounded(&text(item, "label"), false, MAX_LABEL);
        if label.is_empty() {
            return Err(format!("Field {n} needs a label."));
        }
        // A key is kept only if it is one of ours and not already used on
        // this form; anything else gets a new one. Keeping a field's key
        // across edits is what lets a response line up with its question.
        let posted = text(item, "key");
        let key = if is_field_key(&posted) && !seen.contains(&posted) { posted } else { new_key() };
        seen.insert(key.clone());

        let mut options: Vec<String> = Vec::new();
        if kind.has_options() {
            let raw_options: Vec<String> = match item.get("options") {
                Some(serde_json::Value::Array(list)) => {
                    list.iter().filter_map(|v| v.as_str()).map(str::to_string).collect()
                }
                // The builder edits options as one-per-line text.
                Some(serde_json::Value::String(s)) => s.lines().map(str::to_string).collect(),
                _ => Vec::new(),
            };
            for option in raw_options {
                let option = clean_bounded(&option, false, MAX_OPTION);
                if !option.is_empty() && !options.contains(&option) {
                    options.push(option);
                }
            }
            if options.is_empty() {
                return Err(format!("“{label}” needs at least one option."));
            }
            if options.len() > MAX_OPTIONS {
                return Err(format!("“{label}” can have at most {MAX_OPTIONS} options."));
            }
        }

        fields.push(Field {
            key,
            kind,
            label,
            required: item.get("required").and_then(|v| v.as_bool()).unwrap_or(false),
            // A placeholder only means something on a typed field.
            placeholder: if kind.has_options() || matches!(kind, FieldKind::Consent | FieldKind::Date) {
                String::new()
            } else {
                clean_bounded(&text(item, "placeholder"), false, MAX_LABEL)
            },
            help: clean_bounded(&text(item, "help"), false, MAX_HELP),
            options,
        });
    }
    Ok(fields)
}

/// Read stored field definitions. A row that will not parse yields no
/// fields, which renders as no form — never as an error page.
pub fn parse_fields(stored: &str) -> Vec<Field> {
    serde_json::from_str(stored).unwrap_or_default()
}

/// Read stored answers.
pub fn parse_answers(stored: &str) -> Vec<Answer> {
    serde_json::from_str(stored).unwrap_or_default()
}

/// Check a response against a form.
///
/// `input` is the request's fields by name; a name may repeat (checkboxes).
/// Only the form's own keys are consulted. `Ok` carries the answers to
/// store, in the form's order, omitting optional fields left blank. `Err`
/// carries one message per offending field key.
pub fn validate(fields: &[Field], input: &HashMap<String, Vec<String>>) -> Result<Vec<Answer>, Vec<(String, String)>> {
    let mut answers = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut fail = |field: &Field, message: &str| errors.push((field.key.clone(), message.to_string()));

    for field in fields {
        let raw: &[String] = input.get(&field.key).map(Vec::as_slice).unwrap_or(&[]);
        let first = raw.first().map(String::as_str).unwrap_or("");

        let value: String = match field.kind {
            FieldKind::Checkboxes => {
                // Each ticked box must be one the form offers; order follows
                // the form, not the request.
                let picked: HashSet<String> = raw.iter().map(|v| clean(v, false)).filter(|v| !v.is_empty()).collect();
                if picked.iter().any(|p| !field.options.contains(p)) {
                    fail(field, "Choose from the options offered.");
                    continue;
                }
                field.options.iter().filter(|o| picked.contains(*o)).cloned().collect::<Vec<_>>().join(", ")
            }
            FieldKind::Consent => {
                if matches!(first.trim(), "1" | "on" | "yes" | "true") { "Yes".into() } else { String::new() }
            }
            FieldKind::Textarea => {
                let value = clean(first, true);
                if value.chars().count() > MAX_LONG {
                    fail(field, &format!("Keep this under {MAX_LONG} characters."));
                    continue;
                }
                value
            }
            _ => {
                let value = clean(first, false);
                let max = match field.kind {
                    FieldKind::Email => MAX_EMAIL,
                    FieldKind::Url => MAX_URL,
                    _ => MAX_SHORT,
                };
                if value.chars().count() > max {
                    fail(field, &format!("Keep this under {max} characters."));
                    continue;
                }
                value
            }
        };

        if value.is_empty() {
            if field.required {
                fail(
                    field,
                    match field.kind {
                        FieldKind::Consent => "Tick this box to continue.",
                        FieldKind::Select | FieldKind::Radio | FieldKind::Checkboxes => "Choose an option.",
                        _ => "This field is required.",
                    },
                );
            }
            continue;
        }

        let problem = match field.kind {
            FieldKind::Email if !is_email(&value) => Some("Enter a valid email address."),
            FieldKind::Phone if !is_phone(&value) => Some("Enter a valid phone number."),
            FieldKind::Url if !is_http_url(&value) => Some("Enter a full web address, starting with https://."),
            FieldKind::Number if !is_number(&value) => Some("Enter a number."),
            FieldKind::Date if !is_date(&value) => Some("Enter a date."),
            FieldKind::Select | FieldKind::Radio if !field.options.contains(&value) => Some("Choose from the options offered."),
            _ => None,
        };
        if let Some(message) = problem {
            fail(field, message);
            continue;
        }

        answers.push(Answer {
            key: field.key.clone(),
            label: field.label.clone(),
            kind: field.kind.as_str().to_string(),
            value,
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    if answers.is_empty() {
        // Every field optional and all of them blank: there is nothing to
        // file, and an empty response is only ever noise.
        let key = fields.first().map(|f| f.key.clone()).unwrap_or_default();
        return Err(vec![(key, "Fill in at least one field.".into())]);
    }
    Ok(answers)
}

/// The first email address in a response, for the admin's "reply" link.
pub fn reply_address(answers: &[Answer]) -> Option<&str> {
    answers.iter().find(|a| a.kind == "email" && is_email(&a.value)).map(|a| a.value.as_str())
}

/// A one-line summary of a response for a list row.
pub fn summary(answers: &[Answer], max: usize) -> String {
    let joined = answers.iter().map(|a| clean(&a.value, false)).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" · ");
    if joined.chars().count() <= max {
        joined
    } else {
        format!("{}…", joined.chars().take(max).collect::<String>())
    }
}

/// Make a value safe to open in a spreadsheet.
///
/// A cell starting with `=`, `+`, `-` or `@` (or a tab or carriage return
/// before one) is run as a formula by Excel and friends, so an exported
/// response could execute on the machine of whoever opens the file. The
/// standard defence is a leading apostrophe, which spreadsheets show as text.
pub fn csv_safe(value: &str) -> String {
    match value.trim_start_matches([' ', '\t', '\r', '\n']).chars().next() {
        Some('=' | '+' | '-' | '@') => format!("'{value}"),
        _ if value.starts_with(['\t', '\r']) => format!("'{value}"),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(kind: FieldKind, required: bool, options: &[&str]) -> Field {
        Field {
            key: new_key(),
            kind,
            label: "Q".into(),
            required,
            placeholder: String::new(),
            help: String::new(),
            options: options.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn input(pairs: &[(&str, &str)]) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in pairs {
            map.entry(k.to_string()).or_default().push(v.to_string());
        }
        map
    }

    #[test]
    fn issued_keys_have_the_shape_that_is_checked_for() {
        for _ in 0..50 {
            assert!(is_field_key(&new_key()));
        }
        for bad in ["", "f_", "f_1234567", "f_123456789", "g_12345678", "f_1234567G", "_token", "f_12 45678", "f_1234567\""] {
            assert!(!is_field_key(bad), "{bad:?} was accepted");
        }
    }

    #[test]
    fn single_line_cleaning_removes_everything_that_could_break_a_header() {
        assert_eq!(clean("  a\r\nBcc: x@y.z\t b\u{0}\u{7f} ", false), "a Bcc: x@y.z b");
        assert_eq!(clean("a\r\nb\rc\u{0}", true), "a\nb\nc");
        assert_eq!(clean_bounded("éééé", false, 2), "éé");
    }

    #[test]
    fn email_checking_is_strict() {
        for good in ["a@b.co", "first.last+tag@sub.example.com", "o'neil@example.org"] {
            assert!(is_email(good), "{good}");
        }
        for bad in [
            "", "a", "a@b", "@b.co", "a@.co", "a b@c.de", "a@b..co", "a@-b.co", "a@b.co\r\nBcc: x@y.z",
            "a<script>@b.co", "\"a\"@b.co", "a@b.co,c@d.eu", ".a@b.co", "a..b@c.de",
        ] {
            assert!(!is_email(bad), "{bad:?} was accepted");
        }
    }

    #[test]
    fn recipients_are_split_validated_deduplicated_and_capped() {
        let (good, bad) = parse_recipients("A@b.co, a@b.co; nope  c@d.eu\nx@y");
        assert_eq!(good, vec!["a@b.co", "c@d.eu"]);
        assert_eq!(bad, vec!["nope", "x@y"]);
        let many = (0..30).map(|i| format!("u{i}@e.co")).collect::<Vec<_>>().join(",");
        assert_eq!(parse_recipients(&many).0.len(), MAX_RECIPIENTS);
    }

    #[test]
    fn a_definition_is_rebuilt_not_trusted() {
        let raw = serde_json::json!([
            {"key": "f_0123abcd", "kind": "text", "label": "  Name\n", "required": true, "evil": "<script>"},
            {"key": "f_0123abcd", "kind": "email", "label": "Email"},
            {"key": "_token", "kind": "select", "label": "Topic", "options": "Sales\n\nSales\n Support "},
        ])
        .to_string();
        let fields = normalize_fields(&raw).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].key, "f_0123abcd");
        assert_eq!(fields[0].label, "Name");
        // a duplicated key and a forged one are both replaced
        assert_ne!(fields[1].key, "f_0123abcd");
        assert!(is_field_key(&fields[1].key));
        assert!(is_field_key(&fields[2].key));
        assert_eq!(fields[2].options, vec!["Sales", "Support"]);
        // nothing unknown survives the round trip
        assert!(!serde_json::to_string(&fields).unwrap().contains("evil"));
    }

    #[test]
    fn a_definition_with_a_problem_is_refused_with_a_reason() {
        assert!(normalize_fields("not json").is_err());
        assert!(normalize_fields("{}").is_err());
        assert!(normalize_fields("[]").unwrap_err().contains("at least one"));
        assert!(normalize_fields(r#"[{"kind":"file","label":"Upload"}]"#).unwrap_err().contains("type"));
        assert!(normalize_fields(r#"[{"kind":"text","label":"  "}]"#).unwrap_err().contains("label"));
        assert!(normalize_fields(r#"[{"kind":"radio","label":"Pick","options":[]}]"#).unwrap_err().contains("option"));
        let many = serde_json::Value::Array(vec![serde_json::json!({"kind":"text","label":"x"}); MAX_FIELDS + 1]).to_string();
        assert!(normalize_fields(&many).unwrap_err().contains("at most"));
    }

    #[test]
    fn only_defined_fields_are_read() {
        let name = field(FieldKind::Text, true, &[]);
        let answers = validate(
            std::slice::from_ref(&name),
            &input(&[(&name.key, "Ada"), ("is_admin", "1"), ("status", "read"), ("f_ffffffff", "stray")]),
        )
        .unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].value, "Ada");
    }

    #[test]
    fn required_and_typed_fields_are_enforced() {
        let email = field(FieldKind::Email, true, &[]);
        let url = field(FieldKind::Url, false, &[]);
        let phone = field(FieldKind::Phone, false, &[]);
        let number = field(FieldKind::Number, false, &[]);
        let date = field(FieldKind::Date, false, &[]);
        let fields = vec![email.clone(), url.clone(), phone.clone(), number.clone(), date.clone()];

        let errors = validate(&fields, &input(&[])).unwrap_err();
        assert_eq!(errors, vec![(email.key.clone(), "This field is required.".to_string())]);

        let errors = validate(
            &fields,
            &input(&[
                (&email.key, "not-an-address"),
                (&url.key, "javascript:alert(1)"),
                (&phone.key, "call me"),
                (&number.key, "1e999"),
                (&date.key, "2026-02-30"),
            ]),
        )
        .unwrap_err();
        assert_eq!(errors.len(), 5);

        let answers = validate(
            &fields,
            &input(&[
                (&email.key, "ada@example.com"),
                (&url.key, "https://example.com/a?b=c"),
                (&phone.key, "+44 (0)20 7946 0000"),
                (&number.key, "-12.5"),
                (&date.key, "2026-11-12"),
            ]),
        )
        .unwrap();
        assert_eq!(answers.len(), 5);
    }

    #[test]
    fn choices_outside_the_offered_options_are_refused() {
        let select = field(FieldKind::Select, true, &["Sales", "Support"]);
        let boxes = field(FieldKind::Checkboxes, false, &["A", "B", "C"]);
        let fields = vec![select.clone(), boxes.clone()];

        assert!(validate(&fields, &input(&[(&select.key, "Admin")])).is_err());
        assert!(validate(&fields, &input(&[(&select.key, "Sales"), (&boxes.key, "A"), (&boxes.key, "Z")])).is_err());

        let answers =
            validate(&fields, &input(&[(&select.key, "Support"), (&boxes.key, "C"), (&boxes.key, "A"), (&boxes.key, "A")])).unwrap();
        assert_eq!(answers[0].value, "Support");
        // de-duplicated and in the form's order
        assert_eq!(answers[1].value, "A, C");
    }

    #[test]
    fn consent_must_be_ticked_when_required() {
        let consent = field(FieldKind::Consent, true, &[]);
        let fields = vec![consent.clone()];
        assert!(validate(&fields, &input(&[])).is_err());
        assert!(validate(&fields, &input(&[(&consent.key, "0")])).is_err());
        assert_eq!(validate(&fields, &input(&[(&consent.key, "1")])).unwrap()[0].value, "Yes");
    }

    #[test]
    fn overlong_answers_are_refused_rather_than_silently_cut() {
        let short = field(FieldKind::Text, false, &[]);
        let long = field(FieldKind::Textarea, false, &[]);
        let fields = vec![short.clone(), long.clone()];
        assert!(validate(&fields, &input(&[(&short.key, &"x".repeat(MAX_SHORT + 1))])).is_err());
        assert!(validate(&fields, &input(&[(&long.key, &"x".repeat(MAX_LONG + 1))])).is_err());
        assert!(validate(&fields, &input(&[(&long.key, &"x".repeat(MAX_LONG))])).is_ok());
    }

    #[test]
    fn a_wholly_blank_response_is_not_a_response() {
        let optional = field(FieldKind::Text, false, &[]);
        assert!(validate(std::slice::from_ref(&optional), &input(&[(&optional.key, "   ")])).is_err());
    }

    #[test]
    fn multi_line_answers_keep_their_line_breaks_and_lose_control_characters() {
        let long = field(FieldKind::Textarea, true, &[]);
        let answers = validate(std::slice::from_ref(&long), &input(&[(&long.key, "one\r\ntwo\u{1b}[31m")])).unwrap();
        assert_eq!(answers[0].value, "one\ntwo[31m");
    }

    #[test]
    fn the_link_after_sending_is_a_web_address_or_a_path_on_this_site() {
        for good in ["https://example.com/thanks", "http://example.com/a?b=c#d", "/thanks/", "/content/files/2026/11/guide.pdf", ""] {
            assert_eq!(normalize_success_url(good).as_deref(), Ok(good), "{good}");
        }
        assert_eq!(normalize_success_url("  /thanks/  ").as_deref(), Ok("/thanks/"));
        for bad in [
            "javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<script>alert(1)</script>", "vbscript:x",
            "//evil.test/x", "/\\evil.test", "evil.test", "ftp://example.com/f", "https://", "https://user@evil.test/",
            "/a b", "/a\"onclick=\"x", "https://example.com/\r\nSet-Cookie: x", "mailto:a@b.co",
        ] {
            assert!(normalize_success_url(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(SuccessAction::parse("redirect"), SuccessAction::Redirect);
        assert_eq!(SuccessAction::parse("link"), SuccessAction::Link);
        assert_eq!(SuccessAction::parse("anything else"), SuccessAction::Message);
    }

    #[test]
    fn spreadsheet_formulas_are_defused() {
        assert_eq!(csv_safe("=HYPERLINK(\"http://x\")"), "'=HYPERLINK(\"http://x\")");
        assert_eq!(csv_safe("  +1"), "'  +1");
        assert_eq!(csv_safe("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_safe("-2"), "'-2");
        assert_eq!(csv_safe("\tx"), "'\tx");
        assert_eq!(csv_safe("hello"), "hello");
        assert_eq!(csv_safe(""), "");
    }

    #[test]
    fn the_reply_address_is_the_first_valid_email_answer() {
        let answers = vec![
            Answer { key: "a".into(), label: "Name".into(), kind: "text".into(), value: "x@y.zz".into() },
            Answer { key: "b".into(), label: "Email".into(), kind: "email".into(), value: "ada@example.com".into() },
        ];
        assert_eq!(reply_address(&answers), Some("ada@example.com"));
        assert_eq!(summary(&answers, 8), "x@y.zz ·…");
    }
}
