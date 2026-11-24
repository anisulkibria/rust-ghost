//! Each widget's own markup and rules. Shared styling (box, typography,
//! advanced) is added around it by `render::widget`.

use super::dynamic;
use super::icons::{brand, icon_or_text, icon_svg, social_svg};
use super::render::{b, colour, css_url, href, n, paragraphs, px, s, text, R};
use super::esc;
use serde_json::Value;

/// Where a widget's typography applies, below the widget itself.
pub(super) fn typo_target(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "heading" => ".vb-h",
        "text" => ".vb-text",
        "button" | "pay-button" => ".vb-btn",
        "list" => ".vb-list",
        "feature" | "image-box" => ".vb-feature-body",
        "stat" => ".vb-stat-n",
        "testimonial" | "testimonials" => "blockquote",
        "accordion" => "summary",
        "tabs" => ".vb-tab-btn",
        "animated-heading" => ".vb-ah",
        "nav-menu" => ".vb-menu a",
        "blockquote" => "blockquote",
        _ => return None,
    })
}

fn align_class(m: &Value) -> String {
    match s(m, "align") {
        "" => " vb-al-left".into(),
        a => format!(" vb-al-{a}"),
    }
}

/// `align` with tablet and mobile values, as `text-align` + flex alignment.
fn align_rules(r: &mut R, m: &Value, sel: &str) {
    r.resp(m, "align", sel, |v| {
        v.as_str()
            .map(|a| {
                let j = match a {
                    "center" => "center",
                    "right" => "flex-end",
                    "justify" | "full" => "stretch",
                    _ => "flex-start",
                };
                format!("text-align:{};--vb-jc:{j};", if a == "full" { "center" } else { a })
            })
            .unwrap_or_default()
    });
}

