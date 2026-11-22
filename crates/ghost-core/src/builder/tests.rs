use super::*;
use serde_json::json;
use std::collections::HashMap;

fn draw(layout: &Value) -> String {
    render(&normalise(layout).unwrap(), &Settings::default())
}

#[test]
fn settings_default_and_reject_out_of_range() {
    let s = Settings::default();
    assert!(s.enabled);
    assert_eq!(s.width, 1140);
    assert_eq!(s.maintenance, Maintenance::Off);
    let s = Settings::read(|k| match k {
        SETTING_WIDTH => Some("77".into()),
        SETTING_PALETTE => Some("#ABC,#zzz,,,,".into()),
        SETTING_POST_TYPES => Some("pages".into()),
        SETTING_HIDE_HEADER => Some(".a > b{}</style>".into()),
        _ => None,
    });
    assert_eq!(s.width, 1140);
    assert_eq!(s.palette[0], "#abc");
    assert!(s.allows("page") && !s.allows("post"));
    assert!(!s.hide_header.contains('{') && !s.hide_header.contains('<'));
}

#[test]
fn normalise_drops_unknown_and_unsafe() {
    let raw = json!({"sections": [{"id": "s<1", "s": {"bg": "colour", "bg_colour": "red;}", "pt": 99999, "evil": 1, "pt_m": 20},
        "cols": [{"s": {"w": 40}, "items": [
            {"t": "button", "s": {"text": "Go", "href": "java\nscript:alert(1)", "onclick": "x", "attrs": [{"name": "onmouseover", "value": "x"}, {"name": "data-x", "value": "1"}]}},
            {"t": "nope", "s": {}},
            {"t": "heading", "s": {"text": "<b>Hi</b>", "tag": "script"}}
        ]}]}]});
    let l = normalise(&raw).unwrap();
    let sec = &l["sections"][0];
    assert_ne!(sec["id"], "s<1");
    assert!(sec["s"].get("bg_colour").is_none());
    assert!(sec["s"].get("evil").is_none());
    assert_eq!(sec["s"]["pt"], 600);
    assert_eq!(sec["s"]["pt_m"], 20);
    assert_eq!(sec["cols"][0]["s"]["w"], 12);
    let items = sec["cols"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[0]["s"].get("href").is_none());
    assert_eq!(items[0]["s"]["attrs"].as_array().unwrap().len(), 1);
    let html = render(&l, &Settings::default());
    assert!(html.contains("&lt;b&gt;Hi&lt;/b&gt;"));
    assert!(!html.contains("javascript"));
    assert!(!html.contains("onmouseover"));
    assert!(html.contains(r#"data-x="1""#));
    assert!(html.contains("@media (max-width:640px)"), "responsive padding emitted");
}

#[test]
fn text_widget_is_sanitised_and_tags_marked() {
    let html = draw(&json!({"sections": [{"cols": [{"items": [{"t": "text", "s": {"html": "<p onclick=x>Hi [[post.title]] <script>bad()</script><a href=\"javascript:x\">l</a></p>"}}]}]}]}));
    assert!(html.contains("<p>Hi %%vb:post.title%% <a>l</a></p>"), "{html}");
    assert!(!html.contains("bad()"));
}

#[test]
fn limits_are_enforced() {
    let many: Vec<Value> = (0..7).map(|_| json!({})).collect();
    assert!(normalise(&json!({"sections": [{"cols": many}]})).is_err());
    assert!(normalise(&json!([])).is_err());
}

#[test]
fn inner_sections_nest_once() {
    let raw = json!({"sections": [{"cols": [{"items": [{"t": "inner", "cols": [{"items": [{"t": "inner", "cols": []}, {"t": "heading", "s": {"text": "deep"}}]}]}]}]}]});
    let l = normalise(&raw).unwrap();
    let inner = &l["sections"][0]["cols"][0]["items"][0];
    assert_eq!(inner["t"], "inner");
    let nested = inner["cols"][0]["items"].as_array().unwrap();
    assert_eq!(nested.len(), 1, "an inner section inside an inner section is dropped");
    assert!(render(&l, &Settings::default()).contains("vb-inner"));
    assert_eq!(counts(&l), (1, 2));
}

#[test]
fn urls_colours_sides() {
    assert!(safe_url("/about/").is_some());
    assert!(safe_url("#/portal/signup").is_some());
    assert!(safe_url("[[post.url]]").is_some());
    assert!(safe_url("[[evil]]").is_none());
    assert!(safe_url(" JavaScript:alert(1)").is_none());
    assert!(safe_url("//evil.example").is_none());
    assert!(safe_url("data:text/html,x").is_none());
    let l = normalise(&json!({"sections": [{"cols": [{"items": [{"t": "heading", "s": {"margin": "10 20", "padding": "x y", "colour": "c9"}}]}]}]})).unwrap();
    let s = &l["sections"][0]["cols"][0]["items"][0]["s"];
    assert_eq!(s["margin"], "10 20 10 20");
    assert!(s.get("padding").is_none());
    assert!(s.get("colour").is_none());
}

#[test]
fn every_widget_renders_with_defaults_and_empty() {
    for w in spec::WIDGETS {
        let defaults: Value = serde_json::from_str(w.defaults).unwrap_or_else(|e| panic!("{} defaults: {e}", w.key));
        for s in [json!({}), defaults] {
            let l = normalise(&json!({"sections": [{"cols": [{"items": [{"id": "w1", "t": w.key, "s": s}]}]}]})).unwrap();
            let html = render(&l, &Settings::default());
            assert!(html.contains("vb-page"), "{}", w.key);
        }
    }
}

#[test]
fn custom_css_and_page_css_cannot_close_style() {
    let html = draw(&json!({"page": {"css": "a{}</style><script>x</script>"}, "sections": [{"cols": [{"items": [{"t": "heading", "s": {"css": "selector{color:red}</style><b>"}}]}]}]}));
    assert!(!html.contains("</style><script>"));
    assert!(!html.contains("</style><b>"));
    assert!(html.contains(r#".vb-Lp [data-vb-id="x"#) || html.contains(r#"[data-vb-id="#));
}

#[test]
fn conditions_wrap_on_the_site_not_in_the_editor() {
    let l = normalise(&json!({"sections": [{"cols": [{"items": [{"id": "w1", "t": "heading", "s": {"text": "Paid", "if_who": "paid"}}]}]}]})).unwrap();
    let s = Settings::default();
    let none = HashMap::new();
    let site = render_with(&l, &Ctx { settings: &s, scope: "p", editing: false, templates: &none });
    assert!(site.contains("<!--vb-if:w1:who=paid-->"));
    let ed = render_with(&l, &Ctx { settings: &s, scope: "p", editing: true, templates: &none });
    assert!(!ed.contains("vb-if"));
    assert!(ed.contains("draggable"));
}

#[test]
fn global_widgets_follow_their_template() {
    let mut t = HashMap::new();
    t.insert("g1".to_string(), json!({"sections": [{"cols": [{"items": [{"id": "orig", "t": "button", "s": {"text": "Shared label"}}]}]}]}));
    let l = normalise(&json!({"sections": [{"cols": [{"items": [{"id": "here", "t": "global", "s": {"ref": "g1"}}]}]}]})).unwrap();
    let s = Settings::default();
    let html = render_with(&l, &Ctx { settings: &s, scope: "p", editing: false, templates: &t });
    assert!(html.contains("Shared label"));
    assert!(html.contains(r#"data-vb-id="here""#));
}

#[test]
fn dynamic_widgets_leave_markers() {
    let html = draw(&json!({"sections": [{"cols": [{"items": [{"id": "w1", "t": "posts", "s": {"count": 3}}]}]}]}));
    assert!(html.contains(r#"data-vb-dyn="posts""#));
}

#[test]
fn google_fonts_load_only_when_used() {
    let plain = draw(&json!({"sections": []}));
    assert!(!plain.contains("fonts.googleapis.com"));
    let used = draw(&json!({"sections": [{"cols": [{"items": [{"t": "heading", "s": {"font": "poppins"}}]}]}]}));
    assert!(used.contains("family=Poppins"));
    let s = Settings::read(|k| (k == SETTING_GOOGLE_FONTS).then(|| "false".into()));
    let l = normalise(&json!({"sections": [{"cols": [{"items": [{"t": "heading", "s": {"font": "poppins"}}]}]}]})).unwrap();
    assert!(!render(&l, &s).contains("fonts.googleapis.com"));
}

#[test]
fn plain_text_collects_words() {
    let l = starter("Hello", "<p>World <b>here</b></p>");
    let t = plain_text(&l);
    assert!(t.contains("Hello") && t.contains("World here"), "{t}");
}

#[test]
fn spec_json_covers_every_widget() {
    let v = spec::spec_json();
    assert_eq!(v["widgets"].as_array().unwrap().len(), spec::WIDGETS.len());
    for kind in TEMPLATE_KINDS {
        assert!(normalise(&starter_for(kind.0)).is_ok(), "{}", kind.0);
    }
}
