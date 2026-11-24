//! Tools → Page Builder & Visual Editor: the rules.
//!
//! A layout is a tree of **sections → columns → widgets** (a widget of type
//! `inner` holds one more level of columns), stored as JSON beside the post
//! it belongs to (`builder_layouts`) or as a template (`builder_templates`:
//! saved sections and pages, global widgets, and the Theme Builder's
//! headers, footers, single-post templates and popups).
//!
//! * [`spec`] — every setting of every element, once: its type, label, group,
//!   whether it varies per device, and when it is shown. The editor's panels
//!   are generated from it ([`spec::spec_json`]) and [`spec::normalise`]
//!   rebuilds the renderable layout before it is stored or drawn. Elementor
//!   source data is retained separately for validated styling and JSON export.
//! * [`render`] — layout → HTML plus one generated stylesheet of per-element
//!   rules (responsive values, hover states, typography, borders, shadows,
//!   motion). Every text is escaped; the Text widget keeps a small set of
//!   inline tags; the HTML widget and custom CSS are the deliberate,
//!   staff-only exceptions (as the editor's HTML card and code injection are).
//! * [`dynamic`] — what can only be known per request: display conditions,
//!   dynamic tags (`[[post.title]]`) and dynamic widgets (posts, menus,
//!   breadcrumbs…), left in the stored HTML as markers that `vuuut_front`
//!   resolves on each view.
//!
//! [`CSS`] and [`RUNTIME`] are served to the site at `/public/vuuut-builder.*`.

pub mod dynamic;
pub mod elementor;
pub mod icons;
pub mod parts;
pub mod render;
pub mod spec;
mod widgets;

pub use icons::{icon_svg, ICONS};
pub use render::{render, render_with, Ctx};
pub use spec::{normalise, Field, WIDGETS};

use serde_json::{json, Value};

/// The stylesheet shared by the public site and the editor's canvas.
pub const CSS: &str = include_str!("builder.css");
/// The small script that drives carousels, countdowns, tabs, popups, motion…
pub const RUNTIME: &str = include_str!("builder.js");

pub const SETTING_ENABLED: &str = "builder_enabled";
pub const SETTING_POST_TYPES: &str = "builder_post_types";
pub const SETTING_WHO: &str = "builder_who";
pub const SETTING_WIDTH: &str = "builder_container_width";
pub const SETTING_SPACE: &str = "builder_section_space";
pub const SETTING_RADIUS: &str = "builder_radius";
pub const SETTING_PALETTE: &str = "builder_palette";
pub const SETTING_FONT: &str = "builder_heading_font";
pub const SETTING_BODY_FONT: &str = "builder_body_font";
pub const SETTING_GOOGLE_FONTS: &str = "builder_google_fonts";
pub const SETTING_CUSTOM_FONT_NAME: &str = "builder_custom_font_name";
pub const SETTING_CUSTOM_FONT_URL: &str = "builder_custom_font_url";
pub const SETTING_ANIMATIONS: &str = "builder_animations";
pub const SETTING_HIDE_HEADER: &str = "builder_theme_header_selector";
pub const SETTING_HIDE_FOOTER: &str = "builder_theme_footer_selector";
pub const SETTING_MAINTENANCE: &str = "builder_maintenance";
pub const SETTING_HOMEPAGE: &str = "builder_homepage";
pub const SETTING_MAINTENANCE_PAGE: &str = "builder_maintenance_page";

/// Every settings key this tool owns.
pub const SETTING_KEYS: &[&str] = &[
    SETTING_ENABLED, SETTING_POST_TYPES, SETTING_WHO, SETTING_WIDTH, SETTING_SPACE, SETTING_RADIUS,
    SETTING_PALETTE, SETTING_FONT, SETTING_BODY_FONT, SETTING_GOOGLE_FONTS, SETTING_CUSTOM_FONT_NAME,
    SETTING_CUSTOM_FONT_URL, SETTING_ANIMATIONS, SETTING_HIDE_HEADER, SETTING_HIDE_FOOTER,
    SETTING_MAINTENANCE, SETTING_MAINTENANCE_PAGE, SETTING_HOMEPAGE,
];

pub const DEFAULT_HIDE_HEADER: &str = "#gh-head, .gh-head, .site-header, body > header, body > nav";
pub const DEFAULT_HIDE_FOOTER: &str = ".gh-foot, .gh-footer, .site-footer, body > footer";

