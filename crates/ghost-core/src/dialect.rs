//! Translating this project's SQL to MySQL.
//!
//! Almost all of Ghost's SQL is already portable — `?` placeholders on both
//! engines, application-generated ids so nothing needs `AUTO_INCREMENT`, dates
//! as `'YYYY-MM-DD HH:MM:SS'` strings. What is left is a set of constructs
//! SQLite spells one way and MySQL another, and they appear in 152 of this
//! project's roughly 1,100 queries.
//!
//! ## How it is applied
//!
//! The SQL in this project is written in SQLite's dialect, and those 152
//! queries pass it through [`sql`]:
//!
//! ```ignore
//! sqlx::query(dialect::sql("DELETE FROM tokens WHERE datetime(expires_at) < datetime('now')"))
//! ```
//!
//! On SQLite that is the identity function and costs nothing. On MySQL it
//! rewrites and caches the result.
//!
//! Wrapping only the affected queries, rather than every query, is deliberate:
//! the call sites that need translation are the call sites that *do something
//! engine-specific*, and seeing `dialect::sql` there is the signal to a reader
//! that this query is one of them. `ghost-db/tests/dialect_coverage.rs` scans
//! the source tree and fails if a construct listed below appears outside a
//! `dialect::sql(…)` call, so the set cannot drift.
//!
//! ## What it cannot do
//!
//! Two constructs have no MySQL translation and are *not* handled here —
//! they are restructured at the call site instead, because the fix is a
//! different query rather than different words:
//!
//! * **`RETURNING`**, which MySQL does not have at all. A
//!   `DELETE … RETURNING` becomes a `SELECT` and a `DELETE` in one
//!   transaction, and getting the locking right is a decision per call site.
//! * **A bound date modifier**, `datetime('now', ?)`. SQLite takes a modifier
//!   string like `'-7 days'` as a parameter; MySQL's `INTERVAL` is syntax, not
//!   a value. Those sites compute the absolute timestamp in Rust and bind
//!   that, which is both portable and better — the time then comes from one
//!   place rather than from whichever machine the database runs on.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::database::{engine, Engine};

