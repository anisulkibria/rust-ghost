//! Changing a site's publication language without losing its others.
//!
//! The publication language (`locale`) is the *default* language of a
//! multilingual site: the unprefixed one, the one entries with no locale of
//! their own are in, and the one whose menus are Ghost's own `navigation`
//! and `secondary_navigation`. Writing `locale` on its own therefore
//! silently rewrites all three. The language that used to be the default
//! falls out of the site, every untagged entry becomes the new language, and
//! the new default keeps the old default's menus while its own are thrown
//! away. [`change_default`] is the one way to change it, used by Settings →
//! General and by the Admin API alike.

use vuuut_core::locales::{normalise, Locales};
use vuuut_core::Result;
use serde_json::{Map, Value};

use crate::AppState;

/// Where an entry lives on the public site, as a site-relative path with its
/// language prefix — `/bn/<slug>/` for a Bengali post, `/<slug>/` for one
/// in the default language — and without the install's base path.
///
/// The router's permalinks are language-neutral, so anything outside a
/// rendered page that links to an entry (a newsletter, a Slack message, the
/// analytics that match page views to posts, `llms.txt`) has to add the
/// prefix itself; this is the one place that does. Relations are loaded
/// because a permalink may use the primary tag or author. The language
/// settings are read now, so a change of default language is followed.
pub async fn entry_path(state: &AppState, post: &vuuut_core::models::Post) -> String {
    let rel = vuuut_db::repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
    let data = vuuut_core::models::post_to_json(post, &rel.authors_for(&post.id), &rel.tags_for(&post.id), "");
    let path = {
        let router = state.router.read().await;
        router
            .permalink_for(&data)
            .map(|(_, permalink)| vuuut_routing::tree::build_permalink(&permalink, &data))
            .unwrap_or_else(|| format!("/{}/", post.slug.as_deref().unwrap_or_default()))
    };
    let locales = Locales::from_settings(state.settings.get_str("locale").as_deref(), state.settings.get("locales_enabled").as_ref());
    let locale = locales.resolve(post.locale.as_deref());
    locales.localise(&path, &locale)
}

/// [`entry_path`] as an absolute URL.
pub async fn entry_url(state: &AppState, post: &vuuut_core::models::Post) -> String {
    format!("{}{}", state.site_url().trim_end_matches('/'), entry_path(state, post).await)
}

