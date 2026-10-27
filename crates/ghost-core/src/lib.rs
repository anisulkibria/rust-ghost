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

/// The Ghost release this port tracks.
///
/// Ghost reports two forms of its own version and they are not
/// interchangeable: `@tryghost/version` exposes `full` (`6.73.0`), which is
/// what the export file's meta is stamped with, and `safe` (`6.73`), which is
/// what `/members/api/site` publishes to Portal and what a theme or an
/// integration branches on. Both live here, in the one crate every other one
/// depends on, because they were previously declared in two places and had
/// already drifted two dozen releases apart.
pub const GHOST_VERSION: &str = "6.73.0";

/// `major.minor` of [`GHOST_VERSION`] — Ghost's `version.safe`.
pub const GHOST_VERSION_SAFE: &str = "6.73";

/// Labs flags that have shipped — Ghost 6.73's `GA_FEATURES`.
///
/// `labs.getAll()` forces every one of these to `true` regardless of what is
/// stored, and drops them from the writable allowlist, so they are no longer
/// switches: a theme reading `{{#if @labs.editorReact}}` gets `true` on any
/// 6.73 site. They are listed here so the `labs` object this port publishes
/// says the same thing, and so the settings screen knows not to offer a
/// toggle for something that cannot be turned off.
pub const LABS_GA_FLAGS: &[&str] = &[
    "automationsArchive",
    "automationsPerTier",
    "selfServeArchives",
    "admin7EditorImprovements",
    "editorReact",
    "billingReact",
    "stripeCheckoutDesign",
];

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn the_safe_version_is_the_full_ones_major_minor() {
        // Ghost derives one from the other at runtime; these are two consts,
        // so this is the check that keeps a bump to one from leaving the
        // other behind.
        let parts: Vec<&str> = GHOST_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "the full version is major.minor.patch");
        assert_eq!(GHOST_VERSION_SAFE, format!("{}.{}", parts[0], parts[1]));
        assert!(GHOST_VERSION.starts_with(GHOST_VERSION_SAFE));
    }
}
