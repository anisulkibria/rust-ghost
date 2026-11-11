//! The port's GScan against the real one.
//!
//! The unit tests in `gscan.rs` say what this port reports. They cannot say
//! whether Ghost agrees, and that is the only claim that matters for a theme
//! an author will also upload to a real Ghost. So this builds themes on disk,
//! runs both tools over them, and compares code for code and level for level.
//!
//! Skipped — not failed — when the real gscan is not installed beside this
//! checkout. A contributor without a Ghost source tree should not see a red
//! suite, but anybody who has one gets the comparison for free. `GSCAN_PATH`
//! overrides the search.
//!
//! When this disagrees, the real tool is right by definition. Three findings
//! in this port started as "obviously the tidier behaviour" and turned out to
//! be wrong: a missing `package.json` fails every rule about it rather than
//! one, `{{ghost_head}}` is reported even for a theme with no templates at
//! all, and gscan's v6 spec carries every deprecation rule from v1 onwards
//! rather than retiring them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the real gscan lives, if it does.
///
/// pnpm's store path carries a content hash, so it is globbed rather than
/// written out — pinning the hash would break on the next Ghost update and
/// read as a test failure rather than as a moved dependency.
/// The reference Node Ghost installation these parity checks compare
/// against, if this machine has one. It is named by `GHOST_REFERENCE_DIR`
/// (the install root; `current/` and `content/themes` live under it) rather
/// than a path in the source, so the checkout works wherever it is placed and
/// the test simply skips where no reference install exists.
fn reference_ghost() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("GHOST_REFERENCE_DIR")?);
    let current = root.join("current");
    Some(if current.is_dir() { current } else { root })
}

fn find_gscan() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("GSCAN_PATH") {
        let path = PathBuf::from(from_env);
        return path.join("lib/specs/v6.js").exists().then_some(path);
    }
    let ghost = reference_ghost()?;
    let direct = ghost.join("node_modules/gscan");
    if direct.join("lib/specs/v6.js").exists() {
        return Some(direct);
    }
    let store = ghost.join("node_modules/.pnpm");
    let entries = std::fs::read_dir(store).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path().join("node_modules/gscan"))
        .find(|path| path.join("lib/specs/v6.js").exists())
}

fn node_available() -> bool {
    Command::new("node").arg("--version").output().is_ok_and(|out| out.status.success())
}

