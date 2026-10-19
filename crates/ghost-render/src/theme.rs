//! Theme loading, validation and the per-activation Handlebars registry.
use handlebars::{Context, Handlebars, Helper, Output, RenderContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::helpers::{self, HelperDeps};
use crate::preprocess;
use ghost_core::{GhostError, Result};

/// package.json of a Ghost theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemePkg {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub engines: Value,
    #[serde(default, deserialize_with = "de_config")]
    pub config: ThemeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_ppp")]
    pub posts_per_page: i64,
    #[serde(default, deserialize_with = "de_image_sizes")]
    pub image_sizes: Vec<ImageSize>,
    #[serde(default)]
    pub card_assets: CardAssets,
    /// `config.custom` — the theme settings a site owner can change, which
    /// templates read through `@custom`.
    #[serde(default)]
    pub custom: serde_json::Map<String, Value>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            posts_per_page: default_ppp(),
            image_sizes: Vec::new(),
            card_assets: CardAssets::default(),
            custom: serde_json::Map::new(),
        }
    }
}

fn default_ppp() -> i64 {
    5
}

/// A theme's `config.card_assets`.
///
/// Ghost accepts `true`, `false`, `{include: […]}` or `{exclude: […]}`, and
/// treats an absent key as "none" — themes opt in. Here the default is the
/// other way round: the editor writes Koenig cards, and a card that arrives
/// on the page with no stylesheet behind it is visibly broken, so the assets
/// are served unless a theme says otherwise. A theme that ships its own card
/// styles turns them off the same way it would in Ghost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CardAssets {
    /// `true` for every card, `false` for none.
    All(bool),
    /// Named cards only, or everything except the named ones.
    Named {
        #[serde(default)]
        include: Option<Vec<String>>,
        #[serde(default)]
        exclude: Option<Vec<String>>,
    },
}

impl Default for CardAssets {
    fn default() -> Self {
        CardAssets::All(true)
    }
}

impl CardAssets {
    /// Whether the stylesheet and script should be linked at all.
    pub fn any(&self) -> bool {
        match self {
            CardAssets::All(on) => *on,
            // An empty include list asks for nothing; an exclude list still
            // leaves the rest.
            CardAssets::Named { include, exclude } => match (include, exclude) {
                (Some(list), _) => !list.is_empty(),
                (None, Some(_)) => true,
                (None, None) => true,
            },
        }
    }

    /// Whether one card's assets are wanted, by card name (`bookmark`,
    /// `gallery`, …).
    pub fn includes(&self, card: &str) -> bool {
        match self {
            CardAssets::All(on) => *on,
            CardAssets::Named { include: Some(list), .. } => list.iter().any(|c| c == card),
            CardAssets::Named { exclude: Some(list), .. } => !list.iter().any(|c| c == card),
            CardAssets::Named { .. } => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageSize {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: Option<i64>,
}

/// Ghost themes declare `config.image_sizes` as a map keyed by size name:
///
/// ```json
/// "image_sizes": { "s": { "width": 300 }, "m": { "width": 720 } }
/// ```
///
/// Flatten it into a list that carries the name, which is the shape
/// `{{img_url size="s"}}` looks up. A plain array is accepted too.
fn de_image_sizes<'de, D>(d: D) -> std::result::Result<Vec<ImageSize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Value::deserialize(d)? {
        Value::Object(map) => map
            .into_iter()
            .filter_map(|(name, v)| {
                let mut size: ImageSize = serde_json::from_value(v).ok()?;
                size.name = name;
                Some(size)
            })
            .collect(),
        value @ Value::Array(_) => serde_json::from_value(value).unwrap_or_default(),
        _ => Vec::new(),
    })
}

/// The `config` block is theme metadata, not something worth refusing to load
/// a whole theme over — an unexpected shape falls back to the defaults.
fn de_config<'de, D>(d: D) -> std::result::Result<ThemeConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(d)?;
    Ok(serde_json::from_value(value.clone()).unwrap_or_else(|e| {
        tracing::warn!("ignoring theme config ({e}): {value}");
        ThemeConfig::default()
    }))
}

/// A loaded, activated theme: templates compiled, partials registered,
/// helpers wired with this theme's deps.
pub struct ThemeState {
    pub name: String,
    pub dir: PathBuf,
    pub pkg: ThemePkg,
    pub hb: Handlebars<'static>,
    /// template name → layout name (from `{{!< layout}}`)
    pub layouts: HashMap<String, String>,
    /// "css/screen.css" → sha256 hex (for {{asset}})
    pub asset_hashes: HashMap<String, String>,
    /// all template names (without .hbs) — for hasTemplate checks
    pub templates: Vec<String>,
}

