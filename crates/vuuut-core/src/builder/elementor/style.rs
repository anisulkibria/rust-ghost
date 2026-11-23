//! Translate saved Elementor styling into scoped builder rules.
use super::{number, str_at};
use crate::builder::{render::R, safe_url};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const DEVICES: &[(&str, &str, &str)] = &[
    ("", "", ""),
    ("_t", "_tablet", "1024"),
    ("_m", "_mobile", "767"),
];
const LENGTHS: &[&str] = &["px", "%", "rem", "em", "vh", "vw", "svh", "dvh"];
static REM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(-?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+))rem\b").unwrap());
static FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-zA-Z_-][a-zA-Z0-9_-]*)\s*\(").unwrap());
static MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@media\s*\(\s*(min|max)-width\s*:\s*([0-9]{1,4})px\s*\)$").unwrap()
});
static COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
static COLOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:#[0-9a-f]{3,8}|(?:rgba?|hsla?)\([0-9.,% /+-]+\)|transparent|currentcolor)$",
    )
    .unwrap()
});

fn rem_to_px(value: &str) -> String {
    REM.replace_all(value, |c: &regex::Captures<'_>| {
        let n: f64 = c[1].parse().unwrap_or_default();
        format!("{}px", n * 16.0)
    })
    .into_owned()
}

fn length(value: &Value, key: &str, units: &[&str]) -> Option<String> {
    let n = number(&value[key]).filter(|n| n.abs() <= 10_000.0)?;
    let unit = value
        .get("unit")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .unwrap_or("px");
    units.contains(&unit).then(|| {
        if unit == "rem" {
            format!("{}px", n * 16.0)
        } else {
            format!("{n}{unit}")
        }
    })
}

fn grid_tracks(value: &Value, max_tracks: f64) -> Option<String> {
    if value["unit"] == "custom" {
        let tracks = value["size"].as_str()?.trim();
        if tracks.is_empty()
            || tracks.len() > 512
            || !tracks.chars().all(|c| c.is_ascii_alphanumeric() || " \t\n\r-.%+(),".contains(c))
            || FUNCTION.captures_iter(tracks).any(|m| {
                !matches!(&m[1], "repeat" | "minmax" | "fit-content" | "calc" | "min" | "max" | "clamp")
            })
        {
            return None;
        }
        return Some(rem_to_px(tracks));
    }
    number(&value["size"])
        .or_else(|| number(value))
        .filter(|n| *n > 0.0)
        .map(|n| format!("repeat({},minmax(0,1fr))", n.round().clamp(1.0, max_tracks) as u8))
}

fn sides(value: &Value) -> Option<String> {
    ["top", "right", "bottom", "left"]
        .iter()
        .map(|edge| length(value, edge, LENGTHS))
        .collect::<Option<Vec<_>>>()
        .map(|edges| edges.join(" "))
}

fn color(value: &Value) -> Option<&str> {
    let color = value.as_str()?.trim();
    (color.len() < 88 && COLOR.is_match(color)).then_some(color)
}

fn choice(value: &Value, allowed: &[&str]) -> Option<String> {
    let value = value.as_str()?;
    allowed.contains(&value).then(|| value.to_owned())
}

fn append(css: &mut String, property: &str, value: Option<String>) {
    if let Some(value) = value {
        css.push_str(&format!("{property}:{value};"));
    }
}

fn rule(r: &mut R, media: &str, selector: &str, css: &str) {
    if !css.is_empty() {
        if media.is_empty() {
            r.imported(&format!("{selector}{{{css}}}"));
        } else {
            r.imported(&format!(
                "@media(max-width:{media}px){{{selector}{{{css}}}}}"
            ));
        }
    }
}

fn raw(node: &Value) -> &Value {
    &node["elementor"]["source"]["settings"]
}

fn unchanged(node: &Value, key: &str) -> bool {
    let snapshot = &node["elementor"]["snapshot"];
    !snapshot.is_object() || node["s"][key] == snapshot[key]
}

fn layout_unchanged(node: &Value) -> bool {
    ["row_reverse", "reverse", "stack", "valign"]
        .iter()
        .all(|key| unchanged(node, key))
}