/// `level code` per finding, sorted — what both tools are compared on.
fn real_gscan(gscan: &Path, theme: &Path) -> Vec<String> {
    let script = r#"
        const gscan = require(process.argv[1]);
        const spec = require(process.argv[1] + '/lib/specs').get(['v6']);
        gscan.check(process.argv[2], {checkVersion: 'v6'}).then((theme) => {
            const out = Object.keys(theme.results.fail || {}).map((code) => {
                const rule = spec.rules[code] || {};
                return `${rule.level} ${code}`;
            });
            console.log(out.sort().join('\n'));
        }).catch((error) => {
            console.error('GSCAN_FAILED ' + error.message);
            process.exit(3);
        });
    "#;
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(gscan)
        .arg(theme)
        .output()
        .expect("node runs");
    assert!(
        output.status.success(),
        "the real gscan could not read the fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn ported_gscan(theme: &Path) -> Vec<String> {
    let mut found: Vec<String> = ghost_render::gscan::scan(theme)
        .findings
        .iter()
        .map(|finding| format!("{} {}", finding.level.label(), finding.code))
        .collect();
    found.sort();
    found
}

/// A throwaway directory holding one theme.
struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let base = std::env::temp_dir().join(format!(
            "gscan-parity-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        for (file, body) in files {
            let path = base.join(file);
            std::fs::create_dir_all(path.parent().expect("a parent")).unwrap();
            std::fs::write(path, body).unwrap();
        }
        Fixture(base)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compare(name: &str, files: &[(&str, &str)]) {
    let Some(gscan) = find_gscan() else {
        eprintln!("skipping {name}: the real gscan is not installed (set GSCAN_PATH)");
        return;
    };
    if !node_available() {
        eprintln!("skipping {name}: node is not on PATH");
        return;
    }
    let fixture = Fixture::new(name, files);
    let real = real_gscan(&gscan, fixture.path());
    let ported = ported_gscan(fixture.path());
    assert_eq!(
        ported, real,
        "\n{name}: the port and the real gscan disagree.\n  only in the port: {:?}\n  only in gscan:    {:?}\n",
        ported.iter().filter(|f| !real.contains(f)).collect::<Vec<_>>(),
        real.iter().filter(|f| !ported.contains(f)).collect::<Vec<_>>(),
    );
    assert!(!real.is_empty() || name == "clean", "{name} was expected to report something");
}

/// Every field here silences exactly one rule, which is what makes it a
/// useful baseline: anything either tool reports is a disagreement.
fn clean_theme() -> Vec<(&'static str, &'static str)> {
    vec![
        ("package.json", r#"{
            "name": "parity-theme",
            "version": "1.0.0",
            "keywords": ["ghost-theme"],
            "author": {"email": "themes@example.com"},
            "config": {"posts_per_page": 5, "card_assets": true}
        }"#),
        ("index.hbs", "{{!< default}}{{#foreach posts}}{{title}}{{/foreach}}"),
        ("post.hbs", "{{!< default}}{{content}}"),
        ("page.hbs", "{{!< default}}{{#if @page.show_title_and_feature_image}}{{title}}{{/if}}"),
        ("default.hbs", "<html><head>{{ghost_head}}</head><body>{{{body}}}{{ghost_foot}}</body></html>"),
        ("assets/built/screen.css", ":root{--gh-font-heading:serif;--gh-font-body:sans-serif}\
                                     .kg-width-wide{width:100%}.kg-width-full{width:100vw}"),
    ]
}

#[test]
fn a_clean_theme_is_clean_in_both() {
    compare("clean", &clean_theme());
}

/// The catalogue this batch added: helpers and data paths removed across
/// Ghost 1.x–5.x, which a v6 check still reports.
#[test]
fn the_legacy_deprecation_catalogue_agrees() {
    let mut files = clean_theme();
    files.push(("legacy.hbs", concat!(
        "{{@blog.title}} {{@blog.cover}} {{@blog.posts_per_page}}\n",
        "{{meta_description}} {{image}} {{cover}} {{pageUrl}} {{lang}}\n",
        "{{author}} {{author.name}} {{author.cover}} {{author.image}} {{author.twitter}}\n",
        "{{post.author}} {{post.author.name}} {{post.author.cover}} {{post.author_id}}\n",
        "{{primary_author.cover}} {{authors.[0].image}} {{tag.image}} {{tags.[0].image}}\n",
        "{{#if image}}a{{/if}} {{#if cover}}b{{/if}} {{#if author.cover}}c{{/if}}\n",
        "{{#if tag.image}}d{{/if}} {{#if tags.[0].image}}e{{/if}}\n",
        "{{twitter_url}} {{facebook_url}} {{error.code}}\n",
        "{{@site.lang}} {{@site.permalinks}} {{@site.ghost_head}} {{@labs.members}}\n",
        "{{content words=\"0\"}}\n",
        "{{#get \"users\"}}{{/get}}\n",
        "{{#get \"posts\" include=\"author\"}}{{/get}}\n",
    )));
    compare("legacy", &files);
}

/// The tiers rename, the page-builder settings and unknown globals — the
/// checks driven by which data paths a template reads.
#[test]
fn the_data_path_checks_agree() {
    let mut files = clean_theme();
    files.push(("paths.hbs", concat!(
        "{{products}} {{@product}} {{@products}} {{@member.products}}\n",
        "{{@price.currency}} {{@price.monthly}} {{@price.yearly}}\n",
        "{{@nonsense.thing}} {{@page.invented}}\n",
        "{{@custom.never_declared}}\n",
    )));
    compare("paths", &files);
}

/// The structural rules, including the two fatal ones.
#[test]
fn the_structural_rules_agree() {
    let mut files = clean_theme();
    files.push(("amp.hbs", "<html amp><body>amp</body></html>"));
    files.push(("locales/en.json", "{ \"unterminated\": "));
    files.push(("bad.hbs", "<link href=\"/assets/built/screen.css\">"));
    // default.hbs inheriting itself: the cycle gscan refuses outright.
    for file in files.iter_mut().filter(|(name, _)| *name == "default.hbs") {
        file.1 = "{{!< default}}<html><head>{{ghost_head}}</head><body>{{{body}}}{{ghost_foot}}</body></html>";
    }
    compare("structural", &files);
}

/// Everything a theme can get wrong in `package.json`, in one file.
#[test]
fn the_package_metadata_rules_agree() {
    let mut files = clean_theme();
    for file in files.iter_mut().filter(|(name, _)| *name == "package.json") {
        file.1 = r#"{
            "name": "Bad_Name Theme",
            "version": "1.0",
            "keywords": ["blog"],
            "author": {"email": "not-an-email"},
            "engines": {"ghost-api": "v2"},
            "config": {
                "posts_per_page": 0,
                "custom": {
                    "BadCase": {"type": "select", "options": ["only-one"], "default": "absent"},
                    "bad_colour": {"type": "color", "default": "red"},
                    "bad_boolean": {"type": "boolean", "default": "yes"},
                    "bad_image": {"type": "image", "default": "/img.png"},
                    "bad_group": {"type": "text", "group": "sidebar"},
                    "bad_type": {"type": "invented"}
                }
            }
        }"#;
    }
    compare("package", &files);
}

/// A theme with no `package.json` at all.
///
/// This is the case that corrected a wrong assumption in this port: it fails
/// every rule about the file, not just "it is missing".
#[test]
fn a_theme_with_no_package_json_agrees() {
    let files: Vec<_> =
        clean_theme().into_iter().filter(|(name, _)| *name != "package.json").collect();
    compare("nopackage", &files);
}

/// Whatever themes this machine happens to have, as the broadest check
/// available: real themes, written by other people, not shaped to this port's
/// expectations.
#[test]
fn the_installed_themes_agree() {
    let Some(gscan) = find_gscan() else {
        eprintln!("skipping: the real gscan is not installed (set GSCAN_PATH)");
        return;
    };
    if !node_available() {
        eprintln!("skipping: node is not on PATH");
        return;
    }
    let Some(themes) = reference_ghost().map(|ghost| ghost.join("content/themes")) else {
        eprintln!("skipping: GHOST_REFERENCE_DIR is not set");
        return;
    };
    let Ok(entries) = std::fs::read_dir(themes) else {
        eprintln!("skipping: no theme directory on this machine");
        return;
    };
    let mut checked = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        // A theme is a directory; `.zip` and stray files are not.
        if !path.is_dir() || path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        let real = real_gscan(&gscan, &path);
        let ported = ported_gscan(&path);
        assert_eq!(
            ported,
            real,
            "\n{}: the port and the real gscan disagree.\n  only in the port: {:?}\n  only in gscan:    {:?}\n",
            path.display(),
            ported.iter().filter(|f| !real.contains(f)).collect::<Vec<_>>(),
            real.iter().filter(|f| !ported.contains(f)).collect::<Vec<_>>(),
        );
        checked += 1;
    }
    eprintln!("compared {checked} installed themes against the real gscan");
}
