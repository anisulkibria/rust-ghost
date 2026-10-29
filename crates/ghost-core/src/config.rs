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
    #[serde(default)]
    pub storage: serde_json::Value,
    /// What the *host* has decided about this site, as opposed to what its
    /// owner has: plan limits, the billing URL, the subscription's start date.
    ///
    /// Empty on a self-hosted install, which is what makes every limit
    /// inactive there — see [`crate::limits`]. It is config rather than
    /// settings deliberately: these are terms the site is being run under, so
    /// the site's own admin screens must not be able to edit them.
    #[serde(default, rename = "hostSettings")]
    pub host_settings: serde_json::Value,
    /// HTTPS. `true` means "get certificates from Let's Encrypt and serve
    /// them"; see [`Ssl`].
    #[serde(default)]
    pub ssl: Ssl,
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
    /// Left unset so the default can depend on HTTPS: 443 with TLS, 2368
    /// without. Read it through [`Config::port`] rather than directly.
    #[serde(default)]
    pub port: Option<u16>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { host: default_host(), port: None }
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}

/// The `ssl` config value, which takes either a bool or an options object:
///
/// ```json
/// "ssl": true
/// "ssl": { "email": "admin@example.com", "staging": true }
/// "ssl": { "mode": "manual", "cert": "/etc/ssl/site.pem", "key": "/etc/ssl/site.key" }
/// ```
///
/// `true` is the whole configuration for a normal HTTPS site: certificates are
/// issued and renewed automatically for the host in `url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Ssl {
    /// `"ssl": true` / `"ssl": false`
    Toggle(bool),
    /// `"ssl": { … }`
    Options(Box<SslOptions>),
}

impl Default for Ssl {
    fn default() -> Self {
        Ssl::Toggle(false)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SslOptions {
    /// `"auto"` (Let's Encrypt, the default), `"manual"`, or `"off"`.
    #[serde(default)]
    pub mode: Option<String>,
    /// Hostnames to get certificates for. Defaults to the host in `url`.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Contact address registered with the certificate authority, which is
    /// where expiry warnings go if renewal ever stops working.
    #[serde(default)]
    pub email: Option<String>,
    /// Use Let's Encrypt's staging CA. Its certificates are not trusted by
    /// browsers, but its rate limits are far looser — the right place to
    /// confirm a setup works before spending real quota on it.
    #[serde(default)]
    pub staging: bool,
    /// `mode: "manual"`: PEM certificate chain and private key.
    #[serde(default)]
    pub cert: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    /// Where issued certificates are cached between restarts.
    #[serde(default, alias = "cacheDir")]
    pub cache_dir: Option<String>,
    /// Port for the HTTP listener that redirects to HTTPS. `0` disables it.
    /// Defaults to 80, which is also where Let's Encrypt's HTTP challenge
    /// would arrive.
    #[serde(default, alias = "redirectHttp")]
    pub redirect_http: Option<u16>,
}

/// How the server should listen: the resolved form of `server` + `ssl`.
#[derive(Debug, Clone)]
pub struct ServePlan {
    pub host: String,
    pub port: u16,
    pub tls: TlsPlan,
    /// Port serving permanent redirects to the canonical HTTPS url.
    pub redirect_http: Option<u16>,
}

#[derive(Debug, Clone)]
pub enum TlsPlan {
    /// Plain HTTP. Correct on its own for local use, and behind a proxy that
    /// terminates TLS itself.
    Off,
    /// Certificates issued and renewed automatically over ACME.
    Acme { domains: Vec<String>, email: Option<String>, staging: bool, cache_dir: PathBuf },
    /// Certificates supplied as files.
    Manual { cert: PathBuf, key: PathBuf },
}

impl ServePlan {
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn is_tls(&self) -> bool {
        !matches!(self.tls, TlsPlan::Off)
    }
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

    /// Hostname from `url`, with any port removed.
    pub fn host(&self) -> String {
        self.url
            .splitn(4, '/')
            .nth(2)
            .unwrap_or_default()
            .rsplit_once(':')
            // only a trailing *port* is stripped, not part of an IPv6 literal
            .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty())
            .map(|(host, _)| host)
            .unwrap_or_else(|| self.url.splitn(4, '/').nth(2).unwrap_or_default())
            .to_string()
    }

    /// Whether `url` names a host a public certificate authority could
    /// validate. `localhost`, `*.local` and bare IPs cannot be.
    pub fn host_is_public(&self) -> bool {
        let host = self.host();
        if host.is_empty() || host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") {
            return false;
        }
        // an IP address has no DNS record to validate
        if host.parse::<std::net::IpAddr>().is_ok() {
            return false;
        }
        host.contains('.')
    }

    pub fn port(&self) -> u16 {
        self.serve_plan().map(|p| p.port).unwrap_or(2368)
    }

