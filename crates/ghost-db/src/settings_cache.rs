//! In-memory settings cache, refreshed from the settings table and kept
//! fresh via explicit invalidation (the Rust port of shared/settings-cache).
use ghost_core::{GhostError, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Default)]
pub struct SettingsCache {
    map: RwLock<HashMap<String, Value>>,
}

impl SettingsCache {
    pub async fn load(pool: &SqlitePool) -> Result<Self> {
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT key, value, type, flags FROM settings",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
        let mut map = HashMap::new();
        for (key, value, typ, _flags) in rows {
            let typed = match typ.as_str() {
                "boolean" => Value::Bool(matches!(value.as_str(), "true" | "1")),
                "number" => value
                    .parse::<i64>()
                    .map(Value::from)
                    .or_else(|_| value.parse::<f64>().map(Value::from))
                    .unwrap_or(Value::String(value)),
                "array" | "object" => serde_json::from_str(&value).unwrap_or(Value::String(value)),
                _ => Value::String(value),
            };
            map.insert(key, typed);
        }
        Ok(Self { map: RwLock::new(map) })
    }

    /// A cache over values that are already in hand, with no database behind
    /// it. For tests of the code that *reads* settings — the render context in
    /// particular, whose derived keys are worth asserting without standing up
    /// a pool and a full seed to get at three of them.
    pub fn from_values(values: impl IntoIterator<Item = (String, Value)>) -> Self {
        Self { map: RwLock::new(values.into_iter().collect()) }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.map.read().unwrap().get(key).cloned()
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Value::String(s)) => Some(s),
            Some(Value::Bool(b)) => Some(if b { "true".into() } else { "false".into() }),
            Some(v) => Some(v.to_string()),
            None => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> bool {
        match self.get(key) {
            Some(Value::Bool(true)) => true,
            Some(Value::String(s)) => s == "true" || s == "1",
            _ => false,
        }
    }

    /// The domains Settings → Membership → Spam filters refuses signups from.
    ///
    /// Stored the way Ghost stores it: a JSON array, which older rows may hold
    /// as a *string* containing JSON rather than a real array — both shapes
    /// read the same here.
    pub fn blocked_email_domains(&self) -> Vec<String> {
        let raw = match self.get("blocked_email_domains") {
            Some(Value::Array(a)) => {
                return a.iter().filter_map(Value::as_str).map(normalize_domain).collect()
            }
            Some(Value::String(s)) => s,
            _ => return Vec::new(),
        };
        serde_json::from_str::<Vec<String>>(&raw)
            .map(|v| v.iter().map(|d| normalize_domain(d)).collect())
            .unwrap_or_default()
    }

    /// Whether `email` signs up from a blocked domain.
    pub fn email_domain_is_blocked(&self, email: &str) -> bool {
        let Some(domain) = email.rsplit('@').next().map(normalize_domain) else {
            return false;
        };
        if domain.is_empty() {
            return false;
        }
        self.blocked_email_domains().contains(&domain)
    }

    /// Update one key in cache and DB.
    ///
    /// Values pass through `url::unbase_assets` first, so a URL copied out of
    /// an upload response — which is browser-ready and therefore carries the
    /// install's base path — is stored in site terms like every other internal
    /// URL. Covers every writer at once: the settings screens, the navigation
    /// editor and the Admin API. A no-op at a domain root.
    pub async fn set(&self, pool: &SqlitePool, key: &str, value: &Value) -> Result<()> {
        let normalized = normalize_stored(value);
        let value = &normalized;
        let (stored, typ) = match value {
            Value::String(s) => (s.clone(), "string"),
            Value::Bool(b) => (if *b { "true".to_string() } else { "false".to_string() }, "boolean"),
            Value::Number(n) => (n.to_string(), "number"),
            Value::Array(_) | Value::Object(_) => (value.to_string(), "object"),
            Value::Null => (String::new(), "string"),
        };
        let now = ghost_core::dates::now_db();
        let res = sqlx::query("UPDATE settings SET value = ?, type = ?, updated_at = ? WHERE key = ?")
            .bind(&stored)
            .bind(typ)
            .bind(&now)
            .bind(key)
            .execute(pool)
            .await
            .map_err(|e| GhostError::Db(e.to_string()))?;
        if res.rows_affected() == 0 {
            return Err(GhostError::BadRequest(format!("unknown setting: {key}")));
        }
        self.map.write().unwrap().insert(key.to_string(), value.clone());
        Ok(())
    }
}

