//! ghost-core: config, errors, ids, slugs, dates and domain models.
pub mod automations;
pub mod comments;
pub mod config;
pub mod dates;
pub mod error;
pub mod ids;
pub mod metafields;
pub mod models;
pub mod slug;
pub mod url;

pub use config::Config;
pub use error::{GhostError, Result};
