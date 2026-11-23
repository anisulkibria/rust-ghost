//! Adapters for widgets whose PHP and JavaScript are provided by Elementor add-ons.
use super::{Object, number, str_at, toggle};
use serde_json::{Value, json};

pub(super) fn native(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "elementskit-heading"
        | "jkit_heading"
        | "gum_heading"
        | "page-title"
        | "jkit_post_title" => "heading",
        "elementskit-accordion" | "jkit_accordion" => "accordion",
        "elementskit-testimonial" | "jkit_testimonials" => "testimonials",
        "elementskit-blog-posts"
        | "jkit_post_block"
        | "jkit_post_list"
        | "gum_posts_grid"
        | "gum_post_related" => "posts",
        "ekit-nav-menu" | "jkit_nav_menu" | "navigation-menu" => "nav-menu",
        "jkit_video_button" | "elementskit-video" => "button",
        "jkit_fun_fact" => "stat",
        "jkit_animated_text" => "animated-heading",
        "jkit_icon_box" => "feature",
        "jkit_button" => "button",
        "jkit_social_share" => "share",
        "elementskit-progressbar" => "progress",
        "qi_addons_for_elementor_text_marquee" => "marquee",
        _ => return None,
    })
}

fn first<'a>(source: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .map(|key| str_at(source, key))
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default()
}

fn align(value: &str) -> &str {
    value.trim_start_matches("text_")
}

fn text(value: &str) -> String {
    crate::comments::to_plaintext(value)
}

fn decimal(value: &Value) -> Option<f64> {
    number(value).or_else(|| number(&value["size"]))
}

fn set(target: &mut Object, key: &str, value: &str) {
    if !value.trim().is_empty() {
        target.insert(key.into(), json!(value));
    }
}

