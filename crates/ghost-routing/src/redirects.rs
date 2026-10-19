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

/// Parse redirects.yaml. Supported shape (Ghost's default):
/// ```yaml
/// 301:
///   /old/: /new/
/// 302:
///   /temp/: /other/
/// ```
/// `from` may be a regex in `/…/i` form; `to` supports $1..$9 captures.
pub fn parse_redirects_yaml(yaml: &str) -> Result<Redirects> {
    if yaml.trim().is_empty() {
        return Ok(Redirects::default());
    }
    let doc: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| GhostError::BadRequest(format!("redirects.yaml: {e}")))?;
    let mut rules = Vec::new();

    if let serde_yaml::Value::Mapping(m) = doc {
        for (code, entries) in m {
            let status = code.as_u64().unwrap_or(301);
            let permanent = status == 301;
            if let serde_yaml::Value::Mapping(entries) = entries {
                for (from, to) in entries {
                    let (Some(from), Some(to)) = (from.as_str(), to.as_str()) else {
                        continue;
                    };
                    rules.push(RedirectRule {
                        from: compile_from(from),
                        from_raw: from.to_string(),
                        to: to.to_string(),
                        permanent,
                    });
                }
            }
        }
    }
    Ok(Redirects { rules })
}

fn compile_from(from: &str) -> Regex {
    // /regex/i form
    if from.starts_with('/') && from.len() > 2 {
        if let Some(idx) = from.rfind('/') {
            let (pattern, flags) = (&from[1..idx], &from[idx + 1..]);
            let case_insensitive = flags.contains('i');
            let pattern = if pattern.ends_with("/?$") || pattern.ends_with('$') {
                pattern.to_string()
            } else {
                format!("{pattern}/?$")
            };
            let re = Regex::new(&pattern).unwrap_or_else(|_| Regex::new(&regex::escape(from)).unwrap());
            if case_insensitive {
                return Regex::new(&format!("(?i){pattern}")).unwrap_or(re);
            }
            return re;
        }
    }
    let pattern = format!("^{}$", regex::escape(from.trim_end_matches('/')));
    Regex::new(&pattern).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

impl Redirects {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Try to redirect `path` (+query). Returns (target, permanent).
    pub fn match_path(&self, path: &str, query: Option<&str>) -> Option<(String, bool)> {
        let path_no_trailing = path.trim_end_matches('/');
        let lookup = if path_no_trailing.is_empty() { "/" } else { path_no_trailing };
        for rule in &self.rules {
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
                return Some((target, rule.permanent));
            }
        }
        None
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
    fn regex_with_captures() {
        let r = parse_redirects_yaml("301:\n  /post/(\\d+)/: /posts/$1/\n").unwrap();
        let (target, _) = r.match_path("/post/12/", None).unwrap();
        assert_eq!(target, "/posts/12/");
    }
}
