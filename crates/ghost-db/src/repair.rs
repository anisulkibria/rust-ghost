//! Bringing a pre-port SQLite database up to the current dialect, in place.
//!
//! The MySQL port rewrote the SQLite migrations rather than adding new ones:
//! `DATETIME` became `TEXT` and `BOOLEAN` became `INTEGER`, because sqlx's
//! `Any` driver accepts only Null/Integer/Float/Blob/Text from SQLite and
//! rejects the other two outright. Editing applied migrations leaves an older
//! database inconsistent with the binary in two ways at once:
//!
//!   1. `_sqlx_migrations` holds the checksums of the old files, so sqlx
//!      refuses to start — `MigrateError::VersionMismatch`; and
//!   2. its tables still *declare* `DATETIME`/`BOOLEAN`, which `Any` rejects
//!      when a row is read back.
//!
//! Both are fixed here, automatically, the first time such a database is
//! opened. Nothing else would do: the two conditions deadlock every manual
//! route out. sqlx will not apply a pending migration while the checksums
//! look modified, and `tools/convert-sqlite-db.py` — which predates this
//! module and is still the way to convert a database the server is not
//! running — only touches a database that is already fully migrated. A site
//! whose database was both pre-port *and* behind could not be started or
//! converted at all.
//!
//! Not a value is touched, because none was ever wrong: a date was already
//! stored as `'YYYY-MM-DD HH:MM:SS'` text (`DATETIME` carries NUMERIC
//! affinity, and that string is not convertible to a number, so SQLite stored
//! it as text) and a boolean as integer 0/1. Only the declarations disagreed.
//!
//! # Why this is allowed to rewrite `sqlite_master`
//!
//! Because it proves first that the edit is type-tokens-only. It rebuilds the
//! schema the *applied* migrations should have produced, in a scratch
//! database, and refuses to touch anything unless the live schema is
//! identical to that reference once the two renames and a decorative
//! `TEXT(n)` width are normalised away. Any other difference — a missing
//! table, an extra index, a changed constraint — means the database is not
//! merely an old-dialect copy of this schema, and a blind rewrite would
//! corrupt it. In that case the original sqlx error stands.
//!
//! Building the reference from the applied subset rather than from every
//! migration in the binary is what breaks the deadlock: a database that is
//! also behind is converted on the spot, and sqlx then applies the migrations
//! it is missing in the current dialect, by itself.
use crate::pool::Db;
use ghost_core::{GhostError, Result};
use sqlx::{Acquire, Connection, Executor, Row};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The only declared-type rewrites the port made. Applied to both sides
/// before comparing, so a difference that survives is a real one.
const TYPE_ALIASES: [(&str, &str); 2] = [("DATETIME", "TEXT"), ("BOOLEAN", "INTEGER")];

/// One schema object, keyed the way `sqlite_master` lists it.
type Schema = BTreeMap<(String, String), String>;

/// What a repair changed, for the log line that reports it.
pub(crate) struct Repaired {
    pub tables: usize,
    pub checksums: usize,
    pub backup: PathBuf,
}

/// Strip away the differences this module is allowed to introduce.
///
/// That is: the two declared-type renames, and the decorative length on a
/// `TEXT(n)` column — SQLite ignores it entirely (it only matters to the
/// generated MySQL schema, where it becomes a `VARCHAR` width), so a change
/// to it cannot affect stored data. Whitespace is collapsed because the
/// reference is parsed from the migration files and the live copy from
/// SQLite's own round-trip of them.
///
/// Quoted text is passed through untouched, so a type name that appears
/// inside a string literal or a quoted identifier is *not* normalised, and a
/// difference there is reported as the real difference it is.
fn normalise(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut last_was_space = true; // leading whitespace is trimmed

    while i < chars.len() {
        let c = chars[i];

        // Comments are copied verbatim. They have to be recognised before
        // quotes, because this schema's comments contain apostrophes
        // ("Ghost's built-in welcome emails…") and treating one as the start
        // of a string literal would swallow the declarations after it — which
        // is exactly how a convertible database came to look unconvertible.
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            last_was_space = false;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            out.push('/');
            out.push('*');
            i += 2;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push('*');
                    out.push('/');
                    i += 2;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            last_was_space = false;
            continue;
        }

        // Quoted runs are copied verbatim, including the quotes.
        if c == '\'' || c == '"' || c == '`' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                // '' inside a '…' literal is an escaped quote, not the end.
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
            last_was_space = false;
            continue;
        }

        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            i += 1;
            continue;
        }

        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_ascii_uppercase();
            let mapped = TYPE_ALIASES
                .iter()
                .find(|(from, _)| *from == upper)
                .map(|(_, to)| *to)
                .unwrap_or(&word);
            out.push_str(mapped);

            // TEXT(n) -> TEXT. Only the width is dropped; TEXT(n) and TEXT
            // are the same declaration to SQLite.
            if mapped.eq_ignore_ascii_case("TEXT") {
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if chars.get(j) == Some(&'(') {
                    let mut k = j + 1;
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > j + 1 && chars.get(k) == Some(&')') {
                        i = k + 1;
                    }
                }
            }
            last_was_space = false;
            continue;
        }

        out.push(c);
        last_was_space = false;
        i += 1;
    }

    out.trim_end().to_string()
}