    /// Resolve `server` + `ssl` into one listening plan.
    ///
    /// Defaults are chosen so that the common cases need almost no config:
    /// `"ssl": true` alone is a complete HTTPS setup, because the domain comes
    /// from `url`, the port defaults to 443 and the HTTP redirect to 80.
    pub fn serve_plan(&self) -> Result<ServePlan> {
        let opts = match &self.ssl {
            Ssl::Toggle(false) => None,
            Ssl::Toggle(true) => Some(SslOptions::default()),
            Ssl::Options(o) => Some((**o).clone()),
        };
        let mode = opts
            .as_ref()
            .and_then(|o| o.mode.clone())
            .unwrap_or_else(|| match opts.is_some() {
                true => "auto".into(),
                false => "off".into(),
            });

        let tls = match mode.as_str() {
            "off" | "false" | "none" => TlsPlan::Off,
            "manual" | "file" | "files" => {
                let o = opts.clone().unwrap_or_default();
                let (Some(cert), Some(key)) = (o.cert.clone(), o.key.clone()) else {
                    return Err(GhostError::ConfigError(
                        "ssl mode \"manual\" needs both \"cert\" and \"key\" (paths to PEM files)".into(),
                    ));
                };
                TlsPlan::Manual { cert: self.resolve_path(&cert), key: self.resolve_path(&key) }
            }
            "auto" | "acme" | "letsencrypt" | "true" => {
                let o = opts.clone().unwrap_or_default();
                let domains = match o.domains.is_empty() {
                    true => vec![self.host()],
                    false => o.domains.clone(),
                };
                if !self.host_is_public() && o.domains.is_empty() {
                    return Err(GhostError::ConfigError(format!(
                        "automatic certificates need a public domain, but url is '{}'. \
                         Use a real hostname, or for local HTTPS set \
                         ssl.mode to \"manual\" with your own cert and key.",
                        self.url
                    )));
                }
                TlsPlan::Acme {
                    domains,
                    email: o.email.clone(),
                    staging: o.staging,
                    cache_dir: o
                        .cache_dir
                        .as_deref()
                        .map(|d| self.resolve_path(d))
                        .unwrap_or_else(|| self.content_path().join("acme")),
                }
            }
            other => {
                return Err(GhostError::ConfigError(format!(
                    "unknown ssl mode '{other}' (expected \"auto\", \"manual\" or \"off\")"
                )))
            }
        };

        let is_tls = !matches!(tls, TlsPlan::Off);
        // Serving HTTPS on a url that says http:// would make every link the
        // site writes point at a port that is not listening.
        if is_tls && self.url.starts_with("http://") {
            return Err(GhostError::ConfigError(format!(
                "ssl is on but url is '{}' — change it to 'https://{}'",
                self.url,
                self.url.trim_start_matches("http://")
            )));
        }

        let port = self.server.port.unwrap_or(if is_tls { 443 } else { 2368 });
        let redirect_http = match (is_tls, opts.as_ref().and_then(|o| o.redirect_http)) {
            (false, _) => None,
            (true, Some(0)) => None,
            (true, Some(p)) => Some(p),
            (true, None) => Some(80),
        };

        Ok(ServePlan { host: self.server.host.clone(), port, tls, redirect_http })
    }

    /// Resolve a configured path against the config's directory, so relative
    /// paths in a config file mean what the person writing it expected.
    fn resolve_path(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        match path.is_absolute() {
            true => path.to_path_buf(),
            false => self.base_dir.join(path),
        }
    }
}

