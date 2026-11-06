//! Every SQL literal that needs translating must go through `dialect::sql`.
//!
//! This project writes its SQL in SQLite's dialect and translates it for MySQL
//! at the call site — see [`ghost_db::dialect`]. That works only if the call
//! sites are complete, and nothing in the type system enforces it: a query
//! with `ON CONFLICT` in it compiles perfectly well unwrapped and then fails
//! at runtime, on MySQL only, on whichever page happens to run it.
//!
//! So the source tree is scanned here instead. Two rules:
//!
//! * a **translatable** construct must appear inside a `dialect::sql(…)` or
//!   `dialect::sql_owned(…)` call;
//! * an **untranslatable** construct must not appear at all — those have no
//!   MySQL spelling and have to be restructured into different queries, which
//!   is what every site listed in the dialect module's docs has been.
//!
//! The scan is deliberately textual and slightly over-eager: it is better for
//! this test to demand a wrapper around a query that did not strictly need one
//! than to miss one that did.
use std::path::{Path, PathBuf};

/// Constructs the translator rewrites. Fine in a wrapped query, a bug outside.
const TRANSLATABLE: &[(&str, &str)] = &[
    ("datetime('now')", "SQLite's clock; MySQL spells it UTC_TIMESTAMP()"),
    ("ON CONFLICT", "MySQL spells this ON DUPLICATE KEY UPDATE"),
    ("INSERT OR IGNORE", "MySQL spells this INSERT IGNORE"),
    ("json_extract", "MySQL needs JSON_UNQUOTE(JSON_EXTRACT(…))"),
    ("json_valid", "MySQL spells this JSON_VALID"),
    ("COLLATE NOCASE", "MySQL has no such collation; its tables are already ci"),
];

