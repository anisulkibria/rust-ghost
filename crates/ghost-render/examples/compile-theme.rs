//! Compile theme directories and report what the engine rejects.
//!
//! Themes from outside the official set — ThemeForest, the Ghost marketplace,
//! a client's bespoke one — can only be vouched for by running them through
//! the real compiler, so this does exactly what activating a theme does,
//! minus the site:
//!
//! ```text
//! cargo run -p ghost-render --example compile-theme -- content/themes/*
//! ```
//!
//! Exits non-zero when any theme fails, so it also works as a check in CI.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let dirs: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        eprintln!("usage: compile-theme <theme-dir>...");
        std::process::exit(2);
    }

    // An empty in-memory database is enough: compiling a theme never runs a
    // query, it only needs the helpers to exist.
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.expect("in-memory sqlite");

    let mut failed = 0;
    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match compile(&pool, dir) {
            Ok(()) => println!("{name:<16} ok"),
            Err(e) => {
                failed += 1;
                println!("{name:<16} FAILED\n{}\n", indent(&e));
            }
        }
    }
    if failed > 0 {
        eprintln!("{failed} of {} theme(s) failed to compile", dirs.len());
        std::process::exit(1);
    }
}

fn compile(pool: &sqlx::SqlitePool, dir: &Path) -> Result<(), String> {
    let deps = ghost_render::HelperDeps {
        pool: Arc::new(pool.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: Arc::new(|_| "/".to_string()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: String::new(),
        theme_dir: dir.to_path_buf(),
        site_origin: "http://localhost:2368".into(),
        content_api_key: String::new(),
    };
    // A theme is user-supplied, so a malformed one must not take the process
    // down — same guard the activate path uses.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ghost_render::load_and_activate(dir, &deps, HashMap::new())
    }))
    .map_err(|_| "panicked while compiling".to_string())?
    .map(|_| ())
    .map_err(|e| e.message())
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
}
