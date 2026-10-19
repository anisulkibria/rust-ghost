//! The router tree: routes.yaml entries compiled into matchers, tried in
//! Ghost's dominance order, with permalink building/parsing and template
//! resolution.
use crate::routes_yaml::{CollectionEntry, RouteEntry, RouteSettings};
use ghost_core::Result;
use ghost_nql::{self, Expr};
use once_cell::sync::Lazy;
use regex::Regex;

static PERMALINK_PARAM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{(\w+)\}").unwrap());

/// Convert `/{slug}/` permalink notation into a `:slug` path pattern and a
/// matching regex.
pub fn permalink_regex(permalink: &str) -> Regex {
    let mut pattern = String::from("^");
    for seg in permalink.split('/') {
        if seg.is_empty() {
            continue;
        }
        let mut seg_pat = String::new();
        let mut last = 0;
        for m in PERMALINK_PARAM.find_iter(seg) {
            // hyphen-aware bound like Ghost's path-match: stop at '-' too
            seg_pat.push_str(&regex::escape(&seg[last..m.start()]));
            seg_pat.push_str(&format!(r"([a-zA-Z0-9-]+)"));
            last = m.end();
        }
        seg_pat.push_str(&regex::escape(&seg[last..]));
        pattern.push('/');
        pattern.push_str(&seg_pat);
    }
    if permalink.ends_with('/') {
        pattern.push('/');
    }
    pattern.push_str("/?$");
    Regex::new(&pattern).unwrap()
}

/// Substitute `{slug}` (and any param) in a permalink template with values
/// from the resource JSON.
pub fn build_permalink(permalink: &str, resource: &serde_json::Value) -> String {
    PERMALINK_PARAM
        .replace_all(permalink, |caps: &regex::Captures| {
            let param = caps.get(1).map(|c| c.as_str()).unwrap_or("slug");
            let v = resource
                .get(param)
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            v.to_string()
        })
        .to_string()
}

