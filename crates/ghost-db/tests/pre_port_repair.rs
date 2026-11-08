//! Proof that a database created before the MySQL port starts anyway.
//!
//! The port rewrote applied migrations in place — `DATETIME` to `TEXT`,
//! `BOOLEAN` to `INTEGER` — which leaves an older database unable to start
//! and, until `ghost_db::repair`, unable to be converted either: sqlx refuses
//! to run while the checksums look modified, and the offline converter only
//! touches a database that is already fully migrated. A site in both states
//! at once was stuck. These tests build exactly that database and open it.
use ghost_core::database::Target;
use sqlx::any::install_default_drivers;
use sqlx::{AnyConnection, Connection, Executor, Row};
use std::path::{Path, PathBuf};

/// sqlx's own bookkeeping table, as its SQLite migrator creates it.
const SQLX_MIGRATIONS: &str = "CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)";

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ghost-pre-port-{}", ghost_core::ids::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every migration compiled into the binary, in order.
fn embedded() -> Vec<(i64, String, Vec<u8>)> {
    sqlx::migrate!("../../migrations/sqlite")
        .iter()
        .map(|m| (m.version, m.sql.to_string(), m.checksum.to_vec()))
        .collect()
}

/// Build a database in the state the port left behind.
///
/// `through` is the last migration it had applied, so a value below the
/// newest also reproduces the half that nothing could fix: behind *and*
/// unable to migrate.
async fn make_pre_port_db(path: &Path, through: i64) {
    install_default_drivers();
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let mut conn = AnyConnection::connect(&url).await.unwrap();

    conn.execute(SQLX_MIGRATIONS).await.unwrap();
    for (version, sql, _) in embedded().into_iter().filter(|(v, _, _)| *v <= through) {
        conn.execute(sql.as_str()).await.unwrap();
        // The checksum of the file as it was *before* the port. Its value does
        // not matter, only that it is not the one in the binary now.
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (?, ?, ?, 1, ?, 0)",
        )
        .bind(version)
        .bind(format!("migration {version}"))
        // A fixed stamp rather than the clock: this fixture is SQLite-only,
        // and `datetime('now')` is the SQLite spelling the dialect layer
        // exists to keep out of this project's queries.
        .bind("2026-01-01 00:00:00")
        .bind(vec![0u8; 48])
        .execute(&mut conn)
        .await
        .unwrap();
    }

    // Put the old declared types back, the way the pre-port files wrote
    // them. Done through `sqlite_master` so the statements themselves stay
    // exactly what the migrations say.
    let rows = sqlx::query(
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();

    conn.execute("PRAGMA writable_schema=ON").await.unwrap();
    for row in rows {
        let name: String = row.try_get("name").unwrap();
        let sql: String = row.try_get("sql").unwrap();
        let old = unnormalise(&sql);
        sqlx::query("UPDATE sqlite_master SET sql = ? WHERE type = 'table' AND name = ?")
            .bind(&old)
            .bind(&name)
            .execute(&mut conn)
            .await
            .unwrap();
    }
    conn.execute("PRAGMA writable_schema=OFF").await.unwrap();
    conn.close().await.unwrap();
}

