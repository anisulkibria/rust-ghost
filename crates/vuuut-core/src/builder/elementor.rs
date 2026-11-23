//! Elementor template JSON interchange and scoped rendering of supported styles.

pub(super) mod style;

use super::{spec, Settings};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashSet};

const VERSION: &str = "0.4";
pub const STYLE_MARKER: &str = r#"data-vb-elementor="2""#;
pub(super) const MAX_CONTAINER_DEPTH: u8 = 10;

type Object = Map<String, Value>;
type Control = (&'static str, &'static str, Format);

#[derive(Clone, Copy)]
enum Format {
    Plain,
    Responsive,
    Slider(&'static str),
    Link,
    Media,
    Toggle,
    Sides,
    Font,
    Rich,
}

use Format::*;

const ADVANCED: &[Control] = &[
    ("margin", "_margin", Sides),
    ("padding", "_padding", Sides),
    ("anchor", "_element_id", Plain),
    ("class", "_css_classes", Plain),
    ("hide_d", "hide_desktop", Toggle),
    ("hide_t", "hide_tablet", Toggle),
    ("hide_m", "hide_mobile", Toggle),
    ("z", "_z_index", Plain),
    ("position", "_position", Plain),
    ("anim_delay", "_animation_delay", Plain),
    ("box_colour", "_background_color", Plain),
    ("box_colour2", "_background_color_b", Plain),
    ("box_image", "_background_image", Media),
    ("box_angle", "_background_gradient_angle", Slider("deg")),
    ("border_style", "_border_border", Plain),
    ("border_w", "_border_width", Sides),
    ("border_colour", "_border_color", Plain),
    ("radius_box", "_border_radius", Sides),
];

const STRUCTURE: &[Control] = &[
    ("margin", "margin", Sides),
    ("padding", "padding", Sides),
    ("anchor", "css_id", Plain),
    ("class", "css_classes", Plain),
    ("hide_d", "hide_desktop", Toggle),
    ("hide_t", "hide_tablet", Toggle),
    ("hide_m", "hide_mobile", Toggle),
    ("z", "z_index", Plain),
    ("radius_box", "border_radius", Sides),
    ("border_style", "border_border", Plain),
    ("border_w", "border_width", Sides),
    ("border_colour", "border_color", Plain),
    ("tag", "html_tag", Plain),
];

const TYPOGRAPHY: &[Control] = &[
    ("fs", "typography_font_size", Slider("px")),
    ("font", "typography_font_family", Font),
    ("fw", "typography_font_weight", Plain),
    ("tt", "typography_text_transform", Plain),
    ("fst", "typography_font_style", Plain),
    ("deco", "typography_text_decoration", Plain),
];

const BOX: &[Control] = &[
    ("box_colour", "background_color", Plain),
    ("box_colour2", "background_color_b", Plain),
    ("box_image", "background_image", Media),
    ("box_angle", "background_gradient_angle", Slider("deg")),
];

const SECTION: &[Control] = &[
    ("bg_colour", "background_color", Plain),
    ("bg_colour2", "background_color_b", Plain),
    ("bg_image", "background_image", Media),
    ("bg_angle", "background_gradient_angle", Slider("deg")),
    ("bg_gtype", "background_gradient_type", Plain),
    ("width", "boxed_width", Slider("px")),
    ("min_h", "min_height", Slider("vh")),
    ("bg_pos", "background_position", Plain),
    ("bg_size", "background_size", Plain),
];

const PAGE: &[Control] = &[("bg_colour", "background_color", Plain)];

fn widget(kind: &str) -> Option<(&'static str, &'static [Control])> {
    Some(match kind {
        "heading" => (
            "heading",
            &[
                ("text", "title", Plain),
                ("tag", "header_size", Plain),
                ("link", "link", Link),
                ("align", "align", Responsive),
                ("colour", "title_color", Plain),
            ],
        ),
        "text" => (
            "text-editor",
            &[
                ("html", "editor", Rich),
                ("align", "align", Responsive),
                ("colour", "text_color", Plain),
                ("columns", "text_columns", Responsive),
                ("dropcap", "drop_cap", Toggle),
            ],
        ),
        "button" => (
            "button",
            &[
                ("text", "text", Plain),
                ("href", "link", Link),
                ("align", "align", Responsive),
                ("colour", "background_color", Plain),
                ("text_colour", "button_text_color", Plain),
                ("hover_bg", "button_background_hover_color", Plain),
                ("radius_box", "border_radius", Sides),
                ("hover_text", "hover_color", Plain),
            ],
        ),
        "image" => (
            "image",
            &[
                ("src", "image", Media),
                ("caption", "caption", Plain),
                ("link", "link", Link),
                ("align", "align", Responsive),
                ("width", "width", Slider("%")),
                ("max_h", "height", Slider("px")),
                ("fit", "object-fit", Responsive),
            ],
        ),
        "image-box" => (
            "image-box",
            &[
                ("src", "image", Media),
                ("title", "title_text", Plain),
                ("text", "description_text", Plain),
                ("link", "link", Link),
                ("align", "text_align", Responsive),
                ("layout", "position", Responsive),
            ],
        ),
        "feature" => (
            "icon-box",
            &[
                ("title", "title_text", Plain),
                ("text", "description_text", Plain),
                ("link", "link", Link),
                ("layout", "position", Responsive),
                ("align", "text_align", Responsive),
                ("icon_colour", "primary_color", Plain),
                ("title_colour", "title_color", Plain),
            ],
        ),
        "spacer" => ("spacer", &[("height", "space", Slider("px"))]),
        "divider" => (
            "divider",
            &[
                ("text", "text", Plain),
                ("style", "style", Plain),
                ("width", "width", Slider("%")),
                ("thickness", "weight", Slider("px")),
                ("colour", "color", Plain),
                ("align", "align", Plain),
                ("gap", "gap", Slider("px")),
            ],
        ),
        "anchor" => ("menu-anchor", &[("id", "anchor", Plain)]),
        "video" => (
            "video",
            &[
                ("poster", "image_overlay", Media),
                ("autoplay", "autoplay", Toggle),
                ("loop", "loop", Toggle),
                ("start", "start", Plain),
                ("radius", "border_radius", Slider("px")),
            ],
        ),
        "counter" => (
            "counter",
            &[
                ("from", "starting_number", Plain),
                ("to", "ending_number", Plain),
                ("prefix", "prefix", Plain),
                ("suffix", "suffix", Plain),
                ("label", "title", Plain),
                ("duration", "duration", Plain),
            ],
        ),
        "progress" => (
            "progress",
            &[
                ("title", "title", Plain),
                ("value", "percent", Slider("%")),
                ("colour", "bar_color", Plain),
            ],
        ),
        "map" => (
            "google_maps",
            &[
                ("address", "address", Plain),
                ("zoom", "zoom", Slider("px")),
                ("height", "height", Slider("px")),
            ],
        ),
        "testimonial" => (
            "testimonial",
            &[
                ("quote", "testimonial_content", Plain),
                ("name", "testimonial_name", Plain),
                ("role", "testimonial_job", Plain),
                ("avatar", "testimonial_image", Media),
            ],
        ),
        "alert" => (
            "alert",
            &[
                ("kind", "alert_type", Plain),
                ("title", "alert_title", Plain),
                ("text", "alert_description", Plain),
            ],
        ),
        "gallery" => (
            "image-gallery",
            &[("columns", "gallery_columns", Responsive)],
        ),
        "icon" => (
            "icon",
            &[
                ("size", "size", Slider("px")),
                ("link", "link", Link),
                ("align", "align", Responsive),
            ],
        ),
        "list" => (
            "icon-list",
            &[
                ("icon_colour", "icon_color", Plain),
                ("gap", "space_between", Slider("px")),
            ],
        ),
        "form" => ("form", &[]),
        "accordion" => ("accordion", &[]),
        "carousel" => (
            "image-carousel",
            &[
                ("per_view", "slides_to_show", Responsive),
                ("gap", "image_spacing_custom", Slider("px")),
            ],
        ),
        "social" => (
            "social-icons",
            &[
                ("size", "icon_size", Slider("px")),
                ("gap", "icon_spacing", Slider("px")),
                ("colour", "icon_secondary_color", Plain),
                ("bg", "icon_primary_color", Plain),
            ],
        ),
        "html" => ("html", &[("code", "html", Plain)]),
        _ => return None,
    })
}

fn native_widget(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "text-editor" => "text",
        "menu-anchor" => "anchor",
        "icon-box" => "feature",
        "heading" => "heading",
        "button" => "button",
        "image" => "image",
        "image-box" => "image-box",
        "spacer" => "spacer",
        "divider" => "divider",
        "video" => "video",
        "counter" => "counter",
        "progress" => "progress",
        "google_maps" => "map",
        "testimonial" => "testimonial",
        "alert" => "alert",
        "image-gallery" => "gallery",
        "icon" => "icon",
        "icon-list" => "list",
        "form" => "form",
        "image-carousel" => "carousel",
        "social-icons" => "social",
        "accordion" | "toggle" => "accordion",
        // Imported HTML can contain executable code. It is retained as
        // source data and never inserted into the canvas or public page.
        _ => return None,
    })
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

