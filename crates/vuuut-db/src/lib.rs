//! vuuut-db: SQLite pool, migrations, seeding and repositories.
pub mod demo;
pub mod pool;
pub mod schema_kinds;
pub mod seed;
pub mod settings_cache;
/// The content generation the page cache keys on — see `content`.
pub mod content;
pub mod repos;
pub mod limits;
pub mod mail_usage;

pub use pool::{connect, connect_in_memory, connect_test, Db, TestDb};

/// Re-exported so `vuuut_db::dialect` keeps working: the translator lives in
/// `vuuut-core` because `vuuut-nql` sits below this crate and needs it too.
pub use vuuut_core::dialect;
pub use settings_cache::SettingsCache;

pub mod transfer;
pub mod snapshot;
pub mod types;
