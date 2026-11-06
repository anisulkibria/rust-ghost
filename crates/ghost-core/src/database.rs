//! Which database this site runs on, and how to reach it.
//!
//! Ghost's config names the engine the way knex does — a `client` and a
//! `connection` — and this module turns that into something sqlx can open:
//!
//! ```json
//! { "database": { "client": "sqlite3",
//!                 "connection": { "filename": "content/data/ghost.db" } } }
//!
//! { "database": { "client": "mysql",
//!                 "connection": { "host": "db.internal", "port": 3306,
//!                                 "user": "ghost_abc", "password": "…",
//!                                 "database": "ghost_abc" } } }
//! ```
//!
//! Both shapes are accepted because both are real. A single site installed by
//! hand is a file, needs no server, and is what `ghost serve` does with no
//! configuration at all. A site provisioned by a host — Ghost Pro, or this
//! project's own control plane — is handed a database and a login of its own
//! on a shared MySQL server, and that is written into exactly this block.
//!
//! Keeping the same vocabulary as upstream Ghost is deliberate: an operator
//! moving a site between stock Ghost and this one should not have to rewrite
//! their config, and a host that already knows how to provision for Ghost
//! Pro already knows how to provision for this.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::error::{GhostError, Result};

/// The database engines a site can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Sqlite,
    MySql,
}

impl Engine {
    /// The name used in logs and in `ghost doctor`.
    pub fn name(self) -> &'static str {
        match self {
            Engine::Sqlite => "sqlite",
            Engine::MySql => "mysql",
        }
    }

    /// Read a knex `client` name.
    ///
    /// `mysql2` is accepted because it is the driver name Ghost itself moved
    /// to, and a config copied from a modern Ghost install will say that.
    pub fn from_client(client: &str) -> Result<Engine> {
        match client.trim().to_ascii_lowercase().as_str() {
            "sqlite3" | "sqlite" | "better-sqlite3" => Ok(Engine::Sqlite),
            "mysql" | "mysql2" | "mariadb" => Ok(Engine::MySql),
            other => Err(GhostError::ConfigError(format!(
                "database.client is {other:?}, which this build cannot open. \
                 Use \"sqlite3\" for a file or \"mysql\" for a server"
            ))),
        }
    }
}

/// A database this site can open: an engine plus the URL to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    engine: Engine,
    url: String,
}

impl Target {
    /// A SQLite database in a file, created on first open.
    pub fn sqlite_file(path: &Path) -> Self {
        // `?mode=rwc` is what creates the file when it is absent. It has to be
        // in the URL rather than set on an options struct, because the `Any`
        // driver reaches SQLite through `SqliteConnectOptions::from_url` and
        // discards anything not expressible as a URL parameter.
        Target { engine: Engine::Sqlite, url: format!("sqlite://{}?mode=rwc", path.display()) }
    }

    /// A private SQLite database that exists only while it is open, for tests.
    pub fn sqlite_memory() -> Self {
        Target { engine: Engine::Sqlite, url: "sqlite::memory:".into() }
    }

    /// A MySQL server, from the parts a knex `connection` block carries.
    ///
    /// The password is percent-encoded, which matters more than it looks: a
    /// host generates these, and a generated password containing `@`, `/` or
    /// `#` would otherwise be parsed as part of the host or the path and the
    /// site would fail to connect for reasons nothing explains.
    pub fn mysql(host: &str, port: u16, user: &str, password: &str, database: &str) -> Self {
        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            encode(user),
            encode(password),
            host,
            port,
            database
        );
        Target { engine: Engine::MySql, url }
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    /// The URL to hand to sqlx. Carries the password; never log this.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The URL with the password replaced, for logs and `doctor`.
    ///
    /// Every path that tells an operator which database they are on goes
    /// through here. On a hosted site this password was issued by the platform
    /// and belongs to that site alone, and writing it into a log file the
    /// site's own owner can download would hand it to them.
    pub fn redacted(&self) -> String {
        redact(&self.url)
    }

    /// The file a SQLite target lives in, or `None` for a server or memory.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        if self.engine != Engine::Sqlite {
            return None;
        }
        let rest =
            self.url.strip_prefix("sqlite://").or_else(|| self.url.strip_prefix("sqlite:"))?;
        let path = rest.split('?').next().unwrap_or(rest);
        (!path.is_empty() && path != ":memory:").then(|| PathBuf::from(path))
    }
}

