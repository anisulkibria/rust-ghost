//! In-memory settings cache, refreshed from the settings table and kept
//! fresh via explicit invalidation (the Rust port of shared/settings-cache).
use ghost_core::{GhostError, Result};
use serde_json::Value;
use crate::Db;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Default)]
pub struct SettingsCache {
    map: RwLock<HashMap<String, Value>>,
    managed_mail: HashMap<String, Value>,
    mail_defaults: HashMap<String, Value>,
    mail_pool: Option<Db>,
    mail_host: Value,
}

impl SettingsCache {
    pub async fn load(pool: &Db) -> Result<Self> {
        let rows = sqlx::query_as::<_, (String, crate::types::Text, String, Option<String>)>(
            "SELECT `key`, value, type, flags FROM settings",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
        let mut map = HashMap::new();
        for (key, value, typ, _flags) in rows {
            // `settings.value` is long enough to be MEDIUMTEXT on MySQL, which
            // arrives as a blob; `types::Text` accepts either form.
            let value = value.0;
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
        Ok(Self { map: RwLock::new(map), managed_mail: HashMap::new(), mail_defaults: HashMap::new(), mail_pool: Some(pool.clone()), mail_host: Value::Null })
    }

    pub async fn reload(&self, pool: &Db) -> Result<()> {
        let loaded = Self::load(pool).await?;
        *self.map.write().unwrap() = loaded.map.into_inner().unwrap();
        Ok(())
    }

    /// A cache over values that are already in hand, with no database behind
    /// it. For tests of the code that *reads* settings — the render context in
    /// particular, whose derived keys are worth asserting without standing up
    /// a pool and a full seed to get at three of them.
    pub fn from_values(values: impl IntoIterator<Item = (String, Value)>) -> Self {
        Self { map: RwLock::new(values.into_iter().collect()), managed_mail: HashMap::new(), mail_defaults: HashMap::new(), mail_pool: None, mail_host: Value::Null }
    }

    /// A deployment SMTP relay is the default until the site configures its own
    /// relay. Keep credentials in memory and out of the tenant settings table.
    pub fn with_mail_config(mut self, mail: &Value) -> Self {
        let transport = mail["transport"].as_str().unwrap_or("");
        let options = &mail["options"];
        let host = options["host"].as_str().unwrap_or("").trim();
        if !transport.eq_ignore_ascii_case("smtp") || host.is_empty() {
            return self;
        }
        let port = options["port"].as_u64().or_else(|| options["port"].as_str().and_then(|v| v.parse().ok())).unwrap_or(587);
        let security = if options["ignoreTLS"].as_bool() == Some(true) { "none" }
            else if options["secure"].as_bool() == Some(true) || port == 465 { "tls" }
            else { "starttls" };
        self.mail_defaults.extend([
            ("mail_transport".into(), Value::String("SMTP".into())),
            ("mail_smtp_host".into(), Value::String(host.into())),
            ("mail_smtp_port".into(), Value::String(port.to_string())),
            ("mail_smtp_security".into(), Value::String(security.into())),
            ("mail_smtp_user".into(), Value::String(options["auth"]["user"].as_str().unwrap_or("").into())),
            ("mail_smtp_pass".into(), Value::String(options["auth"]["pass"].as_str().unwrap_or("").into())),
        ]);
        let from = mail["from"].as_str().filter(|v| !v.trim().is_empty())
            .or_else(|| options["auth"]["user"].as_str().filter(|v| v.contains('@')));
        if let Some(from) = from {
            let from = from.trim();
            let address = if let Some((name, address)) = from.strip_suffix('>').and_then(|v| v.rsplit_once('<')) {
                self.mail_defaults.insert("mail_from_name".into(), Value::String(name.trim().trim_matches('"').into()));
                address.trim()
            } else { from };
            self.mail_defaults.insert("mail_from_address".into(), Value::String(address.into()));
        }
        self
    }

    /// Apply the operator's credentials without persisting them into the tenant DB.
    /// Missing mail keys are explicitly empty to prevent stale tenant credentials
    /// from becoming fallback transports while platform management is active.
    pub fn with_managed_mail(mut self, host_settings: &Value) -> Self {
        self.mail_host = host_settings.clone();
        if let Some(values) = host_settings.get("managedMail").and_then(Value::as_object) {
            self.managed_mail = ghost_core::mail_settings::KEYS.iter()
                .map(|key| (key.to_string(), values.get(*key).cloned().unwrap_or(Value::String(String::new()))))
                .collect();
        }
        self
    }

    pub fn with_mail_pool(mut self, pool: &Db) -> Self { self.mail_pool = Some(pool.clone()); self }

    pub fn mail_meter(&self) -> Option<crate::mail_usage::Meter> {
        self.mail_pool.as_ref().map(|pool| crate::mail_usage::Meter::new(pool.clone(), self.mail_host.clone()))
    }

    pub fn is_locked(&self, key: &str) -> bool {
        self.managed_mail.contains_key(key)
    }

    pub fn mail_is_managed(&self) -> bool {
        !self.managed_mail.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        if let Some(value) = self.managed_mail.get(key) { return Some(value.clone()); }
        let values = self.map.read().unwrap();
        let own_relay = values.get("mail_transport").and_then(Value::as_str)
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("smtp") || v.trim().eq_ignore_ascii_case("ses"));
        let sender = matches!(key, "mail_from_address" | "mail_from_name" | "mail_reply_to");
        let own_value = values.get(key);
        if sender && own_value.and_then(Value::as_str).is_some_and(|v| !v.trim().is_empty()) {
            return own_value.cloned();
        }
        if !own_relay {
            if let Some(value) = self.mail_defaults.get(key) { return Some(value.clone()); }
        }
        // Saving the email form leaves its password blank. Preserve the
        // deployment credential only for the exact same SMTP host and user.
        let same_smtp = values.get("mail_transport").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case("smtp"))
            && ["mail_smtp_host", "mail_smtp_user"].iter().all(|k| {
                self.mail_defaults.get(*k).is_some_and(|v| values.get(*k) == Some(v))
            });
        if same_smtp && (sender || key == "mail_smtp_pass")
            && own_value.and_then(Value::as_str).is_none_or(|v| v.is_empty()) {
            if let Some(value) = self.mail_defaults.get(key) { return Some(value.clone()); }
        }
        own_value.cloned()
    }