pub(in crate::builder) fn container_row(r: &mut R, node: &Value, selector: &str) -> bool {
    let cols = node["cols"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if node["elementor"]["source"]["elType"] != "container"
        || !layout_unchanged(node)
        || cols
            .iter()
            .any(|c| c["elementor"]["synthetic"] != true || c["s"] != c["elementor"]["snapshot"])
    {
        return false;
    }
    let source = raw(node);
    let grid = source["container_type"] == "grid";
    for &(local, remote, media) in DEVICES {
        let mut css = String::new();
        if remote.is_empty() {
            css.push_str(if grid {
                "display:grid;align-items:stretch;gap:20px;"
            } else {
                "display:flex;flex-direction:column;align-items:stretch;gap:20px;"
            });
        }
        let choice_at =
            |name: &str, allowed: &[&str]| choice(&source[format!("{name}{remote}")], allowed);
        if grid {
            let columns = &source[format!("grid_columns_grid{remote}")];
            append(
                &mut css,
                "grid-template-columns",
                grid_tracks(columns, 12.0)
                    .or_else(|| grid_tracks(&source[format!("grid_column{remote}")], 12.0))
                    .or_else(|| remote.is_empty().then(|| "repeat(3,minmax(0,1fr))".into())),
            );
            append(&mut css, "grid-template-rows", grid_tracks(&source[format!("grid_rows_grid{remote}")], 40.0));
            append(
                &mut css,
                "justify-items",
                choice_at("grid_justify_items", &["start", "end", "center", "stretch"]),
            );
        } else {
            let direction = choice_at(
                "flex_direction",
                &["row", "row-reverse", "column", "column-reverse"],
            );
            append(
                &mut css,
                "flex-direction",
                direction.or_else(|| {
                    (remote == "_mobile"
                        && matches!(str_at(source, "flex_direction"), "row" | "row-reverse"))
                    .then(|| "column".into())
                }),
            );
            append(
                &mut css,
                "flex-wrap",
                choice_at("flex_wrap", &["wrap", "nowrap", "wrap-reverse"])
                    .or_else(|| choice_at("wrap", &["wrap", "nowrap", "wrap-reverse"])),
            );
            append(
                &mut css,
                "align-content",
                choice_at(
                    "flex_align_content",
                    &[
                        "normal",
                        "center",
                        "start",
                        "end",
                        "space-between",
                        "space-around",
                        "stretch",
                    ],
                ),
            );
        }
        append(
            &mut css,
            "align-items",
            choice_at(
                if grid { "grid_align_items" } else { "flex_align_items" },
                &[
                    "flex-start",
                    "flex-end",
                    "start",
                    "end",
                    "center",
                    "stretch",
                    "baseline",
                ],
            ),
        );
        append(
            &mut css,
            "justify-content",
            choice_at(
                "flex_justify_content",
                &[
                    "flex-start",
                    "flex-end",
                    "start",
                    "end",
                    "center",
                    "space-around",
                    "space-evenly",
                    "space-between",
                ],
            ),
        );
        let key = format!("gap{local}");
        if unchanged(node, &key) {
            let gap = &source[format!("{}{remote}", if grid { "grid_gaps" } else { "flex_gap" })];
            for (name, axis) in [("row-gap", "row"), ("column-gap", "column")] {
                append(
                    &mut css,
                    name,
                    length(gap, axis, LENGTHS).or_else(|| length(gap, "size", LENGTHS)),
                );
            }
        } else if let Some(gap) = node["s"][key].as_i64() {
            append(&mut css, "gap", Some(format!("{gap}px")));
        }
        rule(r, media, selector, &css);
    }
    r.rule(
        "",
        &format!("{selector} > .vb-col:has(.vb-w)"),
        "display:contents;",
    );
    true
}

fn shadow(source: &Value) -> Option<String> {
    let c = color(&source["color"])?;
    let lengths: Vec<_> = ["horizontal", "vertical", "blur", "spread"]
        .into_iter()
        .map(|key| {
            format!(
                "{}px",
                number(&source[key])
                    .unwrap_or_default()
                    .clamp(-500.0, 500.0)
            )
        })
        .collect();
    Some(format!("{} {c}", lengths.join(" ")))
}

fn background(source: &Value, prefix: &str) -> Option<String> {
    let get = |name: &str| &source[format!("{prefix}{name}")];
    let first = color(get("color"));
    if get("background") == "gradient" {
        let second = color(get("color_b"))?;
        let first = first.unwrap_or("transparent");
        let a = length(get("color_stop"), "size", &["%"]).unwrap_or_else(|| "0%".into());
        let b = length(get("color_b_stop"), "size", &["%"]).unwrap_or_else(|| "100%".into());
        let gradient = if get("gradient_type") == "radial" {
            "radial-gradient(circle at center".into()
        } else {
            let angle = number(&get("gradient_angle")["size"])
                .unwrap_or(180.0)
                .clamp(-360.0, 360.0);
            format!("linear-gradient({angle}deg")
        };
        return Some(format!("{gradient},{first} {a},{second} {b})"));
    }
    let image = get("image")["url"].as_str().and_then(safe_url);
    if let Some(image) = image {
        let url = crate::url::base_asset(&image);
        let url = super::super::render::css_url(&url)
            .replace('<', "%3C")
            .replace('>', "%3E");
        return Some(format!(
            "{} url('{url}') center/cover no-repeat",
            first.unwrap_or("transparent")
        ));
    }
    first.map(str::to_owned)
}

pub(in crate::builder) fn overlay(r: &mut R, node: &Value, selector: &str) -> &'static str {
    let source = raw(node);
    if !matches!(
        str_at(source, "background_overlay_background"),
        "classic" | "gradient"
    ) || !unchanged(node, "overlay")
        || !unchanged(node, "overlay2")
        || !unchanged(node, "overlay_opacity")
    {
        return "";
    }
    let Some(paint) = background(source, "background_overlay_") else {
        return "";
    };
    let opacity = number(&source["background_overlay_opacity"]["size"])
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    r.rule("", selector, "position:relative;");
    r.rule(
        "",
        &format!("{selector} > .vb-el-overlay"),
        &format!("background:{paint};opacity:{opacity};"),
    );
    r.rule(
        "",
        &format!("{selector} > .vb-el-row"),
        "position:relative;",
    );
    r#"<span class="vb-el-overlay" aria-hidden="true"></span>"#
}

