//! Every setting of every element, once.
//!
//! An [`F`] is one setting: its key, what it may hold ([`Field`]), its label
//! and panel group (`c` Content, `s` Style, `a` Advanced), whether it varies
//! per device (`resp`: the keys `key_t` and `key_m` hold the tablet and
//! mobile values), and when it is shown (`when`). The editor builds its
//! panels from [`spec_json`]; [`normalise`] rebuilds every layout from the
//! same lists, so a setting the editor offers is a setting the server keeps,
//! and nothing else is.

use super::{esc, hex_colour, safe_url};
use serde_json::{json, Map, Value};

/// What one setting may hold.
#[derive(Clone, Copy, Debug)]
pub enum Field {
    /// A panel subheading; holds nothing.
    Heading,
    /// One line of plain text, at most this many characters.
    Text(usize),
    /// Several lines of plain text.
    Long(usize),
    /// Inline rich text, sanitised on render.
    Rich(usize),
    /// Raw HTML or CSS, staff-written.
    Code(usize),
    Url,
    Image,
    Colour,
    /// A whole number, clamped, with the unit the editor shows.
    Num(i64, i64, &'static str),
    /// One of these `(value, label)` pairs.
    Choice(&'static [(&'static str, &'static str)]),
    Bool,
    /// A list of objects, each rebuilt from these fields, at most `max`.
    Items(&'static [F], usize),
    /// `[a-zA-Z0-9_- ]`, for anchors and classes.
    Ident(usize),
    /// Four sides, `top right bottom left`, in px, each clamped.
    Sides(i64, i64),
    /// `YYYY-MM-DDTHH:MM`.
    Date,
    /// A built-in icon name or an emoji.
    Icon,
    /// One of [`super::FONTS`].
    Font,
    /// The id of a saved template of this kind.
    Template(&'static str),
    /// A vuuut Forms id.
    FormRef,
    /// Selected records from a public catalogue.
    Catalogue(&'static str),
}

/// One setting.
#[derive(Clone, Copy, Debug)]
pub struct F {
    pub key: &'static str,
    pub field: Field,
    pub label: &'static str,
    pub group: char,
    pub resp: bool,
    pub when: Option<(&'static str, &'static [&'static str])>,
    pub hint: &'static str,
}

pub const fn f(key: &'static str, field: Field, label: &'static str, group: char) -> F {
    F { key, field, label, group, resp: false, when: None, hint: "" }
}
const fn head(label: &'static str, group: char) -> F {
    F { key: "", field: Field::Heading, label, group, resp: false, when: None, hint: "" }
}

impl F {
    pub const fn r(mut self) -> Self {
        self.resp = true;
        self
    }
    pub const fn w(mut self, key: &'static str, values: &'static [&'static str]) -> Self {
        self.when = Some((key, values));
        self
    }
    pub const fn h(mut self, hint: &'static str) -> Self {
        self.hint = hint;
        self
    }
}

/// One widget type.
pub struct W {
    pub key: &'static str,
    pub label: &'static str,
    /// `basic`, `pro`, `media`, `site` (dynamic, reads the site), `theme`.
    pub cat: &'static str,
    pub fields: &'static [F],
    /// Whether the Typography group applies.
    pub typo: bool,
    /// The settings a new one starts with, as JSON.
    pub defaults: &'static str,
}

// ---------------------------------------------------------------------------
// shared choices
// ---------------------------------------------------------------------------

use Field::*;

pub const ALIGN: &[(&str, &str)] = &[("left", "Left"), ("center", "Center"), ("right", "Right")];
pub const ALIGN_J: &[(&str, &str)] = &[("left", "Left"), ("center", "Center"), ("right", "Right"), ("justify", "Justify")];
pub const ANIMS: &[(&str, &str)] = &[
    ("none", "None"), ("fade", "Fade in"), ("rise", "Rise"), ("fall", "Fall"), ("slide-left", "Slide from right"),
    ("slide-right", "Slide from left"), ("zoom", "Zoom in"), ("zoom-out", "Zoom out"), ("flip", "Flip"), ("bounce", "Bounce"), ("blur", "Blur in"),
];
pub const HOVERS: &[(&str, &str)] = &[
    ("none", "None"), ("grow", "Grow"), ("shrink", "Shrink"), ("float", "Float"), ("sink", "Sink"), ("pulse", "Pulse"),
    ("rotate", "Rotate"), ("skew", "Skew"), ("glow", "Glow"), ("wobble", "Wobble"),
];
pub const SHADOWS: &[(&str, &str)] = &[("none", "None"), ("sm", "Soft"), ("md", "Medium"), ("lg", "Large"), ("xl", "Huge"), ("inner", "Inset"), ("glow", "Glow")];
pub const BORDERS: &[(&str, &str)] = &[("none", "None"), ("solid", "Solid"), ("dashed", "Dashed"), ("dotted", "Dotted"), ("double", "Double")];
pub const TAGS: &[(&str, &str)] = &[("h1", "H1"), ("h2", "H2"), ("h3", "H3"), ("h4", "H4"), ("h5", "H5"), ("h6", "H6"), ("p", "P"), ("div", "Div")];
pub const WHO: &[(&str, &str)] = &[("any", "Everyone"), ("visitors", "Signed-out visitors"), ("members", "Any member"), ("free", "Free members"), ("paid", "Paid members")];
pub const SIZES: &[(&str, &str)] = &[("s", "S"), ("m", "M"), ("l", "L"), ("xl", "XL")];
pub const RATIOS: &[(&str, &str)] = &[("16/9", "16:9"), ("4/3", "4:3"), ("1/1", "1:1"), ("3/4", "3:4"), ("9/16", "9:16"), ("21/9", "21:9")];
pub const CROPS: &[(&str, &str)] = &[("auto", "None"), ("1/1", "1:1"), ("4/3", "4:3"), ("3/4", "3:4"), ("16/9", "16:9")];
pub const NETS: &[(&str, &str)] = &[
    ("x", "X"), ("facebook", "Facebook"), ("instagram", "Instagram"), ("linkedin", "LinkedIn"), ("youtube", "YouTube"), ("tiktok", "TikTok"),
    ("github", "GitHub"), ("mastodon", "Mastodon"), ("threads", "Threads"), ("bluesky", "Bluesky"), ("pinterest", "Pinterest"),
    ("reddit", "Reddit"), ("whatsapp", "WhatsApp"), ("telegram", "Telegram"), ("discord", "Discord"), ("email", "Email"), ("website", "Website"), ("rss", "RSS"),
];
pub const SHARE_NETS: &[(&str, &str)] = &[
    ("x", "X"), ("facebook", "Facebook"), ("linkedin", "LinkedIn"), ("reddit", "Reddit"), ("whatsapp", "WhatsApp"), ("telegram", "Telegram"),
    ("pinterest", "Pinterest"), ("bluesky", "Bluesky"), ("threads", "Threads"), ("email", "Email"), ("copy", "Copy link"),
];
const BG: &[(&str, &str)] = &[("none", "None"), ("colour", "Colour"), ("gradient", "Gradient"), ("image", "Image")];
const SHAPES: &[(&str, &str)] = &[
    ("none", "None"), ("wave", "Wave"), ("waves", "Waves"), ("curve", "Curve"), ("tilt", "Tilt"), ("triangle", "Triangle"),
    ("arrow", "Arrow"), ("zigzag", "Zigzag"), ("mountains", "Mountains"), ("clouds", "Clouds"), ("book", "Book"),
];

// ---------------------------------------------------------------------------
// shared groups
// ---------------------------------------------------------------------------

/// Typography, for widgets that hold text.
pub const TYPO: &[F] = &[
    head("Typography", 's'),
    f("font", Font, "Font", 's'),
    f("fs", Num(0, 200, "px"), "Font size", 's').r().h("0 keeps the widget's own size."),
    f("fw", Choice(&[("", "Default"), ("300", "Light"), ("400", "Regular"), ("500", "Medium"), ("600", "Semibold"), ("700", "Bold"), ("800", "Extra bold"), ("900", "Black")]), "Weight", 's'),
    f("lh", Num(0, 30, "×0.1"), "Line height", 's').r().h("12 means 1.2. 0 is the default."),
    f("ls", Num(-50, 200, "×0.01em"), "Letter spacing", 's').r(),
    f("tt", Choice(&[("", "Default"), ("uppercase", "UPPER"), ("lowercase", "lower"), ("capitalize", "Title")]), "Case", 's'),
    f("fst", Choice(&[("", "Normal"), ("italic", "Italic")]), "Style", 's'),
    f("deco", Choice(&[("", "None"), ("underline", "Underline"), ("line-through", "Strike")]), "Decoration", 's'),
    f("text_shadow", Choice(&[("none", "None"), ("soft", "Soft"), ("hard", "Hard"), ("glow", "Glow")]), "Text shadow", 's'),
    f("colour_hover", Colour, "Text colour on hover", 's'),
];

/// Background, border and shadow: every widget and column.
pub const BOX: &[F] = &[
    head("Box", 's'),
    f("box_bg", Choice(BG), "Background", 's'),
    f("box_colour", Colour, "Colour", 's').w("box_bg", &["colour", "gradient", "image"]),
    f("box_colour2", Colour, "Second colour", 's').w("box_bg", &["gradient"]),
    f("box_angle", Num(0, 360, "°"), "Angle", 's').w("box_bg", &["gradient"]),
    f("box_image", Image, "Image", 's').w("box_bg", &["image"]),
    f("box_hover", Colour, "Background on hover", 's'),
    f("border_style", Choice(BORDERS), "Border", 's'),
    f("border_w", Sides(0, 40), "Border width", 's').w("border_style", &["solid", "dashed", "dotted", "double"]),
    f("border_colour", Colour, "Border colour", 's').w("border_style", &["solid", "dashed", "dotted", "double"]),
    f("border_hover", Colour, "Border colour on hover", 's').w("border_style", &["solid", "dashed", "dotted", "double"]),
    f("radius_box", Sides(0, 400), "Rounded corners", 's').h("Top-left, top-right, bottom-right, bottom-left."),
    f("shadow", Choice(SHADOWS), "Shadow", 's'),
    f("shadow_hover", Choice(SHADOWS), "Shadow on hover", 's'),
];

/// Advanced: spacing, size, position, motion, visibility, conditions, code.
pub const ADV: &[F] = &[
    head("Layout", 'a'),
    f("margin", Sides(-400, 400), "Margin", 'a').r(),
    f("padding", Sides(0, 400), "Padding", 'a').r(),
    f("width_mode", Choice(&[("", "Default"), ("full", "Full width"), ("inline", "Inline (auto)"), ("custom", "Custom")]), "Width", 'a'),
    f("width_px", Num(0, 2400, "px"), "Custom width", 'a').r().w("width_mode", &["custom"]),
    f("self_align", Choice(&[("", "Default"), ("start", "Start"), ("center", "Center"), ("end", "End")]), "Align itself", 'a').r(),
    f("z", Num(-10, 9999, ""), "Z-index", 'a'),
    f("opacity", Num(0, 100, "%"), "Opacity", 'a'),
    f("position", Choice(&[("", "Default"), ("absolute", "Absolute"), ("fixed", "Fixed")]), "Position", 'a'),
    f("pos_h", Choice(&[("left", "Left"), ("right", "Right")]), "Horizontal from", 'a').w("position", &["absolute", "fixed"]),
    f("pos_x", Num(-3000, 3000, "px"), "Horizontal offset", 'a').r().w("position", &["absolute", "fixed"]),
    f("pos_v", Choice(&[("top", "Top"), ("bottom", "Bottom")]), "Vertical from", 'a').w("position", &["absolute", "fixed"]),
    f("pos_y", Num(-3000, 3000, "px"), "Vertical offset", 'a').r().w("position", &["absolute", "fixed"]),
    f("sticky", Choice(&[("", "None"), ("top", "Top"), ("bottom", "Bottom")]), "Sticky", 'a'),
    f("sticky_offset", Num(0, 600, "px"), "Sticky offset", 'a').w("sticky", &["top", "bottom"]),
    f("rotate", Num(-360, 360, "°"), "Rotate", 'a'),
    head("Motion effects", 'a'),
    f("anim", Choice(ANIMS), "Entrance animation", 'a'),
    f("anim_dur", Num(100, 4000, "ms"), "Duration", 'a').w("anim", &["fade", "rise", "fall", "slide-left", "slide-right", "zoom", "zoom-out", "flip", "bounce", "blur"]),
    f("anim_delay", Num(0, 4000, "ms"), "Delay", 'a').w("anim", &["fade", "rise", "fall", "slide-left", "slide-right", "zoom", "zoom-out", "flip", "bounce", "blur"]),
    f("hover_anim", Choice(HOVERS), "Hover animation", 'a'),
    f("fx_y", Num(-10, 10, ""), "Scroll: vertical parallax", 'a').h("Speed. Negative moves against the scroll."),
    f("fx_x", Num(-10, 10, ""), "Scroll: horizontal slide", 'a'),
    f("fx_fade", Choice(&[("", "None"), ("in", "Fade in"), ("out", "Fade out"), ("in-out", "In and out")]), "Scroll: transparency", 'a'),
    f("fx_scale", Choice(&[("", "None"), ("up", "Scale up"), ("down", "Scale down")]), "Scroll: scale", 'a'),
    f("fx_rotate", Num(-180, 180, "°"), "Scroll: rotate", 'a'),
    f("fx_blur", Num(0, 20, "px"), "Scroll: blur", 'a'),
    f("fx_mouse", Choice(&[("", "None"), ("track", "Mouse track"), ("tilt", "3D tilt")]), "Mouse effect", 'a'),
    f("fx_mouse_s", Num(1, 10, ""), "Mouse effect strength", 'a').w("fx_mouse", &["track", "tilt"]),
    head("Visibility", 'a'),
    f("hide_d", Bool, "Hide on desktop", 'a'),
    f("hide_t", Bool, "Hide on tablet", 'a'),
    f("hide_m", Bool, "Hide on mobile", 'a'),
    f("if_who", Choice(WHO), "Show to", 'a').h("Display condition, checked on every visit."),
    f("if_from", Date, "Show from", 'a'),
    f("if_until", Date, "Show until", 'a'),
    f("if_days", Text(7), "Only on weekdays", 'a').h("Digits 1–7, Monday is 1. Empty means every day."),
    head("Attributes and code", 'a'),
    f("anchor", Ident(40), "CSS id / anchor", 'a').h("Link to it with #anchor."),
    f("class", Ident(120), "CSS classes", 'a'),
    f("attrs", Items(&[f("name", Ident(40), "Attribute", 'c'), f("value", Text(300), "Value", 'c')], 12), "Custom attributes", 'a').h("data-*, aria-*, title, rel and role. Event handlers are dropped."),
    f("css", Code(10_000), "Custom CSS", 'a').h("Use `selector` for this element, e.g. selector:hover { opacity: .8 }"),
];

pub const SECTION: &[F] = &[
    head("Layout", 'c'),
    f("layout", Choice(&[("boxed", "Boxed"), ("full", "Full width")]), "Content width", 'c'),
    f("width", Num(0, 2400, "px"), "Boxed width", 'c').r().w("layout", &["boxed", ""]).h("0 uses the site's width."),
    f("gap", Num(0, 160, "px"), "Column gap", 'c').r(),
    f("min_h", Num(0, 100, "vh"), "Minimum height", 'c').r(),
    f("valign", Choice(&[("top", "Top"), ("center", "Middle"), ("bottom", "Bottom"), ("stretch", "Stretch")]), "Align columns", 'c'),
    f("stack", Choice(&[("mobile", "Mobile"), ("tablet", "Tablet"), ("never", "Never")]), "Stack columns on", 'c'),
    f("row_reverse", Bool, "Reverse column order", 'c'),
    f("edge_padding", Sides(0, 600), "Padding", 's').r(),
    f("reverse", Bool, "Reverse order when stacked", 'c'),
    f("tag", Choice(&[("section", "section"), ("header", "header"), ("footer", "footer"), ("aside", "aside"), ("article", "article"), ("nav", "nav"), ("div", "div")]), "HTML tag", 'c'),
    f("overflow", Bool, "Clip overflowing content", 'c'),
    head("Background", 's'),
    f("bg", Choice(&[("none", "None"), ("colour", "Colour"), ("gradient", "Gradient"), ("image", "Image"), ("video", "Video"), ("slideshow", "Slideshow")]), "Background", 's'),
    f("bg_colour", Colour, "Colour", 's').w("bg", &["colour", "gradient", "image", "video", "slideshow"]),
    f("bg_colour2", Colour, "Second colour", 's').w("bg", &["gradient"]),
    f("bg_angle", Num(0, 360, "°"), "Angle", 's').w("bg", &["gradient"]),
    f("bg_gtype", Choice(&[("linear", "Linear"), ("radial", "Radial")]), "Gradient type", 's').w("bg", &["gradient"]),
    f("bg_image", Image, "Image", 's').w("bg", &["image"]).r(),
    f("bg_pos", Choice(&[("center", "Center"), ("top", "Top"), ("bottom", "Bottom"), ("left", "Left"), ("right", "Right")]), "Position", 's').w("bg", &["image", "slideshow"]),
    f("bg_size", Choice(&[("cover", "Cover"), ("contain", "Contain"), ("auto", "Original")]), "Size", 's').w("bg", &["image"]),
    f("bg_fixed", Bool, "Fixed while scrolling (parallax)", 's').w("bg", &["image"]),
    f("bg_video", Url, "Video", 's').w("bg", &["video"]).h("YouTube, Vimeo or an .mp4. Muted and looping."),
    f("bg_slides", Items(&[f("src", Image, "Image", 'c')], 12), "Slides", 's').w("bg", &["slideshow"]),
    f("slide_s", Num(2, 30, "s"), "Seconds per slide", 's').w("bg", &["slideshow"]),
    f("kenburns", Bool, "Ken Burns zoom", 's').w("bg", &["slideshow", "image"]),
    f("overlay", Colour, "Overlay colour", 's'),
    f("overlay2", Colour, "Overlay gradient to", 's'),
    f("overlay_opacity", Num(0, 100, "%"), "Overlay strength", 's'),
    f("colour", Colour, "Text colour", 's'),
    f("link_colour", Colour, "Link colour", 's'),
    head("Spacing", 's'),
    f("pt", Num(-1, 600, "px"), "Padding top", 's').r().h("Leave empty for the site default."),
    f("pb", Num(-1, 600, "px"), "Padding bottom", 's').r(),
    head("Shape dividers", 's'),
    f("shape_top", Choice(SHAPES), "Top", 's'),
    f("shape_top_colour", Colour, "Top colour", 's').w("shape_top", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_top_h", Num(10, 400, "px"), "Top height", 's').r().w("shape_top", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_top_flip", Bool, "Flip top", 's').w("shape_top", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_top_front", Bool, "Top in front of content", 's').w("shape_top", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_bottom", Choice(SHAPES), "Bottom", 's'),
    f("shape_bottom_colour", Colour, "Bottom colour", 's').w("shape_bottom", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_bottom_h", Num(10, 400, "px"), "Bottom height", 's').r().w("shape_bottom", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_bottom_flip", Bool, "Flip bottom", 's').w("shape_bottom", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    f("shape_bottom_front", Bool, "Bottom in front of content", 's').w("shape_bottom", &["wave", "waves", "curve", "tilt", "triangle", "arrow", "zigzag", "mountains", "clouds", "book"]),
    head("Border", 's'),
    f("border_style", Choice(BORDERS), "Border", 's'),
    f("border_w", Sides(0, 40), "Border width", 's').w("border_style", &["solid", "dashed", "dotted", "double"]),
    f("border_colour", Colour, "Border colour", 's').w("border_style", &["solid", "dashed", "dotted", "double"]),
    f("radius_box", Sides(0, 400), "Rounded corners", 's'),
    f("shadow", Choice(SHADOWS), "Shadow", 's'),
];

/// What sections take from [`ADV`]: everything but margin-collapsing width
/// controls, which a full-width band does not need.
pub fn section_adv() -> impl Iterator<Item = &'static F> {
    ADV.iter().filter(|f| !matches!(f.key, "width_mode" | "width_px" | "self_align" | "padding"))
}

pub const COLUMN: &[F] = &[
    f("w", Num(1, 12, ""), "Width share", 'c').r().h("Columns divide the row by these shares: 2 and 1 make two-thirds and one-third."),
    f("order", Num(-10, 10, ""), "Order", 'c').r().h("Reorder a column on tablet or mobile."),
    f("align", Choice(&[("", "Default"), ("left", "Left"), ("center", "Center"), ("right", "Right")]), "Text alignment", 'c').r(),
    f("valign", Choice(&[("", "Auto"), ("top", "Top"), ("center", "Middle"), ("bottom", "Bottom"), ("between", "Space between")]), "Vertical position", 'c'),
    f("gap", Num(0, 160, "px"), "Space between widgets", 'c').r(),
    f("min_h", Num(0, 2000, "px"), "Minimum height", 'c').r(),
    f("colour", Colour, "Text colour", 's'),
    f("link_colour", Colour, "Link colour", 's'),
    f("link", Url, "Link the whole column", 'c'),
];

/// Page-level options of a layout.
pub const PAGE: &[F] = &[
    f("bg_colour", Colour, "Background", 'c'),
    f("colour", Colour, "Text colour", 'c'),
    f("font", Font, "Body font", 'c'),
    f("hfont", Font, "Heading font", 'c'),
    f("css", Code(30_000), "Custom CSS", 'c'),
];

// ---------------------------------------------------------------------------
// widgets
// ---------------------------------------------------------------------------

const LINK_ITEM: &[F] = &[f("text", Text(300), "Text", 'c'), f("icon", Icon, "Icon", 'c'), f("link", Url, "Link", 'c')];
const ACC_ITEM: &[F] = &[f("q", Text(300), "Title", 'c'), f("a", Long(6000), "Content", 'c')];
const TAB_ITEM: &[F] = &[f("title", Text(120), "Title", 'c'), f("icon", Icon, "Icon", 'c'), f("text", Long(8000), "Content", 'c')];
const IMG_ITEM: &[F] = &[f("src", Image, "Image", 'c'), f("alt", Text(300), "Alt text", 'c'), f("caption", Text(300), "Caption", 'c'), f("link", Url, "Link", 'c')];
const SOCIAL_ITEM: &[F] = &[f("net", Choice(NETS), "Network", 'c'), f("href", Url, "Link", 'c')];
const SHARE_ITEM: &[F] = &[f("net", Choice(SHARE_NETS), "Network", 'c')];
const SLIDE_ITEM: &[F] = &[
    f("image", Image, "Background image", 'c'), f("colour", Colour, "Background colour", 'c'), f("heading", Text(200), "Heading", 'c'),
    f("text", Long(600), "Text", 'c'), f("button", Text(80), "Button", 'c'), f("href", Url, "Button link", 'c'),
    f("align", Choice(ALIGN), "Alignment", 'c'),
];
const QUOTE_ITEM: &[F] = &[f("quote", Long(1200), "Quote", 'c'), f("name", Text(120), "Name", 'c'), f("role", Text(160), "Role", 'c'), f("avatar", Image, "Photo", 'c'), f("stars", Num(0, 5, ""), "Stars", 'c')];
const BAR_ITEM: &[F] = &[f("label", Text(120), "Label", 'c'), f("value", Num(0, 100, "%"), "Value", 'c'), f("colour", Colour, "Colour", 'c')];
const PRICE_ITEM: &[F] = &[f("title", Text(120), "Item", 'c'), f("price", Text(40), "Price", 'c'), f("desc", Long(400), "Description", 'c'), f("image", Image, "Image", 'c'), f("link", Url, "Link", 'c')];
const SPOT_ITEM: &[F] = &[f("x", Num(0, 100, "%"), "Left", 'c'), f("y", Num(0, 100, "%"), "Top", 'c'), f("title", Text(120), "Title", 'c'), f("text", Long(600), "Text", 'c'), f("link", Url, "Link", 'c')];
const PLAY_ITEM: &[F] = &[f("title", Text(160), "Title", 'c'), f("url", Url, "Video", 'c'), f("duration", Text(12), "Duration", 'c')];
const TEXT_ITEM: &[F] = &[f("text", Text(500), "Text", 'c')];

pub const WIDGETS: &[W] = &[
    // ---- basic ----
    W { key: "heading", label: "Heading", cat: "basic", typo: true, defaults: r#"{"text":"Add your heading here","tag":"h2"}"#, fields: &[
        f("text", Long(600), "Text", 'c').h("Dynamic tags work here: [[post.title]], [[site.title]]…"),
        f("tag", Choice(TAGS), "HTML tag", 'c'),
        f("link", Url, "Link", 'c'),
        f("align", Choice(ALIGN_J), "Alignment", 's').r(),
        f("size", Num(0, 200, "px"), "Size", 's').r().h("0 is automatic."),
        f("weight", Choice(&[("", "Default"), ("400", "Regular"), ("500", "Medium"), ("600", "Semibold"), ("700", "Bold"), ("800", "Extra bold")]), "Weight", 's'),
        f("colour", Colour, "Colour", 's'),
        f("gradient", Colour, "Gradient text to", 's').h("Fills the text from its colour to this one."),
    ]},
    W { key: "text", label: "Text", cat: "basic", typo: true, defaults: r#"{"html":"<p>Write something worth reading. Double-click to edit this text right on the page.</p>"}"#, fields: &[
        f("html", Rich(60_000), "Text", 'c'),
        f("align", Choice(ALIGN_J), "Alignment", 's').r(),
        f("size", Num(0, 60, "px"), "Size", 's').r(),
        f("colour", Colour, "Colour", 's'),
        f("columns", Num(1, 4, ""), "Columns", 's').r(),
        f("dropcap", Bool, "Drop cap", 's'),
    ]},
    W { key: "button", label: "Button", cat: "basic", typo: true, defaults: r##"{"text":"Get started","href":"#","style":"filled","size":"m"}"##, fields: &[
        f("text", Text(120), "Label", 'c'),
        f("href", Url, "Link", 'c'),
        f("new_tab", Bool, "Open in a new tab", 'c'),
        f("nofollow", Bool, "Add nofollow", 'c'),
        f("icon", Icon, "Icon", 'c'),
        f("icon_pos", Choice(&[("before", "Before"), ("after", "After")]), "Icon position", 'c'),
        f("text2", Text(120), "Second button", 'c'),
        f("href2", Url, "Second link", 'c').w("text2", &["*"]),
        f("style", Choice(&[("filled", "Filled"), ("outline", "Outline"), ("soft", "Soft"), ("link", "Link"), ("gradient", "Gradient")]), "Style", 's'),
        f("size", Choice(SIZES), "Size", 's'),
        f("align", Choice(&[("left", "Left"), ("center", "Center"), ("right", "Right"), ("full", "Full")]), "Alignment", 's').r(),
        f("colour", Colour, "Button colour", 's'),
        f("text_colour", Colour, "Label colour", 's'),
        f("hover_bg", Colour, "Colour on hover", 's'),
        f("hover_text", Colour, "Label on hover", 's'),
        f("btn_radius", Num(0, 100, "px"), "Rounding", 's'),
    ]},
    W { key: "image", label: "Image", cat: "basic", typo: false, defaults: r#"{"alt":""}"#, fields: &[
        f("src", Image, "Image", 'c').h("Or [[post.feature_image]] in a single-post template."),
        f("alt", Text(300), "Alt text", 'c'),
        f("caption", Text(300), "Caption", 'c'),
        f("link", Url, "Link", 'c'),
        f("lightbox", Bool, "Open in a lightbox", 'c'),
        f("width", Num(5, 100, "%"), "Width", 's').r(),
        f("max_h", Num(0, 2000, "px"), "Maximum height", 's').r(),
        f("fit", Choice(&[("", "Default"), ("cover", "Cover"), ("contain", "Contain")]), "Fit", 's'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("radius", Num(0, 400, "px"), "Rounded corners", 's'),
        f("img_shadow", Bool, "Shadow", 's'),
        f("hover", Choice(&[("none", "None"), ("zoom", "Zoom"), ("grayscale", "Colour on hover"), ("lift", "Lift"), ("blur", "Unblur")]), "Hover effect", 's'),
    ]},
    W { key: "feature", label: "Icon box", cat: "basic", typo: true, defaults: r#"{"icon":"bolt","title":"A clear benefit","text":"One or two sentences that explain why it matters to your reader."}"#, fields: &[
        f("icon", Icon, "Icon", 'c'),
        f("title", Text(200), "Title", 'c'),
        f("text", Long(2000), "Text", 'c'),
        f("link", Url, "Link", 'c'),
        f("layout", Choice(&[("top", "Top"), ("left", "Left"), ("right", "Right")]), "Icon position", 's').r(),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("icon_size", Num(12, 160, "px"), "Icon size", 's').r(),
        f("icon_shape", Choice(&[("square", "Square"), ("circle", "Circle"), ("none", "None")]), "Icon shape", 's'),
        f("icon_colour", Colour, "Icon colour", 's'),
        f("icon_bg", Colour, "Icon background", 's'),
        f("title_colour", Colour, "Title colour", 's'),
    ]},
    W { key: "image-box", label: "Image box", cat: "basic", typo: true, defaults: r#"{"title":"A title for this image","text":"A short description beneath it."}"#, fields: &[
        f("src", Image, "Image", 'c'),
        f("title", Text(200), "Title", 'c'),
        f("text", Long(2000), "Text", 'c'),
        f("link", Url, "Link", 'c'),
        f("layout", Choice(&[("top", "Top"), ("left", "Left"), ("right", "Right")]), "Image position", 's').r(),
        f("img_w", Num(10, 100, "%"), "Image width", 's').r(),
        f("radius", Num(0, 200, "px"), "Image rounding", 's'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
    ]},
    W { key: "icon", label: "Icon", cat: "basic", typo: false, defaults: r#"{"icon":"star","size":48}"#, fields: &[
        f("icon", Icon, "Icon", 'c'),
        f("link", Url, "Link", 'c'),
        f("size", Num(8, 300, "px"), "Size", 's').r(),
        f("shape", Choice(&[("none", "None"), ("square", "Square"), ("circle", "Circle")]), "Shape", 's'),
        f("colour", Colour, "Colour", 's'),
        f("bg", Colour, "Background", 's'),
        f("hover_colour", Colour, "Colour on hover", 's'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
    ]},
    W { key: "list", label: "Icon list", cat: "basic", typo: true, defaults: r#"{"icon":"check","items":[{"text":"First point worth making"},{"text":"Second point worth making"},{"text":"Third point worth making"}]}"#, fields: &[
        f("items", Items(LINK_ITEM, 60), "Items", 'c'),
        f("icon", Choice(&[("check", "✓"), ("dot", "•"), ("arrow", "→"), ("star", "★"), ("number", "1."), ("none", "None")]), "Marker", 'c').h("An item's own icon wins."),
        f("layout", Choice(&[("stack", "Stacked"), ("inline", "Inline")]), "Layout", 's'),
        f("divider", Bool, "Divider between items", 's'),
        f("icon_colour", Colour, "Marker colour", 's'),
        f("size", Num(0, 60, "px"), "Text size", 's').r(),
        f("gap", Num(0, 60, "px"), "Space between", 's').r(),
    ]},
    W { key: "spacer", label: "Spacer", cat: "basic", typo: false, defaults: r#"{"height":40}"#, fields: &[
        f("height", Num(0, 800, "px"), "Height", 'c').r(),
    ]},
    W { key: "divider", label: "Divider", cat: "basic", typo: true, defaults: r#"{"style":"solid","width":100,"thickness":1}"#, fields: &[
        f("text", Text(120), "Label", 'c').h("Optional text in the middle."),
        f("style", Choice(&[("solid", "Solid"), ("dashed", "Dashed"), ("dotted", "Dotted"), ("double", "Double")]), "Style", 's'),
        f("width", Num(5, 100, "%"), "Width", 's').r(),
        f("thickness", Num(1, 20, "px"), "Thickness", 's'),
        f("colour", Colour, "Colour", 's'),
        f("align", Choice(ALIGN), "Alignment", 's'),
        f("gap", Num(0, 200, "px"), "Space around", 's').r(),
    ]},
    W { key: "anchor", label: "Menu anchor", cat: "basic", typo: false, defaults: r#"{"id":"section"}"#, fields: &[
        f("id", Ident(40), "Anchor id", 'c').h("Link to it with #id — menus and buttons scroll here."),
    ]},
    W { key: "inner", label: "Inner section", cat: "basic", typo: false, defaults: r#"{}"#, fields: &[
        f("gap", Num(0, 160, "px"), "Column gap", 'c').r(),
        f("valign", Choice(&[("top", "Top"), ("center", "Middle"), ("bottom", "Bottom"), ("stretch", "Stretch")]), "Align columns", 'c'),
        f("stack", Choice(&[("mobile", "Mobile"), ("tablet", "Tablet"), ("never", "Never")]), "Stack columns on", 'c'),
        f("row_reverse", Bool, "Reverse column order", 'c'),
    ]},
    W { key: "html", label: "HTML", cat: "basic", typo: false, defaults: r#"{"code":"<div style=\"padding:24px;border:1px dashed #ccc;text-align:center\">Your custom HTML</div>"}"#, fields: &[
        f("code", Code(150_000), "HTML", 'c').h("Shown as written. Scripts run on the site, not in this editor."),
    ]},
    W { key: "elementor-widget", label: "Elementor widget", cat: "imported", typo: false, defaults: r#"{}"#, fields: &[
        f("label", Text(70), "Original element type", 'c'),
    ]},
    // ---- media ----
    W { key: "video", label: "Video", cat: "media", typo: false, defaults: r#"{"url":"https://www.youtube.com/watch?v=aqz-KE-bpKQ","ratio":"16/9"}"#, fields: &[
        f("url", Url, "Video link", 'c').h("YouTube, Vimeo or an .mp4/.webm address."),
        f("poster", Image, "Cover image", 'c').h("Shown with a play button until clicked — nothing loads from YouTube before then."),
        f("autoplay", Bool, "Autoplay (muted)", 'c'),
        f("loop", Bool, "Loop", 'c'),
        f("controls", Choice(&[("on", "Show controls"), ("off", "Hide controls")]), "Controls", 'c'),
        f("start", Num(0, 36000, "s"), "Start at", 'c'),
        f("ratio", Choice(RATIOS), "Shape", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
    ]},
    W { key: "audio", label: "Audio", cat: "media", typo: false, defaults: r#"{"url":""}"#, fields: &[
        f("url", Url, "Audio link", 'c').h("SoundCloud, Spotify, or an .mp3 address."),
        f("title", Text(200), "Title", 'c'),
    ]},
    W { key: "gallery", label: "Gallery", cat: "media", typo: false, defaults: r#"{"images":[],"columns":3,"gap":12,"ratio":"1/1","lightbox":true}"#, fields: &[
        f("images", Items(IMG_ITEM, 60), "Images", 'c'),
        f("layout", Choice(&[("grid", "Grid"), ("masonry", "Masonry"), ("justified", "Justified")]), "Layout", 'c'),
        f("lightbox", Bool, "Open in a lightbox", 'c'),
        f("captions", Bool, "Show captions", 'c'),
        f("columns", Num(1, 8, ""), "Columns", 's').r(),
        f("gap", Num(0, 80, "px"), "Gap", 's').r(),
        f("ratio", Choice(CROPS), "Crop", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
        f("hover", Choice(&[("none", "None"), ("zoom", "Zoom"), ("grayscale", "Colour on hover"), ("dim", "Dim others")]), "Hover effect", 's'),
    ]},
    W { key: "carousel", label: "Image carousel", cat: "media", typo: false, defaults: r#"{"images":[],"per_view":3,"gap":16,"arrows":true,"dots":true,"ratio":"4/3"}"#, fields: &[
        f("images", Items(IMG_ITEM, 40), "Slides", 'c'),
        f("lightbox", Bool, "Open in a lightbox", 'c'),
        f("captions", Bool, "Show captions", 'c'),
        f("per_view", Num(1, 8, ""), "Slides at once", 's').r(),
        f("gap", Num(0, 80, "px"), "Gap", 's'),
        f("ratio", Choice(CROPS), "Crop", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
        f("autoplay", Bool, "Autoplay", 'c'),
        f("speed", Num(1, 20, "s"), "Seconds per slide", 'c').w("autoplay", &["true"]),
        f("loop", Bool, "Loop", 'c'),
        f("arrows", Bool, "Arrows", 'c'),
        f("dots", Bool, "Dots", 'c'),
    ]},
    W { key: "slides", label: "Slides", cat: "media", typo: true, defaults: r##"{"items":[{"heading":"Slide one","text":"Something worth saying.","button":"Learn more","href":"#","colour":"c1","align":"center"},{"heading":"Slide two","text":"And the next thing.","button":"Get started","href":"#","colour":"c2","align":"center"}],"height":480,"autoplay":true,"speed":6,"arrows":true,"dots":true}"##, fields: &[
        f("items", Items(SLIDE_ITEM, 20), "Slides", 'c'),
        f("height", Num(120, 1200, "px"), "Height", 's').r(),
        f("transition", Choice(&[("slide", "Slide"), ("fade", "Fade")]), "Transition", 'c'),
        f("autoplay", Bool, "Autoplay", 'c'),
        f("speed", Num(1, 30, "s"), "Seconds per slide", 'c'),
        f("arrows", Bool, "Arrows", 'c'),
        f("dots", Bool, "Dots", 'c'),
        f("kenburns", Bool, "Ken Burns zoom", 's'),
        f("overlay", Colour, "Overlay", 's'),
        f("overlay_opacity", Num(0, 100, "%"), "Overlay strength", 's'),
        f("text_colour", Colour, "Text colour", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
    ]},
    W { key: "playlist", label: "Video playlist", cat: "media", typo: false, defaults: r#"{"items":[{"title":"Introduction","url":"https://www.youtube.com/watch?v=aqz-KE-bpKQ","duration":"9:56"}]}"#, fields: &[
        f("items", Items(PLAY_ITEM, 40), "Videos", 'c'),
        f("title", Text(160), "Playlist title", 'c'),
        f("layout", Choice(&[("side", "List beside"), ("below", "List below")]), "Layout", 's'),
    ]},
    W { key: "lottie", label: "Lottie", cat: "media", typo: false, defaults: r#"{"url":"","loop":true,"trigger":"view"}"#, fields: &[
        f("url", Url, "Animation JSON", 'c').h("A Lottie .json address (LottieFiles: copy the JSON URL)."),
        f("trigger", Choice(&[("view", "When visible"), ("hover", "On hover"), ("always", "Always"), ("scroll", "With scrolling")]), "Play", 'c'),
        f("loop", Bool, "Loop", 'c'),
        f("speed", Num(1, 30, "×0.1"), "Speed", 'c'),
        f("width", Num(10, 100, "%"), "Width", 's').r(),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "map", label: "Map", cat: "media", typo: false, defaults: r#"{"address":"Times Square, New York","zoom":14,"height":360}"#, fields: &[
        f("address", Text(300), "Address or place", 'c'),
        f("zoom", Num(1, 20, ""), "Zoom", 'c'),
        f("height", Num(120, 1200, "px"), "Height", 's').r(),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
        f("grayscale", Bool, "Black and white", 's'),
    ]},
    W { key: "embed", label: "Embed", cat: "media", typo: false, defaults: r#"{"url":"","height":420}"#, fields: &[
        f("url", Url, "Address to embed", 'c').h("Any https page that allows embedding: a calendar, a booking form, a podcast player…"),
        f("title", Text(200), "Title (for screen readers)", 'c'),
        f("height", Num(80, 2400, "px"), "Height", 's').r(),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
    ]},
    W { key: "hotspot", label: "Hotspots", cat: "media", typo: true, defaults: r#"{"items":[{"x":30,"y":40,"title":"Feature one","text":"What it does."},{"x":68,"y":60,"title":"Feature two","text":"Why it matters."}],"pulse":true}"#, fields: &[
        f("src", Image, "Image", 'c'),
        f("items", Items(SPOT_ITEM, 30), "Hotspots", 'c'),
        f("pulse", Bool, "Pulse", 's'),
        f("colour", Colour, "Hotspot colour", 's'),
        f("trigger", Choice(&[("hover", "Hover"), ("click", "Click")]), "Open on", 'c'),
    ]},
    // ---- pro ----
    W { key: "posts", label: "Posts", cat: "site", typo: true, defaults: r#"{"source":"latest","count":6,"columns":3,"layout":"cards","show_image":true,"show_excerpt":true,"show_meta":true}"#, fields: &[
        f("source", Choice(&[("latest", "Latest"), ("featured", "Featured"), ("tag", "By tag"), ("author", "By author"), ("related", "Related to this post")]), "Show", 'c'),
        f("tag", Ident(80), "Tag slug", 'c').w("source", &["tag"]),
        f("author", Ident(80), "Author slug", 'c').w("source", &["author"]),
        f("kind", Choice(&[("post", "Posts"), ("page", "Pages")]), "Of type", 'c'),
        f("count", Num(1, 48, ""), "How many", 'c'),
        f("offset", Num(0, 50, ""), "Skip the first", 'c'),
        f("order", Choice(&[("newest", "Newest first"), ("oldest", "Oldest first"), ("title", "A–Z")]), "Order", 'c'),
        f("exclude_current", Bool, "Leave out the current post", 'c'),
        f("filter", Bool, "Tag filter bar (portfolio)", 'c'),
        f("pagination", Choice(&[("none", "None"), ("more", "Load more button")]), "More", 'c'),
        f("layout", Choice(&[("cards", "Cards"), ("classic", "Classic"), ("list", "List"), ("overlay", "Overlay"), ("minimal", "Titles only"), ("carousel", "Carousel")]), "Skin", 's'),
        f("columns", Num(1, 6, ""), "Columns", 's').r(),
        f("gap", Num(0, 80, "px"), "Gap", 's'),
        f("ratio", Choice(CROPS), "Image crop", 's'),
        f("show_image", Bool, "Image", 's'),
        f("show_excerpt", Bool, "Excerpt", 's'),
        f("show_meta", Bool, "Date and author", 's'),
        f("show_tags", Bool, "Primary tag", 's'),
        f("read_more", Text(60), "Read more label", 's'),
    ]},
    W { key: "testimonials", label: "Testimonial carousel", cat: "pro", typo: true, defaults: r#"{"items":[{"quote":"It made our week easier from day one.","name":"Sam Lee","role":"Editor","stars":5},{"quote":"Exactly what we needed, nothing we did not.","name":"Priya Shah","role":"Founder","stars":5},{"quote":"Our readers noticed the difference.","name":"Jon Park","role":"Designer","stars":5}],"per_view":2,"dots":true,"autoplay":true,"speed":7}"#, fields: &[
        f("items", Items(QUOTE_ITEM, 30), "Testimonials", 'c'),
        f("skin", Choice(&[("card", "Cards"), ("bubble", "Speech bubble"), ("plain", "Plain")]), "Skin", 's'),
        f("per_view", Num(1, 4, ""), "At once", 's').r(),
        f("autoplay", Bool, "Autoplay", 'c'),
        f("speed", Num(2, 30, "s"), "Seconds per slide", 'c'),
        f("arrows", Bool, "Arrows", 'c'),
        f("dots", Bool, "Dots", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
        f("card_bg", Colour, "Card background", 's'),
    ]},
    W { key: "testimonial", label: "Testimonial", cat: "basic", typo: true, defaults: r#"{"quote":"This changed the way we work. I cannot imagine going back.","name":"Alex Morgan","role":"Founder, Northwind","stars":5}"#, fields: &[
        f("quote", Long(2000), "Quote", 'c'),
        f("name", Text(120), "Name", 'c'),
        f("role", Text(160), "Role or company", 'c'),
        f("avatar", Image, "Photo", 'c'),
        f("stars", Num(0, 5, ""), "Stars", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "accordion", label: "Accordion / FAQ", cat: "basic", typo: true, defaults: r#"{"items":[{"q":"What is included?","a":"Everything you need to get started, and nothing you do not."},{"q":"Can I cancel at any time?","a":"Yes. There are no contracts and no fees for leaving."},{"q":"Do you offer support?","a":"Real people answer every message, usually within a day."}],"open_first":true}"#, fields: &[
        f("items", Items(ACC_ITEM, 60), "Items", 'c'),
        f("open_first", Bool, "Open the first one", 'c'),
        f("single", Bool, "Only one open at a time", 'c'),
        f("schema", Bool, "FAQ schema for search engines", 'c'),
        f("icon", Choice(&[("plus", "+ / −"), ("chevron", "Chevron"), ("none", "None")]), "Icon", 's'),
        f("skin", Choice(&[("lines", "Lines"), ("boxed", "Boxed"), ("cards", "Cards")]), "Skin", 's'),
        f("title_colour", Colour, "Title colour", 's'),
        f("active_colour", Colour, "Open title colour", 's'),
    ]},
    W { key: "tabs", label: "Tabs", cat: "basic", typo: true, defaults: r#"{"items":[{"title":"Overview","text":"What this is, in a paragraph."},{"title":"Details","text":"The specifics people ask about."},{"title":"Pricing","text":"What it costs and what is included."}]}"#, fields: &[
        f("items", Items(TAB_ITEM, 20), "Tabs", 'c'),
        f("layout", Choice(&[("top", "Horizontal"), ("left", "Vertical")]), "Layout", 's'),
        f("align", Choice(&[("left", "Start"), ("center", "Center"), ("justify", "Fill")]), "Tab alignment", 's'),
        f("skin", Choice(&[("underline", "Underline"), ("pills", "Pills"), ("boxed", "Boxed")]), "Skin", 's'),
        f("active_colour", Colour, "Active colour", 's'),
    ]},
    W { key: "stat", label: "Counter", cat: "basic", typo: true, defaults: r#"{"number":"120","suffix":"+","label":"Happy customers","animate":true}"#, fields: &[
        f("number", Text(24), "Number", 'c'),
        f("prefix", Text(12), "Before", 'c'),
        f("suffix", Text(12), "After", 'c'),
        f("label", Text(160), "Label", 'c'),
        f("animate", Bool, "Count up when visible", 'c'),
        f("duration", Num(200, 8000, "ms"), "Count duration", 'c').w("animate", &["true"]),
        f("separator", Bool, "Thousands separator", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("colour", Colour, "Number colour", 's'),
        f("size", Num(0, 200, "px"), "Number size", 's').r(),
    ]},
    W { key: "progress", label: "Progress bars", cat: "basic", typo: true, defaults: r#"{"items":[{"label":"Design","value":90},{"label":"Writing","value":75},{"label":"Code","value":60}],"show_value":true}"#, fields: &[
        f("items", Items(BAR_ITEM, 20), "Bars", 'c'),
        f("show_value", Bool, "Show the percentage", 'c'),
        f("striped", Bool, "Striped", 's'),
        f("height", Num(2, 60, "px"), "Height", 's'),
        f("colour", Colour, "Bar colour", 's'),
        f("track", Colour, "Track colour", 's'),
    ]},
    W { key: "rating", label: "Star rating", cat: "basic", typo: true, defaults: r#"{"value":45,"max":5}"#, fields: &[
        f("value", Num(0, 100, "×0.1"), "Rating", 'c').h("45 means 4.5."),
        f("max", Choice(&[("5", "Out of 5"), ("10", "Out of 10")]), "Scale", 'c'),
        f("label", Text(160), "Label", 'c'),
        f("icon", Choice(&[("star", "Star"), ("heart", "Heart")]), "Icon", 's'),
        f("colour", Colour, "Colour", 's'),
        f("size", Num(8, 120, "px"), "Size", 's').r(),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("schema", Bool, "Rating schema for search engines", 'c'),
    ]},
    W { key: "alert", label: "Alert", cat: "basic", typo: true, defaults: r#"{"kind":"info","title":"Heads up","text":"Something your reader should know."}"#, fields: &[
        f("kind", Choice(&[("info", "Info"), ("success", "Success"), ("warning", "Warning"), ("danger", "Danger")]), "Type", 'c'),
        f("title", Text(200), "Title", 'c'),
        f("text", Long(1000), "Text", 'c'),
        f("dismiss", Bool, "Can be dismissed", 'c'),
    ]},
    W { key: "pricing", label: "Price table", cat: "pro", typo: true, defaults: r##"{"title":"Pro","price":"$19","period":"/month","note":"For growing teams.","items":[{"text":"Unlimited projects"},{"text":"Priority support"},{"text":"Advanced analytics"}],"button":"Choose Pro","href":"#"}"##, fields: &[
        f("title", Text(120), "Plan", 'c'),
        f("price", Text(40), "Price", 'c'),
        f("old_price", Text(40), "Was", 'c'),
        f("period", Text(40), "Per", 'c'),
        f("note", Text(300), "Note", 'c'),
        f("items", Items(TEXT_ITEM, 40), "Features", 'c'),
        f("button", Text(120), "Button label", 'c'),
        f("href", Url, "Button link", 'c'),
        f("footer", Text(200), "Small print", 'c'),
        f("featured", Bool, "Highlight this plan", 's'),
        f("badge", Text(40), "Ribbon", 's'),
        f("accent", Colour, "Accent colour", 's'),
    ]},
    W { key: "price-list", label: "Price list", cat: "pro", typo: true, defaults: r#"{"items":[{"title":"Espresso","price":"$3","desc":"Double shot, single origin."},{"title":"Flat white","price":"$4.50","desc":"Silky, strong, small."}]}"#, fields: &[
        f("items", Items(PRICE_ITEM, 60), "Items", 'c'),
        f("leader", Choice(&[("dotted", "Dotted"), ("dashed", "Dashed"), ("solid", "Solid"), ("none", "None")]), "Leader line", 's'),
        f("price_colour", Colour, "Price colour", 's'),
    ]},
    W { key: "flipbox", label: "Flip box", cat: "pro", typo: true, defaults: r##"{"front_icon":"rocket","front_title":"Hover me","front_text":"There is more on the back.","back_title":"Here it is","back_text":"The detail, revealed.","back_button":"Learn more","back_href":"#","effect":"flip","front_bg":"c1","back_bg":"c3","height":280}"##, fields: &[
        f("front_icon", Icon, "Front icon", 'c'),
        f("front_title", Text(200), "Front title", 'c'),
        f("front_text", Long(800), "Front text", 'c'),
        f("front_image", Image, "Front image", 'c'),
        f("back_title", Text(200), "Back title", 'c'),
        f("back_text", Long(800), "Back text", 'c'),
        f("back_button", Text(80), "Back button", 'c'),
        f("back_href", Url, "Back link", 'c'),
        f("effect", Choice(&[("flip", "Flip"), ("slide", "Slide"), ("push", "Push"), ("zoom", "Zoom"), ("fade", "Fade")]), "Effect", 's'),
        f("direction", Choice(&[("left", "Left"), ("right", "Right"), ("up", "Up"), ("down", "Down")]), "Direction", 's'),
        f("height", Num(120, 900, "px"), "Height", 's').r(),
        f("front_bg", Colour, "Front background", 's'),
        f("back_bg", Colour, "Back background", 's'),
        f("text_colour", Colour, "Text colour", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
    ]},
    W { key: "cta", label: "Call to action", cat: "pro", typo: true, defaults: r##"{"title":"Ready when you are","text":"One clear sentence about what happens next.","button":"Get started","href":"#","layout":"cover","overlay":"c3","overlay_opacity":55,"height":360}"##, fields: &[
        f("image", Image, "Image", 'c'),
        f("title", Text(200), "Title", 'c'),
        f("text", Long(800), "Text", 'c'),
        f("button", Text(80), "Button", 'c'),
        f("href", Url, "Link", 'c'),
        f("ribbon", Text(40), "Ribbon", 'c'),
        f("layout", Choice(&[("cover", "Text over image"), ("left", "Image left"), ("right", "Image right"), ("above", "Image above")]), "Skin", 's'),
        f("height", Num(120, 1200, "px"), "Minimum height", 's').r(),
        f("overlay", Colour, "Overlay", 's'),
        f("overlay_opacity", Num(0, 100, "%"), "Overlay strength", 's'),
        f("bg", Colour, "Content background", 's'),
        f("text_colour", Colour, "Text colour", 's'),
        f("hover", Choice(&[("zoom", "Zoom image"), ("none", "None"), ("blur", "Blur image")]), "Hover", 's'),
        f("align", Choice(ALIGN), "Alignment", 's'),
        f("radius", Num(0, 100, "px"), "Rounded corners", 's'),
    ]},
    W { key: "animated-heading", label: "Animated headline", cat: "pro", typo: true, defaults: r#"{"before":"We make publishing","words":"simple\nbeautiful\nprofitable","after":"","style":"rotate","anim":"none","tag":"h2"}"#, fields: &[
        f("before", Text(200), "Before", 'c'),
        f("words", Long(600), "Rotating or highlighted words", 'c').h("One per line. Highlight uses only the first."),
        f("after", Text(200), "After", 'c'),
        f("style", Choice(&[("rotate", "Rotate"), ("typing", "Typing"), ("clip", "Clip"), ("flip", "Flip"), ("highlight", "Highlight")]), "Style", 'c'),
        f("shape", Choice(&[("underline", "Underline"), ("circle", "Circle"), ("curly", "Curly"), ("double", "Double"), ("strike", "Strike"), ("marker", "Marker"), ("x", "Cross")]), "Highlight shape", 'c').w("style", &["highlight"]),
        f("speed", Num(1, 20, "s"), "Seconds per word", 'c'),
        f("tag", Choice(TAGS), "HTML tag", 'c'),
        f("link", Url, "Link", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("size", Num(0, 200, "px"), "Size", 's').r(),
        f("colour", Colour, "Colour", 's'),
        f("word_colour", Colour, "Word colour", 's'),
        f("shape_colour", Colour, "Shape colour", 's'),
    ]},
    W { key: "countdown", label: "Countdown", cat: "pro", typo: true, defaults: r#"{"until":"","labels":true,"style":"boxes","expired":"It has started."}"#, fields: &[
        f("mode", Choice(&[("date", "To a date"), ("evergreen", "Evergreen (per visitor)")]), "Count down", 'c'),
        f("until", Date, "Until", 'c').w("mode", &["date", ""]).h("In the site's time zone as UTC."),
        f("hours", Num(1, 720, "h"), "Hours from first visit", 'c').w("mode", &["evergreen"]),
        f("labels", Bool, "Show labels", 'c'),
        f("show_days", Choice(&[("on", "Days"), ("off", "No days")]), "Days", 'c'),
        f("style", Choice(&[("boxes", "Boxes"), ("inline", "Inline"), ("minimal", "Minimal")]), "Style", 's'),
        f("expired", Text(300), "When it ends, show", 'c'),
        f("expired_href", Url, "…or go to", 'c'),
        f("box_bg", Colour, "Box colour", 's'),
        f("digit_colour", Colour, "Digit colour", 's'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "toc", label: "Table of contents", cat: "pro", typo: true, defaults: r#"{"title":"On this page","levels":"h2h3","numbered":false}"#, fields: &[
        f("title", Text(120), "Title", 'c'),
        f("levels", Choice(&[("h2", "H2"), ("h2h3", "H2–H3"), ("h2h4", "H2–H4"), ("h1h6", "All")]), "Headings", 'c'),
        f("numbered", Bool, "Numbered", 'c'),
        f("collapsible", Bool, "Collapsible", 'c'),
        f("collapsed", Bool, "Start collapsed", 'c').w("collapsible", &["true"]),
        f("box", Bool, "In a box", 's'),
        f("active_colour", Colour, "Current section colour", 's'),
    ]},
    W { key: "share", label: "Share buttons", cat: "pro", typo: true, defaults: r#"{"items":[{"net":"x"},{"net":"facebook"},{"net":"linkedin"},{"net":"whatsapp"},{"net":"copy"}],"view":"both","shape":"rounded"}"#, fields: &[
        f("items", Items(SHARE_ITEM, 12), "Networks", 'c'),
        f("view", Choice(&[("icon", "Icon"), ("label", "Label"), ("both", "Icon and label")]), "View", 's'),
        f("shape", Choice(&[("square", "Square"), ("rounded", "Rounded"), ("circle", "Circle")]), "Shape", 's'),
        f("colours", Choice(&[("brand", "Brand colours"), ("custom", "One colour"), ("outline", "Outline")]), "Colours", 's'),
        f("colour", Colour, "Colour", 's').w("colours", &["custom", "outline"]),
        f("align", Choice(&[("left", "Left"), ("center", "Center"), ("right", "Right"), ("justify", "Fill")]), "Alignment", 's').r(),
    ]},
    W { key: "social", label: "Social icons", cat: "basic", typo: false, defaults: r#"{"items":[{"net":"x","href":"https://x.com/"},{"net":"instagram","href":"https://instagram.com/"},{"net":"linkedin","href":"https://linkedin.com/"}],"size":24}"#, fields: &[
        f("items", Items(SOCIAL_ITEM, 20), "Profiles", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
        f("size", Num(12, 96, "px"), "Size", 's').r(),
        f("colours", Choice(&[("brand", "Brand colours"), ("custom", "One colour")]), "Colours", 's'),
        f("colour", Colour, "Colour", 's').w("colours", &["custom"]),
    ]},
    W { key: "blockquote", label: "Blockquote", cat: "pro", typo: true, defaults: r#"{"quote":"The best way to predict the future is to publish it.","author":"A wise editor","skin":"border"}"#, fields: &[
        f("quote", Long(2000), "Quote", 'c'),
        f("author", Text(160), "Author", 'c'),
        f("tweet", Bool, "Share on X button", 'c'),
        f("skin", Choice(&[("border", "Border"), ("quotation", "Quotation mark"), ("boxed", "Boxed"), ("clean", "Clean")]), "Skin", 's'),
        f("accent", Colour, "Accent colour", 's'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "code", label: "Code", cat: "pro", typo: false, defaults: r#"{"code":"fn main() {\n    println!(\"Hello\");\n}","language":"rust","copy":true,"theme":"dark"}"#, fields: &[
        f("code", Code(60_000), "Code", 'c'),
        f("language", Ident(30), "Language", 'c'),
        f("copy", Bool, "Copy button", 'c'),
        f("lines", Bool, "Line numbers", 'c'),
        f("theme", Choice(&[("dark", "Dark"), ("light", "Light")]), "Theme", 's'),
        f("max_h", Num(0, 2000, "px"), "Maximum height", 's'),
    ]},
    W { key: "text-path", label: "Text path", cat: "pro", typo: true, defaults: r#"{"text":"Words that follow a curve — ","path":"wave","size":24}"#, fields: &[
        f("text", Text(300), "Text", 'c'),
        f("path", Choice(&[("wave", "Wave"), ("arc", "Arc"), ("circle", "Circle"), ("line", "Line")]), "Path", 'c'),
        f("link", Url, "Link", 'c'),
        f("size", Num(8, 120, "px"), "Text size", 's'),
        f("width", Num(80, 1200, "px"), "Width", 's').r(),
        f("colour", Colour, "Colour", 's'),
        f("rotate_path", Num(0, 360, "°"), "Turn", 's'),
    ]},
    W { key: "progress-tracker", label: "Reading progress", cat: "pro", typo: false, defaults: r#"{"position":"top","height":4}"#, fields: &[
        f("kind", Choice(&[("bar", "Bar"), ("circle", "Circle")]), "Type", 'c'),
        f("position", Choice(&[("top", "Top of the window"), ("bottom", "Bottom of the window"), ("inline", "Where it is placed")]), "Position", 'c'),
        f("height", Num(1, 20, "px"), "Thickness", 's'),
        f("colour", Colour, "Colour", 's'),
        f("percent", Bool, "Show percentage", 'c').w("kind", &["circle"]),
    ]},
    W { key: "off-canvas", label: "Off-canvas", cat: "pro", typo: true, defaults: r#"{"button":"Menu","side":"right","width":360}"#, fields: &[
        f("button", Text(80), "Button", 'c'),
        f("icon", Icon, "Button icon", 'c'),
        f("template", Template("section"), "Content (a saved section)", 'c').h("Save a section as a template, then pick it here."),
        f("html", Rich(20_000), "…or this text", 'c'),
        f("side", Choice(&[("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")]), "Opens from", 's'),
        f("width", Num(160, 1200, "px"), "Size", 's'),
        f("panel_bg", Colour, "Panel background", 's'),
        f("overlay", Bool, "Dim the page", 's'),
    ]},
    W { key: "pay-button", label: "Payment button", cat: "pro", typo: true, defaults: r##"{"provider":"stripe","text":"Buy now","price":"$29","href":""}"##, fields: &[
        f("provider", Choice(&[("stripe", "Stripe"), ("paypal", "PayPal"), ("portal", "Ghost membership"), ("other", "Other")]), "Provider", 'c'),
        f("href", Url, "Payment link", 'c').h("A Stripe Payment Link or PayPal.me / checkout address. For memberships, Portal opens instead."),
        f("text", Text(80), "Label", 'c'),
        f("price", Text(40), "Price", 'c'),
        f("plan", Choice(&[("signup", "Sign-up"), ("upgrade", "Upgrade"), ("account", "Account")]), "Portal screen", 'c').w("provider", &["portal"]),
        f("align", Choice(&[("left", "Left"), ("center", "Center"), ("right", "Right"), ("full", "Full")]), "Alignment", 's'),
        f("colour", Colour, "Colour", 's'),
    ]},
    W { key: "form", label: "Form", cat: "pro", typo: false, defaults: r#"{"form":""}"#, fields: &[
        f("form", FormRef, "Form", 'c').h("Build forms under Tools → Forms; their actions, emails and responses live there."),
    ]},
    W { key: "signup", label: "Newsletter signup", cat: "site", typo: true, defaults: r#"{"button":"Subscribe","placeholder":"Your email address","layout":"inline"}"#, fields: &[
        f("layout", Choice(&[("inline", "Inline"), ("stacked", "Stacked")]), "Layout", 'c'),
        f("name", Bool, "Ask for a name", 'c'),
        f("placeholder", Text(80), "Email placeholder", 'c'),
        f("button", Text(60), "Button", 'c'),
        f("success", Text(200), "When it worked", 'c'),
        f("note", Text(200), "Small print", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
        f("colour", Colour, "Button colour", 's'),
    ]},
    W { key: "login", label: "Member login", cat: "site", typo: true, defaults: r#"{"signin":"Sign in","signup":"Subscribe","account":"Account"}"#, fields: &[
        f("signin", Text(40), "Sign-in label", 'c'),
        f("signup", Text(40), "Sign-up label", 'c'),
        f("account", Text(40), "Account label (signed in)", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
        f("colour", Colour, "Button colour", 's'),
    ]},
    W { key: "search", label: "Search", cat: "site", typo: true, defaults: r#"{"style":"field","placeholder":"Search…"}"#, fields: &[
        f("style", Choice(&[("field", "Search field"), ("button", "Button"), ("icon", "Icon")]), "Look", 'c'),
        f("placeholder", Text(80), "Placeholder", 'c'),
        f("label", Text(40), "Button label", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "nav-menu", label: "Nav menu", cat: "site", typo: true, defaults: r#"{"menu":"primary","layout":"horizontal","mobile":"tablet"}"#, fields: &[
        f("menu", Choice(&[("primary", "Primary navigation"), ("secondary", "Secondary navigation")]), "Menu", 'c').h("Edit menus under Tools → Navigations."),
        f("layout", Choice(&[("horizontal", "Horizontal"), ("vertical", "Vertical")]), "Layout", 'c'),
        f("mobile", Choice(&[("tablet", "Toggle on tablet"), ("mobile", "Toggle on mobile"), ("never", "Never")]), "Hamburger menu", 'c'),
        f("align", Choice(&[("left", "Left"), ("center", "Center"), ("right", "Right"), ("justify", "Fill")]), "Alignment", 's').r(),
        f("pointer", Choice(&[("underline", "Underline"), ("overline", "Overline"), ("background", "Background"), ("none", "None")]), "Hover pointer", 's'),
        f("colour", Colour, "Link colour", 's'),
        f("hover_colour", Colour, "Hover colour", 's'),
        f("active_colour", Colour, "Current page colour", 's'),
        f("gap", Num(0, 80, "px"), "Space between", 's').r(),
    ]},
    W { key: "site-logo", label: "Site logo", cat: "site", typo: true, defaults: r#"{"height":40}"#, fields: &[
        f("height", Num(12, 300, "px"), "Height", 's').r(),
        f("show", Choice(&[("auto", "Logo, else title"), ("logo", "Logo only"), ("title", "Title only"), ("icon", "Site icon")]), "Show", 'c'),
        f("link", Bool, "Link to the homepage", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's').r(),
    ]},
    W { key: "breadcrumbs", label: "Breadcrumbs", cat: "site", typo: true, defaults: r#"{"separator":"›","home":"Home"}"#, fields: &[
        f("home", Text(40), "Home label", 'c'),
        f("separator", Text(4), "Separator", 'c'),
        f("schema", Bool, "Breadcrumb schema", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "author-box", label: "Author box", cat: "site", typo: true, defaults: r#"{"avatar":true,"bio":true,"link":true}"#, fields: &[
        f("avatar", Bool, "Photo", 'c'),
        f("bio", Bool, "Biography", 'c'),
        f("link", Bool, "Link to their posts", 'c'),
        f("layout", Choice(&[("left", "Photo beside"), ("top", "Photo above")]), "Layout", 's'),
        f("boxed", Bool, "In a box", 's'),
    ]},
    W { key: "post-nav", label: "Post navigation", cat: "site", typo: true, defaults: r#"{"prev":"Previous","next":"Next","titles":true}"#, fields: &[
        f("prev", Text(40), "Previous label", 'c'),
        f("next", Text(40), "Next label", 'c'),
        f("titles", Bool, "Show post titles", 'c'),
        f("images", Bool, "Show images", 'c'),
    ]},
    W { key: "post-info", label: "Post info", cat: "site", typo: true, defaults: r#"{"author":true,"date":true,"reading":true,"tags":false}"#, fields: &[
        f("author", Bool, "Author", 'c'),
        f("date", Bool, "Date", 'c'),
        f("updated", Bool, "Last updated", 'c'),
        f("reading", Bool, "Reading time", 'c'),
        f("tags", Bool, "Tags", 'c'),
        f("separator", Text(4), "Separator", 'c'),
        f("align", Choice(ALIGN), "Alignment", 's'),
    ]},
    W { key: "post-content", label: "Post content", cat: "theme", typo: true, defaults: r#"{}"#, fields: &[
        f("note", Heading, "Draws the post's own content — for single-post templates.", 'c'),
    ]},
    W { key: "sitemap", label: "Sitemap", cat: "site", typo: true, defaults: r#"{"source":"pages","columns":2}"#, fields: &[
        f("source", Choice(&[("pages", "Pages"), ("posts", "Posts"), ("tags", "Tags"), ("authors", "Authors")]), "List", 'c'),
        f("title", Text(120), "Title", 'c'),
        f("limit", Num(1, 500, ""), "At most", 'c'),
        f("columns", Num(1, 4, ""), "Columns", 's').r(),
    ]},
    W { key: "products", label: "Shop products", cat: "site", typo: false, defaults: r#"{}"#, fields: &[
        f("ids", Catalogue("products"), "Products", 'c'),
        f("columns", Num(1, 6, ""), "Columns", 's').r().h("Defaults to three on desktop, two on tablet and one on mobile."),
        f("gap", Num(0, 80, "px"), "Gap", 's').r(),
    ]},
    W { key: "directory-listings", label: "Directory listings", cat: "site", typo: false, defaults: r#"{}"#, fields: &[
        f("ids", Catalogue("directory"), "Listings", 'c'),
        f("columns", Num(1, 6, ""), "Columns", 's').r().h("Defaults to three on desktop, two on tablet and one on mobile."),
        f("gap", Num(0, 80, "px"), "Gap", 's').r(),
    ]},
    W { key: "template", label: "Template", cat: "basic", typo: false, defaults: r#"{}"#, fields: &[
        f("ref", Template("section"), "Saved section or page", 'c').h("Changes to the template show here too."),
    ]},
    W { key: "global", label: "Global widget", cat: "basic", typo: false, defaults: r#"{}"#, fields: &[
        f("ref", Template("widget"), "Global widget", 'c').h("Edit it once under Tools → Page Builder → Templates; every copy updates."),
    ]},
];

