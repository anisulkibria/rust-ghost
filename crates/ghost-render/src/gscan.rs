//! GScan — Ghost's theme validator, ported.
//!
//! Ported from `gscan` 6.6.1, the tool Ghost runs when a theme is uploaded or
//! activated. Rule codes, levels, wording and the fatal/non-fatal split are
//! upstream's, so a code reported here can be looked up in Ghost's own
//! documentation and the answer applies.
//!
//! Scope: gscan carries 266 rules, and all of them are ported.
//!
//! An earlier cut of this module left out the ~100 `GS001-DEPR-*` rules on the
//! reasoning that they are an upgrade catalogue for Ghost 1.x–5.x themes and
//! so do not apply to a port that does not claim to run those themes. Reading
//! gscan more closely showed that reasoning to be wrong, and in an instructive
//! way: its `specs/v6` is `_.merge({}, previousRules, rules)` over v5, which
//! merges v4, and so on down to v1. **Every deprecation rule is live when
//! checking against v6.** That is the point of them — they are not there to
//! describe old Ghost, they are there to tell the author of a theme being
//! brought forward which helper stopped working. A theme using
//! `{{twitter_url}}` or `{{author.cover}}` is told so by Ghost and was not
//! told so here.
//!
//! The rule table itself is generated from the real tool's own spec by
//! `tools/gscan-rules.js` into [`crate::gscan_rules`] — codes, levels,
//! fatal flags, regexes and wording. Transcribing 266 rules by hand is how a
//! port drifts from the thing it claims to match. What is written here is the
//! part gscan writes as code too: which check applies which rules, to which
//! files, and what it says when one fires.
use crate::gscan_rules::{self, Rule};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Recommendation,
}

impl Level {
    pub fn label(&self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Recommendation => "recommendation",
        }
    }
}

/// One rule that fired, and every place it fired.
#[derive(Debug, Clone)]
pub struct Finding {
    pub code: &'static str,
    pub level: Level,
    /// Upstream's one-line statement of the rule.
    pub rule: String,
    /// What to do about it.
    pub details: String,
    /// `true` when the theme cannot be activated until this is fixed.
    pub fatal: bool,
    /// `file:line`, or just the file, for each occurrence.
    pub failures: Vec<String>,
}

/// Everything one scan found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.level == Level::Error)
    }
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.level == Level::Warning)
    }
    pub fn recommendations(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.level == Level::Recommendation)
    }
    /// Whether anything found would stop the theme being activated.
    pub fn is_fatal(&self) -> bool {
        self.findings.iter().any(|f| f.fatal)
    }
    /// The first fatal finding, worded for an error response.
    pub fn fatal_message(&self) -> Option<String> {
        self.findings.iter().find(|f| f.fatal).map(|f| {
            if f.failures.is_empty() {
                format!("{}: {}", f.code, f.rule)
            } else {
                format!("{}: {} ({})", f.code, f.rule, f.failures.join(", "))
            }
        })
    }
}

/// The helpers Ghost ships, from gscan's `knownHelpers` for v6.
///
/// Exposed so the renderer and the admin can agree on what counts as a known
/// helper rather than each keeping its own list. Generated with the rules, so
/// a helper added upstream arrives here by regenerating rather than by
/// somebody noticing.
pub use crate::gscan_rules::KNOWN_HELPERS;

pub fn scan(theme_dir: &Path) -> Report {
    let mut report = Report::default();
    let package = read_package_json(theme_dir);
    check_package_json(theme_dir, &mut report);
    check_structure(theme_dir, &mut report);
    let files = collect_files(theme_dir);
    let templates = templates_from(&files);
    let reachable = reachable_templates(&templates);
    check_templates(&templates, &reachable, &mut report);
    check_rules(&files, package.as_ref(), &mut report);
    check_references(&templates, &reachable, package.as_ref(), &mut report);
    check_assets(&files, theme_dir, &mut report);
    check_translations(&files, &mut report);
    check_custom_fonts(&files, &mut report);
    check_layout_recursion(&templates, &mut report);
    check_koenig_css(theme_dir, &mut report);
    report.findings.sort_by(|a, b| a.level.cmp(&b.level).then(a.code.cmp(b.code)));
    report
}

/// Compiled lazily and once: 200-odd regexes are cheap to hold and not cheap
/// to rebuild per theme, and a theme upload compiles the whole set.
///
/// A pattern that fails to compile is dropped with a log line rather than
/// panicking. The generator keeps the table faithful to upstream; if one ever
/// slips through, failing to report one rule is a far better outcome than
/// refusing to validate any theme at all. `every_generated_pattern_compiles`
/// is what makes sure that path stays unused.
static COMPILED: Lazy<HashMap<&'static str, regex::Regex>> = Lazy::new(|| {
    gscan_rules::RULES
        .iter()
        .filter_map(|rule| {
            let pattern = rule.regex?;
            match regex::Regex::new(pattern) {
                Ok(compiled) => Some((rule.code, compiled)),
                Err(error) => {
                    tracing::warn!("gscan rule {} has an unusable pattern: {error}", rule.code);
                    None
                }
            }
        })
        .collect()
});

/// Look a rule up by code, for the checks written out by hand.
///
/// They still take their level, fatal flag and wording from the generated
/// table — only the test is different — so there is one source for the text a
/// theme author reads.
fn rule_for(code: &str) -> Option<&'static Rule> {
    gscan_rules::RULES.iter().find(|rule| rule.code == code)
}

/// Record a finding from a table rule, if it fired anywhere.
fn add_rule(report: &mut Report, code: &str, failures: Vec<String>) {
    if failures.is_empty() {
        return;
    }
    let Some(rule) = rule_for(code) else { return };
    report.findings.push(Finding {
        code: rule.code,
        level: rule.level,
        rule: rule.rule.to_string(),
        details: rule.details.to_string(),
        fatal: rule.fatal,
        failures,
    });
}

/// Any theme file, not just the templates.
///
/// The deprecation rules reach into stylesheets (`GS001-DEPR-CSS-*`) and the
/// removed JS API check reads `.js`, so the file list cannot be `.hbs` alone.
struct ThemeFile {
    /// Path relative to the theme root, as upstream's `ref`.
    name: String,
    extension: String,
    content: String,
    symlink: bool,
}