pub(in crate::builder) fn node(
    r: &mut R,
    node: &Value,
    selector: &str,
    section: bool,
    typo: Option<&str>,
) {
    let source = raw(node);
    if !source.is_object() {
        return;
    }
    r.has_elementor = true;
    let container = node["elementor"]["source"]["elType"] == "container";
    let widget = str_at(node, "t");
    let prefix = if section || container || widget.is_empty() {
        ""
    } else {
        "_"
    };
    let mut base = String::new();
    if container {
        base.push_str("min-width:0;");
        if section
            && unchanged(node, "pt")
            && unchanged(node, "pb")
            && unchanged(node, "edge_padding")
        {
            base.push_str("padding:0;");
        }
        if !section {
            base.push_str("display:flex;flex-direction:column;width:100%;max-width:100%;");
            r.rule(
                "",
                &format!("{selector} > .vb-row.vb-el-row"),
                "flex:1;min-width:0;",
            );
        } else {
            r.rule(
                "",
                &format!("{selector} > .vb-in"),
                "display:flex;flex:1;flex-direction:column;",
            );
            r.rule(
                "",
                &format!("{selector} > .vb-in > .vb-el-row"),
                "flex:1;min-width:0;",
            );
        }
    }
    let bg_key = if section { "bg" } else { "box_bg" };
    let colour_key = if section { "bg_colour" } else { "box_colour" };
    let image_key = if section { "bg_image" } else { "box_image" };
    if [
        bg_key,
        colour_key,
        image_key,
        if section { "bg_colour2" } else { "box_colour2" },
        if section { "bg_angle" } else { "box_angle" },
    ]
    .into_iter()
    .all(|key| unchanged(node, key))
    {
        append(
            &mut base,
            "background",
            background(source, &format!("{prefix}background_")),
        );
    }
    if unchanged(node, "shadow") {
        append(
            &mut base,
            "box-shadow",
            shadow(&source[format!("{prefix}box_shadow_box_shadow")]),
        );
        if let Some(value) = shadow(&source[format!("{prefix}box_shadow_hover_box_shadow")]) {
            rule(
                r,
                "",
                &format!("{selector}:hover"),
                &format!("box-shadow:{value};"),
            );
        }
    }
    append(
        &mut base,
        "overflow",
        choice(&source["overflow"], &["hidden", "clip", "auto", "visible"]),
    );
    if unchanged(node, "colour") && matches!(widget, "heading" | "text" | "list") {
        let key = if widget == "heading" {
            "title_color"
        } else {
            "text_color"
        };
        append(&mut base, "color", color(&source[key]).map(str::to_owned));
    }
    if widget == "icon" {
        let stacked = source["view"] == "stacked";
        let mut css = String::new();
        if unchanged(node, "colour") {
            append(
                &mut css,
                "color",
                color(
                    &source[if stacked {
                        "secondary_color"
                    } else {
                        "primary_color"
                    }],
                )
                .map(str::to_owned),
            );
        }
        if stacked && unchanged(node, "bg") {
            append(
                &mut css,
                "background",
                color(&source["primary_color"]).map(str::to_owned),
            );
        }
        rule(r, "", &format!("{selector} .vb-icon"), &css);
    }
    let target = typo.map(|t| format!("{selector} {t}"));
    if let Some(target) = &target {
        let family = if widget == "list" {
            &source["icon_typography_font_family"]
        } else {
            &source["typography_font_family"]
        };
        if unchanged(node, "font") {
            let css = match family.as_str() {
                Some("Courier New") => "font-family:'Courier New',monospace;",
                Some("Arial") => "font-family:Arial,sans-serif;",
                Some("Segoe UI") => "font-family:'Segoe UI',sans-serif;",
                Some("Georgia" | "Georgia, serif") => "font-family:Georgia,serif;",
                _ => "",
            };
            rule(r, "", target, css);
            if css.is_empty() {
                if let Some((key, ..)) = crate::builder::FONTS.iter().find(|f| {
                    family.as_str().is_some_and(|name| {
                        name.eq_ignore_ascii_case(f.1)
                            || (name == "Source Sans Pro" && f.0 == "source-sans-3")
                    })
                }) {
                    let font = r.font(key);
                    rule(r, "", target, &format!("font-family:{font};"));
                }
            }
        }
    }
    rule(r, "", selector, &base);
    for &(local, remote, media) in DEVICES {
        let mut css = String::new();
        if remote.is_empty() && source["_flex_size"] == "none" && unchanged(node, "width_mode") {
            css.push_str("flex:0 0 auto;");
        }
        for (name, native, external) in [
            ("margin", "margin", "margin"),
            ("border-radius", "radius_box", "border_radius"),
            ("border-width", "border_w", "border_width"),
            ("padding", "padding", "padding"),
        ] {
            let key = format!("{native}{local}");
            if unchanged(node, &key)
                && !(section
                    && native == "padding"
                    && (!unchanged(node, &format!("edge_padding{local}"))
                        || !unchanged(node, &format!("pt{local}"))
                        || !unchanged(node, &format!("pb{local}"))))
                && !(widget == "button" && native == "border-radius")
            {
                append(
                    &mut css,
                    name,
                    sides(&source[format!("{prefix}{external}{remote}")]),
                );
            }
        }
        if container {
            if unchanged(node, &format!("min_h{local}")) {
                append(
                    &mut css,
                    "min-height",
                    length(&source[format!("min_height{remote}")], "size", LENGTHS),
                );
            }
            if !section {
                let width = length(&source[format!("width{remote}")], "size", LENGTHS);
                let width = width.or_else(|| {
                    (remote == "_mobile"
                        && source["width"]["unit"] == "%"
                        && node["elementor"]["source"]["elType"] == "container")
                        .then(|| "100%".into())
                });
                append(&mut css, "width", width);
            }
            if unchanged(node, &format!("width{local}")) && unchanged(node, "layout") {
                let boxed = length(&source[format!("boxed_width{remote}")], "size", LENGTHS);
                let inner = if section {
                    format!("{selector} > .vb-in")
                } else {
                    format!("{selector} > .vb-row")
                };
                if let Some(width) = boxed {
                    rule(
                        r,
                        media,
                        &inner,
                        &format!("width:100%;max-width:{width};margin-inline:auto;"),
                    );
                }
            }
        }
        if unchanged(node, &format!("self_align{local}")) {
            append(
                &mut css,
                "align-self",
                choice(
                    &source[format!("_flex_align_self{remote}")],
                    &[
                        "auto", "start", "end", "flex-start", "flex-end", "center", "stretch",
                        "baseline",
                    ],
                ),
            );
        }
        if !container
            && unchanged(node, "width_mode")
            && unchanged(node, &format!("width_px{local}"))
        {
            let mode = source[format!("_element_width{remote}")]
                .as_str()
                .filter(|mode| !mode.is_empty())
                .or_else(|| source["_element_width"].as_str());
            if matches!(mode, Some("initial" | "custom")) {
                let width = &source[format!("_element_custom_width{remote}")];
                if let Some(width) = length(width, "size", LENGTHS) {
                    css.push_str(&format!("width:{width};max-width:100%;"));
                }
            }
        }
        if let Some(target) = &target {
            let typ = if widget == "list" {
                "icon_typography"
            } else {
                "typography"
            };
            let mut text = String::new();
            for (property, suffix, key) in [
                ("font-size", "font_size", "fs"),
                ("line-height", "line_height", "lh"),
                ("letter-spacing", "letter_spacing", "ls"),
            ] {
                if unchanged(node, &format!("{key}{local}")) {
                    append(
                        &mut text,
                        property,
                        length(&source[format!("{typ}_{suffix}{remote}")], "size", LENGTHS),
                    );
                }
            }
            rule(r, media, target, &text);
        }
        if widget == "button" {
            let mut button = String::new();
            if unchanged(node, "btn_radius") && unchanged(node, &format!("radius_box{local}")) {
                append(
                    &mut button,
                    "border-radius",
                    sides(&source[format!("border_radius{remote}")]),
                );
            }
            append(
                &mut button,
                "padding",
                sides(&source[format!("text_padding{remote}")]),
            );
            append(
                &mut button,
                "border-width",
                sides(&source[format!("border_width{remote}")]),
            );
            if remote.is_empty() {
                append(
                    &mut button,
                    "border-style",
                    choice(
                        &source["border_border"],
                        &["solid", "dashed", "dotted", "double", "none"],
                    ),
                );
                append(
                    &mut button,
                    "border-color",
                    color(&source["border_color"]).map(str::to_owned),
                );
            }
            rule(r, media, &format!("{selector} .vb-btn"), &button);
        }
        if widget == "image" {
            let mut image = String::new();
            if unchanged(node, "radius") {
                append(
                    &mut image,
                    "border-radius",
                    sides(&source[format!("image_border_radius{remote}")]),
                );
            }
            append(
                &mut image,
                "border-width",
                sides(&source[format!("image_border_width{remote}")]),
            );
            let position = source[format!("object-position{remote}")]
                .as_str()
                .filter(|v| {
                    v.split_whitespace()
                        .all(|p| matches!(p, "left" | "right" | "top" | "bottom" | "center"))
                });
            append(&mut image, "object-position", position.map(str::to_owned));
            if remote.is_empty() {
                append(
                    &mut image,
                    "border-style",
                    choice(
                        &source["image_border_border"],
                        &["solid", "dashed", "dotted", "double", "none"],
                    ),
                );
                append(
                    &mut image,
                    "border-color",
                    color(&source["image_border_color"]).map(str::to_owned),
                );
            }
            rule(r, media, &format!("{selector} img"), &image);
        }
        rule(r, media, selector, &css);
    }
    if unchanged(node, "css") {
        let row = format!(
            "{selector} > {}",
            if section {
                ".vb-in > .vb-el-row"
            } else {
                ".vb-el-row"
            }
        );
        r.imported(&custom_css(
            str_at(source, "custom_css"),
            selector,
            container.then_some(row.as_str()),
        ));
    }
}