pub fn widget(kind: &str) -> Option<&'static W> {
    WIDGETS.iter().find(|w| w.key == kind)
}

/// Every setting of a widget: its own, typography, box, advanced.
pub fn widget_fields(w: &'static W) -> impl Iterator<Item = &'static F> {
    w.fields.iter().chain(if w.typo { TYPO } else { &[] }.iter()).chain(BOX.iter()).chain(ADV.iter())
}

pub fn section_fields() -> impl Iterator<Item = &'static F> {
    SECTION.iter().chain(section_adv())
}

pub fn column_fields() -> impl Iterator<Item = &'static F> {
    COLUMN.iter().chain(BOX.iter()).chain(ADV.iter().filter(|f| !matches!(f.key, "width_mode" | "width_px" | "self_align" | "position" | "pos_h" | "pos_x" | "pos_v" | "pos_y")))
}

// ---------------------------------------------------------------------------
// normalising
// ---------------------------------------------------------------------------

pub const MAX_SECTIONS: usize = 150;
pub const MAX_COLUMNS: usize = 6;
pub const MAX_CATALOGUE_ITEMS: usize = 24;
pub const MAX_WIDGETS: usize = 100;
/// The largest layout JSON accepted, in bytes.
pub const MAX_BYTES: usize = 3_000_000;