/// The global colours, in order, with the names the editor shows. The first
/// one, left blank, follows the site's accent colour.
pub const PALETTE_NAMES: [&str; 6] = ["Primary", "Secondary", "Text", "Muted", "Light", "White"];
pub const DEFAULT_PALETTE: [&str; 6] = ["", "#7c4dff", "#15171a", "#6b7280", "#f5f5f7", "#ffffff"];

pub const WIDTHS: &[u32] = &[960, 1040, 1140, 1240, 1320, 1440];
pub const SPACES: &[u32] = &[40, 56, 64, 80, 96, 120];
pub const RADII: &[u32] = &[0, 4, 8, 12, 999];

/// Typefaces: `(key, label, CSS stack, Google Fonts family)`. A Google font
/// is only requested from Google when a page uses it and the site allows it.
pub const FONTS: &[(&str, &str, &str, &str)] = &[
    ("theme", "The theme's font", "inherit", ""),
    ("sans", "System sans-serif", "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif", ""),
    ("serif", "System serif", "ui-serif, Georgia, Cambria, 'Times New Roman', Times, serif", ""),
    ("rounded", "System rounded", "ui-rounded, 'SF Pro Rounded', system-ui, sans-serif", ""),
    ("mono", "Monospace", "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace", ""),
    ("custom", "Your custom font", "var(--vb-custom-font, inherit)", ""),
    ("inter", "Inter", "'Inter', sans-serif", "Inter:wght@400;500;600;700;800"),
    ("roboto", "Roboto", "'Roboto', sans-serif", "Roboto:wght@400;500;700;900"),
    ("open-sans", "Open Sans", "'Open Sans', sans-serif", "Open+Sans:wght@400;600;700;800"),
    ("lato", "Lato", "'Lato', sans-serif", "Lato:wght@400;700;900"),
    ("montserrat", "Montserrat", "'Montserrat', sans-serif", "Montserrat:wght@400;500;600;700;800"),
    ("poppins", "Poppins", "'Poppins', sans-serif", "Poppins:wght@400;500;600;700;800"),
    ("dm-sans", "DM Sans", "'DM Sans', sans-serif", "DM+Sans:wght@400;500;700"),
    ("space-grotesk", "Space Grotesk", "'Space Grotesk', sans-serif", "Space+Grotesk:wght@400;500;700"),
    ("outfit", "Outfit", "'Outfit', sans-serif", "Outfit:wght@400;500;600;700;800"),
    ("work-sans", "Work Sans", "'Work Sans', sans-serif", "Work+Sans:wght@400;500;600;700"),
    ("raleway", "Raleway", "'Raleway', sans-serif", "Raleway:wght@400;500;600;700;800"),
    ("nunito", "Nunito", "'Nunito', sans-serif", "Nunito:wght@400;600;700;800"),
    ("oswald", "Oswald", "'Oswald', sans-serif", "Oswald:wght@400;500;600;700"),
    ("bebas", "Bebas Neue", "'Bebas Neue', sans-serif", "Bebas+Neue"),
    ("playfair", "Playfair Display", "'Playfair Display', serif", "Playfair+Display:wght@400;600;700;800"),
    ("merriweather", "Merriweather", "'Merriweather', serif", "Merriweather:wght@400;700;900"),
    ("lora", "Lora", "'Lora', serif", "Lora:wght@400;500;600;700"),
    ("fraunces", "Fraunces", "'Fraunces', serif", "Fraunces:wght@400;600;700;800"),
    ("source-serif", "Source Serif 4", "'Source Serif 4', serif", "Source+Serif+4:wght@400;600;700"),
    ("rubik", "Rubik", "'Rubik', sans-serif", "Rubik:wght@400;500;600;700;800"),
    ("plus-jakarta-sans", "Plus Jakarta Sans", "'Plus Jakarta Sans', sans-serif", "Plus+Jakarta+Sans:wght@400;500;600;700;800"),
    ("karla", "Karla", "'Karla', sans-serif", "Karla:wght@300;400;500;600;700;800"),
    ("archivo", "Archivo", "'Archivo', sans-serif", "Archivo:wght@400;500;600;700;800"),
    ("oxygen", "Oxygen", "'Oxygen', sans-serif", "Oxygen:wght@300;400;700"),
    ("bricolage-grotesque", "Bricolage Grotesque", "'Bricolage Grotesque', sans-serif", "Bricolage+Grotesque:wght@400;500;600;700;800"),
    ("barlow", "Barlow", "'Barlow', sans-serif", "Barlow:wght@400;500;600;700;800"),
    ("trispace", "Trispace", "'Trispace', sans-serif", "Trispace:wght@400;500;600;700;800"),
    ("manrope", "Manrope", "'Manrope', sans-serif", "Manrope:wght@400;500;600;700;800"),
    ("quicksand", "Quicksand", "'Quicksand', sans-serif", "Quicksand:wght@400;500;600;700"),
    ("amiri", "Amiri", "'Amiri', serif", "Amiri:wght@400;700"),
    ("sora", "Sora", "'Sora', sans-serif", "Sora:wght@400;500;600;700;800"),
    ("source-sans-3", "Source Sans 3", "'Source Sans 3', sans-serif", "Source+Sans+3:wght@400;500;600;700;800"),
    ("baloo-2", "Baloo 2", "'Baloo 2', sans-serif", "Baloo+2:wght@400;500;600;700;800"),
    ("pt-serif", "PT Serif", "'PT Serif', serif", "PT+Serif:wght@400;700"),
    ("salsa", "Salsa", "'Salsa', sans-serif", "Salsa"),
    ("port-lligat-sans", "Port Lligat Sans", "'Port Lligat Sans', sans-serif", "Port+Lligat+Sans"),
    ("sanchez", "Sanchez", "'Sanchez', serif", "Sanchez"),
    ("akshar", "Akshar", "'Akshar', sans-serif", "Akshar:wght@400;500;600;700"),
    ("preahvihear", "Preahvihear", "'Preahvihear', sans-serif", "Preahvihear"),
    ("cinzel", "Cinzel", "'Cinzel', serif", "Cinzel:wght@400;500;600;700;800;900"),
    ("cormorant-garamond", "Cormorant Garamond", "'Cormorant Garamond', serif", "Cormorant+Garamond:wght@400;500;600;700"),
    ("josefin-sans", "Josefin Sans", "'Josefin Sans', sans-serif", "Josefin+Sans:wght@400;500;600;700"),
    ("didact-gothic", "Didact Gothic", "'Didact Gothic', sans-serif", "Didact+Gothic"),
    ("pinyon-script", "Pinyon Script", "'Pinyon Script', cursive", "Pinyon+Script"),
    ("roboto-slab", "Roboto Slab", "'Roboto Slab', serif", "Roboto+Slab:wght@400;500;600;700;800"),
    ("nunito-sans", "Nunito Sans", "'Nunito Sans', sans-serif", "Nunito+Sans:wght@400;500;600;700;800"),
    ("ibm-plex-mono", "IBM Plex Mono", "'IBM Plex Mono', monospace", "IBM+Plex+Mono:wght@400;600"),
];

