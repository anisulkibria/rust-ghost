//! Opening the site's database, on either engine.
//!
//! A site runs on SQLite — one file, no setup, which is what `ghost serve`
//! does out of the box — or on MySQL, which is what a host provisions when it
//! puts many sites on one server. Which one is decided by
//! [`ghost_core::Config::db_target`] from the `database` block, and the query
//! code above this module does not know the answer.
//!
//! Three things about sqlx's `Any` driver shape everything here:
//!
//! **It carries a narrow set of types** — `bool`, `i16`/`i32`/`i64`,
//! `f32`/`f64`, `String`, `Vec<u8>`. There is no timestamp type, which is why
//! every date in this schema is a `'YYYY-MM-DD HH:MM:SS'` string, as Ghost's
//! own schema has them. A column added as a native `DATETIME` would not
//! decode.
//!
//! **MySQL sends `TEXT` as a blob**, so a long column has to be read through
//! [`crate::types::Text`]. See that module; it is the reason this project can
//! keep Ghost's schema rather than rewriting it.
//!
//! **It discards per-engine connect options.** SQLite is reached through
//! `SqliteConnectOptions::from_url`, whose parser understands only `mode`,
//! `cache`, `immutable` and `vfs` — so WAL, foreign keys and the busy timeout
//! cannot be set the way they were before. They are applied in
//! [`after_connect`] instead, on every new connection. Losing them is a
//! correctness regression, not a slow one.
use ghost_core::database::{Engine, Target};
use ghost_core::{GhostError, Result};
use sqlx::any::{install_default_drivers, AnyPoolOptions};
use sqlx::migrate::MigrateDatabase;
use sqlx::{Any, AnyPool, Executor};
use std::path::Path;

/// The site's connection pool, whichever engine is behind it.
pub type Db = AnyPool;

/// The engine this site is on.
///
/// Delegates to [`ghost_core::database::engine`], which is where the flag
/// lives so that `ghost-nql` — below this crate in the dependency graph — can
/// read it as well.
pub fn engine() -> Engine {
    ghost_core::database::engine()
}

fn remember(engine: Engine) {
    ghost_core::database::set_engine(engine);
}

/// Open the pool for a configured target and run the migrations.
pub async fn open(target: &Target) -> Result<Db> {
    // `Any` panics on first use if no driver is registered, and registering is
    // idempotent. Done here so no caller — including a test — can forget.
    install_default_drivers();
    remember(target.engine());

    match target.engine() {
        Engine::Sqlite => {
            if let Some(path) = target.sqlite_path() {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
            }
        }
        Engine::MySql => ensure_database(target).await?,
    }

    let engine = target.engine();
    let pool = AnyPoolOptions::new()
        // A site is a single process serving its own traffic; MySQL handles
        // real concurrency where SQLite serialises writes, so a larger pool
        // helps on one and only adds lock contention on the other.
        .max_connections(match engine {
            Engine::Sqlite => 5,
            Engine::MySql => 16,
        })
        .acquire_timeout(std::time::Duration::from_secs(10))
        .after_connect(move |conn, _| Box::pin(async move { after_connect(engine, conn).await }))
        .connect(target.url())
        .await
        .map_err(|e| {
            // Named from the *redacted* url: on a hosted site this password
            // was issued by the platform, and this message reaches a log the
            // site's owner can read.
            GhostError::Db(format!("failed to open database {}: {e}", target.redacted()))
        })?;

    migrate_at(&pool, target.sqlite_path().as_deref()).await?;
    Ok(pool)
}

/// Open the SQLite pool for a file. The long-standing entry point.
pub async fn connect(db_path: &Path) -> Result<Db> {
    open(&Target::sqlite_file(db_path)).await
}

/// An in-memory database with the schema applied, for tests.
///
/// `max_connections(1)` matters: each connection to `:memory:` would otherwise
/// get its own empty database, so a test that wrote on one connection and read
/// on another would see nothing.
pub async fn connect_in_memory() -> Result<Db> {
    install_default_drivers();
    remember(Engine::Sqlite);

    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| Box::pin(async move { after_connect(Engine::Sqlite, conn).await }))
        .connect(Target::sqlite_memory().url())
        .await
        .map_err(|e| GhostError::Db(format!("failed to open in-memory database: {e}")))?;

    migrate(&pool).await?;
    Ok(pool)
}

