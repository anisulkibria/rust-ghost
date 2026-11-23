//! Layout → HTML, plus one generated stylesheet.
//!
//! Every element's look is written as rules against its `data-vb-id`, scoped
//! by a class unique to the layout (`.vb-L<scope>`), so a header template
//! and the page under it can both have a `w1` without touching each other.
//! Responsive values become media-query rules (tablet ≤1024px, mobile
//! ≤640px), hover values `:hover` rules. Nothing a layout holds reaches a
//! `style` attribute or the stylesheet without having passed `normalise`
//! (numbers clamped, colours and URLs checked) — the one exception is the
//! staff-written custom CSS, which is code injection by another name.

use super::dynamic;
use super::spec::{self, TYPO};
use super::{esc, font_stack, google_family, Settings};
use serde_json::Value;
use std::collections::HashMap;

pub(super) const TABLET: &str = "@media (max-width:1024px)";
pub(super) const MOBILE: &str = "@media (max-width:640px)";

/// What a render needs besides the layout.
pub struct Ctx<'a> {
    pub settings: &'a Settings,
    /// Unique per layout: the post or template id.
    pub scope: &'a str,
    /// Drawing for the editor's canvas: placeholders for empty columns,
    /// draggable widgets, every element visible regardless of conditions.
    pub editing: bool,
    /// Saved templates by id, for the Template, Global and Off-canvas widgets.
    pub templates: &'a HashMap<String, Value>,
}

/// The stylesheet being written, and what the page will need loaded.
pub(super) struct R<'a> {
    pub ctx: &'a Ctx<'a>,
    base: String,
    tablet: String,
    mobile: String,
    pub fonts: Vec<String>,
    pub depth: u8,
    pub uses_runtime: bool,
}

impl<'a> R<'a> {
    pub fn sel(&self, id: &str) -> String {
        format!(".vb-L{} [data-vb-id=\"{}\"]", self.ctx.scope, id)
    }
    /// One rule; `dev` is `""`, `"t"` or `"m"`.
    pub fn rule(&mut self, dev: &str, sel: &str, decl: &str) {
        if decl.is_empty() {
            return;
        }
        let buf = match dev {
            "t" => &mut self.tablet,
            "m" => &mut self.mobile,
            _ => &mut self.base,
        };
        buf.push_str(sel);
        buf.push('{');
        buf.push_str(decl);
        buf.push('}');
    }
    /// A responsive setting: `decl(value)` for each device it has a value on.
    pub fn resp(&mut self, m: &Value, key: &str, sel: &str, decl: impl Fn(&Value) -> String) {
        for (dev, k) in [("", key.to_string()), ("t", format!("{key}_t")), ("m", format!("{key}_m"))] {
            if let Some(v) = m.get(&k).filter(|v| !v.is_null()) {
                let d = decl(v);
                self.rule(dev, sel, &d);
            }
        }
    }
    pub fn raw(&mut self, css: &str) {
        self.base.push_str(css);
    }
    pub fn font(&mut self, key: &str) -> &'static str {
        if google_family(key).is_some() && !self.fonts.iter().any(|f| f == key) {
            self.fonts.push(key.to_string());
        }
        font_stack(key)
    }
    fn css(&self) -> String {
        let mut out = self.base.clone();
        if !self.tablet.is_empty() {
            out.push_str(&format!("{TABLET}{{{}}}", self.tablet));
        }
        if !self.mobile.is_empty() {
            out.push_str(&format!("{MOBILE}{{{}}}", self.mobile));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// value helpers
// ---------------------------------------------------------------------------

pub(super) fn s<'v>(m: &'v Value, k: &str) -> &'v str {
    m[k].as_str().unwrap_or("")
}
pub(super) fn n(m: &Value, k: &str) -> Option<i64> {
    m[k].as_i64()
}
pub(super) fn b(m: &Value, k: &str) -> bool {
    m[k].as_bool().unwrap_or(false)
}

