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

    /// Update one key in cache and DB.
    pub async fn set(&self, pool: &SqlitePool, key: &str, value: &Value) -> Result<()> {
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
