//! Shipping operational rows to Tinybird, incrementally.
//!
//! Ported from Ghost's `services/tinybird-sync`. The port already had SQL
//! analytics and a read-token endpoint; what it did not have was the pipeline
//! that *puts* data in the warehouse, which is what made the analytics
//! upstream reads from unavailable here.
//!
//! Four details carry the correctness, and all four are upstream's:
//!
//! * **A watermark per table**, stored in `tinybird_syncs`, so a run resumes
//!   where the last one stopped instead of re-reading the table.
//! * **A compound cursor** of `(updated_at, id)`. `updated_at` alone is not
//!   unique, and a watermark on a non-unique column either skips rows written
//!   in the same second or resends them forever.
//! * **A safety lag.** A row's `updated_at` is set before its transaction
//!   commits, so a row can become visible *after* rows with later timestamps.
//!   Holding back the last minute stops the watermark moving past a row whose
//!   transaction was still open — the one bug in this shape that silently
//!   loses data rather than duplicating it.
//! * **Chunking by bytes and by count**, because the receiving endpoint
//!   bounds both.
//!
//! Delivery is at-least-once by construction: the watermark advances only
//! after a batch is accepted, so a crash mid-run resends that batch rather
//! than dropping it. The warehouse de-duplicates on `(type, id, updated_at)`.
use crate::AppState;
use ghost_core::{dates, ids};
use serde_json::{json, Value};

/// A table to ship, and the columns the warehouse reads from it.
///
/// Deliberately narrow, as upstream is: member identity and lock bookkeeping
/// stay in the database. Only what the materialized views actually read goes
/// over the wire.
pub struct Target {
    pub table: &'static str,
    pub columns: &'static [&'static str],
}

/// Upstream's `AUTOMATION_SYNC_TARGETS`.
pub const TARGETS: &[Target] = &[
    Target { table: "automation_runs", columns: &["id", "automation_id", "created_at", "updated_at"] },
    Target {
        table: "automation_run_steps",
        columns: &[
            "id",
            "automation_run_id",
            "automation_action_revision_id",
            "status",
            "step_attempts",
            "ready_at",
            "started_at",
            "finished_at",
            "created_at",
            "updated_at",
        ],
    },
];

/// How far back from now the cutoff sits. Upstream's `SAFETY_LAG_MS`.
pub const SAFETY_LAG_SECS: i64 = 60;
/// Rows read per query.
const BATCH_SIZE: i64 = 1000;
/// Upstream's payload bounds for one POST.
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_PAYLOAD_MESSAGES: usize = 1000;

/// Where to ship to, and as whom.
#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: String,
    pub token: String,
    pub site_uuid: String,
}

impl Config {
    /// Read the ingestion configuration, or `None` when this install has none.
    ///
    /// Absent is the normal state for a self-hosted site: there is no
    /// warehouse to ship to, and the native SQL analytics are what the admin
    /// reads. The sync simply does not run.
    pub fn from_state(state: &AppState) -> Option<Config> {
        let tinybird = &state.config.host_settings["analytics"]["tinybird"];
        let endpoint = tinybird["ingestEndpoint"].as_str()?;
        // The ingestion credential, which is not the read token the admin
        // gets — shipping data needs a writer and reading needs a reader, and
        // handing the browser a writer would be the whole warehouse.
        let token = tinybird["ingestToken"].as_str()?;
        if endpoint.trim().is_empty() || token.trim().is_empty() {
            return None;
        }
        let site_uuid = tinybird["siteUuid"]
            .as_str()
            .map(str::to_string)
            .or_else(|| state.settings.get_str("site_uuid"))
            .unwrap_or_default();
        if site_uuid.is_empty() {
            return None;
        }
        Some(Config {
            endpoint: endpoint.to_string(),
            token: token.to_string(),
            site_uuid,
        })
    }
}

/// Where a table's sync has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub updated_at: String,
    pub id: String,
}

/// One row, as the warehouse expects it.
///
/// The envelope — `type`, `id`, `updated_at`, `site_uuid` — is what the
/// warehouse de-duplicates and partitions on; the row itself rides in
/// `payload`.
pub fn event_line(table: &str, row: &Value, site_uuid: &str) -> String {
    let mut payload = row.as_object().cloned().unwrap_or_default();
    payload.insert("site_uuid".into(), json!(site_uuid));
    json!({
        "type": table,
        "site_uuid": site_uuid,
        "id": row.get("id").cloned().unwrap_or(Value::Null),
        "updated_at": row.get("updated_at").cloned().unwrap_or(Value::Null),
        "payload": payload,
    })
    .to_string()
}

