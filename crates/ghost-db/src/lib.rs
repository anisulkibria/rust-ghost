//! ghost-db: SQLite pool, migrations, seeding and repositories.
pub mod demo;
pub mod pool;
pub mod schema_kinds;
pub mod seed;
pub mod settings_cache;
pub mod repos;
pub mod limits;
pub mod mail_usage;

pub use pool::{connect, connect_in_memory, connect_test, Db, TestDb};

/// Re-exported so `ghost_db::dialect` keeps working: the translator lives in
/// `ghost-core` because `ghost-nql` sits below this crate and needs it too.
pub use ghost_core::dialect;
pub use settings_cache::SettingsCache;

pub mod transfer;
pub mod types;