impl ThemeState {
    pub fn has_template(&self, name: &str) -> bool {
        self.templates.iter().any(|t| t == name)
    }

    /// The `@custom` object: every setting the theme declares, at its
    /// declared default, with `overrides` (what the site owner has changed)
    /// applied on top.
    ///
    /// A setting with no `default` is null, except booleans, which Ghost
    /// treats as off.
    pub fn custom_settings(&self, overrides: Option<&Value>) -> Value {
        let mut out = match Self::custom_settings_defaults(&self.pkg.config.custom) {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if let Some(Value::Object(overrides)) = overrides {
            for (key, value) in overrides {
                // only settings the theme declares are addressable
                if out.contains_key(key) {
                    out.insert(key.clone(), value.clone());
                }
            }
        }
        Value::Object(out)
    }

    /// Every declared custom setting at its default — the base both the
    /// renderer (`@custom`) and the admin Design form build on.
    pub fn custom_settings_defaults(decls: &serde_json::Map<String, Value>) -> Value {
        let mut out = serde_json::Map::new();
        for (key, def) in decls {
            let value = def.get("default").cloned().unwrap_or_else(|| {
                match def.get("type").and_then(|t| t.as_str()) {
                    Some("boolean") => Value::Bool(false),
                    _ => Value::Null,
                }
            });
            out.insert(key.clone(), value);
        }
        Value::Object(out)
    }

    pub fn posts_per_page(&self) -> i64 {
        if self.pkg.config.posts_per_page > 0 {
            self.pkg.config.posts_per_page
        } else {
            5
        }
    }

    /// Render `template` with `data` applying its layout (two-pass).
    pub fn render(&self, template: &str, data: &Value) -> Result<String> {
        // `{{#contentFor}}` in the template fills blocks the layout reads
        // back, so the store has to be per-render — a registry-wide one would
        // leak content between concurrent requests.
        let blocks: helpers::BlockStore = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let mut hb = self.hb.clone();
        hb.register_helper("block", Box::new(helpers::BlockHelper { blocks: blocks.clone() }));
        hb.register_helper("contentFor", Box::new(helpers::ContentFor { blocks: blocks.clone() }));

        let inner = hb
            .render(template, data)
            .map_err(|e| GhostError::Render(format!("template {template}: {e}")))?;

        let layout = self
            .layouts
            .get(template)
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        if !hb.has_template(&layout) {
            return Ok(inner);
        }

        hb.register_helper("body", Box::new(BodyHelper { body: inner }));
        hb.render(&layout, data)
            .map_err(|e| GhostError::Render(format!("layout {layout}: {e}")))
    }
}

struct BodyHelper {
    body: String,
}
impl handlebars::HelperDef for BodyHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> handlebars::HelperResult {
        let _ = rc;
        out.write(&self.body)?;
        Ok(())
    }
}

/// gscan-lite: the minimum a directory needs to be a usable Ghost theme.
pub fn validate(theme_dir: &Path) -> Result<()> {
    let name = theme_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "theme".into());
    for required in ["index.hbs", "post.hbs"] {
        if !theme_dir.join(required).exists() {
            return Err(GhostError::ThemeError(format!("theme {name} is missing {required}")));
        }
    }
    Ok(())
}