pub(super) fn settings(kind: &str, s: &Value, out: &mut Object) {
    match kind {
        "elementskit-heading" | "jkit_heading" | "gum_heading" => {
            let mut title = first(
                s,
                &[
                    "ekit_heading_title",
                    "sg_title_text",
                    "title",
                    "heading_title",
                    "main_heading",
                ],
            )
            .to_owned();
            if kind == "jkit_heading" && !str_at(s, "sg_title_before").is_empty() {
                title = ["sg_title_before", "sg_title_focused", "sg_title_after"]
                    .map(|key| str_at(s, key).trim())
                    .into_iter()
                    .filter(|word| !word.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            let title = text(&title.replace("{{", "").replace("}}", ""));
            set(out, "text", &title);
            set(
                out,
                "tag",
                first(
                    s,
                    &[
                        "ekit_heading_title_tag",
                        "sg_title_tag",
                        "sg_setting_html_tag",
                        "heading_tag",
                    ],
                ),
            );
            set(
                out,
                "align",
                align(first(
                    s,
                    &[
                        "ekit_heading_title_align",
                        "st_general_alignment_responsive",
                    ],
                )),
            );
            let subtitle = first(s, &["ekit_heading_sub_title", "sg_subtitle_heading"]);
            if !subtitle.starts_with("Lorem ipsum")
                || !str_at(s, "ekit_heading_sub_title").is_empty()
            {
                set(out, "subtitle", &text(subtitle));
            }
            set(
                out,
                "colour",
                first(
                    s,
                    &["ekit_heading_title_color", "st_title_color_responsive"],
                ),
            );
            for (prefix, suffix) in [("", ""), ("_tablet", "_t"), ("_mobile", "_m")] {
                for (native, key) in [
                    ("align", "ekit_heading_title_align"),
                    ("align", "st_general_alignment_responsive"),
                ] {
                    set(
                        out,
                        &format!("{native}{suffix}"),
                        align(str_at(s, &format!("{key}{prefix}"))),
                    );
                }
            }
        }
        "page-title" | "jkit_post_title" => {
            set(out, "text", "[[post.title]]");
            set(out, "tag", "h1");
        }
        "elementskit-accordion" | "jkit_accordion" => {
            let (items, title, body, open) = if kind == "jkit_accordion" {
                (
                    "sg_accordion_list",
                    "sg_accordion_list_title",
                    "sg_accordion_list_content",
                    "sg_accordion_list_open",
                )
            } else {
                (
                    "ekit_accordion_items",
                    "acc_title",
                    "acc_content",
                    "acc_active",
                )
            };
            let list: Vec<_> = s[items].as_array().into_iter().flatten()
                .map(|row| json!({"q":str_at(row,title),"a":text(str_at(row,body)),"open":toggle(&row[open])})).collect();
            out.insert("items".into(), json!(list));
            out.insert("single".into(), json!(true));
            set(
                out,
                "title_colour",
                first(
                    s,
                    &[
                        "st_title_close_color_responsive",
                        "ekit_accordion_title_color",
                    ],
                ),
            );
            set(
                out,
                "active_colour",
                first(
                    s,
                    &[
                        "st_title_open_color_responsive",
                        "ekit_accordion_title_color_active",
                    ],
                ),
            );
        }
        "elementskit-testimonial" | "jkit_testimonials" => {
            let (rows, name, role, review, photo) = if kind == "jkit_testimonials" {
                (
                    "sg_testimonials_list",
                    "sg_testimonials_list_client_name",
                    "sg_testimonials_list_designation",
                    "sg_testimonials_list_review",
                    "sg_testimonials_list_client_avatar",
                )
            } else {
                (
                    "ekit_testimonial_data",
                    "client_name",
                    "designation",
                    "review",
                    "client_photo",
                )
            };
            let list: Vec<_> = s[rows].as_array().into_iter().flatten().map(|row| {
                json!({"quote":text(str_at(row,review)), "name":str_at(row,name), "role":str_at(row,role), "avatar":row[photo]["url"]})
            }).collect();
            out.extend(Object::from_iter([
                ("items".into(), json!(list)),
                ("arrows".into(), json!(true)),
                ("dots".into(), json!(true)),
                (
                    "autoplay".into(),
                    json!(
                        toggle(&s["sg_setting_autoplay"])
                            || toggle(&s["ekit_testimonial_autoplay"])
                    ),
                ),
                (
                    "per_view".into(),
                    json!(decimal(&s["ekit_testimonial_slidetoshow"]).unwrap_or(1.0)),
                ),
            ]));
        }
        "elementskit-blog-posts"
        | "jkit_post_block"
        | "jkit_post_list"
        | "gum_posts_grid"
        | "gum_post_related" => {
            let count = ["ekit_blog_posts_num", "number_post", "posts_per_page"]
                .iter()
                .find_map(|key| decimal(&s[*key]))
                .unwrap_or(6.0);
            let columns = ["sg_content_column_responsive", "grid_layout"]
                .iter()
                .find_map(|key| decimal(&s[*key]))
                .unwrap_or(3.0);
            out.extend(Object::from_iter([
                ("count".into(), json!(count)),
                ("columns".into(), json!(columns)),
                (
                    "layout".into(),
                    json!(if kind == "jkit_post_list" {
                        "list"
                    } else {
                        "cards"
                    }),
                ),
                (
                    "source".into(),
                    json!(if kind == "gum_post_related" {
                        "related"
                    } else {
                        "latest"
                    }),
                ),
                ("show_image".into(), json!(true)),
                ("show_excerpt".into(), json!(kind != "jkit_post_list")),
                ("show_meta".into(), json!(true)),
            ]));
            set(
                out,
                "read_more",
                first(s, &["ekit_blog_posts_btn_text", "sg_content_readmore_text"]),
            );
        }
        "ekit-nav-menu" | "jkit_nav_menu" | "navigation-menu" => {
            out.extend(Object::from_iter([
                ("menu".into(), json!("primary")),
                ("layout".into(), json!("horizontal")),
                ("mobile".into(), json!("tablet")),
            ]));
        }
        "jkit_video_button" | "elementskit-video" => {
            let url = first(
                s,
                &[
                    "sg_video_url",
                    "sg_video_youtube_url",
                    "sg_video_vimeo_url",
                    "ekit_video_popup_url",
                    "ekit_video_inline_url",
                ],
            );
            set(
                out,
                "href",
                if url.is_empty() {
                    str_at(&s["sg_video_hosted"], "url")
                } else {
                    url
                },
            );
            let label = first(
                s,
                &[
                    "sg_video_button_title",
                    "ekit_video_popup_button_title",
                    "ekit_video_inline_button_title",
                ],
            );
            set(
                out,
                "text",
                if label.is_empty() {
                    "Play video"
                } else {
                    label
                },
            );
            set(out, "icon", "play");
            out.insert("new_tab".into(), json!(true));
            out.insert("video_popup".into(), json!(true));
            set(
                out,
                "align",
                align(str_at(s, "ekit_video_popup_title_align")),
            );
        }
        "qi_addons_for_elementor_text_marquee" => {
            let items: Vec<_> = s["children"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    let label = text(str_at(row, "item_text"));
                    (!label.is_empty()).then(|| json!({"text":label}))
                })
                .collect();
            out.insert("items".into(), json!(items));
            set(out, "colour", str_at(s, "color"));
        }
        "counter" | "jkit_fun_fact" => {
            let (num, pre, post, label) = if kind == "counter" {
                ("ending_number", "prefix", "suffix", "title")
            } else {
                (
                    "sg_content_number",
                    "sg_content_number_prefix",
                    "sg_content_number_suffix",
                    "sg_content_title",
                )
            };
            if let Some(value) = decimal(&s[num]) {
                set(out, "number", &value.to_string());
            }
            set(out, "prefix", str_at(s, pre));
            let suffix = first(s, &[post, "sg_content_super"]);
            set(out, "suffix", suffix);
            set(out, "label", str_at(s, label));
            if let Some(duration) = decimal(&s["duration"]) {
                out.insert("duration".into(), json!(duration));
            }
            out.insert("animate".into(), json!(true));
        }
        "jkit_animated_text" => {
            let highlight = s["sg_text_style"] == "highlighted";
            let words = if highlight {
                str_at(s, "sg_text_animated").to_owned()
            } else {
                s["sg_text_rotating_list"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|row| str_at(row, "sg_text_rotating_list_text"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            set(out, "words", &words);
            set(out, "before", str_at(s, "sg_text_before").trim());
            set(out, "after", str_at(s, "sg_text_after").trim());
            set(out, "shape", str_at(s, "sg_text_shape"));
            set(out, "style", if highlight { "highlight" } else { "rotate" });
            set(out, "align", str_at(s, "sg_text_alignment_responsive"));
            set(
                out,
                "word_colour",
                str_at(s, "st_text_animated_normal_color_responsive"),
            );
        }
        "jkit_icon_box" => {
            set(
                out,
                "title",
                &text(first(s, &["sg_content_title", "sg_title", "sg_icon_text"])),
            );
            set(
                out,
                "text",
                &text(first(
                    s,
                    &[
                        "sg_content_description",
                        "sg_description_text",
                        "sg_content_text",
                    ],
                )),
            );
            set(
                out,
                "align",
                str_at(s, "sg_setting_content_alignment_responsive"),
            );
            if let Some(icon) = s.get("sg_icon_choose") {
                set(out, "icon", super::icon_name(&icon["value"]));
            }
        }
        "jkit_button" => {
            set(out, "text", str_at(s, "sg_content_label"));
            set(out, "href", first(&s["sg_content_link"], &["url"]));
            set(
                out,
                "colour",
                str_at(s, "st_button_normal_background_background_color"),
            );
            set(
                out,
                "text_colour",
                str_at(s, "st_button_normal_text_color_responsive"),
            );
        }
        "jkit_social_share" => {
            let items: Vec<_> = s["sg_social_list"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    let icon = str_at(&row["sg_social_icon"], "value");
                    let net = match str_at(row, "sg_social_brand") {
                        "twitter" => "x",
                        net @ ("facebook" | "linkedin" | "whatsapp" | "telegram" | "email") => net,
                        _ if icon.ends_with("fa-facebook-f") => "facebook",
                        _ => return None,
                    };
                    Some(json!({"net":net}))
                })
                .collect();
            out.insert("items".into(), json!(items));
        }
        "rating" => {
            if let Some(rating) = decimal(&s["rating_value"]) {
                out.insert("value".into(), json!((rating * 10.0).round() as i64));
            }
            set(out, "colour", str_at(s, "icon_color"));
        }
        "elementskit-progressbar" | "progress" => {
            let value = ["ekit_progressbar_count", "percent"]
                .iter()
                .find_map(|key| decimal(&s[*key]))
                .unwrap_or(0.0);
            out.insert(
                "items".into(),
                json!([{"label":first(s,&["ekit_progressbar_title","title"]),"value":value}]),
            );
            out.insert("show_value".into(), json!(true));
        }
        _ => {}
    }
}