fn children(value: &Value) -> &[Value] {
    value["elements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn toggle(value: &Value) -> bool {
    value == true
        || matches!(
            value.as_str(),
            Some(
                "yes" | "on" | "1" | "true" | "hidden-desktop" | "hidden-tablet" | "hidden-mobile"
            )
        )
}

fn copy_value(target: &mut Object, key: &str, value: &Value) {
    if !value.is_null() {
        target.insert(key.to_owned(), value.clone());
    }
}

const DEVICES: &[(&str, &str)] = &[("", ""), ("_t", "_tablet"), ("_m", "_mobile")];
const EDGES: &[&str] = &["top", "right", "bottom", "left"];

fn import_controls(source: &Value, target: &mut Object, controls: &[Control]) {
    for &(native, external, format) in controls {
        let devices = match format {
            Responsive | Slider(_) | Sides => DEVICES,
            _ => &DEVICES[..1],
        };
        for (local, remote) in devices {
            let v = &source[format!("{external}{remote}")];
            if v.is_null() {
                continue;
            }
            let converted = match format {
                Plain | Responsive => Some(v.clone()),
                Rich => v
                    .as_str()
                    .map(|s| json!(crate::comments::sanitize_rich_html(s))),
                Link | Media => v.get("url").cloned(),
                Toggle => Some(json!(toggle(v))),
                Slider(unit) => (str_at(v, "unit").is_empty() || v["unit"] == unit)
                    .then(|| number(&v["size"]))
                    .flatten()
                    .map(|n| json!(n)),
                Sides if str_at(v, "unit").is_empty() || v["unit"] == "px" => {
                    let values: Option<Vec<_>> = EDGES.iter().map(|k| number(&v[*k])).collect();
                    values.map(|values| {
                        json!(values
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" "))
                    })
                }
                Sides => None,
                Font => super::FONTS
                    .iter()
                    .find(|f| {
                        v.as_str().is_some_and(|s| {
                            s.eq_ignore_ascii_case(f.3)
                                || s.eq_ignore_ascii_case(f.1)
                                || (s.eq_ignore_ascii_case("Source Sans Pro")
                                    && f.0 == "source-sans-3")
                        })
                    })
                    .map(|f| json!(f.0)),
            };
            if let Some(converted) = converted {
                target.insert(format!("{native}{local}"), converted);
            }
        }
    }
}

fn background(source: &Value, target: &mut Object, key: &str, prefix: &str) {
    let mode = match str_at(source, &format!("{prefix}background_background")) {
        "gradient" => "gradient",
        "classic" if !str_at(&source[format!("{prefix}background_image")], "url").is_empty() => {
            "image"
        }
        "classic" => "colour",
        _ => return,
    };
    target.insert(key.into(), json!(mode));
}

fn icon_name(value: &Value) -> &'static str {
    match value
        .as_str()
        .and_then(|icon| icon.split_whitespace().last())
        .unwrap_or("")
    {
        "fa-arrow-right" | "fa-chevron-right" => "arrow",
        "fa-check" | "fa-check-circle" | "fa-circle-check" => "check",
        "fa-users" | "fa-user-group" => "users",
        "fa-graduation-cap" | "fa-book" => "book",
        "fa-video" | "fa-play" | "fa-play-circle" => "play",
        "fa-tasks" | "fa-list" | "fa-layer-group" => "menu",
        "fa-infinity" | "fa-circle-notch" | "fa-circle" => "○",
        "fa-heart" => "heart",
        "fa-envelope" => "mail",
        "fa-star" => "star",
        _ => "star",
    }
}

pub(super) fn upgrade_widget(metadata: Option<&Value>) -> Option<(&'static str, Object)> {
    let source = &metadata?["source"];
    if !children(source).is_empty() {
        return None;
    }
    let kind = native_widget(str_at(source, "widgetType"))?;
    Some((kind, widget_import(kind, &source["settings"])))
}

