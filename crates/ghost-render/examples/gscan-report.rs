//! Scan theme directories and print what GScan finds.
//!
//! `cargo run --example gscan-report -p ghost-render -- content/themes/source`
//!
//! Exists because comparing this port against the real `gscan` is the only
//! way to know the two agree, and that comparison wants a report on stdout
//! rather than a test assertion. `tests/gscan_parity.rs` automates the same
//! comparison; this is for looking at one theme by hand.
//!
//! Exits non-zero if any theme has a finding that would block activation, so
//! it can be used as a check in a script.

fn main() {
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    if dirs.is_empty() {
        eprintln!("usage: gscan-report <theme-dir>...");
        std::process::exit(2);
    }
    let mut blocked = false;
    for dir in dirs {
        let report = ghost_render::gscan::scan(std::path::Path::new(&dir));
        println!("=== {dir}");
        for finding in &report.findings {
            println!(
                "{:<14} {:<48} {}{}",
                finding.level.label(),
                finding.code,
                finding.failures.first().map(String::as_str).unwrap_or("-"),
                match finding.failures.len() {
                    0 | 1 => String::new(),
                    n => format!(" (+{} more)", n - 1),
                },
            );
            if finding.fatal {
                println!("{:<14} ^ blocks activation: {}", "", finding.rule);
            }
        }
        if report.findings.is_empty() {
            println!("  clean");
        }
        blocked |= report.is_fatal();
    }
    if blocked {
        std::process::exit(1);
    }
}
