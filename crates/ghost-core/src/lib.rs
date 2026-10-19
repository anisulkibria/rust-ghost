//! ghost-core: config, errors, ids, slugs, dates and domain models.
pub mod config;
pub mod dates;
pub mod error;
pub mod ids;
pub mod models;
pub mod slug;

pub use config::Config;
pub use error::{GhostError, Result};