/// A colour a layout may use: hex, one of the six global colours, or
/// `transparent`/`currentColor`.
pub fn colour(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(h) = hex_colour(s) {
        return Some(h);
    }
    if s == "transparent" || s == "currentColor" {
        return Some(s.into());
    }
    if s.len() == 2 && s.starts_with('c') && matches!(s.as_bytes()[1], b'1'..=b'6') {
        return Some(s.into());
    }
    None
}

fn clip(s: &str, max: usize) -> String {
    let s = s.replace('\u{0}', "");
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect()
    }
}

/// Attribute names a layout may set on an element.
pub fn attr_allowed(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.starts_with("data-") && n.len() > 5 && !n.starts_with("data-vb"))
        || n.starts_with("aria-")
        || matches!(n.as_str(), "title" | "rel" | "role" | "tabindex" | "lang" | "dir" | "itemprop" | "itemscope" | "itemtype")
}

fn clean_field(v: &Value, f: &F) -> Option<Value> {
    match f.field {
        Heading => None,
        Text(max) | Long(max) | Rich(max) | Code(max) => v.as_str().map(|s| json!(clip(s, max))).or_else(|| v.as_f64().map(|n| json!(clip(&n.to_string(), max)))),
        Url | Image => v.as_str().and_then(safe_url).map(Value::String),
        Colour => v.as_str().and_then(colour).map(Value::String),
        Num(lo, hi, _) => {
            let n = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))?;
            n.is_finite().then(|| json!((n.round() as i64).clamp(lo, hi)))
        }
        Choice(opts) => v.as_str().filter(|s| opts.iter().any(|(k, _)| k == s)).map(|s| json!(s)),
        Bool => v.as_bool().or_else(|| v.as_str().map(|s| s == "1" || s == "true")).map(Value::Bool),
        Ident(max) => v.as_str().map(|s| {
            let s: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ')).take(max).collect();
            json!(s.split_whitespace().collect::<Vec<_>>().join(" "))
        }),
        Items(fields, max) => {
            let arr = v.as_array()?;
            let items: Vec<Value> = arr
                .iter()
                .take(max)
                .filter_map(|i| i.as_object())
                .map(|o| {
                    let mut m = Map::new();
                    for f in fields {
                        if let Some(v) = o.get(f.key).and_then(|v| clean_field(v, f)) {
                            m.insert(f.key.to_string(), v);
                        }
                    }
                    Value::Object(m)
                })
                .filter(|o| f.key != "attrs" || o["name"].as_str().is_some_and(attr_allowed))
                .collect();
            Some(Value::Array(items))
        }
        Sides(lo, hi) => {
            let s = v.as_str()?;
            let parts: Vec<i64> = s.split_whitespace().take(4).filter_map(|p| p.trim_end_matches("px").parse::<f64>().ok()).map(|n| (n.round() as i64).clamp(lo, hi)).collect();
            let full = match parts.len() {
                1 => vec![parts[0]; 4],
                2 => vec![parts[0], parts[1], parts[0], parts[1]],
                3 => vec![parts[0], parts[1], parts[2], parts[1]],
                4 => parts,
                _ => return None,
            };
            Some(json!(full.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" ")))
        }
        Date => {
            let s = v.as_str()?.trim();
            let ok = s.len() >= 10 && s.len() <= 16 && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | 'T' | ':'));
            ok.then(|| json!(s))
        }
        Icon => v.as_str().map(|s| json!(clip(s.trim(), 16))),
        Font => v.as_str().filter(|s| super::FONTS.iter().any(|f| f.0 == *s)).map(|s| json!(s)),
        Catalogue(_) => {
            let mut seen = std::collections::HashSet::new();
            let ids: Vec<_> = v.as_array()?.iter().filter_map(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric()))
                .filter(|id| seen.insert(*id)).take(MAX_CATALOGUE_ITEMS).collect();
            Some(json!(ids))
        }
        Template(_) | FormRef => v.as_str().filter(|s| !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric())).map(|s| json!(s)),
    }
}