pub fn font_stack(key: &str) -> &'static str {
    FONTS.iter().find(|f| f.0 == key).map(|f| f.2).unwrap_or("inherit")
}

pub fn google_family(key: &str) -> Option<&'static str> {
    FONTS.iter().find(|f| f.0 == key).map(|f| f.3).filter(|g| !g.is_empty())
}

/// Which posts can have a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostTypes {
    Pages,
    All,
}

/// Who may open the visual editor, on top of being allowed to edit the post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Everyone,
    Editors,
    Administrators,
}

impl Who {
    pub fn key(self) -> &'static str {
        match self {
            Who::Everyone => "everyone",
            Who::Editors => "editors",
            Who::Administrators => "administrators",
        }
    }
    pub fn from_key(k: &str) -> Self {
        match k {
            "editors" => Who::Editors,
            "administrators" => Who::Administrators,
            _ => Who::Everyone,
        }
    }
}

/// Site-wide maintenance: nobody but signed-in staff sees the site; visitors
/// get one builder page instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maintenance {
    Off,
    /// 200, `noindex`: a launch page.
    ComingSoon,
    /// 503 with `Retry-After`: search engines come back later.
    Maintenance,
}

impl Maintenance {
    pub fn key(self) -> &'static str {
        match self {
            Maintenance::Off => "off",
            Maintenance::ComingSoon => "coming_soon",
            Maintenance::Maintenance => "maintenance",
        }
    }
    pub fn from_key(k: &str) -> Self {
        match k {
            "coming_soon" => Maintenance::ComingSoon,
            "maintenance" => Maintenance::Maintenance,
            _ => Maintenance::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub post_types: PostTypes,
    pub who: Who,
    pub width: u32,
    pub space: u32,
    pub radius: u32,
    pub palette: [String; 6],
    pub font: String,
    pub body_font: String,
    pub google_fonts: bool,
    pub custom_font_name: String,
    pub custom_font_url: String,
    pub animations: bool,
    pub hide_header: String,
    pub hide_footer: String,
    pub maintenance: Maintenance,
    pub maintenance_page: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::read(|_| None)
    }
}