fn selectors(input: &str, scope: &str, stylesheet: bool) -> Option<String> {
    input
        .split(',')
        .map(|s| {
            let s = s.trim();
            let tail = if stylesheet { s } else { s.strip_prefix("selector")? };
            if stylesheet && tail.is_empty()
                || !stylesheet && !tail.is_empty() && !tail.starts_with([' ', '.', ':', '>'])
            {
                return None;
            }
            if !tail
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " .:_->".contains(c) || (stylesheet && c == '#'))
            {
                return None;
            }
            let tail = tail
                .replace(".elementor-heading-title", ".vb-h")
                .replace(".elementor-field-group", ".vb-el-form label")
                .replace(".elementor-field", ".vb-el-input")
                .replace(".elementor-button", ":is(.vb-btn,.vb-el-submit)")
                .replace(".elementor-background-overlay", ".vb-el-overlay")
                .replace(".elementor-icon", ".vb-icon")
                .replace(".fa-arrow-right", ".vb-icon");
            let separator = if stylesheet { " " } else { "" };
            Some(format!("{scope}{separator}{tail}"))
        })
        .collect::<Option<Vec<_>>>()
        .map(|v| v.join(","))
}

fn declarations(input: &str) -> (String, String) {
    let mut normal = String::new();
    let mut layout = String::new();
    for entry in input.split(';') {
        let Some((property, value)) = entry.split_once(':') else {
            continue;
        };
        let property = property.trim();
        let value = value.trim();
        let property_allowed = matches!(
            property,
            "background"
                | "background-color"
                | "background-image"
                | "box-shadow"
                | "backdrop-filter"
                | "filter"
                | "aspect-ratio"
                | "background-size"
                | "background-position"
                | "object-fit"
                | "object-position"
                | "inset"
                | "font-size"
                | "font-weight"
                | "font-style"
                | "text-transform"
                | "text-decoration-line"
                | "letter-spacing"
                | "line-height"
                | "border-top-color"
                | "border-right-color"
                | "border-bottom-color"
                | "border-left-color"
                | "border-top-left-radius"
                | "border-top-right-radius"
                | "border-bottom-left-radius"
                | "border-bottom-right-radius"
                | "display"
                | "position"
                | "top"
                | "right"
                | "bottom"
                | "left"
                | "width"
                | "max-width"
                | "min-width"
                | "height"
                | "min-height"
                | "max-height"
                | "transform"
                | "transition"
                | "pointer-events"
                | "z-index"
                | "content"
                | "color"
                | "opacity"
                | "cursor"
                | "text-align"
                | "align-items"
                | "justify-content"
                | "flex-direction"
                | "flex-wrap"
                | "grid-template-columns"
                | "grid-template-rows"
                | "gap"
                | "row-gap"
                | "column-gap"
                | "border"
                | "border-color"
                | "border-style"
                | "border-width"
                | "border-radius"
                | "border-top"
                | "border-right"
                | "border-bottom"
                | "border-left"
                | "border-top-width"
                | "border-right-width"
                | "border-bottom-width"
                | "border-left-width"
                | "border-top-style"
                | "border-right-style"
                | "border-bottom-style"
                | "border-left-style"
                | "padding"
                | "padding-top"
                | "padding-right"
                | "padding-bottom"
                | "padding-left"
                | "margin"
                | "margin-top"
                | "margin-right"
                | "margin-bottom"
                | "margin-left"
                | "overflow"
        );
        if !property_allowed
            || value.is_empty()
            || value.len() > 1000
            || (property == "position" && !matches!(value, "relative" | "absolute" | "static"))
            || (property == "content" && !matches!(value, "''" | "\"\""))
            || (property != "content"
                && !value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || " -_#.%+(),/!\t\n\r".contains(c)))
            || FUNCTION.captures_iter(value).any(|m| {
                !matches!(
                    &m[1],
                    "rgb"
                        | "rgba"
                        | "hsl"
                        | "hsla"
                        | "linear-gradient"
                        | "radial-gradient"
                        | "blur"
                        | "rotate"
                        | "scale"
                        | "translate"
                        | "translateX"
                        | "translateY"
                        | "cubic-bezier"
                        | "calc"
                        | "min"
                        | "max"
                        | "clamp"
                        | "repeat"
                        | "minmax"
                )
            })
        {
            continue;
        }
        let output = if matches!(
            property,
            "flex-direction"
                | "flex-wrap"
                | "grid-template-columns"
                | "grid-template-rows"
                | "align-items"
                | "justify-content"
                | "gap"
                | "row-gap"
                | "column-gap"
        ) {
            &mut layout
        } else {
            &mut normal
        };
        output.push_str(&format!("{property}:{};", rem_to_px(value)));
    }
    (normal, layout)
}

