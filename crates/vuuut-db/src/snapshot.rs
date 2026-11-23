//! Whole-database snapshots, for backups that can be restored.
//!
//! [`crate::transfer`] moves *content* between sites and merges it into
//! whatever is there. A backup is a different promise: put this site back
//! exactly as it was. So a snapshot is every table, every row and every
//! column — credentials and operational state included — and restoring one
//! replaces each table's contents rather than merging into them.
//!
//! It is a logical dump rather than a copy of the database file because the
//! site may be on SQLite or MySQL, and because a file copy cannot be taken
//! safely while the pool is writing. Rows are read through the same `Any`
//! decoding as the content export, so both engines produce the same JSON.
//!
//! Restoring across a schema change is tolerated in one direction: a column
//! the backup has and the database no longer has is dropped, and one the
//! database has and the backup lacks gets its default. A table the backup
//! does not mention is left alone.
use crate::Db;
use vuuut_core::database::Engine;
use vuuut_core::{GhostError, Result};
use serde_json::{json, Map, Value};
use sqlx::any::AnyTypeInfoKind;
use sqlx::{Column, Row, ValueRef};

/// Tables a snapshot never carries: the migration ledger belongs to the
/// binary, not to the data.
const NEVER: &[&str] = &["_sqlx_migrations"];

/// Tables written to by triggers when *other* tables are inserted into. A
/// restore fills these last, after wiping them again, so the trigger rows the
/// restore itself provoked are not left behind — for `click_outbox` that
/// would mean re-sending every member signup in the backup.
const TRIGGER_TARGETS: &[&str] = &["click_outbox"];

fn db(e: sqlx::Error) -> GhostError {
    GhostError::Db(e.to_string())
}

fn quoted(table: &str) -> String {
    match crate::pool::engine() {
        Engine::Sqlite => format!("\"{}\"", table.replace('"', "")),
        Engine::MySql => format!("`{}`", table.replace('`', "")),
    }
}

async fn tables(conn: &mut sqlx::AnyConnection) -> Result<Vec<String>> {
    let names: Vec<String> = match crate::pool::engine() {
        Engine::Sqlite => sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?,
        Engine::MySql => sqlx::query_scalar(
            "SELECT table_name AS name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?,
    };
    Ok(names.into_iter().filter(|n| !NEVER.contains(&n.as_str())).collect())
}

async fn columns(conn: &mut sqlx::AnyConnection, table: &str) -> Result<Vec<String>> {
    Ok(match crate::pool::engine() {
        Engine::Sqlite => sqlx::query(&format!("PRAGMA table_info({})", quoted(table)))
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?
            .iter()
            .map(|r| r.get::<String, _>("name"))
            .collect(),
        Engine::MySql => sqlx::query_scalar(
            "SELECT column_name AS name FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
        )
        .bind(table)
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?,
    })
}

fn decode(row: &sqlx::any::AnyRow, i: usize) -> Result<Value> {
    let raw = row.try_get_raw(i).map_err(db)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    Ok(match raw.type_info().kind() {
        k if k.is_integer() || k == AnyTypeInfoKind::Bool => json!(row.try_get::<i64, _>(i).map_err(db)?),
        AnyTypeInfoKind::Real | AnyTypeInfoKind::Double => json!(row.try_get::<f64, _>(i).map_err(db)?),
        AnyTypeInfoKind::Blob => json!(row.try_get::<crate::types::Text, _>(i).map_err(db)?.0),
        _ => json!(row.try_get::<String, _>(i).map_err(db)?),
    })
}

/// Summary numbers for a snapshot, for the backup list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub tables: usize,
    pub rows: usize,
}

/// Read every table into one JSON document:
/// `{"engine": "...", "tables": {"posts": {"columns": [...], "rows": [[...], ...]}}}`.
///
/// Read inside one transaction so the snapshot is a single moment, not a
/// smear across whatever was written while it ran.
pub async fn dump(pool: &Db) -> Result<(Value, Counts)> {
    let mut tx = pool.begin().await.map_err(db)?;
    let names = tables(&mut tx).await?;
    let mut out = Map::new();
    let mut counts = Counts::default();
    for table in &names {
        let rows = sqlx::query(&format!("SELECT * FROM {}", quoted(table)))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        let cols: Vec<String> = match rows.first() {
            Some(r) => r.columns().iter().map(|c| c.name().to_string()).collect(),
            None => columns(&mut tx, table).await?,
        };
        let mut values = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut cells = Vec::with_capacity(cols.len());
            for i in 0..cols.len() {
                cells.push(decode(row, i)?);
            }
            values.push(Value::Array(cells));
        }
        counts.tables += 1;
        counts.rows += values.len();
        out.insert(table.clone(), json!({"columns": cols, "rows": values}));
    }
    tx.commit().await.map_err(db)?;
    let engine = match crate::pool::engine() {
        Engine::Sqlite => "sqlite",
        Engine::MySql => "mysql",
    };
    Ok((json!({"engine": engine, "tables": out}), counts))
}

fn bind<'q>(q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>, v: &Value) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    match v {
        Value::Null => q.bind(None::<String>),
        Value::Bool(b) => q.bind(*b as i64),
        Value::Number(n) => match n.as_i64() {
            Some(i) => q.bind(i),
            None => q.bind(n.as_f64().unwrap_or_default()),
        },
        Value::String(s) => q.bind(s.clone()),
        other => q.bind(other.to_string()),
    }
}