fn collect_files(theme_dir: &Path) -> Vec<ThemeFile> {
    let mut found = Vec::new();
    let mut stack = vec![theme_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            // Upstream reads a theme out of its zip, so there is no `.git` or
            // `node_modules` in it to begin with; skipping them here keeps a
            // theme checked out of version control scanning the same way.
            if file_name.starts_with('.') || file_name == "node_modules" {
                continue;
            }
            let symlink = entry.file_type().is_ok_and(|kind| kind.is_symlink());
            if path.is_dir() {
                if !symlink {
                    stack.push(path);
                }
                continue;
            }
            let name = path
                .strip_prefix(theme_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| file_name.clone());
            let extension =
                path.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
            // A symlink is reported for being one; its target is not read.
            let content = if symlink {
                String::new()
            } else {
                std::fs::read_to_string(&path).unwrap_or_default()
            };
            found.push(ThemeFile { name, extension, content, symlink });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn read_package_json(theme_dir: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(theme_dir.join("package.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The `ghost-api` version a theme declares, which decides whether the
/// behaviour-change warnings apply to it.
///
/// gscan defaults this to its own current version, so a theme that says
/// nothing is treated as a current theme and is not warned about a change it
/// predates.
fn target_api(package: Option<&serde_json::Value>) -> String {
    package
        .and_then(|p| p.get("engines"))
        .and_then(|e| e.get("ghost-api"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("v5")
        .to_string()
}

// ---------------------------------------------------------------------------
// GS001 / GS060 / GS080 — the regex-driven catalogue
// ---------------------------------------------------------------------------

/// The rules this pass owns, and one DFA that answers "which of these appear
/// in this text" in a single scan.
///
/// The obvious loop — every rule against every file — was measured at 33–47 ms
/// per theme, and the themes screen scans every installed theme on each
/// render, so ten themes meant nearly half a second of regex. A `RegexSet`
/// compiles all 160-odd patterns into one automaton and reports which matched
/// after one pass over the text; only those few then get an individual search
/// for the line number. Same answers, one scan instead of 160.
struct RuleSet {
    set: regex::RegexSet,
    rules: Vec<&'static Rule>,
}

static CATALOGUE: Lazy<RuleSet> = Lazy::new(|| {
    let rules: Vec<&Rule> = gscan_rules::RULES
        .iter()
        // GS050 is the Koenig stylesheet check, which has its own pass: it
        // searches one concatenated haystack and honours `card_assets`.
        .filter(|rule| !rule.code.starts_with("GS050-"))
        .filter(|rule| rule.regex.is_some() && COMPILED.contains_key(rule.code))
        .collect();
    let set = regex::RegexSet::new(rules.iter().map(|rule| rule.regex.expect("filtered")))
        .expect("every pattern compiled individually above");
    RuleSet { set, rules }
});

/// Apply every table rule that is a pattern over a file's text.
///
/// This is gscan's `001-deprecations`, `060-js-api-usage` and
/// `080-helper-usage` in one pass, because all three are the same operation:
/// a regex, a set of extensions, and a message.
fn check_rules(files: &[ThemeFile], package: Option<&serde_json::Value>, report: &mut Report) {
    let api = target_api(package);
    let catalogue = &*CATALOGUE;
    let mut failures: HashMap<&'static str, Vec<String>> = HashMap::new();

    for file in files {
        // Nothing in this catalogue looks at any other kind of file, so the
        // DFA is not run over images or fonts.
        if !matches!(file.extension.as_str(), ".hbs" | ".css" | ".js") {
            continue;
        }
        for index in catalogue.set.matches(&file.content) {
            let rule = catalogue.rules[index];
            if !rule_covers(rule, file) {
                continue;
            }
            // A behaviour-change warning aimed at v3 themes says nothing
            // useful to a v5 theme, which never saw the old behaviour.
            if rule.valid_in_api.is_some_and(|allowed| !allowed.contains(&api.as_str())) {
                continue;
            }
            let pattern = &COMPILED[rule.code];
            let into = failures.entry(rule.code).or_default();
            if rule.css {
                // Upstream reports each matched declaration in a stylesheet,
                // not just the file, because one stylesheet can carry many.
                for found in pattern.find_iter(&file.content) {
                    into.push(format!("{}: {}", file.name, found.as_str().trim()));
                }
            } else if let Some(line) = first_matching_line(pattern, &file.content) {
                into.push(match rule.helper {
                    Some(helper) => format!("{}:{line}: remove or replace {helper}", file.name),
                    None => format!("{}:{line}", file.name),
                });
            }
        }
    }

    // Reported in the catalogue's order rather than the order files happened
    // to be read, so two scans of one theme give the same report.
    for rule in &catalogue.rules {
        if let Some(found) = failures.remove(rule.code) {
            add_rule(report, rule.code, found);
        }
    }
}

/// Whether a rule looks at this file at all.
fn rule_covers(rule: &Rule, file: &ThemeFile) -> bool {
    // The one rule that is legitimate in the very template it would flag:
    // `{{author.<x>}}` is a deprecation everywhere except `author.hbs`, where
    // the author *is* the context.
    if rule.not_valid_in.is_some_and(|only| only == file.name) {
        return false;
    }
    if rule.css {
        return file.extension == ".css";
    }
    match rule.code {
        // The removed v0.1 JS API could be called from either.
        "GS060-JS-GUA" => matches!(file.extension.as_str(), ".js" | ".hbs"),
        _ => file.extension == ".hbs",
    }
}

/// The 1-based line of the first match, so a finding points somewhere.
///
/// Upstream reports the file alone. A line is strictly more use to whoever has
/// to fix it, and costs one scan of text already in memory.
fn first_matching_line(pattern: &regex::Regex, content: &str) -> Option<usize> {
    let found = pattern.find(content)?;
    Some(content[..found.start()].lines().count().max(1))
}

// ---------------------------------------------------------------------------
// GS030 — assets
// ---------------------------------------------------------------------------

/// Local assets must go through `{{asset}}`, and symlinks are refused.
///
/// The `{{asset}}` rule is one of the two whose upstream regex uses a negative
/// lookahead — "an `/assets/` path that is *not* already absolute" — so the
/// absolute case is excluded here instead. The reason for the rule is cache
/// busting: `{{asset}}` appends the theme's hash, and a hand-written path is
/// served stale after an update.
fn check_assets(files: &[ThemeFile], theme_dir: &Path, report: &mut Report) {
    static ASSET_PATH: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r#"(?:src|href)=['"]([^'"]*?/assets/[^'"]*?)['"]"#).expect("valid pattern")
    });
    let mut unhelped = Vec::new();
    for file in files.iter().filter(|f| f.extension == ".hbs") {
        for found in ASSET_PATH.captures_iter(&file.content) {
            let path = found.get(1).map_or("", |m| m.as_str());
            // Someone else's CDN is not this theme's asset to version.
            if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("//") {
                continue;
            }
            let line = file.content[..found.get(0).expect("whole match").start()].lines().count().max(1);
            unhelped.push(format!("{}:{line}: {path}", file.name));
        }
    }
    add_rule(report, "GS030-ASSET-REQ", unhelped);

    // Upstream refuses symlinks outright, and fatally: a theme is untrusted
    // input, and a link is a way to read a file outside the theme directory.
    let mut links: Vec<String> = files.iter().filter(|f| f.symlink).map(|f| f.name.clone()).collect();
    // Directory symlinks are not in the file list — `collect_files` does not
    // descend them — so they are looked for separately rather than silently
    // allowed.
    links.extend(symlinked_dirs(theme_dir));
    links.sort();
    links.dedup();
    add_rule(report, "GS030-ASSET-SYM", links);
}

fn symlinked_dirs(theme_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![theme_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let is_link = entry.file_type().is_ok_and(|kind| kind.is_symlink());
            if is_link && path.is_dir() {
                found.push(
                    path.strip_prefix(theme_dir)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or(name),
                );
            } else if path.is_dir() && !is_link {
                stack.push(path);
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// GS070 — translations
// ---------------------------------------------------------------------------

/// Every `locales/*.json` has to parse, or its translations silently vanish.
fn check_translations(files: &[ThemeFile], report: &mut Report) {
    let failures = files
        .iter()
        .filter(|file| {
            file.extension == ".json" && (file.name.starts_with("locales/") || file.name.contains("/locales/"))
        })
        .filter(|file| serde_json::from_str::<serde_json::Value>(&file.content).is_err())
        .map(|file| format!("{}: unable to parse", file.name))
        .collect();
    add_rule(report, "GS070-VALID-TRANSLATIONS", failures);
}

// ---------------------------------------------------------------------------
// GS051 — custom fonts
// ---------------------------------------------------------------------------

/// The inverse of every other rule: this one fires when something is *absent*.
///
/// A theme supports Ghost 6's font settings by consuming both
/// `--gh-font-heading` and `--gh-font-body`; one without the other is a theme
/// where changing the font in the admin half works. Upstream expresses "both
/// present" as two conjunctive lookaheads, which Rust's `regex` does not do,
/// so it is two `contains` calls.
fn check_custom_fonts(files: &[ThemeFile], report: &mut Report) {
    let supports = files
        .iter()
        .filter(|file| matches!(file.extension.as_str(), ".css" | ".hbs"))
        .any(|file| {
            file.content.contains("--gh-font-heading") && file.content.contains("--gh-font-body")
        });
    if !supports {
        add_rule(report, "GS051-CUSTOM-FONTS", vec!["styles".into()]);
    }
}

// ---------------------------------------------------------------------------
// GS090 / GS100 / GS110 / GS120 — what the templates reference
// ---------------------------------------------------------------------------

/// The `@`-globals gscan's scope allows, from its `ghostGlobals` and
/// `dataVars`.
///
/// `setting` is on the list and should not be — upstream says so in a comment
/// and keeps it because one published theme still uses it. Kept here for the
/// same reason: dropping it would report a theme Ghost itself accepts.
const KNOWN_GLOBALS: [&str; 16] = [
    // ghostGlobals
    "site", "member", "setting", "config", "labs", "custom", "page",
    // dataVars — block-local in truth, but indistinguishable from a global
    // without tracking which `{{#foreach}}` we are inside.
    "index", "number", "key", "first", "last", "odd", "even", "rowStart", "rowEnd",
];

/// One `@`-prefixed path a template referenced, and where.
struct DataPath {
    /// The segments after the `@`, so `@custom.my_setting` is `[custom, my_setting]`.
    parts: Vec<String>,
    file: String,
    line: usize,
}

/// Every `@…` path in every template, with its position.
///
/// gscan walks a real Handlebars AST for this. The port reads the mustache
/// bodies it already parses, which reaches the same paths for every form a
/// theme actually writes: the path of a statement (`{{@custom.x}}`), an
/// argument to a block (`{{#if @custom.x}}`), a subexpression
/// (`{{#if (match @custom.x "y")}}`) and a hash value (`{{> card x=@custom.y}}`).
fn data_paths(templates: &[&Template]) -> Vec<DataPath> {
    let mut found = Vec::new();
    for template in templates {
        for (index, line) in template.source.lines().enumerate() {
            for body in mustaches(line) {
                for token in body.split(|c: char| c.is_whitespace() || "()\"'=".contains(c)) {
                    let Some(path) = token.strip_prefix('@') else { continue };
                    // `{{@index}}` and `{{@custom.x}}`; `.[0]` indices are
                    // segments too, as upstream's `parts` has them.
                    let parts: Vec<String> = path
                        .trim_end_matches(['.', '}'])
                        .split('.')
                        .filter(|segment| !segment.is_empty())
                        .map(|segment| segment.trim_matches(['[', ']']).to_string())
                        .collect();
                    if parts.is_empty() {
                        continue;
                    }
                    found.push(DataPath { parts, file: template.name.clone(), line: index + 1 });
                }
            }
        }
    }
    found
}

/// Plain (non-`@`) mustache paths, for the helpers that moved rather than the
/// globals that did.
fn plain_paths(templates: &[&Template]) -> Vec<DataPath> {
    let mut found = Vec::new();
    for template in templates {
        for (index, line) in template.source.lines().enumerate() {
            for body in mustaches(line) {
                let trimmed = body.trim().trim_start_matches(['#', '/', '^', '>', '&']).trim();
                let Some(head) = trimmed.split_whitespace().next() else { continue };
                if head.starts_with('@') || head.starts_with('!') {
                    continue;
                }
                let parts: Vec<String> =
                    head.split('.').filter(|s| !s.is_empty()).map(str::to_string).collect();
                if !parts.is_empty() {
                    found.push(DataPath { parts, file: template.name.clone(), line: index + 1 });
                }
            }
        }
    }
    found
}

/// The checks that are "which paths does this theme mention".
///
/// Four of gscan's checks are the same question asked four ways, so they share
/// one walk: the tiers-era `GS090` renames, the unused-setting report
/// (`GS100`), the page-builder settings (`GS110`) and unknown globals
/// (`GS120`).
fn check_references(
    templates: &[Template],
    reachable: &HashSet<String>,
    package: Option<&serde_json::Value>,
    report: &mut Report,
) {
    // Only what Ghost can actually render: an orphan partial is not part of
    // the theme as far as these four checks are concerned.
    let visited: Vec<&Template> =
        templates.iter().filter(|t| reachable.contains(&t.name)).collect();
    let data = data_paths(&visited);
    let plain = plain_paths(&visited);
    let at = |path: &DataPath| format!("{}:{}", path.file, path.line);

    // --- GS090: the paths that were renamed when products became tiers ----
    let mut renamed: HashMap<&str, Vec<String>> = HashMap::new();
    for path in &data {
        let head = path.parts[0].as_str();
        let second = path.parts.get(1).map(String::as_str);
        let code = match (head, second) {
            ("product", _) => Some("GS090-NO-PRODUCT-DATA-HELPER"),
            ("products", _) => Some("GS090-NO-PRODUCTS-DATA-HELPER"),
            ("member", Some("products")) => Some("GS090-NO-MEMBER-PRODUCTS-DATA-HELPER"),
            ("price", Some("currency")) => Some("GS090-NO-PRICE-DATA-CURRENCY-GLOBAL"),
            ("price", Some("monthly" | "yearly")) => Some("GS090-NO-PRICE-DATA-MONTHLY-YEARLY"),
            _ => None,
        };
        if let Some(code) = code {
            renamed.entry(code).or_default().push(at(path));
        }
    }
    for path in &plain {
        if path.parts[0] == "products" {
            renamed.entry("GS090-NO-PRODUCTS-HELPER").or_default().push(at(path));
        }
    }
    for (code, failures) in renamed {
        add_rule(report, code, failures);
    }

    // --- GS090/GS100: the theme's own declared settings -------------------
    let declared: Vec<String> = package
        .and_then(|p| p.get("config"))
        .and_then(|c| c.get("custom"))
        .and_then(serde_json::Value::as_object)
        .map(|custom| custom.keys().cloned().collect())
        .unwrap_or_default();

    let used: HashSet<&str> = data
        .iter()
        .filter(|path| path.parts[0] == "custom")
        .filter_map(|path| path.parts.get(1).map(String::as_str))
        .collect();

    // A `{{@custom.x}}` the theme never declared renders empty, with nothing
    // in the admin to set it.
    let unknown: Vec<String> = data
        .iter()
        .filter(|path| path.parts[0] == "custom")
        .filter(|path| {
            path.parts.get(1).is_some_and(|name| !declared.contains(name))
        })
        .map(|path| format!("{}: @custom.{}", at(path), path.parts.get(1).cloned().unwrap_or_default()))
        .collect();
    add_rule(report, "GS090-NO-UNKNOWN-CUSTOM-THEME-SETTINGS", unknown);

    // And the reverse: a declared setting no template reads is a control in
    // the admin that changes nothing, which is worse than no control.
    //
    // Only reported for a theme that declares settings at all, as upstream —
    // its `isEnabled` is `!!theme.customSettings`.
    if !declared.is_empty() {
        let unused: Vec<String> = declared
            .iter()
            .filter(|name| !used.contains(name.as_str()))
            .map(|name| format!("package.json: config.custom.{name} is declared but never referenced from a template"))
            .collect();
        add_rule(report, "GS100-NO-UNUSED-CUSTOM-THEME-SETTING", unused);
    }

    // --- GS110: the page-builder settings ---------------------------------
    let page_properties: HashSet<&str> = data
        .iter()
        .filter(|path| path.parts[0] == "page")
        .filter_map(|path| path.parts.get(1).map(String::as_str))
        .collect();

    // Not supporting `{{@page.show_title_and_feature_image}}` means the
    // editor's per-post toggle silently does nothing on this theme.
    let missing: Vec<String> = gscan_rules::PAGE_BUILDER_PROPERTIES
        .iter()
        .filter(|property| !page_properties.contains(**property))
        .map(|property| format!("{}: {{{{@page.{property}}}}} is not used", page_template_ref(templates)))
        .collect();
    add_rule(report, "GS110-NO-MISSING-PAGE-BUILDER-USAGE", missing);

    let unknown_page: Vec<String> = data
        .iter()
        .filter(|path| path.parts[0] == "page")
        .filter(|path| {
            path.parts.len() > 2
                || path
                    .parts
                    .get(1)
                    .is_some_and(|name| !gscan_rules::PAGE_BUILDER_PROPERTIES.contains(&name.as_str()))
        })
        .map(|path| format!("{}: @{}", at(path), path.parts.join(".")))
        .collect();
    add_rule(report, "GS110-NO-UNKNOWN-PAGE-BUILDER-USAGE", unknown_page);

    // --- GS120: globals that are not globals ------------------------------
    let unknown_globals: Vec<String> = data
        .iter()
        .filter(|path| !KNOWN_GLOBALS.contains(&path.parts[0].as_str()))
        .map(|path| format!("{}: @{} is not a known global", at(path), path.parts.join(".")))
        .collect();
    add_rule(report, "GS120-NO-UNKNOWN-GLOBALS", unknown_globals);
}

/// Which template GS110 points at: `page.hbs`, else the first `page-*.hbs`,
/// else `page.hbs` as the name of the file the theme ought to have.
fn page_template_ref(templates: &[Template]) -> String {
    if templates.iter().any(|t| t.name == "page.hbs") {
        return "page.hbs".into();
    }
    templates
        .iter()
        .find(|t| {
            t.name.starts_with("page-") && t.name.ends_with(".hbs") && !t.name.contains('/')
        })
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "page.hbs".into())
}

// ---------------------------------------------------------------------------
// GS130 — layout inheritance
// ---------------------------------------------------------------------------

/// A template must not inherit a layout that leads back to itself.
///
/// Fatal, and for a concrete reason rather than tidiness: rendering follows
/// the chain, so a cycle recurses until the process dies. Catching it at
/// upload is the difference between a refused theme and a site that falls
/// over on its first request.
fn check_layout_recursion(templates: &[Template], report: &mut Report) {
    static LAYOUT: Lazy<regex::Regex> =
        Lazy::new(|| regex::Regex::new(r"\{\{!<\s*([^}\s]+)\s*\}\}").expect("valid pattern"));

    let layout_of: HashMap<&str, String> = templates
        .iter()
        .filter_map(|template| {
            let found = LAYOUT.captures(&template.source)?;
            Some((template.name.as_str(), format!("{}.hbs", found.get(1)?.as_str())))
        })
        .collect();

    let mut failures = Vec::new();
    for template in templates {
        let mut seen = HashSet::new();
        let mut at = template.name.as_str();
        // Walk the chain from this template. Revisiting any file means a
        // cycle, and the template that started the walk is the one reported.
        while let Some(next) = layout_of.get(at) {
            if !seen.insert(at.to_string()) {
                failures.push(template.name.clone());
                break;
            }
            match templates.iter().find(|t| &t.name == next) {
                Some(found) => at = found.name.as_str(),
                None => break,
            }
        }
    }
    failures.sort();
    failures.dedup();
    add_rule(report, "GS130-NO-RECURSIVE-LAYOUT", failures);
}


// ---------------------------------------------------------------------------
// GS010 — package.json
// ---------------------------------------------------------------------------

/// The GS010 rules a v6 check applies.
///
/// Upstream builds this by merging v1 through v6 and then deleting three
/// `ghost-api` rules by name, because in v6 declaring an API version is itself
/// what gets reported — `GS010-PJ-GHOST-API-PRESENT` — rather than omitting
/// one. That list is spelled out here for the same reason it is spelled out
/// there: it is the check's own logic, not data about a rule.
const PACKAGE_RULES: &[&str] = &[
    "GS010-PJ-REQ",
    "GS010-PJ-PARSE",
    "GS010-PJ-NAME-REQ",
    "GS010-PJ-NAME-LC",
    "GS010-PJ-NAME-HY",
    "GS010-PJ-VERSION-REQ",
    "GS010-PJ-VERSION-SEM",
    "GS010-PJ-AUT-EM-REQ",
    "GS010-PJ-AUT-EM-VAL",
    "GS010-PJ-CONF-PPP",
    "GS010-PJ-KEYWORDS",
    "GS010-PJ-GHOST-API-PRESENT",
    "GS010-PJ-GHOST-CARD-ASSETS-NOT-PRESENT",
    "GS010-PJ-CUST-THEME-TOTAL-SETTINGS",
    "GS010-PJ-CUST-THEME-SETTINGS-CASE",
    "GS010-PJ-CUST-THEME-SETTINGS-TYPE",
    "GS010-PJ-CUST-THEME-SETTINGS-GROUP",
    "GS010-PJ-CUST-THEME-SETTINGS-SELECT-OPTIONS",
    "GS010-PJ-CUST-THEME-SETTINGS-SELECT-DEFAULT",
    "GS010-PJ-CUST-THEME-SETTINGS-BOOLEAN-DEFAULT",
    "GS010-PJ-CUST-THEME-SETTINGS-COLOR-DEFAULT",
    "GS010-PJ-CUST-THEME-SETTINGS-IMAGE-DEFAULT",
    "GS010-PJ-CUST-THEME-SETTINGS-DESCRIPTION-LENGTH",
    // The conditional ones are part of the set, as upstream's `_.extend`
    // puts them there; what differs is where they are left out, below.
    "GS010-PJ-CONF-PPP-INT",
    "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-SYNTAX",
    "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-VALUE",
];

/// Rules that only make sense once the thing they refine exists.
///
/// Upstream calls these conditional and omits them from the blanket failure
/// when there is no `package.json` at all: "posts_per_page is not an integer"
/// says nothing about a file that is missing, where `GS010-PJ-CONF-PPP`
/// already says the useful thing.
const PACKAGE_CONDITIONAL_RULES: &[&str] = &[
    "GS010-PJ-CONF-PPP-INT",
    "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-SYNTAX",
    "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-VALUE",
];

/// The rules reported when there is no usable `package.json` to check.
fn unconditional_package_rules() -> impl Iterator<Item = &'static &'static str> {
    PACKAGE_RULES.iter().filter(|code| !PACKAGE_CONDITIONAL_RULES.contains(code))
}

fn check_package_json(theme_dir: &Path, report: &mut Report) {
    let path = theme_dir.join("package.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        // No package.json fails every non-conditional rule, as upstream:
        // none of them can be satisfied, and reporting only "it is missing"
        // would understate what the theme has to add.
        for code in unconditional_package_rules() {
            add_rule(report, code, vec!["package.json".into()]);
        }
        return;
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            // Unparseable is treated the same way, except that the parse
            // failure replaces "it is missing" and carries the reason.
            for code in unconditional_package_rules().filter(|c| ***c != *"GS010-PJ-REQ") {
                let failures = if **code == *"GS010-PJ-PARSE" {
                    vec![format!("package.json: {error}")]
                } else {
                    vec!["package.json".into()]
                };
                add_rule(report, code, failures);
            }
            return;
        }
    };
    check_package_fields(&parsed, report);
}

/// Report a `package.json` rule against the file itself.
fn pj(report: &mut Report, code: &str) {
    add_rule(report, code, vec!["package.json".into()]);
}

fn check_package_fields(package: &serde_json::Value, report: &mut Report) {
    let string = |key: &str| package.get(key).and_then(serde_json::Value::as_str);
    let config = package.get("config");

    match string("name") {
        None => pj(report, "GS010-PJ-NAME-REQ"),
        Some(name) => {
            // The name becomes a directory and a URL segment, so uppercase
            // and spaces are refused rather than quietly normalised.
            if name != name.to_lowercase() {
                pj(report, "GS010-PJ-NAME-LC");
            }
            if !is_hyphenated(name) {
                pj(report, "GS010-PJ-NAME-HY");
            }
        }
    }

    match string("version") {
        None => pj(report, "GS010-PJ-VERSION-REQ"),
        Some(version) => {
            if !is_semver(version) {
                pj(report, "GS010-PJ-VERSION-SEM");
            }
        }
    }

    // `posts_per_page` drives pagination, so it has to be a positive whole
    // number. Upstream reports exactly one of the two: absent, or present and
    // wrong.
    match config.and_then(|c| c.get("posts_per_page")) {
        None | Some(serde_json::Value::Null) => pj(report, "GS010-PJ-CONF-PPP"),
        Some(value) => {
            if !value.as_i64().is_some_and(|n| n >= 1) {
                pj(report, "GS010-PJ-CONF-PPP-INT");
            }
        }
    }

    match package.get("author").and_then(|a| a.get("email")).and_then(serde_json::Value::as_str) {
        None => pj(report, "GS010-PJ-AUT-EM-REQ"),
        Some(email) => {
            if !looks_like_email(email) {
                pj(report, "GS010-PJ-AUT-EM-VAL");
            }
        }
    }

    // Ghost's directory finds themes by this keyword.
    let marked = package
        .get("keywords")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|words| words.iter().any(|w| w.as_str() == Some("ghost-theme")));
    if !marked {
        pj(report, "GS010-PJ-KEYWORDS");
    }

    // In v6 the finding is that an API version is declared at all: the
    // versioned API is gone, so pinning one no longer does anything.
    if package.get("engines").and_then(|e| e.get("ghost-api")).is_some() {
        pj(report, "GS010-PJ-GHOST-API-PRESENT");
    }

    // `card_assets` decides whether Ghost injects its card CSS or the theme
    // supplies it. Saying nothing leaves it ambiguous — and it is the same
    // setting GS050 keys off, which is why a theme is asked to state it.
    let card_assets = config.and_then(|c| c.get("card_assets"));
    let states_card_assets = match card_assets {
        Some(serde_json::Value::Bool(_)) => true,
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Object(map)) => !map.is_empty(),
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        _ => false,
    };
    if !states_card_assets {
        pj(report, "GS010-PJ-GHOST-CARD-ASSETS-NOT-PRESENT");
    }

    if let Some(custom) = config.and_then(|c| c.get("custom")).and_then(serde_json::Value::as_object) {
        check_custom_settings(custom, report);
    }
}

/// The `config.custom` block: Ghost's theme settings, as declared.
///
/// Each of these is a setting that would otherwise reach the admin's theme
/// settings screen malformed — an unknown type renders no control, a select
/// whose default is not among its options opens with nothing chosen.
fn check_custom_settings(
    custom: &serde_json::Map<String, serde_json::Value>,
    report: &mut Report,
) {
    const TYPES: [&str; 5] = ["select", "boolean", "color", "image", "text"];
    const GROUPS: [&str; 2] = ["post", "homepage"];

    if custom.len() > 20 {
        pj(report, "GS010-PJ-CUST-THEME-TOTAL-SETTINGS");
    }
    if custom.keys().any(|key| !is_snake_case(key)) {
        pj(report, "GS010-PJ-CUST-THEME-SETTINGS-CASE");
    }
    if custom
        .values()
        .any(|entry| !entry.get("type").and_then(serde_json::Value::as_str).is_some_and(|t| TYPES.contains(&t)))
    {
        pj(report, "GS010-PJ-CUST-THEME-SETTINGS-TYPE");
    }
    // `group` is optional, so only a stated one is checked.
    if custom.values().any(|entry| {
        entry.get("group").is_some_and(|g| !g.as_str().is_some_and(|g| GROUPS.contains(&g)))
    }) {
        pj(report, "GS010-PJ-CUST-THEME-SETTINGS-GROUP");
    }

    for entry in custom.values() {
        let default = entry.get("default");
        match entry.get("type").and_then(serde_json::Value::as_str) {
            Some("select") => match entry.get("options").and_then(serde_json::Value::as_array) {
                // One option is not a choice, so upstream wants at least two.
                Some(options) if options.len() >= 2 => {
                    if !options.iter().any(|option| Some(option) == default) {
                        pj(report, "GS010-PJ-CUST-THEME-SETTINGS-SELECT-DEFAULT");
                    }
                }
                _ => pj(report, "GS010-PJ-CUST-THEME-SETTINGS-SELECT-OPTIONS"),
            },
            Some("boolean") => {
                if !matches!(default, Some(serde_json::Value::Bool(_))) {
                    pj(report, "GS010-PJ-CUST-THEME-SETTINGS-BOOLEAN-DEFAULT");
                }
            }
            Some("color") => {
                if !default.and_then(serde_json::Value::as_str).is_some_and(is_hex_colour) {
                    pj(report, "GS010-PJ-CUST-THEME-SETTINGS-COLOR-DEFAULT");
                }
            }
            // An image has nowhere to default to: the theme does not ship the
            // file, so any value here points at something that is not there.
            Some("image") => {
                if default.is_some_and(|d| !d.is_null()) {
                    pj(report, "GS010-PJ-CUST-THEME-SETTINGS-IMAGE-DEFAULT");
                }
            }
            _ => {}
        }

        if entry
            .get("description")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|d| d.chars().count() > 100)
        {
            pj(report, "GS010-PJ-CUST-THEME-SETTINGS-DESCRIPTION-LENGTH");
        }

        // `visibility` is an NQL filter deciding when a setting is shown, and
        // it may only mention other custom settings. A typo here hides a
        // control with no sign of why.
        if let Some(visibility) = entry.get("visibility").and_then(serde_json::Value::as_str) {
            if ghost_nql::parse(visibility).is_err() {
                pj(report, "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-SYNTAX");
            }
            let unknown = referenced_keys(visibility)
                .into_iter()
                .any(|key| !custom.contains_key(&key));
            if unknown {
                pj(report, "GS010-PJ-CUST-THEME-SETTINGS-VISIBILITY-VALUE");
            }
        }
    }
}

/// The setting names an NQL visibility filter mentions — the `key:` parts.
fn referenced_keys(visibility: &str) -> Vec<String> {
    static KEY: Lazy<regex::Regex> =
        Lazy::new(|| regex::Regex::new(r"[a-zA-Z_][a-zA-Z0-9_.]+:").expect("valid pattern"));
    KEY.find_iter(visibility)
        .map(|found| found.as_str().trim_end_matches(':').to_string())
        .collect()
}

/// `major.minor.patch`, each numeric, with an optional pre-release suffix.
fn is_semver(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Upstream's `^([a-z0-9]+-)*[a-z0-9]+$`, case-insensitively — so the
/// lowercase rule and the hyphenation rule stay separate findings.
fn is_hyphenated(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn is_snake_case(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Enough of an address check to catch the mistakes this rule is for — a
/// missing `@`, a bare domain, a trailing comma — without pretending to
/// implement the spec.
fn looks_like_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else { return false };
    !local.is_empty()
        && !value.contains(char::is_whitespace)
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain.split('.').all(|part| !part.is_empty())
}

// ---------------------------------------------------------------------------
// GS020 — required templates
// ---------------------------------------------------------------------------

fn check_structure(theme_dir: &Path, report: &mut Report) {
    // Both are fatal upstream: Ghost cannot serve a site without them.
    for (code, file) in [("GS020-INDEX-REQ", "index.hbs"), ("GS020-POST-REQ", "post.hbs")] {
        if !theme_dir.join(file).exists() {
            add_rule(report, code, vec![file.to_string()]);
        }
    }
    if !theme_dir.join("default.hbs").exists() {
        add_rule(report, "GS020-DEF-REC", vec!["default.hbs".into()]);
    }
}

// ---------------------------------------------------------------------------
// templates
// ---------------------------------------------------------------------------

struct Template {
    /// Path relative to the theme root, for the failure reference.
    name: String,
    source: String,
}

/// The `.hbs` files, from the list already read.
///
/// This used to walk the directory a second time and read every file again.
/// One walk is both faster and the only version that cannot disagree with
/// itself about what the theme contains.
fn templates_from(files: &[ThemeFile]) -> Vec<Template> {
    files
        .iter()
        .filter(|file| file.extension == ".hbs")
        .map(|file| Template { name: file.name.clone(), source: file.content.clone() })
        .collect()
}

/// The templates a check that walks the theme actually reaches.
///
/// gscan does not scan every `.hbs` for these: it starts at the templates
/// Ghost can route to — everything outside `partials/` — and follows
/// `{{> name}}` references from there, recursively. A partial nothing
/// includes is never visited.
///
/// This is not a detail. A theme whose pages are `.html` with a couple of
/// stray `.hbs` partials reaches *nothing*, so gscan reports its
/// `{{ghost_head}}` as missing even though the text is right there in
/// `partials/ghost_head.hbs` — because Ghost would never render that file.
/// Scanning every file instead made this port disagree with gscan in both
/// directions at once: it stayed quiet about the missing `{{ghost_head}}`,
/// and it reported unknown globals out of partials gscan never looked at.
fn reachable_templates(templates: &[Template]) -> HashSet<String> {
    static PARTIAL: Lazy<regex::Regex> = Lazy::new(|| {
        // `{{> name}}`, `{{#> name}}`, and the `{{> "name"}}` quoted form.
        regex::Regex::new(r#"\{\{#?>\s*['"]?([^}\s'"()]+)"#).expect("valid pattern")
    });

    let mut reached: HashSet<String> = HashSet::new();
    let mut queue: Vec<&Template> =
        templates.iter().filter(|t| !t.name.starts_with("partials/")).collect();
    for entry in &queue {
        reached.insert(entry.name.clone());
    }
    while let Some(template) = queue.pop() {
        for found in PARTIAL.captures_iter(&template.source) {
            let Some(name) = found.get(1) else { continue };
            let target = format!("partials/{}.hbs", name.as_str());
            if !reached.insert(target.clone()) {
                continue;
            }
            if let Some(next) = templates.iter().find(|t| t.name == target) {
                queue.push(next);
            }
        }
    }
    reached
}

fn check_templates(templates: &[Template], reachable: &HashSet<String>, report: &mut Report) {
    let mut compile_failures = Vec::new();
    let mut bad_conditionals = Vec::new();
    let mut limit_all = Vec::new();
    let mut limit_over = Vec::new();
    let mut empty_translations = Vec::new();
    let mut inline_dynamic_partials = Vec::new();
    let mut has_ghost_head = false;
    let mut has_ghost_foot = false;

    for template in templates {
        // GS005 — the template has to compile at all. Through the same
        // preprocessing the renderer applies, or constructs this port
        // rewrites before parsing would be reported as syntax errors.
        let mut hb = handlebars::Handlebars::new();
        let prepared = crate::preprocess::prepare(&template.source);
        if let Err(e) = hb.register_template_string("scan", &prepared) {
            compile_failures.push(format!("{}: {}", template.name, first_line(&e.to_string())));
        }
        // GS040 asks whether *Ghost* will emit these, so an unreachable
        // partial mentioning `{{ghost_head}}` does not count: nothing
        // includes it, so nothing renders it.
        let visited = reachable.contains(&template.name);
        if visited && template.source.contains("ghost_head") {
            has_ghost_head = true;
        }
        if visited && template.source.contains("ghost_foot") {
            has_ghost_foot = true;
        }
        for (index, line) in template.source.lines().enumerate() {
            let at = || format!("{}:{}", template.name, index + 1);
            for body in mustaches(line) {
                let trimmed = body.trim();
                // The GS090 rules walk the same reachable set as GS040;
                // GS005 and GS080 below walk every file, as upstream does.
                if visited {
                    if let Some(rest) = strip_block(trimmed, "if").or_else(|| strip_block(trimmed, "unless")) {
                        if argument_count(rest) > 1 {
                            bad_conditionals.push(at());
                        }
                    }
                }
                if visited && strip_block(trimmed, "get").is_some() {
                    if let Some(limit) = hash_value(trimmed, "limit") {
                        if limit.eq_ignore_ascii_case("all") {
                            limit_all.push(at());
                        } else if limit.parse::<i64>().is_ok_and(|n| n > 100) {
                            limit_over.push(at());
                        }
                    }
                }
                // GS005-NO-INLINE-DYNAMIC-PARTIAL: `{{> (whichPartial)}}`.
                //
                // Fatal, and the distinction is the whole point: the inline
                // form throws and takes the page down when the computed
                // partial does not exist, while the block form
                // `{{#> (whichPartial)}}fallback{{/undefined}}` renders its
                // inner content instead. So `#>` is fine and `>` is not.
                if let Some(target) = trimmed.strip_prefix('>') {
                    if target.trim_start().starts_with('(') {
                        inline_dynamic_partials.push(at());
                    }
                }
                // GS080-NO-EMPTY-TRANSLATIONS: `{{t}}` with nothing in it.
                let is_t = trimmed == "t"
                    || (trimmed.starts_with("t ") && argument_count(trimmed[2..].trim()) == 0);
                if is_t {
                    empty_translations.push(at());
                }
            }
        }
    }

    add_rule(report, "GS005-TPL-ERR", compile_failures);
    add_rule(report, "GS090-NO-INVALID-CONDITIONAL-ARGUMENTS", bad_conditionals);
    add_rule(report, "GS090-NO-LIMIT-ALL-IN-GET-HELPER", limit_all);
    add_rule(report, "GS090-NO-LIMIT-OVER-100-IN-GET-HELPER", limit_over);
    add_rule(report, "GS080-NO-EMPTY-TRANSLATIONS", empty_translations);
    add_rule(report, "GS005-NO-INLINE-DYNAMIC-PARTIAL", inline_dynamic_partials);
    // Reported whether or not the theme has templates, as upstream: a
    // directory with no templates is missing these helpers as surely as one
    // with templates that forgot them, and GS020's separate findings about
    // the missing files do not say anything about `{{ghost_head}}`.
    if !has_ghost_head {
        add_rule(report, "GS040-GH-REQ", vec!["default.hbs".into()]);
    }
    if !has_ghost_foot {
        add_rule(report, "GS040-GF-REQ", vec!["default.hbs".into()]);
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

/// Every `{{…}}` body on a line, without the braces.
///
/// Quote-aware, because a `}}` inside a string argument does not close the
/// mustache — `filter="tags:[{{post.tags}}]"` is one call, not two.
fn mustaches(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] != '{' || chars[i + 1] != '{' {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        while j < chars.len() && chars[j] == '{' {
            j += 1;
        }
        let start = j;
        let mut quote: Option<char> = None;
        let mut end = None;
        while j < chars.len() {
            let c = chars[j];
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '}' && j + 1 < chars.len() && chars[j + 1] == '}' => {
                    end = Some(j);
                    break;
                }
                None => {}
            }
            j += 1;
        }
        match end {
            Some(stop) => {
                let body: String = chars[start..stop].iter().collect();
                found.push(body.trim_end_matches('}').to_string());
                i = stop + 2;
            }
            None => break,
        }
    }
    found
}

/// `{{#if foo}}` → `Some("foo")`, for the named block helper only.
fn strip_block<'a>(body: &'a str, helper: &str) -> Option<&'a str> {
    let rest = body.strip_prefix('#')?.trim_start().strip_prefix(helper)?;
    // `#iframe` must not match `#if`.
    if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    Some(rest.trim())
}

/// Positional arguments in a helper call, ignoring `key=value` hash pairs.
fn argument_count(args: &str) -> usize {
    split_args(args)
        .iter()
        .filter(|a| a.starts_with('(') || !a.contains('='))
        .count()
}

/// Split on whitespace, keeping quoted strings and `(subexpressions)` whole.
fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    for c in args.chars() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                '(' => {
                    depth += 1;
                    current.push(c);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    current.push(c);
                }
                c if c.is_whitespace() && depth == 0 => {
                    if !current.is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                }
                c => current.push(c),
            },
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The value of a `key=value` hash argument, unquoted.
fn hash_value(body: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    split_args(body)
        .into_iter()
        .find_map(|arg| arg.strip_prefix(&prefix).map(|v| v.trim_matches(['"', '\'']).to_string()))
}

// ---------------------------------------------------------------------------
// GS050 — Koenig card styles
// ---------------------------------------------------------------------------

/// The GS050 Koenig card rules, from the generated table.
fn koenig_rules() -> impl Iterator<Item = &'static Rule> {
    gscan_rules::RULES.iter().filter(|rule| rule.code.starts_with("GS050-"))
}

fn check_koenig_css(theme_dir: &Path, report: &mut Report) {
    // A theme that opts into Ghost's card assets is not expected to style the
    // cards itself — Ghost injects that CSS. Upstream skips every tagged rule
    // in that case, which is why Ghost's own Casper reports none of these.
    let card_assets = read_card_assets(theme_dir);

    // gscan searches `.css` **and** `.hbs`: a theme may carry its card styles
    // in an inline <style> block, and that counts.
    let mut haystack = String::new();
    let mut stack = vec![theme_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if matches!(path.extension().and_then(|e| e.to_str()), Some("css") | Some("hbs")) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    haystack.push_str(&text);
                    haystack.push('\n');
                }
            }
        }
    }
    // The class list, its levels and its `cardAsset` tags all come from the
    // generated table now. They used to be a second, hand-transcribed copy of
    // the same 103 rules living in this file — the exact duplication the
    // generator exists to remove.
    for rule in koenig_rules() {
        let Some(class) = rule.class_name else { continue };
        if card_assets.covers(rule.card_asset) {
            continue;
        }
        // Upstream's rule regexes are plain `/\.kg-foo/g` searches with no
        // word boundary, so a longer class containing this one satisfies it.
        // Matched the same way deliberately: a theme gscan passes must pass
        // here, and being stricter than Ghost would report faults that are
        // not faults.
        if haystack.contains(class) {
            continue;
        }
        add_rule(report, rule.code, vec!["styles".into()]);
    }
}