fn scoped_rules(
    mut input: &str,
    scope: &str,
    row: Option<&str>,
    media_allowed: bool,
    stylesheet: bool,
) -> String {
    let mut out = String::new();
    while let Some((prelude, rest)) = input.trim().split_once('{') {
        let mut depth = 1;
        let end = rest.char_indices().find_map(|(i, c)| {
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
            }
            (depth == 0).then_some(i)
        });
        let Some(end) = end else { break };
        let body = &rest[..end];
        input = &rest[end + 1..];
        let prelude = prelude.trim();
        if media_allowed && MEDIA.is_match(prelude) {
            let css = scoped_rules(body, scope, row, false, stylesheet);
            if !css.is_empty() {
                out.push_str(&format!("{prelude}{{{css}}}"));
            }
        } else if let Some(selector) = selectors(prelude, scope, stylesheet) {
            let (normal, layout) = declarations(body);
            if !normal.is_empty() {
                out.push_str(&format!("{selector}{{{normal}}}"));
            }
            if !layout.is_empty() {
                let selector = row.filter(|_| prelude == "selector").unwrap_or(&selector);
                out.push_str(&format!("{selector}{{{layout}}}"));
            }
        }
    }
    out
}

fn custom_css(input: &str, scope: &str, row: Option<&str>) -> String {
    if input.len() > 16_000 {
        return String::new();
    }
    scoped_rules(&COMMENTS.replace_all(input, ""), scope, row, true, false)
}

