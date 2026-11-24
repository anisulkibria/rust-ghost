//! Reshape a content export so stock Ghost's importer accepts it.
//!
//! The regular export ([`crate::transfer::export`]) is Ghost-*shaped* but
//! carries this port's own schema: extra columns, extra settings groups, and
//! nullable columns Ghost declares `NOT NULL`. Vuuut's importer takes all of
//! that back; Ghost's does not, and it fails the whole import with
//! "One or more errors occurred". Reproduced against Ghost 6.76, the failures
//! were:
//!
//! - `posts.email_recipient_filter` exported as NULL (Ghost: NOT NULL, no default);
//! - settings whose `group` differs from Ghost's for the same key
//!   (`machine_payments_*` → Ghost keeps them under `site`), failing
//!   `Validation (isIn) failed for group`;
//! - settings whose `type` differs (`navigation` is an `array` in Ghost).
//!
//! The fix is driven by `ghost_schema.json`, generated from Ghost's own
//! `schema.js` and `default-settings.json`, so it states Ghost's rules rather
//! than guessing at them: every row keeps only the columns Ghost has, NULLs in
//! NOT NULL columns take Ghost's default, values outside an `isIn` list fall
//! back to the default, booleans become booleans, overlong strings are cut to
//! Ghost's maxlength, and settings keep only keys Ghost knows, with Ghost's
//! group and type. Tables Ghost does not have are dropped.
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// Ghost's rules for the tables the content export carries.
fn spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(|| serde_json::from_str(include_str!("ghost_schema.json")).expect("ghost_schema.json is valid JSON"))
}

/// The Ghost version the embedded rules were generated from.
pub fn schema_version() -> &'static str {
    spec()["ghost_version"].as_str().unwrap_or("")
}

/// A value for a NOT NULL column that has no Ghost default.
fn fallback(table: &str, column: &str, kind: &str) -> Value {
    match (table, column) {
        // Ghost's own value for a post that has not been sent to anyone.
        ("posts", "email_recipient_filter") => json!("all"),
        _ => match kind {
            "boolean" => json!(false),
            "integer" | "bigInteger" => json!(0),
            "dateTime" => json!(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            _ => json!(""),
        },
    }
}

/// Cut `s` to at most `max` UTF-16 code units — the unit Ghost's
/// (JavaScript) length check counts — without splitting a character.
fn truncate_utf16(s: &str, max: usize) -> Option<String> {
    if s.encode_utf16().count() <= max {
        return None;
    }
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        used += c.len_utf16();
        if used > max {
            break;
        }
        out.push(c);
    }
    Some(out)
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn normalize_row(table: &str, row: &Map<String, Value>, columns: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, rule) in columns {
        let kind = rule["t"].as_str().unwrap_or("string");
        let not_null = rule["nn"].as_bool().unwrap_or(false);
        let default = rule.get("def");
        let mut value = row.get(name).cloned().unwrap_or(Value::Null);

        if kind == "boolean" && !value.is_null() {
            if let Some(b) = as_bool(&value) {
                value = json!(b);
            }
        }
        if let (Some(allowed), false) = (rule["in"].as_array(), value.is_null()) {
            let text = match &value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if !allowed.iter().any(|a| a.as_str() == Some(text.as_str())) {
                if let Some(d) = default {
                    value = d.clone();
                }
            }
        }
        if value.is_null() {
            if !row.contains_key(name) && !not_null {
                // Absent and optional: leave it to Ghost's default.
                continue;
            }
            if not_null {
                value = default.cloned().unwrap_or_else(|| fallback(table, name, kind));
            }
        }
        if let (Some(max), Value::String(s)) = (rule["max"].as_u64(), &value) {
            if let Some(cut) = truncate_utf16(s, max as usize) {
                value = json!(cut);
            }
        }
        out.insert(name.clone(), value);
    }
    out
}