fn clean_map<'a>(input: &Map<String, Value>, fields: impl Iterator<Item = &'a F>) -> Map<String, Value> {
    let mut out = Map::new();
    for f in fields {
        if f.key.is_empty() {
            continue;
        }
        if let Some(v) = input.get(f.key).and_then(|v| clean_field(v, f)) {
            out.insert(f.key.to_string(), v);
        }
        if f.resp {
            for sfx in ["_t", "_m"] {
                let k = format!("{}{sfx}", f.key);
                if let Some(v) = input.get(&k).and_then(|v| clean_field(v, f)) {
                    out.insert(k, v);
                }
            }
        }
    }
    out
}

fn clean_id(v: Option<&Value>, n: &mut u32) -> String {
    match v.and_then(Value::as_str) {
        Some(s) if !s.is_empty() && s.len() <= 24 && s.chars().all(|c| c.is_ascii_alphanumeric()) => s.to_string(),
        _ => {
            *n += 1;
            format!("x{n}")
        }
    }
}

fn clean_columns(cols: &[Value], n: &mut u32, depth: u8) -> Result<Vec<Value>, String> {
    if cols.len() > MAX_COLUMNS {
        return Err(format!("A row can have at most {MAX_COLUMNS} columns."));
    }
    let empty = Map::new();
    let mut out = Vec::new();
    for c in cols {
        let Some(c) = c.as_object() else { continue };
        let cs = clean_map(c.get("s").and_then(Value::as_object).unwrap_or(&empty), column_fields());
        let items = c.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        if items.len() > MAX_WIDGETS {
            return Err(format!("A column can hold at most {MAX_WIDGETS} widgets."));
        }
        let mut out_items = Vec::new();
        for w in items {
            let Some(w) = w.as_object() else { continue };
            let Some(kind) = w.get("t").and_then(Value::as_str) else { continue };
            let upgraded = if kind == "elementor-widget" { super::elementor::upgrade_widget(w.get("elementor")) } else { None };
            let kind = upgraded.as_ref().map(|(kind, _)| *kind).unwrap_or(kind);
            let Some(spec) = widget(kind) else { continue };
            // Native inner sections are one level deep. Imported containers
            // can contain further containers up to the conversion limit.
            if kind == "inner" && (depth >= super::elementor::MAX_CONTAINER_DEPTH
                || (depth > 0 && w.get("elementor").and_then(|m| m["source"]["elType"].as_str()) != Some("container"))) {
                continue;
            }
            let raw = upgraded.as_ref().map(|(_, settings)| settings)
                .or_else(|| w.get("s").and_then(Value::as_object)).unwrap_or(&empty);
            let ws = clean_map(raw, widget_fields(spec));
            let mut obj = json!({"id": clean_id(w.get("id"), n), "t": kind, "s": ws});
            if kind == "inner" {
                let inner = w.get("cols").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                let mut cols = clean_columns(inner, n, depth + 1)?;
                if cols.is_empty() {
                    cols = vec![json!({"id": clean_id(None, n), "s": {}, "items": []}), json!({"id": clean_id(None, n), "s": {}, "items": []})];
                }
                obj["cols"] = Value::Array(cols);
            }
            preserve_elementor(w, &mut obj);
            if upgraded.is_some() {
                obj["elementor"]["snapshot"] = obj["s"].clone();
            }
            out_items.push(obj);
        }
        let mut column = json!({"id": clean_id(c.get("id"), n), "s": cs, "items": out_items});
        preserve_elementor(c, &mut column);
        out.push(column);
    }
    Ok(out)
}