/// Constructs with no MySQL equivalent at all.
const UNTRANSLATABLE: &[(&str, &str)] = &[
    ("RETURNING", "MySQL has no RETURNING: split into a mutation and a read, \
                   and let the mutation's row count arbitrate the race"),
    ("julianday", "MySQL has no julianday: do the arithmetic in Rust"),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strip `//` line comments, so prose describing a construct is not mistaken
/// for one. Deliberately does not try to understand `/* */` — this project
/// does not use block comments in the files that contain SQL.
fn without_comments(src: &str) -> String {
    src.lines()
        .map(|line| {
            // Not a comment if the `//` is inside a string literal; the only
            // way that happens here is a URL, which never precedes SQL on the
            // same line, so a plain find is enough.
            match line.find("//") {
                // Replaced by spaces of the same length rather than truncated:
                // the offsets of everything after it must not move.
                Some(i) if !line[..i].contains('"') => {
                    let mut kept = line[..i].to_string();
                    kept.push_str(&" ".repeat(line.len() - i));
                    kept
                }
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every string literal in `src`, with its byte offset.
fn literals(src: &str) -> Vec<(usize, String)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            let from = i;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 2,
                    b'"' => break,
                    _ => i += 1,
                }
            }
            if i <= bytes.len() {
                if let Some(text) = src.get(from..i.min(src.len())) {
                    out.push((start, text.to_string()));
                }
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// An explicit opt-out for a literal translated somewhere other than in place.
///
/// A query assembled from fragments is translated once, after assembly, so the
/// fragments themselves are bare. There is no way to see that textually, so
/// the author says so with this marker directly above the literal. Deliberately
/// a long, searchable string: it should be obvious when reading the code, and
/// easy to audit every use of.
const OPT_OUT: &str = "dialect-translated-after-assembly";

/// Whether the literal starting at `at` is an argument to `dialect::sql`.
///
/// Checked by looking back over the few characters that can legally sit
/// between the call and its argument — whitespace, an `&`, a `format!(`.
fn is_wrapped(src: &str, at: usize) -> bool {
    // The marker is in a comment, so look at the raw text, not the stripped
    // copy the scan otherwise works on.
    if src[at.saturating_sub(400)..at].contains(OPT_OUT) {
        return true;
    }
    let window_start = at.saturating_sub(80);
    let before = &src[window_start..at];
    let trimmed = before.trim_end();
    let trimmed = trimmed.strip_suffix("format!(").unwrap_or(trimmed).trim_end();
    let trimmed = trimmed.strip_suffix('&').unwrap_or(trimmed).trim_end();
    trimmed.ends_with("dialect::sql(") || trimmed.ends_with("dialect::sql_owned(")
}

/// Files whose SQL is translated somewhere other than at the literal.
///
/// * `dialect.rs` is the translator; its tests use these constructs as input.
/// * `ghost-nql` builds clause *templates* and `ghost_nql::compile` translates
///   the assembled clause before returning it, which is the single chokepoint
///   for all eight places a filter is pasted into a query. Wrapping each
///   template as well would translate twice.
fn is_exempt(file: &Path) -> bool {
    let p = file.to_string_lossy();
    file.ends_with("dialect.rs")
        || file.ends_with("dialect_coverage.rs")
        || p.contains("ghost-nql")
}

fn looks_like_sql(lit: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "SELECT ", "INSERT ", "UPDATE ", "DELETE ", " FROM ", " WHERE ", " VALUES",
    ];
    let upper = lit.to_ascii_uppercase();
    KEYWORDS.iter().any(|k| upper.contains(k))
}

#[test]
fn every_translatable_query_goes_through_the_dialect_layer() {
    let mut files = Vec::new();
    rust_files(&crates_dir(), &mut files);
    assert!(files.len() > 50, "only found {} source files", files.len());

    let mut problems = Vec::new();
    for file in &files {
        if is_exempt(file) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(file) else { continue };
        // Comments are blanked rather than removed, so byte offsets still line
        // up with the raw source and the opt-out marker stays findable.
        let src = without_comments(&raw);
        for (at, lit) in literals(&src) {
            if !looks_like_sql(&lit) {
                continue;
            }
            for (needle, why) in TRANSLATABLE {
                if lit.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
                    && !is_wrapped(&src, at)
                    && !is_wrapped(&raw, at)
                {
                    let line = src[..at].matches('\n').count() + 1;
                    problems.push(format!(
                        "{}:{line}\n      `{needle}` is not wrapped in dialect::sql — {why}",
                        file.display()
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} query/queries need dialect::sql:\n  - {}",
        problems.len(),
        problems.join("\n  - ")
    );
}

#[test]
fn no_query_uses_a_construct_mysql_cannot_express() {
    let mut files = Vec::new();
    rust_files(&crates_dir(), &mut files);

    let mut problems = Vec::new();
    for file in &files {
        if is_exempt(file) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(file) else { continue };
        let src = without_comments(&raw);
        for (at, lit) in literals(&src) {
            if !looks_like_sql(&lit) {
                continue;
            }
            for (needle, why) in UNTRANSLATABLE {
                if lit.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()) {
                    let line = src[..at].matches('\n').count() + 1;
                    problems.push(format!(
                        "{}:{line}\n      `{needle}` — {why}",
                        file.display()
                    ));
                }
            }
            // A bound date modifier: SQLite takes the modifier as a parameter,
            // MySQL's INTERVAL is syntax. Compute the instant in Rust instead.
            let lower = lit.to_ascii_lowercase();
            for form in ["datetime('now',?", "datetime('now', ?", "date('now',?", "date('now', ?"] {
                if lower.contains(form) {
                    let line = src[..at].matches('\n').count() + 1;
                    problems.push(format!(
                        "{}:{line}\n      `{form}…)` binds a date modifier, which MySQL \
                         cannot express — bind an absolute timestamp instead",
                        file.display()
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} query/queries use SQL MySQL cannot run:\n  - {}",
        problems.len(),
        problems.join("\n  - ")
    );
}