/// Replace the database's contents with a snapshot's.
///
/// One transaction: either every table is restored or none is. Tables in
/// `keep` are left as they are (the caller keeps `sessions`, so whoever is
/// restoring stays signed in). Foreign keys are checked at commit rather
/// than row by row, since tables are refilled in name order, not dependency
/// order.
pub async fn restore(pool: &Db, snapshot: &Value, keep: &[&str]) -> Result<Counts> {
    let source = snapshot
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(|| GhostError::BadRequest("This backup holds no database snapshot.".into()))?;

    let mut conn = pool.acquire().await.map_err(db)?;
    if crate::pool::engine() == Engine::MySql {
        sqlx::query("SET FOREIGN_KEY_CHECKS = 0").execute(&mut *conn).await.map_err(db)?;
    }
    let result = restore_on(&mut conn, source, keep).await;
    if crate::pool::engine() == Engine::MySql {
        let _ = sqlx::query("SET FOREIGN_KEY_CHECKS = 1").execute(&mut *conn).await;
    }
    result
}

async fn restore_on(conn: &mut sqlx::AnyConnection, source: &Map<String, Value>, keep: &[&str]) -> Result<Counts> {
    use sqlx::Connection;
    let mut tx = conn.begin().await.map_err(db)?;
    if crate::pool::engine() == Engine::Sqlite {
        sqlx::query("PRAGMA defer_foreign_keys = ON").execute(&mut *tx).await.map_err(db)?;
    }
    let mut present = tables(&mut tx).await?;
    present.retain(|t| source.contains_key(t) && !keep.contains(&t.as_str()));
    // trigger targets last
    present.sort_by_key(|t| TRIGGER_TARGETS.contains(&t.as_str()));

    // Empty everything first, so no restored row trips over a stale one.
    for table in &present {
        sqlx::query(&format!("DELETE FROM {}", quoted(table))).execute(&mut *tx).await.map_err(db)?;
    }
    let mut counts = Counts::default();
    for table in &present {
        if TRIGGER_TARGETS.contains(&table.as_str()) {
            sqlx::query(&format!("DELETE FROM {}", quoted(table))).execute(&mut *tx).await.map_err(db)?;
        }
        let spec = &source[table];
        let cols: Vec<&str> = spec["columns"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let live = columns(&mut tx, table).await?;
        // positions in the backup row of the columns the table still has
        let keep_cols: Vec<(usize, &str)> =
            cols.iter().enumerate().filter(|(_, c)| live.iter().any(|l| l == *c)).map(|(i, c)| (i, *c)).collect();
        counts.tables += 1;
        let rows = spec["rows"].as_array().cloned().unwrap_or_default();
        if keep_cols.is_empty() || rows.is_empty() {
            continue;
        }
        let col_list = keep_cols.iter().map(|(_, c)| quoted(c)).collect::<Vec<_>>().join(", ");
        let one = format!("({})", vec!["?"; keep_cols.len()].join(", "));
        // well under SQLite's oldest bound-parameter limit of 999
        let per_batch = (900 / keep_cols.len()).max(1);
        for chunk in rows.chunks(per_batch) {
            let sql = format!(
                "INSERT INTO {} ({col_list}) VALUES {}",
                quoted(table),
                vec![one.as_str(); chunk.len()].join(", ")
            );
            let mut q = sqlx::query(&sql);
            for row in chunk {
                let cells = row.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                for (i, _) in &keep_cols {
                    q = bind(q, cells.get(*i).unwrap_or(&Value::Null));
                }
            }
            q.execute(&mut *tx).await.map_err(|e| GhostError::Db(format!("restoring {table}: {e}")))?;
            counts.rows += chunk.len();
        }
    }
    tx.commit().await.map_err(db)?;
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_snapshot_puts_the_database_back() {
        let test = crate::pool::connect_test().await.unwrap();
        let pool = test.pool();
        crate::seed::seed_if_empty(&pool).await.unwrap();
        sqlx::query("INSERT INTO tags (id, name, slug, created_at, updated_at) VALUES ('t1', 'Before', 'before', '2026-01-01 00:00:00', '2026-01-01 00:00:00')")
            .execute(&pool)
            .await
            .unwrap();
        let tags_then: Vec<String> = sqlx::query_scalar("SELECT name FROM tags ORDER BY name").fetch_all(&pool).await.unwrap();
        assert!(tags_then.contains(&"Before".to_string()));
        let (snap, counts) = dump(&pool).await.unwrap();
        assert!(counts.tables > 10 && counts.rows > 0);

        sqlx::query("DELETE FROM tags").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tags (id, name, slug, created_at, updated_at) VALUES ('t2', 'After', 'after', '2026-01-01 00:00:00', '2026-01-01 00:00:00')")
            .execute(&pool)
            .await
            .unwrap();
        let settings_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings").fetch_one(&pool).await.unwrap();

        // round-trip through text, as a backup file does
        let snap: Value = serde_json::from_str(&snap.to_string()).unwrap();
        restore(&pool, &snap, &["sessions"]).await.unwrap();

        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM tags ORDER BY name").fetch_all(&pool).await.unwrap();
        assert_eq!(names, tags_then);
        let settings_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings").fetch_one(&pool).await.unwrap();
        assert_eq!(settings_before, settings_after);
    }
}
