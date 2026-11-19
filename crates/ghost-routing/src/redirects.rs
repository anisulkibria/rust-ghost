//! redirects.yaml — site-level 301/302 redirect rules.
use ghost_core::{GhostError, Result};
use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RedirectRule {
    pub from: Regex,
    pub from_raw: String,
    pub to: String,
    pub permanent: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Redirects {
    rules: Vec<RedirectRule>,
}

/// Parse redirects.yaml. Two shapes are read:
///
/// Ghost's YAML, grouped by status:
/// ```yaml
/// 301:
///   /old/: /new/
/// 302:
///   /temp/: /other/
/// ```
/// and Ghost's `redirects.json` shape, a list — which is also valid YAML, and
/// is the only one of the two that can keep a 302 ahead of a 301:
/// ```yaml
/// - from: /old/
///   to: /new/
///   permanent: true
/// ```
/// `from` is a regular expression matched against the whole path (with or
/// without its trailing slash); `/pattern/i` adds flags. `to` supports
/// `$1`..`$9` captures. Rules are tried in file order and the first match wins.
pub fn parse_redirects_yaml(yaml: &str) -> Result<Redirects> {
    if yaml.trim().is_empty() {
        return Ok(Redirects::default());
    }
    let doc: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| GhostError::BadRequest(format!("redirects.yaml: {e}")))?;
    let mut rules = Vec::new();

    match doc {
        serde_yaml::Value::Mapping(m) => {
            for (code, entries) in m {
                let status = code.as_u64().or_else(|| code.as_str().and_then(|s| s.parse().ok())).unwrap_or(301);
                let permanent = status == 301;
                if let serde_yaml::Value::Mapping(entries) = entries {
                    for (from, to) in entries {
                        let (Some(from), Some(to)) = (from.as_str(), to.as_str()) else {
                            continue;
                        };
                        rules.push(rule(from, to, permanent));
                    }
                }
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                let get = |k: &str| item.get(k).and_then(|v| v.as_str()).map(str::to_string);
                let (Some(from), Some(to)) = (get("from"), get("to")) else { continue };
                let permanent = match (item.get("permanent"), item.get("status")) {
                    (Some(p), _) => p.as_bool().unwrap_or(true),
                    (None, Some(s)) => s.as_u64().map(|s| s == 301).unwrap_or(true),
                    (None, None) => false,
                };
                rules.push(rule(&from, &to, permanent));
            }
        }
        serde_yaml::Value::Null => {}
        _ => return Err(GhostError::BadRequest("redirects.yaml: expected 301/302 groups or a list of rules".into())),
    }
    Ok(Redirects { rules })
}

fn rule(from: &str, to: &str, permanent: bool) -> RedirectRule {
    RedirectRule { from: compile_from(from), from_raw: from.to_string(), to: to.to_string(), permanent }
}

/// `/pattern/flags` — only when there really are flags. A bare `/old/` is
/// a path, not a regex between delimiters.
fn split_flags(from: &str) -> (&str, bool) {
    if from.starts_with('/') {
        if let Some(idx) = from.rfind('/') {
            let flags = &from[idx + 1..];
            if idx > 0 && !flags.is_empty() && flags.chars().all(|c| "gimsuy".contains(c)) {
                return (&from[1..idx], flags.contains('i'));
            }
        }
    }
    (from, false)
}

/// The rule's pattern, anchored at both ends, with the trailing slash made
/// optional — the path it is tested against has had its own trimmed.
fn anchored_pattern(from: &str) -> String {
    let (pattern, ci) = split_flags(from);
    let p = pattern.strip_prefix('^').unwrap_or(pattern);
    let p = p.strip_suffix('$').unwrap_or(p);
    let p = p.strip_suffix("/?").unwrap_or(p);
    let p = p.strip_suffix("\\/").or_else(|| p.strip_suffix('/')).unwrap_or(p);
    format!("{}^{}/?$", if ci { "(?i)" } else { "" }, p)
}

fn compile_from(from: &str) -> Regex {
    Regex::new(&anchored_pattern(from)).unwrap_or_else(|_| {
        // Not a valid expression: match it as the literal path it most
        // likely is, rather than dropping the rule.
        Regex::new(&format!("^{}/?$", regex::escape(from.trim_end_matches('/')))).unwrap()
    })
}

/// Why a `from` could not be used as written, if it cannot. The parser is
/// forgiving (a broken pattern falls back to a literal match) so an old file
/// still loads; an editor wants to be told instead.
pub fn validate_from(from: &str) -> std::result::Result<(), String> {
    if from.trim().is_empty() {
        return Err("The old URL is empty.".into());
    }
    if !(from.starts_with('/') || from.starts_with('^')) {
        return Err(format!("“{from}” must start with a slash."));
    }
    Regex::new(&anchored_pattern(from)).map(|_| ()).map_err(|e| {
        let msg = e.to_string();
        let last = msg.lines().last().unwrap_or("invalid pattern").trim().to_string();
        format!("“{from}” is not a valid pattern: {last}")
    })
}

impl Redirects {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules, in the order they are tried.
    pub fn rules(&self) -> &[RedirectRule] {
        &self.rules
    }