/// A stored colour as CSS.
pub fn css_colour(s: &str) -> String {
    if s.len() == 2 && s.starts_with('c') {
        format!("var(--vb-{s})")
    } else {
        s.to_string()
    }
}
pub(super) fn colour(m: &Value, k: &str) -> Option<String> {
    m[k].as_str().map(css_colour)
}
pub(super) fn px(v: &Value) -> String {
    v.as_i64().map(|n| format!("{n}px")).unwrap_or_default()
}
pub(super) fn sides(v: &Value) -> String {
    v.as_str().map(|s| s.split_whitespace().map(|p| format!("{p}px")).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}
/// A URL for `url('…')` in CSS: stored URLs are already checked; quotes and
/// parentheses are encoded so it cannot end the token.
pub(super) fn css_url(u: &str) -> String {
    u.replace('\'', "%27").replace('(', "%28").replace(')', "%29").replace('\\', "%5C")
}
/// A link or image address for an attribute, dynamic tags left as
/// placeholders for the request-time pass.
pub(super) fn href(u: &str) -> String {
    match u.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        Some(tag) => dynamic::placeholder(tag),
        None => esc(u),
    }
}
/// Plain text for display: escaped, line breaks kept, dynamic tags marked.
pub(super) fn text(t: &str) -> String {
    dynamic::mark_tags(&esc(t).replace('\n', "<br>"))
}
pub(super) fn paragraphs(t: &str) -> String {
    t.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).map(|p| format!("<p>{}</p>", text(p))).collect()
}

pub(super) fn shadow(kind: &str) -> &'static str {
    match kind {
        "sm" => "0 1px 2px rgba(0,0,0,.06),0 4px 12px -4px rgba(0,0,0,.12)",
        "md" => "0 2px 4px rgba(0,0,0,.06),0 12px 28px -10px rgba(0,0,0,.22)",
        "lg" => "0 4px 8px rgba(0,0,0,.06),0 24px 48px -16px rgba(0,0,0,.3)",
        "xl" => "0 8px 16px rgba(0,0,0,.08),0 40px 80px -24px rgba(0,0,0,.4)",
        "inner" => "inset 0 2px 10px rgba(0,0,0,.15)",
        "glow" => "0 0 0 1px color-mix(in srgb,var(--vb-c1) 40%,transparent),0 10px 40px -8px color-mix(in srgb,var(--vb-c1) 60%,transparent)",
        _ => "none",
    }
}

// ---------------------------------------------------------------------------
// shared element styling
// ---------------------------------------------------------------------------

/// Classes and attributes every element can carry.
pub(super) struct Common {
    pub classes: String,
    pub attrs: String,
}