fn widget_import(kind: &str, source: &Value) -> Object {
    let mut settings = Object::new();
    let (_, controls) = widget(kind).expect("mapped widget");
    import_controls(source, &mut settings, ADVANCED);
    background(source, &mut settings, "box_bg", "_");
    import_controls(source, &mut settings, TYPOGRAPHY);
    import_controls(source, &mut settings, controls);
    match kind {
        "icon" => {
            settings.insert(
                "icon".into(),
                json!(icon_name(&source["selected_icon"]["value"])),
            );
            let stacked = source["view"] == "stacked";
            if stacked {
                settings.insert(
                    "shape".into(),
                    json!(if source["shape"] == "square" {
                        "square"
                    } else {
                        "circle"
                    }),
                );
                copy_value(&mut settings, "bg", &source["primary_color"]);
            }
            copy_value(
                &mut settings,
                "colour",
                &source[if stacked {
                    "secondary_color"
                } else {
                    "primary_color"
                }],
            );
        }
        "list" => {
            let items: Vec<_> = source["icon_list"].as_array().into_iter().flatten().map(|item| {
                json!({"text": item["text"], "link": item["link"]["url"], "icon": icon_name(&item["selected_icon"]["value"])})
            }).collect();
            settings.insert("items".into(), json!(items));
            if source["view"] == "inline" {
                settings.insert("layout".into(), json!("inline"));
            }
            let typ = |key: &str| source.get(format!("icon_{key}")).cloned();
            let typography = Object::from_iter(
                TYPOGRAPHY
                    .iter()
                    .filter_map(|(_, key, _)| typ(key).map(|v| (key.to_string(), v))),
            );
            import_controls(&Value::Object(typography), &mut settings, TYPOGRAPHY);
        }
        "carousel" => {
            let images: Vec<_> = source["carousel"].as_array().into_iter().flatten()
                .filter_map(|image| Some(json!({"src": image["url"].as_str()?, "alt": image["alt"], "caption": image["caption"]})))
                .collect();
            settings.insert("images".into(), json!(images));
            let navigation = str_at(source, "navigation");
            settings.insert(
                "arrows".into(),
                json!(!matches!(navigation, "dots" | "none")),
            );
            settings.insert(
                "dots".into(),
                json!(!matches!(navigation, "arrows" | "none")),
            );
            settings.insert("lightbox".into(), json!(source["open_lightbox"] == "yes"));
            if source["autoplay"] == "yes" {
                settings.insert("autoplay".into(), json!(true));
                if let Some(speed) = number(&source["autoplay_speed"]) {
                    settings.insert("speed".into(), json!(speed / 1000.0));
                }
            }
        }
        "social" => {
            let items: Vec<_> = source["social_icon_list"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    let network = item["social_icon"]["value"]
                        .as_str()?
                        .split_whitespace()
                        .last()?
                        .strip_prefix("fa-")?;
                    let network = match network {
                        "twitter" | "x-twitter" => "x",
                        "envelope" => "email",
                        "facebook-f" => "facebook",
                        "linkedin-in" => "linkedin",
                        _ => network,
                    };
                    Some(json!({"net": network, "href": item["link"]["url"]}))
                })
                .collect();
            settings.insert("items".into(), json!(items));
            copy_value(&mut settings, "shape", &source["shape"]);
        }
        "accordion" => {
            let items: Vec<_> = source["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| {
                    let body = crate::comments::to_plaintext(str_at(item, "tab_content"));
                    json!({"q": item["tab_title"], "a": body})
                })
                .collect();
            settings.insert("items".into(), json!(items));
        }
        "heading" => {
            if let Some(title) = source["title"].as_str() {
                settings.insert("text".into(), json!(crate::comments::to_plaintext(title)));
            }
        }
        "button" => {
            let sizes = [
                ("xs", "xs"),
                ("sm", "s"),
                ("md", "m"),
                ("lg", "l"),
                ("xl", "xl"),
            ];
            if let Some((_, size)) = sizes
                .iter()
                .find(|(external, _)| source["size"] == *external)
            {
                settings.insert("size".into(), json!(size));
            }
            settings.insert(
                "new_tab".into(),
                json!(toggle(&source["link"]["is_external"])),
            );
            settings.insert(
                "nofollow".into(),
                json!(toggle(&source["link"]["nofollow"])),
            );
            for (device, suffix) in DEVICES {
                if source[format!("align{suffix}")] == "justify" {
                    settings.insert(format!("align{device}"), json!("full"));
                }
            }
        }
        "image" => {
            copy_value(
                &mut settings,
                "alt",
                source.get("image_alt").unwrap_or(&source["image"]["alt"]),
            );
            if source["link_to"] == "file" {
                settings.insert("lightbox".into(), json!(source["open_lightbox"] != "no"));
            }
            if source["caption_source"] == "attachment" {
                copy_value(&mut settings, "caption", &source["image"]["caption"]);
            }
        }
        "gallery" => {
            let images: Vec<_> = source["wp_gallery"].as_array().into_iter().flatten()
                .filter_map(|image| Some(json!({"src": image["url"].as_str()?, "alt": image["alt"], "caption": image["caption"]})))
                .collect();
            settings.insert("images".into(), json!(images));
            settings.insert(
                "lightbox".into(),
                json!(source["gallery_link"] != "none" && source["open_lightbox"] != "no"),
            );
        }
        "video" => {
            let url = match str_at(source, "video_type") {
                "vimeo" => &source["vimeo_url"],
                "hosted" if toggle(&source["insert_url"]) => &source["external_url"]["url"],
                "hosted" => source["hosted_url"]
                    .get("url")
                    .unwrap_or(&source["external_url"]["url"]),
                _ => &source["youtube_url"],
            };
            copy_value(&mut settings, "url", url);
            if source["controls"] == "" {
                settings.insert("controls".into(), json!("off"));
            }
            let ratio = match str_at(source, "aspect_ratio") {
                "169" => "16/9",
                "43" => "4/3",
                "11" => "1/1",
                "916" => "9/16",
                _ => "",
            };
            if !ratio.is_empty() {
                settings.insert("ratio".into(), json!(ratio));
            }
        }
        _ => {}
    }
    settings
}

fn source(element: &Value, opaque: bool) -> Value {
    let mut source = element.as_object().cloned().unwrap_or_default();
    if !opaque {
        source.remove("elements");
    }
    json!({"source": source})
}

pub struct Import {
    pub title: String,
    pub layout: Value,
    pub warnings: Vec<String>,
}

struct Importer {
    next_id: usize,
    unsupported: BTreeSet<String>,
}

impl Importer {
    fn id(&mut self) -> String {
        self.next_id += 1;
        format!("el{}", self.next_id)
    }

    fn column(&mut self, settings: Object, items: Vec<Value>) -> Value {
        json!({"id": self.id(), "s": settings, "items": items})
    }

