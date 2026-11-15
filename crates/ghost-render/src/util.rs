//! Small shared utilities for helpers.
use handlebars::{Context, Output, RenderContext, RenderError};
use serde_json::Value;

/// The current block context value (what `{{.}}` / paramless helpers target).
///
/// handlebars-rust resolves paths from the root data; `rc.evaluate(ctx, ".")`
/// returns the ROOT, so we read the block stack ourselves:
/// base_value first, then base_path walked against the root.
pub fn current_value(rc: &RenderContext<'_, '_>, ctx: &Context) -> Value {
    if let Some(block) = rc.block() {
        if let Some(v) = block.base_value() {
            return v.clone();
        }
        if !block.base_path().is_empty() {
            let path = block.base_path();
            if let Some(v) = walk(ctx.data(), path) {
                return v.clone();
            }
        }
        // block params ("as |item|") shadow the base value for named access
        // but bare helpers target the base — fall through to root otherwise.
    }
    ctx.data().clone()
}

/// The thing the page is *about* — the post, page, tag or author — however
/// deep in the template the helper was called.
///
/// `{{ghost_head}}` and the `meta_*` helpers live in the layout, where the
/// block context is the render root rather than the resource. Ghost resolves
/// these against the routed resource instead, which is what makes a post's
/// own meta title, social cards, canonical URL and code injection reach the
/// page; reading `{{.}}` there would only ever see the root.
pub fn current_resource(rc: &RenderContext<'_, '_>, ctx: &Context) -> Value {
    let this = current_value(rc, ctx);
    // Inside `{{#post}}` or a `{{#foreach}}`, the block value already is one.
    if this.get("id").and_then(Value::as_str).is_some() {
        return this;
    }
    for key in ["post", "page", "tag", "author"] {
        let candidate = root_get(rc, ctx, key);
        if candidate.get("id").and_then(Value::as_str).is_some() {
            return candidate;
        }
    }
    this
}