/// Translate a SQL literal for the engine in use.
///
/// Takes and returns `&'static str` so the result can be handed straight to
/// `sqlx::query`, which borrows its SQL for the life of the query. That is
/// safe to cache because every caller passes a string literal: the set is
/// fixed at compile time, so the cache is bounded by the number of queries in
/// the binary and never grows at runtime.
///
/// Dynamic SQL — anything built with `format!` — must not come through here,
/// for exactly that reason. [`sql_owned`] exists for those.
pub fn sql(sqlite_sql: &'static str) -> &'static str {
    if engine() == Engine::Sqlite {
        return sqlite_sql;
    }

    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = guard.get(sqlite_sql) {
        return found;
    }
    // Leaked once per distinct query, which is why only literals may call
    // this. The alternative is returning a `Cow` and making every call site
    // bind it to a local first.
    let translated: &'static str = Box::leak(to_mysql(sqlite_sql).into_boxed_str());
    guard.insert(sqlite_sql, translated);
    translated
}

/// [`sql`] for SQL built at runtime.
///
/// Returns an owned `String` rather than caching, so a query whose text varies
/// — an NQL filter, a dynamic `IN` list — cannot grow the cache without bound.
/// The caller binds it to a local and passes a reference.
pub fn sql_owned(sqlite_sql: &str) -> String {
    match engine() {
        Engine::Sqlite => sqlite_sql.to_string(),
        Engine::MySql => to_mysql(sqlite_sql),
    }
}

/// Rewrite SQLite-dialect SQL as MySQL.
///
/// Ordered deliberately: the two-argument `datetime` forms are matched before
/// the one-argument form, and `ON CONFLICT … DO NOTHING` before
/// `ON CONFLICT … DO UPDATE`, because the general patterns would otherwise
/// consume the specific ones.
fn to_mysql(sql: &str) -> String {
    let mut out = sql.to_string();

    // Double-quoted identifiers become backticks *first*: MySQL (without
    // `ANSI_QUOTES`, which this project does not set) reads `"` as a string
    // delimiter, so `INSERT INTO "posts"` is a syntax error there. SQLite
    // accepts both spellings, which is why the source can be written with
    // `"` and translated here. Before the other rewrites, because the upsert
    // rewrite has to recognise `excluded.` through a backticked column name.
    out = rewrite_double_quoted_identifiers(&out);
    // Reserved words are quoted next, before the date rewrites run. Those
    // emit `INTERVAL 5 MINUTE`, and `INTERVAL` is itself a reserved word — so
    // quoting afterwards would turn the keyword this function just wrote into
    // `` `INTERVAL` `` and break the statement.
    out = quote_reserved_words(&out);
    out = rewrite_conflict_do_nothing(&out);
    out = rewrite_conflict_do_update(&out);
    out = rewrite_insert_or_ignore(&out);
    out = rewrite_datetime_with_modifier(&out);
    out = rewrite_datetime_now(&out);
    out = rewrite_datetime_of_expression(&out);
    out = rewrite_json(&out);
    out = rewrite_collate_nocase(&out);
    out
}

/// `"name"` → `` `name` ``, outside single-quoted string literals.
///
/// Import/export builds its SQL dynamically — the table and every column name
/// come from data — and quotes them the SQLite way. Backticks are legal on
/// both engines, so this makes that SQL readable by MySQL without a second
/// quoting convention at the call sites. A `"` inside a `'…'` literal is
/// content, not an identifier, and is left alone; a `''` escape toggles the
/// scanner twice, which lands back inside the same literal.
fn rewrite_double_quoted_identifiers(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_string = false;
    for c in sql.chars() {
        if c == '\'' {
            in_string = !in_string;
            out.push(c);
        } else if c == '"' && !in_string {
            out.push('`');
        } else {
            out.push(c);
        }
    }
    out
}

/// Drop SQLite's `COLLATE NOCASE`.
///
/// Not a translation but a deletion, and that is the point worth recording:
/// the MySQL tables are declared `utf8mb4_unicode_ci`, which already compares
/// case-insensitively, so removing the clause preserves the behaviour rather
/// than losing it. A case-*sensitive* comparison is the one that would need
/// spelling out on MySQL.
fn rewrite_collate_nocase(sql: &str) -> String {
    replace_ci(sql, " COLLATE NOCASE", "")
}

/// `INSERT OR IGNORE INTO` → `INSERT IGNORE INTO`.
fn rewrite_insert_or_ignore(sql: &str) -> String {
    replace_ci(sql, "INSERT OR IGNORE INTO", "INSERT IGNORE INTO")
}

/// `INSERT … ON CONFLICT(…) DO NOTHING` → `INSERT IGNORE … `.
///
/// MySQL has no way to say "ignore a conflict on these specific columns", so
/// this moves the intent to the verb and drops the tail. The difference is
/// real and worth knowing: `INSERT IGNORE` also swallows other errors that
/// `DO NOTHING` would have raised — a row too long, a bad foreign key. Every
/// site using this inserts a row whose only plausible failure *is* the
/// duplicate it is guarding against.
fn rewrite_conflict_do_nothing(sql: &str) -> String {
    let Some(at) = find_ci(sql, "ON CONFLICT") else {
        return sql.to_string();
    };
    // Only this form; `DO UPDATE` is handled separately.
    let tail = &sql[at..];
    let Some(end) = find_ci(tail, "DO NOTHING") else {
        return sql.to_string();
    };
    let after = &tail[end + "DO NOTHING".len()..];
    let before = &sql[..at];

    let head = replace_ci(before, "INSERT INTO", "INSERT IGNORE INTO");
    format!("{}{}", head.trim_end(), after)
}

/// `ON CONFLICT(…) DO UPDATE SET x=excluded.x` →
/// `ON DUPLICATE KEY UPDATE x=VALUES(x)`.
///
/// MySQL cannot name *which* constraint conflicted; it reacts to any unique
/// key on the table. On a table with one unique key the two are equivalent,
/// which covers every site here — a second unique key would make the MySQL
/// form update on a conflict SQLite would have let fail.
///
/// `excluded.col` becomes `VALUES(col)` rather than MySQL 8.0.19's `AS new`
/// row alias, because `VALUES()` also works on MySQL 5.7 and MariaDB. It is
/// deprecated in 8.0.20 but not removed.
fn rewrite_conflict_do_update(sql: &str) -> String {
    let Some(at) = find_ci(sql, "ON CONFLICT") else {
        return sql.to_string();
    };
    let tail = &sql[at..];
    let Some(do_at) = find_ci(tail, "DO UPDATE SET") else {
        return sql.to_string();
    };

    let head = &sql[..at];
    let assignments = &tail[do_at + "DO UPDATE SET".len()..];
    // `excluded.` is SQLite's name for the row that failed to insert.
    let assignments = replace_ci(assignments, "excluded.", "__EXCLUDED__.");
    let assignments = rewrite_excluded_refs(&assignments);

    format!("{}ON DUPLICATE KEY UPDATE{}", head, assignments)
}

/// `__EXCLUDED__.col` → `VALUES(col)`.
///
/// The column may be backticked — `` __EXCLUDED__.`col` `` — because the
/// dynamic upserts quote every identifier; see
/// [`rewrite_double_quoted_identifiers`].
fn rewrite_excluded_refs(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(at) = rest.find("__EXCLUDED__.") {
        out.push_str(&rest[..at]);
        let mut after = &rest[at + "__EXCLUDED__.".len()..];
        let quoted = after.starts_with('`');
        if quoted {
            after = &after[1..];
        }
        let end = after
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let (column, remainder) = after.split_at(end);
        if quoted {
            after = remainder.strip_prefix('`').unwrap_or(remainder);
        } else {
            after = remainder;
        }
        // The backticks, where there were any, must come back: the column may
        // be a reserved word (`interval` is, on tiers and offers), which
        // `VALUES(interval)` without them would reject.
        let reference = if quoted {
            format!("VALUES(`{column}`)")
        } else {
            format!("VALUES({column})")
        };
        out.push_str(&reference);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `datetime('now','-5 minutes')` →
/// `CAST(DATE_ADD(UTC_TIMESTAMP(), INTERVAL -5 MINUTE) AS CHAR)`.
///
/// SQLite's modifier is a string; MySQL's `INTERVAL` is syntax, so the number
/// and the unit have to be parsed out. Only the literal form is handled — a
/// bound modifier cannot be, which is why those sites are restructured
/// instead. See the module docs.
///
/// The `CAST` is not decoration. `DATE_ADD` returns a MySQL `DATETIME`, and
/// the `Any` driver refuses that type outright — so a query that *fetches*
/// one of these expressions (`SELECT datetime('now','-29 days')` is real, in
/// the newsletter analytics) would fail on MySQL no matter what Rust type it
/// decodes into. As `CHAR` it arrives as text, in exactly the
/// `'YYYY-MM-DD HH:MM:SS'` form this project stores everywhere. In a
/// comparison the cast is harmless: the string coerces back to a datetime
/// against a `CAST(col AS DATETIME)` on the other side.
fn rewrite_datetime_with_modifier(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;

    while let Some(at) = find_ci(rest, "datetime('now'") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "datetime('now'".len()..];

        // Either `)` — handled by the one-argument rewrite — or `, '<mod>')`.
        let trimmed = after.trim_start_matches([' ', '\t']);
        let Some(rest_after_comma) = trimmed.strip_prefix(',') else {
            out.push_str("datetime('now'");
            rest = after;
            continue;
        };
        let modifier_start = rest_after_comma.trim_start_matches([' ', '\t']);
        let Some(quoted) = modifier_start.strip_prefix('\'') else {
            // `datetime('now', ?)`: a bound modifier, untranslatable.
            out.push_str("datetime('now'");
            rest = after;
            continue;
        };
        let Some(close) = quoted.find('\'') else {
            out.push_str("datetime('now'");
            rest = after;
            continue;
        };
        let modifier = &quoted[..close];
        let remainder = &quoted[close + 1..];
        let remainder = remainder.trim_start_matches([' ', '\t']);
        let Some(remainder) = remainder.strip_prefix(')') else {
            out.push_str("datetime('now'");
            rest = after;
            continue;
        };

        match interval(modifier) {
            Some(expr) => out.push_str(&format!("CAST({expr} AS CHAR)")),
            // Unrecognised: leave it alone so the failure is a clear MySQL
            // syntax error naming this query, rather than a silently wrong
            // date.
            None => {
                out.push_str(&format!("datetime('now','{modifier}')"));
            }
        }
        rest = remainder;
    }
    out.push_str(rest);
    out
}

/// Turn a SQLite modifier like `-5 minutes` into a MySQL `INTERVAL` expression.
fn interval(modifier: &str) -> Option<String> {
    let m = modifier.trim();
    let (sign, body) = match m.strip_prefix('-') {
        Some(b) => ("-", b),
        None => ("", m.strip_prefix('+').unwrap_or(m)),
    };
    let (count, unit) = body.trim().split_once(' ')?;
    let count: u64 = count.trim().parse().ok()?;
    // MySQL's units are singular.
    let unit = match unit.trim().trim_end_matches('s').to_ascii_lowercase().as_str() {
        "second" => "SECOND",
        "minute" => "MINUTE",
        "hour" => "HOUR",
        "day" => "DAY",
        "month" => "MONTH",
        "year" => "YEAR",
        _ => return None,
    };
    Some(format!("DATE_ADD(UTC_TIMESTAMP(), INTERVAL {sign}{count} {unit})"))
}

/// `datetime('now')` → `CAST(UTC_TIMESTAMP() AS CHAR)`.
///
/// `UTC_TIMESTAMP()` rather than `NOW()`: the session is pinned to UTC in
/// `pool::after_connect`, but `UTC_TIMESTAMP()` is correct regardless of that
/// and every date this project stores is UTC. The cast is for the `Any`
/// driver, which cannot carry a `DATETIME` result at all — see
/// [`rewrite_datetime_with_modifier`].
fn rewrite_datetime_now(sql: &str) -> String {
    replace_ci(sql, "datetime('now')", "CAST(UTC_TIMESTAMP() AS CHAR)")
}

/// `datetime(expr)` → `CAST(expr AS DATETIME)`.
///
/// SQLite uses this to normalise a stored text date before comparing it.
/// MySQL would coerce the string implicitly in most comparisons, but an
/// explicit cast keeps the comparison a date comparison rather than a string
/// one — which matters the moment a stored value is missing its seconds.
fn rewrite_datetime_of_expression(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;

    while let Some(at) = find_ci(rest, "datetime(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "datetime(".len()..];

        // Find the matching close paren, tracking nesting.
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push_str("datetime(");
            rest = after;
            continue;
        };
        let inner = &after[..end];
        // A remaining `'now'` means a bound-modifier form the earlier pass
        // left alone; do not touch it.
        if inner.contains('?') || inner.to_ascii_lowercase().contains("'now'") {
            out.push_str("datetime(");
            rest = after;
            continue;
        }
        out.push_str(&format!("CAST({inner} AS DATETIME)"));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// SQLite's JSON functions to MySQL's.
///
/// `JSON_EXTRACT` alone would return a *quoted* JSON scalar — `"abc"` rather
/// than `abc` — where SQLite's `json_extract` returns the bare value. Every
/// caller here compares the result to a plain string, so the unquoting is
/// required, not cosmetic.
fn rewrite_json(sql: &str) -> String {
    let mut out = replace_ci(sql, "json_valid(", "JSON_VALID(");
    out = wrap_calls(&out, "json_extract(", "JSON_UNQUOTE(JSON_EXTRACT(", "))");
    out
}

/// Rewrite `name(args)` as `open args close`, respecting nesting.
fn wrap_calls(sql: &str, name: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(at) = find_ci(rest, name) {
        out.push_str(&rest[..at]);
        let after = &rest[at + name.len()..];
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push_str(name);
            rest = after;
            continue;
        };
        out.push_str(open);
        out.push_str(&after[..end]);
        out.push_str(close);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Backquote the column names that are reserved words in MySQL.
///
/// Only `key` and `interval` in this schema. `status` and `value` read as
/// reserved but are not — MySQL lists them as non-reserved keywords, usable
/// unquoted. Backticks are used because SQLite accepts them too, for MySQL
/// compatibility, so a quoted name is valid on both.
fn quote_reserved_words(sql: &str) -> String {
    let mut out = sql.to_string();
    for word in ["interval"] {
        out = quote_word(&out, word);
    }
    out
}

/// Replace a bare word with its backquoted form, leaving it alone where it is
/// already quoted or part of a longer identifier.
fn quote_word(sql: &str, word: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let lower = sql.to_ascii_lowercase();
    let mut i = 0usize;

    while let Some(rel) = lower[i..].find(word) {
        let at = i + rel;
        let end = at + word.len();
        let before = sql[..at].chars().next_back();
        let after = sql[end..].chars().next();
        let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        // `INTERVAL 5 MINUTE` is syntax this translator emits itself, and the
        // backtick before it would be a syntax error — so a word already
        // adjacent to a backtick, or followed by a digit, is left alone.
        let already_quoted = before == Some('`') || after == Some('`');

        out.push_str(&sql[i..at]);
        if boundary(before) && boundary(after) && !already_quoted {
            out.push_str(&format!("`{}`", &sql[at..end]));
        } else {
            out.push_str(&sql[at..end]);
        }
        i = end;
    }
    out.push_str(&sql[i..]);
    out
}

// ---------------------------------------------------------------------------
// Case-insensitive helpers
// ---------------------------------------------------------------------------

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
}

fn replace_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let lower_needle = needle.to_ascii_lowercase();
    let mut rest = haystack;
    loop {
        let lower = rest.to_ascii_lowercase();
        match lower.find(&lower_needle) {
            Some(at) => {
                out.push_str(&rest[..at]);
                out.push_str(replacement);
                rest = &rest[at + needle.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `to_mysql` is tested directly: `sql()` short-circuits on SQLite, which
    /// is the engine the test suite runs on.
    fn m(sql: &str) -> String {
        to_mysql(sql)
    }

    #[test]
    fn now_becomes_utc_timestamp() {
        // The cast is what makes a *fetched* `datetime('now')` decodable: the
        // `Any` driver refuses a MySQL `DATETIME` result outright.
        assert_eq!(
            m("SELECT 1 WHERE a < datetime('now')"),
            "SELECT 1 WHERE a < CAST(UTC_TIMESTAMP() AS CHAR)"
        );
    }

    #[test]
    fn a_modifier_becomes_an_interval() {
        assert_eq!(
            m("WHERE a < datetime('now','-5 minutes')"),
            "WHERE a < CAST(DATE_ADD(UTC_TIMESTAMP(), INTERVAL -5 MINUTE) AS CHAR)"
        );
        assert_eq!(
            m("WHERE a > datetime('now','+10 days')"),
            "WHERE a > CAST(DATE_ADD(UTC_TIMESTAMP(), INTERVAL 10 DAY) AS CHAR)"
        );
        // Spacing after the comma varies across the codebase.
        assert_eq!(
            m("WHERE a < datetime('now', '-29 days')"),
            "WHERE a < CAST(DATE_ADD(UTC_TIMESTAMP(), INTERVAL -29 DAY) AS CHAR)"
        );
    }

    #[test]
    fn every_unit_used_in_this_project_is_understood() {
        for (modifier, expected) in [
            ("-1 minute", "INTERVAL -1 MINUTE"),
            ("-1 hour", "INTERVAL -1 HOUR"),
            ("-60 days", "INTERVAL -60 DAY"),
            ("+1 day", "INTERVAL 1 DAY"),
            ("-30 seconds", "INTERVAL -30 SECOND"),
        ] {
            let got = m(&format!("WHERE a < datetime('now','{modifier}')"));
            assert!(got.contains(expected), "{modifier} produced {got}");
        }
    }

    /// An unrecognised modifier must survive untranslated, so the failure is a
    /// loud MySQL syntax error rather than a silently wrong date.
    #[test]
    fn an_unknown_modifier_is_left_alone() {
        let got = m("WHERE a < datetime('now','start of month')");
        assert!(got.contains("datetime('now','start of month')"), "{got}");
    }

    /// A bound modifier cannot be translated and must be left for the call
    /// site to restructure — silently mistranslating it would compare against
    /// the wrong instant.
    #[test]
    fn a_bound_modifier_is_left_alone() {
        let got = m("WHERE a < datetime('now', ?)");
        assert!(got.contains("datetime('now', ?)"), "{got}");
        let got = m("WHERE a < datetime('now',?)");
        assert!(got.contains("datetime('now',?)"), "{got}");
    }

    #[test]
    fn a_column_is_cast_to_a_date() {
        assert_eq!(
            m("WHERE datetime(expires_at) > datetime('now')"),
            "WHERE CAST(expires_at AS DATETIME) > CAST(UTC_TIMESTAMP() AS CHAR)"
        );
        // Qualified columns appear throughout.
        assert_eq!(
            m("WHERE datetime(mp.expiry_at) < datetime('now')"),
            "WHERE CAST(mp.expiry_at AS DATETIME) < CAST(UTC_TIMESTAMP() AS CHAR)"
        );
    }

    /// The dynamic import upserts quote every identifier the SQLite way;
    /// MySQL reads `"` as a string delimiter, so those must become backticks.
    #[test]
    fn double_quoted_identifiers_become_backticks() {
        assert_eq!(
            m("INSERT INTO \"posts\" (\"id\",\"html\") VALUES (?,?)"),
            "INSERT INTO `posts` (`id`,`html`) VALUES (?,?)"
        );
        // A double quote inside a string literal is content, not an identifier.
        assert_eq!(
            m("SELECT 'say \"hi\"' FROM \"posts\""),
            "SELECT 'say \"hi\"' FROM `posts`"
        );
        // `''` is an escaped quote inside the literal, not the end of it.
        assert_eq!(
            m("SELECT 'it''s \"quoted\"' FROM t"),
            "SELECT 'it''s \"quoted\"' FROM t"
        );
    }

    /// A quoted column in an upsert's `excluded.` reference has to survive the
    /// `VALUES(…)` rewrite — this exact shape is what `transfer::import` emits
    /// for every column of every table.
    #[test]
    fn an_upsert_with_quoted_columns_is_rewritten() {
        assert_eq!(
            m("INSERT INTO \"tiers\" (\"id\",\"interval\") VALUES (?,?) \
               ON CONFLICT(id) DO UPDATE SET \"interval\"=excluded.\"interval\""),
            "INSERT INTO `tiers` (`id`,`interval`) VALUES (?,?) \
             ON DUPLICATE KEY UPDATE `interval`=VALUES(`interval`)"
        );
        // And the `DO NOTHING` form.
        assert_eq!(
            m("INSERT INTO \"tags\" (\"id\") VALUES (?) ON CONFLICT(id) DO NOTHING"),
            "INSERT IGNORE INTO `tags` (`id`) VALUES (?)"
        );
    }

    #[test]
    fn insert_or_ignore_moves_to_the_verb() {
        assert_eq!(
            m("INSERT OR IGNORE INTO tags (id) VALUES (?)"),
            "INSERT IGNORE INTO tags (id) VALUES (?)"
        );
    }

    #[test]
    fn do_nothing_becomes_insert_ignore() {
        assert_eq!(
            m("INSERT INTO tokens (token) VALUES (?) ON CONFLICT(token) DO NOTHING"),
            "INSERT IGNORE INTO tokens (token) VALUES (?)"
        );
    }

    #[test]
    fn do_update_becomes_on_duplicate_key() {
        assert_eq!(
            m("INSERT INTO settings (`key`,value) VALUES (?,?) \
               ON CONFLICT(`key`) DO UPDATE SET value=excluded.value"),
            "INSERT INTO settings (`key`,value) VALUES (?,?) \
             ON DUPLICATE KEY UPDATE value=VALUES(value)"
        );
    }

    /// Several upserts assign more than one column, and one wraps the excluded
    /// value in `COALESCE` to keep an existing value when the new one is null.
    #[test]
    fn several_excluded_columns_are_all_rewritten() {
        let got = m("INSERT INTO members (email,name,note) VALUES (?,?,?) \
                     ON CONFLICT(email) DO UPDATE SET \
                     name=COALESCE(excluded.name,members.name),note=excluded.note");
        assert!(got.contains("ON DUPLICATE KEY UPDATE"), "{got}");
        assert!(got.contains("COALESCE(VALUES(name),members.name)"), "{got}");
        assert!(got.contains("note=VALUES(note)"), "{got}");
        assert!(!got.contains("excluded"), "an excluded reference survived: {got}");
    }

    #[test]
    fn json_extract_is_unquoted() {
        assert_eq!(
            m("SELECT json_extract(data,'$.gift') FROM t"),
            "SELECT JSON_UNQUOTE(JSON_EXTRACT(data,'$.gift')) FROM t"
        );
        assert_eq!(m("WHERE json_valid(data)"), "WHERE JSON_VALID(data)");
    }

    /// Two extracts in one select list, which is how `stripe_connect` reads
    /// its state and started flags.
    #[test]
    fn several_json_extracts_are_each_wrapped() {
        let got = m("SELECT json_extract(d,'$.a'), json_extract(d,'$.b') FROM t");
        assert_eq!(
            got,
            "SELECT JSON_UNQUOTE(JSON_EXTRACT(d,'$.a')), JSON_UNQUOTE(JSON_EXTRACT(d,'$.b')) FROM t"
        );
    }

    /// `interval` is a reserved word in MySQL and is a real column name in
    /// this schema — on `tiers` and on `offers`.
    #[test]
    fn a_reserved_column_is_quoted() {
        assert_eq!(
            m("SELECT interval FROM tiers"),
            "SELECT `interval` FROM tiers"
        );
        assert_eq!(m("SELECT t.interval FROM tiers t"), "SELECT t.`interval` FROM tiers t");
    }

    /// The translator emits `INTERVAL 5 MINUTE` itself, and quoting that would
    /// be a syntax error. This is the interaction between two rewrites, which
    /// is the kind of thing that only shows up once both run.
    #[test]
    fn the_interval_keyword_it_emits_is_not_quoted() {
        let got = m("WHERE a < datetime('now','-5 minutes')");
        assert!(!got.contains("`INTERVAL`"), "{got}");
        assert!(got.contains("INTERVAL -5 MINUTE"), "{got}");
    }

    /// An already-quoted column must not be quoted twice.
    #[test]
    fn an_already_quoted_column_is_left_alone() {
        assert_eq!(m("SELECT `interval` FROM tiers"), "SELECT `interval` FROM tiers");
    }

    /// Nothing engine-specific means nothing changes, which keeps the wrapper
    /// safe to apply to a query that only partly needs it.
    /// SQLite's case-insensitive collation has no MySQL spelling, because the
    /// tables are already case-insensitive. Dropping it keeps the meaning.
    #[test]
    fn collate_nocase_is_dropped() {
        assert_eq!(
            m("SELECT slug FROM tags ORDER BY name COLLATE NOCASE"),
            "SELECT slug FROM tags ORDER BY name"
        );
        assert_eq!(
            m("WHERE `key`=? COLLATE NOCASE LIMIT 1"),
            "WHERE `key`=? LIMIT 1"
        );
    }

    #[test]
    fn portable_sql_is_untouched() {
        let sql = "SELECT id, title FROM posts WHERE slug = ? ORDER BY published_at DESC LIMIT 10";
        assert_eq!(m(sql), sql);
    }

    /// On SQLite the entry point is the identity function, and must return the
    /// very same pointer — this is on the path of every wrapped query.
    #[test]
    fn sqlite_translation_is_free() {
        let sql: &'static str = "SELECT 1 WHERE a < datetime('now')";
        assert!(std::ptr::eq(sql, sql_identity(sql)));
    }

    fn sql_identity(s: &'static str) -> &'static str {
        // `sql` short-circuits on SQLite, which is what the suite runs on.
        sql(s)
    }

    #[test]
    fn owned_translation_matches_the_cached_one() {
        let sql = "WHERE a < datetime('now')";
        assert_eq!(sql_owned(sql), sql, "sqlite should be unchanged");
        assert_eq!(to_mysql(sql), "WHERE a < CAST(UTC_TIMESTAMP() AS CHAR)");
    }
}