    fn item(&mut self, element: &Value, depth: u8) -> Value {
        let kind = str_at(element, "widgetType");
        match str_at(element, "elType") {
            "container" if depth < MAX_CONTAINER_DEPTH => {
                let (settings, cols) = self.container(element, false, depth + 1);
                json!({"id": self.id(), "t": "inner", "s": settings, "cols": cols, "elementor": source(element, false)})
            }
            "section" if depth == 0 => {
                let settings = structure_settings(&element["settings"], false, false);
                let cols = self.legacy_columns(element, depth + 1);
                json!({"id": self.id(), "t": "inner", "s": settings, "cols": cols, "elementor": source(element, false)})
            }
            "widget" if children(element).is_empty() && native_widget(kind).is_some() => {
                let native = native_widget(kind).expect("matched widget");
                let settings = widget_import(native, &element["settings"]);
                json!({"id": self.id(), "t": native, "s": settings, "elementor": source(element, false)})
            }
            _ => {
                let label: String = if kind.is_empty() {
                    str_at(element, "elType")
                } else {
                    kind
                }
                .chars()
                .take(70)
                .collect();
                self.unsupported.insert(label.clone());
                json!({"id": self.id(), "t": "elementor-widget", "s": {"label": label}, "elementor": source(element, true)})
            }
        }
    }

    fn legacy_columns(&mut self, element: &Value, depth: u8) -> Vec<Value> {
        let items = children(element);
        let widths: Vec<_> = items
            .iter()
            .map(|c| {
                number(&c["settings"]["_inline_size"])
                    .or_else(|| number(&c["settings"]["_column_size"]))
            })
            .collect();
        let min_width = widths
            .iter()
            .flatten()
            .copied()
            .filter(|w| *w > 0.0)
            .reduce(f64::min)
            .unwrap_or(1.0);
        items
            .iter()
            .zip(widths)
            .map(|(col, width)| {
                let mut settings = structure_settings(&col["settings"], false, false);
                if let Some(width) = width {
                    settings.insert(
                        "w".into(),
                        json!((width / min_width).round().clamp(1.0, 12.0)),
                    );
                }
                let items = children(col)
                    .iter()
                    .map(|child| self.item(child, depth))
                    .collect();
                let mut converted = self.column(settings, items);
                converted["elementor"] = source(col, false);
                converted
            })
            .collect()
    }

    fn container(&mut self, element: &Value, top: bool, depth: u8) -> (Object, Vec<Value>) {
        let raw = &element["settings"];
        let mut settings = structure_settings(raw, top, true);
        let direction = str_at(raw, "flex_direction");
        let row = matches!(direction, "row" | "row-reverse");
        let elements = children(element);
        let cols = if row && elements.len() <= spec::MAX_COLUMNS {
            if direction == "row-reverse" {
                settings.insert("row_reverse".into(), json!(true));
            }
            if str_at(raw, "flex_direction_tablet").starts_with("column") {
                settings.insert("stack".into(), json!("tablet"));
            } else if !str_at(raw, "flex_direction_mobile").starts_with("column") {
                settings.insert("stack".into(), json!("never"));
            }
            let widths: Vec<_> = elements
                .iter()
                .map(|child| {
                    let width = &child["settings"]["width"];
                    (width["unit"] == "%")
                        .then(|| number(&width["size"]))
                        .flatten()
                })
                .collect();
            let min = widths
                .iter()
                .flatten()
                .copied()
                .filter(|w| *w > 0.0)
                .reduce(f64::min)
                .unwrap_or(1.0);
            elements
                .iter()
                .zip(widths)
                .map(|(child, width)| {
                    let mut settings = Object::new();
                    if let Some(width) = width {
                        settings.insert("w".into(), json!((width / min).round().clamp(1.0, 12.0)));
                    }
                    let item = self.item(child, depth);
                    let mut column = self.column(settings, vec![item]);
                    column["elementor"] = json!({"synthetic": true});
                    column
                })
                .collect()
        } else {
            let items = elements
                .iter()
                .map(|child| self.item(child, depth))
                .collect();
            let mut column = Object::new();
            copy_value(&mut column, "gap", &json!(container_gap(raw, "row")));
            let mut column = self.column(column, items);
            column["elementor"] = json!({"synthetic": true});
            vec![column]
        };
        (settings, cols)
    }

    fn section(&mut self, element: &Value) -> Value {
        let (settings, cols) = match str_at(element, "elType") {
            "container" => self.container(element, true, 0),
            "section" => (
                structure_settings(&element["settings"], true, false),
                self.legacy_columns(element, 0),
            ),
            _ => {
                let item = self.item(element, 0);
                return json!({"id": self.id(), "s": {}, "cols": [self.column(Object::new(), vec![item])]});
            }
        };
        json!({"id": self.id(), "s": settings, "cols": cols, "elementor": source(element, false)})
    }
}

fn container_gap(source: &Value, axis: &str) -> Option<f64> {
    let gaps = &source["flex_gap"];
    if !str_at(gaps, "unit").is_empty() && gaps["unit"] != "px" {
        return None;
    }
    number(&gaps[axis]).or_else(|| number(&gaps["size"]))
}

fn structure_settings(raw: &Value, section: bool, container: bool) -> Object {
    let mut settings = Object::new();
    import_controls(raw, &mut settings, STRUCTURE);
    if section {
        import_controls(raw, &mut settings, SECTION);
        background(raw, &mut settings, "bg", "");
        let layout = if raw["content_width"] == "full" || raw["layout"] == "full_width" {
            "full"
        } else {
            "boxed"
        };
        settings.insert("layout".into(), json!(layout));
        for (device, _) in DEVICES {
            if let Some(padding) = settings.remove(&format!("padding{device}")) {
                settings.insert(format!("edge_padding{device}"), padding);
            }
        }
        if !settings.contains_key("edge_padding") {
            settings.insert("pt".into(), json!(0));
            settings.insert("pb".into(), json!(0));
        }
    } else {
        import_controls(raw, &mut settings, BOX);
        background(raw, &mut settings, "box_bg", "");
    }
    if container {
        if let Some(anchor) = raw.get("_element_id") {
            copy_value(&mut settings, "anchor", anchor);
        }
        copy_value(&mut settings, "gap", &json!(container_gap(raw, "column")));
        let align = match str_at(raw, "flex_align_items") {
            "center" => "center",
            "flex-end" => "bottom",
            "stretch" => "stretch",
            _ => "",
        };
        if !align.is_empty() {
            settings.insert("valign".into(), json!(align));
        }
    }
    settings
}

fn snapshot(node: &mut Value) {
    if node["elementor"].is_object() {
        node["elementor"]["snapshot"] = node["s"].clone();
    }
    for key in ["sections", "cols", "items"] {
        if let Some(items) = node[key].as_array_mut() {
            for item in items {
                snapshot(item);
            }
        }
    }
}