    /// Which rule (by position) catches `path`, and where it sends it.
    pub fn match_rule(&self, path: &str, query: Option<&str>) -> Option<(usize, String, bool)> {
        let path_no_trailing = path.trim_end_matches('/');
        let lookup = if path_no_trailing.is_empty() { "/" } else { path_no_trailing };
        for (i, rule) in self.rules.iter().enumerate() {
            if let Some(caps) = rule.from.captures(lookup) {
                let mut target = rule.to.clone();
                for i in 1..caps.len().min(10) {
                    if let Some(m) = caps.get(i) {
                        target = target.replace(&format!("${i}"), m.as_str());
                    }
                }
                // merge query params
                if let Some(q) = query {
                    if !q.is_empty() {
                        target = if target.contains('?') {
                            format!("{target}&{q}")
                        } else {
                            format!("{target}?{q}")
                        };
                    }
                }
                return Some((i, target, rule.permanent));
            }
        }
        None
    }

    /// Try to redirect `path` (+query). Returns (target, permanent).
    pub fn match_path(&self, path: &str, query: Option<&str>) -> Option<(String, bool)> {
        self.match_rule(path, query).map(|(_, target, permanent)| (target, permanent))
    }
}

/// Write rules back out as a redirects file. Ghost's grouped YAML when the
/// order allows it (every 301 ahead of every 302, no `from` twice in a
/// group); the list form otherwise, so the order the rules are tried in is
/// exactly the order they were given in.
pub fn to_yaml(rules: &[(String, String, bool)]) -> String {
    if rules.is_empty() {
        return "# No redirects configured yet.\n".into();
    }
    let ordered = rules.windows(2).all(|w| w[0].2 || !w[1].2);
    let mut seen = std::collections::HashSet::new();
    let unique = rules.iter().all(|(from, _, permanent)| seen.insert((from.clone(), *permanent)));
    if ordered && unique {
        let mut doc = serde_yaml::Mapping::new();
        for (code, permanent) in [(301u64, true), (302u64, false)] {
            let mut group = serde_yaml::Mapping::new();
            for (from, to, _) in rules.iter().filter(|r| r.2 == permanent) {
                group.insert(from.clone().into(), to.clone().into());
            }
            if !group.is_empty() {
                doc.insert(code.into(), group.into());
            }
        }
        serde_yaml::to_string(&doc).unwrap_or_default()
    } else {
        let list: Vec<serde_yaml::Value> = rules
            .iter()
            .map(|(from, to, permanent)| {
                let mut m = serde_yaml::Mapping::new();
                m.insert("from".into(), from.clone().into());
                m.insert("to".into(), to.clone().into());
                m.insert("permanent".into(), (*permanent).into());
                m.into()
            })
            .collect();
        serde_yaml::to_string(&list).unwrap_or_default()
    }
}

/// Read redirects.yaml from a path if it exists.
pub async fn load_from(path: &std::path::Path) -> Result<Redirects> {
    if path.exists() {
        let yaml = tokio::fs::read_to_string(path).await?;
        parse_redirects_yaml(&yaml)
    } else {
        Ok(Redirects::default())
    }
}

/// Keep a map for admin display.
pub fn rules_map(r: &Redirects) -> HashMap<String, serde_json::Value> {
    let mut out = HashMap::new();
    for rule in &r.rules {
        out.insert(
            rule.from_raw.clone(),
            serde_json::json!({ "to": rule.to, "permanent": rule.permanent }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml() {
        let r = parse_redirects_yaml("301:\n  /old/: /new/\n302:\n  /temp/: /other/\n").unwrap();
        assert_eq!(r.rules.len(), 2);
    }

    #[test]
    fn matches_and_redirects() {
        let r = parse_redirects_yaml("301:\n  /old/: /new/\n").unwrap();
        let (target, perm) = r.match_path("/old/", None).unwrap();
        assert_eq!(target, "/new/");
        assert!(perm);
        assert!(r.match_path("/other/", None).is_none());
    }

    #[test]
    fn a_plain_path_matches_only_itself() {
        let r = parse_redirects_yaml("301:\n  /old/: /new/\n").unwrap();
        assert!(r.match_path("/old", None).is_some());
        assert!(r.match_path("/bold/", None).is_none());
        assert!(r.match_path("/blog/old/", None).is_none());
    }

    #[test]
    fn regex_with_captures() {
        let r = parse_redirects_yaml("301:\n  /post/(\\d+)/: /posts/$1/\n").unwrap();
        let (target, _) = r.match_path("/post/12/", None).unwrap();
        assert_eq!(target, "/posts/12/");
    }

    #[test]
    fn flagged_form_is_case_insensitive() {
        let r = parse_redirects_yaml("301:\n  /^\\/Old\\/$/i: /new/\n").unwrap();
        assert!(r.match_path("/old/", None).is_some());
    }

    #[test]
    fn list_form_keeps_its_order() {
        let r = parse_redirects_yaml("- from: /a/\n  to: /b/\n  permanent: false\n- from: /c/\n  to: /d/\n  permanent: true\n").unwrap();
        assert_eq!(r.rules.len(), 2);
        assert!(!r.rules[0].permanent);
        assert!(r.rules[1].permanent);
    }

    #[test]
    fn written_rules_read_back_the_same() {
        for rules in [
            vec![("/a/".to_string(), "/b/".to_string(), true), ("/c/".into(), "/d/".into(), false)],
            vec![("/c/".to_string(), "/d/".to_string(), false), ("/a/".into(), "/b/".into(), true)],
        ] {
            let parsed = parse_redirects_yaml(&to_yaml(&rules)).unwrap();
            let back: Vec<_> = parsed.rules.iter().map(|r| (r.from_raw.clone(), r.to.clone(), r.permanent)).collect();
            assert_eq!(back, rules);
        }
    }

    #[test]
    fn a_broken_pattern_is_reported() {
        assert!(validate_from("/post/(\\d+/").is_err());
        assert!(validate_from("old/").is_err());
        assert!(validate_from("/old/").is_ok());
    }
}