/// One of the per-language menu maps as an object, whatever shape it was
/// stored in.
fn menu_map(state: &AppState, key: &str) -> Map<String, Value> {
    match state.settings.get(key) {
        Some(Value::Object(o)) => o,
        Some(Value::String(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
        _ => Map::new(),
    }
}

fn menu(state: &AppState, key: &str) -> Value {
    match state.settings.get(key) {
        Some(v @ Value::Array(_)) => v,
        Some(Value::String(raw)) => serde_json::from_str(&raw).unwrap_or_else(|_| Value::Array(vec![])),
        _ => Value::Array(vec![]),
    }
}

/// Keep a menu URL pointing at the same language after prefixes change.
fn rebase_menu(menu: &mut Value, old: &Locales, next: &Locales, site_url: &str) {
    let Some(items) = menu.as_array_mut() else { return };
    for item in items {
        let Some(Value::String(url)) = item.get_mut("url") else { continue };
        let Some((base, path, suffix)) = vuuut_core::url::site_link(url, site_url) else { continue };
        let (locale, path) = old.split_path(path);
        if old.prefix(&locale) == next.prefix(&locale) {
            continue;
        }
        // These routes are shared by all languages.
        if matches!(path.split('/').nth(1), Some("ghost" | "assets" | "content" | "public" | "members" | "robots.txt" | "sitemap.xml")) {
            continue;
        }
        *url = format!("{base}{}{suffix}", next.localise(&path, &locale));
    }
}

/// What the language settings become when the default moves from `old` to
/// `new`: the languages besides the default, and both menu sets.
///
/// Pure, so the rules are testable without a database:
///
/// * the old default stays a language the site publishes in, and the new
///   default stops being listed as an extra one;
/// * the old default's menus move into the map under its code;
/// * the new default's own menus, if it had any, become the site-wide ones
///   and leave the map. A new default with no menus of its own keeps the
///   current site-wide menus, so nothing is left without a menu.
#[derive(Debug, PartialEq)]
pub(crate) struct Swap {
    pub enabled: Vec<String>,
    pub navigation: Value,
    pub secondary_navigation: Value,
    pub navigation_locales: Map<String, Value>,
    pub secondary_navigation_locales: Map<String, Value>,
}

pub(crate) fn swap(
    old: &Locales,
    new: &str,
    menus: (Value, Value),
    mut maps: (Map<String, Value>, Map<String, Value>),
    site_url: &str,
) -> Swap {
    let old_default = old.default_locale().to_string();
    let enabled: Vec<String> = old.enabled().iter().filter(|c| c.as_str() != new).cloned().collect();
    let codes: Vec<&str> = enabled.iter().map(String::as_str).collect();
    let next = Locales::new(new, &codes);
    let (mut navigation, mut secondary) = menus;
    rebase_menu(&mut navigation, old, &next, site_url);
    rebase_menu(&mut secondary, old, &next, site_url);
    for menu in maps.0.values_mut().chain(maps.1.values_mut()) {
        rebase_menu(menu, old, &next, site_url);
    }
    let promote = |map: &mut Map<String, Value>, current: Value| -> Value {
        let own = map.remove(new);
        map.insert(old_default.clone(), current.clone());
        match own {
            Some(v @ Value::Array(_)) => v,
            _ => current,
        }
    };
    let navigation = promote(&mut maps.0, navigation);
    let secondary_navigation = promote(&mut maps.1, secondary);
    Swap {
        enabled,
        navigation,
        secondary_navigation,
        navigation_locales: maps.0,
        secondary_navigation_locales: maps.1,
    }
}

/// Make `raw` the site's publication language.
///
/// On a single-language site this is only a relabel: the site was in one
/// language and still is, so nothing else moves. On a multilingual site the
/// old default stays a language, existing entries keep the language they
/// were written in, and the menus move with their languages (see [`swap`]).
///
/// A value that is not a language code is stored as given, as it always
/// was. Ghost accepts any string here, and refusing it would make an
/// unrelated settings save fail.
pub async fn change_default(state: &AppState, raw: &str) -> Result<()> {
    set_default(state, raw, false).await
}

/// The Languages tool keeps the previous language and its content even when
/// this is the first time the site publishes in more than one language.
pub async fn promote_default(state: &AppState, code: &str) -> Result<()> {
    set_default(state, code, true).await
}

async fn set_default(state: &AppState, raw: &str, keep_previous: bool) -> Result<()> {
    let old = Locales::from_settings(state.settings.get_str("locale").as_deref(), state.settings.get("locales_enabled").as_ref());
    let Some(new) = normalise(raw) else {
        return state.settings.set(&state.db, "locale", &Value::String(raw.to_string())).await;
    };
    if new == old.default_locale() || (!keep_previous && !old.multilingual()) {
        state.settings.set(&state.db, "locale", &Value::String(new.clone())).await?;
        if new != old.default_locale() {
            reload_theme(state).await;
        }
        return Ok(());
    }

    let swapped = swap(
        &old,
        &new,
        (menu(state, "navigation"), menu(state, "secondary_navigation")),
        (menu_map(state, "navigation_locales"), menu_map(state, "secondary_navigation_locales")),
        &state.site_url(),
    );

    // Content first: once it carries its language explicitly, nothing below
    // can mislabel it, whatever order the settings land in.
    let stamped = vuuut_db::repos::locales::stamp_default(&state.db, old.default_locale()).await?;
    state.settings.set_many(&state.db, &[
        ("locales_enabled", serde_json::json!(swapped.enabled)),
        ("navigation", swapped.navigation),
        ("secondary_navigation", swapped.secondary_navigation),
        ("navigation_locales", Value::Object(swapped.navigation_locales)),
        ("secondary_navigation_locales", Value::Object(swapped.secondary_navigation_locales)),
        ("locale", Value::String(new.clone())),
    ]).await?;
    tracing::info!(
        "publication language {} -> {new}; {stamped} entries kept as {}",
        old.default_locale(),
        old.default_locale()
    );
    reload_theme(state).await;
    Ok(())
}

/// The theme's compiled registry holds one locale file per language, keyed
/// with the default under `""`, so a new default renders in the old one's
/// strings until it is rebuilt.
async fn reload_theme(state: &AppState) {
    if let Err(e) = crate::themes::reactivate(state).await {
        tracing::error!("locale change: theme reload failed: {}", e.message());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn en() -> Value {
        json!([{"label": "Home", "url": "/"}])
    }
    fn bn() -> Value {
        json!([{"label": "হোম", "url": "/bn/"}])
    }

    #[test]
    fn the_old_default_stays_and_the_menus_move_with_their_languages() {
        let old = Locales::new("en", &["bn", "fr"]);
        let mut primary = Map::new();
        primary.insert("bn".into(), bn());
        primary.insert("fr".into(), json!([]));
        let mut secondary = Map::new();
        secondary.insert("bn".into(), json!([{"label": "যোগ দিন", "url": "#/portal/"}]));
        let s = swap(&old, "bn", (en(), json!([{"label": "Sign up", "url": "#/portal/"}])), (primary, secondary), "https://example.test");

        assert_eq!(s.enabled, ["en", "fr"], "English must stay, Bengali is now the default");
        assert_eq!(s.navigation, json!([{"label": "হোম", "url": "/"}]));
        assert_eq!(s.secondary_navigation, json!([{"label": "যোগ দিন", "url": "#/portal/"}]));
        assert_eq!(s.navigation_locales["en"], json!([{"label": "Home", "url": "/en/"}]));
        assert_eq!(s.secondary_navigation_locales["en"], json!([{"label": "Sign up", "url": "#/portal/"}]));
        assert!(!s.navigation_locales.contains_key("bn"), "the default's menu is the site-wide one");
        // Another language's menu, even a deliberately empty one, is untouched.
        assert_eq!(s.navigation_locales["fr"], json!([]));
    }

    #[test]
    fn rebasing_updates_cross_language_links_without_losing_url_parts() {
        let old = Locales::new("en", &["bn", "fr"]);
        let next = Locales::new("bn", &["en", "fr"]);
        let mut menu = json!([
            {"url": "https://example.test/blog/bn/বাংলা/?ref=nav#top", "label": "বাংলা"},
            {"url": "/about/#team"},
            {"url": "/fr/about/"},
            {"url": "/content/files/guide.pdf"},
            {"url": "https://xবাংলাদেশ.example/bn/"},
            {"url": "#language-switcher"},
        ]);
        rebase_menu(&mut menu, &old, &next, "https://example.test/blog");
        assert_eq!(menu, json!([
            {"url": "https://example.test/blog/বাংলা/?ref=nav#top", "label": "বাংলা"},
            {"url": "/en/about/#team"},
            {"url": "/fr/about/"},
            {"url": "/content/files/guide.pdf"},
            {"url": "https://xবাংলাদেশ.example/bn/"},
            {"url": "#language-switcher"},
        ]));
    }

    #[test]
    fn a_new_default_with_no_menus_of_its_own_keeps_the_current_ones() {
        let old = Locales::new("en", &["bn"]);
        let s = swap(&old, "de", (en(), json!([])), (Map::new(), Map::new()), "https://example.test");
        assert_eq!(s.enabled, ["en", "bn"]);
        assert_eq!(s.navigation, json!([{"label": "Home", "url": "/en/"}]));
        assert_eq!(s.navigation_locales["en"], json!([{"label": "Home", "url": "/en/"}]));
    }
}