fn nearest(list: &[u32], v: Option<u32>, default: u32) -> u32 {
    match v {
        Some(v) if list.contains(&v) => v,
        _ => default,
    }
}

/// A CSS selector list a setting may hold: no braces, no `<`, no `@`, so it
/// can only ever be a selector.
pub fn clean_selector(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '{' | '}' | '<' | '>' | '@' | ';' | '\\') || *c == '>').take(400).collect::<String>().trim().to_string()
}

impl Settings {
    /// Read from any key → string source (the settings cache, or a posted
    /// form). Anything missing or out of range falls back to the default.
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |k: &str, d: bool| get(k).map(|v| v.trim() == "true" || v.trim() == "1").unwrap_or(d);
        let num = |k: &str| get(k).and_then(|v| v.trim().trim_matches('"').parse::<f64>().ok()).map(|f| f as u32);
        let raw_palette = get(SETTING_PALETTE).unwrap_or_default();
        let parts: Vec<&str> = raw_palette.split(',').collect();
        let palette: [String; 6] = std::array::from_fn(|i| {
            let p = parts.get(i).map(|s| s.trim()).unwrap_or("");
            if parts.len() == 6 {
                if i == 0 && p.is_empty() {
                    String::new()
                } else {
                    hex_colour(p).unwrap_or_else(|| DEFAULT_PALETTE[i].to_string())
                }
            } else {
                DEFAULT_PALETTE[i].to_string()
            }
        });
        let font_of = |k: &str| {
            let f = get(k).unwrap_or_default();
            if FONTS.iter().any(|x| x.0 == f) { f } else { "theme".into() }
        };
        let custom_url = get(SETTING_CUSTOM_FONT_URL).unwrap_or_default();
        let selector = |k: &str, d: &str| match get(k) {
            Some(v) => clean_selector(&v),
            None => d.to_string(),
        };
        Settings {
            enabled: flag(SETTING_ENABLED, true),
            post_types: if get(SETTING_POST_TYPES).as_deref() == Some("pages") { PostTypes::Pages } else { PostTypes::All },
            who: Who::from_key(get(SETTING_WHO).as_deref().unwrap_or("")),
            width: nearest(WIDTHS, num(SETTING_WIDTH), 1140),
            space: nearest(SPACES, num(SETTING_SPACE), 80),
            radius: nearest(RADII, num(SETTING_RADIUS), 8),
            palette,
            font: font_of(SETTING_FONT),
            body_font: font_of(SETTING_BODY_FONT),
            google_fonts: flag(SETTING_GOOGLE_FONTS, true),
            custom_font_name: get(SETTING_CUSTOM_FONT_NAME).unwrap_or_default().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').take(60).collect(),
            custom_font_url: if custom_url.starts_with("https://") || custom_url.starts_with("/content/") { custom_url.chars().filter(|c| !matches!(c, '\'' | '"' | ')' | '(' | '\\' | ' ')).collect() } else { String::new() },
            animations: flag(SETTING_ANIMATIONS, true),
            hide_header: selector(SETTING_HIDE_HEADER, DEFAULT_HIDE_HEADER),
            hide_footer: selector(SETTING_HIDE_FOOTER, DEFAULT_HIDE_FOOTER),
            maintenance: Maintenance::from_key(get(SETTING_MAINTENANCE).as_deref().unwrap_or("")),
            maintenance_page: get(SETTING_MAINTENANCE_PAGE).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).take(40).collect(),
        }
    }

    pub fn palette_value(&self) -> String {
        self.palette.join(",")
    }

    /// Whether a post of this type may have a layout.
    pub fn allows(&self, post_type: &str) -> bool {
        self.enabled && (self.post_types == PostTypes::All || post_type == "page")
    }

    /// The custom properties every layout is drawn with.
    pub fn css_vars(&self) -> String {
        let mut out = String::new();
        for (i, c) in self.palette.iter().enumerate() {
            let v = if c.is_empty() { "var(--ghost-accent-color, #1f4e79)".to_string() } else { c.clone() };
            out.push_str(&format!("--vb-c{}:{v};", i + 1));
        }
        out.push_str(&format!(
            "--vb-w:{}px;--vb-sp:{}px;--vb-r:{}px;--vb-hf:{};--vb-bf:{};",
            self.width,
            self.space,
            self.radius,
            font_stack(&self.font),
            font_stack(&self.body_font)
        ));
        if !self.custom_font_name.is_empty() {
            out.push_str(&format!("--vb-custom-font:'{}';", self.custom_font_name));
        }
        out
    }

    /// `<link>`/`<style>` needed for the site-wide fonts plus `extra` font
    /// keys a layout uses.
    pub fn font_head(&self, extra: &[String]) -> String {
        let mut out = String::new();
        if !self.custom_font_name.is_empty() && !self.custom_font_url.is_empty() {
            out.push_str(&format!(
                "<style>@font-face{{font-family:'{}';src:url('{}');font-display:swap}}</style>",
                self.custom_font_name, self.custom_font_url
            ));
        }
        if !self.google_fonts {
            return out;
        }
        let mut fams: Vec<&str> = Vec::new();
        for k in [self.font.as_str(), self.body_font.as_str()].into_iter().chain(extra.iter().map(String::as_str)) {
            if let Some(g) = google_family(k) {
                if !fams.contains(&g) {
                    fams.push(g);
                }
            }
        }
        if !fams.is_empty() {
            let q: Vec<String> = fams.iter().map(|f| format!("family={f}")).collect();
            out.push_str(&format!(
                r#"<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin><link rel="stylesheet" href="https://fonts.googleapis.com/css2?{}&amp;display=swap">"#,
                q.join("&amp;")
            ));
        }
        out
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`, lowercased.
pub fn hex_colour(s: &str) -> Option<String> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    if matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(format!("#{}", hex.to_ascii_lowercase()))
    } else {
        None
    }
}

/// A link a visitor may follow: http(s), mailto, tel, a site path, an
/// in-page anchor, Portal's `#/portal/…`, or a dynamic link tag.
pub fn safe_url(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 2000 {
        return None;
    }
    if let Some(tag) = s.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        return dynamic::URL_TAGS.contains(&tag).then(|| s.to_string());
    }
    let squashed: String = s.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    let ok = squashed.starts_with("https://")
        || squashed.starts_with("http://")
        || squashed.starts_with("mailto:")
        || squashed.starts_with("tel:")
        || squashed.starts_with("sms:")
        || squashed.starts_with('#')
        || (squashed.starts_with('/') && !squashed.starts_with("//"));
    ok.then(|| s.to_string())
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// How big a layout is, for the overview.
pub fn counts(layout: &Value) -> (usize, usize) {
    fn widgets(cols: &Value) -> usize {
        cols.as_array()
            .map(|cs| {
                cs.iter()
                    .flat_map(|c| c["items"].as_array().cloned().unwrap_or_default())
                    .map(|w| 1 + if w["t"] == "inner" { widgets(&w["cols"]) } else { 0 })
                    .sum()
            })
            .unwrap_or(0)
    }
    let sections = layout["sections"].as_array().map(Vec::len).unwrap_or(0);
    let n = layout["sections"].as_array().map(|s| s.iter().map(|s| widgets(&s["cols"])).sum()).unwrap_or(0);
    (sections, n)
}

/// The words on a builder page, for excerpts and search.
pub fn plain_text(layout: &Value) -> String {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if m.get("t").is_some_and(|kind| kind == "elementor-widget") {
                    return;
                }
                for (k, v) in m {
                    if k == "elementor" {
                        continue;
                    }
                    if matches!(k.as_str(), "text" | "title" | "quote" | "q" | "a" | "label" | "caption" | "before" | "after" | "words" | "heading" | "desc") {
                        if let Some(s) = v.as_str() {
                            out.push(s.to_string());
                            continue;
                        }
                    }
                    if k == "html" {
                        if let Some(s) = v.as_str() {
                            out.push(crate::comments::to_plaintext(&crate::comments::sanitize_rich_html(s)));
                            continue;
                        }
                    }
                    walk(v, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    walk(&layout["sections"], &mut out);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
}

/// A fresh layout: one empty section with one column.
pub fn blank() -> Value {
    json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {}, "cols": [{"id": "c1", "s": {}, "items": []}]}]})
}