// ---------------------------------------------------------------------------
// Databases for tests
// ---------------------------------------------------------------------------

/// Names a MySQL server for the test suites to run against, for example
/// `mysql://root:rootpw@127.0.0.1:13306`. Set it and every fixture that
/// opens a database through [`connect_test`] runs on MySQL instead of
/// SQLite — the whole suite, not a token few queries, because the failures
/// this engine produces happen at *read* time and only surface under the
/// real schema.
pub const TEST_MYSQL_URL: &str = "GHOST_TEST_MYSQL_URL";

/// A database for one test, on whichever engine the run selected.
///
/// SQLite gives each test a private in-memory database. MySQL cannot: a
/// second connection to the same name would share one database, and the
/// tests assume one each — same slugs, same seeds. So [`connect_test`]
/// creates a database named for the test run on the server
/// [`TEST_MYSQL_URL`] names, and this value drops it when the test is done.
///
/// Keep it alive for as long as the test uses the pool — a field on the
/// fixture next to the state that holds a clone of the pool is the shape
/// that works.
pub struct TestDb {
    pool: Db,
    // Held for its `Drop`, which removes the MySQL database; the leading
    // underscore is the usual signal for that, not a sign it is unused.
    _drop: Option<DropDatabase>,
}

impl TestDb {
    /// The pool to query through, and to hand to the code under test.
    ///
    /// The clone shares one underlying pool; the original stays here so the
    /// database lives exactly as long as the test does.
    pub fn pool(&self) -> Db {
        self.pool.clone()
    }
}

/// What `TestDb` needs in order to remove a MySQL database: where, and which.
struct DropDatabase {
    server: String,
    name: String,
}

/// Drop the database this test created.
///
/// Synchronous on purpose: a detached thread would be killed by process exit
/// just as the last test finished, and the whole point is that a run of the
/// suite leaves the server as it found it. `Drop` cannot be async, so the
/// work runs on a fresh thread joined from here — which briefly blocks the
/// thread dropping the fixture, a trade a test is the right place to make.
///
/// Retried because the drop can briefly lose a race with the pool's own
/// connections being torn down; a database left behind by a failed retry is
/// untidy, not fatal, and the next `tools/mysql-test-cleanup` run removes it.
impl Drop for DropDatabase {
    fn drop(&mut self) {
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return,
                };
                rt.block_on(async {
                    for _ in 0..3 {
                        if let Ok(admin) = sqlx::AnyPool::connect(&format!("{}/mysql", self.server)).await {
                            // Bound the wait rather than queue behind a lock
                            // for ever: a wedged drop should fail and be
                            // retried, not hang the test binary.
                            let _ = sqlx::query("SET SESSION lock_wait_timeout = 5")
                                .execute(&admin)
                                .await;
                            // The pool this test used may still hold
                            // connections open against its database. sqlx
                            // rolls a dropped `Transaction` back *lazily*, so
                            // a connection returned to the pool mid-
                            // transaction sits idle holding InnoDB's metadata
                            // lock, and `DROP DATABASE` would wait on it
                            // indefinitely. The test is over; its connections
                            // have no further use. `CONNECTION_ID()` is this
                            // admin session, excluded so the drop does not
                            // kill its own connection.
                            if let Ok(users) = sqlx::query_scalar::<_, i64>(
                                "SELECT id FROM information_schema.processlist \
                                 WHERE db = ? AND id <> CONNECTION_ID()",
                            )
                            .bind(&self.name)
                            .fetch_all(&admin)
                            .await
                            {
                                for id in users {
                                    let _ = sqlx::query(&format!("KILL {id}"))
                                        .execute(&admin)
                                        .await;
                                }
                            }
                            let dropped = sqlx::query(&format!("DROP DATABASE IF EXISTS `{}`", self.name))
                                .execute(&admin)
                                .await
                                .is_ok();
                            admin.close().await;
                            if dropped {
                                return;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                });
            })
            .join()
            .ok();
        });
    }
}