fn has_form(element: &Value) -> bool {
    element["widgetType"] == "form" || children(element).iter().any(has_form)
}

pub fn import(input: &Value) -> Result<Import, String> {
    let content = input["content"]
        .as_array()
        .ok_or("Choose a single Elementor template JSON file exported from WordPress.")?;
    if !input["type"].is_string() || !input["version"].is_string() {
        return Err("The file is missing the Elementor template type or version.".into());
    }
    fn valid_elements(elements: &[Value]) -> bool {
        elements.iter().all(|element| {
            element["elType"]
                .as_str()
                .is_some_and(|kind| !kind.is_empty())
                && element.get("elements").is_none_or(Value::is_array)
                && valid_elements(children(element))
                && (element["elType"] != "section"
                    || children(element).iter().all(|e| e["elType"] == "column"))
        })
    }
    if !valid_elements(content) {
        return Err("The Elementor file contains invalid elements.".into());
    }
    let mut importer = Importer {
        next_id: 0,
        unsupported: BTreeSet::new(),
    };
    let sections: Vec<_> = content.iter().map(|e| importer.section(e)).collect();
    let mut page = Object::new();
    import_controls(&input["page_settings"], &mut page, PAGE);
    let mut metadata = input.as_object().cloned().unwrap_or_default();
    metadata.remove("content");
    metadata.remove("title");
    let mut layout = super::normalise(&json!({
        "page": page,
        "sections": sections,
        "elementor": {"document": metadata},
    }))?;
    snapshot(&mut layout);
    layout["elementor"]["snapshot"] = layout["page"].clone();
    if layout.to_string().len() > spec::MAX_BYTES {
        return Err("The converted Elementor template exceeds the builder’s size limit.".into());
    }
    let mut warnings = Vec::new();
    if !importer.unsupported.is_empty() {
        let names = importer
            .unsupported
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(format!("These elements are preserved for Elementor export and hidden on the Vuuut page ({names})."));
    }
    if content.iter().any(has_form) {
        warnings.push(
            "Imported forms show their design. Connect a Vuuut form to accept submissions.".into(),
        );
    }
    let title: String = str_at(input, "title").trim().chars().take(200).collect();
    Ok(Import {
        title: if title.is_empty() {
            "Imported Elementor template".into()
        } else {
            title
        },
        layout,
        warnings,
    })
}

/// The existing Vuuut clipboard format and Elementor files share the same
/// import action. A native import keeps the original section insertion behavior.
pub fn import_layout(input: &Value) -> Result<Import, String> {
    if input.get("content").is_some() {
        return import(input);
    }
    let sections = input
        .as_array()
        .or_else(|| input["sections"].as_array())
        .ok_or("Choose an Elementor or Vuuut layout JSON file.")?;
    if !sections.iter().all(|section| section["cols"].is_array()) {
        return Err("The Vuuut file contains invalid sections.".into());
    }
    let layout = if input.is_array() {
        json!({"sections": sections})
    } else {
        input.clone()
    };
    Ok(Import {
        title: "Imported layout".into(),
        layout: super::normalise(&layout)?,
        warnings: Vec::new(),
    })
}

fn portable_url(url: &str, site_url: &str) -> String {
    if url.starts_with('/') && !url.starts_with("//") {
        format!("{}{url}", site_url.trim_end_matches('/'))
    } else {
        url.to_owned()
    }
}

fn external_colour(value: &Value, site: &Settings) -> Value {
    if let Some(index) = value
        .as_str()
        .and_then(|v| v.strip_prefix('c'))
        .and_then(|v| v.parse::<usize>().ok())
    {
        if let Some(colour) = index.checked_sub(1).and_then(|i| site.palette.get(i)) {
            return json!(if colour.is_empty() {
                "#ff1a75"
            } else {
                colour.as_str()
            });
        }
    }
    value.clone()
}

fn export_controls(
    source: &Value,
    snapshot: &Value,
    target: &mut Object,
    controls: &[Control],
    export: &Exporter<'_>,
) {
    for &(native, external, format) in controls {
        let devices = match format {
            Responsive | Slider(_) | Sides => DEVICES,
            _ => &DEVICES[..1],
        };
        for (local, remote) in devices {
            let v = &source[format!("{native}{local}")];
            if v == &snapshot[format!("{native}{local}")] {
                continue;
            }
            let key = format!("{external}{remote}");
            let converted = match format {
                _ if v.is_null() => None,
                Plain if native.contains("colour") => Some(external_colour(v, export.site)),
                Plain | Responsive | Rich => Some(v.clone()),
                Link | Media => v.as_str().map(|url| json!({"url": portable_url(url, export.site_url), "id": ""})),
                Toggle => Some(json!(if v == true {
                    match external {
                        "hide_desktop" => "hidden-desktop",
                        "hide_tablet" => "hidden-tablet",
                        "hide_mobile" => "hidden-mobile",
                        _ => "yes",
                    }
                } else { "" })),
                Slider(unit) => number(v).map(|n| json!({"unit": unit, "size": n, "sizes": []})),
                Sides => v.as_str().and_then(|v| {
                    let parts: Vec<_> = v.split_whitespace().collect();
                    let [top, right, bottom, left] = parts.as_slice() else { return None };
                    Some(json!({"unit":"px", "top":top, "right":right, "bottom":bottom, "left":left, "isLinked":false}))
                }),
                Font => v.as_str().and_then(|v| super::FONTS.iter().find(|f| f.0 == v && !f.3.is_empty()))
                    .map(|f| json!(f.1)),
            };
            if let Some(value) = converted {
                target.insert(key, value);
            } else {
                target.remove(&key);
            }
            if let Some(globals) = target.get_mut("__globals__").and_then(Value::as_object_mut) {
                globals.remove(&format!("{external}{remote}"));
            }
        }
    }
}

fn setting_changed(node: &Value, key: &str) -> bool {
    node["s"][key] != node["elementor"]["snapshot"][key]
}

struct Exporter<'a> {
    site: &'a Settings,
    site_url: &'a str,
    templates: &'a std::collections::HashMap<String, Value>,
    ids: HashSet<String>,
    next_id: u64,
    warnings: BTreeSet<String>,
    fallback_css: bool,
}