/// Percent-encode the characters that would otherwise change a URL's meaning.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Replace the password in a connection URL with `***`.
///
/// Conservative by design: anything that does not look like
/// `scheme://user:pass@host` is returned untouched, because a URL this does
/// not understand is better shown in full than mangled into something an
/// operator cannot recognise. A SQLite path has no credential to hide.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let (authority, tail) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    // The host is after the *last* `@`: a password may contain one.
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return url.to_string();
    };
    if !userinfo.contains(':') {
        return url.to_string();
    }
    let user = userinfo.split_once(':').map(|(u, _)| u).unwrap_or(userinfo);
    format!("{scheme}://{user}:***@{host}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knex_client_names_map_to_engines() {
        for name in ["sqlite3", "sqlite", "better-sqlite3", "SQLite3"] {
            assert_eq!(Engine::from_client(name).unwrap(), Engine::Sqlite, "{name}");
        }
        // `mysql2` is what a config from a current Ghost install says.
        for name in ["mysql", "mysql2", "mariadb", "MySQL"] {
            assert_eq!(Engine::from_client(name).unwrap(), Engine::MySql, "{name}");
        }
    }

    #[test]
    fn an_unsupported_client_is_refused_by_name() {
        let err = Engine::from_client("pg").unwrap_err().to_string();
        assert!(err.contains("pg"), "{err}");
        assert!(err.contains("sqlite3"), "{err}");
    }

    #[test]
    fn a_file_target_creates_itself() {
        let t = Target::sqlite_file(Path::new("content/data/ghost.db"));
        assert_eq!(t.engine(), Engine::Sqlite);
        assert!(t.url().contains("mode=rwc"), "{}", t.url());
        assert_eq!(t.sqlite_path().unwrap(), PathBuf::from("content/data/ghost.db"));
    }

    #[test]
    fn memory_is_not_a_path() {
        assert!(Target::sqlite_memory().sqlite_path().is_none());
    }

    #[test]
    fn a_mysql_target_is_built_from_its_parts() {
        let t = Target::mysql("db.internal", 3307, "ghost_abc", "secret", "ghost_abc");
        assert_eq!(t.engine(), Engine::MySql);
        assert_eq!(t.url(), "mysql://ghost_abc:secret@db.internal:3307/ghost_abc");
        assert!(t.sqlite_path().is_none());
    }

    /// A host generates these passwords, and one containing `@` or `/` would
    /// otherwise be read as part of the host or the path.
    #[test]
    fn an_awkward_password_is_encoded() {
        let t = Target::mysql("db", 3306, "u", "p@ss/w#rd?", "d");
        assert!(t.url().contains("p%40ss%2Fw%23rd%3F"), "{}", t.url());
        assert!(t.url().ends_with("@db:3306/d"), "{}", t.url());
    }

    /// The whole point of `redacted`: this string ends up in a log file.
    #[test]
    fn a_password_never_reaches_a_log() {
        let t = Target::mysql("db.internal", 3306, "ghost", "s3cret", "ghost");
        assert_eq!(t.redacted(), "mysql://ghost:***@db.internal:3306/ghost");
        assert!(!t.redacted().contains("s3cret"));
    }

    #[test]
    fn a_url_with_no_credential_is_left_alone() {
        assert_eq!(redact("mysql://db.internal/g"), "mysql://db.internal/g");
        assert_eq!(
            redact("sqlite://content/data/ghost.db?mode=rwc"),
            "sqlite://content/data/ghost.db?mode=rwc"
        );
    }
}

// ---------------------------------------------------------------------------
// The engine in use
// ---------------------------------------------------------------------------

/// Which engine the open database is on.
///
/// A process serves exactly one site and therefore has one database, so this
/// is a property of the process rather than something to thread through a
/// thousand query functions. `ghost_db::pool::open` sets it; [`crate::dialect`]
/// reads it, and so does `ghost_nql`, which is why it lives here rather than
/// in `ghost-db` — `ghost-nql` sits below that crate and still has to know how
/// to spell a date comparison.
static ENGINE: AtomicU8 = AtomicU8::new(ENGINE_SQLITE);
const ENGINE_SQLITE: u8 = 0;
const ENGINE_MYSQL: u8 = 1;

/// The engine in use. SQLite until a database is opened, which is also the
/// right answer for a test on an in-memory one.
pub fn engine() -> Engine {
    match ENGINE.load(Ordering::Relaxed) {
        ENGINE_MYSQL => Engine::MySql,
        _ => Engine::Sqlite,
    }
}

/// Record the engine a pool was just opened against.
pub fn set_engine(engine: Engine) {
    ENGINE.store(
        match engine {
            Engine::Sqlite => ENGINE_SQLITE,
            Engine::MySql => ENGINE_MYSQL,
        },
        Ordering::Relaxed,
    );
}