/// Every object a migration created, as `sqlite_master` lists it.
///
/// `_sqlx_migrations` is excluded: sqlx creates it itself, in whatever
/// dialect the running version writes, and it is not part of this schema.
async fn schema_of<'c, E>(conn: E) -> Result<Schema>
where
    E: Executor<'c, Database = sqlx::Any>,
{
    let rows = sqlx::query(
        "SELECT type, name, COALESCE(sql, '') AS sql FROM sqlite_master \
         WHERE name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(conn)
    .await
    .map_err(|e| GhostError::Db(format!("could not read the schema: {e}")))?;

    let mut schema = Schema::new();
    for row in rows {
        let kind: String = row.try_get("type").map_err(decode_err)?;
        let name: String = row.try_get("name").map_err(decode_err)?;
        let sql: String = row.try_get("sql").map_err(decode_err)?;
        schema.insert((kind, name), sql);
    }
    Ok(schema)
}

fn decode_err(e: sqlx::Error) -> GhostError {
    GhostError::Db(format!("could not read the schema: {e}"))
}

/// The schema the given migrations produce, built from scratch.
///
/// A private in-memory database, so this cannot disturb the live one.
async fn reference_schema(migrations: &[(i64, String)]) -> Result<Schema> {
    let mut conn = sqlx::AnyConnection::connect("sqlite::memory:")
        .await
        .map_err(|e| GhostError::Db(format!("could not open a scratch database: {e}")))?;

    for (version, sql) in migrations {
        conn.execute(sql.as_str()).await.map_err(|e| {
            GhostError::Db(format!("migration {version} does not apply to an empty database: {e}"))
        })?;
    }
    let schema = schema_of(&mut conn).await?;
    let _ = conn.close().await;
    Ok(schema)
}

/// Convert a pre-port database in place, or explain why it was left alone.
///
/// `Ok(None)` means the database is not a pre-port copy of this schema, and
/// the caller should report the migration error it already has.
pub(crate) async fn convert_pre_port_sqlite(
    pool: &Db,
    path: Option<&Path>,
    embedded: &[(i64, String, Vec<u8>)],
) -> Result<Option<Repaired>> {
    // A rewrite of `sqlite_master` is not something to do without a copy to
    // fall back on, and a copy needs a file. An in-memory database is created
    // by this binary and so is never pre-port.
    let Some(path) = path else {
        return Ok(None);
    };

    // One connection for the whole operation: `PRAGMA writable_schema` is
    // per-connection, so a pooled call that landed elsewhere would run
    // against a connection that still has it off.
    let mut handle = pool
        .acquire()
        .await
        .map_err(|e| GhostError::Db(format!("could not acquire a connection: {e}")))?;
    let conn = handle.acquire().await.map_err(|e| {
        GhostError::Db(format!("could not acquire a connection: {e}"))
    })?;

    // --- what the database says it has applied ---

    let applied_rows = sqlx::query("SELECT version, checksum FROM _sqlx_migrations")
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| GhostError::Db(format!("could not read _sqlx_migrations: {e}")))?;

    let mut applied: BTreeMap<i64, Vec<u8>> = BTreeMap::new();
    for row in applied_rows {
        let version: i64 = row.try_get("version").map_err(decode_err)?;
        let checksum: Vec<u8> = row.try_get("checksum").map_err(decode_err)?;
        applied.insert(version, checksum);
    }
    if applied.is_empty() {
        return Ok(None);
    }

    // A database that has applied something this binary does not have is not
    // an old copy of this schema — it is a newer one, or another project's.
    if applied.keys().any(|v| !embedded.iter().any(|(ev, _, _)| ev == v)) {
        return Ok(None);
    }

    // --- prove it is an old-dialect copy of exactly the applied subset ---

    let subset: Vec<(i64, String)> = embedded
        .iter()
        .filter(|(v, _, _)| applied.contains_key(v))
        .map(|(v, sql, _)| (*v, sql.clone()))
        .collect();

    let reference = reference_schema(&subset).await?;
    let current = schema_of(&mut *conn).await?;

    let missing: Vec<_> = reference.keys().filter(|k| !current.contains_key(*k)).collect();
    let unexpected: Vec<_> = current.keys().filter(|k| !reference.contains_key(*k)).collect();
    if !missing.is_empty() || !unexpected.is_empty() {
        tracing::debug!(
            "not a pre-port database: {} object(s) missing, {} unexpected",
            missing.len(),
            unexpected.len()
        );
        return Ok(None);
    }

    for (key, want) in &reference {
        if normalise(want) != normalise(&current[key]) {
            tracing::debug!(
                "not a pre-port database: {} {} differs by more than its declared types",
                key.0,
                key.1
            );
            return Ok(None);
        }
    }

    // Only a table definition is convertible. An index or trigger that
    // differs beyond its declared types was caught above; one that differs
    // *in* them should not exist, and is not something to rewrite blind.
    let stale: Vec<(String, String)> = reference
        .iter()
        .filter(|(key, want)| *want != &current[*key])
        .map(|(key, _)| key.clone())
        .collect();
    if let Some((kind, name)) = stale.iter().find(|(kind, _)| kind != "table") {
        tracing::debug!("not converting: {kind} {name} is not a table");
        return Ok(None);
    }

    let rechecksum: Vec<(i64, Vec<u8>)> = embedded
        .iter()
        .filter_map(|(version, _, checksum)| {
            let have = applied.get(version)?;
            (have != checksum).then(|| (*version, checksum.clone()))
        })
        .collect();

    if stale.is_empty() && rechecksum.is_empty() {
        return Ok(None);
    }

    // --- a copy to fall back on ---

    // `VACUUM INTO` rather than a file copy: the pool runs in WAL mode, so
    // the committed state is spread across the database and its write-ahead
    // log, and copying the one file alone can miss it.
    let backup = backup_path(path);
    let quoted = backup.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{quoted}'"))
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            GhostError::Db(format!(
                "refusing to convert {} without a backup: VACUUM INTO {} failed: {e}",
                path.display(),
                backup.display()
            ))
        })?;

    // --- rewrite ---

    let (schema_version,): (i64,) = sqlx::query_as("PRAGMA schema_version")
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| GhostError::Db(format!("could not read the schema version: {e}")))?;

    conn.execute("PRAGMA writable_schema=ON")
        .await
        .map_err(|e| GhostError::Db(format!("could not enable schema writes: {e}")))?;

    let outcome = rewrite(&mut *conn, &reference, &stale, &rechecksum, schema_version).await;

    // Left on, this connection could corrupt the schema on any later
    // statement, so it goes off whether the rewrite worked or not.
    let off = conn.execute("PRAGMA writable_schema=OFF").await;
    outcome?;
    off.map_err(|e| GhostError::Db(format!("could not disable schema writes: {e}")))?;

    // --- verify, having made SQLite reparse the schema ---

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| GhostError::Db(format!("could not run integrity_check: {e}")))?;
    if integrity != "ok" {
        return Err(GhostError::Db(format!(
            "integrity_check failed after converting {}: {integrity}. \
             The database as it was is at {}",
            path.display(),
            backup.display()
        )));
    }

    let after = schema_of(&mut *conn).await?;
    if after != reference {
        return Err(GhostError::Db(format!(
            "{} still differs from the migrations after converting it. \
             The database as it was is at {}",
            path.display(),
            backup.display()
        )));
    }

    Ok(Some(Repaired { tables: stale.len(), checksums: rechecksum.len(), backup }))
}