impl Exporter<'_> {
    fn generated_id(&mut self) -> String {
        loop {
            self.next_id += 1;
            let id = format!("{:07x}", self.next_id + 0x100000);
            if self.ids.insert(id.clone()) {
                return id;
            }
        }
    }

    fn element(
        &mut self,
        node: &Value,
        kind: &str,
        widget: Option<&str>,
        settings: Object,
        elements: Vec<Value>,
        inner: bool,
    ) -> Value {
        let mut element = node["elementor"]["source"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if element.get("id").and_then(Value::as_str).is_none() {
            element.insert("id".into(), json!(self.generated_id()));
        }
        element.insert("elType".into(), json!(kind));
        element.entry("isInner").or_insert(json!(inner));
        element.insert(
            "settings".into(),
            if settings.is_empty() {
                element
                    .get("settings")
                    .filter(|s| s.is_object())
                    .map_or_else(|| json!([]), |_| json!({}))
            } else {
                json!(settings)
            },
        );
        element.insert("elements".into(), json!(elements));
        if let Some(widget) = widget {
            element.insert("widgetType".into(), json!(widget));
        }
        Value::Object(element)
    }

    fn widget(&mut self, node: &Value, legacy: bool) -> Option<Value> {
        let kind = str_at(node, "t");
        if kind == "inner" {
            return Some(self.structure(node, false, legacy));
        }
        if kind == "elementor-widget" {
            return node["elementor"]["source"]
                .is_object()
                .then(|| node["elementor"]["source"].clone());
        }
        if !node["elementor"]["source"].is_object()
            && matches!(
                kind,
                "icon" | "list" | "form" | "accordion" | "carousel" | "social"
            )
        {
            return Some(self.fallback(node));
        }
        let Some((external, controls)) = widget(kind) else {
            return Some(self.fallback(node));
        };
        let m = &node["s"];
        let snapshot = &node["elementor"]["snapshot"];
        let imported = node["elementor"]["source"].is_object();
        let mut settings = source_settings(node);
        export_controls(m, snapshot, &mut settings, ADVANCED, self);
        write_background(node, "box_bg", &mut settings, "_");
        export_controls(m, snapshot, &mut settings, TYPOGRAPHY, self);
        export_controls(m, snapshot, &mut settings, controls, self);
        if TYPOGRAPHY
            .iter()
            .any(|(key, _, _)| m[*key].is_string() || m[*key].is_number())
        {
            settings
                .entry("typography_typography")
                .or_insert(json!("custom"));
        }
        match kind {
            "heading" if setting_changed(node, "text") => {
                settings.insert("title".into(), json!(super::esc(str_at(m, "text"))));
            }
            "button" => {
                if setting_changed(node, "size") {
                    let size = match str_at(m, "size") {
                        "s" => "sm",
                        "l" => "lg",
                        "xs" => "xs",
                        "xl" => "xl",
                        _ => "md",
                    };
                    settings.insert("size".into(), json!(size));
                }
                for (device, suffix) in DEVICES {
                    if m[format!("align{device}")] == "full"
                        && m[format!("align{device}")] != snapshot[format!("align{device}")]
                    {
                        settings.insert(format!("align{suffix}"), json!("justify"));
                    }
                }
                if setting_changed(node, "new_tab")
                    || setting_changed(node, "nofollow")
                    || setting_changed(node, "href")
                {
                    let link = settings.entry("link").or_insert(json!({"url": ""}));
                    if let Some(link) = link.as_object_mut() {
                        for (key, remote) in [("new_tab", "is_external"), ("nofollow", "nofollow")]
                        {
                            link.insert(
                                remote.into(),
                                json!(if m[key] == true { "on" } else { "" }),
                            );
                        }
                    }
                }
            }
            "image" => {
                if setting_changed(node, "caption") {
                    settings.insert("caption_source".into(), json!("custom"));
                }
                if setting_changed(node, "src") || setting_changed(node, "alt") {
                    if let Some(image) = settings.get_mut("image").and_then(Value::as_object_mut) {
                        image.insert("alt".into(), m["alt"].clone());
                    }
                }
                if !imported || setting_changed(node, "link") || setting_changed(node, "lightbox") {
                    let link_to = if m["lightbox"] == true {
                        "file"
                    } else if m["link"].is_string() {
                        "custom"
                    } else {
                        "none"
                    };
                    settings.insert("link_to".into(), json!(link_to));
                    settings.insert(
                        "open_lightbox".into(),
                        json!(if m["lightbox"] == true { "yes" } else { "no" }),
                    );
                }
            }
            "gallery" => {
                if setting_changed(node, "images") {
                    let images: Vec<_> = m["images"].as_array().into_iter().flatten().filter_map(|image| {
                        Some(json!({"url": portable_url(image["src"].as_str()?, self.site_url), "id": "", "alt": image["alt"], "caption": image["caption"]}))
                    }).collect();
                    settings.insert("wp_gallery".into(), json!(images));
                }
                if !imported || setting_changed(node, "lightbox") {
                    settings.insert(
                        "gallery_link".into(),
                        json!(if m["lightbox"] == true {
                            "file"
                        } else {
                            "none"
                        }),
                    );
                }
            }
            "video" => {
                if setting_changed(node, "url") {
                    let url = portable_url(str_at(m, "url"), self.site_url);
                    let (kind, key, value) = if url.contains("vimeo.com/") {
                        ("vimeo", "vimeo_url", json!(url))
                    } else if url.contains("youtube.com/") || url.contains("youtu.be/") {
                        ("youtube", "youtube_url", json!(url))
                    } else {
                        ("hosted", "hosted_url", json!({"url": url, "id": ""}))
                    };
                    settings.insert("video_type".into(), json!(kind));
                    settings.insert(key.into(), value);
                }
                if setting_changed(node, "controls") {
                    settings.insert(
                        "controls".into(),
                        json!(if m["controls"] == "off" { "" } else { "yes" }),
                    );
                }
                if setting_changed(node, "poster") {
                    settings.insert(
                        "show_image_overlay".into(),
                        json!(if m["poster"].is_string() { "yes" } else { "" }),
                    );
                }
                if setting_changed(node, "ratio") {
                    let ratio = match str_at(m, "ratio") {
                        "4/3" => "43",
                        "1/1" => "11",
                        "9/16" => "916",
                        _ => "169",
                    };
                    settings.insert("aspect_ratio".into(), json!(ratio));
                }
            }
            _ => {}
        }
        let external = node["elementor"]["source"]["widgetType"]
            .as_str()
            .unwrap_or(external);
        Some(self.element(node, "widget", Some(external), settings, Vec::new(), false))
    }

    fn fallback(&mut self, node: &Value) -> Value {
        let kind = str_at(node, "t");
        self.warnings.insert(format!(
            "Exported as HTML: {}.",
            spec::widget(kind).map_or(kind, |widget| widget.label)
        ));
        let scope = format!("elementor{}", self.generated_id());
        let layout = json!({"sections": [{"id": "section", "s": {"pt": 0, "pb": 0, "layout": "full"},
            "cols": [{"id": "column", "s": {}, "items": [node]}]}]});
        let ctx = super::Ctx {
            settings: self.site,
            scope: &scope,
            editing: false,
            templates: self.templates,
        };
        let html = super::render_with(&layout, &ctx);
        let css = if std::mem::replace(&mut self.fallback_css, true) {
            ""
        } else {
            super::CSS
        };
        let prefix = self.site_url.trim_end_matches('/');
        let html = html
            .replace("src=\"/content/", &format!("src=\"{prefix}/content/"))
            .replace("href=\"/content/", &format!("href=\"{prefix}/content/"))
            .replace("url('/content/", &format!("url('{prefix}/content/"));
        let html = format!("<style>{css}.vb-L{scope}{{width:100%;max-width:100%;margin-inline:0!important}}</style>{html}");
        self.element(
            node,
            "widget",
            Some("html"),
            Object::from_iter([("html".into(), json!(html))]),
            Vec::new(),
            false,
        )
    }

    fn column(&mut self, node: &Value, legacy: bool, width: f64) -> Value {
        let mut settings = source_settings(node);
        let m = &node["s"];
        let snapshot = &node["elementor"]["snapshot"];
        let imported = node["elementor"]["source"].is_object();
        export_controls(m, snapshot, &mut settings, STRUCTURE, self);
        export_controls(m, snapshot, &mut settings, BOX, self);
        write_background(node, "box_bg", &mut settings, "");
        let elements = node["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| self.widget(w, legacy))
            .collect();
        if legacy {
            if !imported || setting_changed(node, "w") {
                settings.insert("_column_size".into(), json!(width.round() as i64));
                settings.insert("_inline_size".into(), json!(width));
            }
        } else {
            settings.insert("flex_direction".into(), json!("column"));
            settings.insert("content_width".into(), json!("full"));
            settings.insert(
                "width".into(),
                json!({"unit":"%", "size":width, "sizes":[]}),
            );
            if let Some(gap) = number(&m["gap"]) {
                settings.insert(
                    "flex_gap".into(),
                    json!({"unit":"px", "column":gap, "row":gap, "isLinked":true}),
                );
            }
        }
        self.element(
            node,
            if legacy { "column" } else { "container" },
            None,
            settings,
            elements,
            !legacy,
        )
    }

    fn structure(&mut self, node: &Value, section: bool, legacy_parent: bool) -> Value {
        let original = &node["elementor"]["source"];
        let imported = original.is_object();
        let legacy = original["elType"] == "section" || (!imported && legacy_parent);
        let m = &node["s"];
        let snapshot = &node["elementor"]["snapshot"];
        let mut settings = source_settings(node);
        export_controls(m, snapshot, &mut settings, STRUCTURE, self);
        let cols: Vec<_> = node["cols"].as_array().into_iter().flatten().collect();
        if section {
            export_controls(m, snapshot, &mut settings, SECTION, self);
            write_background(node, "bg", &mut settings, "");
            for (device, suffix) in DEVICES {
                let key = format!("edge_padding{device}");
                let top = format!("pt{device}");
                let bottom = format!("pb{device}");
                if setting_changed(node, &key)
                    || setting_changed(node, &top)
                    || setting_changed(node, &bottom)
                    || (!imported && device.is_empty())
                {
                    let existing = str_at(m, &key);
                    let mut sides: Vec<_> =
                        existing.split_whitespace().map(str::to_owned).collect();
                    if sides.len() != 4 {
                        sides = vec!["0".into(); 4];
                    }
                    for (index, key) in [(0, top), (2, bottom)] {
                        if let Some(size) = m[&key].as_i64().filter(|size| *size >= 0) {
                            sides[index] = size.to_string();
                        } else if existing.is_empty() {
                            sides[index] = self.site.space.to_string();
                        }
                    }
                    settings.insert(format!("padding{suffix}"), json!({"unit":"px", "top":sides[0], "right":sides[1], "bottom":sides[2], "left":sides[3], "isLinked":false}));
                }
            }
        } else {
            export_controls(m, snapshot, &mut settings, BOX, self);
            write_background(node, "box_bg", &mut settings, "");
        }
        if !imported || setting_changed(node, "layout") {
            let full = m["layout"] == "full" || !section;
            if legacy {
                settings.insert(
                    "layout".into(),
                    json!(if full { "full_width" } else { "boxed" }),
                );
            } else {
                settings.insert(
                    "content_width".into(),
                    json!(if full { "full" } else { "boxed" }),
                );
                if !full && !settings.contains_key("boxed_width") {
                    settings.insert(
                        "boxed_width".into(),
                        json!({"unit":"px", "size":self.site.width, "sizes":[]}),
                    );
                }
            }
        }
        if !legacy {
            if setting_changed(node, "anchor") {
                settings.remove("css_id");
                copy_value(&mut settings, "_element_id", &m["anchor"]);
                if m["anchor"].is_null() {
                    settings.remove("_element_id");
                }
            }
            if !imported || setting_changed(node, "row_reverse") {
                settings.insert(
                    "flex_direction".into(),
                    json!(if m["row_reverse"] == true {
                        "row-reverse"
                    } else {
                        "row"
                    }),
                );
            }
            if !imported || setting_changed(node, "stack") {
                let mobile = if m["stack"] == "never" {
                    "row"
                } else {
                    "column"
                };
                settings.insert("flex_direction_mobile".into(), json!(mobile));
                if m["stack"] == "tablet" {
                    settings.insert("flex_direction_tablet".into(), json!("column"));
                }
            }
            if !imported || setting_changed(node, "valign") {
                let align = match str_at(m, "valign") {
                    "center" => "center",
                    "bottom" => "flex-end",
                    "stretch" => "stretch",
                    _ => "flex-start",
                };
                settings.insert("flex_align_items".into(), json!(align));
            }
            for (device, suffix) in DEVICES {
                let key = format!("gap{device}");
                if setting_changed(node, &key) || (!imported && device.is_empty()) {
                    let gap = number(&m[&key]).unwrap_or(32.0);
                    settings.insert(
                        format!("flex_gap{suffix}"),
                        json!({"unit":"px", "size":gap, "column":gap, "row":gap, "isLinked":true}),
                    );
                }
            }
        }
        let total = cols
            .iter()
            .map(|c| number(&c["s"]["w"]).unwrap_or(1.0))
            .sum::<f64>();
        let row = matches!(
            settings.get("flex_direction").and_then(Value::as_str),
            Some("row" | "row-reverse")
        );
        let mut elements = Vec::new();
        for column in cols {
            let items = column["items"].as_array().into_iter().flatten();
            let unchanged = column["elementor"]["synthetic"] == true
                && column["s"] == column["elementor"]["snapshot"];
            if !legacy && imported && unchanged && (!row || items.clone().count() == 1) {
                elements.extend(items.filter_map(|widget| self.widget(widget, false)));
            } else {
                let width = 100.0 * number(&column["s"]["w"]).unwrap_or(1.0) / total;
                elements.push(self.column(column, legacy, width));
            }
        }
        self.element(
            node,
            if legacy { "section" } else { "container" },
            None,
            settings,
            elements,
            !section,
        )
    }
}

fn write_background(node: &Value, key: &str, settings: &mut Object, prefix: &str) {
    if setting_changed(node, key) {
        let kind = match str_at(&node["s"], key) {
            "colour" | "image" => "classic",
            "gradient" => "gradient",
            _ => "",
        };
        settings.insert(format!("{prefix}background_background"), json!(kind));
    }
}

fn source_settings(node: &Value) -> Object {
    node["elementor"]["source"]["settings"]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn visit_source_ids(node: &Value, ids: &mut HashSet<String>) {
    if let Some(id) = node["id"].as_str() {
        ids.insert(id.to_owned());
    }
    for child in children(node) {
        visit_source_ids(child, ids);
    }
}

fn reserve_source_ids(node: &Value, ids: &mut HashSet<String>) {
    visit_source_ids(&node["elementor"]["source"], ids);
    for key in ["sections", "cols", "items"] {
        for child in node[key].as_array().into_iter().flatten() {
            reserve_source_ids(child, ids);
        }
    }
}

fn make_ids_unique(elements: &mut [Value], used: &mut HashSet<String>, export: &mut Exporter<'_>) {
    for element in elements {
        if !element["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && used.insert(id.to_owned()))
        {
            let id = export.generated_id();
            used.insert(id.clone());
            element["id"] = json!(id);
        }
        if let Some(children) = element["elements"].as_array_mut() {
            make_ids_unique(children, used, export);
        }
    }
}

pub struct Export {
    pub document: Value,
    pub warnings: Vec<String>,
}

pub fn export(
    layout: &Value,
    title: &str,
    template_kind: &str,
    site: &Settings,
    site_url: &str,
    templates: &std::collections::HashMap<String, Value>,
) -> Result<Export, String> {
    let mut export = Exporter {
        site,
        site_url,
        templates,
        ids: HashSet::new(),
        next_id: 0,
        warnings: BTreeSet::new(),
        fallback_css: false,
    };
    reserve_source_ids(layout, &mut export.ids);
    let mut content: Vec<_> = layout["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| export.structure(node, true, false))
        .collect();
    if content.is_empty() {
        return Err("Add a section before exporting this layout.".into());
    }
    make_ids_unique(&mut content, &mut HashSet::new(), &mut export);
    let metadata = &layout["elementor"];
    let mut document = metadata["document"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut page_settings = document
        .get("page_settings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let page = &layout["page"];
    export_controls(
        page,
        &metadata["snapshot"],
        &mut page_settings,
        PAGE,
        &export,
    );
    if page["bg_colour"] != metadata["snapshot"]["bg_colour"] {
        page_settings.insert(
            "background_background".into(),
            json!(if page["bg_colour"].is_string() {
                "classic"
            } else {
                ""
            }),
        );
    }
    document.insert(
        "page_settings".into(),
        if page_settings.is_empty() {
            json!([])
        } else {
            json!(page_settings)
        },
    );
    let kind = match template_kind {
        "section" | "header" | "footer" | "single" | "popup" => template_kind,
        _ => "page",
    };
    document.entry("type").or_insert(json!(kind));
    document.insert("version".into(), json!(VERSION));
    document.insert("title".into(), json!(title));
    document.insert("content".into(), json!(content));
    Ok(Export {
        document: Value::Object(document),
        warnings: export.warnings.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sections_and_opaque_widgets_keep_their_elementor_data() {
        let elements = json!([{
            "id": "ae123ff", "elType": "section", "isInner": false,
            "settings": {"layout": "boxed", "padding_mobile": {"unit": "%", "top": "4"}, "custom_css": "selector { opacity: .9 }"},
            "elements": [{"id": "ae123f1", "elType": "column", "isInner": false,
                "settings": {"_column_size": 100},
                "elements": [
                    {"id": "ae123f2", "elType": "widget", "isInner": false, "widgetType": "heading", "settings": {"title": "<b>A title</b>", "align_mobile": "center"}, "elements": []},
                    {"id": "ae123f3", "elType": "widget", "isInner": false, "widgetType": "third-party-form", "settings": {"shortcode": "[example]", "__globals__": {"colour": "globals/colors?id=primary"}}, "elements": []}
                ]
            }]
        }]);
        let input = json!({"title":"Legacy","type":"section","version":"0.4","page_settings":[],"content":elements});
        let imported = import(&input).unwrap();
        let layout = super::super::normalise(&imported.layout).unwrap();
        let rendered = super::super::render(&layout, &Settings::default());
        assert!(rendered.contains("A title"));
        assert!(!rendered.contains("[example]"));
        assert!(!super::super::plain_text(&layout).contains("third-party-form"));
        let output = export(
            &layout,
            "Legacy",
            "page",
            &Settings::default(),
            "https://example.test",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(output.document, input);
    }

    #[test]
    fn native_layout_exports_portable_media_and_editable_widgets() {
        let input = json!({"sections":[{"s":{"layout":"full"},"cols":[{"items":[
            {"t":"heading", "s":{"text":"< Product & news >", "align_m":"center"}},
            {"t":"image", "s":{"src":"/content/images/example.png", "alt":"Example"}},
            {"t":"button", "s":{"text":"Learn more", "href":"https://example.org", "new_tab":true}}
        ]}]}]});
        let layout = import_layout(&input).unwrap().layout;
        let output = export(
            &layout,
            "Native page",
            "page",
            &Settings::default(),
            "https://example.test",
            &Default::default(),
        )
        .unwrap();
        let items = &output.document["content"][0]["elements"][0]["elements"];
        assert_eq!(items[0]["widgetType"], "heading");
        assert_eq!(items[0]["settings"]["align_mobile"], "center");
        assert_eq!(
            items[1]["settings"]["image"]["url"],
            "https://example.test/content/images/example.png"
        );
        assert_eq!(items[2]["settings"]["link"]["is_external"], "on");
        let restored = import(&output.document).unwrap().layout;
        let items = &restored["sections"][0]["cols"][0]["items"][0]["cols"][0]["items"];
        assert_eq!(items[0]["s"]["text"], "< Product & news >");
        assert_eq!(items[2]["s"]["new_tab"], true);
    }
}
