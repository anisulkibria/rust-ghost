//! Ghost-compatible layered configuration.
//!
//! Priority (highest wins), mirroring Ghost's nconf stack:
//! 1. `GHOST_*` environment variables (`__` = nesting separator)
//! 2. `config.<env>.json` next to the binary / in working dir
//! 3. Built-in defaults
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{GhostError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub url: String,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub paths: PathsConfig,
    #[serde(default)]
    pub mail: serde_json::Value,
    #[serde(default)]
    pub admin: serde_json::Value,
    /// Environment name this config was loaded for.
    #[serde(skip)]
    pub env: String,
    /// Directory the config file was loaded from (defaults to cwd).
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    2368
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_client")]
    pub client: String,
    pub connection: ConnectionConfig,
}

fn default_client() -> String {
    "sqlite3".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Ghost config files use `contentPath`; internal code uses snake_case.
    #[serde(default = "default_content_path", alias = "contentPath")]
    pub content_path: String,
}

fn default_content_path() -> String {
    "content".into()
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self { content_path: default_content_path() }
    }
}

impl Config {
    /// Load config for `env` (from `GHOST_ENV` or NODE_ENV or "development").
    pub fn load(env: Option<&str>) -> Result<Self> {
        let env = env
            .map(str::to_string)
            .or_else(|| std::env::var("GHOST_ENV").ok().or_else(|| std::env::var("NODE_ENV").ok()))
            .unwrap_or_else(|| "development".into());

        let base_dir = std::env::current_dir()?;
        let mut merged = default_config(&env);

        // config.<env>.json from cwd (Ghost's location)
        let user_file = base_dir.join(format!("config.{env}.json"));
        if user_file.exists() {
            let raw = std::fs::read_to_string(&user_file)?;
            let val: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| GhostError::ConfigError(format!("{}: {e}", user_file.display())))?;
            merge_into(&mut merged, val);
        }

        // Environment variable overrides: GHOST_DATABASE__CONNECTION__FILENAME=x
        apply_env_overrides(&mut merged);

        let mut cfg: Config = serde_json::from_value(merged)
            .map_err(|e| GhostError::ConfigError(format!("invalid config: {e}")))?;
        cfg.env = env;
        cfg.base_dir = base_dir;

        // Normalisations mirroring Ghost's config sanitation
        if !cfg.url.contains("://") {
            return Err(GhostError::ConfigError(format!("url needs a protocol: {}", cfg.url)));
        }
        // The path component of `url` becomes the base path the whole app
        // mounts under, so an unusable one has to be a boot failure with a
        // readable reason rather than a site that 404s everywhere.
        crate::url::normalize(&cfg.url_path()).map_err(|e| {
            GhostError::ConfigError(format!("config url '{}': {e}", cfg.url))
        })?;
        Ok(cfg)
    }

    pub fn content_path(&self) -> PathBuf {
        let p = Path::new(&self.paths.content_path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.content_path().join("themes")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.content_path().join("data")
    }

    pub fn images_dir(&self) -> PathBuf {
        self.content_path().join("images")
    }

    pub fn files_dir(&self) -> PathBuf {
        self.content_path().join("files")
    }

    pub fn media_dir(&self) -> PathBuf {
        self.content_path().join("media")
    }

    pub fn db_path(&self) -> PathBuf {
        let f = Path::new(&self.database.connection.filename);
        if f.is_absolute() {
            f.to_path_buf()
        } else {
            self.base_dir.join(f)
        }
    }

    /// Base path of the configured url: `""` at a domain root, `"/blog"` when
    /// the site is installed under a subdirectory.
    ///
    /// This is the authoritative base path — `Config::load` has already
    /// rejected any value that could not be mounted, so this cannot fail.
    pub fn subdir(&self) -> String {
        crate::url::normalize(&self.url_path()).unwrap_or_default()
    }

    fn url_path(&self) -> String {
        self.url
            .splitn(4, '/')
            .nth(3)
            .map(|s| format!("/{s}"))
            .unwrap_or_default()
    }

    /// The site url with any subdir stripped, no trailing slash.
    pub fn site_origin(&self) -> String {
        let parts: Vec<&str> = self.url.splitn(4, '/').collect();
        format!("{}//{}", parts[0], parts[2])
    }

    /// The configured url in canonical form: origin + base path, no trailing
    /// slash. `http://localhost:2368/blog/` normalizes to
    /// `http://localhost:2368/blog`, which is what every absolute URL the site
    /// emits is built from.
    pub fn canonical_url(&self) -> String {
        format!("{}{}", self.site_origin(), self.subdir())
    }
}