fn link_open(href_: &str, new_tab: bool, class: &str, extra: &str) -> String {
    format!(
        r#"<a class="{class}" href="{}"{}{extra}>"#,
        href(href_),
        if new_tab { r#" target="_blank" rel="noopener""# } else { "" },
    )
}

pub fn youtube_id(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = ["https://www.youtube.com/", "https://youtube.com/", "https://m.youtube.com/", "https://youtu.be/", "https://www.youtube-nocookie.com/"]
        .iter()
        .find_map(|p| u.strip_prefix(p).map(|r| (*p, r)))?;
    let candidate = if rest.0.contains("youtu.be") {
        rest.1.split(['?', '&', '/', '#']).next()?.to_string()
    } else if let Some(q) = rest.1.strip_prefix("watch?") {
        q.split('&').find_map(|kv| kv.strip_prefix("v="))?.to_string()
    } else if let Some(p) = rest.1.strip_prefix("embed/").or_else(|| rest.1.strip_prefix("shorts/")).or_else(|| rest.1.strip_prefix("live/")) {
        p.split(['?', '&', '/', '#']).next()?.to_string()
    } else {
        return None;
    };
    (candidate.len() == 11 && candidate.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(candidate)
}

pub fn vimeo_id(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = u.strip_prefix("https://vimeo.com/").or_else(|| u.strip_prefix("https://player.vimeo.com/video/"))?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!id.is_empty() && id.len() < 15).then_some(id)
}

/// The embeddable address of a video link, or `None` for a direct file.
pub fn embed_src(url: &str, autoplay: bool, mute: bool, looped: bool, controls: bool, start: i64) -> Option<String> {
    if let Some(id) = youtube_id(url) {
        let mut q = vec!["rel=0".to_string(), "playsinline=1".into()];
        if autoplay {
            q.push("autoplay=1".into());
        }
        if mute {
            q.push("mute=1".into());
        }
        if looped {
            q.push(format!("loop=1&amp;playlist={id}"));
        }
        if !controls {
            q.push("controls=0".into());
        }
        if start > 0 {
            q.push(format!("start={start}"));
        }
        return Some(format!("https://www.youtube-nocookie.com/embed/{id}?{}", q.join("&amp;")));
    }
    if let Some(id) = vimeo_id(url) {
        let mut q = vec!["dnt=1".to_string()];
        if autoplay {
            q.push("autoplay=1".into());
        }
        if mute {
            q.push("muted=1".into());
        }
        if looped {
            q.push("loop=1".into());
        }
        if !controls {
            q.push("controls=0".into());
        }
        let hash = if start > 0 { format!("#t={start}s") } else { String::new() };
        return Some(format!("https://player.vimeo.com/video/{id}?{}{hash}", q.join("&amp;")));
    }
    None
}

fn empty(label: &str) -> String {
    format!(r#"<div class="vb-empty"><span>{}</span></div>"#, esc(label))
}

fn percent(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn stars(count: f64, max: i64, icon: &str) -> String {
    let ic = icon_svg(icon).unwrap_or_default();
    let mut out = String::new();
    for i in 0..max {
        let fill = (count - i as f64).clamp(0.0, 1.0);
        out.push_str(&format!(r#"<i class="vb-star" style="--vb-fill:{}%">{ic}{ic}</i>"#, (fill * 100.0).round()));
    }
    out
}

fn carousel_attrs(m: &Value, r: &mut R, sel: &str, default_pv: i64) -> String {
    r.uses_runtime = true;
    let pv = n(m, "per_view").unwrap_or(default_pv);
    r.rule("", sel, &format!("--vb-pv:{pv};--vb-cg:{}px;", n(m, "gap").unwrap_or(16)));
    r.resp(m, "per_view", sel, |v| v.as_i64().map(|p| format!("--vb-pv:{p};")).unwrap_or_default());
    format!(
        r#" data-vb-carousel data-autoplay="{}" data-loop="{}" data-arrows="{}" data-dots="{}"{}"#,
        if b(m, "autoplay") { n(m, "speed").unwrap_or(5) * 1000 } else { 0 },
        b(m, "loop") || b(m, "autoplay"),
        m["arrows"].as_bool().unwrap_or(true),
        m["dots"].as_bool().unwrap_or(true),
        if s(m, "transition") == "fade" { r#" data-fade="1""# } else { "" },
    )
}

fn panel_content(r: &mut R, widget: &Value, index: usize, text: &str) -> String {
    match widget["cols"].get(index) {
        Some(col) => super::render::column(r, col),
        None => paragraphs(text),
    }
}

pub(super) fn html(r: &mut R, kind: &str, id: &str, m: &Value, sel: &str, w: &Value) -> String {
    match kind {
        "heading" => {
            let tag = match s(m, "tag") {
                "" => "h2",
                t => t,
            };
            let hs = format!("{sel} .vb-h");
            align_rules(r, m, &hs);
            r.resp(m, "size", &hs, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:min({n}px,calc({n}px * .55 + 2.2vw));")).unwrap_or_default());
            let mut d = String::new();
            if !s(m, "weight").is_empty() {
                d.push_str(&format!("font-weight:{};", s(m, "weight")));
            }
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("color:{c};"));
            }
            if let Some(g) = colour(m, "gradient") {
                d.push_str(&format!("background:linear-gradient(90deg,{},{g});-webkit-background-clip:text;background-clip:text;color:transparent;", colour(m, "colour").unwrap_or_else(|| "var(--vb-c1)".into())));
            }
            r.rule("", &hs, &d);
            let t = text(s(m, "text"));
            let t = match m["link"].as_str() {
                Some(l) => format!("{}{t}</a>", link_open(l, false, "vb-hlink", "")),
                None => t,
            };
            let subtitle = s(m, "subtitle");
            let subtitle = if subtitle.is_empty() { String::new() } else {
                align_rules(r, m, &format!("{sel} .vb-h-subtitle"));
                format!(r#"<p class="vb-h-subtitle">{}</p>"#, text(subtitle))
            };
            format!(r#"<{tag} class="vb-h">{t}</{tag}>{subtitle}"#)
        }
        "text" => {
            let ts = format!("{sel} .vb-text");
            align_rules(r, m, &ts);
            r.resp(m, "size", &ts, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:{n}px;")).unwrap_or_default());
            r.resp(m, "columns", &ts, |v| v.as_i64().map(|c| format!("columns:{c};column-gap:32px;")).unwrap_or_default());
            if let Some(c) = colour(m, "colour") {
                r.rule("", &ts, &format!("color:{c};"));
            }
            let body = dynamic::mark_tags(&crate::comments::sanitize_rich_html(s(m, "html")));
            format!(r#"<div class="vb-text{}">{body}</div>"#, if b(m, "dropcap") { " vb-dropcap" } else { "" })
        }
        "button" => button(r, m, sel, w),
        "marquee" => {
            if let Some(c) = colour(m, "colour") {
                r.rule("", sel, &format!("color:{c};"));
            }
            let words: String = m["items"].as_array().into_iter().flatten()
                .map(|item| format!("<span>{}</span>", text(s(item, "text")))).collect();
            format!(r#"<div class="vb-marquee"><div class="vb-marquee-track"><div>{words}</div><div aria-hidden="true">{words}</div></div></div>"#)
        }
        "image" => {
            let Some(src) = m["src"].as_str() else { return empty("Image") };
            let is = format!("{sel} img");
            r.resp(m, "width", &is, |v| v.as_i64().map(|w| format!("width:{w}%;")).unwrap_or_default());
            r.resp(m, "max_h", &is, |v| v.as_i64().filter(|h| *h > 0).map(|h| format!("max-height:{h}px;width:auto;")).unwrap_or_default());
            r.resp(m, "align", &format!("{sel} .vb-img"), |v| v.as_str().map(|a| format!("text-align:{a};")).unwrap_or_default());
            let mut d = String::new();
            if let Some(rad) = n(m, "radius") {
                d.push_str(&format!("border-radius:{rad}px;"));
            }
            if !s(m, "fit").is_empty() {
                d.push_str(&format!("object-fit:{};", s(m, "fit")));
            }
            r.rule("", &is, &d);
            let img = format!(
                r#"<img src="{}" alt="{}" loading="lazy" decoding="async"{}>"#,
                href(src),
                esc(s(m, "alt")),
                if b(m, "img_shadow") { r#" class="vb-shadow""# } else { "" },
            );
            let img = if b(m, "lightbox") {
                r.uses_runtime = true;
                format!(r#"<a class="vb-img-link" href="{}" data-vb-lightbox="{}">{img}</a>"#, href(src), esc(id))
            } else {
                match m["link"].as_str() {
                    Some(l) => format!("{}{img}</a>", link_open(l, false, "vb-img-link", "")),
                    None => img,
                }
            };
            let cap = match s(m, "caption") {
                "" => String::new(),
                c => format!("<figcaption>{}</figcaption>", text(c)),
            };
            format!(r#"<figure class="vb-img vb-ih-{}">{img}{cap}</figure>"#, match s(m, "hover") { "" => "none", h => h })
        }
        "feature" => {
            let mut ic = String::new();
            if let Some(c) = colour(m, "icon_colour") {
                ic.push_str(&format!("color:{c};"));
            }
            if let Some(c) = colour(m, "icon_bg") {
                ic.push_str(&format!("background:{c};"));
            }
            let isel = format!("{sel} .vb-icon");
            r.rule("", &isel, &ic);
            r.resp(m, "icon_size", &isel, |v| v.as_i64().map(|z| format!("font-size:{}px;width:{}px;height:{}px;", z, z * 2, z * 2)).unwrap_or_default());
            if let Some(c) = colour(m, "title_colour") {
                r.rule("", &format!("{sel} h3"), &format!("color:{c};"));
            }
            align_rules(r, m, &format!("{sel} .vb-feature"));
            r.resp(m, "layout", &format!("{sel} .vb-feature"), |v| match v.as_str() {
                Some("left") => "flex-direction:row;text-align:left;".into(),
                Some("right") => "flex-direction:row-reverse;text-align:right;".into(),
                Some("top") => "flex-direction:column;".into(),
                _ => String::new(),
            });
            let icon = match s(m, "icon") {
                "" => icon_svg("sparkle").unwrap_or_default(),
                i => icon_or_text(i),
            };
            let title = match (s(m, "title"), m["link"].as_str()) {
                ("", _) => String::new(),
                (t, Some(l)) => format!("<h3>{}{}</a></h3>", link_open(l, false, "vb-hlink", ""), text(t)),
                (t, None) => format!("<h3>{}</h3>", text(t)),
            };
            format!(
                r#"<div class="vb-feature vb-feature-{layout}"><span class="vb-icon vb-shape-{shape}">{icon}</span><div class="vb-feature-body">{title}{body}</div></div>"#,
                layout = match s(m, "layout") { "left" => "left", "right" => "right", _ => "top" },
                shape = match s(m, "icon_shape") { "" => "square", x => x },
                body = paragraphs(s(m, "text")),
            )
        }
        "image-box" => {
            align_rules(r, m, &format!("{sel} .vb-feature"));
            r.resp(m, "img_w", &format!("{sel} .vb-ib-img"), |v| v.as_i64().map(|w| format!("width:{w}%;")).unwrap_or_default());
            r.resp(m, "layout", &format!("{sel} .vb-feature"), |v| match v.as_str() {
                Some("left") => "flex-direction:row;text-align:left;".into(),
                Some("right") => "flex-direction:row-reverse;text-align:right;".into(),
                Some("top") => "flex-direction:column;".into(),
                _ => String::new(),
            });
            if let Some(rad) = n(m, "radius") {
                r.rule("", &format!("{sel} .vb-ib-img img"), &format!("border-radius:{rad}px;"));
            }
            let img = m["src"].as_str().map(|u| format!(r#"<span class="vb-ib-img"><img src="{}" alt="{}" loading="lazy"></span>"#, href(u), esc(s(m, "alt")))).unwrap_or_default();
            let title = match (s(m, "title"), m["link"].as_str()) {
                ("", _) => String::new(),
                (t, Some(l)) => format!("<h3>{}{}</a></h3>", link_open(l, false, "vb-hlink", ""), text(t)),
                (t, None) => format!("<h3>{}</h3>", text(t)),
            };
            format!(
                r#"<div class="vb-feature vb-feature-{layout} vb-imagebox">{img}<div class="vb-feature-body">{title}{body}</div></div>"#,
                layout = match s(m, "layout") { "left" => "left", "right" => "right", _ => "top" },
                body = paragraphs(s(m, "text")),
            )
        }
        "icon" => {
            let isel = format!("{sel} .vb-icon");
            let mut d = String::new();
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("color:{c};"));
            }
            match colour(m, "bg") {
                Some(c) => d.push_str(&format!("background:{c};")),
                None => d.push_str("background:none;"),
            }
            r.rule("", &isel, &d);
            r.resp(m, "size", &isel, |v| v.as_i64().map(|z| if s(m, "shape") == "none" || s(m, "shape").is_empty() { format!("font-size:{z}px;width:auto;height:auto;") } else { format!("font-size:{z}px;width:{}px;height:{}px;", z * 2, z * 2) }).unwrap_or_default());
            if let Some(c) = colour(m, "hover_colour") {
                r.rule("", &format!("{isel}:hover"), &format!("color:{c};"));
            }
            align_rules(r, m, &format!("{sel} .vb-icon-wrap"));
            let icon = icon_or_text(match s(m, "icon") { "" => "star", i => i });
            let inner = format!(r#"<span class="vb-icon vb-shape-{}">{icon}</span>"#, match s(m, "shape") { "" => "none", x => x });
            let inner = match m["link"].as_str() {
                Some(l) => format!("{}{inner}</a>", link_open(l, false, "vb-icon-link", "")),
                None => inner,
            };
            format!(r#"<div class="vb-icon-wrap">{inner}</div>"#)
        }
        "list" => {
            let icon = match s(m, "icon") { "" => "check", i => i };
            let mut d = String::new();
            if let Some(c) = colour(m, "icon_colour") {
                d.push_str(&format!("--vb-ic:{c};"));
            }
            let lsel = format!("{sel} .vb-list");
            r.rule("", &lsel, &d);
            r.resp(m, "size", &lsel, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:{n}px;")).unwrap_or_default());
            r.resp(m, "gap", &lsel, |v| format!("gap:{};", px(v)));
            let tag = if icon == "number" { "ol" } else { "ul" };
            let items: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let own = s(i, "icon");
                    let mark = if !own.is_empty() {
                        format!(r#"<i class="vb-li-ic">{}</i>"#, icon_or_text(own))
                    } else if matches!(icon, "check" | "arrow" | "star") {
                        format!(r#"<i class="vb-li-ic">{}</i>"#, icon_svg(icon).unwrap_or_default())
                    } else {
                        String::new()
                    };
                    let t = format!("<span>{}</span>", text(s(i, "text")));
                    let body = match i["link"].as_str() {
                        Some(l) => format!("{}{mark}{t}</a>", link_open(l, false, "vb-li-link", "")),
                        None => format!("{mark}{t}"),
                    };
                    format!("<li>{body}</li>")
                })
                .collect();
            format!(
                r#"<{tag} class="vb-list vb-list-{icon}{inline}{div}">{items}</{tag}>"#,
                inline = if s(m, "layout") == "inline" { " vb-list-inline" } else { "" },
                div = if b(m, "divider") { " vb-list-div" } else { "" },
            )
        }
        "spacer" => {
            r.resp(m, "height", &format!("{sel} .vb-spacer"), |v| format!("height:{};", px(v)));
            r#"<div class="vb-spacer" style="height:40px"></div>"#.into()
        }
        "divider" => {
            let line = format!(
                "border-top:{}px {} {};",
                n(m, "thickness").unwrap_or(1),
                match s(m, "style") { "" => "solid", v => v },
                colour(m, "colour").unwrap_or_else(|| "color-mix(in srgb,currentColor 25%,transparent)".into()),
            );
            let dsel = format!("{sel} .vb-divider");
            r.rule("", &format!("{dsel}::before,{dsel}::after"), &line);
            r.resp(m, "width", &dsel, |v| v.as_i64().map(|w| format!("width:{w}%;")).unwrap_or_default());
            r.resp(m, "gap", &dsel, |v| format!("margin-block:{};", px(v)));
            let margin = match s(m, "align") {
                "left" => "margin-right:auto;",
                "right" => "margin-left:auto;",
                _ => "margin-inline:auto;",
            };
            r.rule("", &dsel, margin);
            match s(m, "text") {
                "" => r#"<div class="vb-divider" role="separator"></div>"#.into(),
                t => format!(r#"<div class="vb-divider vb-divider-text" role="separator"><span>{}</span></div>"#, text(t)),
            }
        }
        "anchor" => format!(r#"<span class="vb-anchor" id="{}"></span>"#, esc(s(m, "id"))),
        "inner" => {
            let row_sel = format!("{sel} > .vb-row");
            r.resp(m, "gap", &row_sel, |v| format!("--vb-gap:{};", px(v)));
            let row = super::render::row(r, w, &row_sel).replacen("class=\"vb-row ", "class=\"vb-row vb-inner ", 1);
            let overlay = super::elementor::style::overlay(r, w, sel);
            format!("{overlay}{row}")
        }
        "html" => s(m, "code").to_string(),
        "elementor-widget" => empty(&format!("Elementor: {}", s(m, "label"))),
        "video" => {
            let url = s(m, "url");
            if url.is_empty() {
                return empty("Video");
            }
            if let Some(rad) = n(m, "radius") {
                r.rule("", &format!("{sel} .vb-video"), &format!("border-radius:{rad}px;"));
            }
            let ratio = match s(m, "ratio") { "" => "16/9", x => x };
            let controls = s(m, "controls") != "off";
            let body = match embed_src(url, b(m, "autoplay") || m["poster"].is_string(), b(m, "autoplay"), b(m, "loop"), controls, n(m, "start").unwrap_or(0)) {
                Some(src) => {
                    if let (Some(p), false) = (m["poster"].as_str(), b(m, "autoplay")) {
                        r.uses_runtime = true;
                        format!(
                            r#"<button class="vb-facade" type="button" data-vb-src="{src}" aria-label="Play video" style="background-image:url('{}')"><span class="vb-play">{}</span></button>"#,
                            esc(&css_url(&href(p))),
                            icon_svg("play").unwrap_or_default()
                        )
                    } else {
                        format!(r#"<iframe src="{src}" title="Video" loading="lazy" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture" allowfullscreen></iframe>"#)
                    }
                }
                None => format!(
                    r#"<video src="{}"{}{}{}{} playsinline preload="metadata"{}></video>"#,
                    href(url),
                    if controls { " controls" } else { "" },
                    if b(m, "autoplay") { " autoplay muted" } else { "" },
                    if b(m, "loop") { " loop" } else { "" },
                    "",
                    m["poster"].as_str().map(|p| format!(r#" poster="{}""#, href(p))).unwrap_or_default(),
                ),
            };
            format!(r#"<div class="vb-video" style="aspect-ratio:{ratio}">{body}</div>"#)
        }
        "audio" => {
            let url = s(m, "url");
            if url.is_empty() {
                return empty("Audio");
            }
            let title = match s(m, "title") { "" => String::new(), t => format!(r#"<p class="vb-audio-title">{}</p>"#, text(t)) };
            if url.starts_with("https://soundcloud.com/") {
                format!(r#"{title}<iframe class="vb-audio-embed" height="166" src="https://w.soundcloud.com/player/?url={}&amp;color=%231f4e79" title="SoundCloud" loading="lazy" allow="autoplay"></iframe>"#, percent(url))
            } else if let Some(rest) = url.strip_prefix("https://open.spotify.com/") {
                let path: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '/').collect();
                format!(r#"{title}<iframe class="vb-audio-embed" height="152" src="https://open.spotify.com/embed/{path}" title="Spotify" loading="lazy" allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture"></iframe>"#)
            } else {
                format!(r#"{title}<audio controls preload="metadata" src="{}"></audio>"#, href(url))
            }
        }
        "gallery" => {
            let gsel = format!("{sel} .vb-gallery");
            r.rule("", &gsel, &format!("--vb-gc:{};--vb-gg:{}px;", n(m, "columns").unwrap_or(3), n(m, "gap").unwrap_or(12)));
            r.resp(m, "columns", &gsel, |v| v.as_i64().map(|c| format!("--vb-gc:{c};")).unwrap_or_default());
            r.resp(m, "gap", &gsel, |v| v.as_i64().map(|g| format!("--vb-gg:{g}px;")).unwrap_or_default());
            let mut d = String::new();
            if let Some(rad) = n(m, "radius") {
                d.push_str(&format!("border-radius:{rad}px;"));
            }
            match s(m, "ratio") {
                "" | "auto" => {}
                ratio => d.push_str(&format!("aspect-ratio:{ratio};")),
            }
            r.rule("", &format!("{gsel} img"), &d);
            let lb = b(m, "lightbox");
            if lb {
                r.uses_runtime = true;
            }
            let caps = b(m, "captions");
            let imgs: String = m["images"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["src"].as_str().map(|src| (src, i)))
                .map(|(src, i)| {
                    let img = format!(r#"<img src="{}" alt="{}" loading="lazy" decoding="async">"#, href(src), esc(s(i, "alt")));
                    let cap = if caps && !s(i, "caption").is_empty() { format!("<figcaption>{}</figcaption>", text(s(i, "caption"))) } else { String::new() };
                    let link = if lb {
                        format!(r#"<a href="{}" data-vb-lightbox="{}" data-caption="{}">{img}</a>"#, href(src), esc(id), esc(s(i, "caption")))
                    } else if let Some(l) = i["link"].as_str() {
                        format!("{}{img}</a>", link_open(l, false, "", ""))
                    } else {
                        img
                    };
                    format!("<figure>{link}{cap}</figure>")
                })
                .collect();
            if imgs.is_empty() {
                return empty("Gallery");
            }
            format!(
                r#"<div class="vb-gallery vb-gl-{} vb-gh-{}">{imgs}</div>"#,
                match s(m, "layout") { "" => "grid", x => x },
                match s(m, "hover") { "" => "none", x => x }
            )
        }
        "carousel" => {
            let attrs = carousel_attrs(m, r, &format!("{sel} .vb-carousel"), 3);
            let mut d = String::new();
            if let Some(rad) = n(m, "radius") {
                d.push_str(&format!("border-radius:{rad}px;"));
            }
            match s(m, "ratio") {
                "" | "auto" => {}
                ratio => d.push_str(&format!("aspect-ratio:{ratio};")),
            }
            r.rule("", &format!("{sel} .vb-slide img"), &d);
            let lb = b(m, "lightbox");
            let slides: String = m["images"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["src"].as_str().map(|src| (src, i)))
                .map(|(src, i)| {
                    let img = format!(r#"<img src="{}" alt="{}" loading="lazy" decoding="async">"#, href(src), esc(s(i, "alt")));
                    let inner = if lb {
                        format!(r#"<a href="{}" data-vb-lightbox="{}" data-caption="{}">{img}</a>"#, href(src), esc(id), esc(s(i, "caption")))
                    } else if let Some(l) = i["link"].as_str() {
                        format!("{}{img}</a>", link_open(l, false, "", ""))
                    } else {
                        img
                    };
                    let cap = if b(m, "captions") && !s(i, "caption").is_empty() { format!("<figcaption>{}</figcaption>", text(s(i, "caption"))) } else { String::new() };
                    format!(r#"<figure class="vb-slide">{inner}{cap}</figure>"#)
                })
                .collect();
            if slides.is_empty() {
                return empty("Carousel");
            }
            format!(r#"<div class="vb-carousel"{attrs}><div class="vb-track">{slides}</div></div>"#)
        }
        "slides" => {
            if let Some(columns) = w["cols"].as_array().filter(|cols| !cols.is_empty()) {
                let attrs = carousel_attrs(m, r, &format!("{sel} .vb-carousel"), 1);
                let slides: String = columns.iter().map(|col| {
                    format!(r#"<div class="vb-slide">{}</div>"#, super::render::column(r, col))
                }).collect();
                return format!(r#"<div class="vb-carousel vb-linked-carousel"{attrs}><div class="vb-track">{slides}</div></div>"#);
            }
            let csel = format!("{sel} .vb-carousel");
            let attrs = carousel_attrs(m, r, &csel, 1);
            r.rule("", &csel, "--vb-pv:1;--vb-cg:0px;");
            r.resp(m, "height", &format!("{sel} .vb-slide"), |v| format!("min-height:{};", px(v)));
            if let Some(rad) = n(m, "radius") {
                r.rule("", &csel, &format!("border-radius:{rad}px;overflow:hidden;"));
            }
            if let Some(c) = colour(m, "text_colour") {
                r.rule("", &format!("{sel} .vb-slide"), &format!("color:{c};"));
            }
            let overlay = colour(m, "overlay").map(|c| format!(r#"<span class="vb-overlay" style="background:{};opacity:{}"></span>"#, esc(&c), n(m, "overlay_opacity").unwrap_or(40) as f64 / 100.0)).unwrap_or_default();
            let kb = b(m, "kenburns");
            let slides: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let mut st = String::new();
                    if let Some(c) = colour(i, "colour") {
                        st.push_str(&format!("background-color:{c};"));
                    }
                    let img = i["image"].as_str().map(|u| format!(r#"<span class="vb-slide-img{}" style="background-image:url('{}')"></span>"#, if kb { " vb-kenburns" } else { "" }, esc(&css_url(&href(u))))).unwrap_or_default();
                    let btn = match s(i, "button") {
                        "" => String::new(),
                        t => format!("{}{}</a>", link_open(i["href"].as_str().unwrap_or("#"), false, "vb-btn vb-btn-filled vb-btn-l", ""), text(t)),
                    };
                    format!(
                        r#"<div class="vb-slide vb-slide-hero vb-al-{al}" style="{st}">{img}{overlay}<div class="vb-slide-body"><h2 class="vb-h">{h}</h2>{p}{btn}</div></div>"#,
                        al = match s(i, "align") { "" => "center", a => a },
                        st = esc(&st),
                        h = text(s(i, "heading")),
                        p = paragraphs(s(i, "text")),
                    )
                })
                .collect();
            format!(r#"<div class="vb-carousel vb-slides"{attrs}><div class="vb-track">{slides}</div></div>"#)
        }
        "playlist" => {
            r.uses_runtime = true;
            let items: Vec<&Value> = m["items"].as_array().into_iter().flatten().filter(|i| i["url"].is_string()).collect();
            let Some(first) = items.first() else { return empty("Playlist") };
            let src = |u: &str| embed_src(u, false, false, false, true, 0).unwrap_or_else(|| href(u));
            let list: String = items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    format!(
                        r#"<button type="button" class="vb-pl-item{on}" data-vb-src="{}"><span class="vb-pl-n">{}</span><span class="vb-pl-t">{}</span><span class="vb-pl-d">{}</span></button>"#,
                        src(s(it, "url")).replace("?rel=0", "?autoplay=1&amp;rel=0"),
                        i + 1,
                        text(s(it, "title")),
                        esc(s(it, "duration")),
                        on = if i == 0 { " on" } else { "" },
                    )
                })
                .collect();
            let first_src = src(s(first, "url"));
            let player = if embed_src(s(first, "url"), false, false, false, true, 0).is_some() {
                format!(r#"<iframe src="{first_src}" title="Video" allow="autoplay; encrypted-media; picture-in-picture" allowfullscreen></iframe>"#)
            } else {
                format!(r#"<video src="{first_src}" controls playsinline></video>"#)
            };
            format!(
                r#"<div class="vb-playlist vb-pl-{lay}"><div class="vb-video vb-pl-player" style="aspect-ratio:16/9">{player}</div><div class="vb-pl-list">{title}{list}</div></div>"#,
                lay = if s(m, "layout") == "below" { "below" } else { "side" },
                title = match s(m, "title") { "" => String::new(), t => format!(r#"<p class="vb-pl-title">{}</p>"#, text(t)) },
            )
        }
        "lottie" => {
            let Some(u) = m["url"].as_str() else { return empty("Lottie") };
            r.uses_runtime = true;
            r.resp(m, "width", &format!("{sel} .vb-lottie"), |v| v.as_i64().map(|w| format!("width:{w}%;")).unwrap_or_default());
            format!(
                r#"<div class="vb-lottie-wrap vb-al-{al}"><div class="vb-lottie" data-vb-lottie="{}" data-trigger="{}" data-loop="{}" data-speed="{}"></div></div>"#,
                href(u),
                match s(m, "trigger") { "" => "view", t => t },
                b(m, "loop"),
                n(m, "speed").unwrap_or(10) as f64 / 10.0,
                al = match s(m, "align") { "" => "center", a => a },
            )
        }
        "map" => {
            let addr = s(m, "address");
            if addr.trim().is_empty() {
                return empty("Map");
            }
            let msel = format!("{sel} .vb-map");
            r.resp(m, "height", &msel, |v| format!("height:{};", px(v)));
            if let Some(rad) = n(m, "radius") {
                r.rule("", &msel, &format!("border-radius:{rad}px;"));
            }
            format!(
                r#"<div class="vb-map{g}" style="height:360px"><iframe src="https://maps.google.com/maps?q={q}&amp;z={z}&amp;output=embed" title="{t}" loading="lazy" referrerpolicy="no-referrer-when-downgrade"></iframe></div>"#,
                g = if b(m, "grayscale") { " vb-gray" } else { "" },
                q = percent(addr),
                z = n(m, "zoom").unwrap_or(14),
                t = esc(addr),
            )
        }
        "embed" => {
            let Some(u) = m["url"].as_str().filter(|u| u.starts_with("https://")) else { return empty("Embed") };
            let esel = format!("{sel} iframe");
            r.resp(m, "height", &esel, |v| format!("height:{};", px(v)));
            if let Some(rad) = n(m, "radius") {
                r.rule("", &esel, &format!("border-radius:{rad}px;"));
            }
            format!(
                r#"<iframe class="vb-embed" src="{}" title="{}" loading="lazy" style="height:420px" allow="clipboard-write; encrypted-media; fullscreen" referrerpolicy="strict-origin-when-cross-origin"></iframe>"#,
                esc(u),
                esc(match s(m, "title") { "" => "Embedded content", t => t })
            )
        }
        "hotspot" => {
            let Some(src) = m["src"].as_str() else { return empty("Hotspots") };
            if let Some(c) = colour(m, "colour") {
                r.rule("", sel, &format!("--vb-hs:{c};"));
            }
            if s(m, "trigger") == "click" {
                r.uses_runtime = true;
            }
            let spots: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let tip = format!("<b>{}</b>{}", text(s(i, "title")), paragraphs(s(i, "text")));
                    let tip = match i["link"].as_str() {
                        Some(l) => format!("{tip}{}More →</a>", link_open(l, false, "vb-hs-link", "")),
                        None => tip,
                    };
                    format!(
                        r#"<div class="vb-spot" style="left:{}%;top:{}%"><button type="button" class="vb-spot-dot" aria-label="{}"></button><div class="vb-spot-tip">{tip}</div></div>"#,
                        n(i, "x").unwrap_or(50),
                        n(i, "y").unwrap_or(50),
                        esc(s(i, "title")),
                    )
                })
                .collect();
            format!(
                r#"<div class="vb-hotspots{p}{c}"><img src="{}" alt="" loading="lazy">{spots}</div>"#,
                href(src),
                p = if b(m, "pulse") { " vb-pulse" } else { "" },
                c = if s(m, "trigger") == "click" { " vb-hs-click" } else { "" },
            )
        }
        "testimonial" => {
            let st = n(m, "stars").unwrap_or(0);
            let stars_html = if st > 0 { format!(r#"<div class="vb-stars" aria-label="{st} out of 5">{}</div>"#, icon_svg("star").unwrap_or_default().repeat(st as usize)) } else { String::new() };
            let avatar = m["avatar"].as_str().map(|a| format!(r#"<img src="{}" alt="" loading="lazy">"#, href(a))).unwrap_or_default();
            let role = match s(m, "role") {
                "" => String::new(),
                x => format!("<small>{}</small>", text(x)),
            };
            format!(
                r#"<figure class="vb-quote{al}">{stars_html}<blockquote>{q}</blockquote><figcaption>{avatar}<span><b>{name}</b>{role}</span></figcaption></figure>"#,
                al = align_class(m),
                q = text(s(m, "quote")),
                name = text(s(m, "name")),
            )
        }
        "testimonials" => {
            let csel = format!("{sel} .vb-carousel");
            let attrs = carousel_attrs(m, r, &csel, 2);
            if let Some(c) = colour(m, "card_bg") {
                r.rule("", &format!("{sel} .vb-tcard"), &format!("background:{c};"));
            }
            let skin = match s(m, "skin") { "" => "card", x => x };
            let slides: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let st = n(i, "stars").unwrap_or(0);
                    let stars_html = if st > 0 { format!(r#"<div class="vb-stars">{}</div>"#, icon_svg("star").unwrap_or_default().repeat(st as usize)) } else { String::new() };
                    let avatar = i["avatar"].as_str().map(|a| format!(r#"<img src="{}" alt="" loading="lazy">"#, href(a))).unwrap_or_default();
                    format!(
                        r#"<figure class="vb-slide vb-quote vb-tcard vb-tskin-{skin}{al}">{stars_html}<blockquote>{q}</blockquote><figcaption>{avatar}<span><b>{name}</b><small>{role}</small></span></figcaption></figure>"#,
                        al = align_class(m),
                        q = text(s(i, "quote")),
                        name = text(s(i, "name")),
                        role = text(s(i, "role")),
                    )
                })
                .collect();
            format!(r#"<div class="vb-carousel"{attrs}><div class="vb-track">{slides}</div></div>"#)
        }
        "accordion" => {
            let open = b(m, "open_first");
            let group = if b(m, "single") { format!(r#" name="acc-{}""#, esc(id)) } else { String::new() };
            if let Some(c) = colour(m, "title_colour") {
                r.rule("", &format!("{sel} summary"), &format!("color:{c};"));
            }
            if let Some(c) = colour(m, "active_colour") {
                r.rule("", &format!("{sel} details[open] > summary"), &format!("color:{c};"));
            }
            let items: Vec<&Value> = m["items"].as_array().into_iter().flatten().collect();
            let html: String = items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    format!(
                        r#"<details class="vb-acc-i"{group}{o}><summary>{}<span class="vb-acc-x" aria-hidden="true"></span></summary><div class="vb-acc-a">{}</div></details>"#,
                        text(s(it, "q")),
                        panel_content(r, w, i, s(it, "a")),
                        o = if (open && i == 0) || b(it, "open") { " open" } else { "" },
                    )
                })
                .collect();
            let schema = if b(m, "schema") && !r.ctx.editing {
                let qa: Vec<Value> = items.iter().map(|it| serde_json::json!({"@type": "Question", "name": s(it, "q"), "acceptedAnswer": {"@type": "Answer", "text": s(it, "a")}})).collect();
                format!(r#"<script type="application/ld+json">{}</script>"#, serde_json::json!({"@context": "https://schema.org", "@type": "FAQPage", "mainEntity": qa}).to_string().replace("</", "<\\/"))
            } else {
                String::new()
            };
            format!(
                r#"<div class="vb-acc vb-acc-{skin} vb-acc-ic-{ic}">{html}</div>{schema}"#,
                skin = match s(m, "skin") { "" => "lines", x => x },
                ic = match s(m, "icon") { "" => "plus", x => x },
            )
        }
        "tabs" => {
            r.uses_runtime = true;
            if let Some(c) = colour(m, "active_colour") {
                r.rule("", sel, &format!("--vb-tab:{c};"));
            }
            let items: Vec<&Value> = m["items"].as_array().into_iter().flatten().collect();
            let buttons: String = items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let ic = match s(it, "icon") { "" => String::new(), x => format!("<i>{}</i>", icon_or_text(x)) };
                    format!(
                        r#"<button type="button" role="tab" class="vb-tab-btn{on}" aria-selected="{sel_}" id="t-{id}-{i}" aria-controls="p-{id}-{i}">{ic}{}</button>"#,
                        text(s(it, "title")),
                        on = if i == 0 { " on" } else { "" },
                        sel_ = i == 0,
                        id = esc(id),
                    )
                })
                .collect();
            let panels: String = items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    format!(
                        r#"<div role="tabpanel" class="vb-tab-panel{on}" id="p-{id}-{i}" aria-labelledby="t-{id}-{i}"><p class="vb-tab-nojs">{}</p>{}</div>"#,
                        text(s(it, "title")),
                        panel_content(r, w, i, s(it, "text")),
                        on = if i == 0 { " on" } else { "" },
                        id = esc(id),
                    )
                })
                .collect();
            format!(
                r#"<div class="vb-tabs vb-tabs-{lay} vb-tabs-{skin} vb-tabs-al-{al}" data-vb-tabs><div class="vb-tab-list" role="tablist">{buttons}</div><div class="vb-tab-panels">{panels}</div></div>"#,
                lay = if s(m, "layout") == "left" { "left" } else { "top" },
                skin = match s(m, "skin") { "" => "underline", x => x },
                al = match s(m, "align") { "" => "left", x => x },
            )
        }
        "stat" => {
            let nsel = format!("{sel} .vb-stat-n");
            if let Some(c) = colour(m, "colour") {
                r.rule("", &nsel, &format!("color:{c};"));
            }
            r.resp(m, "size", &nsel, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:min({n}px,calc({n}px * .55 + 2.2vw));")).unwrap_or_default());
            align_rules(r, m, &format!("{sel} .vb-stat"));
            let number = s(m, "number");
            let count = if b(m, "animate") && number.trim().parse::<f64>().is_ok() {
                r.uses_runtime = true;
                format!(r#" data-vb-count="{}" data-duration="{}" data-sep="{}""#, esc(number.trim()), n(m, "duration").unwrap_or(1600), b(m, "separator"))
            } else {
                String::new()
            };
            format!(
                r#"<div class="vb-stat"><span class="vb-stat-n">{p}<span class="vb-num"{count}>{num}</span>{sfx}</span><span class="vb-stat-l">{label}</span></div>"#,
                p = esc(s(m, "prefix")),
                num = esc(number),
                sfx = esc(s(m, "suffix")),
                label = text(s(m, "label")),
            )
        }
        "progress" => {
            r.uses_runtime = true;
            let psel = format!("{sel} .vb-bar");
            let mut d = format!("height:{}px;", n(m, "height").unwrap_or(10));
            if let Some(c) = colour(m, "track") {
                d.push_str(&format!("background:{c};"));
            }
            r.rule("", &psel, &d);
            if let Some(c) = colour(m, "colour") {
                r.rule("", &format!("{psel} i"), &format!("background:{c};"));
            }
            let bars: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let v = n(i, "value").unwrap_or(0);
                    let c = colour(i, "colour").map(|c| format!("background:{c};")).unwrap_or_default();
                    format!(
                        r#"<div class="vb-prog"><div class="vb-prog-head"><span>{}</span>{}</div><div class="vb-bar{st}" role="progressbar" aria-valuenow="{v}" aria-valuemin="0" aria-valuemax="100"><i style="--vb-v:{v}%;{}"></i></div></div>"#,
                        text(s(i, "label")),
                        if b(m, "show_value") { format!("<b>{v}%</b>") } else { String::new() },
                        esc(&c),
                        st = if b(m, "striped") { " vb-striped" } else { "" },
                    )
                })
                .collect();
            format!(r#"<div class="vb-progs">{bars}</div>"#)
        }
        "rating" => {
            let max: i64 = if s(m, "max") == "10" { 10 } else { 5 };
            let val = n(m, "value").unwrap_or(45) as f64 / 10.0;
            let rsel = format!("{sel} .vb-rating");
            if let Some(c) = colour(m, "colour") {
                r.rule("", &rsel, &format!("--vb-star:{c};"));
            }
            r.resp(m, "size", &rsel, |v| v.as_i64().map(|z| format!("font-size:{z}px;")).unwrap_or_default());
            align_rules(r, m, &rsel);
            let schema = if b(m, "schema") && !r.ctx.editing {
                format!(r#"<script type="application/ld+json">{}</script>"#, serde_json::json!({"@context": "https://schema.org", "@type": "AggregateRating", "ratingValue": val, "bestRating": max, "itemReviewed": {"@type": "Thing", "name": s(m, "label")}}).to_string().replace("</", "<\\/"))
            } else {
                String::new()
            };
            format!(
                r#"<div class="vb-rating" role="img" aria-label="{val} out of {max}">{}{label}</div>{schema}"#,
                stars(val, max, if s(m, "icon") == "heart" { "heart" } else { "star" }),
                label = match s(m, "label") { "" => String::new(), l => format!(r#"<span class="vb-rating-l">{}</span>"#, text(l)) },
            )
        }
        "alert" => {
            if b(m, "dismiss") {
                r.uses_runtime = true;
            }
            let kind = match s(m, "kind") { "" => "info", k => k };
            let icon = match kind {
                "success" => "check",
                "warning" | "danger" => "warning",
                _ => "info",
            };
            format!(
                r#"<div class="vb-alert vb-alert-{kind}" role="{role}"><i class="vb-alert-ic">{ic}</i><div>{t}{p}</div>{x}</div>"#,
                role = if kind == "danger" { "alert" } else { "status" },
                ic = icon_svg(icon).unwrap_or_default(),
                t = match s(m, "title") { "" => String::new(), t => format!("<b>{}</b>", text(t)) },
                p = paragraphs(s(m, "text")),
                x = if b(m, "dismiss") { format!(r#"<button type="button" class="vb-alert-x" aria-label="Dismiss">{}</button>"#, icon_svg("x").unwrap_or_default()) } else { String::new() },
            )
        }
        "pricing" => {
            if let Some(c) = colour(m, "accent") {
                r.rule("", sel, &format!("--vb-c1:{c};"));
            }
            let items: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| format!(r#"<li><i class="vb-li-ic">{}</i><span>{}</span></li>"#, icon_svg("check").unwrap_or_default(), text(s(i, "text"))))
                .collect();
            let button = match s(m, "button") {
                "" => String::new(),
                t => format!(
                    "{}{}</a>",
                    link_open(m["href"].as_str().unwrap_or("#"), false, &format!("vb-btn vb-btn-m {}", if b(m, "featured") { "vb-btn-filled" } else { "vb-btn-outline" }), ""),
                    text(t)
                ),
            };
            format!(
                r#"<div class="vb-price{feat}">{badge}<h3>{title}</h3><div class="vb-price-amt">{old}<b>{price}</b><span>{period}</span></div>{note}<ul class="vb-list vb-list-check">{items}</ul>{button}{foot}</div>"#,
                feat = if b(m, "featured") { " vb-price-feat" } else { "" },
                badge = match s(m, "badge") { "" => String::new(), t => format!(r#"<span class="vb-badge">{}</span>"#, text(t)) },
                title = text(s(m, "title")),
                old = match s(m, "old_price") { "" => String::new(), t => format!("<s>{}</s> ", esc(t)) },
                price = esc(s(m, "price")),
                period = esc(s(m, "period")),
                note = match s(m, "note") { "" => String::new(), t => format!(r#"<p class="vb-price-note">{}</p>"#, text(t)) },
                foot = match s(m, "footer") { "" => String::new(), t => format!(r#"<p class="vb-price-foot">{}</p>"#, text(t)) },
            )
        }
        "price-list" => {
            if let Some(c) = colour(m, "price_colour") {
                r.rule("", &format!("{sel} .vb-pl-price"), &format!("color:{c};"));
            }
            let items: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let img = i["image"].as_str().map(|u| format!(r#"<img src="{}" alt="" loading="lazy">"#, href(u))).unwrap_or_default();
                    let title = match i["link"].as_str() {
                        Some(l) => format!("{}{}</a>", link_open(l, false, "", ""), text(s(i, "title"))),
                        None => text(s(i, "title")),
                    };
                    format!(
                        r#"<li class="vb-pli">{img}<div><div class="vb-pl-head"><b>{title}</b><span class="vb-pl-lead"></span><b class="vb-pl-price">{}</b></div>{}</div></li>"#,
                        esc(s(i, "price")),
                        match s(i, "desc") { "" => String::new(), d => format!("<p>{}</p>", text(d)) },
                    )
                })
                .collect();
            format!(r#"<ul class="vb-pricelist vb-lead-{}">{items}</ul>"#, match s(m, "leader") { "" => "dotted", x => x })
        }
        "flipbox" => {
            let fsel = format!("{sel} .vb-flip");
            r.resp(m, "height", &fsel, |v| format!("height:{};", px(v)));
            let mut d = String::new();
            if let Some(c) = colour(m, "text_colour") {
                d.push_str(&format!("color:{c};"));
            }
            if let Some(rad) = n(m, "radius") {
                d.push_str(&format!("--vb-fr:{rad}px;"));
            }
            r.rule("", &fsel, &d);
            let bg = |k: &str| colour(m, k).map(|c| format!("background-color:{c};")).unwrap_or_default();
            let front_img = m["front_image"].as_str().map(|u| format!("background-image:url('{}');background-size:cover;background-position:center;", css_url(&href(u)))).unwrap_or_default();
            let btn = match s(m, "back_button") {
                "" => String::new(),
                t => format!("{}{}</a>", link_open(m["back_href"].as_str().unwrap_or("#"), false, "vb-btn vb-btn-outline vb-btn-m", r#" style="--vb-bc:currentColor;--vb-bt:currentColor""#), text(t)),
            };
            format!(
                r#"<div class="vb-flip vb-flip-{fx} vb-flip-{dir}" tabindex="0"><div class="vb-flip-in"><div class="vb-flip-front" style="{fbg}"><span class="vb-flip-ic">{ic}</span><h3>{ft}</h3>{fp}</div><div class="vb-flip-back" style="{bbg}"><h3>{bt}</h3>{bp}{btn}</div></div></div>"#,
                fx = match s(m, "effect") { "" => "flip", x => x },
                dir = match s(m, "direction") { "" => "left", x => x },
                fbg = esc(&format!("{}{front_img}", bg("front_bg"))),
                bbg = esc(&bg("back_bg")),
                ic = match s(m, "front_icon") { "" => String::new(), i => icon_or_text(i) },
                ft = text(s(m, "front_title")),
                fp = paragraphs(s(m, "front_text")),
                bt = text(s(m, "back_title")),
                bp = paragraphs(s(m, "back_text")),
            )
        }
        "cta" => {
            let csel = format!("{sel} .vb-cta");
            r.resp(m, "height", &csel, |v| format!("min-height:{};", px(v)));
            let mut d = String::new();
            if let Some(c) = colour(m, "text_colour") {
                d.push_str(&format!("color:{c};"));
            }
            if let Some(rad) = n(m, "radius") {
                d.push_str(&format!("border-radius:{rad}px;"));
            }
            if let Some(c) = colour(m, "bg") {
                d.push_str(&format!("--vb-cta-bg:{c};"));
            }
            r.rule("", &csel, &d);
            let img = m["image"].as_str().map(|u| format!(r#"<div class="vb-cta-img" style="background-image:url('{}')"></div>"#, esc(&css_url(&href(u))))).unwrap_or_default();
            let overlay = colour(m, "overlay").map(|c| format!(r#"<span class="vb-overlay" style="background:{};opacity:{}"></span>"#, esc(&c), n(m, "overlay_opacity").unwrap_or(50) as f64 / 100.0)).unwrap_or_default();
            let btn = match s(m, "button") {
                "" => String::new(),
                t => format!("{}{}</a>", link_open(m["href"].as_str().unwrap_or("#"), false, "vb-btn vb-btn-filled vb-btn-l", ""), text(t)),
            };
            format!(
                r#"<div class="vb-cta vb-cta-{lay} vb-cta-h-{hv} vb-al-{al}">{ribbon}<div class="vb-cta-media">{img}{ov}</div><div class="vb-cta-body"><h3 class="vb-h">{t}</h3>{p}{btn}</div></div>"#,
                lay = match s(m, "layout") { "" => "cover", x => x },
                hv = match s(m, "hover") { "" => "zoom", x => x },
                al = match s(m, "align") { "" => "center", x => x },
                ribbon = match s(m, "ribbon") { "" => String::new(), t => format!(r#"<span class="vb-ribbon">{}</span>"#, text(t)) },
                ov = if s(m, "layout").is_empty() || s(m, "layout") == "cover" { overlay } else { String::new() },
                t = text(s(m, "title")),
                p = paragraphs(s(m, "text")),
            )
        }
        "animated-heading" => {
            let tag = match s(m, "tag") { "" => "h2", t => t };
            let hs = format!("{sel} .vb-ah");
            align_rules(r, m, &hs);
            r.resp(m, "size", &hs, |v| v.as_i64().filter(|n| *n > 0).map(|n| format!("font-size:min({n}px,calc({n}px * .55 + 2.2vw));")).unwrap_or_default());
            if let Some(c) = colour(m, "colour") {
                r.rule("", &hs, &format!("color:{c};"));
            }
            if let Some(c) = colour(m, "word_colour") {
                r.rule("", &format!("{hs} .vb-aw"), &format!("color:{c};"));
            }
            if let Some(c) = colour(m, "shape_colour") {
                r.rule("", &hs, &format!("--vb-shape:{c};"));
            }
            let words: Vec<&str> = s(m, "words").lines().map(str::trim).filter(|w| !w.is_empty()).collect();
            let style = match s(m, "style") { "" => "rotate", x => x };
            let inner = if style == "highlight" {
                let path = match s(m, "shape") {
                    "circle" => r#"<path d="M325,18C228.7-0.6,31.2-1.6,23.1,25.8c-14.1,47.8,180.2,60.7,280.9,47.2c61-8.2,180-46.3,93-63.7c-83.6-16.7-222.5,5-273.3,14.2"/>"#,
                    "curly" => r#"<path d="M3,146.1c17.1-8.8,33.5-17.8,51.4-17.8c15.6,0,17.1,18.1,30.2,18.1c22.9,0,36-18.6,53.9-18.6 c17.1,0,21.3,18.5,37.5,18.5c21.3,0,31.8-18.6,49-18.6c22.1,0,18.8,18.8,36.8,18.8c18.8,0,37.5-18.6,49-18.6c20.4,0,17.1,19,36.8,19 c22.9,0,36.8-20.6,54.7-18.6c17.7,1.4,7.1,19.5,33.5,18.8c17.1,0,47.2-6.5,61.1-15.6"/>"#,
                    "double" => r#"<path d="M8.4,143.1c14.2-8,97.6-8.8,200.6-9.2c122.3-0.4,287.5,7.2,287.5,7.2"/><path d="M8,19.4c72.3-5.3,162-7.8,216-7.8c54,0,136.2,0,267,7.8"/>"#,
                    "strike" => r#"<path d="M3,75h493.5"/>"#,
                    "marker" => r#"<path d="M3,120 C150,110 350,130 497,118" stroke-width="40" opacity=".35"/>"#,
                    "x" => r#"<path d="M497.4,23.9C301.6,40,155.9,80.6,4,144.4"/><path d="M14.1,27.6c204.5,20.3,393.8,74,467.3,111.7"/>"#,
                    _ => r#"<path d="M7.7,145.6C109,125,299.9,116.2,401,121.3c42.1,2.2,87.6,11.8,87.3,25.7"/>"#,
                };
                format!(
                    r#"<span class="vb-aw-hl"><span class="vb-aw">{}</span><svg viewBox="0 0 500 150" preserveAspectRatio="none" aria-hidden="true">{path}</svg></span>"#,
                    text(words.first().copied().unwrap_or(""))
                )
            } else {
                r.uses_runtime = true;
                let ws: String = words.iter().enumerate().map(|(i, w)| format!(r#"<span class="vb-aw{}">{}</span>"#, if i == 0 { " on" } else { "" }, text(w))).collect();
                format!(r#"<span class="vb-aw-rot" data-vb-rotate="{style}" data-speed="{}">{ws}</span>"#, n(m, "speed").unwrap_or(3) * 1000)
            };
            let body = format!(
                r#"{}{inner}{}"#,
                match s(m, "before") { "" => String::new(), t => format!("{} ", text(t)) },
                match s(m, "after") { "" => String::new(), t => format!(" {}", text(t)) },
            );
            let body = match m["link"].as_str() {
                Some(l) => format!("{}{body}</a>", link_open(l, false, "vb-hlink", "")),
                None => body,
            };
            format!(r#"<{tag} class="vb-h vb-ah vb-ah-{style}">{body}</{tag}>"#)
        }
        "countdown" => {
            r.uses_runtime = true;
            let mut d = String::new();
            if let Some(c) = colour(m, "box_bg") {
                d.push_str(&format!("--vb-cd-bg:{c};"));
            }
            if let Some(c) = colour(m, "digit_colour") {
                d.push_str(&format!("--vb-cd-fg:{c};"));
            }
            r.rule("", sel, &d);
            let parts: Vec<(&str, &str)> = if s(m, "show_days") == "off" { vec![("h", "Hours"), ("m", "Minutes"), ("s", "Seconds")] } else { vec![("d", "Days"), ("h", "Hours"), ("m", "Minutes"), ("s", "Seconds")] };
            let labels = m["labels"].as_bool().unwrap_or(true);
            let boxes: String = parts.iter().map(|(k, l)| format!(r#"<span class="vb-cd-u"><b data-u="{k}">00</b>{}</span>"#, if labels { format!("<small>{l}</small>") } else { String::new() })).collect();
            format!(
                r#"<div class="vb-countdown vb-cd-{style} vb-al-{al}" data-vb-countdown data-until="{until}" data-evergreen="{ever}" data-id="{id}" data-expired-href="{eh}"><div class="vb-cd-units">{boxes}</div><p class="vb-cd-done" hidden>{done}</p></div>"#,
                style = match s(m, "style") { "" => "boxes", x => x },
                al = match s(m, "align") { "" => "center", x => x },
                until = esc(s(m, "until")),
                ever = if s(m, "mode") == "evergreen" { n(m, "hours").unwrap_or(24) } else { 0 },
                id = esc(id),
                eh = m["expired_href"].as_str().map(href).unwrap_or_default(),
                done = text(s(m, "expired")),
            )
        }
        "toc" => {
            r.uses_runtime = true;
            if let Some(c) = colour(m, "active_colour") {
                r.rule("", sel, &format!("--vb-toc:{c};"));
            }
            let levels = match s(m, "levels") {
                "h2" => "h2",
                "h2h4" => "h2,h3,h4",
                "h1h6" => "h1,h2,h3,h4,h5,h6",
                _ => "h2,h3",
            };
            let title = match s(m, "title") { "" => "Contents", t => t };
            let head = if b(m, "collapsible") {
                format!(r#"<details class="vb-toc-d"{}><summary class="vb-toc-title">{}</summary><nav class="vb-toc-nav"></nav></details>"#, if b(m, "collapsed") { "" } else { " open" }, text(title))
            } else {
                format!(r#"<p class="vb-toc-title">{}</p><nav class="vb-toc-nav"></nav>"#, text(title))
            };
            format!(r#"<div class="vb-toc{box_}{num}" data-vb-toc="{levels}">{head}</div>"#, box_ = if b(m, "box") { " vb-toc-box" } else { "" }, num = if b(m, "numbered") { " vb-toc-num" } else { "" })
        }
        "share" => {
            r.uses_runtime = true;
            let colours = match s(m, "colours") { "" => "brand", x => x };
            if let Some(c) = colour(m, "colour") {
                r.rule("", sel, &format!("--vb-sh:{c};"));
            }
            r.resp(m, "align", &format!("{sel} .vb-share"), |v| v.as_str().map(|a| format!("justify-content:{};", match a { "center" => "center", "right" => "flex-end", "justify" => "stretch", _ => "flex-start" })).unwrap_or_default());
            let view = match s(m, "view") { "" => "both", x => x };
            let items: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let net = s(i, "net");
                    let label = super::spec::SHARE_NETS.iter().find(|(k, _)| *k == net).map(|x| x.1).unwrap_or(net);
                    format!(
                        r##"<a class="vb-share-b" href="#" data-vb-share="{net}" style="--vb-brand:{br}" rel="noopener nofollow" aria-label="Share on {label}">{ic}{lb}</a>"##,
                        net = esc(net),
                        br = brand(net),
                        ic = if view != "label" { format!("<i>{}</i>", social_svg(net)) } else { String::new() },
                        lb = if view != "icon" { format!("<span>{}</span>", esc(label)) } else { String::new() },
                        label = esc(label),
                    )
                })
                .collect();
            format!(r#"<div class="vb-share vb-share-{} vb-sh-{colours} vb-sh-{view}">{items}</div>"#, match s(m, "shape") { "" => "rounded", x => x })
        }
        "social" => {
            let ssel = format!("{sel} .vb-social");
            r.rule("", &ssel, &format!("--vb-ss:{}px;", n(m, "size").unwrap_or(24)));
            r.resp(m, "size", &ssel, |v| format!("--vb-ss:{};", px(v)));
            r.resp(m, "align", &ssel, |v| v.as_str().map(|a| format!("justify-content:{};", match a { "center" => "center", "right" => "flex-end", _ => "flex-start" })).unwrap_or_default());
            if let Some(c) = colour(m, "colour") {
                r.rule("", &ssel, &format!("color:{c};"));
            }
            let brand_on = s(m, "colours") == "brand";
            let items: String = m["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["href"].as_str().map(|h| (h, s(i, "net"))))
                .map(|(h, net)| {
                    format!(
                        r#"<a href="{}" target="_blank" rel="noopener me" aria-label="{}"{}>{}</a>"#,
                        href(h),
                        esc(net),
                        if brand_on { format!(r#" style="color:{}""#, brand(net)) } else { String::new() },
                        social_svg(net)
                    )
                })
                .collect();
            format!(r#"<div class="vb-social">{items}</div>"#)
        }
        "blockquote" => {
            if let Some(c) = colour(m, "accent") {
                r.rule("", sel, &format!("--vb-c1:{c};"));
            }
            let tweet = if b(m, "tweet") {
                let t = format!("“{}” — {}", s(m, "quote"), s(m, "author"));
                format!(r#"<a class="vb-bq-tweet" href="https://x.com/intent/post?text={}" target="_blank" rel="noopener nofollow">{} Share</a>"#, percent(&t), social_svg("x"))
            } else {
                String::new()
            };
            format!(
                r#"<figure class="vb-bq vb-bq-{skin}{al}"><blockquote>{q}</blockquote>{a}{tweet}</figure>"#,
                skin = match s(m, "skin") { "" => "border", x => x },
                al = align_class(m),
                q = text(s(m, "quote")),
                a = match s(m, "author") { "" => String::new(), a => format!("<figcaption>— {}</figcaption>", text(a)) },
            )
        }
        "code" => {
            if b(m, "copy") {
                r.uses_runtime = true;
            }
            if let Some(h) = n(m, "max_h").filter(|h| *h > 0) {
                r.rule("", &format!("{sel} pre"), &format!("max-height:{h}px;"));
            }
            let code = s(m, "code");
            let body = if b(m, "lines") {
                code.lines().map(|l| format!(r#"<span class="vb-ln">{}</span>"#, esc(l))).collect::<Vec<_>>().join("\n")
            } else {
                esc(code)
            };
            format!(
                r#"<div class="vb-code vb-code-{th}"><div class="vb-code-bar"><span>{lang}</span>{copy}</div><pre><code class="language-{lang}">{body}</code></pre></div>"#,
                th = match s(m, "theme") { "" => "dark", x => x },
                lang = esc(s(m, "language")),
                copy = if b(m, "copy") { r#"<button type="button" class="vb-copy">Copy</button>"# } else { "" },
            )
        }
        "text-path" => {
            let path = match s(m, "path") {
                "arc" => "M 20,180 A 230,230 0 0 1 480,180",
                "circle" => "M 250,250 m -200,0 a 200,200 0 1,1 400,0 a 200,200 0 1,1 -400,0",
                "line" => "M 10,150 L 490,150",
                _ => "M 10,160 C 90,60 170,60 250,160 S 410,260 490,160",
            };
            let tsel = format!("{sel} .vb-tpath");
            r.resp(m, "width", &tsel, |v| format!("width:{};", px(v)));
            let mut d = format!("font-size:{}px;", n(m, "size").unwrap_or(24));
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("fill:{c};"));
            }
            if let Some(rot) = n(m, "rotate_path") {
                d.push_str(&format!("rotate:{rot}deg;"));
            }
            r.rule("", &tsel, &d);
            let pid = format!("tp-{}-{}", esc(r.ctx.scope), esc(id));
            let t = esc(s(m, "text"));
            let body = format!(r##"<text><textPath href="#{pid}" startOffset="0">{t}</textPath></text>"##);
            let body = match m["link"].as_str() {
                Some(l) => format!(r#"<a href="{}">{body}</a>"#, href(l)),
                None => body,
            };
            format!(r#"<svg class="vb-tpath" viewBox="0 0 500 {}" aria-label="{t}"><path id="{pid}" d="{path}" fill="none"/>{body}</svg>"#, if s(m, "path") == "circle" { 500 } else { 300 })
        }
        "progress-tracker" => {
            r.uses_runtime = true;
            let mut d = format!("--vb-pt-h:{}px;", n(m, "height").unwrap_or(4));
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("--vb-pt-c:{c};"));
            }
            r.rule("", sel, &d);
            if s(m, "kind") == "circle" {
                format!(
                    r#"<div class="vb-ptrack vb-pt-circle vb-pt-{pos}" data-vb-progress><svg viewBox="0 0 36 36"><circle cx="18" cy="18" r="16" fill="none" stroke-width="3" class="vb-pt-bg"/><circle cx="18" cy="18" r="16" fill="none" stroke-width="3" class="vb-pt-fg" pathLength="100"/></svg>{}</div>"#,
                    if b(m, "percent") { "<b>0%</b>" } else { "" },
                    pos = match s(m, "position") { "" => "top", x => x },
                )
            } else {
                format!(r#"<div class="vb-ptrack vb-pt-bar vb-pt-{}" data-vb-progress><i></i></div>"#, match s(m, "position") { "" => "top", x => x })
            }
        }
        "off-canvas" => {
            r.uses_runtime = true;
            let mut d = format!("--vb-oc-w:{}px;", n(m, "width").unwrap_or(360));
            if let Some(c) = colour(m, "panel_bg") {
                d.push_str(&format!("--vb-oc-bg:{c};"));
            }
            r.rule("", sel, &d);
            let content = match m["template"].as_str().and_then(|t| r.ctx.templates.get(t)).cloned() {
                Some(l) if r.depth < 2 => {
                    r.depth += 1;
                    let html = super::render::sections_html(r, l["sections"].as_array().map(Vec::as_slice).unwrap_or(&[]));
                    r.depth -= 1;
                    html
                }
                _ => format!(r#"<div class="vb-text">{}</div>"#, dynamic::mark_tags(&crate::comments::sanitize_rich_html(s(m, "html")))),
            };
            let icon = match s(m, "icon") { "" => icon_svg("menu").unwrap_or_default(), i => icon_or_text(i) };
            format!(
                r#"<button type="button" class="vb-btn vb-btn-soft vb-btn-m vb-oc-open" aria-expanded="false" aria-controls="oc-{id}">{icon}<span>{label}</span></button><div class="vb-oc vb-oc-{side}{ov}" id="oc-{id}" hidden><div class="vb-oc-panel" role="dialog" aria-modal="true"><button type="button" class="vb-oc-x" aria-label="Close">{x}</button>{content}</div></div>"#,
                id = esc(id),
                label = text(s(m, "button")),
                side = match s(m, "side") { "" => "right", x => x },
                ov = if m["overlay"].as_bool().unwrap_or(true) { " vb-oc-dim" } else { "" },
                x = icon_svg("x").unwrap_or_default(),
            )
        }
        "pay-button" => {
            let provider = match s(m, "provider") { "" => "stripe", p => p };
            let link = if provider == "portal" {
                format!("#/portal/{}", match s(m, "plan") { "" => "signup", p => p })
            } else {
                m["href"].as_str().unwrap_or("#").to_string()
            };
            let mut d = String::new();
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("--vb-bc:{c};"));
            }
            r.rule("", sel, &d);
            let mark = match provider {
                "stripe" => icon_svg("lock").unwrap_or_default(),
                "paypal" => icon_svg("wallet").unwrap_or_default(),
                _ => icon_svg("cart").unwrap_or_default(),
            };
            format!(
                r#"<div class="vb-btns vb-al-{al}"><a class="vb-btn vb-btn-filled vb-btn-l vb-pay vb-pay-{provider}" href="{}"{}>{mark}<span>{}</span>{price}</a></div>"#,
                href(&link),
                if provider == "portal" { "" } else { r#" target="_blank" rel="noopener""# },
                text(match s(m, "text") { "" => "Buy now", t => t }),
                al = match s(m, "align") { "" => "left", x => x },
                price = match s(m, "price") { "" => String::new(), p => format!("<b>{}</b>", esc(p)) },
            )
        }
        "form" => match m["form"].as_str() {
            Some(f) => format!(r#"<div class="kg-card kg-form-card" data-form-id="{}"></div>"#, esc(f)),
            None if w["elementor"]["source"]["widgetType"] == "form" => super::elementor::style::form(r, w, sel),
            None => empty("Form"),
        },
        "signup" => {
            if let Some(c) = colour(m, "colour") {
                r.rule("", sel, &format!("--vb-bc:{c};"));
            }
            format!(
                r#"<form class="vb-signup vb-su-{lay} vb-al-{al}" data-members-form="subscribe">{name}<div class="vb-su-row"><input type="email" required data-members-email placeholder="{ph}" aria-label="Email address"><button class="vb-btn vb-btn-filled vb-btn-m" type="submit"><span class="vb-su-label">{btn}</span></button></div><p class="vb-su-ok">{ok}</p><p class="vb-su-err">Something went wrong. Please try again.</p>{note}</form>"#,
                lay = if s(m, "layout") == "stacked" { "stacked" } else { "inline" },
                al = match s(m, "align") { "" => "left", x => x },
                name = if b(m, "name") { r#"<input type="text" data-members-name placeholder="Your name" aria-label="Name">"# } else { "" },
                ph = esc(match s(m, "placeholder") { "" => "Your email address", p => p }),
                btn = text(match s(m, "button") { "" => "Subscribe", t => t }),
                ok = text(match s(m, "success") { "" => "Check your inbox to confirm.", t => t }),
                note = match s(m, "note") { "" => String::new(), t => format!(r#"<small class="vb-su-note">{}</small>"#, text(t)) },
            )
        }
        "search" => {
            let style = match s(m, "style") { "" => "field", x => x };
            let ph = esc(match s(m, "placeholder") { "" => "Search…", p => p });
            let ic = icon_svg("search").unwrap_or_default();
            let inner = match style {
                "icon" => format!(r#"<button type="button" class="vb-search-icon" data-ghost-search aria-label="Search">{ic}</button>"#),
                "button" => format!(r#"<button type="button" class="vb-btn vb-btn-outline vb-btn-m" data-ghost-search>{ic}<span>{}</span></button>"#, text(match s(m, "label") { "" => "Search", l => l })),
                _ => format!(r#"<button type="button" class="vb-search-field" data-ghost-search>{ic}<span>{ph}</span></button>"#),
            };
            format!(r#"<div class="vb-search vb-al-{}">{inner}</div>"#, match s(m, "align") { "" => "left", x => x })
        }
        "nav-menu" => {
            let msel = format!("{sel} .vb-menu");
            let mut d = String::new();
            if let Some(c) = colour(m, "colour") {
                d.push_str(&format!("--vb-mc:{c};"));
            }
            if let Some(c) = colour(m, "hover_colour") {
                d.push_str(&format!("--vb-mh:{c};"));
            }
            if let Some(c) = colour(m, "active_colour") {
                d.push_str(&format!("--vb-ma:{c};"));
            }
            r.rule("", &msel, &d);
            r.resp(m, "gap", &format!("{msel} ul"), |v| format!("gap:{};", px(v)));
            r.resp(m, "align", &format!("{msel} ul"), |v| v.as_str().map(|a| format!("justify-content:{};", match a { "center" => "center", "right" => "flex-end", "justify" => "space-between", _ => "flex-start" })).unwrap_or_default());
            r.uses_runtime = true;
            dynamic::dyn_marker(kind, id, m)
        }
        "site-logo" => {
            r.resp(m, "height", &format!("{sel} .vb-logo img"), |v| format!("height:{};", px(v)));
            r.resp(m, "align", &format!("{sel} .vb-logo"), |v| v.as_str().map(|a| format!("text-align:{a};")).unwrap_or_default());
            dynamic::dyn_marker(kind, id, m)
        }
        "posts" => {
            let gsel = format!("{sel} .vb-posts");
            r.rule("", &gsel, &format!("--vb-gc:{};--vb-gg:{}px;", n(m, "columns").unwrap_or(3), n(m, "gap").unwrap_or(24)));
            r.resp(m, "columns", &gsel, |v| v.as_i64().map(|c| format!("--vb-gc:{c};")).unwrap_or_default());
            match s(m, "ratio") {
                "" | "auto" => r.rule("", &format!("{gsel} .vb-pc-img"), "aspect-ratio:16/10;"),
                ratio => r.rule("", &format!("{gsel} .vb-pc-img"), &format!("aspect-ratio:{ratio};")),
            }
            if b(m, "filter") || s(m, "pagination") == "more" || s(m, "layout") == "carousel" {
                r.uses_runtime = true;
            }
            dynamic::dyn_marker(kind, id, m)
        }
        "products" | "directory-listings" => {
            let count = m["ids"].as_array().map_or(0, Vec::len);
            if count == 0 {
                return if r.ctx.editing { empty(if kind == "products" { "Choose shop products" } else { "Choose directory listings" }) } else { String::new() };
            }
            let grid = format!("{sel} .vb-catalogue");
            let desktop = n(m, "columns").unwrap_or(3).min(count as i64);
            let tablet = n(m, "columns_t").unwrap_or(desktop.min(2)).min(count as i64);
            let mobile = n(m, "columns_m").unwrap_or(1).min(count as i64);
            for (device, columns) in [("", desktop), ("t", tablet), ("m", mobile)] {
                r.rule(device, &grid, &format!("--vb-catalogue-columns:{columns};"));
            }
            r.resp(m, "gap", &grid, |v| format!("gap:{};", px(v)));
            let mut settings = m.clone();
            if r.ctx.editing {
                settings["editing"] = Value::Bool(true);
            }
            dynamic::dyn_marker(kind, id, &settings)
        }
        "login" | "breadcrumbs" | "author-box" | "post-nav" | "post-info" | "post-content" | "sitemap" => dynamic::dyn_marker(kind, id, m),
        "template" => match m["ref"].as_str().and_then(|t| r.ctx.templates.get(t)).cloned() {
            Some(l) if r.depth < 2 => {
                r.depth += 1;
                let html = super::render::sections_html(r, l["sections"].as_array().map(Vec::as_slice).unwrap_or(&[]));
                r.depth -= 1;
                format!(r#"<div class="vb-embedded">{html}</div>"#)
            }
            _ => empty("Template"),
        },
        "global" => empty("Global widget"),
        _ => String::new(),
    }
}

fn button(r: &mut R, m: &Value, sel: &str, widget: &Value) -> String {
    let bsel = format!("{sel} .vb-btn");
    let mut d = String::new();
    if let Some(c) = colour(m, "colour") {
        d.push_str(&format!("--vb-bc:{c};"));
    }
    if let Some(rad) = n(m, "btn_radius") {
        d.push_str(&format!("border-radius:{rad}px;"));
    }
    r.rule("", &bsel, &d);
    if let Some(c) = colour(m, "text_colour") {
        r.rule("", &format!("{bsel}:first-child"), &format!("--vb-bt:{c};"));
    }
    // the second button keeps only the colour: an outline takes its label
    // colour from its border
    let mut hv = String::new();
    if let Some(c) = colour(m, "hover_bg") {
        hv.push_str(&format!("background:{c};border-color:{c};"));
    }
    if let Some(c) = colour(m, "hover_text") {
        hv.push_str(&format!("color:{c};"));
    }
    r.rule("", &format!("{bsel}:first-child:hover"), &hv);
    r.resp(m, "align", &format!("{sel} .vb-btns"), |v| {
        v.as_str()
            .map(|a| match a {
                "center" => "justify-content:center;".to_string(),
                "right" => "justify-content:flex-end;".into(),
                "full" => "justify-content:stretch;--vb-bw:1;".into(),
                _ => "justify-content:flex-start;".into(),
            })
            .unwrap_or_default()
    });
    let kind = match s(m, "style") { "" => "filled", k => k };
    let size = match s(m, "size") { "" => "m", k => k };
    let icon = match s(m, "icon") { "" => String::new(), i => format!("<i>{}</i>", icon_or_text(i)) };
    let label = format!("<span>{}</span>", text(match s(m, "text") { "" => "Click here", t => t }));
    let content = if s(m, "icon_pos") == "after" { format!("{label}{icon}") } else { format!("{icon}{label}") };
    let rel = match (b(m, "new_tab"), b(m, "nofollow")) {
        (true, true) => r#" target="_blank" rel="noopener nofollow""#,
        (true, false) => r#" target="_blank" rel="noopener""#,
        (false, true) => r#" rel="nofollow""#,
        _ => "",
    };
    let link = m["href"].as_str().unwrap_or("#");
    let popup = m["video_popup"].as_bool().unwrap_or(widget["elementor"]["source"]["widgetType"] == "jkit_video_button");
    let dialog = if popup {
        embed_src(link, true, false, false, true, 0).map(|url| {
            r.uses_runtime = true;
            format!(r#" data-vb-video-dialog="{url}" aria-haspopup="dialog""#)
        }).unwrap_or_default()
    } else {
        String::new()
    };
    let mut html = format!(r#"<a class="vb-btn vb-btn-{kind} vb-btn-{size}" href="{}"{rel}{dialog}>{content}</a>"#, href(link));
    if !s(m, "text2").is_empty() {
        let c2 = colour(m, "colour").map(|c| format!(r#" style="--vb-bc:{}""#, esc(&c))).unwrap_or_default();
        html.push_str(&format!(r#"<a class="vb-btn vb-btn-outline vb-btn-{size}" href="{}"{rel}{c2}><span>{}</span></a>"#, href(m["href2"].as_str().unwrap_or("#")), text(s(m, "text2"))));
    }
    format!(r#"<div class="vb-btns{}">{html}</div>"#, if s(m, "align") == "full" { " vb-al-full" } else { "" })
}