/// Read a theme's package.json, falling back to defaults when absent.
pub fn read_pkg(theme_dir: &Path) -> Option<ThemePkg> {
    let raw = std::fs::read_to_string(theme_dir.join("package.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Scan a theme directory and build the activation state.
pub fn load_and_activate(theme_dir: &Path, deps: &HelperDeps, translations: HashMap<String, String>) -> Result<ThemeState> {
    let name = theme_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "theme".into());

    let pkg_path = theme_dir.join("package.json");
    let pkg: ThemePkg = if pkg_path.exists() {
        let raw = std::fs::read_to_string(&pkg_path)?;
        serde_json::from_str(&raw)
            .map_err(|e| GhostError::ThemeError(format!("{} package.json: {e}", name)))?
    } else {
        ThemePkg {
            name: name.clone(),
            version: None,
            description: None,
            engines: Value::Null,
            config: ThemeConfig::default(),
        }
    };

    validate(theme_dir)?;

    // asset hashes
    let mut asset_hashes = HashMap::new();
    hash_assets(&theme_dir.join("assets"), "", &mut asset_hashes)?;

    // helpers need the real hashes; clone into Arc for the closure
    let mut deps = deps.clone();
    deps.global_asset_hash = global_asset_hash(&asset_hashes);
    deps.asset_hashes = Arc::new(asset_hashes.clone());
    deps.theme_dir = theme_dir.to_path_buf();

    let mut hb = Handlebars::new();
    hb.set_prevent_indent(true);
    let blocks: helpers::BlockStore = Arc::new(std::sync::Mutex::new(HashMap::new()));
    helpers::register_all(&mut hb, &deps, translations, blocks);

    // builtin partials first (theme overrides by registering later)
    register_builtin_partials(&mut hb);

    let mut layouts = HashMap::new();
    let mut templates = Vec::new();

    // theme partials — nested too: partials/icons/facebook.hbs registers as
    // "icons/facebook", the name themes reference it by
    let partials_dir = theme_dir.join("partials");
    if partials_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&partials_dir) {
            let entry = entry.map_err(|e| GhostError::Io(std::io::Error::other(e.to_string())))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("hbs") {
                let rel = path.strip_prefix(&partials_dir).unwrap_or(path);
                let pname = rel.with_extension("").to_string_lossy().replace('\\', "/");
                let src = std::fs::read_to_string(path)?;
                // partials don't take layouts in express-hbs; drop any declaration
                let (_, body) = preprocess::extract_layout(&src);
                hb.register_partial(&pname, preprocess::prepare(&body))
                    .map_err(|e| GhostError::ThemeError(format!("partial {pname}: {e}")))?;
            }
        }
    }

    // theme templates
    for entry in walkdir::WalkDir::new(theme_dir).max_depth(1) {
        let entry = entry.map_err(|e| GhostError::Io(std::io::Error::other(e.to_string())))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("hbs") {
            continue;
        }
        let tname = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let src = std::fs::read_to_string(path)?;
        let (layout, body) = preprocess::extract_layout(&src);
        if let Some(l) = layout {
            layouts.insert(tname.clone(), l);
        }
        hb.register_template_string(&tname, preprocess::prepare(&body))
            .map_err(|e| GhostError::ThemeError(format!("template {tname}: {e}")))?;
        templates.push(tname.clone());

        // layouts referenced by templates are themselves templates
        if tname == "default" {
            layouts.insert(tname.clone(), "default".into());
        }
    }

    // a bare layout template renders itself through its own body → no layout
    if let Some(l) = layouts.get("default") {
        if l == "default" {
            layouts.remove("default");
        }
    }

    Ok(ThemeState {
        name,
        dir: theme_dir.to_path_buf(),
        pkg,
        hb,
        layouts,
        asset_hashes,
        templates,
    })
}

fn hash_assets(dir: &Path, prefix: &str, out: &mut HashMap<String, String>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.map_err(|e| GhostError::Io(std::io::Error::other(e.to_string())))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.strip_prefix(dir).unwrap_or(path);
        let key = format!("{prefix}{}", rel.to_string_lossy());
        let data = std::fs::read(path)?;
        let hash = hex::encode(Sha256::digest(&data));
        out.insert(key, hash);
    }
    Ok(())
}

/// Built-in partials — ports of ghost/core/core/frontend/helpers/tpl/ —
/// that a theme may override by shipping a partial of the same name.
fn register_builtin_partials(hb: &mut Handlebars<'_>) {
    for (name, source) in BUILTIN_PARTIALS {
        if let Err(e) = hb.register_partial(name, preprocess::prepare(source)) {
            tracing::error!("builtin partial {name} failed to compile: {e}");
        }
    }
}