    /// A request-local settings copy for previews; never changes persisted or live settings.
    pub fn with_overrides(&self, values: impl IntoIterator<Item = (String, Value)>) -> Self {
        let mut map = self.map.read().unwrap().clone();
        map.extend(values);
        Self { map: RwLock::new(map), managed_mail: self.managed_mail.clone(), mail_defaults: self.mail_defaults.clone(), mail_pool: self.mail_pool.clone(), mail_host: self.mail_host.clone() }
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Value::String(s)) => Some(s),
            Some(Value::Bool(b)) => Some(if b { "true".into() } else { "false".into() }),
            Some(v) => Some(v.to_string()),
            None => None,
        }
    }

    /// The publication's short name, with the title as an existing-site fallback.
    pub fn site_name(&self) -> String {
        ["name", "title"].iter()
            .filter_map(|key| self.get(key).and_then(|value| value.as_str().map(|value| value.trim().to_string())))
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| "Ghost".into())
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
    pub async fn set(&self, pool: &Db, key: &str, value: &Value) -> Result<()> {
        if self.is_locked(key) {
            return Err(GhostError::NoPermissionError { message: "Email delivery is managed by your hosting provider".into(), context: None });
        }
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
        let res = sqlx::query("UPDATE settings SET value = ?, type = ?, updated_at = ? WHERE `key` = ?")
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

    #[tokio::test]
    async fn deployment_mail_defaults_survive_reload_without_copying_credentials_to_the_database() {
        let test_db = crate::pool::connect_test().await.unwrap();
        let db = test_db.pool();
        crate::seed::seed_if_empty(&db).await.unwrap();
        let file = serde_json::json!({"transport":"SMTP","from":"Publication <support@example.test>","options":{"host":"smtp.example.test","port":465,"auth":{"user":"file-user","pass":"file-secret"}}});
        let settings = SettingsCache::load(&db).await.unwrap().with_mail_config(&file);
        assert_eq!(settings.get_str("mail_transport").as_deref(), Some("SMTP"));
        assert_eq!(settings.get_str("mail_smtp_security").as_deref(), Some("tls"));
        assert_eq!(settings.get_str("mail_smtp_pass").as_deref(), Some("file-secret"));
        assert_eq!(settings.get_str("mail_from_address").as_deref(), Some("support@example.test"));
        assert_eq!(settings.get_str("mail_from_name").as_deref(), Some("Publication"));
        settings.reload(&db).await.unwrap();
        assert_eq!(settings.get_str("mail_smtp_pass").as_deref(), Some("file-secret"));
        let stored: crate::types::Text = sqlx::query_scalar("SELECT value FROM settings WHERE `key`='mail_smtp_pass'").fetch_one(&db).await.unwrap();
        assert!(stored.0.is_empty());
        let saved_form = settings.with_overrides([
            ("mail_transport".into(), serde_json::json!("SMTP")),
            ("mail_smtp_host".into(), serde_json::json!("smtp.example.test")),
            ("mail_smtp_user".into(), serde_json::json!("file-user")),
            ("mail_smtp_pass".into(), serde_json::json!("")),
        ]);
        assert_eq!(saved_form.get_str("mail_smtp_pass").as_deref(), Some("file-secret"));
        let changed_host = saved_form.with_overrides([("mail_smtp_host".into(), serde_json::json!("different.example.test"))]);
        assert_eq!(changed_host.get_str("mail_smtp_pass").as_deref(), Some(""));
        let own = settings.with_overrides([("mail_transport".into(), serde_json::json!("SES")), ("ses_smtp_user".into(), serde_json::json!("own-user"))]);
        assert_eq!(own.get_str("mail_transport").as_deref(), Some("SES"));
        let managed = own.with_managed_mail(&serde_json::json!({"managedMail":{"mail_transport":"SES","ses_smtp_user":"host-user"}}));
        assert_eq!(managed.get_str("ses_smtp_user").as_deref(), Some("host-user"));
    }

    #[tokio::test]
    async fn managed_mail_survives_reload_and_refuses_writes() {
        let test_db = crate::pool::connect_test().await.unwrap();
        let db = test_db.pool();
        crate::seed::seed_if_empty(&db).await.unwrap();
        let settings = SettingsCache::load(&db).await.unwrap().with_managed_mail(&serde_json::json!({
            "managedMail": {"mail_transport":"SES", "ses_region":"eu-west-1", "ses_smtp_pass":"platform-secret", "title":"must-not-override"}
        }));
        let before = settings.get_str("title");
        assert!(settings.set(&db, "ses_smtp_pass", &serde_json::json!("tenant-secret")).await.is_err());
        assert!(settings.set(&db, "mailgun_api_key", &serde_json::json!("tenant-key")).await.is_err());
        assert!(!settings.is_locked("title"));
        assert_ne!(before.as_deref(), Some("must-not-override"));
        assert_eq!(settings.get_str("mailgun_api_key").as_deref(), Some(""));
        crate::repos::settings::upsert(&db, "ses_smtp_pass", "imported-secret", "string").await.unwrap();
        settings.reload(&db).await.unwrap();
        assert_eq!(settings.get_str("ses_smtp_pass").as_deref(), Some("platform-secret"));
        let preview = settings.with_overrides([("ses_smtp_pass".into(), serde_json::json!("preview-secret"))]);
        assert_eq!(preview.get_str("ses_smtp_pass").as_deref(), Some("platform-secret"));
        // Removing hosting management exposes the original standalone settings.
        let standalone = SettingsCache::load(&db).await.unwrap();
        assert!(!standalone.mail_is_managed());
        assert_eq!(standalone.get_str("ses_smtp_pass").as_deref(), Some("imported-secret"));
        standalone.set(&db, "ses_smtp_pass", &serde_json::json!("own-secret")).await.unwrap();
    }

}