/// What a matched route wants rendered.
#[derive(Debug, Clone)]
pub enum RouteMatch {
    /// `/p/:uuid/:options?` preview
    Preview { uuid: String, options: Option<String> },
    /// A static (routes:) channel page
    Channel {
        name: String,
        path: String,
        filter: Option<Expr>,
        order: Option<String>,
        limit: Option<i64>,
        templates: Vec<String>,
        page: i64,
    },
    /// A static (routes:) template-only page
    Custom {
        name: String,
        templates: Vec<String>,
        data: Option<serde_yaml::Value>,
        content_type: Option<String>,
    },
    /// Collection index `/blog/` (+ `/page/2/`)
    CollectionIndex {
        name: String,
        path: String,
        filter: Option<Expr>,
        order: Option<String>,
        limit: Option<i64>,
        templates: Vec<String>,
        data: Option<serde_yaml::Value>,
        rss: bool,
        page: i64,
    },
    /// A post/page permalink owned by a collection
    CollectionEntry {
        collection: String,
        permalink: String,
        slug: String,
    },
    /// `/rss/` for a collection or taxonomy
    Rss {
        base: String,
        name: String,
        kind: RssKind,
    },
    /// Static page (fixed `/:slug/` router)
    StaticPage { slug: String },
    /// Taxonomy channel (`/tag/news/`, `/author/x/`)
    Taxonomy {
        kind: TaxonomyKind,
        slug: String,
        permalink: String,
        page: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaxonomyKind {
    Tag,
    Author,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RssKind {
    Collection,
    Tag,
    Author,
}

#[derive(Debug, Clone)]
struct CompiledCollection {
    entry: CollectionEntry,
    index_re: Option<Regex>,
    page_re: Option<Regex>,
    permalink_re: Regex,
    filter: Option<Expr>,
    rss: bool,
    name: String,
}

#[derive(Debug, Clone)]
struct CompiledTaxonomy {
    kind: TaxonomyKind,
    permalink: String,
    page_re: Regex,
}

/// The full router registry for the current routes.yaml.
#[derive(Debug, Clone)]
pub struct RouterTree {
    collections: Vec<CompiledCollection>,
    channels: Vec<(RouteEntry, Option<Expr>, Option<Regex>)>,
    customs: Vec<RouteEntry>,
    taxonomies: Vec<CompiledTaxonomy>,
    static_page_re: Regex,
    preview_re: Regex,
}

fn route_name(path: &str) -> String {
    let n = path.trim_matches('/');
    if n.is_empty() {
        "index".into()
    } else {
        n.replace(['/', '.'], "-")
    }
}

fn parse_limit(v: &Option<serde_yaml::Value>) -> Option<i64> {
    match v {
        Some(serde_yaml::Value::Number(n)) => n.as_i64(),
        Some(serde_yaml::Value::String(s)) => {
            if s == "all" {
                Some(-1)
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

impl RouterTree {
    pub fn build(settings: &RouteSettings) -> Result<Self> {
        let mut collections = Vec::new();
        for entry in &settings.collections {
            let base = entry.path.trim_end_matches('/').to_string();
            let index_re = if entry.path == "/" {
                None
            } else {
                Some(Regex::new(&format!("^{}(?:/page/(\\d+))?/?$", regex::escape(&base))).unwrap())
            };
            let page_re = Regex::new(&format!("^{}(?:/page/(\\d+))?/?$", regex::escape(&base))).unwrap();
            collections.push(CompiledCollection {
                name: route_name(&entry.path),
                entry: entry.clone(),
                index_re,
                page_re: Some(page_re),
                permalink_re: permalink_regex(&entry.permalink),
                filter: entry
                    .filter
                    .as_deref()
                    .map(ghost_nql::parse)
                    .transpose()?,
                rss: entry.rss.unwrap_or(true),
            });
        }

        let mut channels = Vec::new();
        let mut customs = Vec::new();
        for entry in &settings.routes {
            // Ghost's rule in practice: routes with a filter (or no template
            // at all) are channels; template-bearing routes are custom pages.
            if entry.filter.is_some() {
                let re = Regex::new(&format!("^{}(?:/page/(\\d+))?/?$", regex::escape(entry.path.trim_end_matches('/')))).unwrap();
                let filter = ghost_nql::parse(entry.filter.as_deref().unwrap())?;
                channels.push((entry.clone(), Some(filter), Some(re)));
            } else {
                customs.push(entry.clone());
            }
        }

        let mut taxonomies = Vec::new();
        if settings.taxonomies.tag.is_some() {
            let permalink = settings.taxonomies.tag_permalink();
            let base = permalink.replace("{slug}", "").trim_end_matches('/').to_string();
            taxonomies.push(CompiledTaxonomy {
                kind: TaxonomyKind::Tag,
                permalink: permalink.clone(),
                page_re: Regex::new(&format!(
                    "^{}/([a-zA-Z0-9-]+)(?:/page/(\\d+))?/?$",
                    regex::escape(&base)
                ))
                .unwrap(),
            });
        }
        if settings.taxonomies.author.is_some() {
            let permalink = settings.taxonomies.author_permalink();
            let base = permalink.replace("{slug}", "").trim_end_matches('/').to_string();
            taxonomies.push(CompiledTaxonomy {
                kind: TaxonomyKind::Author,
                permalink,
                page_re: Regex::new(&format!(
                    "^{}/([a-zA-Z0-9-]+)(?:/page/(\\d+))?/?$",
                    regex::escape(&base)
                ))
                .unwrap(),
            });
        }

        Ok(Self {
            collections,
            channels,
            customs,
            taxonomies,
            static_page_re: Regex::new(r"^/([a-zA-Z0-9-]+)/?$").unwrap(),
            preview_re: Regex::new(r"^/p/([0-9a-fA-F-]{8,64})(?:/([a-z]+))?/?$").unwrap(),
        })
    }

    /// Match a request path (no query string). Dominance order:
    /// preview > channels/custom static routes > collections (index,
    /// permalinks) > static pages > taxonomies.
    pub fn match_path(&self, path: &str) -> Option<RouteMatch> {
        // 1. Preview
        if let Some(c) = self.preview_re.captures(path) {
            return Some(RouteMatch::Preview {
                uuid: c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
                options: c.get(2).map(|m| m.as_str().to_string()),
            });
        }

        // 2. RSS: /rss/, /blog/rss/, /tag/news/rss/ (before other routes)
        if path == "/rss" || path == "/rss/" {
            if let Some(name) = self.primary_collection() {
                return Some(RouteMatch::Rss {
                    base: "/".into(),
                    name,
                    kind: RssKind::Collection,
                });
            }
        }
        if path.ends_with("/rss") || path.ends_with("/rss/") {
            let stripped = path.trim_end_matches('/');
            let without_rss = stripped.strip_suffix("/rss").unwrap_or(stripped);
            // collection rss?
            for col in &self.collections {
                if route_base(&col.entry.path) == route_base(without_rss) && col.rss {
                    return Some(RouteMatch::Rss {
                        base: col.entry.path.clone(),
                        name: col.name.clone(),
                        kind: RssKind::Collection,
                    });
                }
            }
            // taxonomy rss?
            for t in &self.taxonomies {
                if let Some(c) = t.page_re.captures(&format!("{without_rss}/")) {
                    let slug = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                    return Some(RouteMatch::Rss {
                        base: t.permalink.clone(),
                        name: slug,
                        kind: match t.kind {
                            TaxonomyKind::Tag => RssKind::Tag,
                            TaxonomyKind::Author => RssKind::Author,
                        },
                    });
                }
            }
        }

        // 3. Static routes (channels)
        for (entry, filter, re) in &self.channels {
            if let Some(re) = re {
                if let Some(c) = re.captures(path) {
                    let page = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
                    return Some(RouteMatch::Channel {
                        name: route_name(&entry.path),
                        path: entry.path.clone(),
                        filter: filter.clone(),
                        order: entry.order.clone(),
                        limit: parse_limit(&entry.limit),
                        templates: entry.templates.clone(),
                        page,
                    });
                }
            }
        }

        // 4. Custom template routes (exact path)
        for entry in &self.customs {
            if path == normalize(&entry.path) {
                return Some(RouteMatch::Custom {
                    name: route_name(&entry.path),
                    templates: entry.templates.clone(),
                    data: entry.data.clone(),
                    content_type: entry.content_type.clone(),
                });
            }
        }

        // 5. Collections: index, page/N, then permalinks
        for col in &self.collections {
            if col.entry.path == "/" {
                // home: "/", "/page/2/", plus permalink match
                if path == "/" {
                    return Some(RouteMatch::CollectionIndex {
                        name: col.name.clone(),
                        path: col.entry.path.clone(),
                        filter: col.filter.clone(),
                        order: col.entry.order.clone(),
                        limit: parse_limit(&col.entry.limit),
                        templates: col.entry.templates.clone(),
                        data: col.entry.data.clone(),
                        rss: col.rss,
                        page: 1,
                    });
                }
                if let Some(c) = col.page_re.as_ref().unwrap().captures(path) {
                    let page = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
                    return Some(RouteMatch::CollectionIndex {
                        name: col.name.clone(),
                        path: col.entry.path.clone(),
                        filter: col.filter.clone(),
                        order: col.entry.order.clone(),
                        limit: parse_limit(&col.entry.limit),
                        templates: col.entry.templates.clone(),
                        data: col.entry.data.clone(),
                        rss: col.rss,
                        page,
                    });
                }
            } else if let Some(re) = &col.index_re {
                if let Some(c) = re.captures(path) {
                    if !path.ends_with("/rss/") {
                        let page = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
                        return Some(RouteMatch::CollectionIndex {
                            name: col.name.clone(),
                            path: col.entry.path.clone(),
                            filter: col.filter.clone(),
                            order: col.entry.order.clone(),
                            limit: parse_limit(&col.entry.limit),
                            templates: col.entry.templates.clone(),
                            data: col.entry.data.clone(),
                            rss: col.rss,
                            page,
                        });
                    }
                }
            }
            // permalink?
            if let Some(c) = col.permalink_re.captures(path) {
                if let Some(slug) = c.get(1) {
                    return Some(RouteMatch::CollectionEntry {
                        collection: col.name.clone(),
                        permalink: col.entry.permalink.clone(),
                        slug: slug.as_str().to_string(),
                    });
                }
            }
        }

        // 6. Static pages: /:slug/
        if let Some(c) = self.static_page_re.captures(path) {
            let slug = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            // avoid swallowing reserved paths
            if !matches!(slug.as_str(), "ghost" | "rss" | "amp" | "p" | "sitemap.xml" | "tag" | "author" | "assets" | "content" | "public" | "members") {
                return Some(RouteMatch::StaticPage { slug });
            }
        }

        // 7. Taxonomies
        for t in &self.taxonomies {
            if let Some(c) = t.page_re.captures(path) {
                let slug = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                let page = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
                return Some(RouteMatch::Taxonomy {
                    kind: t.kind,
                    slug,
                    permalink: t.permalink.clone(),
                    page,
                });
            }
        }

        None
    }

    /// The permalink template for a taxonomy archive, e.g. `/tag/{slug}/`.
    /// Themes reach these through `{{url}}` on a tag or author.
    pub fn taxonomy_permalink(&self, kind: TaxonomyKind) -> Option<&str> {
        self.taxonomies
            .iter()
            .find(|t| t.kind == kind)
            .map(|t| t.permalink.as_str())
    }

    pub fn primary_collection(&self) -> Option<String> {
        // index collection first, else the only/first rss-enabled one
        if let Some(c) = self.collections.iter().find(|c| c.entry.path == "/") {
            return Some(c.name.clone());
        }
        self.collections.iter().find(|c| c.rss).map(|c| c.name.clone())
    }

    /// The permalink template a post/page renders under, given its
    /// serialized JSON (used for url building + ownership).
    pub fn permalink_for(&self, resource: &serde_json::Value) -> Option<(String, String)> {
        // Tags and authors are taxonomies, not entries. They carry no `type`,
        // so without this they fall through to the posts collection and get a
        // post's permalink — a tag would report `/news/` while its archive
        // actually lives at `/tag/news/`, and every link built from it 404s.
        if let Some(kind) = taxonomy_kind_of(resource) {
            if let Some(t) = self.taxonomies.iter().find(|t| t.kind == kind) {
                let name = match kind {
                    TaxonomyKind::Tag => "tag",
                    TaxonomyKind::Author => "author",
                };
                return Some((name.to_string(), t.permalink.clone()));
            }
            // routes.yaml can switch a taxonomy off; nothing to link to then
            return None;
        }

        let post_type = resource.get("type").and_then(|v| v.as_str()).unwrap_or("post");
        if post_type == "page" {
            return Some(("/".into(), "/{slug}/".into()));
        }
        for col in &self.collections {
            if let Some(filter) = &col.filter {
                if ghost_nql::evaluate(filter, resource) {
                    return Some((col.name.clone(), col.entry.permalink.clone()));
                }
            } else {
                // first collection without filter owns posts
                return Some((col.name.clone(), col.entry.permalink.clone()));
            }
        }
        None
    }

    /// Template resolution order for a collection index.
    pub fn collection_templates(&self, name: &str, yaml_templates: &[String], is_front_page: bool) -> Vec<String> {
        let mut list: Vec<String> = Vec::new();
        if is_front_page {
            list.push("home".into());
        }
        list.extend(yaml_templates.iter().cloned());
        if name != "index" {
            list.push(name.to_string());
        }
        list.push("index".into());
        list
    }
}

/// Classify a serialized resource as a taxonomy, or `None` for an entry.
///
/// Posts and pages always carry a `title` key (null when untitled), and tags
/// and authors never do — that is the first cut. Among taxonomies,
/// `accent_color` only exists on tags and `profile_image` only on authors.
/// `visibility` alone would not do it: posts have one too.
fn taxonomy_kind_of(resource: &serde_json::Value) -> Option<TaxonomyKind> {
    let has = |k: &str| resource.get(k).is_some();
    if has("title") {
        return None;
    }
    if has("profile_image") || has("bio") {
        return Some(TaxonomyKind::Author);
    }
    if has("accent_color") || has("visibility") {
        return Some(TaxonomyKind::Tag);
    }
    None
}

fn normalize(path: &str) -> String {
    if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

fn route_base(path: &str) -> String {
    let p = path.trim_end_matches('/');
    if p.is_empty() {
        "/".into()
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes_yaml::parse_routes_yaml;

    fn tree_default() -> RouterTree {
        RouterTree::build(&parse_routes_yaml(crate::DEFAULT_ROUTES_YAML).unwrap()).unwrap()
    }

    #[test]
    fn matches_home() {
        let t = tree_default();
        assert!(matches!(t.match_path("/"), Some(RouteMatch::CollectionIndex { .. })));
    }

    #[test]
    fn matches_post_permalink() {
        let t = tree_default();
        assert!(matches!(
            t.match_path("/my-post/"),
            Some(RouteMatch::CollectionEntry { slug, .. }) if slug == "my-post"
        ));
    }

    #[test]
    fn matches_page_and_taxonomy() {
        let t = tree_default();
        // /about/ matches the collection permalink first; the handler
        // resolves post-then-page (Ghost's fallthrough order).
        assert!(matches!(t.match_path("/about/"), Some(RouteMatch::CollectionEntry { slug, .. }) if slug == "about"));
        assert!(matches!(
            t.match_path("/tag/news/"),
            Some(RouteMatch::Taxonomy { kind: TaxonomyKind::Tag, slug, .. }) if slug == "news"
        ));
        assert!(matches!(
            t.match_path("/author/ghost/"),
            Some(RouteMatch::Taxonomy { kind: TaxonomyKind::Author, .. })
        ));
    }

    #[test]
    fn matches_preview() {
        let t = tree_default();
        assert!(matches!(
            t.match_path("/p/6b4f1c2a9d3e4f5a80112233/"),
            Some(RouteMatch::Preview { .. })
        ));
    }

    #[test]
    fn matches_flux_collections() {
        let yaml = "routes:\ncollections:\n  /blog/:\n    permalink: /{slug}/\n    template: blog\n  /:\n    permalink: /{slug}/\n    template: index\ntaxonomies:\n  tag: /tag/{slug}/\n  author: /author/{slug}/\n";
        let t = RouterTree::build(&parse_routes_yaml(yaml).unwrap()).unwrap();
        assert!(matches!(t.match_path("/blog/"), Some(RouteMatch::CollectionIndex { name, .. }) if name == "blog"));
        assert!(matches!(t.match_path("/blog/page/2/"), Some(RouteMatch::CollectionIndex { page: 2, .. })));
        assert!(matches!(t.match_path("/"), Some(RouteMatch::CollectionIndex { name, .. }) if name == "index"));
        assert!(matches!(t.match_path("/my-post/"), Some(RouteMatch::CollectionEntry { .. })));
        assert!(matches!(t.match_path("/blog/rss/"), Some(RouteMatch::Rss { .. })));
    }

    #[test]
    fn builds_permalinks() {
        let r = serde_json::json!({ "slug": "hello-world", "year": "2024" });
        assert_eq!(build_permalink("/{slug}/", &r), "/hello-world/");
        assert_eq!(build_permalink("/{year}/{slug}/", &r), "/2024/hello-world/");
    }

    #[test]
    fn reserved_slugs_not_pages() {
        let t = tree_default();
        assert!(t.match_path("/ghost/").is_none() || !matches!(t.match_path("/ghost/"), Some(RouteMatch::StaticPage { .. })));
    }
}

#[cfg(test)]
mod permalink_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn taxonomies_are_told_apart_from_entries() {
        // a tag: no title, and accent_color is tag-only
        let tag = json!({"id":"1","name":"News","slug":"news","visibility":"public","accent_color":null});
        assert_eq!(taxonomy_kind_of(&tag), Some(TaxonomyKind::Tag));

        // an author: no title, profile_image is author-only
        let author = json!({"id":"1","name":"Ada","slug":"ada","profile_image":null,"bio":null});
        assert_eq!(taxonomy_kind_of(&author), Some(TaxonomyKind::Author));

        // a post carries `title` even when it is null, and also has its own
        // `visibility` — which is exactly why title is checked first
        let post = json!({"id":"1","title":null,"slug":"hello","visibility":"public","status":"published"});
        assert_eq!(taxonomy_kind_of(&post), None);

        let page = json!({"id":"1","title":"About","slug":"about","type":"page"});
        assert_eq!(taxonomy_kind_of(&page), None);
    }
}