fn default_config(env: &str) -> serde_json::Value {
    let db_file = if env == "production" { "content/data/ghost.db" } else { "content/data/ghost-dev.db" };
    serde_json::json!({
        "url": "http://localhost:2368",
        "server": { "host": default_host(), "port": default_port() },
        "database": {
            "client": "sqlite3",
            "connection": { "filename": db_file }
        },
        "paths": { "contentPath": "content" },
        "mail": { "transport": "Direct" }
    })
}

fn merge_into(target: &mut serde_json::Value, src: serde_json::Value) {
    match (target, src) {
        (serde_json::Value::Object(t), serde_json::Value::Object(s)) => {
            for (k, v) in s {
                match t.get_mut(&k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge_into(existing, v),
                    _ => {
                        t.insert(k, v);
                    }
                }
            }
        }
        (t, s) => *t = s,
    }
}

fn apply_env_overrides(cfg: &mut serde_json::Value) {
    for (k, v) in std::env::vars() {
        if !k.starts_with("GHOST_") || k == "GHOST_ENV" {
            continue;
        }
        let path: Vec<String> = k["GHOST_".len()..].split("__").map(|s| snake_to_camel(s)).collect();
        let val: serde_json::Value = parse_env_value(&v);
        // navigate + set
        set_path(cfg, &path, val);
    }
}

fn set_path(cfg: &mut serde_json::Value, path: &[String], val: serde_json::Value) {
    let last = path.len() - 1;
    let mut cur = cfg;
    for (i, seg) in path.iter().enumerate() {
        if i == last {
            cur[seg] = val;
            return;
        }
        if !cur.get(seg).map(|c| c.is_object()).unwrap_or(false) {
            cur[seg] = serde_json::json!({});
        }
        cur = cur.get_mut(seg).unwrap();
    }
}

fn parse_env_value(v: &str) -> serde_json::Value {
    match v {
        "true" => serde_json::json!(true),
        "false" => serde_json::json!(false),
        "null" => serde_json::json!(null),
        _ => {
            if let Ok(n) = v.parse::<i64>() {
                serde_json::json!(n)
            } else {
                serde_json::json!(v)
            }
        }
    }
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::new();
    let mut upper_next = false;
    for c in s.chars() {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c.to_lowercase().next().unwrap_or(c));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Config::load` reads the filesystem and the environment; the base-path
    /// derivation under test here is pure, so build the value directly.
    fn with_url(url: &str) -> Config {
        Config {
            url: url.into(),
            server: ServerConfig { host: default_host(), port: default_port() },
            database: DatabaseConfig {
                client: default_client(),
                connection: ConnectionConfig { filename: "x.db".into() },
            },
            paths: PathsConfig::default(),
            mail: serde_json::Value::Null,
            admin: serde_json::Value::Null,
            env: "test".into(),
            base_dir: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn derives_the_base_path_from_the_url() {
        assert_eq!(with_url("https://site.com").subdir(), "");
        assert_eq!(with_url("https://site.com/").subdir(), "");
        assert_eq!(with_url("https://site.com/blog").subdir(), "/blog");
        assert_eq!(with_url("https://site.com/blog/").subdir(), "/blog");
        assert_eq!(with_url("https://site.com/a/b/").subdir(), "/a/b");
        // a port must not be mistaken for part of the path
        assert_eq!(with_url("http://localhost:2368").subdir(), "");
        assert_eq!(with_url("http://localhost:2368/blog").subdir(), "/blog");
    }

    #[test]
    fn canonical_url_is_origin_plus_base_path() {
        assert_eq!(with_url("https://site.com/blog/").canonical_url(), "https://site.com/blog");
        assert_eq!(with_url("https://site.com/").canonical_url(), "https://site.com");
        assert_eq!(
            with_url("http://localhost:2368/blog").canonical_url(),
            "http://localhost:2368/blog"
        );
    }

    #[test]
    fn site_origin_drops_the_base_path() {
        assert_eq!(with_url("https://site.com/blog").site_origin(), "https://site.com");
        assert_eq!(with_url("http://localhost:2368/a/b").site_origin(), "http://localhost:2368");
    }
}