/// Split lines into POSTs that respect both the byte and the message bound.
pub fn chunk(lines: Vec<String>, max_bytes: usize, max_messages: usize) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    for line in lines {
        let size = line.len() + 1;
        if !current.is_empty() && (bytes + size > max_bytes || current.len() >= max_messages) {
            out.push(std::mem::take(&mut current));
            bytes = 0;
        }
        bytes += size;
        current.push(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The SQL for one batch: everything after the cursor and before the cutoff.
///
/// The tie-breaker is the point of it — `updated_at > cursor OR (updated_at =
/// cursor AND id > cursor_id)` is what makes two rows sharing a timestamp
/// both get sent, exactly once each.
fn batch_sql(target: &Target, has_cursor: bool) -> String {
    let columns = target
        .columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let after = if has_cursor {
        "AND (updated_at > ?2 OR (updated_at = ?2 AND id > ?3)) "
    } else {
        ""
    };
    format!(
        "SELECT {columns} FROM \"{}\" WHERE updated_at IS NOT NULL AND updated_at < ?1 {after}\
         ORDER BY updated_at, id LIMIT {BATCH_SIZE}",
        target.table
    )
}

async fn read_watermark(pool: &sqlx::SqlitePool, table: &str) -> Option<Cursor> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT last_synced_updated_at, last_synced_id FROM tinybird_syncs WHERE table_name = ?",
    )
    .bind(table)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.map(|(updated_at, id)| Cursor { updated_at, id })
}

async fn write_watermark(pool: &sqlx::SqlitePool, table: &str, cursor: &Cursor) {
    let now = dates::now_db();
    let _ = sqlx::query(
        "INSERT INTO tinybird_syncs (id, table_name, last_synced_updated_at, last_synced_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(table_name) DO UPDATE SET \
           last_synced_updated_at = excluded.last_synced_updated_at, \
           last_synced_id = excluded.last_synced_id, \
           updated_at = excluded.updated_at",
    )
    .bind(ids::new_id())
    .bind(table)
    .bind(&cursor.updated_at)
    .bind(&cursor.id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await;
}

/// POST one chunk as newline-delimited JSON.
async fn post(config: &Config, lines: &[String]) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(&config.endpoint)
        .header("Authorization", format!("Bearer {}", config.token))
        .header("Content-Type", "application/x-ndjson")
        .header("User-Agent", "Ghost-rust")
        .header("x-site-uuid", &config.site_uuid)
        .body(lines.join("\n"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        // The status, never the body: an ingestion endpoint can echo the
        // credential back in an error.
        Err(format!("Traffic Analytics returned {}", response.status().as_u16()))
    }
}

/// Ship one table, resuming from its watermark. Returns rows sent.
///
/// Takes the pool rather than the whole application state: the sync reads
/// rows and writes a watermark, and nothing else about the site concerns it.
pub async fn sync_table(
    pool: &sqlx::SqlitePool,
    config: &Config,
    target: &Target,
) -> Result<u64, String> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(SAFETY_LAG_SECS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let mut cursor = read_watermark(pool, target.table).await;
    let mut sent = 0u64;

    loop {
        let sql = batch_sql(target, cursor.is_some());
        let mut query = sqlx::query(&sql).bind(&cutoff);
        if let Some(cursor) = &cursor {
            query = query.bind(&cursor.updated_at).bind(&cursor.id);
        }
        let rows = query.fetch_all(pool).await.map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return Ok(sent);
        }
        let count = rows.len();

        let mut lines = Vec::with_capacity(count);
        let mut last: Option<Cursor> = None;
        for row in &rows {
            let value = row_to_json(row);
            last = Some(Cursor {
                updated_at: value["updated_at"].as_str().unwrap_or_default().to_string(),
                id: value["id"].as_str().unwrap_or_default().to_string(),
            });
            lines.push(event_line(target.table, &value, &config.site_uuid));
        }

        for chunk in chunk(lines, MAX_PAYLOAD_BYTES, MAX_PAYLOAD_MESSAGES) {
            // The watermark is not advanced past a batch that was refused, so
            // the next run resends it. At-least-once, never at-most-once.
            post(config, &chunk).await?;
        }
        if let Some(last) = last {
            write_watermark(pool, target.table, &last).await;
            cursor = Some(last);
        }
        sent += count as u64;
        if (count as i64) < BATCH_SIZE {
            return Ok(sent);
        }
    }
}

/// Turn a row into JSON, which is what the event envelope is built from.
fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    use sqlx::{Column, Row, TypeInfo, ValueRef};
    let mut out = serde_json::Map::new();
    for column in row.columns() {
        let raw = match row.try_get_raw(column.ordinal()) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" | "BOOLEAN" => row
                    .try_get::<i64, _>(column.ordinal())
                    .map(|v| json!(v))
                    .unwrap_or(Value::Null),
                "REAL" => row
                    .try_get::<f64, _>(column.ordinal())
                    .map(|v| json!(v))
                    .unwrap_or(Value::Null),
                _ => row
                    .try_get::<String, _>(column.ordinal())
                    .map(|v| json!(v))
                    .unwrap_or(Value::Null),
            }
        };
        out.insert(column.name().to_string(), value);
    }
    Value::Object(out)
}