/// A theme's `config.card_assets`, which decides whose job the card CSS is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAssets {
    /// No opt-in: the theme styles every card itself.
    None,
    /// `true` — Ghost supplies every card's CSS.
    All,
    /// `{"include": [...]}` — Ghost supplies these.
    Include(Vec<String>),
    /// `{"exclude": [...]}` — Ghost supplies everything but these.
    Exclude(Vec<String>),
}

impl CardAssets {
    /// Is this card's CSS Ghost's responsibility rather than the theme's?
    fn covers(&self, card: Option<&str>) -> bool {
        // The two untagged rules are never covered: width-wide and width-full
        // are layout the theme owns whatever Ghost injects.
        let Some(card) = card else { return false };
        match self {
            CardAssets::None => false,
            CardAssets::All => true,
            CardAssets::Include(list) => list.iter().any(|c| c == card),
            // Include takes priority upstream; exclude means "all but these".
            CardAssets::Exclude(list) => !list.iter().any(|c| c == card),
        }
    }
}

fn read_card_assets(theme_dir: &Path) -> CardAssets {
    // Ghost 6's default theme config is `{posts_per_page: 5, card_assets: true}`
    // and a theme's own `config` merges *over* it (gscan's
    // `utils/package-json.js`). So card assets are on unless the theme turns
    // them off — which is why a theme that says nothing about them, like
    // Flux, reports no GS050 findings in Ghost. Defaulting the other way
    // reported a hundred faults against every theme that had none.
    let Ok(raw) = std::fs::read_to_string(theme_dir.join("package.json")) else {
        return CardAssets::All;
    };
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return CardAssets::All;
    };
    let Some(value) = pkg.pointer("/config/card_assets") else { return CardAssets::All };
    if value == &serde_json::Value::Bool(false) {
        // An explicit opt-out: the theme styles every card itself.
        return CardAssets::None;
    }
    if value == &serde_json::Value::Bool(true) {
        return CardAssets::All;
    }
    let list = |key: &str| -> Option<Vec<String>> {
        let items = value.get(key)?.as_array()?;
        Some(items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    };
    // Upstream gives `include` priority over `exclude`.
    if let Some(include) = list("include") {
        return CardAssets::Include(include);
    }
    if let Some(exclude) = list("exclude") {
        return CardAssets::Exclude(exclude);
    }
    CardAssets::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn theme(files: &[(&str, &str)]) -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        for (name, body) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        dir
    }

    /// A theme with nothing wrong with it, as a baseline for the rest.
    ///
    /// Every field here earns its place by silencing one rule, which is why
    /// it is longer than it looks like it needs to be: `keywords` for
    /// GS010-PJ-KEYWORDS, `author.email` for GS010-PJ-AUT-EM-REQ,
    /// `posts_per_page` for GS010-PJ-CONF-PPP, `card_assets` for
    /// GS010-PJ-GHOST-CARD-ASSETS-NOT-PRESENT, the two font properties for
    /// GS051, and `{{@page.show_title_and_feature_image}}` for GS110. The
    /// same files are checked against the real gscan in
    /// `tests/gscan_parity.rs`, so this baseline cannot quietly become one
    /// only this port considers clean.
    fn good() -> Vec<(&'static str, &'static str)> {
        vec![
            // `card_assets: true` and the two untagged width classes are
            // what every current Ghost theme ships, Casper included. No
            // `engines.ghost-api`: in v6 declaring one is itself a finding.
            ("package.json", r#"{
                "name": "my-theme",
                "version": "1.0.0",
                "keywords": ["ghost-theme"],
                "author": {"email": "themes@example.com"},
                "config": {"posts_per_page": 5, "card_assets": true}
            }"#),
            ("index.hbs", "{{#foreach posts}}{{title}}{{/foreach}}"),
            ("post.hbs", "{{content}}"),
            ("page.hbs", "{{#if @page.show_title_and_feature_image}}{{title}}{{/if}}"),
            ("default.hbs", "<head>{{ghost_head}}</head><body>{{{body}}}{{ghost_foot}}</body>"),
            ("assets/screen.css", ":root{--gh-font-heading:serif;--gh-font-body:sans-serif}\
                                   .kg-width-wide{width:100%}.kg-width-full{width:100vw}"),
        ]
    }

    fn codes(report: &Report) -> Vec<&str> {
        report.findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn a_sound_theme_reports_nothing() {
        let dir = theme(&good());
        let report = scan(dir.path());
        assert_eq!(codes(&report), Vec::<&str>::new(), "{:#?}", report.findings);
        assert!(!report.is_fatal());
    }

    #[test]
    fn missing_required_templates_are_fatal() {
        let dir = theme(&[("package.json", r#"{"name":"t","version":"1.0.0"}"#)]);
        let report = scan(dir.path());
        assert!(codes(&report).contains(&"GS020-INDEX-REQ"));
        assert!(codes(&report).contains(&"GS020-POST-REQ"));
        // Activation has to be refused: Ghost cannot serve a site without them.
        assert!(report.is_fatal());
        assert!(report.fatal_message().unwrap().contains("index.hbs"));
    }

    #[test]
    fn a_missing_default_layout_is_only_a_recommendation() {
        let mut files = good();
        files.retain(|(n, _)| *n != "default.hbs");
        let dir = theme(&files);
        let report = scan(dir.path());
        assert!(codes(&report).contains(&"GS020-DEF-REC"));
        assert!(!report.is_fatal(), "a theme without default.hbs still works");
    }

    #[test]
    fn package_metadata_is_checked_the_way_ghost_checks_it() {
        for (pkg, expected) in [
            (r#"{"version":"1.0.0"}"#, "GS010-PJ-NAME-REQ"),
            (r#"{"name":"t"}"#, "GS010-PJ-VERSION-REQ"),
            (r#"{"name":"My Theme","version":"1.0.0"}"#, "GS010-PJ-NAME-LC"),
            (r#"{"name":"t","version":"v1"}"#, "GS010-PJ-VERSION-SEM"),
            ("{not json}", "GS010-PJ-PARSE"),
        ] {
            let mut files = good();
            files[0] = ("package.json", pkg);
            let report = scan(theme(&files).path());
            assert!(codes(&report).contains(&expected), "{pkg} should report {expected}");
        }
    }

    /// A missing `package.json` fails every rule about it, not just "it is
    /// missing".
    ///
    /// This test previously asserted the opposite — one finding, and none of
    /// the property rules — which read as the tidier answer. Running the real
    /// gscan settled it: upstream reports all 23 non-conditional GS010 rules,
    /// and that is the more useful report, because it lists what the file has
    /// to contain rather than only that it is absent.
    #[test]
    fn a_missing_package_json_fails_every_rule_about_it() {
        let mut files = good();
        files.retain(|(name, _)| *name != "package.json");
        let report = scan(theme(&files).path());
        let found = codes(&report);
        for expected in ["GS010-PJ-REQ", "GS010-PJ-NAME-REQ", "GS010-PJ-KEYWORDS", "GS010-PJ-AUT-EM-REQ"] {
            assert!(found.contains(&expected), "{expected} should be reported");
        }
        assert_eq!(
            found.iter().filter(|code| code.starts_with("GS010-")).count(),
            unconditional_package_rules().count(),
        );
        // The conditional ones stay quiet: "posts_per_page is not an integer"
        // says nothing about a file that does not exist.
        for conditional in PACKAGE_CONDITIONAL_RULES {
            assert!(!found.contains(conditional), "{conditional} needs a file to check");
        }
    }

    /// Every generated pattern compiles.
    ///
    /// This is not theoretical: the first generated table had 93 of 107
    /// patterns fail, because JavaScript lets a bare `{` stand for itself
    /// and Rust does not — and they failed *silently*, logged and skipped, so
    /// a hundred rules quietly did nothing while the suite stayed green.
    #[test]
    fn every_generated_pattern_compiles() {
        let broken: Vec<&str> = gscan_rules::RULES
            .iter()
            .filter(|rule| rule.regex.is_some_and(|p| regex::Regex::new(p).is_err()))
            .map(|rule| rule.code)
            .collect();
        assert_eq!(broken, Vec::<&str>::new(), "patterns that cannot be compiled");
        // And they are all actually loaded, not merely compilable.
        let with_regex = gscan_rules::RULES.iter().filter(|r| r.regex.is_some()).count();
        assert_eq!(COMPILED.len(), with_regex);
    }

    /// A partial nothing includes is not part of the theme.
    ///
    /// Ghost renders routable templates and whatever they pull in, so that is
    /// the set gscan walks for GS040 and the data-path checks. Scanning every
    /// file instead made this port disagree with gscan in both directions on
    /// a real theme: quiet about a missing `{{ghost_head}}`, and reporting
    /// unknown globals out of files gscan never opened.
    #[test]
    fn an_unreachable_partial_is_not_walked() {
        let mut files = good();
        files.push(("partials/orphan.hbs", "{{@nonsense.thing}}"));
        let report = scan(theme(&files).path());
        assert!(
            !codes(&report).contains(&"GS120-NO-UNKNOWN-GLOBALS"),
            "nothing includes this partial, so Ghost never renders it: {:#?}",
            report.findings
        );

        // Included from a routable template, it is walked.
        let mut included = good();
        included.push(("partials/orphan.hbs", "{{@nonsense.thing}}"));
        for file in included.iter_mut().filter(|(name, _)| *name == "index.hbs") {
            file.1 = "{{> orphan}}";
        }
        assert!(codes(&scan(theme(&included).path())).contains(&"GS120-NO-UNKNOWN-GLOBALS"));
    }

    /// And the same rule decides whether `{{ghost_head}}` counts.
    #[test]
    fn ghost_head_in_an_unreachable_partial_does_not_count() {
        let mut files = good();
        for file in files.iter_mut().filter(|(name, _)| *name == "default.hbs") {
            file.1 = "<head></head><body>{{{body}}}</body>";
        }
        files.push(("partials/head.hbs", "{{ghost_head}}{{ghost_foot}}"));
        // Nothing includes `partials/head.hbs`, so Ghost would emit neither.
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS040-GH-REQ"));
        assert!(codes(&report).contains(&"GS040-GF-REQ"));

        for file in files.iter_mut().filter(|(name, _)| *name == "default.hbs") {
            file.1 = "<head>{{> head}}</head><body>{{{body}}}</body>";
        }
        let report = scan(theme(&files).path());
        assert!(!codes(&report).contains(&"GS040-GH-REQ"), "reached through the partial");
        assert!(!codes(&report).contains(&"GS040-GF-REQ"));
    }

    /// Every rule in the generated table is reachable by some check.
    ///
    /// The table is the full 266-rule spec, and a rule nobody applies is a
    /// rule that silently does nothing. This counts the ones a check can
    /// reach — a regex, the package lists, the Koenig table, or a code-driven
    /// check named here — and refuses to let that number quietly fall.
    #[test]
    fn every_rule_in_the_spec_is_applied_by_some_check() {
        // The checks written out by hand, by the code they report.
        const CODE_DRIVEN: &[&str] = &[
            "GS005-TPL-ERR", "GS005-NO-INLINE-DYNAMIC-PARTIAL",
            "GS020-INDEX-REQ", "GS020-POST-REQ", "GS020-DEF-REC",
            "GS030-ASSET-REQ", "GS030-ASSET-SYM",
            "GS040-GH-REQ", "GS040-GF-REQ",
            "GS051-CUSTOM-FONTS",
            "GS070-VALID-TRANSLATIONS",
            "GS080-NO-EMPTY-TRANSLATIONS",
            "GS090-NO-INVALID-CONDITIONAL-ARGUMENTS",
            "GS090-NO-LIMIT-ALL-IN-GET-HELPER",
            "GS090-NO-LIMIT-OVER-100-IN-GET-HELPER",
            "GS090-NO-PRODUCT-DATA-HELPER", "GS090-NO-PRODUCTS-DATA-HELPER",
            "GS090-NO-PRODUCTS-HELPER", "GS090-NO-MEMBER-PRODUCTS-DATA-HELPER",
            "GS090-NO-PRICE-DATA-CURRENCY-GLOBAL", "GS090-NO-PRICE-DATA-MONTHLY-YEARLY",
            "GS090-NO-UNKNOWN-CUSTOM-THEME-SETTINGS",
            "GS100-NO-UNUSED-CUSTOM-THEME-SETTING",
            "GS110-NO-MISSING-PAGE-BUILDER-USAGE", "GS110-NO-UNKNOWN-PAGE-BUILDER-USAGE",
            "GS120-NO-UNKNOWN-GLOBALS",
            "GS130-NO-RECURSIVE-LAYOUT",
        ];
        // What is knowingly not reached, each for a stated reason. These need
        // a real Handlebars scope — which block a path sits inside — rather
        // than the path itself, which is all the mustache scan sees.
        const NEEDS_SCOPE: &[&str] = &[
            // "{{author}} while rendering a post" is the same text as
            // "{{author}} on an author page", which is legitimate.
            "GS090-NO-AUTHOR-HELPER-IN-POST-CONTEXT",
            // Both depend on being inside a `{{#foreach tiers}}` body.
            "GS090-NO-TIER-BENEFIT-AS-OBJECT",
            "GS090-NO-TIER-PRICE-AS-OBJECT",
            // Needs to know the enclosing context to pick which message.
            "GS090-NO-PRICE-DATA-CURRENCY-CONTEXT",
            // Needs the argument tree of an enclosing conditional.
            "GS090-NO-IMG-URL-IN-CONDITIONALS",
            "GS090-NO-UNKNOWN-CUSTOM-THEME-SELECT-VALUE-IN-MATCH",
            // v6 deletes these three from the package check.
            "GS010-PJ-GHOST-API", "GS010-PJ-GHOST-API-V01", "GS010-PJ-GHOST-API-V2",
        ];

        let mut unreached = Vec::new();
        for rule in gscan_rules::RULES {
            let reached = rule.regex.is_some()
                || rule.code.starts_with("GS050-")
                || PACKAGE_RULES.contains(&rule.code)
                || PACKAGE_CONDITIONAL_RULES.contains(&rule.code)
                || CODE_DRIVEN.contains(&rule.code);
            if !reached && !NEEDS_SCOPE.contains(&rule.code) {
                unreached.push(rule.code);
            }
        }
        assert_eq!(unreached, Vec::<&str>::new(), "rules in the spec that no check applies");
        assert_eq!(gscan_rules::RULES.len(), 266, "the whole v6 spec");
        // Pinned so dropping a check cannot pass by being added to the
        // exemption list without anybody noticing.
        assert_eq!(NEEDS_SCOPE.len(), 9);
    }

    /// The inline form throws and takes the page down; the block form falls
    /// back to its inner content. That difference is why one is fatal.
    #[test]
    fn an_inline_dynamic_partial_is_fatal_but_the_block_form_is_fine() {
        let mut files = good();
        files.push(("dyn.hbs", "{{> (whichPartial)}}"));
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS005-NO-INLINE-DYNAMIC-PARTIAL"));
        assert!(report.is_fatal());

        let mut block = good();
        block.push(("dyn.hbs", "{{#> (whichPartial)}}fallback{{/undefined}}"));
        assert!(!codes(&scan(theme(&block).path())).contains(&"GS005-NO-INLINE-DYNAMIC-PARTIAL"));

        // A named partial is not dynamic and is not this rule's business.
        let mut named = good();
        named.push(("dyn.hbs", "{{> card}}"));
        assert!(!codes(&scan(theme(&named).path())).contains(&"GS005-NO-INLINE-DYNAMIC-PARTIAL"));
    }

    #[test]
    fn deprecated_helpers_are_reported_with_upstreams_wording() {
        let mut files = good();
        files.push(("legacy.hbs", "{{twitter_url}}\n{{author.cover}}\n{{@blog.title}}"));
        let report = scan(theme(&files).path());
        let found = codes(&report);
        for expected in ["GS001-DEPR-TWITTER-URL", "GS001-DEPR-AC", "GS001-DEPR-BLOG"] {
            assert!(found.contains(&expected), "{expected} missing from {found:?}");
        }
        let twitter = report.findings.iter().find(|f| f.code == "GS001-DEPR-TWITTER-URL").unwrap();
        // gscan's own sentence, with its HTML flattened.
        assert_eq!(twitter.rule, r#"Replace {{twitter_url}} with {{social_url type="twitter"}}"#);
        assert_eq!(twitter.failures, vec!["legacy.hbs:1: remove or replace {{twitter_url}}"]);
        assert!(!report.is_fatal(), "a deprecation is a warning or error, never fatal");
    }

    /// The one rule that is legitimate in the very file it would flag.
    #[test]
    fn the_author_block_is_allowed_in_author_hbs() {
        let mut files = good();
        files.push(("author.hbs", "{{#author}}{{name}}{{/author}}"));
        assert!(!codes(&scan(theme(&files).path())).contains(&"GS001-DEPR-AUTHBL"));

        let mut elsewhere = good();
        elsewhere.push(("tag.hbs", "{{#author}}{{name}}{{/author}}"));
        assert!(codes(&scan(theme(&elsewhere).path())).contains(&"GS001-DEPR-AUTHBL"));
    }

    /// A v3 theme is warned about a v3 behaviour change; a current theme,
    /// which never saw the old behaviour, is not.
    #[test]
    fn behaviour_change_warnings_follow_the_declared_api_version() {
        let with_api = |api: &str| {
            let mut files = good();
            let package = format!(
                r#"{{"name":"t","version":"1.0.0","keywords":["ghost-theme"],
                     "author":{{"email":"a@b.com"}},
                     "engines":{{"ghost-api":"{api}"}},
                     "config":{{"posts_per_page":5,"card_assets":true}}}}"#
            );
            files.retain(|(name, _)| *name != "package.json");
            files.push(("feach.hbs", "{{#foreach posts}}{{title}}{{/foreach}}"));
            let dir = theme(&files);
            fs::write(dir.path().join("package.json"), package).unwrap();
            codes(&scan(dir.path())).into_iter().map(String::from).collect::<Vec<_>>()
        };
        assert!(with_api("v3").contains(&"GS080-FEACH-POSTS".to_string()));
        assert!(!with_api("v5").contains(&"GS080-FEACH-POSTS".to_string()));
    }

    #[test]
    fn a_layout_that_inherits_itself_is_fatal() {
        let mut files = good();
        for file in files.iter_mut().filter(|(name, _)| *name == "default.hbs") {
            file.1 = "{{!< default}}<head>{{ghost_head}}</head><body>{{{body}}}{{ghost_foot}}</body>";
        }
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS130-NO-RECURSIVE-LAYOUT"));
        // Rendering follows the chain, so a cycle recurses until the process
        // dies. Refusing the theme is the only safe answer.
        assert!(report.is_fatal());
    }

    #[test]
    fn a_symlink_in_a_theme_is_refused() {
        let dir = theme(&good());
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", dir.path().join("leak.hbs")).unwrap();
        let report = scan(dir.path());
        assert!(codes(&report).contains(&"GS030-ASSET-SYM"));
        // A theme is untrusted input and a link is a way out of its directory.
        assert!(report.is_fatal());
    }

    #[test]
    fn a_hand_written_asset_path_is_reported_but_a_remote_one_is_not() {
        let mut files = good();
        files.push(("a.hbs", concat!(
            "<link href=\"/assets/css/screen.css\">\n",
            "<script src=\"https://cdn.example.com/assets/x.js\"></script>\n",
            "<script src=\"//cdn.example.com/assets/y.js\"></script>\n",
            "<link href=\"{{asset \"css/screen.css\"}}\">",
        )));
        let report = scan(theme(&files).path());
        let finding = report.findings.iter().find(|f| f.code == "GS030-ASSET-REQ").unwrap();
        // Only the local, un-helped path: the other three are either on
        // somebody else's host or already going through {{asset}}.
        assert_eq!(finding.failures, vec!["a.hbs:1: /assets/css/screen.css"]);
    }

    #[test]
    fn unparseable_translations_are_reported_against_their_file() {
        let mut files = good();
        files.push(("locales/en.json", "{ \"broken\": "));
        files.push(("locales/de.json", r#"{"ok": "yes"}"#));
        let report = scan(theme(&files).path());
        let finding = report.findings.iter().find(|f| f.code == "GS070-VALID-TRANSLATIONS").unwrap();
        assert_eq!(finding.failures, vec!["locales/en.json: unable to parse"]);
        // Translations that silently fail to load are worse than none.
        assert!(finding.fatal);
    }

    #[test]
    fn an_undeclared_custom_setting_and_an_unread_one_are_both_reported() {
        let mut files = good();
        files.retain(|(name, _)| *name != "package.json");
        files.push(("package.json", r#"{
            "name": "t", "version": "1.0.0", "keywords": ["ghost-theme"],
            "author": {"email": "a@b.com"},
            "config": {"posts_per_page": 5, "card_assets": true,
                       "custom": {"declared_and_used": {"type": "text"},
                                  "declared_not_used": {"type": "text"}}}
        }"#));
        files.push(("custom.hbs", "{{@custom.declared_and_used}} {{@custom.never_declared}}"));
        let report = scan(theme(&files).path());
        let unknown = report.findings.iter().find(|f| f.code == "GS090-NO-UNKNOWN-CUSTOM-THEME-SETTINGS").unwrap();
        assert_eq!(unknown.failures, vec!["custom.hbs:1: @custom.never_declared"]);
        let unused = report.findings.iter().find(|f| f.code == "GS100-NO-UNUSED-CUSTOM-THEME-SETTING").unwrap();
        assert_eq!(unused.failures.len(), 1);
        assert!(unused.failures[0].contains("declared_not_used"));
    }

    #[test]
    fn an_at_path_that_is_not_a_ghost_global_is_reported() {
        let mut files = good();
        // `@index` and friends look exactly like globals and are allowed;
        // `@nonsense` is not.
        files.push(("g.hbs", "{{#foreach posts}}{{@index}}{{/foreach}} {{@nonsense.thing}}"));
        let report = scan(theme(&files).path());
        let finding = report.findings.iter().find(|f| f.code == "GS120-NO-UNKNOWN-GLOBALS").unwrap();
        assert_eq!(finding.failures, vec!["g.hbs:1: @nonsense.thing is not a known global"]);
    }

    #[test]
    fn the_page_builder_setting_must_be_read_and_only_the_known_one_exists() {
        // The baseline reads it, so it is not reported.
        assert!(!codes(&scan(theme(&good()).path())).contains(&"GS110-NO-MISSING-PAGE-BUILDER-USAGE"));

        let mut files = good();
        for file in files.iter_mut().filter(|(name, _)| *name == "page.hbs") {
            file.1 = "{{title}}{{@page.invented_property}}";
        }
        let report = scan(theme(&files).path());
        let found = codes(&report);
        // The editor's toggle would silently do nothing on this theme.
        assert!(found.contains(&"GS110-NO-MISSING-PAGE-BUILDER-USAGE"));
        assert!(found.contains(&"GS110-NO-UNKNOWN-PAGE-BUILDER-USAGE"));
    }

    /// The two package lists together are exactly the GS010 rules the spec
    /// carries, minus the three `ghost-api` rules v6 deletes.
    ///
    /// Without this, a rule added to the generated table would simply never
    /// run, and nothing would say so.
    #[test]
    fn every_package_rule_is_wired_up() {
        let deleted_in_v6 = ["GS010-PJ-GHOST-API", "GS010-PJ-GHOST-API-V01", "GS010-PJ-GHOST-API-V2"];
        let mut from_spec: Vec<&str> = gscan_rules::RULES
            .iter()
            .map(|rule| rule.code)
            .filter(|code| code.starts_with("GS010-"))
            .filter(|code| !deleted_in_v6.contains(code))
            .collect();
        from_spec.sort();
        let mut wired: Vec<&str> = PACKAGE_RULES.to_vec();
        wired.sort();
        assert_eq!(wired, from_spec, "a GS010 rule is in the spec but no check applies it");
    }

    #[test]
    fn semver_accepts_what_ghost_accepts() {
        assert!(is_semver("1.0.0"));
        assert!(is_semver("0.0.1"));
        assert!(is_semver("10.20.30"));
        assert!(is_semver("1.0.0-beta.1"), "pre-release suffixes are valid semver");
        assert!(!is_semver("1.0"));
        assert!(!is_semver("v1.0.0"));
        assert!(!is_semver("1.0.x"));
        assert!(!is_semver(""));
    }

    #[test]
    fn a_template_that_cannot_compile_is_fatal() {
        let mut files = good();
        files.push(("broken.hbs", "{{#foreach posts}}unclosed"));
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS005-TPL-ERR"));
        assert!(report.is_fatal());
        let finding = report.findings.iter().find(|f| f.code == "GS005-TPL-ERR").unwrap();
        assert!(finding.failures[0].starts_with("broken.hbs"), "{:?}", finding.failures);
    }

    #[test]
    fn two_arguments_to_if_are_fatal() {
        // Ghost 6 made this fatal because `{{#if a b}}` silently ignores the
        // second argument, so the condition is not what the author wrote.
        let mut files = good();
        files.push(("bad.hbs", "{{#if statusCode 404}}x{{/if}}"));
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS090-NO-INVALID-CONDITIONAL-ARGUMENTS"));
        assert!(report.is_fatal());
    }

    #[test]
    fn one_argument_and_hash_arguments_are_fine() {
        let mut files = good();
        files.push(("ok.hbs", "{{#if featured}}x{{/if}}\n{{#unless @member}}y{{/unless}}\n{{#if (match a b)}}z{{/if}}"));
        let report = scan(theme(&files).path());
        assert!(!codes(&report).contains(&"GS090-NO-INVALID-CONDITIONAL-ARGUMENTS"), "{:#?}", report.findings);
    }

    #[test]
    fn a_helper_whose_name_starts_with_if_is_not_an_if() {
        let mut files = good();
        // `#iframely a b` is not `#if`, and must not be reported as one.
        files.push(("ok.hbs", "{{#foreach posts}}{{title}}{{/foreach}}"));
        let report = scan(theme(&files).path());
        assert!(!codes(&report).contains(&"GS090-NO-INVALID-CONDITIONAL-ARGUMENTS"));
        assert_eq!(strip_block("#iframely a b", "if"), None);
        assert_eq!(strip_block("#if featured", "if"), Some("featured"));
    }

    #[test]
    fn get_helper_limits_are_reported_against_the_cap() {
        let mut files = good();
        files.push(("list.hbs", "{{#get \"posts\" limit=\"all\"}}a{{/get}}\n{{#get \"tags\" limit=\"500\"}}b{{/get}}\n{{#get \"posts\" limit=\"5\"}}c{{/get}}"));
        let report = scan(theme(&files).path());
        let all = report.findings.iter().find(|f| f.code == "GS090-NO-LIMIT-ALL-IN-GET-HELPER").unwrap();
        assert_eq!(all.failures, vec!["list.hbs:1"]);
        let over = report.findings.iter().find(|f| f.code == "GS090-NO-LIMIT-OVER-100-IN-GET-HELPER").unwrap();
        assert_eq!(over.failures, vec!["list.hbs:2"], "a limit of 5 is fine");
        // Neither stops the theme working; the cap is applied at render time.
        assert!(!report.is_fatal());
    }

    #[test]
    fn a_missing_ghost_head_or_foot_is_a_warning() {
        let mut files = good();
        // By name, not by index: the baseline gains files over time, and an
        // index silently started pointing at a different one.
        for file in files.iter_mut().filter(|(name, _)| *name == "default.hbs") {
            file.1 = "<head></head><body>{{{body}}}</body>";
        }
        let report = scan(theme(&files).path());
        assert!(codes(&report).contains(&"GS040-GH-REQ"));
        assert!(codes(&report).contains(&"GS040-GF-REQ"));
        assert!(!report.is_fatal(), "a site without them renders, just without Portal or SEO tags");
    }

    #[test]
    fn an_empty_translation_helper_is_reported() {
        let mut files = good();
        files.push(("t.hbs", "{{t}}\n{{t \"Subscribe\"}}"));
        let report = scan(theme(&files).path());
        let finding = report.findings.iter().find(|f| f.code == "GS080-NO-EMPTY-TRANSLATIONS").unwrap();
        assert_eq!(finding.failures, vec!["t.hbs:1"], "the one with a string is fine");
    }

    #[test]
    fn a_theme_that_styles_no_cards_and_opts_out_is_told_about_all_of_them() {
        // Only with an explicit `card_assets: false`. Upstream then reports
        // every GS050 rule the theme neither styles nor hands to Ghost.
        let files = vec![
            ("package.json",
             r#"{"name":"t","version":"1.0.0","config":{"card_assets":false}}"#),
            ("index.hbs", "x"),
            ("post.hbs", "x"),
            ("default.hbs", "{{ghost_head}}{{ghost_foot}}"),
        ];
        let report = scan(theme(&files).path());
        let gs050 = codes(&report).into_iter().filter(|c| c.starts_with("GS050")).count();
        assert_eq!(gs050, koenig_rules().count());
    }

    #[test]
    fn an_unstyled_koenig_card_is_reported_against_its_own_rule() {
        // An explicit opt-out, so the theme is expected to style them itself.
        let mut files = good();
        files[0] = ("package.json",
            r#"{"name":"t","version":"1.0.0","config":{"card_assets":false}}"#);
        files.retain(|(n, _)| *n != "assets/screen.css");
        files.push(("assets/screen.css", ".kg-width-wide { width: 100% }"));
        let report = scan(theme(&files).path());
        assert!(!codes(&report).contains(&"GS050-CSS-KGWW"), "this one is styled");
        assert!(codes(&report).contains(&"GS050-CSS-KGWF"), "this one is not");
        assert!(codes(&report).contains(&"GS050-CSS-KGGC"), "and the gallery card is not");
    }

    #[test]
    fn opting_into_ghosts_card_assets_skips_the_cards_it_supplies() {
        // Ghost's own Casper sets `card_assets: true` and gscan reports no
        // GS050 findings for it, because Ghost injects that CSS. Reporting
        // them would fail every official theme.
        let mut files = good();
        files[0] = ("package.json",
            r#"{"name":"t","version":"1.0.0","config":{"card_assets":true}}"#);
        files.retain(|(n, _)| *n != "assets/screen.css");
        files.push(("assets/screen.css", "body { margin: 0 }"));
        let report = scan(theme(&files).path());
        let gs050: Vec<&str> = codes(&report).into_iter().filter(|c| c.starts_with("GS050")).collect();
        // The two untagged rules still apply: they are layout the theme owns.
        assert_eq!(gs050, vec!["GS050-CSS-KGWF", "GS050-CSS-KGWW"], "{gs050:?}");
    }

    #[test]
    fn an_include_list_covers_only_the_cards_it_names() {
        assert!(CardAssets::All.covers(Some("gallery")));
        assert!(!CardAssets::All.covers(None), "untagged rules are never covered");
        assert!(CardAssets::Include(vec!["gallery".into()]).covers(Some("gallery")));
        assert!(!CardAssets::Include(vec!["gallery".into()]).covers(Some("bookmark")));
        // exclude means "Ghost supplies everything but these"
        assert!(!CardAssets::Exclude(vec!["gallery".into()]).covers(Some("gallery")));
        assert!(CardAssets::Exclude(vec!["gallery".into()]).covers(Some("bookmark")));
        assert!(!CardAssets::None.covers(Some("gallery")));
        // Absent config means Ghost supplies them — the Ghost 6 default.
        assert_eq!(read_card_assets(Path::new("/nonexistent")), CardAssets::All);
    }

    #[test]
    fn card_styles_may_live_in_a_template_as_well_as_a_stylesheet() {
        // gscan searches .hbs too, for themes with an inline <style> block.
        let mut files = good();
        // Drop the stylesheet; the same classes live in a template instead.
        files.retain(|(n, _)| *n != "assets/screen.css");
        files.push(("styles.hbs", "<style>.kg-width-wide{width:100%}.kg-width-full{width:100vw}</style>"));
        let report = scan(theme(&files).path());
        assert!(!codes(&report).iter().any(|c| c.starts_with("GS050")), "{:#?}", report.findings);
    }

    #[test]
    fn a_closing_brace_inside_a_string_does_not_end_the_mustache() {
        // `filter="tags:[{{post.tags}}]"` is one call, not two.
        let found = mustaches(r#"{{#get "posts" filter="tags:[{{post.tags}}]"}}"#);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("filter="));
    }

    #[test]
    fn triple_stash_helpers_are_read_too() {
        assert_eq!(mustaches("{{{ghost_head}}}"), vec!["ghost_head"]);
    }

    #[test]
    fn hash_values_are_read_unquoted() {
        assert_eq!(hash_value(r#"#get "posts" limit="all""#, "limit").as_deref(), Some("all"));
        assert_eq!(hash_value("#get \"posts\" limit=5", "limit").as_deref(), Some("5"));
        assert_eq!(hash_value("#get \"posts\"", "limit"), None);
    }

    #[test]
    fn findings_come_back_worst_first() {
        let dir = theme(&[("package.json", "{}")]);
        let report = scan(dir.path());
        let levels: Vec<Level> = report.findings.iter().map(|f| f.level).collect();
        let mut sorted = levels.clone();
        sorted.sort();
        assert_eq!(levels, sorted, "errors before warnings before recommendations");
    }

    /// Generated from gscan 6.6.1; a drift here is a drift from Ghost.
    ///
    /// These were hand-transcribed tables until this batch, including a
    /// 103-entry copy of the Koenig card list sitting in this file beside the
    /// generated one. The counts are still pinned, because the point of
    /// generating them is that they match the real spec, and a regeneration
    /// against a different gscan should be a visible decision.
    #[test]
    fn the_rule_tables_match_the_gscan_spec_they_were_taken_from() {
        assert_eq!(koenig_rules().count(), 103, "GS050-CSS-* rule count");
        assert_eq!(KNOWN_HELPERS.len(), 69, "v6 knownHelpers count");
        assert!(KNOWN_HELPERS.contains(&"social_accounts"), "added in v6");
        assert!(KNOWN_HELPERS.contains(&"foreach"));
        let by_code = |code: &str| koenig_rules().find(|rule| rule.code == code).expect(code);
        // The two untagged rules apply to every theme; the rest are skipped
        // for a theme using Ghost's card assets.
        assert_eq!(by_code("GS050-CSS-KGWW").level, Level::Error);
        assert_eq!(by_code("GS050-CSS-KGWW").card_asset, None);
        assert_eq!(by_code("GS050-CSS-KGCO").level, Level::Warning);
        assert_eq!(by_code("GS050-CSS-KGAUD").card_asset, Some("audio"));
        assert_eq!(
            koenig_rules().filter(|rule| rule.card_asset.is_none()).count(),
            2,
            "only .kg-width-wide and .kg-width-full are untagged"
        );
    }

    /// A throwaway directory that removes itself.
    mod tempdir {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let base = std::env::temp_dir().join(format!(
                    "gscan-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir_all(&base).unwrap();
                Dir(base)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
