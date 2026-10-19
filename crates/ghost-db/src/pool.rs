use ghost_core::{Result, GhostError};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

/// Open the SQLite pool (WAL, foreign keys on, busy timeout) and run
/// embedded migrations. Creates the data directory if needed.
pub async fn connect(db_path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let url = format!("sqlite://{}", db_path.display());
    let opts = SqliteConnectOptions::from_str(&url)
        .map_err(|e| GhostError::ConfigError(format!("db path: {e}")))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .map_err(|e| GhostError::Db(format!("failed to open database: {e}")))?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| GhostError::Db(format!("migration failed: {e}")))?;

    Ok(pool)
}