/// Elementor exports page-level CSS as HTML widgets containing only style tags.
pub(super) fn style_blocks(mut html: &str) -> Option<Vec<&str>> {
    if html.len() > 16_000 {
        return None;
    }
    let mut blocks = Vec::new();
    while !html.trim().is_empty() {
        let (css, rest) = html.trim_start().strip_prefix("<style>")?.split_once("</style>")?;
        blocks.push(css);
        html = rest;
    }
    (!blocks.is_empty()).then_some(blocks)
}

pub(in crate::builder) fn stylesheet_widget(r: &mut R, node: &Value) -> bool {
    if node["elementor"]["source"]["widgetType"] != "html" {
        return false;
    }
    let Some(blocks) = style_blocks(str_at(raw(node), "html")) else {
        return false;
    };
    r.has_elementor = true;
    let scope = format!(".vb-L{}", r.ctx.scope);
    for css in blocks {
        r.imported(&scoped_rules(&COMMENTS.replace_all(css, ""), &scope, None, true, true));
    }
    true
}

/// A WordPress form has no local submission destination until staff select a Vuuut form.
pub(in crate::builder) fn form(r: &mut R, node: &Value, selector: &str) -> String {
    use crate::builder::esc;

    let source = raw(node);
    let mut grid = String::new();
    append(
        &mut grid,
        "row-gap",
        length(&source["row_gap"], "size", LENGTHS),
    );
    append(
        &mut grid,
        "column-gap",
        length(&source["column_gap"], "size", LENGTHS),
    );
    rule(r, "", &format!("{selector} .vb-el-form"), &grid);
    for (class, prefix) in [
        ("vb-el-input", "field"),
        ("vb-el-submit", "button"),
        ("vb-el-label", "label"),
    ] {
        let mut css = String::new();
        let get = |key: &str| &source[format!("{prefix}_{key}")];
        append(
            &mut css,
            "background-color",
            color(get("background_color")).map(str::to_owned),
        );
        let color_key = if prefix == "label" {
            "label_color"
        } else {
            "text_color"
        };
        append(&mut css, "color", color(get(color_key)).map(str::to_owned));
        append(
            &mut css,
            "border-color",
            color(get("border_color")).map(str::to_owned),
        );
        append(&mut css, "border-radius", sides(get("border_radius")));
        append(&mut css, "border-width", sides(get("border_width")));
        append(
            &mut css,
            "border-style",
            choice(
                get("border_border"),
                &["solid", "dashed", "dotted", "double", "none"],
            ),
        );
        append(&mut css, "padding", sides(get("text_padding")));
        append(
            &mut css,
            "font-size",
            length(get("typography_font_size"), "size", LENGTHS),
        );
        append(
            &mut css,
            "letter-spacing",
            length(get("typography_letter_spacing"), "size", LENGTHS),
        );
        append(
            &mut css,
            "font-weight",
            number(get("typography_font_weight")).map(|n| n.clamp(100.0, 900.0).to_string()),
        );
        if let Some((key, ..)) = crate::builder::FONTS.iter().find(|f| {
            get("typography_font_family")
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(f.1))
        }) {
            let stack = r.font(key);
            append(&mut css, "font-family", Some(stack.to_string()));
        }
        rule(r, "", &format!("{selector} .{class}"), &css);
    }
    let mut fields = String::new();
    for field in source["form_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .take(40)
    {
        let kind = str_at(field, "field_type");
        if matches!(
            kind,
            "hidden" | "honeypot" | "recaptcha" | "recaptcha_v3" | "html" | "step"
        ) {
            continue;
        }
        let width = number(&field["width"]).unwrap_or(100.0).clamp(5.0, 100.0);
        let span = (width * 0.12).round().max(1.0);
        let title = str_at(field, "field_label");
        let placeholder = esc(str_at(field, "placeholder"));
        let label = if !title.is_empty() && source["show_labels"] != "" {
            format!(r#"<span class="vb-el-label">{}</span>"#, esc(title))
        } else {
            String::new()
        };
        let control = if kind == "textarea" {
            format!(
                r#"<textarea class="vb-el-input" placeholder="{placeholder}" aria-label="{}" disabled></textarea>"#,
                esc(title)
            )
        } else {
            let kind = if matches!(
                kind,
                "text" | "email" | "tel" | "number" | "date" | "checkbox"
            ) {
                kind
            } else {
                "text"
            };
            format!(
                r#"<input class="vb-el-input" type="{kind}" placeholder="{placeholder}" aria-label="{}" disabled>"#,
                esc(title)
            )
        };
        fields.push_str(&format!(
            r#"<label style="--vb-field-span:{span}">{label}{control}</label>"#
        ));
    }
    let button = esc(source["button_text"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Submit"));
    let notice = if r.ctx.editing {
        r#"<div class="vb-el-form-note">Choose a Vuuut form to accept submissions.</div>"#
    } else {
        ""
    };
    format!(
        r#"<div class="vb-el-form">{fields}<button class="vb-el-submit" type="button" disabled>{button}</button>{notice}</div>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{elementor, normalise, render, Settings};
    use serde_json::json;

    #[test]
    fn scopes_css_and_filters_remote_or_global_rules() {
        let css = custom_css(
            r#"body,selector:hover{color:red} selector::before{content:'';border:1px solid #def}
            selector{width:2rem;background:url(https://example.test/a);position:fixed;}
            @media(max-width:480px){selector{grid-template-columns:1fr!important;}}"#,
            ".scope [data-vb-id=\"x\"]",
            Some(".scope [data-vb-id=\"x\"] > .vb-row"),
        );
        assert!(!css.contains("body") && !css.contains("url(") && !css.contains("fixed"));
        assert!(
            css.contains(r#".scope [data-vb-id="x"]::before{content:'';border:1px solid #def;}"#)
        );
        assert!(css.contains("width:32px"));
        assert!(css.contains(r#"@media(max-width:480px){.scope [data-vb-id="x"] > .vb-row{grid-template-columns:1fr!important;}}"#));
    }

    #[test]
    fn restores_stylesheet_widgets_and_centers_imported_boxed_content() {
        let document = json!({
            "type":"page", "version":"0.4", "content":[{
                "id":"outer", "elType":"container", "settings":{"_element_id":"design"},
                "elements":[
                    {"id":"styles","elType":"widget","widgetType":"html","settings":{
                        "html":"<style>#design .card{border-bottom-width:1px;background:url(https://example.test/a);}</style>\n<style>@media(max-width:480px){#design .card{padding:8px}}</style>"
                    }},
                    {"id":"box","elType":"container","settings":{
                        "content_width":"boxed","boxed_width":{"unit":"px","size":800},
                        "margin":{"unit":"px","top":0,"right":0,"bottom":24,"left":0}
                    },"elements":[
                        {"id":"text","elType":"widget","widgetType":"text-editor","settings":{
                            "editor":"<p>Centered content</p>",
                            "_element_width":"initial",
                            "_element_custom_width":{"unit":"px","size":600},
                            "_flex_align_self":"center"
                        }}
                    ]}
                ]
            }]
        });
        let imported = elementor::import(&document).unwrap();
        assert!(imported.warnings.is_empty());
        let html = render(&imported.layout, &Settings::default());
        assert!(html.contains(".vb-Lp #design .card{border-bottom-width:1px;}"));
        assert!(html.contains("@media(max-width:480px){.vb-Lp #design .card{padding:8px;}}"));
        assert!(html.contains("> .vb-row{width:100%;max-width:800px;margin-inline:auto;}"));
        assert!(html.contains("align-self:center;width:600px;max-width:100%;"));
        assert!(!html.contains("example.test") && !html.contains("vb-w-elementor-widget"));
    }

    #[test]
    fn restores_imported_grid_forms_icons_and_responsive_styles() {
        let document = json!({
            "title":"Template", "type":"page", "version":"0.4", "content":[{
                "id":"grid", "elType":"container", "settings":{
                    "container_type":"grid", "grid_columns_grid":{"unit":"fr","size":2},
                    "grid_columns_grid_tablet":{"unit":"custom","size":"minmax(0px, 0.8fr) minmax(0px, 1.2fr)"},
                    "grid_columns_grid_mobile":{"unit":"custom","size":"minmax(0px, 1fr)"},
                    "grid_rows_grid":{"unit":"custom","size":"auto"},
                    "padding":{"unit":"rem","top":3,"right":1,"bottom":3,"left":1},
                    "background_background":"classic","background_color":"rgba(26,0,51,0.45)"
                }, "elements":[
                    {"id":"a","elType":"widget","widgetType":"heading","settings":{
                        "title":"Example title","typography_font_size":{"unit":"rem","size":2.5},
                        "typography_font_size_mobile":{"unit":"rem","size":1.5},
                        "typography_line_height":{"unit":"em","size":1.25},
                        "custom_css":"selector { border:1px solid #abc; }"
                    }},
                    {"id":"b","elType":"widget","widgetType":"form","settings":{
                        "form_fields":[{"field_label":"First name","width":"50"},{"field_label":"Email","field_type":"email","width":"50"}],
                        "button_text":"Email us","email_to":"private@example.test"
                    }},
                    {"id":"c","elType":"widget","widgetType":"icon-list","settings":{
                        "icon_list":[{"text":"Checklist","selected_icon":{"value":"fas fa-check-circle"}}]
                    }}
                ]
            }]
        });
        let mut imported = elementor::import(&document).unwrap().layout;
        let column = &mut imported["sections"][0]["cols"][0]["items"];
        column[2]["t"] = json!("elementor-widget");
        column[2]["s"] = json!({"label":"icon-list"});
        let imported = normalise(&imported).unwrap();
        let html = render(&imported, &Settings::default());
        for expected in [
            elementor::STYLE_MARKER,
            "repeat(2,minmax(0,1fr))",
            "grid-template-columns:minmax(0px, 0.8fr) minmax(0px, 1.2fr)",
            "grid-template-columns:minmax(0px, 1fr)",
            "grid-template-rows:auto",
            "padding:48px 16px 48px 16px",
            "background:rgba(26,0,51,0.45)",
            "font-size:40px",
            "font-size:24px",
            "@media(max-width:767px)",
            "line-height:1.25em",
            "border:1px solid #abc",
            "Email us",
            "Checklist",
            "disabled",
        ] {
            assert!(html.contains(expected), "missing {expected}");
        }
        assert!(!html.contains("private@example.test"));
        assert!(grid_tracks(&json!({"unit":"custom","size":"1fr;display:none"}), 12.0).is_none());
        assert!(grid_tracks(&json!({"unit":"custom","size":"url(remote)"}), 12.0).is_none());
    }
}
