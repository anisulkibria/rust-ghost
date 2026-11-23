//! Resolve per-kit Elementor settings before converting individual widgets.
use super::str_at;
use serde_json::{Map, Value};
use std::collections::HashMap;

pub(super) fn apply(document: &mut Value, kit: Option<&Value>) -> Result<bool, String> {
    let mut globals = HashMap::new();
    if let Some(kit) = kit {
        let settings = &kit["page_settings"];
        if !settings.is_object() {
            return Err("Choose the matching Elementor kit global.json file.".into());
        }
        for (section, group) in [
            ("system_colors", "colors"),
            ("custom_colors", "colors"),
            ("system_typography", "typography"),
            ("custom_typography", "typography"),
        ] {
            for entry in settings[section].as_array().into_iter().flatten() {
                let id = str_at(entry, "_id");
                if !id.is_empty() {
                    globals.insert(format!("globals/{group}?id={id}"), entry);
                }
            }
        }
        if globals.is_empty() {
            return Err(
                "The kit settings file has no Elementor global colors or typography.".into(),
            );
        }
    }
    let mut unresolved = false;
    fn visit(nodes: &mut [Value], globals: &HashMap<String, &Value>, unresolved: &mut bool) {
        for node in nodes {
            if let Some(settings) = node.get_mut("settings").and_then(Value::as_object_mut) {
                resolve(settings, globals, unresolved);
            }
            if let Some(children) = node.get_mut("elements").and_then(Value::as_array_mut) {
                visit(children, globals, unresolved);
            }
        }
    }
    if let Some(settings) = document
        .get_mut("page_settings")
        .and_then(Value::as_object_mut)
    {
        resolve(settings, &globals, &mut unresolved);
    }
    if let Some(nodes) = document.get_mut("content").and_then(Value::as_array_mut) {
        visit(nodes, &globals, &mut unresolved);
    }
    Ok(unresolved)
}

fn resolve(
    settings: &mut Map<String, Value>,
    globals: &HashMap<String, &Value>,
    unresolved: &mut bool,
) {
    let Some(refs) = settings
        .get("__globals__")
        .and_then(Value::as_object)
        .cloned()
    else {
        return;
    };
    for (key, reference) in refs {
        let Some(reference) = reference.as_str().filter(|r| !r.is_empty()) else {
            continue;
        };
        if !reference.starts_with("globals/") {
            continue;
        }
        let Some(entry) = globals.get(reference) else {
            *unresolved = true;
            continue;
        };
        if reference.starts_with("globals/colors?") {
            if let Some(color) = entry
                .get("color")
                .and_then(Value::as_str)
                .and_then(super::spec::colour)
            {
                settings.insert(key, Value::String(color));
            }
        } else if let Some(prefix) = key.strip_suffix("typography_typography") {
            for (name, value) in entry.as_object().into_iter().flatten() {
                if name.starts_with("typography_") {
                    settings.insert(format!("{prefix}{name}"), value.clone());
                }
            }
        }
    }
}