/// The box group: background, border, rounding, shadow, and their hovers.
pub(super) fn box_style(r: &mut R, m: &Value, sel: &str) {
    let mut d = String::new();
    match s(m, "box_bg") {
        "colour" => {
            if let Some(c) = colour(m, "box_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        "gradient" => d.push_str(&format!(
            "background-image:linear-gradient({}deg,{},{});",
            n(m, "box_angle").unwrap_or(135),
            colour(m, "box_colour").unwrap_or_else(|| "var(--vb-c1)".into()),
            colour(m, "box_colour2").unwrap_or_else(|| "var(--vb-c2)".into())
        )),
        "image" => {
            if let Some(u) = m["box_image"].as_str() {
                d.push_str(&format!("background:url('{}') center/cover no-repeat;", css_url(&href(u))));
            }
            if let Some(c) = colour(m, "box_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        _ => {}
    }
    let bs = s(m, "border_style");
    if !bs.is_empty() && bs != "none" {
        d.push_str(&format!("border-style:{bs};border-width:{};border-color:{};", if m["border_w"].is_string() { sides(&m["border_w"]) } else { "1px".into() }, colour(m, "border_colour").unwrap_or_else(|| "currentColor".into())));
    }
    if m["radius_box"].is_string() {
        d.push_str(&format!("border-radius:{};overflow:hidden;", sides(&m["radius_box"])));
    }
    let sh = s(m, "shadow");
    if !sh.is_empty() && sh != "none" {
        d.push_str(&format!("box-shadow:{};", shadow(sh)));
    }
    let mut hover = String::new();
    if let Some(c) = colour(m, "box_hover") {
        hover.push_str(&format!("background-color:{c};background-image:none;"));
    }
    if let Some(c) = colour(m, "border_hover") {
        hover.push_str(&format!("border-color:{c};"));
    }
    let shh = s(m, "shadow_hover");
    if !shh.is_empty() {
        hover.push_str(&format!("box-shadow:{};", shadow(shh)));
    }
    if !hover.is_empty() {
        d.push_str("transition:background-color .25s,border-color .25s,box-shadow .25s,color .25s;");
        r.rule("", &format!("{sel}:hover"), &hover);
    }
    r.rule("", sel, &d);
}

/// Typography, on `target` (the widget, or the part of it holding text).
pub(super) fn typo(r: &mut R, m: &Value, target: &str) {
    if !TYPO.iter().any(|f| !f.key.is_empty() && !m[f.key].is_null()) && !TYPO.iter().any(|f| f.resp && (!m[format!("{}_t", f.key)].is_null() || !m[format!("{}_m", f.key)].is_null())) {
        return;
    }
    let mut d = String::new();
    if let Some(f) = m["font"].as_str() {
        let stack = r.font(f);
        d.push_str(&format!("font-family:{stack};"));
    }
    if !s(m, "fw").is_empty() {
        d.push_str(&format!("font-weight:{};", s(m, "fw")));
    }
    if !s(m, "tt").is_empty() {
        d.push_str(&format!("text-transform:{};", s(m, "tt")));
    }
    if !s(m, "fst").is_empty() {
        d.push_str(&format!("font-style:{};", s(m, "fst")));
    }
    if !s(m, "deco").is_empty() {
        d.push_str(&format!("text-decoration:{};", s(m, "deco")));
    }
    match s(m, "text_shadow") {
        "soft" => d.push_str("text-shadow:0 2px 12px rgba(0,0,0,.25);"),
        "hard" => d.push_str("text-shadow:2px 2px 0 rgba(0,0,0,.35);"),
        "glow" => d.push_str("text-shadow:0 0 18px color-mix(in srgb,currentColor 60%,transparent);"),
        _ => {}
    }
    r.rule("", target, &d);
    r.resp(m, "fs", target, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:{n}px;")).unwrap_or_default());
    r.resp(m, "lh", target, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("line-height:{};", n as f64 / 10.0)).unwrap_or_default());
    r.resp(m, "ls", target, |v| v.as_i64().map(|n| format!("letter-spacing:{}em;", n as f64 / 100.0)).unwrap_or_default());
    if let Some(c) = colour(m, "colour_hover") {
        r.rule("", &format!("{target}:hover"), &format!("color:{c};"));
        r.rule("", target, "transition:color .2s;");
    }
}

/// Advanced: spacing, size, position, motion, visibility, attributes, code.
pub(super) fn advanced(r: &mut R, m: &Value, id: &str, sel: &str) -> Common {
    let mut classes = String::new();
    let mut attrs = String::new();
    r.resp(m, "margin", sel, |v| format!("margin:{};", sides(v)));
    r.resp(m, "padding", sel, |v| format!("padding:{};", sides(v)));
    match s(m, "width_mode") {
        "full" => r.rule("", sel, "width:100%;align-self:stretch;"),
        "inline" => r.rule("", sel, "width:auto;align-self:flex-start;display:inline-block;"),
        "custom" => r.resp(m, "width_px", sel, |v| format!("width:{};max-width:100%;", px(v))),
        _ => {}
    }
    r.resp(m, "self_align", sel, |v| v.as_str().map(|a| format!("align-self:{};", match a { "start" => "flex-start", "end" => "flex-end", x => x })).unwrap_or_default());
    let mut d = String::new();
    if let Some(z) = n(m, "z") {
        d.push_str(&format!("z-index:{z};position:relative;"));
    }
    if let Some(o) = n(m, "opacity") {
        d.push_str(&format!("opacity:{};", o as f64 / 100.0));
    }
    if let Some(rot) = n(m, "rotate").filter(|v| *v != 0) {
        d.push_str(&format!("rotate:{rot}deg;"));
    }
    let pos = s(m, "position");
    if pos == "absolute" || pos == "fixed" {
        d.push_str(&format!("position:{pos};"));
        if !(m["z"].is_i64()) {
            d.push_str("z-index:5;");
        }
        let h = if s(m, "pos_h") == "right" { "right" } else { "left" };
        let v = if s(m, "pos_v") == "bottom" { "bottom" } else { "top" };
        r.resp(m, "pos_x", sel, |x| format!("{h}:{};", px(x)));
        r.resp(m, "pos_y", sel, |y| format!("{v}:{};", px(y)));
        if m["pos_x"].is_null() {
            d.push_str(&format!("{h}:0;"));
        }
        if m["pos_y"].is_null() {
            d.push_str(&format!("{v}:0;"));
        }
    }
    match s(m, "sticky") {
        "top" | "bottom" => {
            let side = s(m, "sticky");
            d.push_str(&format!("position:sticky;{side}:{}px;", n(m, "sticky_offset").unwrap_or(0)));
            if !(m["z"].is_i64()) {
                d.push_str("z-index:20;");
            }
        }
        _ => {}
    }
    r.rule("", sel, &d);
    // entrance animation
    let anim = s(m, "anim");
    if r.ctx.settings.animations && !anim.is_empty() && anim != "none" {
        classes.push_str(&format!(" vb-anim vb-anim-{anim}"));
        let mut a = String::new();
        if let Some(v) = n(m, "anim_dur") {
            a.push_str(&format!("--vb-ad:{v}ms;"));
        }
        if let Some(v) = n(m, "anim_delay") {
            a.push_str(&format!("--vb-adl:{v}ms;"));
        }
        r.rule("", sel, &a);
        r.uses_runtime = true;
    }
    let hv = s(m, "hover_anim");
    if !hv.is_empty() && hv != "none" {
        classes.push_str(&format!(" vb-hv-{hv}"));
    }
    // scroll effects: one scroll-driven animation per element
    let (fy, fx, fade, scale, frot, blur) = (n(m, "fx_y").unwrap_or(0), n(m, "fx_x").unwrap_or(0), s(m, "fx_fade"), s(m, "fx_scale"), n(m, "fx_rotate").unwrap_or(0), n(m, "fx_blur").unwrap_or(0));
    if r.ctx.settings.animations && (fy != 0 || fx != 0 || !fade.is_empty() || !scale.is_empty() || frot != 0 || blur != 0) {
        let name = format!("vbfx-{}-{}", r.ctx.scope, id);
        let tf = |sign: i64| {
            let mut t = String::new();
            if fy != 0 || fx != 0 {
                t.push_str(&format!("translate({}px,{}px) ", fx * 12 * sign, fy * 12 * sign));
            }
            match (scale, sign) {
                ("up", -1) | ("down", 1) => t.push_str("scale(.85) "),
                ("up", 1) | ("down", -1) => t.push_str("scale(1.1) "),
                _ => {}
            }
            if frot != 0 {
                t.push_str(&format!("rotate({}deg) ", frot * sign / 2));
            }
            if t.is_empty() { "none".to_string() } else { t }
        };
        let (o0, o1, o2) = match fade {
            "in" => ("0", "1", "1"),
            "out" => ("1", "1", "0"),
            "in-out" => ("0", "1", "0"),
            _ => ("1", "1", "1"),
        };
        let bl = |v: i64| if blur > 0 { format!("filter:blur({}px);", v) } else { String::new() };
        r.raw(&format!(
            "@keyframes {name}{{0%{{transform:{};opacity:{o0};{}}}50%{{opacity:{o1};{}}}100%{{transform:{};opacity:{o2};{}}}}}",
            tf(1), bl(blur), bl(0), tf(-1), bl(blur)
        ));
        r.raw(&format!("@supports (animation-timeline:view()){{@media (prefers-reduced-motion:no-preference){{{sel}{{animation:{name} linear both;animation-timeline:view();animation-range:cover}}}}}}"));
    }
    match s(m, "fx_mouse") {
        "track" | "tilt" => {
            attrs.push_str(&format!(r#" data-vb-mouse="{}" data-vb-ms="{}""#, s(m, "fx_mouse"), n(m, "fx_mouse_s").unwrap_or(4)));
            r.uses_runtime = true;
        }
        _ => {}
    }
    for (k, c) in [("hide_d", " vb-hide-d"), ("hide_t", " vb-hide-t"), ("hide_m", " vb-hide-m")] {
        if b(m, k) {
            classes.push_str(c);
        }
    }
    if !s(m, "class").is_empty() {
        classes.push(' ');
        classes.push_str(&esc(s(m, "class")));
    }
    if !s(m, "anchor").is_empty() {
        attrs.push_str(&format!(r#" id="{}""#, esc(s(m, "anchor"))));
    }
    for a in m["attrs"].as_array().into_iter().flatten() {
        let name = s(a, "name");
        if spec::attr_allowed(name) {
            attrs.push_str(&format!(r#" {}="{}""#, esc(&name.to_ascii_lowercase()), esc(s(a, "value"))));
        }
    }
    if let Some(css) = m["css"].as_str().filter(|c| !c.trim().is_empty()) {
        r.raw(&css.replace("selector", sel).replace("</", "<\\/"));
    }
    Common { classes, attrs }
}

/// Wrap `html` in a display condition's markers, outside the editor.
pub(super) fn conditional(r: &R, m: &Value, id: &str, html: String) -> String {
    if r.ctx.editing {
        return html;
    }
    match dynamic::cond_of(m) {
        Some(c) => format!("{}{html}{}", dynamic::open_marker(id, &c), dynamic::close_marker(id)),
        None => html,
    }
}

// ---------------------------------------------------------------------------
// the layout
// ---------------------------------------------------------------------------

/// Draw a layout (already through `normalise`) for the site.
pub fn render(layout: &Value, settings: &Settings) -> String {
    let empty = HashMap::new();
    render_with(layout, &Ctx { settings, scope: "p", editing: false, templates: &empty })
}

/// Draw a layout with everything the context says.
pub fn render_with(layout: &Value, ctx: &Ctx) -> String {
    let mut r = R { ctx, base: String::new(), tablet: String::new(), mobile: String::new(), fonts: Vec::new(), depth: 0, uses_runtime: false };
    let body = sections_html(&mut r, layout["sections"].as_array().map(Vec::as_slice).unwrap_or(&[]));
    let page = &layout["page"];
    let mut style = ctx.settings.css_vars();
    if let Some(c) = colour(page, "bg_colour") {
        style.push_str(&format!("background:{c};"));
    }
    if let Some(c) = colour(page, "colour") {
        style.push_str(&format!("color:{c};"));
    }
    if let Some(f) = page["font"].as_str() {
        let st = r.font(f);
        style.push_str(&format!("--vb-bf:{st};"));
    }
    if let Some(f) = page["hfont"].as_str() {
        let st = r.font(f);
        style.push_str(&format!("--vb-hf:{st};"));
    }
    let mut css = r.css();
    if let Some(pc) = page["css"].as_str().filter(|c| !c.trim().is_empty()) {
        css.push_str(&pc.replace("</", "<\\/"));
    }
    let head = ctx.settings.font_head(&r.fonts);
    format!(
        r#"{head}<div class="vb-page vb-L{scope}{rt}" style="{style}">{css}{body}</div>"#,
        scope = esc(ctx.scope),
        rt = if r.uses_runtime { " vb-rt" } else { "" },
        style = esc(&style),
        css = if css.is_empty() { String::new() } else { format!("<style>{css}</style>") },
    )
}

pub(super) fn sections_html(r: &mut R, sections: &[Value]) -> String {
    sections.iter().map(|s| section(r, s)).collect()
}

fn shape_svg(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "wave" => r#"<path d="M0,60 C150,100 350,0 500,50 C650,100 850,10 1000,50 L1000,100 L0,100 Z"/>"#,
        "waves" => r#"<path opacity=".33" d="M0,40 C200,90 400,10 600,50 C800,90 900,30 1000,40 L1000,100 L0,100 Z"/><path opacity=".66" d="M0,60 C180,20 380,90 600,60 C800,30 900,80 1000,60 L1000,100 L0,100 Z"/><path d="M0,80 C250,50 450,100 700,75 C850,60 950,85 1000,80 L1000,100 L0,100 Z"/>"#,
        "curve" => r#"<path d="M0,100 C300,0 700,0 1000,100 Z"/>"#,
        "tilt" => r#"<path d="M0,100 L1000,0 L1000,100 Z"/>"#,
        "triangle" => r#"<path d="M0,100 L500,0 L1000,100 Z"/>"#,
        "arrow" => r#"<path d="M0,100 L0,70 L460,70 L500,20 L540,70 L1000,70 L1000,100 Z"/>"#,
        "zigzag" => r#"<path d="M0,100 L0,60 L50,30 L100,60 L150,30 L200,60 L250,30 L300,60 L350,30 L400,60 L450,30 L500,60 L550,30 L600,60 L650,30 L700,60 L750,30 L800,60 L850,30 L900,60 L950,30 L1000,60 L1000,100 Z"/>"#,
        "mountains" => r#"<path opacity=".4" d="M0,100 L0,50 L120,10 L260,60 L400,20 L560,70 L700,15 L860,60 L1000,25 L1000,100 Z"/><path d="M0,100 L0,70 L150,35 L300,75 L450,40 L600,80 L750,45 L900,75 L1000,55 L1000,100 Z"/>"#,
        "clouds" => r#"<path d="M0,100 L0,70 Q40,40 80,65 Q110,30 160,60 Q200,35 240,65 Q290,30 330,62 Q370,40 410,66 Q460,28 500,62 Q540,38 590,66 Q630,30 680,62 Q720,40 760,66 Q810,30 850,62 Q890,38 930,66 Q970,45 1000,62 L1000,100 Z"/>"#,
        "book" => r#"<path d="M0,100 C250,100 400,40 500,0 C600,40 750,100 1000,100 Z"/>"#,
        _ => return None,
    })
}

fn shape(r: &mut R, m: &Value, sel: &str, side: &str) -> String {
    let kind = s(m, &format!("shape_{side}"));
    let Some(paths) = shape_svg(kind) else { return String::new() };
    let fill = colour(m, &format!("shape_{side}_colour")).unwrap_or_else(|| "#fff".into());
    let ssel = format!("{sel} > .vb-shape-{side}");
    r.resp(m, &format!("shape_{side}_h"), &ssel, |v| format!("height:{};", px(v)));
    format!(
        r#"<div class="vb-shape vb-shape-{side}{flip}{front}" aria-hidden="true"><svg viewBox="0 0 1000 100" preserveAspectRatio="none" fill="{fill}">{paths}</svg></div>"#,
        flip = if b(m, &format!("shape_{side}_flip")) { " vb-flip" } else { "" },
        front = if b(m, &format!("shape_{side}_front")) { " vb-front" } else { "" },
        fill = esc(&fill),
    )
}

fn video_bg(url: &str) -> String {
    if let Some(id) = super::widgets::youtube_id(url) {
        return format!(r#"<div class="vb-bgvideo" aria-hidden="true"><iframe src="https://www.youtube-nocookie.com/embed/{id}?autoplay=1&amp;mute=1&amp;loop=1&amp;playlist={id}&amp;controls=0&amp;playsinline=1&amp;rel=0" title="" tabindex="-1" allow="autoplay; encrypted-media"></iframe></div>"#);
    }
    if let Some(id) = super::widgets::vimeo_id(url) {
        return format!(r#"<div class="vb-bgvideo" aria-hidden="true"><iframe src="https://player.vimeo.com/video/{id}?background=1&amp;autoplay=1&amp;loop=1&amp;muted=1" title="" tabindex="-1" allow="autoplay"></iframe></div>"#);
    }
    format!(r#"<div class="vb-bgvideo" aria-hidden="true"><video src="{}" autoplay muted loop playsinline></video></div>"#, href(url))
}

fn section(r: &mut R, sec: &Value) -> String {
    let id = s(sec, "id").to_string();
    let m = &sec["s"];
    let sel = r.sel(&id);
    let mut d = String::new();
    let mut extra = String::new();
    let bg = s(m, "bg");
    match bg {
        "colour" => {
            if let Some(c) = colour(m, "bg_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        "gradient" => {
            let a = colour(m, "bg_colour").unwrap_or_else(|| "var(--vb-c1)".into());
            let z = colour(m, "bg_colour2").unwrap_or_else(|| "var(--vb-c2)".into());
            if s(m, "bg_gtype") == "radial" {
                d.push_str(&format!("background-image:radial-gradient(circle at center,{a},{z});"));
            } else {
                d.push_str(&format!("background-image:linear-gradient({}deg,{a},{z});", n(m, "bg_angle").unwrap_or(135)));
            }
        }
        "image" => {
            r.resp(m, "bg_image", &sel, |v| v.as_str().map(|u| format!("background-image:url('{}');", css_url(&href(u)))).unwrap_or_default());
            d.push_str(&format!(
                "background-size:{};background-position:{};background-repeat:no-repeat;",
                match s(m, "bg_size") { "" => "cover", x => x },
                match s(m, "bg_pos") { "" => "center", x => x }
            ));
            if b(m, "bg_fixed") {
                d.push_str("background-attachment:fixed;");
            }
            if let Some(c) = colour(m, "bg_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        "video" => {
            if let Some(u) = m["bg_video"].as_str() {
                extra.push_str(&video_bg(u));
            }
            if let Some(c) = colour(m, "bg_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        "slideshow" => {
            let slides: Vec<&str> = m["bg_slides"].as_array().into_iter().flatten().filter_map(|x| x["src"].as_str()).collect();
            if !slides.is_empty() {
                let each = n(m, "slide_s").unwrap_or(6);
                let total = each * slides.len() as i64;
                let name = format!("vbsl-{}-{}", r.ctx.scope, id);
                let share = 100.0 / slides.len() as f64;
                r.raw(&format!("@keyframes {name}{{0%{{opacity:0}}{a:.2}%{{opacity:1}}{b:.2}%{{opacity:1}}{c:.2}%{{opacity:0}}100%{{opacity:0}}}}", a = share * 0.15, b = share, c = share * 1.15));
                extra.push_str(r#"<div class="vb-slideshow" aria-hidden="true">"#);
                for (i, u) in slides.iter().enumerate() {
                    extra.push_str(&format!(
                        r#"<div class="vb-slide-bg{kb}" style="background-image:url('{}');background-position:{};animation:{name} {total}s linear {}s infinite"></div>"#,
                        esc(&css_url(&href(u))),
                        match s(m, "bg_pos") { "" => "center", x => x },
                        each * i as i64,
                        kb = if b(m, "kenburns") { " vb-kenburns" } else { "" },
                    ));
                }
                extra.push_str("</div>");
            }
            if let Some(c) = colour(m, "bg_colour") {
                d.push_str(&format!("background-color:{c};"));
            }
        }
        _ => {}
    }
    if bg == "image" && b(m, "kenburns") {
        // the image lives on the section; a pseudo-layer zooms it
        d.push_str("--vb-kb:1;");
    }
    if let Some(c) = colour(m, "overlay") {
        let fill = match colour(m, "overlay2") {
            Some(z) => format!("linear-gradient(180deg,{c},{z})"),
            None => c,
        };
        extra.push_str(&format!(r#"<span class="vb-overlay" style="background:{};opacity:{}"></span>"#, esc(&fill), n(m, "overlay_opacity").unwrap_or(50) as f64 / 100.0));
    }
    if let Some(c) = colour(m, "colour") {
        d.push_str(&format!("color:{c};"));
    }
    if let Some(c) = colour(m, "link_colour") {
        r.rule("", &format!("{sel} a:not(.vb-btn)"), &format!("color:{c};"));
    }
    if b(m, "overflow") {
        d.push_str("overflow:hidden;");
    }
    let bs = s(m, "border_style");
    if !bs.is_empty() && bs != "none" {
        d.push_str(&format!("border-style:{bs};border-width:{};border-color:{};", if m["border_w"].is_string() { sides(&m["border_w"]) } else { "1px".into() }, colour(m, "border_colour").unwrap_or_else(|| "currentColor".into())));
    }
    if m["radius_box"].is_string() {
        d.push_str(&format!("border-radius:{};overflow:hidden;", sides(&m["radius_box"])));
    }
    if !s(m, "shadow").is_empty() && s(m, "shadow") != "none" {
        d.push_str(&format!("box-shadow:{};", shadow(s(m, "shadow"))));
    }
    r.rule("", &sel, &d);
    r.resp(m, "edge_padding", &sel, |v| {
        let padding = sides(v);
        format!("--vb-pt:{};--vb-pb:{};", padding.split_whitespace().next().unwrap_or("0px"), padding.split_whitespace().nth(2).unwrap_or("0px"))
    });
    r.resp(m, "pt", &sel, |v| v.as_i64().filter(|v| *v >= 0).map(|v| format!("--vb-pt:{v}px;")).unwrap_or_default());
    r.resp(m, "pb", &sel, |v| v.as_i64().filter(|v| *v >= 0).map(|v| format!("--vb-pb:{v}px;")).unwrap_or_default());
    r.resp(m, "min_h", &sel, |v| v.as_i64().filter(|v| *v > 0).map(|v| format!("min-height:{v}vh;")).unwrap_or_else(|| "min-height:0;".into()));
    let inner = format!("{sel} > .vb-in");
    r.resp(m, "edge_padding", &inner, |v| {
        let padding = sides(v);
        let mut edges = padding.split_whitespace();
        format!("padding-right:{};padding-left:{};", edges.nth(1).unwrap_or("0px"), edges.nth(1).unwrap_or("0px"))
    });
    if s(m, "layout") == "full" {
        r.rule("", &inner, "max-width:none;");
    } else {
        r.resp(m, "width", &inner, |v| v.as_i64().filter(|w| *w >= 200).map(|w| format!("max-width:{w}px;")).unwrap_or_default());
    }
    let row_sel = format!("{sel} > .vb-in > .vb-row");
    r.resp(m, "gap", &row_sel, |v| format!("--vb-gap:{};", px(v)));
    let top = shape(r, m, &sel, "top");
    let bottom = shape(r, m, &sel, "bottom");
    let common = advanced(r, m, &id, &sel);
    let row = row(r, &sec["cols"], m, &row_sel);
    let tag = match s(m, "tag") {
        "" => "section",
        t => t,
    };
    let html = format!(
        r#"<{tag} class="vb-s{cls}{ov}" data-vb-id="{id}"{attrs}>{extra}{top}<div class="vb-in">{row}</div>{bottom}</{tag}>"#,
        cls = common.classes,
        ov = if extra.contains("vb-overlay") || extra.contains("vb-bgvideo") || extra.contains("vb-slideshow") { " vb-has-ov" } else { "" },
        id = esc(&id),
        attrs = common.attrs,
    );
    conditional(r, m, &id, html)
}

/// A row of columns: a section's, or an inner section's.
pub(super) fn row(r: &mut R, cols: &Value, m: &Value, row_sel: &str) -> String {
    let mut cols: Vec<_> = cols.as_array().into_iter().flatten().collect();
    if b(m, "row_reverse") {
        cols.reverse();
    }
    let tpl = |dev: &str| -> Option<String> {
        let key = if dev.is_empty() { "w".to_string() } else { format!("w_{dev}") };
        if !dev.is_empty() && !cols.iter().any(|c| c["s"][&key].is_i64()) {
            return None;
        }
        Some(
            cols.iter()
                .map(|c| {
                    let w = c["s"][&key].as_i64().or_else(|| c["s"]["w"].as_i64()).unwrap_or(1).max(1);
                    format!("minmax(0,{w}fr)")
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    };
    let desktop = tpl("").unwrap_or_default();
    for dev in ["t", "m"] {
        if let Some(t) = tpl(dev) {
            r.rule(dev, row_sel, &format!("grid-template-columns:{t};display:grid;"));
        }
    }
    let mut rc = format!("vb-row vb-n{}", cols.len().min(6));
    rc.push_str(match s(m, "valign") {
        "center" => " vb-va-c",
        "bottom" => " vb-va-b",
        "stretch" => " vb-va-s",
        _ => "",
    });
    rc.push_str(match s(m, "stack") {
        "tablet" => " vb-stack-t",
        "never" => " vb-stack-n",
        _ => "",
    });
    if b(m, "reverse") {
        rc.push_str(" vb-rev");
    }
    let body: String = cols.iter().map(|c| column(r, c)).collect();
    format!(r#"<div class="{rc}" style="--vb-cols:{desktop}">{body}</div>"#)
}

fn column(r: &mut R, col: &Value) -> String {
    let id = s(col, "id").to_string();
    let m = &col["s"];
    let sel = r.sel(&id);
    let mut cls = String::from("vb-col");
    let mut d = String::new();
    if let Some(c) = colour(m, "colour") {
        d.push_str(&format!("color:{c};"));
    }
    if let Some(c) = colour(m, "link_colour") {
        r.rule("", &format!("{sel} a:not(.vb-btn)"), &format!("color:{c};"));
    }
    cls.push_str(match s(m, "valign") {
        "center" => " vb-cva-c",
        "bottom" => " vb-cva-b",
        "between" => " vb-cva-sb",
        _ => "",
    });
    if s(m, "box_bg") != "" && s(m, "box_bg") != "none" {
        cls.push_str(" vb-boxed");
    }
    r.rule("", &sel, &d);
    r.resp(m, "align", &sel, |v| v.as_str().filter(|a| !a.is_empty()).map(|a| format!("text-align:{a};")).unwrap_or_default());
    r.resp(m, "gap", &sel, |v| format!("--vb-wgap:{};", px(v)));
    r.resp(m, "min_h", &sel, |v| format!("min-height:{};", px(v)));
    r.resp(m, "order", &sel, |v| v.as_i64().map(|o| format!("order:{o};")).unwrap_or_default());
    box_style(r, m, &sel);
    let common = advanced(r, m, &id, &sel);
    let mut body: String = col["items"].as_array().into_iter().flatten().map(|w| widget(r, w)).collect();
    if body.is_empty() && r.ctx.editing {
        body = format!(r#"<div class="vbx-ph" data-vbx-col="{}"><span>+ Drag a widget here</span></div>"#, esc(&id));
    }
    let link = m["link"].as_str().map(|u| format!(r#"<a class="vb-col-link" href="{}" aria-hidden="true" tabindex="-1"></a>"#, href(u))).unwrap_or_default();
    let html = format!(r#"<div class="{cls}{c}" data-vb-id="{id}"{attrs}>{body}{link}</div>"#, c = common.classes, id = esc(&id), attrs = common.attrs);
    conditional(r, m, &id, html)
}

pub(super) fn widget(r: &mut R, w: &Value) -> String {
    let kind = s(w, "t").to_string();
    if kind == "elementor-widget" && !r.ctx.editing {
        return String::new();
    }
    let id = s(w, "id").to_string();
    // a global widget is drawn as the widget it points at, under this id
    if kind == "global" && r.depth < 3 {
        let target = w["s"]["ref"].as_str().and_then(|t| r.ctx.templates.get(t)).and_then(|l| l["sections"][0]["cols"][0]["items"][0].as_object().cloned());
        if let Some(mut g) = target {
            g.insert("id".into(), Value::String(id.clone()));
            r.depth += 1;
            let html = widget(r, &Value::Object(g));
            r.depth -= 1;
            return html.replacen("class=\"vb-w ", "class=\"vb-w vb-global ", 1);
        }
    }
    let m = &w["s"];
    let sel = r.sel(&id);
    let inner = super::widgets::html(r, &kind, &id, m, &sel, w);
    box_style(r, m, &sel);
    if spec::widget(&kind).is_some_and(|s| s.typo) {
        let target = super::widgets::typo_target(&kind).map(|t| format!("{sel} {t}")).unwrap_or_else(|| sel.clone());
        typo(r, m, &target);
    }
    let common = advanced(r, m, &id, &sel);
    let html = format!(
        r#"<div class="vb-w vb-w-{kind}{cls}" data-vb-id="{id}"{drag}{attrs}>{inner}</div>"#,
        kind = esc(&kind),
        cls = common.classes,
        id = esc(&id),
        drag = if r.ctx.editing { r#" draggable="true""# } else { "" },
        attrs = common.attrs,
    );
    conditional(r, m, &id, html)
}