/// The rewrite itself, in one transaction.
async fn rewrite(
    conn: &mut sqlx::AnyConnection,
    reference: &Schema,
    stale: &[(String, String)],
    rechecksum: &[(i64, Vec<u8>)],
    schema_version: i64,
) -> Result<()> {
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(|e| GhostError::Db(format!("could not begin the conversion: {e}")))?;

    for key in stale {
        sqlx::query("UPDATE sqlite_master SET sql = ? WHERE type = 'table' AND name = ?")
            .bind(&reference[key])
            .bind(&key.1)
            .execute(&mut *tx)
            .await
            .map_err(|e| GhostError::Db(format!("could not convert table {}: {e}", key.1)))?;
    }
    for (version, checksum) in rechecksum {
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(checksum.as_slice())
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                GhostError::Db(format!("could not refresh the checksum of migration {version}: {e}"))
            })?;
    }

    tx.commit()
        .await
        .map_err(|e| GhostError::Db(format!("could not commit the conversion: {e}")))?;

    // Tell every other connection — in this process and any other — that the
    // schema moved under it, so none keeps serving from a stale parse.
    sqlx::query(&format!("PRAGMA schema_version = {}", schema_version + 1))
        .execute(&mut *conn)
        .await
        .map_err(|e| GhostError::Db(format!("could not bump the schema version: {e}")))?;
    Ok(())
}

