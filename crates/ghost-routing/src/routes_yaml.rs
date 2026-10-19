//! routes.yaml parsing (Ghost's dynamic routing config).
use ghost_core::{GhostError, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ROUTES_YAML: &str = "routes:\ncollections:\n  /:\n    permalink: /{slug}/\n    template: index\ntaxonomies:\n  tag: /tag/{slug}/\n  author: /author/{slug}/\n";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteSettings {
    #[serde(default)]
    pub routes: Vec<RouteEntry>,
    #[serde(default)]
    pub collections: Vec<CollectionEntry>,
    #[serde(default)]
    pub taxonomies: TaxonomyConfig,
}

/// A `routes:` entry — either a channel or a template route.
/// In YAML it's a map { path: {...} }; we accept the common shapes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub path: String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub limit: Option<serde_yaml::Value>,
    #[serde(default)]
    pub data: Option<serde_yaml::Value>,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionEntry {
    pub path: String,
    pub permalink: String,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub limit: Option<serde_yaml::Value>,
    #[serde(default)]
    pub data: Option<serde_yaml::Value>,
    #[serde(default)]
    pub rss: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaxonomyConfig {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

impl TaxonomyConfig {
    pub fn tag_permalink(&self) -> String {
        self.tag.clone().unwrap_or_else(|| "/tag/{slug}/".into())
    }
    pub fn author_permalink(&self) -> String {
        self.author.clone().unwrap_or_else(|| "/author/{slug}/".into())
    }
}

/// Raw YAML shapes we tolerate: `{ "/path/": { ... } }` maps.
#[derive(Debug, Deserialize)]
struct RawYaml {
    #[serde(default)]
    routes: serde_yaml::Value,
    #[serde(default)]
    collections: serde_yaml::Value,
    #[serde(default)]
    taxonomies: serde_yaml::Value,
}

fn parse_entries(v: &serde_yaml::Value) -> Vec<(String, serde_yaml::Mapping)> {
    let mut out = Vec::new();
    if let serde_yaml::Value::Mapping(m) = v {
        for (k, val) in m {
            if let (serde_yaml::Value::String(path), inner) = (k, val) {
                let spec = match inner {
                    serde_yaml::Value::Mapping(inner) => inner.clone(),
                    serde_yaml::Value::Null => serde_yaml::Mapping::new(),
                    _ => serde_yaml::Mapping::new(),
                };
                out.push((path.clone(), spec));
            }
        }
    }
    out
}

fn get_str(m: &serde_yaml::Mapping, key: &str) -> Option<String> {
    m.get(serde_yaml::Value::String(key.into()))
        .and_then(|v| v.as_str().map(str::to_string))
}

fn get_str_vec(m: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    match m.get(serde_yaml::Value::String(key.into())) {
        Some(serde_yaml::Value::String(s)) => vec![s.clone()],
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse routes.yaml content. `template:` (string) and `templates:` (list)
/// both populate `templates` in YAML order.
pub fn parse_routes_yaml(yaml: &str) -> Result<RouteSettings> {
    let raw: RawYaml = serde_yaml::from_str(yaml)
        .map_err(|e| GhostError::BadRequest(format!("routes.yaml: {e}")))?;

    let mut settings = RouteSettings::default();

    for (path, spec) in parse_entries(&raw.routes) {
        settings.routes.push(RouteEntry {
            path,
            template: get_str(&spec, "template"),
            templates: {
                let mut t = get_str_vec(&spec, "templates");
                if let Some(one) = get_str(&spec, "template") {
                    t.insert(0, one);
                }
                t
            },
            filter: get_str(&spec, "filter"),
            order: get_str(&spec, "order"),
            limit: spec.get(serde_yaml::Value::String("limit".into())).cloned(),
            data: spec.get(serde_yaml::Value::String("data".into())).cloned(),
            content_type: get_str(&spec, "content_type"),
        });
    }

    for (path, spec) in parse_entries(&raw.collections) {
        let permalink = get_str(&spec, "permalink")
            .ok_or_else(|| GhostError::BadRequest(format!("collection {path} requires a permalink")))?;
        if !permalink.contains("{slug}") && !permalink.contains("{id}") && !permalink.contains("{uuid}") {
            return Err(GhostError::BadRequest(format!(
                "collection {path} permalink must contain a {{param}}"
            )));
        }
        settings.collections.push(CollectionEntry {
            path,
            permalink,
            template: get_str(&spec, "template"),
            templates: {
                let mut t = get_str_vec(&spec, "templates");
                if let Some(one) = get_str(&spec, "template") {
                    t.insert(0, one);
                }
                t
            },
            filter: get_str(&spec, "filter"),
            order: get_str(&spec, "order"),
            limit: spec.get(serde_yaml::Value::String("limit".into())).cloned(),
            data: spec.get(serde_yaml::Value::String("data".into())).cloned(),
            rss: spec.get(serde_yaml::Value::String("rss".into())).and_then(|v| v.as_bool()),
        });
    }

    if let serde_yaml::Value::Mapping(t) = raw.taxonomies {
        settings.taxonomies.tag = t
            .get(serde_yaml::Value::String("tag".into()))
            .and_then(|v| v.as_str().map(str::to_string));
        settings.taxonomies.author = t
            .get(serde_yaml::Value::String("author".into()))
            .and_then(|v| v.as_str().map(str::to_string));
    }

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default() {
        let s = parse_routes_yaml(DEFAULT_ROUTES_YAML).unwrap();
        assert_eq!(s.collections.len(), 1);
        assert_eq!(s.collections[0].path, "/");
        assert_eq!(s.collections[0].permalink, "/{slug}/");
        assert_eq!(s.taxonomies.tag.as_deref(), Some("/tag/{slug}/"));
    }

    #[test]
    fn parses_flux_routes() {
        let yaml = r#"
routes:
collections:
  /blog/:
    permalink: /{slug}/
    template: blog
    data: page.blog
  /:
    permalink: /{slug}/
    template: index
taxonomies:
  tag: /tag/{slug}/
  author: /author/{slug}/
"#;
        let s = parse_routes_yaml(yaml).unwrap();
        assert_eq!(s.collections.len(), 2);
        assert_eq!(s.collections[0].path, "/blog/");
        assert_eq!(s.collections[0].template.as_deref(), Some("blog"));
        let data = s.collections[0].data.clone().unwrap();
        assert_eq!(data.as_str(), Some("page.blog"));
    }

    #[test]
    fn rejects_collection_without_permalink() {
        let yaml = "collections:\n  /:\n    template: index\n";
        assert!(parse_routes_yaml(yaml).is_err());
    }
}