/// Turn current declarations back into pre-port ones: every `TEXT` becomes
/// `DATETIME` and every `INTEGER` becomes `BOOLEAN`.
///
/// Deliberately indiscriminate. It makes a database that is *more* wrong than
/// any real one, so a repair that copes with this copes with the real thing;
/// and because the rewrite is still types-only, the repair must accept it.
/// Quoted text is left alone, so inserted data keeps its own words.
fn unnormalise(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // As in `repair::normalise`: a comment is not a literal, and this
        // schema's comments contain apostrophes.
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            out.push_str("/*");
            i += 2;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push_str("*/");
                    i += 2;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '\'' || c == '"' || c == '`' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == close {
                    if chars.get(i + 1) == Some(&close) {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push_str(match word.to_ascii_uppercase().as_str() {
                "TEXT" => "DATETIME",
                "INTEGER" => "BOOLEAN",
                _ => &word,
            });
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The failure this all started from: a pre-port database that is also behind
/// on migrations. Nothing could start or convert it; now opening it is enough.
#[tokio::test]
async fn a_pre_port_database_that_is_behind_opens_and_finishes_migrating() {
    let dir = scratch_dir();
    let db = dir.join("ghost.db");
    let newest = embedded().last().unwrap().0;
    let behind = newest - 3;
    make_pre_port_db(&db, behind).await;

    let pool = ghost_db::pool::open(&Target::sqlite_file(&db))
        .await
        .expect("a pre-port database should be converted and opened");

    // Every migration is applied now, including the ones it was behind on.
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, embedded().len() as i64, "not every migration was applied");

    // And logged with the checksums of the files in this binary, so the next
    // start needs no repair at all.
    for (version, _, checksum) in embedded() {
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(version)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, checksum, "migration {version} still has the old checksum");
    }

    // The declared types `Any` rejects are gone from the schema itself.
    let (left,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE name <> '_sqlx_migrations' \
         AND (sql LIKE '%DATETIME%' OR sql LIKE '%BOOLEAN%')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0, "old declared types are still in the schema");

    // The point of all of it: rows read back. This is what `DATETIME` and
    // `BOOLEAN` declarations break, and it fails on the *read*, not the open.
    ghost_db::seed::seed_if_empty(&pool).await.expect("seeding a converted database");
    let rows = sqlx::query("SELECT key, value, created_at FROM settings LIMIT 5")
        .fetch_all(&pool)
        .await
        .expect("reading a converted database");
    assert!(!rows.is_empty(), "no settings to read back");
    for row in rows {
        let _: String = row.try_get("key").unwrap();
        let _: String = row.try_get("created_at").unwrap();
    }

    // The database as it was is kept, unasked.
    assert!(dir.join("ghost.db.pre-port").exists(), "no backup was written");

    std::fs::remove_dir_all(&dir).ok();
}

/// A fully migrated pre-port database is the case the offline converter
/// already handled. It has to work here too.
#[tokio::test]
async fn a_pre_port_database_that_is_current_opens() {
    let dir = scratch_dir();
    let db = dir.join("ghost.db");
    make_pre_port_db(&db, embedded().last().unwrap().0).await;

    let pool = ghost_db::pool::open(&Target::sqlite_file(&db)).await.expect("should open");
    let (left,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE name <> '_sqlx_migrations' \
         AND (sql LIKE '%DATETIME%' OR sql LIKE '%BOOLEAN%')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);

    std::fs::remove_dir_all(&dir).ok();
}

/// Opening a converted database again must be an ordinary open: no second
/// conversion, and above all no second backup overwriting the first.
#[tokio::test]
async fn converting_happens_once() {
    let dir = scratch_dir();
    let db = dir.join("ghost.db");
    make_pre_port_db(&db, embedded().last().unwrap().0).await;

    let pool = ghost_db::pool::open(&Target::sqlite_file(&db)).await.unwrap();
    pool.close().await;
    let pool = ghost_db::pool::open(&Target::sqlite_file(&db)).await.unwrap();
    pool.close().await;

    assert!(dir.join("ghost.db.pre-port").exists());
    assert!(
        !dir.join("ghost.db.pre-port1").exists(),
        "the database was converted twice"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The safety limit. A database that differs from this schema by anything
/// more than the two renames is not a pre-port copy of it, and a blind
/// rewrite would corrupt it — so it is left exactly as it is, and the
/// migration error stands.
#[tokio::test]
async fn a_structurally_different_database_is_refused() {
    let dir = scratch_dir();
    let db = dir.join("ghost.db");
    make_pre_port_db(&db, embedded().last().unwrap().0).await;

    // One column nothing in this schema has.
    install_default_drivers();
    let url = format!("sqlite://{}", db.display());
    let mut conn = AnyConnection::connect(&url).await.unwrap();
    conn.execute("ALTER TABLE posts ADD COLUMN something_else TEXT").await.unwrap();
    conn.close().await.unwrap();

    let error = ghost_db::pool::open(&Target::sqlite_file(&db))
        .await
        .expect_err("a database this unlike the schema must not be converted");
    let message = error.to_string();
    assert!(
        message.contains("previously applied but has been modified"),
        "expected sqlx's own error, got: {message}"
    );
    assert!(!dir.join("ghost.db.pre-port").exists(), "an unconvertible database was backed up");

    // Nothing was rewritten: the extra column is still there and the old
    // declared types are untouched. Compared through the schema rather than
    // the file's bytes, because opening the pool sets `journal_mode = WAL`
    // and that alone rewrites the header.
    let mut conn = AnyConnection::connect(&url).await.unwrap();
    let sql: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'posts'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert!(sql.contains("something_else"), "the extra column was dropped");
    assert!(sql.contains("DATETIME"), "the schema was converted anyway");
    let (stale,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE checksum = zeroblob(48)",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert!(stale > 0, "checksums were refreshed for a database that was refused");
    conn.close().await.unwrap();

    std::fs::remove_dir_all(&dir).ok();
}
