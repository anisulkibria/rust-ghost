//! Theme Builder: where a header, footer, single-post template or popup
//! applies, and how a popup opens. Stored as `builder_templates.options`.

use super::esc;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct PartOptions {
    /// `all`, `home`, or `paths`.
    pub include: String,
    /// Paths for `include = paths`; `*` at the end matches everything under.
    pub paths: Vec<String>,
    pub exclude: Vec<String>,
    /// Display condition: `any`, `visitors`, `members`, `free`, `paid`.
    pub who: String,
    /// Single templates: `post`, `page` or `all`.
    pub post_types: String,
    /// Single templates: only posts with one of these tag slugs (empty: any).
    pub tags: Vec<String>,
    /// Higher wins when two parts of a kind apply.
    pub priority: i64,
    // popups
    pub trigger: String,
    pub value: i64,
    pub freq: String,
    pub position: String,
    pub width: i64,
    pub overlay: bool,
    pub overlay_close: bool,
    pub radius: i64,
}

pub const TRIGGERS: &[(&str, &str)] = &[("load", "After the page loads"), ("scroll", "After scrolling"), ("exit", "On exit intent"), ("idle", "After inactivity"), ("click", "Only from a link")];
pub const FREQS: &[(&str, &str)] = &[("session", "Once per visit"), ("day", "At most once a day"), ("week", "At most once a week"), ("once", "Only once ever"), ("always", "Every time")];
pub const POSITIONS: &[(&str, &str)] = &[("center", "Centre"), ("top", "Top"), ("bottom", "Bottom"), ("left", "Bottom left"), ("right", "Bottom right"), ("bar", "Full-width bar"), ("full", "Full screen")];

fn pick<'a>(v: &'a str, allowed: &[(&str, &str)], default: &'a str) -> String {
    if allowed.iter().any(|(k, _)| *k == v) { v.to_string() } else { default.to_string() }
}

fn path_list(v: &Value) -> Vec<String> {
    let raw: Vec<String> = match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Value::String(s) => s.split(['\n', ',']).map(String::from).collect(),
        _ => Vec::new(),
    };
    raw.into_iter()
        .map(|p| p.trim().chars().filter(|c| !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | '\'')).take(200).collect::<String>())
        .filter(|p| p.starts_with('/'))
        .take(50)
        .collect()
}

impl PartOptions {
    pub fn from_value(v: &Value) -> Self {
        let st = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let num = |k: &str, lo: i64, hi: i64, d: i64| v[k].as_i64().or_else(|| v[k].as_str().and_then(|s| s.parse().ok())).map(|n| n.clamp(lo, hi)).unwrap_or(d);
        let flag = |k: &str, d: bool| v[k].as_bool().or_else(|| v[k].as_str().map(|s| s == "1" || s == "true")).unwrap_or(d);
        PartOptions {
            include: pick(&st("include"), &[("all", ""), ("home", ""), ("paths", "")], "all"),
            paths: path_list(&v["paths"]),
            exclude: path_list(&v["exclude"]),
            who: pick(&st("who"), super::spec::WHO, "any"),
            post_types: pick(&st("post_types"), &[("all", ""), ("post", ""), ("page", "")], "post"),
            tags: st("tags").split(',').map(|t| t.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()).filter(|t| !t.is_empty()).take(20).collect(),
            priority: num("priority", -100, 100, 0),
            trigger: pick(&st("trigger"), TRIGGERS, "load"),
            value: num("value", 0, 3600, 3),
            freq: pick(&st("freq"), FREQS, "session"),
            position: pick(&st("position"), POSITIONS, "center"),
            width: num("width", 200, 1600, 560),
            overlay: flag("overlay", true),
            overlay_close: flag("overlay_close", true),
            radius: num("radius", 0, 60, 20),
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "include": self.include, "paths": self.paths.join("\n"), "exclude": self.exclude.join("\n"), "who": self.who,
            "post_types": self.post_types, "tags": self.tags.join(","), "priority": self.priority,
            "trigger": self.trigger, "value": self.value, "freq": self.freq, "position": self.position,
            "width": self.width, "overlay": self.overlay, "overlay_close": self.overlay_close, "radius": self.radius,
        })
    }

    /// Whether a header, footer or popup applies to this path.
    pub fn applies_to_path(&self, path: &str) -> bool {
        if self.exclude.iter().any(|p| path_matches(p, path)) {
            return false;
        }
        match self.include.as_str() {
            "home" => path == "/" || path.is_empty(),
            "paths" => self.paths.iter().any(|p| path_matches(p, path)),
            _ => true,
        }
    }

    /// Whether a single-post template applies to this entry.
    pub fn applies_to_post(&self, post_type: &str, tags: &[String], path: &str) -> bool {
        if self.post_types != "all" && self.post_types != post_type {
            return false;
        }
        if !self.tags.is_empty() && !tags.iter().any(|t| self.tags.contains(t)) {
            return false;
        }
        !self.exclude.iter().any(|p| path_matches(p, path))
    }

    /// The display condition, for the request-time pass.
    pub fn cond(&self) -> Option<String> {
        (self.who != "any").then(|| format!("who={}", self.who))
    }

    /// Human summary for the Theme Builder list.
    pub fn summary(&self, kind: &str) -> String {
        let place = match kind {
            "single" => {
                let t = match self.post_types.as_str() {
                    "page" => "every page",
                    "all" => "every post and page",
                    _ => "every post",
                };
                if self.tags.is_empty() { t.to_string() } else { format!("{t} tagged {}", self.tags.join(", ")) }
            }
            _ => match self.include.as_str() {
                "home" => "the homepage".into(),
                "paths" => format!("{} address{}", self.paths.len(), if self.paths.len() == 1 { "" } else { "es" }),
                _ => "the entire site".into(),
            },
        };
        let mut out = format!("On {place}");
        if !self.exclude.is_empty() {
            out.push_str(&format!(", except {}", self.exclude.len()));
        }
        if self.who != "any" {
            out.push_str(&format!(" · {}", super::spec::WHO.iter().find(|(k, _)| *k == self.who).map(|x| x.1).unwrap_or("")));
        }
        if kind == "popup" {
            let t = TRIGGERS.iter().find(|(k, _)| *k == self.trigger).map(|x| x.1).unwrap_or("");
            out.push_str(&format!(" · {t}"));
        }
        out
    }
}