/// Ship every configured target once.
pub async fn run_once(state: &AppState) -> Result<u64, String> {
    let Some(config) = Config::from_state(state) else {
        return Ok(0);
    };
    let mut total = 0;
    for target in TARGETS {
        total += sync_table(&state.db, &config, target).await?;
    }
    Ok(total)
}

/// Upstream runs this as a scheduled job; here it is a loop beside the other
/// background workers. Does nothing at all when no warehouse is configured,
/// which is the normal self-hosted case.
pub fn spawn(state: AppState) {
    if Config::from_state(&state).is_none() {
        return;
    }
    tokio::spawn(async move {
        loop {
            match run_once(&state).await {
                Ok(0) => {}
                Ok(sent) => tracing::info!("Tinybird sync: shipped {sent} row(s)"),
                Err(e) => tracing::warn!("Tinybird sync: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_targets_match_upstreams() {
        // Transcribed from `AUTOMATION_SYNC_TARGETS`; shipping a column the
        // warehouse does not read is data leaving the site for no reason.
        assert_eq!(TARGETS.len(), 2);
        assert_eq!(TARGETS[0].table, "automation_runs");
        assert_eq!(TARGETS[0].columns, &["id", "automation_id", "created_at", "updated_at"]);
        assert_eq!(TARGETS[1].table, "automation_run_steps");
        assert_eq!(TARGETS[1].columns.len(), 10);
        // Member identity stays here.
        assert!(TARGETS.iter().all(|t| !t.columns.contains(&"member_id")));
        assert_eq!(SAFETY_LAG_SECS, 60);
    }

    #[test]
    fn an_event_line_carries_the_envelope_the_warehouse_keys_on() {
        let row = json!({ "id": "r1", "automation_id": "a1", "updated_at": "2026-10-31 10:00:00" });
        let line = event_line("automation_runs", &row, "site-uuid");
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["type"], json!("automation_runs"));
        assert_eq!(parsed["id"], json!("r1"));
        assert_eq!(parsed["updated_at"], json!("2026-10-31 10:00:00"));
        assert_eq!(parsed["site_uuid"], json!("site-uuid"));
        // The row rides in payload, with the site stamped on it too.
        assert_eq!(parsed["payload"]["automation_id"], json!("a1"));
        assert_eq!(parsed["payload"]["site_uuid"], json!("site-uuid"));
        // One line, so a batch can be newline-joined.
        assert!(!line.contains('\n'));
    }

    #[test]
    fn chunking_respects_the_message_bound() {
        let lines: Vec<String> = (0..2500).map(|i| format!("{{\"n\":{i}}}")).collect();
        let chunks = chunk(lines, 10 * 1024 * 1024, 1000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1000);
        assert_eq!(chunks[2].len(), 500);
    }

    #[test]
    fn chunking_respects_the_byte_bound() {
        let lines: Vec<String> = (0..10).map(|_| "x".repeat(100)).collect();
        // Room for two lines per chunk (101 bytes each with the newline).
        let chunks = chunk(lines, 250, 1000);
        assert!(chunks.iter().all(|c| c.len() <= 2), "{:?}", chunks.iter().map(Vec::len).collect::<Vec<_>>());
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 10, "no line dropped");
    }

    #[test]
    fn a_single_oversized_line_is_still_sent_rather_than_dropped() {
        let chunks = chunk(vec!["x".repeat(5000)], 100, 1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1);
    }

    #[test]
    fn chunking_nothing_produces_no_requests() {
        assert!(chunk(Vec::new(), 100, 10).is_empty());
    }

    #[test]
    fn the_batch_query_breaks_ties_on_id() {
        // Without the tie-break, two rows sharing an `updated_at` either both
        // resend forever or one is skipped — the difference between
        // at-least-once and silently losing a row.
        let sql = batch_sql(&TARGETS[0], true);
        assert!(sql.contains("updated_at > ?2 OR (updated_at = ?2 AND id > ?3)"), "{sql}");
        assert!(sql.contains("ORDER BY updated_at, id"), "{sql}");
        // And the cutoff is always applied, cursor or not.
        assert!(sql.contains("updated_at < ?1"));
        assert!(batch_sql(&TARGETS[0], false).contains("updated_at < ?1"));
    }

    #[test]
    fn the_first_batch_has_no_cursor_predicate_to_bind() {
        let sql = batch_sql(&TARGETS[0], false);
        assert!(!sql.contains("?2"), "{sql}");
        assert!(!sql.contains("?3"), "{sql}");
    }

    #[test]
    fn only_the_named_columns_are_selected() {
        let sql = batch_sql(&TARGETS[1], false);
        for column in TARGETS[1].columns {
            assert!(sql.contains(&format!("\"{column}\"")), "{column} missing from {sql}");
        }
        assert!(!sql.contains("member_id"));
    }

    #[test]
    fn rows_with_no_updated_at_are_not_shipped() {
        // They have no position in the cursor's ordering, so including them
        // would make the watermark meaningless.
        assert!(batch_sql(&TARGETS[0], false).contains("updated_at IS NOT NULL"));
    }
}