fn walk<'a>(mut v: &'a Value, path: &'a [String]) -> Option<&'a Value> {
    for seg in path {
        v = match v {
            Value::Object(m) => m.get(seg.as_str())?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Read `@root.<path>` from the render data (post-preprocess access to
/// site/config/member/...).
pub fn root_get(rc: &RenderContext<'_, '_>, ctx: &Context, path: &str) -> Value {
    rc.evaluate(ctx, &format!("@root.{path}"))
        .map(|v| v.as_json().clone())
        .unwrap_or_else(|_| {
            // fallback: walk the root data directly
            walk(ctx.data(), &path.split('.').map(str::to_string).collect::<Vec<_>>())
                .cloned()
                .unwrap_or(Value::Null)
        })
}

/// The whole render root data.
pub fn root_data(rc: &RenderContext<'_, '_>, ctx: &Context) -> Value {
    let _ = rc;
    ctx.data().clone()
}

/// Fill in the `url` of a post's nested tags and authors.
///
/// `post_to_json` has no router, so it leaves them unset; without this
/// `{{url}}` inside `{{#foreach tags}}` and `{{primary_tag.url}}` fall back
/// to "/" and every tag and author link on a theme points at the homepage.
pub fn resolve_nested_urls(post: &mut Value, resolve: &dyn Fn(&Value) -> String) {
    // The entry's own language. Its tags and authors link to *that*
    // language's archives — a tag on a Bengali post goes to
    // `/bn/tag/news/`, where the Bengali posts with that tag are — so a
    // theme's post card needs no change to stay in the reader's language.
    // Passed to the resolver as `url_locale`, which only it reads; a tag
    // with a language of its own keeps that one.
    let lang = post.get("locale").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
    let url_for = |item: &Value, is_tag: bool| -> String {
        let own = is_tag && item.get("locale").and_then(Value::as_str).is_some_and(|c| !c.is_empty());
        match &lang {
            Some(code) if !own => {
                let mut probe = item.clone();
                if let Value::Object(m) = &mut probe {
                    m.insert("url_locale".into(), Value::String(code.clone()));
                }
                resolve(&probe)
            }
            _ => resolve(item),
        }
    };
    for (field, is_tag) in [("tags", true), ("authors", false)] {
        if let Some(items) = post.get_mut(field).and_then(|v| v.as_array_mut()) {
            for item in items.iter_mut() {
                let url = url_for(item, is_tag);
                if let Value::Object(m) = item {
                    m.insert("url".into(), Value::String(url));
                }
            }
        }
    }
    for (field, is_tag) in [("primary_tag", true), ("primary_author", false)] {
        let Some(item) = post.get_mut(field).filter(|v| v.is_object()) else {
            continue;
        };
        let url = url_for(item, is_tag);
        if let Value::Object(m) = item {
            m.insert("url".into(), Value::String(url));
        }
    }
}

/// Resolve a standalone tag or author for a page in `page_locale` — a
/// `{{#get "tags"}}` list on `/bn/`, or the archive's own `tag` on
/// `/bn/tag/news/` — so it links to that language's archive. A tag with a
/// language of its own keeps it.
pub fn resolve_for_page(item: &Value, page_locale: Option<&str>, is_tag: bool, resolve: &dyn Fn(&Value) -> String) -> String {
    let own = is_tag && item.get("locale").and_then(Value::as_str).is_some_and(|c| !c.is_empty());
    match page_locale.filter(|c| !c.is_empty()) {
        Some(code) if !own => {
            let mut probe = item.clone();
            if let Value::Object(m) = &mut probe {
                m.insert("url_locale".into(), Value::String(code.to_string()));
            }
            resolve(&probe)
        }
        _ => resolve(item),
    }
}

/// Write helper output. `escape=true` pre-escapes plain text (Ghost's
/// non-SafeString helper outputs are HTML-escaped).
pub fn write_out(
    _rc: &mut RenderContext<'_, '_>,
    out: &mut dyn Output,
    s: &str,
    escape: bool,
) -> Result<(), RenderError> {
    if escape {
        let escaped = s
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#x27;");
        out.write(&escaped)?;
    } else {
        out.write(s)?;
    }
    Ok(())
}

#[cfg(test)]
mod nested_url_tests {
    use super::*;
    use serde_json::json;

    /// A stand-in resolver: what it was asked to localise, as a path.
    fn fake(item: &Value) -> String {
        let lang = item.get("url_locale").and_then(Value::as_str).map(|c| format!("/{c}")).unwrap_or_default();
        format!("{lang}/x/{}/", item["slug"].as_str().unwrap_or(""))
    }

    #[test]
    fn nested_tags_and_authors_follow_the_entrys_language() {
        let mut post = json!({
            "locale": "bn",
            "tags": [{"slug": "news", "locale": null}, {"slug": "khobor", "locale": "bn"}, {"slug": "english-only", "locale": "en"}],
            "authors": [{"slug": "ada", "locale": "fr"}],
            "primary_tag": {"slug": "news"},
            "primary_author": {"slug": "ada", "locale": "fr"},
        });
        resolve_nested_urls(&mut post, &fake);
        assert_eq!(post["tags"][0]["url"], "/bn/x/news/", "a tag for every language follows the post");
        // A tag with its own language is left to the resolver, which uses it.
        assert_eq!(post["tags"][1]["url"], "/x/khobor/");
        assert_eq!(post["tags"][2]["url"], "/x/english-only/");
        // An author's own `locale` (their admin language) is never used.
        assert_eq!(post["authors"][0]["url"], "/bn/x/ada/");
        assert_eq!(post["primary_tag"]["url"], "/bn/x/news/");
        assert_eq!(post["primary_author"]["url"], "/bn/x/ada/");
        // The probe key never reaches the theme.
        assert!(post["tags"][0].get("url_locale").is_none());
    }

    #[test]
    fn a_default_language_entry_leaves_its_links_alone() {
        let mut post = json!({"locale": null, "tags": [{"slug": "news"}]});
        resolve_nested_urls(&mut post, &fake);
        assert_eq!(post["tags"][0]["url"], "/x/news/");
    }
}

