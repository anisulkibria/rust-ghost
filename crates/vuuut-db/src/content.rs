//! The site's content generation: a number that goes up whenever something
//! that changes a rendered page is written — a post, a tag, a setting.
//!
//! The page cache stores each page with the generation it was rendered in
//! and never serves one from an older generation. Bumping here, in the
//! repository layer, rather than in each admin handler means every path that
//! writes — the editor, the Admin API, the scheduler, an import — empties
//! the cache without having to remember to.
//!
//! Process-wide: one site per process in production. In a test binary with
//! several fixtures a bump costs the others a cache miss, nothing more.

use std::sync::atomic::{AtomicU64, Ordering};

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// Something that changes pages was written.
pub fn touch() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}