const BUILTIN_PARTIALS: &[(&str, &str)] = &[
    (
        "navigation",
        r#"<ul class="nav">
    {{#foreach navigation}}
    <li class="{{link_class for=url class=(concat "nav-" slug)}}"><a href="{{url}}">{{label}}</a></li>
    {{/foreach}}
</ul>"#,
    ),
    (
        "pagination",
        r#"<nav class="pagination">
    {{#if prev}}
        <a class="newer-posts" href="{{page_url prev}}"><span aria-hidden="true">&larr;</span> {{t "Newer Posts"}}</a>
    {{/if}}
    <span class="page-number">{{t "Page {page} of {totalPages}" page=page totalPages=pages}}</span>
    {{#if next}}
        <a class="older-posts" href="{{page_url next}}">{{t "Older Posts"}} <span aria-hidden="true">&rarr;</span></a>
    {{/if}}
</nav>"#,
    ),
    (
        "cancel_link",
        r#"{{#if cancel_at_period_end}}
    <a class="{{class}}" data-members-continue-subscription="{{id}}" href="javascript:">
        {{continueLabel}}
    </a>
{{else}}
    <a class="{{class}}" data-members-cancel-subscription="{{id}}" href="javascript:">
        {{cancelLabel}}
    </a>
{{/if}}

<span class="{{errorClass}}" data-members-error><!-- error message will appear here --></span>"#,
    ),
    (
        "recommendations",
        r#"{{#if recommendations}}
    <ul class="recommendations">
        {{#each recommendations as |rec|}}
        <li class="recommendation">
            <a href="{{rec.url}}" data-recommendation="{{rec.id}}" target="_blank" rel="noopener">
                <div class="recommendation-favicon">
                    {{#if rec.favicon}}
                        <img src="{{rec.favicon}}" alt="{{rec.title}}" loading="lazy" onerror="this.style.display='none';">
                    {{/if}}
                </div>
                <h5 class="recommendation-title">{{rec.title}}</h5>
                <span class="recommendation-url">{{readable_url rec.url}}</span>
                <p class="recommendation-description">{{rec.description}}</p>
            </a>
        </li>
        {{/each}}
    </ul>
{{/if}}"#,
    ),
    (
        "content-cta",
        r##"{{{html}}}
<aside class="gh-post-upgrade-cta">
    <div class="gh-post-upgrade-cta-content" style="background-color: {{@site.accent_color}}">
        {{#has visibility="paid"}}
            {{#is "page"}}
                <h2>{{t "This page is for paying subscribers only"}}</h2>
            {{else}}
                <h2>{{t "This post is for paying subscribers only"}}</h2>
            {{/is}}
        {{/has}}
        {{#has visibility="members"}}
            {{#is "page"}}
                <h2>{{t "This page is for subscribers only"}}</h2>
            {{else}}
                <h2>{{t "This post is for subscribers only"}}</h2>
            {{/is}}
        {{/has}}
        {{#if @member}}
            <a class="gh-btn" data-portal="account/plans" href="#/portal/account/plans" style="color:{{@site.accent_color}}">{{t "Upgrade your account"}}</a>
        {{else}}
            <a class="gh-btn" data-portal="signup" href="#/portal/signup" style="color:{{@site.accent_color}}">{{t "Subscribe now"}}</a>
            <p><small>{{t "Already have an account?"}} <a data-portal="signin" href="#/portal/signin">{{t "Sign in"}}</a></small></p>
        {{/if}}
    </div>
</aside>"##,
    ),
];

/// A single hash standing in for "the theme's assets", used to cache-bust
/// asset paths that have no file of their own to hash.
fn global_asset_hash(asset_hashes: &HashMap<String, String>) -> String {
    let mut names: Vec<&String> = asset_hashes.keys().collect();
    names.sort();
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update(asset_hashes[name].as_bytes());
    }
    hex::encode(hasher.finalize())[..10].to_string()
}

/// List available themes (directories with package.json or index.hbs).
pub fn list_themes(themes_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(themes_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            out.push((name, path));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> ThemePkg {
        serde_json::from_str(raw).expect("package.json")
    }

    #[test]
    fn reads_ghosts_image_sizes_map() {
        let pkg = parse(
            r#"{"name":"acceler","config":{"posts_per_page":9,
               "image_sizes":{"xxs":{"width":30},"m":{"width":720,"height":400}}}}"#,
        );
        assert_eq!(pkg.config.posts_per_page, 9);

        let mut sizes = pkg.config.image_sizes.clone();
        sizes.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(sizes.len(), 2);
        assert_eq!((sizes[0].name.as_str(), sizes[0].width, sizes[0].height), ("m", 720, Some(400)));
        assert_eq!((sizes[1].name.as_str(), sizes[1].width, sizes[1].height), ("xxs", 30, None));
    }

    #[test]
    fn accepts_image_sizes_array_too() {
        let pkg = parse(r#"{"name":"t","config":{"image_sizes":[{"name":"s","width":300}]}}"#);
        assert_eq!(pkg.config.image_sizes.len(), 1);
        assert_eq!(pkg.config.image_sizes[0].name, "s");
    }

    #[test]
    fn config_defaults_survive_odd_shapes() {
        for raw in [
            r#"{"name":"t"}"#,
            r#"{"name":"t","config":{}}"#,
            r#"{"name":"t","config":[]}"#,
            r#"{"name":"t","config":{"posts_per_page":"many"}}"#,
            r#"{"name":"t","config":{"image_sizes":"nope"}}"#,
        ] {
            let pkg = parse(raw);
            assert_eq!(pkg.config.posts_per_page, 5, "{raw}");
            assert!(pkg.config.image_sizes.is_empty(), "{raw}");
            assert!(pkg.config.card_assets.any(), "{raw}");
        }
    }

    #[test]
    fn missing_name_does_not_fail() {
        assert_eq!(parse(r#"{"version":"1.0.0"}"#).name, "");
    }
}