/// `/blog/*` matches `/blog/` and everything under it; anything else is an
/// exact path, trailing slash optional.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let norm = |p: &str| p.trim_end_matches('/').to_string();
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix) || norm(path) == norm(prefix);
    }
    norm(pattern) == norm(path)
}

/// A popup's wrapper: the layout HTML in a dialog the runtime opens.
pub fn popup_html(id: &str, o: &PartOptions, inner: &str) -> String {
    format!(
        r#"<div class="vb-popup vb-pop-{pos}{dim}" id="popup-{id}" data-id="{id}" data-trigger="{trig}" data-value="{val}" data-freq="{freq}" data-overlay-close="{oc}" style="--vb-pop-w:{w}px;--vb-pop-r:{r}px" role="dialog" aria-modal="true" hidden><div class="vb-pop-box"><button type="button" class="vb-pop-x" aria-label="Close">×</button>{inner}</div></div>"#,
        id = esc(id),
        pos = o.position,
        dim = if o.overlay { " vb-pop-dim" } else { "" },
        trig = o.trigger,
        val = o.value,
        freq = o.freq,
        oc = o.overlay_close,
        w = o.width,
        r = o.radius,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_conditions() {
        let o = PartOptions::from_value(&json!({"include": "paths", "paths": "/blog/*\n/about/\njavascript:x", "exclude": ["/blog/secret/"]}));
        assert_eq!(o.paths.len(), 2);
        assert!(o.applies_to_path("/blog/"));
        assert!(o.applies_to_path("/blog/a-post/"));
        assert!(o.applies_to_path("/about"));
        assert!(!o.applies_to_path("/blog/secret/"));
        assert!(!o.applies_to_path("/contact/"));
        let home = PartOptions::from_value(&json!({"include": "home"}));
        assert!(home.applies_to_path("/") && !home.applies_to_path("/x/"));
        let single = PartOptions::from_value(&json!({"post_types": "post", "tags": "news, x y"}));
        assert!(single.applies_to_post("post", &["news".into()], "/a/"));
        assert!(!single.applies_to_post("page", &["news".into()], "/a/"));
        assert!(!single.applies_to_post("post", &["other".into()], "/a/"));
    }

    #[test]
    fn options_are_cleaned() {
        let o = PartOptions::from_value(&json!({"trigger": "evil", "width": 99999, "position": "x", "who": "paid"}));
        assert_eq!(o.trigger, "load");
        assert_eq!(o.width, 1600);
        assert_eq!(o.position, "center");
        assert_eq!(o.cond().as_deref(), Some("who=paid"));
        let html = popup_html("t1", &o, "<p>x</p>");
        assert!(html.contains(r#"data-trigger="load""#) && html.contains("hidden"));
    }
}