/// Lower-cased, trimmed, and reduced to the domain if a whole address was
/// given — `Hello@Spam.XYZ` and `spam.xyz` are the same block.
fn normalize_domain(raw: &str) -> String {
    raw.trim().to_lowercase().rsplit('@').next().unwrap_or("").to_string()
}

/// Turn the Spam filters textarea into the stored JSON array.
///
/// Split on whitespace or commas the way Ghost's own field does, keep only the
/// domain part of anything that looks like an address, and drop entries with
/// no dot — a bare word cannot be a domain and would silently block nothing.
pub fn parse_blocked_email_domains(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in input.split([' ', ',', '\n', '\r', '\t']) {
        let domain = normalize_domain(part);
        if domain.contains('.') && !out.contains(&domain) {
            out.push(domain);
        }
    }
    out
}

/// Walk a setting value and strip the base path out of any asset URL it holds.
///
/// Settings are a mix of plain strings (`logo`), arrays of objects
/// (`navigation`) and nested objects (`custom_theme_settings`), so this
/// recurses rather than special-casing a list of keys — a theme setting of
/// type `image` is as much an asset URL as `logo` is.
fn normalize_stored(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(ghost_core::url::unbase_assets(s)),
        Value::Array(items) => Value::Array(items.iter().map(normalize_stored).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), normalize_stored(v))).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with(key: &str, value: Value) -> SettingsCache {
        let cache = SettingsCache::default();
        cache.map.write().unwrap().insert(key.to_string(), value);
        cache
    }

    #[test]
    fn domains_are_normalised_and_deduped() {
        let parsed = parse_blocked_email_domains("Spam.XYZ\nhello@spam.xyz , other.test\n  \nspam.xyz");
        assert_eq!(parsed, ["spam.xyz", "other.test"]);
    }

    /// A bare word is not a domain; blocking it would match nothing and read
    /// as though it had worked.
    #[test]
    fn entries_without_a_dot_are_dropped() {
        assert!(parse_blocked_email_domains("spam\nlocalhost\n").is_empty());
    }

    /// Ghost stores the list as JSON; rows written as a JSON *string* and rows
    /// written as a real array must read the same.
    #[test]
    fn both_stored_shapes_read_alike() {
        let as_string = cache_with("blocked_email_domains", Value::String(r#"["Spam.xyz"]"#.into()));
        let as_array = cache_with("blocked_email_domains", serde_json::json!(["spam.xyz"]));
        assert_eq!(as_string.blocked_email_domains(), ["spam.xyz"]);
        assert_eq!(as_array.blocked_email_domains(), ["spam.xyz"]);
    }

    #[test]
    fn blocking_matches_on_the_domain_only() {
        let cache = cache_with("blocked_email_domains", serde_json::json!(["spam.xyz"]));
        assert!(cache.email_domain_is_blocked("hello@spam.xyz"));
        assert!(cache.email_domain_is_blocked("HELLO@Spam.XYZ"));
        assert!(!cache.email_domain_is_blocked("hello@notspam.xyz"));
        // a lookalike suffix is a different domain, not a match
        assert!(!cache.email_domain_is_blocked("hello@myspam.xyz"));
    }

    #[test]
    fn an_empty_list_blocks_nobody() {
        let cache = SettingsCache::default();
        assert!(!cache.email_domain_is_blocked("hello@spam.xyz"));
    }
}