/// Where to put the copy taken before converting.
///
/// Named for what it is, and never overwriting an existing one: a second
/// conversion of the same database would otherwise destroy the only copy of
/// the state the first one started from.
fn backup_path(db: &Path) -> PathBuf {
    let name = db.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
    let first = dir.join(format!("{name}.pre-port"));
    if !first.exists() {
        return first;
    }
    for n in 1.. {
        let candidate = dir.join(format!("{name}.pre-port{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("an unused backup name always exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_renames_are_normalised_away() {
        assert_eq!(
            normalise("created_at DATETIME NOT NULL"),
            normalise("created_at TEXT NOT NULL")
        );
        assert_eq!(normalise("sent BOOLEAN DEFAULT 0"), normalise("sent INTEGER DEFAULT 0"));
    }

    #[test]
    fn a_decorative_text_width_is_normalised_away() {
        assert_eq!(normalise("slug TEXT(191) NOT NULL"), normalise("slug TEXT NOT NULL"));
        assert_eq!(normalise("a TEXT (16),b TEXT"), normalise("a TEXT,b TEXT"));
    }

    #[test]
    fn whitespace_is_not_a_difference() {
        assert_eq!(
            normalise("CREATE TABLE a (\n  id TEXT\n)"),
            normalise("CREATE TABLE a ( id TEXT )")
        );
    }

    /// The whole safety argument rests on this: a difference that is not one
    /// of the two renames has to survive normalisation.
    #[test]
    fn a_real_difference_survives() {
        assert_ne!(normalise("id TEXT NOT NULL"), normalise("id TEXT"));
        assert_ne!(normalise("a TEXT, b TEXT"), normalise("a TEXT"));
        assert_ne!(normalise("x INTEGER"), normalise("x REAL"));
        // A prefix of a type name is not that type name.
        assert_ne!(normalise("x DATETIMEZ"), normalise("x TEXTZ"));
        assert_ne!(normalise("x MYBOOLEAN"), normalise("x MYINTEGER"));
    }

    /// The bug that made a convertible database look unconvertible: an
    /// apostrophe inside a `--` comment is not the start of a string
    /// literal, and the declarations after it still have to be normalised.
    #[test]
    fn an_apostrophe_in_a_comment_does_not_hide_the_columns_after_it() {
        let old = "CREATE TABLE a (\n  -- Ghost's own rows claim these\n                     created_at DATETIME NOT NULL,\n  updated_at DATETIME\n)";
        let new = "CREATE TABLE a (\n  -- Ghost's own rows claim these\n                     created_at TEXT NOT NULL,\n  updated_at TEXT\n)";
        assert_eq!(normalise(old), normalise(new));
        // And a real difference after the comment is still a difference.
        let other = "CREATE TABLE a (\n  -- Ghost's own rows claim these\n                       created_at TEXT,\n  updated_at TEXT\n)";
        assert_ne!(normalise(old), normalise(other));
    }

    /// The same for a block comment.
    #[test]
    fn a_block_comment_is_not_a_literal_either() {
        assert_eq!(
            normalise("a DATETIME, /* it's fine */ b DATETIME"),
            normalise("a TEXT, /* it's fine */ b TEXT")
        );
    }

    /// A type name inside quoted text is data, not a declaration.
    #[test]
    fn quoted_text_is_left_alone() {
        assert_ne!(normalise("x TEXT DEFAULT 'DATETIME'"), normalise("x TEXT DEFAULT 'TEXT'"));
        assert_eq!(
            normalise("x TEXT DEFAULT 'DATETIME'"),
            normalise("x   TEXT   DEFAULT 'DATETIME'")
        );
        assert_ne!(normalise("\"BOOLEAN\" TEXT"), normalise("\"INTEGER\" TEXT"));
        // An escaped quote does not end the literal.
        assert_eq!(normalise("x TEXT DEFAULT 'it''s DATETIME'"), "x TEXT DEFAULT 'it''s DATETIME'");
    }

    #[test]
    fn a_backup_never_overwrites_one() {
        let dir = std::env::temp_dir().join(format!("ghost-repair-{}", ghost_core::ids::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("ghost.db");
        assert_eq!(backup_path(&db), dir.join("ghost.db.pre-port"));
        std::fs::write(dir.join("ghost.db.pre-port"), b"x").unwrap();
        assert_eq!(backup_path(&db), dir.join("ghost.db.pre-port1"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