fn default_config(env: &str) -> serde_json::Value {
    let db_file = if env == "production" { "content/data/ghost.db" } else { "content/data/ghost-dev.db" };
    serde_json::json!({
        "url": "http://localhost:2368",
        // port is left out on purpose: its default depends on whether TLS is
        // on, which `serve_plan` decides
        "server": { "host": default_host() },
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
    pub(super) fn with_url(url: &str) -> Config {
        Config {
            url: url.into(),
            server: ServerConfig::default(),
            database: DatabaseConfig {
                client: default_client(),
                connection: ConnectionConfig { filename: "x.db".into() },
            },
            paths: PathsConfig::default(),
            mail: serde_json::Value::Null,
            admin: serde_json::Value::Null,
            storage: serde_json::Value::Null,
            host_settings: serde_json::Value::Null,
            ssl: Ssl::default(),
            env: "test".into(),
            base_dir: PathBuf::from("/tmp"),
        }
    }

    pub(super) fn with_ssl(url: &str, ssl: Ssl) -> Config {
        Config { ssl, ..with_url(url) }
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

#[cfg(test)]
mod serve_plan_tests {
    use super::tests::{with_ssl, with_url};
    use super::*;

    #[test]
    fn plain_http_is_the_default() {
        let plan = with_url("http://localhost:2368").serve_plan().unwrap();
        assert!(matches!(plan.tls, TlsPlan::Off));
        assert_eq!(plan.port, 2368);
        assert_eq!(plan.redirect_http, None);
    }

    /// `"ssl": true` has to be a complete HTTPS configuration on its own — the
    /// domain, the port and the redirect all follow from `url`.
    #[test]
    fn ssl_true_is_a_whole_https_setup() {
        let plan = with_ssl("https://example.com", Ssl::Toggle(true)).serve_plan().unwrap();
        assert_eq!(plan.port, 443);
        assert_eq!(plan.redirect_http, Some(80));
        match plan.tls {
            TlsPlan::Acme { domains, staging, .. } => {
                assert_eq!(domains, vec!["example.com".to_string()]);
                assert!(!staging);
            }
            other => panic!("expected acme, got {other:?}"),
        }
    }

    #[test]
    fn explicit_port_still_wins() {
        let mut cfg = with_ssl("https://example.com", Ssl::Toggle(true));
        cfg.server.port = Some(8443);
        assert_eq!(cfg.serve_plan().unwrap().port, 8443);
    }

    #[test]
    fn redirect_can_be_turned_off() {
        let cfg = with_ssl(
            "https://example.com",
            Ssl::Options(Box::new(SslOptions { redirect_http: Some(0), ..Default::default() })),
        );
        assert_eq!(cfg.serve_plan().unwrap().redirect_http, None);
    }

    #[test]
    fn manual_mode_takes_cert_and_key() {
        let cfg = with_ssl(
            "https://example.com",
            Ssl::Options(Box::new(SslOptions {
                mode: Some("manual".into()),
                cert: Some("/etc/ssl/a.pem".into()),
                key: Some("relative.key".into()),
                ..Default::default()
            })),
        );
        match cfg.serve_plan().unwrap().tls {
            TlsPlan::Manual { cert, key } => {
                assert_eq!(cert, PathBuf::from("/etc/ssl/a.pem"));
                // relative paths resolve against the config's directory
                assert_eq!(key, PathBuf::from("/tmp/relative.key"));
            }
            other => panic!("expected manual, got {other:?}"),
        }
    }

    #[test]
    fn manual_mode_without_files_is_refused() {
        let cfg = with_ssl(
            "https://example.com",
            Ssl::Options(Box::new(SslOptions { mode: Some("manual".into()), ..Default::default() })),
        );
        assert!(cfg.serve_plan().is_err());
    }

    /// The two ways a config can contradict itself. Both messages have to name
    /// the fix, since this is the first thing a new install hits.
    #[test]
    fn contradictions_are_refused_with_a_usable_message() {
        let http_with_ssl = with_ssl("http://example.com", Ssl::Toggle(true));
        let err = http_with_ssl.serve_plan().unwrap_err().message();
        assert!(err.contains("https://example.com"), "{err}");

        let local_with_acme = with_ssl("https://localhost", Ssl::Toggle(true));
        let err = local_with_acme.serve_plan().unwrap_err().message();
        assert!(err.contains("manual"), "{err}");
    }

    #[test]
    fn unknown_mode_is_refused() {
        let cfg = with_ssl(
            "https://example.com",
            Ssl::Options(Box::new(SslOptions { mode: Some("magic".into()), ..Default::default() })),
        );
        assert!(cfg.serve_plan().is_err());
    }

    #[test]
    fn host_parsing_handles_ports_and_paths() {
        assert_eq!(with_url("https://example.com").host(), "example.com");
        assert_eq!(with_url("https://example.com/blog").host(), "example.com");
        assert_eq!(with_url("http://localhost:2368").host(), "localhost");
        assert_eq!(with_url("http://127.0.0.1:2368/blog").host(), "127.0.0.1");
    }

    #[test]
    fn only_real_domains_can_get_automatic_certificates() {
        assert!(with_url("https://example.com").host_is_public());
        assert!(with_url("https://blog.example.co.uk/x").host_is_public());
        assert!(!with_url("http://localhost:2368").host_is_public());
        assert!(!with_url("http://127.0.0.1:2368").host_is_public());
        assert!(!with_url("https://mymac.local").host_is_public());
        assert!(!with_url("https://intranet").host_is_public());
    }

    /// `ssl` accepts a bool or an object; both shapes come from real configs.
    #[test]
    fn ssl_deserializes_from_bool_or_object() {
        let t: Ssl = serde_json::from_str("true").unwrap();
        assert!(matches!(t, Ssl::Toggle(true)));
        let o: Ssl = serde_json::from_str(r#"{"email":"a@b.c","staging":true}"#).unwrap();
        match o {
            Ssl::Options(o) => {
                assert_eq!(o.email.as_deref(), Some("a@b.c"));
                assert!(o.staging);
            }
            other => panic!("expected options, got {other:?}"),
        }
    }
}