/// Settings Ghost knows, under Ghost's group and type. Unknown keys (this
/// port's own features) are dropped — Ghost would skip them anyway — and the
/// `core` group never leaves, as in Ghost's own export.
fn normalize_settings(rows: &[Value]) -> Vec<Value> {
    let known = spec()["settings"].as_object();
    rows.iter()
        .filter_map(|row| {
            let mut row = row.as_object()?.clone();
            let key = row.get("key")?.as_str()?.to_string();
            let rule = known?.get(&key)?.as_array()?;
            let group = rule.first()?.as_str()?;
            let kind = rule.get(1)?.as_str()?;
            row.insert("group".into(), json!(group));
            row.insert("type".into(), json!(kind));
            Some(Value::Object(row))
        })
        .collect()
}

/// Rewrite a `{"db":[{"meta":…,"data":…}]}` export for Ghost's importer.
pub fn for_ghost(export: &Value) -> Value {
    let tables = spec()["tables"].as_object();
    let mut db = export["db"][0].clone();
    let data = db["data"].as_object().cloned().unwrap_or_default();
    let mut out = Map::new();
    for (table, rows) in data {
        let Some(columns) = tables.and_then(|t| t.get(&table)).and_then(Value::as_object) else {
            continue;
        };
        let rows = rows.as_array().cloned().unwrap_or_default();
        let rows = if table == "settings" { normalize_settings(&rows) } else { rows };
        let rows: Vec<Value> = rows
            .iter()
            .filter_map(Value::as_object)
            .map(|row| Value::Object(normalize_row(&table, row, columns)))
            .collect();
        out.insert(table, json!(rows));
    }
    db["data"] = Value::Object(out);
    json!({ "db": [db] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(data: Value) -> Value {
        json!({"db":[{"meta":{"version":"6.75.0","exported_on":0},"data":data}]})
    }

    #[test]
    fn posts_get_ghosts_required_values_and_lose_foreign_columns() {
        let out = for_ghost(&export(json!({"posts":[{
            "id":"a","uuid":"u","title":"T","slug":"t","type":"post","status":"published",
            "visibility":"public","created_at":"2026-01-01 00:00:00",
            "email_recipient_filter":null,"featured":1,"markdown":"x","translation_group":"g"
        }]})));
        let post = &out["db"][0]["data"]["posts"][0];
        assert_eq!(post["email_recipient_filter"], "all");
        assert_eq!(post["featured"], true);
        assert!(post.get("markdown").is_none() && post.get("translation_group").is_none());
        assert_eq!(post["show_title_and_feature_image"], true);
    }

    #[test]
    fn settings_take_ghosts_group_and_type_and_unknown_keys_go() {
        let out = for_ghost(&export(json!({"settings":[
            {"id":"1","key":"machine_payments_enabled","group":"machine_payments","type":"boolean","value":"false","created_at":"x"},
            {"id":"2","key":"navigation","group":"site","type":"object","value":"[]","created_at":"x"},
            {"id":"3","key":"cookie_banner","group":"cookies","type":"boolean","value":"true","created_at":"x"},
            {"id":"4","key":"db_hash","group":"core","type":"string","value":"h","created_at":"x"}
        ]})));
        let s = out["db"][0]["data"]["settings"].as_array().unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["group"], "site");
        assert_eq!(s[1]["type"], "array");
    }

    #[test]
    fn tables_ghost_lacks_are_dropped_and_bad_enums_fall_back() {
        let out = for_ghost(&export(json!({
            "offer_redemptions":[{"id":"r"}],
            "tags":[{"id":"t","name":"N","slug":"n","visibility":"secret","created_at":"x"}]
        })));
        assert!(out["db"][0]["data"].get("offer_redemptions").is_none());
        assert_eq!(out["db"][0]["data"]["tags"][0]["visibility"], "public");
    }

    #[test]
    fn truncation_counts_utf16_and_keeps_characters_whole() {
        assert_eq!(truncate_utf16("ab😀c", 3).as_deref(), Some("ab"));
        assert_eq!(truncate_utf16("abc", 3), None);
    }
}