/// Keep source metadata for re-export and for the validated styling adapters.
fn preserve_elementor(input: &Map<String, Value>, output: &mut Value) {
    if let Some(metadata) = input.get("elementor").filter(|v| v.is_object()) {
        output["elementor"] = metadata.clone();
    }
}

/// Rebuild a layout from untrusted JSON. Errors are messages for the editor.
pub fn normalise(input: &Value) -> Result<Value, String> {
    let obj = input.as_object().ok_or("The layout is not an object.")?;
    let sections = obj.get("sections").and_then(Value::as_array).ok_or("The layout has no sections list.")?;
    if sections.len() > MAX_SECTIONS {
        return Err(format!("A page can have at most {MAX_SECTIONS} sections."));
    }
    let empty = Map::new();
    let mut n = 0u32;
    let mut out = Vec::with_capacity(sections.len());
    for s in sections {
        let Some(s) = s.as_object() else { continue };
        let settings = clean_map(s.get("s").and_then(Value::as_object).unwrap_or(&empty), section_fields());
        let cols = s.get("cols").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut cols = clean_columns(cols, &mut n, 0)?;
        if cols.is_empty() {
            cols.push(json!({"id": clean_id(None, &mut n), "s": {}, "items": []}));
        }
        let mut section = json!({"id": clean_id(s.get("id"), &mut n), "s": settings, "cols": cols});
        preserve_elementor(s, &mut section);
        out.push(section);
    }
    let page = clean_map(obj.get("page").and_then(Value::as_object).unwrap_or(&empty), PAGE.iter());
    let mut layout = json!({"v": 2, "page": page, "sections": out});
    preserve_elementor(obj, &mut layout);
    Ok(layout)
}