/// Open a database for one test: MySQL when [`TEST_MYSQL_URL`] names a
/// server, an in-memory SQLite otherwise.
///
/// Tests written against this run on both engines: SQLite by default —
/// instant, no server — and the whole suite on MySQL in the environments
/// that have one, which is the only way to meet the read-time failures the
/// `Any` driver produces. See `crates/ghost-db/src/types.rs` for the shape
/// of those.
///
/// The engine is a property of the process (see [`engine`]), so a binary
/// whose fixtures all come through here is uniformly on one engine; mixing
/// this with `connect_in_memory` inside one binary would leave whichever
/// opened last deciding for both.
pub async fn connect_test() -> Result<TestDb> {
    let server = std::env::var(TEST_MYSQL_URL).ok().filter(|s| !s.trim().is_empty());
    let Some(server) = server else {
        return Ok(TestDb { pool: connect_in_memory().await?, _drop: None });
    };
    let server = server.trim_end_matches('/').to_string();

    install_default_drivers();
    // `new_id` is 24 hex characters: valid as an unquoted identifier on
    // MySQL, and unique across every test in every binary of a run.
    let name = format!("ghost_test_{}", ghost_core::ids::new_id());
    let admin = AnyPool::connect(&format!("{server}/mysql"))
        .await
        .map_err(|e| GhostError::Db(format!("could not reach the test MySQL server: {e}")))?;
    sqlx::query(&format!(
        "CREATE DATABASE `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    ))
    .execute(&admin)
    .await
    .map_err(|e| GhostError::Db(format!("could not create the test database {name}: {e}")))?;
    admin.close().await;

    remember(Engine::MySql);
    let pool = AnyPoolOptions::new()
        // Enough for a fixture that serves requests while a query is in
        // flight; small enough that a parallel suite stays well inside the
        // server's `max_connections`.
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .after_connect(move |conn, _| Box::pin(async move { after_connect(Engine::MySql, conn).await }))
        .connect(&format!("{server}/{name}"))
        .await
        .map_err(|e| GhostError::Db(format!("failed to open test database {name}: {e}")))?;

    migrate(&pool).await?;
    Ok(TestDb { pool, _drop: Some(DropDatabase { server, name }) })
}

/// Settings applied to every new connection.
///
/// `after_connect` rather than connect options, because `Any` drops per-engine
/// options; see the module docs.
async fn after_connect(
    engine: Engine,
    conn: &mut sqlx::AnyConnection,
) -> std::result::Result<(), sqlx::Error> {
    match engine {
        Engine::Sqlite => {
            // Each fixes a specific way SQLite misbehaves under a web server:
            //
            // * **WAL** lets readers continue while a writer commits, so
            //   rendering a page does not block on a counter being stamped.
            // * **busy_timeout** makes a contended write wait rather than
            //   fail; SQLite otherwise returns `SQLITE_BUSY` at once, which
            //   surfaces as a random 500.
            // * **foreign_keys** is *off* by default in SQLite, so the
            //   `REFERENCES` clauses in the schema would be documentation
            //   rather than constraints.
            // * **synchronous = NORMAL** is the right trade with WAL on: a
            //   commit survives a crash, and only power loss can lose the last
            //   transactions.
            conn.execute("PRAGMA journal_mode = WAL").await?;
            conn.execute("PRAGMA busy_timeout = 5000").await?;
            conn.execute("PRAGMA foreign_keys = ON").await?;
            conn.execute("PRAGMA synchronous = NORMAL").await?;
        }
        Engine::MySql => {
            // * **time_zone = UTC** so that a date MySQL computes itself —
            //   `UTC_TIMESTAMP()` in a translated comparison — agrees with the
            //   UTC strings this site writes. A server defaulting to local
            //   time would expire a session at the wrong hour.
            // * **STRICT_ALL_TABLES** turns a value too long for its column
            //   into an error instead of a silent truncation. A silently
            //   truncated post body is far worse than a refused write.
            // * **NO_ENGINE_SUBSTITUTION** refuses to quietly swap the storage
            //   engine, which would cost us transactions.
            conn.execute("SET time_zone = '+00:00'").await?;
            conn.execute("SET SESSION sql_mode = 'STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION'")
                .await?;
        }
    }
    Ok(())
}

/// Create the MySQL database named in the config if it is not there yet.
///
/// A host normally creates it when it provisions the site, and then this is a
/// no-op. It matters for the other case: an operator moving their own site to
/// MySQL by hand, who should not have to run `CREATE DATABASE` separately
/// when SQLite needed no such step.
async fn ensure_database(target: &Target) -> Result<()> {
    match Any::database_exists(target.url()).await {
        Ok(true) => Ok(()),
        Ok(false) => Any::create_database(target.url()).await.map_err(|e| {
            GhostError::Db(format!(
                "the database in {} does not exist and could not be created: {e}. \
                 Either create it, or grant CREATE to this account",
                target.redacted()
            ))
        }),
        // Not fatal: an account may use a database without being allowed to
        // list the server's schemas, which is the correct locked-down setup
        // for a per-site login. Let the connection itself be the test.
        Err(e) => {
            tracing::debug!("could not check whether the database exists: {e}");
            Ok(())
        }
    }
}

/// Apply the migrations compiled into the binary, for the engine in use.
///
/// Embedded with `migrate!` rather than read from disk, so a deployed binary
/// cannot be out of step with the schema it expects.
///
/// Two directories rather than one portable dialect, because the differences
/// are not cosmetic: MySQL cannot index a `TEXT` primary key without a prefix
/// length, has no `CREATE INDEX IF NOT EXISTS`, and needs `MEDIUMTEXT` where
/// SQLite's unbounded `TEXT` suffices. The two are kept in step by
/// `ghost-db/tests/schema_parity.rs`.
pub async fn migrate(pool: &Db) -> Result<()> {
    migrate_at(pool, None).await
}

/// The same, told where the SQLite file is.
///
/// The path is only used to recover from one specific failure — see
/// [`crate::repair`] — and a target without a file (MySQL, `:memory:`) passes
/// `None`.
pub async fn migrate_at(pool: &Db, path: Option<&Path>) -> Result<()> {
    let first = run_migrations(pool).await;

    // The one failure worth trying to recover from: a database created before
    // the MySQL port, whose `_sqlx_migrations` holds the checksums of
    // migration files that were later edited in place. It is not corrupt and
    // nothing is missing from it, but every route out of it is blocked while
    // sqlx refuses to run — so the server converts it and carries on rather
    // than stopping with advice. `convert_pre_port_sqlite` proves the
    // database really is that before touching it; anything else keeps the
    // error it already has.
    let Err(sqlx::migrate::MigrateError::VersionMismatch(version)) = &first else {
        return first.map_err(|e| GhostError::Db(format!("migration failed: {e}")));
    };
    if engine() != Engine::Sqlite {
        return first.map_err(|e| GhostError::Db(format!("migration failed: {e}")));
    }

    tracing::warn!(
        "migration {version} does not match the one that was applied — checking whether this \
         database predates the MySQL port"
    );

    let embedded: Vec<(i64, String, Vec<u8>)> = sqlx::migrate!("../../migrations/sqlite")
        .iter()
        .map(|m| (m.version, m.sql.to_string(), m.checksum.to_vec()))
        .collect();

    match crate::repair::convert_pre_port_sqlite(pool, path, &embedded).await {
        Ok(Some(done)) => {
            tracing::warn!(
                "converted a pre-port database to the current dialect: {} table declaration(s), \
                 {} migration checksum(s). The database as it was is at {} — remove it once the \
                 site looks right",
                done.tables,
                done.checksums,
                done.backup.display(),
            );
            // The migrations this database was behind on are applied here, by
            // sqlx, in the current dialect.
            run_migrations(pool)
                .await
                .map_err(|e| GhostError::Db(format!("migration failed after converting: {e}")))
        }
        // Not a pre-port database, so the original error is the real one.
        Ok(None) => first.map_err(|e| GhostError::Db(format!("migration failed: {e}"))),
        Err(e) => Err(e),
    }
}

/// Apply the migrations for the engine in use, reporting sqlx's own error.
async fn run_migrations(pool: &Db) -> std::result::Result<(), sqlx::migrate::MigrateError> {
    match engine() {
        Engine::Sqlite => sqlx::migrate!("../../migrations/sqlite").run(pool).await,
        Engine::MySql => sqlx::migrate!("../../migrations/mysql").run(pool).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Foreign keys are off by default in SQLite, and the schema's
    /// `REFERENCES` clauses are only constraints if the pragma is applied to
    /// every connection. This catches `after_connect` being dropped.
    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let pool = connect_in_memory().await.unwrap();
        let (on,): (i64,) = sqlx::query_as("PRAGMA foreign_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(on, 1, "foreign keys are not being enforced");
    }

    #[test]
    fn the_engine_defaults_to_sqlite() {
        assert_eq!(engine(), Engine::Sqlite);
    }
}