/// The layout a post starts with when it is first opened in the builder:
/// its title as a heading and its existing text as a Text widget.
pub fn starter(title: &str, html: &str) -> Value {
    let body = crate::comments::sanitize_rich_html(html);
    let mut items = vec![json!({"id": "w1", "t": "heading", "s": {"text": title, "tag": "h1"}})];
    if !body.trim().is_empty() {
        items.push(json!({"id": "w2", "t": "text", "s": {"html": body}}));
    }
    json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {}, "cols": [{"id": "c1", "s": {}, "items": items}]}]})
}

/// The starter layout for each Theme Builder part.
pub fn starter_for(kind: &str) -> Value {
    match kind {
        "header" => json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {"pt": 16, "pb": 16, "valign": "center", "tag": "header", "sticky": "top"}, "cols": [
            {"id": "c1", "s": {"w": 1}, "items": [{"id": "w1", "t": "site-logo", "s": {}}]},
            {"id": "c2", "s": {"w": 3, "align": "right"}, "items": [{"id": "w2", "t": "nav-menu", "s": {"align": "right"}}]},
            {"id": "c3", "s": {"w": 1, "align": "right"}, "items": [{"id": "w3", "t": "login", "s": {"align": "right"}}]}]}]}),
        "footer" => json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {"tag": "footer", "bg": "colour", "bg_colour": "c3", "colour": "#ffffff", "pt": 56, "pb": 40}, "cols": [
            {"id": "c1", "s": {}, "items": [{"id": "w1", "t": "site-logo", "s": {}}, {"id": "w2", "t": "text", "s": {"html": "<p>[[site.description]]</p>"}}]},
            {"id": "c2", "s": {}, "items": [{"id": "w3", "t": "nav-menu", "s": {"layout": "vertical", "menu": "secondary"}}]},
            {"id": "c3", "s": {}, "items": [{"id": "w4", "t": "signup", "s": {}}]}]},
            {"id": "s2", "s": {"tag": "div", "bg": "colour", "bg_colour": "c3", "colour": "#ffffff", "pt": 0, "pb": 24}, "cols": [{"id": "c4", "s": {}, "items": [{"id": "w5", "t": "text", "s": {"html": "<p>© [[year]] [[site.title]]</p>", "size": 14}}]}]}]}),
        "single" => json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {"width": 760}, "cols": [{"id": "c1", "s": {}, "items": [
            {"id": "w1", "t": "heading", "s": {"text": "[[post.title]]", "tag": "h1"}},
            {"id": "w2", "t": "text", "s": {"html": "<p>[[post.author]] · [[post.date]] · [[post.reading_time]]</p>", "colour": "c4"}},
            {"id": "w3", "t": "image", "s": {"src": "[[post.feature_image]]", "radius": 16}},
            {"id": "w4", "t": "post-content", "s": {}},
            {"id": "w5", "t": "share", "s": {}},
            {"id": "w6", "t": "author-box", "s": {}},
            {"id": "w7", "t": "post-nav", "s": {}}]}]}]}),
        "popup" => json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {"pt": 40, "pb": 40}, "cols": [{"id": "c1", "s": {"align": "center"}, "items": [
            {"id": "w1", "t": "heading", "s": {"text": "Before you go", "tag": "h2", "align": "center"}},
            {"id": "w2", "t": "text", "s": {"html": "<p>Get the best of what we publish, once a week.</p>", "align": "center"}},
            {"id": "w3", "t": "signup", "s": {"align": "center"}}]}]}]}),
        "widget" => json!({"v": 2, "page": {}, "sections": [{"id": "s1", "s": {}, "cols": [{"id": "c1", "s": {}, "items": [{"id": "w1", "t": "button", "s": {"text": "Get started", "href": "#/portal/signup"}}]}]}]}),
        _ => blank(),
    }
}

/// Theme Builder and library kinds.
pub const TEMPLATE_KINDS: &[(&str, &str)] = &[
    ("section", "Section"),
    ("page", "Page"),
    ("widget", "Global widget"),
    ("header", "Header"),
    ("footer", "Footer"),
    ("single", "Single post template"),
    ("popup", "Popup"),
];

pub fn is_theme_part(kind: &str) -> bool {
    matches!(kind, "header" | "footer" | "single" | "popup")
}

#[cfg(test)]
mod tests;