// ---------------------------------------------------------------------------
// for the editor
// ---------------------------------------------------------------------------

fn field_json(f: &F) -> Value {
    let (kind, extra) = match f.field {
        Heading => ("heading", json!({})),
        Text(max) => ("text", json!({"max": max})),
        Long(max) => ("long", json!({"max": max})),
        Rich(_) => ("rich", json!({})),
        Code(_) => ("code", json!({})),
        Url => ("url", json!({})),
        Image => ("image", json!({})),
        Colour => ("colour", json!({})),
        Num(lo, hi, unit) => ("num", json!({"min": lo, "max": hi, "unit": unit})),
        Choice(opts) => ("choice", json!({"options": opts.iter().map(|(k, l)| json!([k, l])).collect::<Vec<_>>()})),
        Bool => ("bool", json!({})),
        Items(fields, max) => ("items", json!({"fields": fields.iter().map(field_json).collect::<Vec<_>>(), "max": max})),
        Ident(_) => ("ident", json!({})),
        Sides(lo, hi) => ("sides", json!({"min": lo, "max": hi})),
        Date => ("date", json!({})),
        Icon => ("icon", json!({})),
        Font => ("font", json!({})),
        Template(kind) => ("template", json!({"of": kind})),
        FormRef => ("form", json!({})),
        Catalogue(kind) => ("catalogue", json!({"of": kind, "max": MAX_CATALOGUE_ITEMS})),
    };
    let mut v = json!({"key": f.key, "kind": kind, "label": f.label, "group": f.group.to_string(), "resp": f.resp, "hint": f.hint});
    if let Some((k, vals)) = f.when {
        v["when"] = json!([k, vals]);
    }
    if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
        m.extend(e);
    }
    v
}

/// Everything the editor's panels are built from.
pub fn spec_json() -> Value {
    json!({
        "widgets": WIDGETS.iter().map(|w| json!({
            "key": w.key, "label": w.label, "cat": w.cat,
            "defaults": serde_json::from_str::<Value>(w.defaults).unwrap_or(json!({})),
            "fields": widget_fields(w).map(field_json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "section": section_fields().map(field_json).collect::<Vec<_>>(),
        "column": column_fields().map(field_json).collect::<Vec<_>>(),
        "page": PAGE.iter().map(field_json).collect::<Vec<_>>(),
        "fonts": super::FONTS.iter().map(|f| json!([f.0, f.1])).collect::<Vec<_>>(),
        "dynamicTags": super::dynamic::TEXT_TAGS.iter().map(|(k, l)| json!([k, l])).collect::<Vec<_>>(),
    })
}

/// A field's label escaped, for server-drawn hints.
pub fn label_html(f: &F) -> String {
    esc(f.label)
}
