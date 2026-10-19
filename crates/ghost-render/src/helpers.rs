//! Ghost Handlebars helpers. Each helper mirrors the semantics of its
//! counterpart in ghost/core/core/frontend/helpers/*.
use handlebars::{
    Context, Handlebars, Helper, HelperDef, HelperResult, Output, RenderContext,
    Renderable,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::util::{current_resource, current_value, root_get, write_out};

// ---------------------------------------------------------------------------
// shared predicates — ports of frontend/services/data/checks.js and
// @tryghost/helpers utils/visibility.js
// ---------------------------------------------------------------------------

/// Ghost identifies resource kinds structurally, by which keys are present.
fn has_keys(v: &Value, keys: &[&str]) -> bool {
    let Value::Object(m) = v else { return false };
    keys.iter().all(|k| m.contains_key(*k))
}

fn is_post(v: &Value) -> bool {
    has_keys(v, &["html", "title", "slug"])
}

fn is_newsletter(v: &Value) -> bool {
    has_keys(v, &["name", "subscribe_on_signup", "visibility"])
}

/// `visibility.filter` — a comma-separated list, defaulting to "public".
/// Items without a visibility of their own are kept only when the list
/// includes "public"; "all" keeps everything.
fn visibility_keeps(item: &Value, visibility: Option<&str>) -> bool {
    let list: Vec<&str> = visibility
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.split(',').map(str::trim).collect())
        .unwrap_or_else(|| vec!["public"]);
    if list.contains(&"all") {
        return true;
    }
    match item.get("visibility").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        Some(vis) => list.contains(&vis),
        None => list.contains(&"public"),
    }
}

/// A hash option parsed the way Ghost does (`parseInt(x, 10) || fallback`),
/// so both `limit=3` and `limit="3"` work and 0/garbage means "unset".
fn hash_count(h: &Helper<'_>, key: &str) -> Option<usize> {
    let v = h.hash_get(key)?.value();
    let n = v
        .as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))?;
    (n > 0).then_some(n as usize)
}

/// Whether a flag-style hash option is on. Ghost treats the string
/// `"false"` as off, matching how themes write `absolute="false"`.
fn hash_flag(h: &Helper<'_>, key: &str) -> bool {
    match h.hash_get(key).map(|v| v.value()) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty() && s != "false",
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

/// A string hash option, present only when it really is a string (Ghost
/// guards most of these with `isString`).
fn hash_str(h: &Helper<'_>, key: &str) -> Option<String> {
    h.hash_get(key).and_then(|v| v.value().as_str()).map(str::to_string)
}

// ---------------------------------------------------------------------------
// shared deps
// ---------------------------------------------------------------------------

/// Callback computing a resource's public URL (wired to the router tree).
pub type UrlResolver = Arc<dyn Fn(&Value) -> String + Send + Sync>;

#[derive(Clone)]
pub struct HelperDeps {
    pub pool: Arc<sqlx::SqlitePool>,
    pub runtime: tokio::runtime::Handle,
    pub url_resolver: UrlResolver,
    /// theme asset path ("css/screen.css") → hash
    pub asset_hashes: Arc<HashMap<String, String>>,
    /// cache-busting hash for asset paths with no file of their own
    pub global_asset_hash: String,
    pub theme_dir: std::path::PathBuf,
    pub site_origin: String,
    /// public Content API key, for `{{content_api_key}}`
    pub content_api_key: String,
}

// ---------------------------------------------------------------------------
// foreach
// ---------------------------------------------------------------------------

pub struct Foreach;

impl HelperDef for Foreach {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(param) = h.param(0) else {
            tracing::warn!("need to pass an iterator to {{#foreach}}");
            return Ok(());
        };
        let value = param.value().clone();

        // Ghost iterates arrays and plain objects alike; keep each item's own
        // key so the context path still resolves after filtering.
        let pairs: Vec<(String, Value)> = match &value {
            Value::Array(a) => a.iter().cloned().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
            Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        };

        // Collections of posts and newsletters default to visibility="all";
        // everything else to the "public" default of the visibility filter.
        let visibility = hash_str(h, "visibility").or_else(|| {
            let first = pairs.first().map(|(_, v)| v)?;
            (is_post(first) || is_newsletter(first)).then(|| "all".to_string())
        });
        let pairs: Vec<(String, Value)> = pairs
            .into_iter()
            .filter(|(_, v)| visibility_keeps(v, visibility.as_deref()))
            .collect();

        let length = pairs.len();
        if length == 0 {
            if let Some(t) = h.inverse() {
                t.render(r, ctx, rc, out)?;
            }
            return Ok(());
        }

        // limit / from / to — `to` is derived from `limit` when a limit was
        // given that actually narrows the collection (Ghost's rule).
        let limit = hash_count(h, "limit").unwrap_or(length);
        let from = hash_count(h, "from").unwrap_or(1);
        let mut to = hash_count(h, "to").unwrap_or(length);
        if limit < length && from + limit <= length {
            to = from - 1 + limit;
        }
        let columns = hash_count(h, "columns");

        let array_path = param.context_path().cloned();
        rc.push_block(handlebars::BlockContext::new());

        for (offset, (key, item)) in pairs.iter().enumerate() {
            let current = offset + 1;
            if current < from {
                continue;
            }
            if current > to {
                break;
            }
            // index is the position in the whole collection, not in the slice
            let index = current - 1;
            if let Some(block) = rc.block_mut() {
                match &array_path {
                    Some(p) => {
                        let mut newp = p.clone();
                        newp.push(key.clone());
                        *block.base_path_mut() = newp;
                    }
                    None => block.set_base_value(item.clone()),
                }
                let key_var: Value = match &value {
                    Value::Array(_) => index.into(),
                    _ => Value::String(key.clone()),
                };
                block.set_local_var("key", key_var.clone());
                block.set_local_var("index", index.into());
                block.set_local_var("number", (index + 1).into());
                block.set_local_var("first", (index == from - 1).into());
                block.set_local_var("last", (current == to).into());
                block.set_local_var("even", (index % 2 == 1).into());
                block.set_local_var("odd", (index % 2 == 0).into());
                // without columns= these stay false, as they do in Ghost
                // (`index % undefined` is NaN there)
                block.set_local_var("rowStart", columns.map(|c| index % c == 0).unwrap_or(false).into());
                block.set_local_var("rowEnd", columns.map(|c| index % c == c - 1).unwrap_or(false).into());
                let mut params = handlebars::BlockParams::new();
                if let Some((p_val, p_key)) = h.block_param_pair() {
                    params.add_value(p_val, item.clone())?;
                    params.add_value(p_key, key_var)?;
                } else if let Some(p_val) = h.block_param() {
                    params.add_value(p_val, item.clone())?;
                }
                block.set_block_params(params);
            }
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
        }
        rc.pop_block();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// content / excerpt / title / url
// ---------------------------------------------------------------------------

pub struct Content;
impl HelperDef for Content {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let html = this.get("html").and_then(|v| v.as_str()).unwrap_or_default();

        // members gating: render the content-cta partial instead
        if this.get("access") == Some(&Value::Bool(false)) {
            if let Some(tpl) = r.get_template("content-cta") {
                let mut block = handlebars::BlockContext::new();
                block.set_base_value(this.clone());
                rc.push_block(block);
                tpl.render(r, ctx, rc, out)?;
                rc.pop_block();
                return Ok(());
            }
        }

        // words=N / characters=N truncation
        let words = hash_count(h, "words").or_else(|| (h.hash_get("words").is_some()).then_some(0));
        let chars = hash_count(h, "characters").or_else(|| (h.hash_get("characters").is_some()).then_some(0));
        let html_out = downsize(html, words, chars);
        write_out(rc, out, &html_out, false)
    }
}

/// Elements that never have a closing tag, so they must not go on the
/// open-tag stack while truncating.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Port of the `downsize` package Ghost truncates with. Markup does not count
/// towards the budget, and any element left open by the cut is closed again,
/// so the result is still well-formed HTML. Nothing is appended — Ghost does
/// not pass downsize an `append` option.
pub fn downsize(html: &str, words: Option<usize>, characters: Option<usize>) -> String {
    if words.is_none() && characters.is_none() {
        return html.to_string();
    }
    if words == Some(0) || characters == Some(0) {
        return String::new();
    }

    let mut open: Vec<String> = Vec::new();
    // markup seen since the last text character. A truncation discards it,
    // so a cut that lands just after `<p>` does not leave `<p></p>` behind.
    let mut pending = String::new();
    let mut committed_open: Vec<String> = Vec::new();
    let mut out = String::new();
    let mut word_count = 0usize;
    let mut char_count = 0usize;
    let mut in_word = false;
    let mut truncated = false;
    let mut rest = html;

    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(end) => {
                    pending.push_str(&rest[..end + 3]);
                    rest = &rest[end + 3..];
                }
                None => {
                    pending.push_str(rest);
                    break;
                }
            }
            continue;
        }
        if rest.starts_with('<') {
            match rest.find('>') {
                Some(end) => {
                    pending.push_str(&rest[..=end]);
                    track_tag(&rest[..=end], &mut open);
                    rest = &rest[end + 1..];
                    // a tag boundary also ends the current word
                    in_word = false;
                }
                None => {
                    pending.push_str(rest);
                    break;
                }
            }
            continue;
        }

        let ch = rest.chars().next().unwrap_or('\0');
        if ch.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            word_count += 1;
            if words.is_some_and(|w| word_count > w) {
                truncated = true;
                break;
            }
        }
        out.push_str(&pending);
        pending.clear();
        committed_open.clone_from(&open);
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
        char_count += 1;
        if characters.is_some_and(|c| char_count >= c) {
            truncated = true;
            break;
        }
    }

    if !truncated {
        out.push_str(&pending);
        committed_open.clone_from(&open);
    }

    let mut out = out.trim_end().to_string();
    for name in committed_open.iter().rev() {
        out.push_str(&format!("</{name}>"));
    }
    out
}

/// Keep the open-element stack in step with one `<…>` tag.
fn track_tag(tag: &str, open: &mut Vec<String>) {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim();
    // doctype, processing instruction and comment markers open nothing
    if inner.starts_with('!') || inner.starts_with('?') || inner.is_empty() {
        return;
    }
    if let Some(closing) = inner.strip_prefix('/') {
        let name = tag_name(closing);
        if let Some(pos) = open.iter().rposition(|n| *n == name) {
            open.truncate(pos);
        }
        return;
    }
    if inner.ends_with('/') {
        return;
    }
    let name = tag_name(inner);
    if !name.is_empty() && !VOID_ELEMENTS.contains(&name.as_str()) {
        open.push(name);
    }
}

fn tag_name(s: &str) -> String {
    s.trim()
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub struct Excerpt;
impl HelperDef for Excerpt {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let custom = this
            .get("custom_excerpt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let text = custom
            .or_else(|| this.get("excerpt").and_then(|v| v.as_str()))
            .unwrap_or_default();
        // Ghost escapes first, then truncates the escaped string
        let escaped = html_escape_full(text);

        let (words, chars) = if custom.is_some() {
            // a custom excerpt is never word-truncated, only capped at its
            // own (escape-expanded) length
            (None, Some(escaped.chars().count()))
        } else {
            let words = hash_count(h, "words");
            let chars = hash_count(h, "characters");
            if words.is_none() && chars.is_none() {
                (Some(50), None)
            } else {
                (words, chars)
            }
        };

        let excerpt = downsize(&escaped, words, chars);
        write_out(rc, out, &excerpt, false)
    }
}

pub struct Title;
impl HelperDef for Title {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let t = this.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        write_out(rc, out, &escape_expression(t), false)
    }
}

pub struct UrlHelper;
impl HelperDef for UrlHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // {{url}} — current scope; {{url post}} / {{url "/about/"}} — explicit
        let target = match h.param(0).map(|p| p.value()) {
            // a literal path written by the theme is in site terms, so it is
            // the one form here that still needs the base path applied; a
            // resource's `url` field already carries it
            Some(Value::String(s)) => {
                let url = ghost_core::url::base(s);
                let url = match hash_flag(h, "absolute") {
                    true => ghost_core::url::absolute(
                        root_get(rc, ctx, "site.url").as_str().unwrap_or_default(),
                        &url,
                    ),
                    false => url,
                };
                return write_out(rc, out, &encode_uri(&url), false);
            }
            Some(v @ Value::Object(_)) => v.clone(),
            _ => current_value(rc, ctx),
        };
        let mut url = target
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| ghost_core::url::base("/"));
        if hash_flag(h, "absolute") && !url.starts_with("http") {
            let origin = root_get(rc, ctx, "site.url").as_str().unwrap_or_default().to_string();
            url = ghost_core::url::absolute(&origin, &ensure_leading_slash(&url));
        }
        write_out(rc, out, &encode_uri(&url), false)
    }
}

/// JavaScript's `encodeURI`: percent-encode everything outside the unreserved
/// and reserved sets, leaving an already-valid URL untouched. Ghost then
/// restores `[` and `]`.
fn encode_uri(s: &str) -> String {
    const KEEP: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\
                        -_.!~*'();/?:@&=+$,#[]%";
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if KEEP.contains(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// JavaScript's `encodeURIComponent` — the stricter set `{{encode}}` uses.
fn encode_uri_component(s: &str) -> String {
    const KEEP: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if KEEP.contains(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// img_url / asset
// ---------------------------------------------------------------------------

pub struct ImgUrl;
impl HelperDef for ImgUrl {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // {{img_url}} with no argument is a theme bug in Ghost; an argument
        // that resolves to null is a legitimately unset image.
        let Some(param) = h.param(0) else {
            tracing::warn!("attribute is required e.g. {{{{img_url feature_image}}}}");
            return Ok(());
        };
        let Some(path) = param.value().as_str() else {
            return Ok(());
        };
        if path.is_empty() {
            return Ok(());
        }

        let origin = root_get(rc, ctx, "site.url").as_str().unwrap_or_default().to_string();
        let origin = origin.trim_end_matches('/').to_string();
        let size = hash_str(h, "size");
        let format = hash_str(h, "format");
        let sizes = root_get(rc, ctx, "config.image_sizes");
        let absolute = hash_flag(h, "absolute");

        // external images are passed through untouched, except that Unsplash
        // honours the requested size and format via query parameters
        if !is_internal_image(path, &origin) {
            let url = if path.contains("images.unsplash.com") {
                unsplash_url(path, &sizes, size.as_deref(), format.as_deref())
            } else {
                path.to_string()
            };
            return write_out(rc, out, &url, false);
        }

        let mut url = image_with_size(path, &sizes, size.as_deref(), format.as_deref());
        // Three shapes reach this helper: image fields from serializers, which
        // already carry the base path; literal paths written by a theme, which
        // do not; and fields holding a full URL under our own origin.
        // `base_asset` resolves all three onto one browser-ready path without
        // ever applying the prefix twice.
        match url.strip_prefix(origin.as_str()).filter(|_| !origin.is_empty()) {
            Some(rest) => url = ensure_leading_slash(rest),
            None if !url.starts_with("http") => {
                url = ghost_core::url::base_asset(&ensure_leading_slash(&url))
            }
            None => {}
        }
        if absolute {
            url = ghost_core::url::absolute(&origin, &url);
        }
        write_out(rc, out, &url, false)
    }
}

fn ensure_leading_slash(s: &str) -> String {
    if s.starts_with('/') {
        s.to_string()
    } else {
        format!("/{s}")
    }
}

/// Ghost's `detectInternalImage`: anything relative, or absolute under our
/// own origin, is served by us.
fn is_internal_image(url: &str, origin: &str) -> bool {
    let is_absolute = url.starts_with("http://") || url.starts_with("https://");
    if is_absolute {
        return !origin.is_empty() && url.starts_with(origin);
    }
    // "//host/path" is protocol-relative, i.e. external
    !url.starts_with("//")
}

/// Look a named size up in `@config.image_sizes`, which we flatten into a
/// list of `{name, width, height}`.
fn find_image_size(sizes: &Value, name: &str) -> Option<(Option<u64>, Option<u64>)> {
    let entry = sizes
        .as_array()?
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(name))?;
    let width = entry.get("width").and_then(|w| w.as_u64()).filter(|w| *w > 0);
    let height = entry.get("height").and_then(|w| w.as_u64()).filter(|h| *h > 0);
    (width.is_some() || height.is_some()).then_some((width, height))
}

const TRANSFORMABLE_FORMATS: &[&str] = &["avif", "gif", "jpg", "jpeg", "png", "webp"];

/// Port of `getImageWithSize`: `/content/images/x.png` with size "m"
/// becomes `/content/images/size/w720/x.png`.
fn image_with_size(path: &str, sizes: &Value, size: Option<&str>, format: Option<&str>) -> String {
    const PREFIX: &str = "/content/images";
    let Some(size) = size else {
        return path.to_string();
    };
    let Some((width, height)) = find_image_size(sizes, size) else {
        return path.to_string();
    };
    let Some((before, after)) = path.split_once(PREFIX) else {
        return path.to_string();
    };
    if after.is_empty() {
        return path.to_string();
    }
    let dir = format!(
        "{}{}",
        width.map(|w| format!("w{w}")).unwrap_or_default(),
        height.map(|h| format!("h{h}")).unwrap_or_default()
    );
    let fmt = format
        .filter(|f| TRANSFORMABLE_FORMATS.contains(&f.to_ascii_lowercase().as_str()))
        .map(|f| format!("/format/{f}"))
        .unwrap_or_default();
    format!("{before}{PREFIX}/size/{dir}{fmt}{after}")
}

/// Unsplash serves its own transforms, so size and format become `w`/`h`/`fm`
/// query parameters rather than a path segment.
fn unsplash_url(path: &str, sizes: &Value, size: Option<&str>, format: Option<&str>) -> String {
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    let mut params: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    let mut set = |key: &str, value: String| {
        match params.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => params.push((key.to_string(), value)),
        }
    };

    if let Some(f) = format {
        let f = f.to_ascii_lowercase();
        let f = if f == "jpeg" { "jpg".to_string() } else { f };
        if ["avif", "gif", "jpg", "png", "webp"].contains(&f.as_str()) {
            set("fm", f);
        }
    }
    if let Some((width, height)) = size.and_then(|s| find_image_size(sizes, s)) {
        params.retain(|(k, _)| k != "w" && k != "h");
        if let Some(w) = width {
            params.push(("w".into(), w.to_string()));
        }
        if let Some(hgt) = height {
            params.push(("h".into(), hgt.to_string()));
        }
    }

    if params.is_empty() {
        return base.to_string();
    }
    let q: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{base}?{}", q.join("&"))
}

pub struct AssetHelper {
    pub hashes: Arc<HashMap<String, String>>,
    pub site_origin: String,
    /// Used when a path has no file of its own to hash, so the URL is still
    /// cache-busted the way Ghost's global asset hash does it.
    pub global_hash: String,
}
impl HelperDef for AssetHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(path) = h.param(0).and_then(|p| p.value().as_str()).filter(|p| !p.is_empty()) else {
            tracing::warn!("the {{{{asset}}}} helper must be passed a path");
            return Ok(());
        };

        // the favicon is served from its own route, not from the theme
        if path.trim_start_matches('/').starts_with("favicon.") {
            return write_out(rc, out, &ghost_core::url::base("/favicon.ico"), false);
        }

        // Themes write the path both ways — `{{asset "built/x.js"}}` and
        // `{{asset "/built/x.js"}}` — and Ghost takes either. Without this,
        // the second form becomes `/assets//built/x.js` and 404s.
        let path = path.trim_start_matches('/');

        // `public/...` and `asset...` are Ghost-owned paths served from the
        // site root; everything else is a theme asset under /assets/.
        let is_public = path.starts_with("public/");
        let is_theme_asset = !is_public && !path.starts_with("asset");

        let mut path = path.to_string();
        if hash_flag(h, "hasMinFile") {
            if let Some((stem, ext)) = path.rsplit_once('.') {
                path = format!("{stem}.min.{ext}");
            }
        }

        // an anchor has to stay after the cache-busting query
        let (path, anchor) = match path.split_once('#') {
            Some((p, a)) => (p.to_string(), format!("#{a}")),
            None => (path, String::new()),
        };

        let hash = self
            .hashes
            .get(&path)
            .map(|h| h[..10.min(h.len())].to_string())
            .unwrap_or_else(|| self.global_hash.clone());

        let mut url = ghost_core::url::base(&if is_theme_asset {
            format!("/assets/{path}?v={hash}{anchor}")
        } else {
            format!("/{path}?v={hash}{anchor}")
        });
        if hash_flag(h, "absolute") {
            url = ghost_core::url::absolute(&self.site_origin, &url);
        }
        write_out(rc, out, &url, false)
    }
}

// ---------------------------------------------------------------------------
// date
// ---------------------------------------------------------------------------

pub struct DateHelper;
impl HelperDef for DateHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // `{{date}}` with no argument formats the current scope's
        // published_at, falling back to "now" when there isn't one.
        let raw = match h.param(0).map(|p| p.value().clone()) {
            Some(v) => v,
            None => current_value(rc, ctx).get("published_at").cloned().unwrap_or(Value::Null),
        };
        let format = hash_str(h, "format").unwrap_or_else(|| "ll".into());
        let timezone = hash_str(h, "timezone")
            .or_else(|| root_get(rc, ctx, "site.timezone").as_str().map(str::to_string))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Etc/UTC".into());
        let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::Tz::UTC);

        let dt = raw
            .as_str()
            .and_then(parse_flexible)
            .unwrap_or_else(chrono::Utc::now);

        let formatted = if hash_flag(h, "timeago") {
            time_ago(&dt)
        } else {
            format_moment(&dt.with_timezone(&tz), &format)
        };
        write_out(rc, out, &formatted, false)
    }
}

fn parse_flexible(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // ISO (API shape) or Ghost DB shape
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z').split_once('.').map(|(a, _)| a).unwrap_or(s), "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|n| chrono::DateTime::from_naive_utc_and_offset(n, chrono::Utc))
        })
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| chrono::DateTime::from_naive_utc_and_offset(n, chrono::Utc))
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0).map(|t| chrono::DateTime::from_naive_utc_and_offset(t, chrono::Utc)))
        })
}

/// moment.js' localized format tokens, in the `en` locale Ghost defaults to.
/// Longest first so `LLLL` is not read as `LLL` + `L`.
const LOCALIZED_FORMATS: &[(&str, &str)] = &[
    ("LTS", "h:mm:ss A"),
    ("LT", "h:mm A"),
    ("LLLL", "dddd, MMMM D, YYYY h:mm A"),
    ("LLL", "MMMM D, YYYY h:mm A"),
    ("LL", "MMMM D, YYYY"),
    ("L", "MM/DD/YYYY"),
    ("llll", "ddd, MMM D, YYYY h:mm A"),
    ("lll", "MMM D, YYYY h:mm A"),
    ("ll", "MMM D, YYYY"),
    ("l", "M/D/YYYY"),
];

/// Expand `ll`, `LLLL`, `LT`, … into the underlying token string. Text inside
/// `[…]` is literal in moment and is copied through untouched.
fn expand_localized(format: &str) -> String {
    let mut out = String::with_capacity(format.len());
    let mut rest = format;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('[') {
            match stripped.find(']') {
                Some(end) => {
                    out.push_str(&rest[..end + 2]);
                    rest = &rest[end + 2..];
                }
                None => {
                    out.push_str(rest);
                    break;
                }
            }
            continue;
        }
        match LOCALIZED_FORMATS.iter().find(|(tok, _)| rest.starts_with(tok)) {
            Some((tok, pattern)) => {
                out.push_str(pattern);
                rest = &rest[tok.len()..];
            }
            None => {
                let ch = rest.chars().next().unwrap_or('\0');
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

/// moment.js format tokens, longest match first.
const MOMENT_TOKENS: &[&str] = &[
    "YYYY", "YY", "MMMM", "MMM", "MM", "Mo", "M", "DDDD", "DDDo", "DDD", "DD", "Do", "D", "dddd",
    "ddd", "dd", "do", "d", "Qo", "Q", "ww", "w", "HH", "H", "hh", "h", "kk", "k", "mm", "m", "ss",
    "s", "SSS", "SS", "S", "A", "a", "ZZ", "Z", "zz", "z", "X", "x", "E", "e", "gggg", "GGGG",
];

/// moment.js formatting → the equivalent chrono output.
fn format_moment(dt: &chrono::DateTime<chrono_tz::Tz>, format: &str) -> String {
    use chrono::{Datelike, Timelike};

    let expanded = expand_localized(format);
    let mut out = String::with_capacity(expanded.len());
    let mut rest = expanded.as_str();

    while !rest.is_empty() {
        // [literal text] is emitted verbatim, brackets stripped
        if let Some(stripped) = rest.strip_prefix('[') {
            match stripped.find(']') {
                Some(end) => {
                    out.push_str(&stripped[..end]);
                    rest = &stripped[end + 1..];
                }
                None => {
                    out.push_str(stripped);
                    break;
                }
            }
            continue;
        }

        let Some(tok) = MOMENT_TOKENS.iter().find(|t| rest.starts_with(**t)) else {
            let ch = rest.chars().next().unwrap_or('\0');
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
            continue;
        };

        let hour12 = if dt.hour() % 12 == 0 { 12 } else { dt.hour() % 12 };
        let rep = match *tok {
            "YYYY" => dt.format("%Y").to_string(),
            "YY" => dt.format("%y").to_string(),
            "gggg" | "GGGG" => dt.format("%Y").to_string(),
            "MMMM" => dt.format("%B").to_string(),
            "MMM" => dt.format("%b").to_string(),
            "MM" => format!("{:02}", dt.month()),
            "Mo" => ordinal(dt.month() as i64),
            "M" => dt.month().to_string(),
            "DDDD" => format!("{:03}", dt.ordinal()),
            "DDDo" => ordinal(dt.ordinal() as i64),
            "DDD" => dt.ordinal().to_string(),
            "DD" => format!("{:02}", dt.day()),
            "Do" => ordinal(dt.day() as i64),
            "D" => dt.day().to_string(),
            "dddd" => dt.format("%A").to_string(),
            "ddd" => dt.format("%a").to_string(),
            "dd" => dt.format("%a").to_string().chars().take(2).collect(),
            "do" => ordinal(dt.weekday().num_days_from_sunday() as i64),
            "d" | "e" => dt.weekday().num_days_from_sunday().to_string(),
            "E" => dt.weekday().number_from_monday().to_string(),
            "Qo" => ordinal(((dt.month() + 2) / 3) as i64),
            "Q" => ((dt.month() + 2) / 3).to_string(),
            "ww" => dt.format("%W").to_string(),
            "w" => dt.iso_week().week().to_string(),
            "HH" => format!("{:02}", dt.hour()),
            "H" => dt.hour().to_string(),
            "hh" => format!("{hour12:02}"),
            "h" => hour12.to_string(),
            // k is the 1-24 hour clock
            "kk" => format!("{:02}", if dt.hour() == 0 { 24 } else { dt.hour() }),
            "k" => if dt.hour() == 0 { 24 } else { dt.hour() }.to_string(),
            "mm" => format!("{:02}", dt.minute()),
            "m" => dt.minute().to_string(),
            "ss" => format!("{:02}", dt.second()),
            "s" => dt.second().to_string(),
            "SSS" => format!("{:03}", dt.timestamp_subsec_millis()),
            "SS" => format!("{:02}", dt.timestamp_subsec_millis() / 10),
            "S" => (dt.timestamp_subsec_millis() / 100).to_string(),
            "A" => if dt.hour() < 12 { "AM" } else { "PM" }.to_string(),
            "a" => if dt.hour() < 12 { "am" } else { "pm" }.to_string(),
            "ZZ" => dt.format("%z").to_string(),
            "Z" => dt.format("%:z").to_string(),
            "zz" | "z" => dt.format("%Z").to_string(),
            "X" => dt.timestamp().to_string(),
            "x" => dt.timestamp_millis().to_string(),
            _ => tok.to_string(),
        };
        out.push_str(&rep);
        rest = &rest[tok.len()..];
    }
    out
}

/// English ordinal suffix, as moment's `Do`/`Mo` tokens produce.
fn ordinal(n: i64) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn time_ago(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - dt).num_seconds().max(0);
    match secs {
        s if s < 45 => "a few seconds ago".into(),
        s if s < 90 => "a minute ago".into(),
        s if s < 3600 => format!("{} minutes ago", s / 60),
        s if s < 7200 => "an hour ago".into(),
        s if s < 86400 => format!("{} hours ago", s / 3600),
        s if s < 172800 => "a day ago".into(),
        s if s < 2592000 => format!("{} days ago", s / 86400),
        s if s < 5184000 => "a month ago".into(),
        s if s < 31536000 => format!("{} months ago", s / 2592000),
        s if s < 63072000 => "a year ago".into(),
        s => format!("{} years ago", s / 31536000),
    }
}

// ---------------------------------------------------------------------------
// is / has / match
// ---------------------------------------------------------------------------

pub struct Is;
impl HelperDef for Is {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let ctx_list = root_get(rc, ctx, "context");
        let current: Vec<&str> = ctx_list
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str()).collect())
            .unwrap_or_default();

        // `{{#is "index, paged"}}` — one string holding a comma-separated list
        let mut matched = false;
        for p in h.params() {
            let Some(expr) = p.value().as_str() else {
                tracing::warn!("invalid or no attribute given to is helper");
                continue;
            };
            if expr.split(',').map(str::trim).any(|c| current.contains(&c)) {
                matched = true;
                break;
            }
        }
        if matched {
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
        } else if let Some(t) = h.inverse() {
            t.render(r, ctx, rc, out)?;
        }
        Ok(())
    }
}

pub struct Has;
impl HelperDef for Has {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        // Ghost only reads these attributes; anything else is ignored
        const VALID_ATTRS: &[&str] =
            &["tag", "author", "slug", "visibility", "id", "number", "index", "any", "all"];

        let attrs: Vec<(&str, String)> = h
            .hash()
            .iter()
            .filter(|(k, _)| VALID_ATTRS.contains(&k.to_string().as_str()))
            .filter_map(|(k, v)| v.value().as_str().map(|s| (*k, s.to_string())))
            .collect();

        if attrs.is_empty() {
            tracing::warn!("invalid or no attribute given to has helper");
            return Ok(());
        }

        // any one attribute matching is enough
        let matched = attrs.iter().any(|(key, expr)| match *key {
            "tag" => resource_list_matches(&this, "tags", expr, false),
            "author" => resource_list_matches(&this, "authors", expr, true),
            "number" => block_local(rc, "number").is_some_and(|n| integer_matches(expr, n)),
            "index" => block_local(rc, "index").is_some_and(|n| integer_matches(expr, n)),
            "visibility" => string_matches_ci(expr, this.get("visibility")),
            "slug" => string_matches_ci(expr, this.get("slug")),
            "id" => string_matches_ci(expr, this.get("id")),
            "any" => property_list(expr, &this, rc, ctx).iter().any(|p| *p),
            "all" => {
                let checks = property_list(expr, &this, rc, ctx);
                !checks.is_empty() && checks.iter().all(|p| *p)
            }
            _ => false,
        });

        if matched {
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
        } else if let Some(t) = h.inverse() {
            t.render(r, ctx, rc, out)?;
        }
        Ok(())
    }
}

/// `tag="a, b"` / `author="count:>2"`. Tags are matched by name as an
/// anchored case-insensitive pattern; authors by lower-cased name.
fn resource_list_matches(this: &Value, field: &str, expr: &str, authors: bool) -> bool {
    let items = this.get(field).and_then(|f| f.as_array());
    if expr.contains("count:") {
        let Some(items) = items else { return false };
        return count_matches(expr, items.len() as i64);
    }
    let names: Vec<String> = items
        .map(|a| {
            a.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                .map(|n| n.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let _ = authors;
    expr.split(',')
        .map(|v| v.trim().to_lowercase())
        .any(|wanted| names.iter().any(|n| *n == wanted))
}

/// `count:3`, `count:>3`, `count:<3`.
fn count_matches(expr: &str, actual: i64) -> bool {
    let Some(rest) = expr.split("count:").nth(1) else { return false };
    let rest = rest.trim();
    if let Some(n) = rest.strip_prefix('>') {
        n.trim().parse::<i64>().map(|n| n < actual).unwrap_or(false)
    } else if let Some(n) = rest.strip_prefix('<') {
        n.trim().parse::<i64>().map(|n| n > actual).unwrap_or(false)
    } else {
        rest.parse::<i64>().map(|n| n == actual).unwrap_or(false)
    }
}

/// `number="nth:3"` is "every third", not "the third"; a plain list is an
/// exact match on any of its values.
fn integer_matches(expr: &str, value: i64) -> bool {
    if let Some(nth) = expr.strip_prefix("nth:") {
        return match nth.trim().parse::<i64>() {
            Ok(n) if n != 0 => value % n == 0,
            _ => false,
        };
    }
    expr.split(',').filter_map(|v| v.trim().parse::<i64>().ok()).any(|v| v == value)
}

fn string_matches_ci(expr: &str, actual: Option<&Value>) -> bool {
    let Some(actual) = actual.and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
        return false;
    };
    !expr.is_empty() && expr.eq_ignore_ascii_case(actual)
}

/// `any="a, b"` / `all="a, @site.logo"` — each entry is a property that must
/// be present and non-empty, on the current scope or (with `@`) on the root.
fn property_list(expr: &str, this: &Value, rc: &RenderContext<'_, '_>, ctx: &Context) -> Vec<bool> {
    expr.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|prop| {
            let value = match prop.strip_prefix('@') {
                // only site/config/labs are reachable this way in Ghost.
                // Preprocessing has already rewritten `@site` to `@root.site`
                // inside the tag, so strip that back off before re-rooting.
                Some(path) => root_get(rc, ctx, path.strip_prefix("root.").unwrap_or(path)),
                None => walk_path(this, prop),
            };
            !is_empty_value(&value)
        })
        .collect()
}

/// lodash `_.get` over a dotted path.
fn walk_path(v: &Value, path: &str) -> Value {
    let mut cur = v;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = match cur.get(seg) {
            Some(next) => next,
            None => return Value::Null,
        };
    }
    cur.clone()
}

/// lodash `_.isEmpty` as `{{#has any=…}}` relies on it: numbers and booleans
/// are always "empty" to lodash, strings/arrays/objects only when they have
/// no entries.
fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
    }
}

/// Read a loop variable (`@index`, `@number`) from the innermost block.
fn block_local(rc: &RenderContext<'_, '_>, name: &str) -> Option<i64> {
    rc.block().and_then(|b| b.get_local_var(name)).and_then(|v| v.as_i64())
}

fn value_to_plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `{{#match a b}}` (equality), `{{#match a "op" b}}` (explicit operator) or
/// `{{#match a}}` (truthiness, like `{{#if}}`). Also usable inline, where it
/// renders "true"/"false".
pub struct Match;

impl Match {
    fn evaluate(h: &Helper<'_>) -> Option<bool> {
        let params = h.params();
        Some(match params.len() {
            0 => {
                tracing::warn!("invalid or no attribute given to match helper");
                return None;
            }
            1 => is_truthy(params[0].value(), hash_flag(h, "includeZero")),
            2 => compare(params[0].value(), "=", params[1].value()),
            3 => compare(
                params[0].value(),
                params[1].value().as_str().unwrap_or("="),
                params[2].value(),
            ),
            _ => {
                tracing::warn!("invalid or no attribute given to match helper");
                return None;
            }
        })
    }
}

impl HelperDef for Match {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(matched) = Self::evaluate(h) else {
            return Ok(());
        };
        // inline use renders the outcome itself
        if h.template().is_none() && h.inverse().is_none() {
            return write_out(rc, out, if matched { "true" } else { "false" }, false);
        }
        if matched {
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
        } else if let Some(t) = h.inverse() {
            t.render(r, ctx, rc, out)?;
        }
        Ok(())
    }
}

/// `{{#if}}`-style truthiness: null/false/""/0/[] are falsey, with 0 opt-in
/// via `includeZero`.
fn is_truthy(v: &Value, include_zero: bool) -> bool {
    match v {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => include_zero || n.as_f64().is_some_and(|f| f != 0.0),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

/// The operators `{{#match}}` understands. Comparisons are type-aware:
/// numbers compare numerically, everything else as JS would.
fn compare(a: &Value, op: &str, b: &Value) -> bool {
    let ord = json_cmp(a, b);
    match op {
        "!=" => !json_eq(a, b),
        "<" => ord == Some(std::cmp::Ordering::Less),
        ">" => ord == Some(std::cmp::Ordering::Greater),
        "<=" => matches!(ord, Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)),
        ">=" => matches!(ord, Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)),
        "~" => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => x.contains(y),
            _ => false,
        },
        "~^" => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => x.starts_with(y),
            _ => false,
        },
        "~$" => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => x.ends_with(y),
            _ => false,
        },
        _ => json_eq(a, b),
    }
}

/// JavaScript's `===`, which is case- and type-sensitive.
fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// `None` when the values are not orderable against each other, which is
/// how JS relational operators behave with NaN.
fn json_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return x.partial_cmp(&y);
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// tags / authors lists
// ---------------------------------------------------------------------------

pub struct TagsList {
    pub is_authors: bool,
}
impl HelperDef for TagsList {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let field = if self.is_authors { "authors" } else { "tags" };
        // only the literal string "false" turns autolinking off
        let autolink = hash_str(h, "autolink").as_deref() != Some("false");
        let separator = hash_str(h, "separator").unwrap_or_else(|| ", ".into());
        let prefix = hash_str(h, "prefix").unwrap_or_default();
        let suffix = hash_str(h, "suffix").unwrap_or_default();
        let visibility = hash_str(h, "visibility");

        let items = this.get(field).and_then(|f| f.as_array()).cloned().unwrap_or_default();
        // visibility is applied first, then the from/to/limit window
        let rendered: Vec<String> = items
            .iter()
            .filter(|t| visibility_keeps(t, visibility.as_deref()))
            .map(|t| {
                let name = escape_expression(t.get("name").and_then(|n| n.as_str()).unwrap_or_default());
                match (autolink, t.get("url").and_then(|u| u.as_str())) {
                    (true, Some(url)) => format!(r#"<a href="{}">{name}</a>"#, encode_uri(url)),
                    _ => name,
                }
            })
            .collect();

        if rendered.is_empty() {
            return Ok(());
        }

        // `from` is 1-indexed; `to` defaults to from+limit, then to the end
        let len = rendered.len();
        let from = hash_count(h, "from").unwrap_or(1).saturating_sub(1);
        let to = hash_count(h, "to")
            .or_else(|| hash_count(h, "limit").map(|l| l + from))
            .unwrap_or(len)
            .min(len);
        let window = if from >= to { &[][..] } else { &rendered[from..to] };

        if window.is_empty() {
            return Ok(());
        }
        write_out(rc, out, &format!("{prefix}{}{suffix}", window.join(&separator)), false)
    }
}

// ---------------------------------------------------------------------------
// navigation / pagination
// ---------------------------------------------------------------------------

/// `{{navigation}}` / `{{navigation type="secondary"}}`.
pub struct Navigation;
impl HelperDef for Navigation {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let secondary = hash_str(h, "type").as_deref() == Some("secondary");
        let key = if secondary { "site.secondary_navigation" } else { "site.navigation" };
        let nav = root_get(rc, ctx, key);
        let Some(items) = nav.as_array().cloned() else {
            tracing::warn!("navigation data is not an object");
            return Ok(());
        };
        // {{navigation}} is a no-op when nothing is configured
        if items.is_empty() {
            return Ok(());
        }

        let relative = root_get(rc, ctx, "relativeUrl").as_str().unwrap_or("/").to_string();
        let marked: Vec<Value> = items
            .iter()
            .map(|item| {
                let url = item.get("url").and_then(|u| u.as_str()).unwrap_or_default();
                let label = item.get("label").and_then(|l| l.as_str()).unwrap_or_default();
                json!({
                    // `build_site` has already put the base path on these
                    "url": url,
                    "label": label,
                    // Ghost slugifies the label, not the URL
                    "slug": ghost_core::slug::safe_string(label),
                    // ...so the current-page test has to take it back off:
                    // `relative` is the request path, which arrives stripped
                    "current": is_current_nav(&relative, ghost_core::url::strip(url)),
                })
            })
            .collect();

        let mut data = Map::new();
        data.insert("navigation".into(), Value::Array(marked));
        data.insert("isSecondary".into(), Value::Bool(secondary));
        // the helper forwards its own attributes into the partial's scope
        for (k, v) in h.hash() {
            if *k != "type" {
                data.insert((*k).to_string(), v.value().clone());
            }
        }

        let mut block = handlebars::BlockContext::new();
        block.set_base_value(Value::Object(data));
        rc.push_block(block);
        let template = if secondary && r.has_template("secondary_navigation") {
            "secondary_navigation"
        } else {
            "navigation"
        };
        let rendered = r.get_template(template).map(|tpl| tpl.render(r, ctx, rc, out));
        rc.pop_block();
        rendered.transpose()?;
        Ok(())
    }
}

/// Ghost compares nav URLs to the current one after stripping trailing
/// slashes — an exact match only, not a prefix match.
fn is_current_nav(relative: &str, url: &str) -> bool {
    url.trim_end_matches('/') == relative.trim_end_matches('/')
}

pub struct Pagination;
impl HelperDef for Pagination {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let Some(Value::Object(p)) = this.get("pagination").cloned() else {
            tracing::warn!("the {{{{pagination}}}} helper was used outside of a paginated context");
            return Ok(());
        };
        for required in ["page", "pages", "total", "limit"] {
            if !p.contains_key(required) {
                tracing::warn!("all values must be defined for page, pages, limit and total");
                return Ok(());
            }
        }

        // the partial sees the pagination properties at the top level as well
        // as the object itself, plus anything passed to the helper
        let mut data = match this {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        for (k, v) in &p {
            data.insert(k.clone(), v.clone());
        }
        data.insert("pagination".into(), Value::Object(p.clone()));
        for (k, v) in h.hash() {
            data.insert((*k).to_string(), v.value().clone());
        }

        // a ready-made page list, for themes that render numbered pagination
        let page = p.get("page").and_then(|v| v.as_i64()).unwrap_or(1);
        let pages = p.get("pages").and_then(|v| v.as_i64()).unwrap_or(1);
        let base = paginated_base(root_get(rc, ctx, "relativeUrl").as_str().unwrap_or("/"));
        let list: Vec<Value> = (1..=pages)
            .map(|i| json!({ "page": i, "url": paginated_url(&base, i), "current": i == page }))
            .collect();
        data.insert("pages_list".into(), Value::Array(list));

        let mut block = handlebars::BlockContext::new();
        block.set_base_value(Value::Object(data));
        rc.push_block(block);
        let rendered = r.get_template("pagination").map(|tpl| tpl.render(r, ctx, rc, out));
        rc.pop_block();
        rendered.transpose()?;
        Ok(())
    }
}

/// Strip any `/page/N/` suffix off the current URL, leaving the channel's
/// own base (`""` for the index channel, `/tag/foo` for a tag archive).
fn paginated_base(relative_url: &str) -> String {
    match relative_url.find("/page/") {
        Some(i) => relative_url[..i].to_string(),
        None => relative_url.trim_end_matches('/').to_string(),
    }
}

/// `base` comes from the request path, which arrives with the install's base
/// path stripped, so the result is put back into browser terms here — both
/// callers (`{{pagination}}` and `{{page_url}}`) write it straight out.
fn paginated_url(base: &str, page: i64) -> String {
    let tail = if page > 1 { format!("/page/{page}/") } else { "/".to_string() };
    let path = if base.is_empty() { tail } else { format!("{base}{tail}") };
    ghost_core::url::base(&path)
}

// ---------------------------------------------------------------------------
// reading_time / page_url / encode / json / plural / t
// ---------------------------------------------------------------------------

pub struct ReadingTime;
impl HelperDef for ReadingTime {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        // reading time is only meaningful on a post
        if !is_post(&this) {
            return Ok(());
        }
        let minutes = this.get("reading_time").and_then(|v| v.as_i64()).unwrap_or(0);

        // the labels are overridable so themes can translate them
        let text = if minutes < 1 {
            hash_str(h, "seconds").unwrap_or_else(|| "< 1 min read".into())
        } else if minutes == 1 {
            hash_str(h, "minute").unwrap_or_else(|| "1 min read".into())
        } else {
            hash_str(h, "minutes").unwrap_or_else(|| "% min read".into())
        };
        write_out(rc, out, &text.replace('%', &minutes.to_string()), false)
    }
}

pub struct PageUrl;
impl HelperDef for PageUrl {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let relative = root_get(rc, ctx, "relativeUrl").as_str().unwrap_or("/").to_string();
        let pagination = root_get(rc, ctx, "pagination");
        let base = paginated_base(&relative);

        // the argument is a page number, or "next"/"prev" to read it off the
        // pagination object
        let arg = h.param(0).map(|p| p.value().clone()).unwrap_or(json!(1));
        let page: Option<i64> = match &arg {
            Value::String(s) if s == "next" => pagination.get("next").and_then(|v| v.as_i64()),
            Value::String(s) if s == "prev" => pagination.get("prev").and_then(|v| v.as_i64()),
            Value::String(s) => s.trim().parse().ok(),
            other => other.as_i64(),
        };
        let Some(page) = page else {
            return Ok(());
        };
        write_out(rc, out, &paginated_url(&base, page), false)
    }
}

pub struct Encode;
impl HelperDef for Encode {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let s = h.param(0).map(|p| value_to_plain(p.value())).unwrap_or_default();
        write_out(rc, out, &encode_uri_component(&s), false)
    }
}

pub struct JsonHelper;
impl HelperDef for JsonHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // `{{json foo}}` serialises the value; `{{json a=1 b=2}}` the hash
        let payload = if h.hash().is_empty() {
            h.param(0).map(|p| p.value().clone()).unwrap_or_else(|| current_value(rc, ctx))
        } else {
            Value::Object(h.hash().iter().map(|(k, v)| ((*k).to_string(), v.value().clone())).collect())
        };
        // the output lands inside <script>, so the characters that could
        // close the tag early are escaped
        let serialized = payload
            .to_string()
            .replace('<', "\\u003C")
            .replace('>', "\\u003E")
            .replace('&', "\\u0026")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029");
        write_out(rc, out, &serialized, false)
    }
}

/// `{{plural total empty='No posts' singular='% post' plural='% posts'}}`
pub struct Plural;
impl HelperDef for Plural {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let (Some(empty), Some(singular), Some(plural)) =
            (hash_str(h, "empty"), hash_str(h, "singular"), hash_str(h, "plural"))
        else {
            tracing::warn!("all values must be defined for empty, singular and plural");
            return Ok(());
        };

        let number = h
            .param(0)
            .map(|p| p.value())
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
            .unwrap_or(0);

        let text = match number {
            0 => empty,
            1 => singular,
            _ => plural,
        };
        write_out(rc, out, &text.replace('%', &number.to_string()), false)
    }
}

/// `{{t "key" binding=value}}`. Ghost returns a plain string here (not a
/// SafeString) so that `{{t}}` escapes and `{{{t}}}` does not, and so that it
/// composes as a subexpression: `{{tags prefix=(t " on ")}}`.
pub struct Translate {
    pub translations: Arc<HashMap<String, String>>,
}
impl HelperDef for Translate {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let key = h.param(0).and_then(|p| p.value().as_str()).unwrap_or_default().to_string();
        if key.is_empty() {
            return Ok(handlebars::ScopedJson::Derived(json!("")));
        }
        // Ghost's locale files list every string with an empty value until
        // it is actually translated, and fall back to the key itself.
        let mut s = self
            .translations
            .get(&key)
            .filter(|t| !t.is_empty())
            .cloned()
            .unwrap_or(key);
        for (k, v) in h.hash() {
            s = s.replace(&format!("{{{}}}", k), &value_to_plain(v.value()));
        }
        Ok(handlebars::ScopedJson::Derived(Value::String(s)))
    }
}

// ---------------------------------------------------------------------------
// meta helpers (title/description) + class helpers
// ---------------------------------------------------------------------------

pub struct MetaTitle;
impl HelperDef for MetaTitle {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let site_title = root_get(rc, ctx, "site.title").as_str().unwrap_or_default().to_string();
        let ctx_list = root_get(rc, ctx, "context");
        let is_home = ctx_list.as_array().map(|a| a.iter().any(|c| c.as_str() == Some("home"))).unwrap_or(false);
        let t = if is_home {
            let desc = root_get(rc, ctx, "site.description").as_str().unwrap_or_default().to_string();
            let site_meta = root_get(rc, ctx, "site.meta_title").as_str().unwrap_or_default().to_string();
            if !site_meta.is_empty() { site_meta.to_string() } else if !desc.is_empty() { format!("{site_title} - {desc}") } else { site_title.clone() }
        } else {
            let own = this
                .get("meta_title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| this.get("name").and_then(|v| v.as_str()).map(str::to_string))
                .or_else(|| this.get("title").and_then(|v| v.as_str()).map(str::to_string));
            match own {
                Some(o) => format!("{o} - {site_title}"),
                None => site_title,
            }
        };
        write_out(rc, out, &t, true)
    }
}

pub struct MetaDescription;
impl HelperDef for MetaDescription {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let ctx_list = root_get(rc, ctx, "context");
        let is_home = ctx_list.as_array().map(|a| a.iter().any(|c| c.as_str() == Some("home"))).unwrap_or(false);
        let d = if is_home {
            let meta = root_get(rc, ctx, "site.meta_description");
            let desc = root_get(rc, ctx, "site.description");
            meta.as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| desc.as_str().filter(|s| !s.is_empty()))
                .unwrap_or_default()
                .to_string()
        } else {
            this.get("meta_description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| this.get("custom_excerpt").and_then(|v| v.as_str()))
                .or_else(|| this.get("description").and_then(|v| v.as_str()))
                .or_else(|| this.get("excerpt").and_then(|v| v.as_str()))
                .or_else(|| this.get("bio").and_then(|v| v.as_str()))
                .unwrap_or_default()
                .to_string()
        };
        write_out(rc, out, &d, true)
    }
}

pub struct BodyClass;
impl HelperDef for BodyClass {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let ctx_list: Vec<String> = root_get(rc, ctx, "context")
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str()).map(str::to_string).collect())
            .unwrap_or_default();
        let has = |name: &str| ctx_list.iter().any(|c| c == name);

        let this = current_value(rc, ctx);
        let page = this.get("page").filter(|v| v.is_object());
        let obj = this.get("post").filter(|v| v.is_object()).or(page);
        let slug_of = |v: Option<&Value>| {
            v.and_then(|o| o.get("slug")).and_then(|s| s.as_str()).unwrap_or_default().to_string()
        };

        // exactly one template class, chosen by the first matching context
        let mut classes: Vec<String> = Vec::new();
        if has("home") {
            classes.push("home-template".into());
        } else if has("post") && obj.is_some() && page.is_none() {
            classes.push("post-template".into());
        } else if has("page") && page.is_some() {
            classes.push("page-template".into());
            classes.push(format!("page-{}", slug_of(page)));
        } else if has("tag") && this.get("tag").is_some_and(|t| t.is_object()) {
            classes.push("tag-template".into());
            classes.push(format!("tag-{}", slug_of(this.get("tag"))));
        } else if has("author") && this.get("author").is_some_and(|a| a.is_object()) {
            classes.push("author-template".into());
            classes.push(format!("author-{}", slug_of(this.get("author"))));
        } else if has("private") {
            classes.push("private-template".into());
        }

        // plus one class per tag on the post or page being shown
        if let Some(tags) = obj.and_then(|o| o.get("tags")).and_then(|t| t.as_array()) {
            for tag in tags {
                if let Some(slug) = tag.get("slug").and_then(|s| s.as_str()) {
                    classes.push(format!("tag-{slug}"));
                }
            }
        }

        if has("paged") {
            classes.push("paged".into());
        }
        write_out(rc, out, classes.join(" ").trim(), false)
    }
}

pub struct PostClass;
impl HelperDef for PostClass {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        // usable both at the top level of post.hbs (where the post sits under
        // `post`) and inside a {{#foreach posts}} block
        let post = this.get("post").filter(|v| v.is_object());
        let field = |name: &str| post.and_then(|p| p.get(name)).or_else(|| this.get(name));
        let truthy = |v: Option<&Value>| !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
            && v != Some(&Value::String(String::new()));

        let mut classes = vec!["post".to_string()];
        if let Some(tags) = field("tags").and_then(|t| t.as_array()) {
            for t in tags {
                if let Some(slug) = t.get("slug").and_then(|s| s.as_str()) {
                    classes.push(format!("tag-{slug}"));
                }
            }
        }
        if truthy(field("featured")) {
            classes.push("featured".into());
        }
        if !truthy(field("feature_image")) {
            classes.push("no-image".into());
        }
        if truthy(field("page")) || field("type").and_then(|t| t.as_str()) == Some("page") {
            classes.push("page".into());
        }
        write_out(rc, out, classes.join(" ").trim(), false)
    }
}

// ---------------------------------------------------------------------------
// ghost_head / ghost_foot
// ---------------------------------------------------------------------------

/// Render `key="value"` pairs as `data-*` attributes, skipping empties —
/// Ghost's `getDataAttributes`.
fn data_attributes(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!(r#"data-{}="{}""#, k, html_escape(v)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The Portal script plus the engine-rendered CTA styles.
///
/// Portal is what turns every `data-portal` attribute and `#/portal/*` href a
/// theme emits into a working signup/signin/account modal; without this tag
/// those links are inert anchors. It reads site config from
/// `/members/api/site/` and the Content API, and authenticates against
/// `/members/api/*`.
///
/// Ghost skips Portal entirely only when members, donations *and*
/// recommendations are all off, since each of the three needs it.
fn members_helper(site: &serde_json::Value, origin: &str, frontend_key: &str) -> String {
    let flag = |k: &str| match site.get(k) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    };
    if !flag("members_enabled") && !flag("donations_enabled") && !flag("recommendations_enabled") {
        return String::new();
    }

    let locale = site.get("locale").and_then(|l| l.as_str()).unwrap_or("en");
    let attrs = data_attributes(&[
        ("i18n", "true".into()),
        ("ghost", origin.to_string()),
        ("key", frontend_key.to_string()),
        ("api", format!("{origin}/ghost/api/content/")),
        ("locale", locale.to_string()),
    ]);
    let mut out = format!(
        r#"<script defer src="{}" {attrs} crossorigin="anonymous"></script>"#,
        crate::apps::portal_script_url()
    );
    out.push_str(&format!(
        r#"<style id="gh-members-styles">{}</style>"#,
        crate::apps::MEMBERS_CTA_STYLES
    ));
    // Stripe's SDK only matters once a paid tier exists; Portal loads its
    // checkout flows against it.
    if flag("paid_members_enabled") {
        out.push_str(r#"<script async src="https://js.stripe.com/v3/"></script>"#);
    }
    out
}

/// The Sodo Search script — the overlay behind every `data-ghost-search`
/// button and `#/search` link. It builds a client-side FlexSearch index from
/// `/ghost/api/content/search-index/{posts,authors,tags}/`, so the key here
/// must be a Content API key.
///
/// `admin_origin` is an origin, not a path: Sodo Search appends
/// `/ghost/api/content/...` to it itself, so passing this port's `admin_url()`
/// (which already ends in `/ghost`) would produce a doubled `/ghost/ghost/`.
fn search_helper(site: &serde_json::Value, admin_origin: &str, frontend_key: &str) -> String {
    let locale = site.get("locale").and_then(|l| l.as_str()).unwrap_or("en");
    let attrs = data_attributes(&[
        ("key", frontend_key.to_string()),
        ("styles", crate::apps::sodo_search_styles_url()),
        ("sodo-search", admin_origin.to_string()),
        ("locale", locale.to_string()),
    ]);
    format!(
        r#"<script defer src="{}" {attrs} crossorigin="anonymous"></script>"#,
        crate::apps::sodo_search_script_url()
    )
}

pub struct GhostHead;
impl HelperDef for GhostHead {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let _ = (h, ctx);
        let site = root_get(rc, ctx, "site");
        // The routed post/page/tag/author, not the layout's block context —
        // this is what lets a post's own settings override the site's.
        let this = current_resource(rc, ctx);
        let origin = site.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        let relative = root_get(rc, ctx, "relativeUrl").as_str().unwrap_or("/").to_string();
        let mut tags: Vec<String> = Vec::new();

        // charset + viewport (Ghost relies on themes; but ghost_head emits meta description etc.)
        let title = crate::helpers::meta_title_text(rc, ctx);
        tags.push(format!(r#"<title>{title}</title>"#));

        let desc = crate::helpers::meta_description_text(rc, ctx);
        if !desc.is_empty() {
            tags.push(format!(r#"<meta name="description" content="{}" />"#, html_escape(&desc)));
        }

        // canonical — a post may point at the original it was reposted from
        let canonical = this
            .get("canonical_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}{}", origin.trim_end_matches('/'), relative));
        tags.push(format!(r#"<link rel="canonical" href="{}" />"#, html_escape(&canonical)));

        // generator
        tags.push(r#"<meta name="generator" content="Ghost 1.0" />"#.into());

        // RSS alternate (site feed)
        tags.push(format!(
            r#"<link rel="alternate" type="application/rss+xml" title="{}" href="{}/rss/" />"#,
            html_escape(site.get("title").and_then(|t| t.as_str()).unwrap_or("")),
            origin.trim_end_matches('/')
        ));

        // og + twitter — Ghost's precedence: card-specific setting →
        // meta title/description → base
        let site_str = |key: &str| site.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        // The resource's own card settings come first: a post that sets its
        // own og:title must not be overruled by the site's.
        let this_str = |key: &str| this.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let og_title = this_str("og_title").or_else(|| site_str("og_title")).unwrap_or(&title).to_string();
        let og_desc = this_str("og_description").or_else(|| site_str("og_description")).unwrap_or(&desc).to_string();
        let twitter_title = this_str("twitter_title").or_else(|| site_str("twitter_title")).unwrap_or(&og_title).to_string();
        let twitter_desc = this_str("twitter_description").or_else(|| site_str("twitter_description")).unwrap_or(&og_desc).to_string();
        tags.push(format!(r#"<meta property="og:site_name" content="{}" />"#, html_escape(site.get("title").and_then(|t| t.as_str()).unwrap_or(""))));
        // Ghost marks a post as an article and a listing page as a website.
        let is_article = root_get(rc, ctx, "context")
            .as_array()
            .map(|a| a.iter().any(|c| matches!(c.as_str(), Some("post") | Some("page"))))
            .unwrap_or(false);
        tags.push(format!(
            r#"<meta property="og:type" content="{}" />"#,
            if is_article { "article" } else { "website" }
        ));
        tags.push(format!(r#"<meta property="og:title" content="{}" />"#, html_escape(&og_title)));
        if !og_desc.is_empty() {
            tags.push(format!(r#"<meta property="og:description" content="{}" />"#, html_escape(&og_desc)));
        }
        tags.push(format!(r#"<meta property="og:url" content="{}{}" />"#, origin.trim_end_matches('/'), relative));
        let og_image = this_str("og_image")
            .or_else(|| this_str("feature_image"))
            .or_else(|| site_str("og_image"))
            .or_else(|| site_str("cover_image"));
        let twitter_image = this_str("twitter_image").or_else(|| site_str("twitter_image")).or(og_image);
        // these come out of `site`/the resource already carrying the base
        // path, so the absolute form is built against the origin
        if let Some(img) = og_image {
            let img = ghost_core::url::absolute(&origin, img);
            tags.push(format!(r#"<meta property="og:image" content="{}" />"#, html_escape(&img)));
        }
        if let Some(img) = twitter_image {
            let img = ghost_core::url::absolute(&origin, img);
            tags.push(format!(r#"<meta name="twitter:card" content="summary_large_image" />"#));
            tags.push(format!(r#"<meta name="twitter:image" content="{}" />"#, html_escape(&img)));
        } else {
            tags.push(r#"<meta name="twitter:card" content="summary" />"#.into());
        }
        tags.push(format!(r#"<meta name="twitter:title" content="{}" />"#, html_escape(&twitter_title)));
        if !twitter_desc.is_empty() {
            tags.push(format!(r#"<meta name="twitter:description" content="{}" />"#, html_escape(&twitter_desc)));
        }

        // Koenig card assets. A post can contain a gallery, a toggle or an
        // audio player whatever theme is active, so the styles that go with
        // them come from here rather than from the theme — the same split
        // Ghost makes. A theme turns them off with `card_assets: false` in
        // its package.json, usually because it styles the cards itself.
        let config = root_get(rc, ctx, "config");
        if config.get("card_assets").and_then(Value::as_bool).unwrap_or(true) {
            let version = config.get("card_assets_version").and_then(Value::as_str).unwrap_or("1");
            let js = ghost_core::url::base(&format!("/public/cards.min.js?v={version}"));
            let css = ghost_core::url::base(&format!("/public/cards.min.css?v={version}"));
            tags.push(format!(r#"<script defer src="{js}"></script>"#));
            tags.push(format!(r#"<link rel="stylesheet" type="text/css" href="{css}">"#));
        }

        // accent color css var
        if let Some(accent) = site.get("accent_color").and_then(|a| a.as_str()).filter(|a| !a.is_empty()) {
            tags.push(format!(r#"<style>:root {{--ghost-accent-color: {accent};}}</style>"#));
        }

        // Portal, then Sodo Search — Ghost's order. Both need the frontend
        // Content API key, which the render root carries; with no key the
        // scripts would load but every request they make would 401, so skip
        // them rather than emit tags that cannot work.
        let frontend_key = root_get(rc, ctx, "frontendKey").as_str().unwrap_or("").to_string();
        if !frontend_key.is_empty() {
            let origin = origin.trim_end_matches('/');
            let members = members_helper(&site, origin, &frontend_key);
            if !members.is_empty() {
                tags.push(members);
            }
            tags.push(search_helper(&site, origin, &frontend_key));
        }

        // code injection: site head, then post head
        if let Some(ci) = site.get("codeinjection_head").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
            tags.push(ci.to_string());
        }
        if let Some(ci) = this.get("codeinjection_head").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
            tags.push(ci.to_string());
        }

        write_out(rc, out, &format!("\n    {}\n", tags.join("\n    ")), false)
    }
}

/// The announcement bar (Ghost's Settings → Site → Announcement bar). Ghost
/// serves it through a client-side widget that checks the visitor's member
/// status against `announcement_visibility`; this port renders it
/// server-side, so until member sessions exist only the `visitors` audience
/// can see the bar — and an empty visibility list keeps it hidden, exactly
/// like Ghost. Content is owner-authored and rendered as Markdown.
fn announcement_bar(site: &serde_json::Value) -> String {
    let content = match site.get("announcement_content").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
        Some(c) => c,
        None => return String::new(),
    };
    let visible = site
        .get("announcement_visibility")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().any(|v| v.as_str() == Some("visitors")))
        .unwrap_or(false);
    if !visible {
        return String::new();
    }
    let (bg, fg) = match site.get("announcement_background").and_then(|b| b.as_str()).unwrap_or("dark") {
        "light" => ("#ffffff".to_string(), "#111113".to_string()),
        "accent" => ("var(--ghost-accent-color, #ff1a75)".to_string(), "#ffffff".to_string()),
        _ => ("#111113".to_string(), "#ffffff".to_string()),
    };
    let html = crate::markdown::render_markdown(content).0;
    format!(
        r#"<style>.ghost-announcement{{position:fixed;top:0;left:0;right:0;z-index:9000;display:flex;align-items:center;justify-content:center;gap:8px;padding:10px 44px;font-size:13.5px;font-weight:500;text-align:center;background:{bg};color:{fg}}}.ghost-announcement p{{margin:0}}.ghost-announcement a{{color:inherit;text-decoration:underline}}.ghost-announcement button{{position:absolute;right:10px;background:none;border:none;color:inherit;opacity:.7;cursor:pointer;font-size:18px;line-height:1;padding:6px}}.ghost-announcement button:hover{{opacity:1}}</style>
<div class="ghost-announcement" role="banner"><div>{html}</div><button type="button" aria-label="Dismiss announcement" onclick="this.parentElement.remove()">&#215;</button></div>"#,
        bg = bg,
        fg = fg,
        html = html,
    )
}

pub struct GhostFoot;
impl HelperDef for GhostFoot {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let site = root_get(rc, ctx, "site");
        let this = current_value(rc, ctx);
        let mut parts = Vec::new();
        if let Some(ci) = site.get("codeinjection_foot").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
            parts.push(ci.to_string());
        }
        if let Some(ci) = this.get("codeinjection_foot").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
            parts.push(ci.to_string());
        }
        parts.push(announcement_bar(&site));
        let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        write_out(rc, out, &parts.join("\n"), false)
    }
}

pub fn meta_title_text(rc: &RenderContext, ctx: &Context) -> String {
    let this = current_resource(rc, ctx);
    let site_title = root_get(rc, ctx, "site.title").as_str().unwrap_or_default().to_string();
    let ctx_list = root_get(rc, ctx, "context");
    let is_home = ctx_list.as_array().map(|a| a.iter().any(|c| c.as_str() == Some("home"))).unwrap_or(false);
    if is_home {
        let site_meta = root_get(rc, ctx, "site.meta_title").as_str().unwrap_or_default().to_string();
        let desc = root_get(rc, ctx, "site.description").as_str().unwrap_or_default().to_string();
        if !site_meta.is_empty() {
            site_meta.to_string()
        } else if !desc.is_empty() {
            format!("{site_title} - {desc}")
        } else {
            site_title
        }
    } else {
        let own = this
            .get("meta_title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| this.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .or_else(|| this.get("title").and_then(|v| v.as_str()).map(str::to_string));
        match own {
            Some(o) => format!("{o} - {site_title}"),
            None => site_title,
        }
    }
}

pub fn meta_description_text(rc: &RenderContext, ctx: &Context) -> String {
    let this = current_resource(rc, ctx);
    let ctx_list = root_get(rc, ctx, "context");
    let is_home = ctx_list.as_array().map(|a| a.iter().any(|c| c.as_str() == Some("home"))).unwrap_or(false);
    if is_home {
        let meta = root_get(rc, ctx, "site.meta_description");
        let desc = root_get(rc, ctx, "site.description");
        meta.as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| desc.as_str().filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string()
    } else {
        this.get("meta_description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| this.get("custom_excerpt").and_then(|v| v.as_str()))
            .or_else(|| this.get("description").and_then(|v| v.as_str()))
            .or_else(|| this.get("excerpt").and_then(|v| v.as_str()))
            .or_else(|| this.get("bio").and_then(|v| v.as_str()))
            .unwrap_or_default()
            .to_string()
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// lodash's `_.escape`, which `{{excerpt}}` runs its text through.
fn html_escape_full(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// handlebars' `escapeExpression`, which escapes two more characters than
/// lodash does. Ghost uses this one in `{{title}}`, `{{tags}}`, `{{tiers}}`.
fn escape_expression(s: &str) -> String {
    html_escape_full(s).replace('`', "&#x60;").replace('=', "&#x3D;")
}

// ---------------------------------------------------------------------------
// misc small helpers / stubs
// ---------------------------------------------------------------------------

/// `{{concat a b separator="-"}}`. Array arguments are flattened, so
/// `(concat "icons/" type)` and `(concat tag_names separator=", ")` both work.
pub struct Concat;
impl HelperDef for Concat {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let separator = hash_str(h, "separator").unwrap_or_default();
        let parts: Vec<String> = h
            .params()
            .iter()
            .flat_map(|p| match p.value() {
                Value::Array(a) => a.iter().map(value_to_plain).collect::<Vec<_>>(),
                other => vec![value_to_plain(other)],
            })
            .collect();
        Ok(handlebars::ScopedJson::Derived(Value::String(parts.join(&separator))))
    }
}

/// `{{#social_accounts @site}}` — iterate a source's configured social
/// accounts in canonical order, exposing `type`, `name`, `href` and
/// `username` per entry.
pub struct SocialAccounts;

impl SocialAccounts {
    fn accounts(source: &Value) -> Vec<Value> {
        crate::social::SOCIAL_PLATFORMS
            .iter()
            .filter_map(|(kind, source_key, name)| {
                let raw = source.get(*source_key)?.as_str()?.trim();
                if raw.is_empty() {
                    return None;
                }
                Some(json!({
                    "type": kind,
                    "name": name,
                    "href": crate::social::url_for(source_key, raw)?,
                    "username": raw,
                }))
            })
            .collect()
    }
}

impl HelperDef for SocialAccounts {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let site = match h.param(0) {
            Some(p) => p.value().clone(),
            None => root_get(rc, ctx, "site"),
        };
        let items = Self::accounts(&site);

        if items.is_empty() {
            if let Some(t) = h.inverse() {
                t.render(r, ctx, rc, out)?;
            }
            return Ok(());
        }

        let len = items.len();
        rc.push_block(handlebars::BlockContext::new());
        for (i, item) in items.iter().enumerate() {
            if let Some(block) = rc.block_mut() {
                block.set_base_value(item.clone());
                block.set_local_var("index", i.into());
                block.set_local_var("number", (i + 1).into());
                block.set_local_var("first", (i == 0).into());
                block.set_local_var("last", (i == len - 1).into());
                block.set_local_var("even", (i % 2 == 1).into());
                block.set_local_var("odd", (i % 2 == 0).into());
                let mut params = handlebars::BlockParams::new();
                if let Some(name) = h.block_param() {
                    params.add_value(name, item.clone())?;
                }
                block.set_block_params(params);
            }
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
        }
        rc.pop_block();
        Ok(())
    }
}

/// `{{social_url type="facebook"}}` — the profile URL for a handle stored on
/// the current scope, falling back to the site settings.
pub struct SocialUrl {
    /// Fixed platform for the deprecated `{{facebook_url}}`/`{{twitter_url}}`
    /// aliases, which take the handle as a positional argument instead.
    pub fixed_platform: Option<&'static str>,
}
impl HelperDef for SocialUrl {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let null = || Ok(handlebars::ScopedJson::Derived(Value::Null));

        let Some(platform) = self.fixed_platform.map(str::to_string).or_else(|| hash_str(h, "type"))
        else {
            return null();
        };

        // an explicit handle wins, else the current scope's, else the site's
        let site = root_get(rc, ctx, "site");
        let this = current_value(rc, ctx);
        let username = h
            .param(0)
            .and_then(|p| p.value().as_str())
            .map(str::to_string)
            .or_else(|| this.get(&platform).and_then(|v| v.as_str()).map(str::to_string))
            .or_else(|| site.get(&platform).and_then(|v| v.as_str()).map(str::to_string))
            .filter(|u| !u.trim().is_empty());

        let Some(username) = username else { return null() };
        match crate::social::url_for(&platform, username.trim()) {
            Some(url) => Ok(handlebars::ScopedJson::Derived(Value::String(url))),
            None => null(),
        }
    }
}

/// `{{split str separator=","}}` returns an array, so it composes into
/// `{{#foreach (split tags)}}`; used as a block it iterates directly.
pub struct SplitHelper;
impl SplitHelper {
    fn parts(h: &Helper<'_>) -> Vec<Value> {
        let Some(input) = h.param(0).map(|p| p.value()) else {
            return Vec::new();
        };
        if input.is_null() {
            return Vec::new();
        }
        let s = value_to_plain(input);
        // the separator is a hash option; a second positional argument is
        // accepted too, which is how older themes write it
        let sep = hash_str(h, "separator")
            .or_else(|| h.param(1).map(|p| value_to_plain(p.value())))
            .unwrap_or_else(|| ",".into());
        if s.is_empty() {
            return Vec::new();
        }
        if sep.is_empty() {
            return s.chars().map(|c| Value::String(c.to_string())).collect();
        }
        s.split(sep.as_str())
            .filter(|p| !p.is_empty())
            .map(|p| Value::String(p.to_string()))
            .collect()
    }
}
impl HelperDef for SplitHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        Ok(handlebars::ScopedJson::Derived(Value::Array(Self::parts(h))))
    }

    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let parts = Self::parts(h);
        let Some(template) = h.template() else {
            // inline use: render the array the way handlebars would
            let joined: Vec<String> = parts.iter().map(value_to_plain).collect();
            return write_out(rc, out, &joined.join(","), true);
        };

        let len = parts.len();
        rc.push_block(handlebars::BlockContext::new());
        for (i, part) in parts.iter().enumerate() {
            if let Some(block) = rc.block_mut() {
                block.set_base_value(part.clone());
                block.set_local_var("index", i.into());
                block.set_local_var("number", (i + 1).into());
                block.set_local_var("first", (i == 0).into());
                block.set_local_var("last", (i == len - 1).into());
                let mut params = handlebars::BlockParams::new();
                if let Some(name) = h.block_param() {
                    params.add_value(name, part.clone())?;
                }
                block.set_block_params(params);
            }
            template.render(r, ctx, rc, out)?;
        }
        rc.pop_block();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// link / link_class — active-state aware anchors
// ---------------------------------------------------------------------------

/// Port of `buildLinkClasses`: the `class=` classes, plus an active or
/// parent-active class when the href matches the page being rendered.
fn build_link_classes(h: &Helper<'_>, href: &str, site_url: &str, location: &str) -> Vec<String> {
    let relative_href = href.strip_prefix(site_url).unwrap_or(href);

    let mut classes: Vec<String> = hash_str(h, "class")
        .map(|c| c.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();

    let active = h.hash_get("activeClass").map_or_else(
        || Some("nav-current".to_string()),
        |v| v.value().as_str().filter(|s| !s.is_empty()).map(str::to_string),
    );
    let parent_active = h.hash_get("parentActiveClass").map_or_else(
        || Some(format!("{}-parent", active.clone().unwrap_or_else(|| "nav-current".into()))),
        |v| v.value().as_str().filter(|s| !s.is_empty()).map(str::to_string),
    );

    if url_match(relative_href, location) {
        if let Some(c) = active {
            classes.push(c);
        }
    } else if url_parent_match(relative_href, location) {
        if let Some(c) = parent_active {
            classes.push(c);
        }
    }
    classes
}

/// Strip the anchor and the leading/trailing slashes, the way Ghost compares
/// a nav href against the current URL.
fn url_clean(url: &str) -> &str {
    let no_anchor = url.split('#').next().unwrap_or("");
    no_anchor.trim_matches('/')
}

fn url_match(href: &str, location: &str) -> bool {
    !location.is_empty() && url_clean(href) == url_clean(location)
}

/// True when `href` names an ancestor path of the current location.
fn url_parent_match(href: &str, location: &str) -> bool {
    if location.is_empty() {
        return false;
    }
    let loc: Vec<&str> = url_clean(location).split('/').collect();
    let hrf: Vec<&str> = url_clean(href).split('/').collect();
    if loc.len() <= hrf.len() {
        return false;
    }
    // Ghost only keeps the last comparison, so an href of one segment is a
    // parent when that segment matches
    hrf.last() == loc.get(hrf.len() - 1)
}

pub struct LinkClass;
impl HelperDef for LinkClass {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(target) = h.hash_get("for") else {
            tracing::warn!("the {{{{link_class}}}} helper requires a for=\"\" attribute");
            return Ok(());
        };
        let href = value_to_plain(target.value());
        let site_url = root_get(rc, ctx, "site.url").as_str().unwrap_or_default().to_string();
        let location = root_get(rc, ctx, "relativeUrl").as_str().unwrap_or_default().to_string();
        let classes = build_link_classes(h, &href, &site_url, &location);
        write_out(rc, out, &classes.join(" "), false)
    }
}

pub struct LinkHelper;
impl HelperDef for LinkHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(href) = h.hash_get("href").map(|v| value_to_plain(v.value())) else {
            tracing::warn!("the {{{{#link}}}}{{{{/link}}}} helper requires an href=\"\" attribute");
            return Ok(());
        };
        let site_url = root_get(rc, ctx, "site.url").as_str().unwrap_or_default().to_string();
        let location = root_get(rc, ctx, "relativeUrl").as_str().unwrap_or_default().to_string();
        let classes = build_link_classes(h, &href, &site_url, &location);

        // every hash option other than the ones we interpret becomes an
        // attribute on the anchor
        const MANAGED: &[&str] = &["href", "class", "activeClass", "parentActiveClass"];
        let attrs: String = h
            .hash()
            .iter()
            .filter(|(k, _)| !MANAGED.contains(&k.to_string().as_str()))
            .map(|(k, v)| format!(r#"{k}="{}""#, html_escape(&value_to_plain(v.value()))))
            .collect::<Vec<_>>()
            .join(" ");

        let mut open = String::from("<a");
        if !classes.is_empty() {
            open.push_str(&format!(r#" class="{}""#, classes.join(" ")));
        }
        // the href is written by the theme in site terms; the active-class
        // comparison above wants that form, the anchor wants the base path on
        open.push_str(&format!(r#" href="{}""#, html_escape(&ghost_core::url::base(&href))));
        if !attrs.is_empty() {
            open.push(' ');
            open.push_str(&attrs);
        }
        open.push('>');

        out.write(&open)?;
        if let Some(t) = h.template() {
            t.render(r, ctx, rc, out)?;
        }
        out.write("</a>")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// colour helpers
// ---------------------------------------------------------------------------

pub struct ColorToRgba;
impl HelperDef for ColorToRgba {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let color = h.param(0).map(|p| value_to_plain(p.value())).unwrap_or_default();
        let alpha = h.param(1).map(|p| p.value()).and_then(|v| {
            v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        });
        Ok(handlebars::ScopedJson::Derived(json!(crate::color::to_rgba_string(&color, alpha))))
    }
}

pub struct ContrastTextColor;
impl HelperDef for ContrastTextColor {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let color = h.param(0).map(|p| value_to_plain(p.value())).unwrap_or_default();
        Ok(handlebars::ScopedJson::Derived(json!(crate::color::contrast_text_color(&color))))
    }
}

// ---------------------------------------------------------------------------
// readable_url / raw / log
// ---------------------------------------------------------------------------

/// `{{readable_url "https://www.google.com/x/?q=1"}}` → `google.com/x`.
pub struct ReadableUrl;
impl HelperDef for ReadableUrl {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(input) = h.param(0).and_then(|p| p.value().as_str()).filter(|s| !s.is_empty())
        else {
            return Ok(());
        };
        write_out(rc, out, &readable_url(input), false)
    }
}

/// Host without `www.`, plus the path without its trailing slash. A string
/// that is not a URL is returned unchanged, as Ghost does.
fn readable_url(input: &str) -> String {
    let Some((_scheme, rest)) = input.split_once("://") else {
        return input.to_string();
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or(authority);
    if host.is_empty() {
        return input.to_string();
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    format!("{}{}", host.strip_prefix("www.").unwrap_or(host), path.trim_end_matches('/'))
}

/// `{{{{raw}}}}…{{{{/raw}}}}` — emit the block body without interpreting it.
pub struct RawHelper;
impl HelperDef for RawHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        if let Some(t) = h.template() {
            t.render(r, ctx, rc, out)?;
        }
        Ok(())
    }
}

/// `{{log value}}` — writes to the server log, never to the page.
pub struct LogHelper;
impl HelperDef for LogHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
        _out: &mut dyn Output,
    ) -> HelperResult {
        for p in h.params() {
            tracing::info!(target: "theme", "{}", p.value());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// members-era helpers
// ---------------------------------------------------------------------------

/// `{{search}}` — the button Ghost's search script binds to.
pub struct SearchHelper;
impl HelperDef for SearchHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        write_out(rc, out, SEARCH_BUTTON, false)
    }
}

const SEARCH_BUTTON: &str = concat!(
    r#"<button class="gh-search-icon" aria-label="search" data-ghost-search "#,
    r#"style="display: inline-flex; justify-content: center; align-items: center; width: 32px; "#,
    r#"height: 32px; padding: 0; border: 0; color: inherit; background-color: transparent; "#,
    r#"cursor: pointer; outline: none;">"#,
    r#"<svg width="20" height="20" fill="none" viewBox="0 0 24 24">"#,
    r#"<path d="M14.949 14.949a1 1 0 0 1 1.414 0l6.344 6.344a1 1 0 0 1-1.414 1.414l-6.344-6.344a1 1 0 0 1 0-1.414Z" fill="currentColor"/>"#,
    r#"<path d="M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14Zm-9 7a9 9 0 1 1 18 0 9 9 0 0 1-18 0Z" fill="currentColor"/>"#,
    r#"</svg></button>"#
);

/// `{{comment_count}}` — the placeholder Ghost's comment-counts script fills.
pub struct CommentCount;
impl HelperDef for CommentCount {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let id = this.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let empty = hash_str(h, "empty").unwrap_or_default();
        let singular = hash_str(h, "singular").unwrap_or_else(|| "comment".into());
        let plural = hash_str(h, "plural").unwrap_or_else(|| "comments".into());
        let autowrap = hash_str(h, "autowrap").as_deref() != Some("false");
        let tag = if autowrap {
            hash_str(h, "autowrap").unwrap_or_else(|| "span".into())
        } else {
            "script".into()
        };
        let class_name = hash_str(h, "class").unwrap_or_default();
        let html = format!(
            concat!(
                r#"<script data-ghost-comment-count="{id}" data-ghost-comment-count-empty="{empty}" "#,
                r#"data-ghost-comment-count-singular="{singular}" data-ghost-comment-count-plural="{plural}" "#,
                r#"data-ghost-comment-count-tag="{tag}" data-ghost-comment-count-class-name="{class_name}" "#,
                r#"data-ghost-comment-count-autowrap="{autowrap}"></script>"#,
            ),
            id = html_escape(id),
            empty = html_escape(&empty),
            singular = html_escape(&singular),
            plural = html_escape(&plural),
            tag = html_escape(&tag),
            class_name = html_escape(&class_name),
            autowrap = autowrap,
        );
        write_out(rc, out, &html, false)
    }
}

/// `{{comments}}` — the mount point Ghost's comments bundle replaces.
pub struct CommentsHelper;
impl HelperDef for CommentsHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // comments only exist on a post, and only when enabled site-wide
        let this = current_value(rc, ctx);
        let post = this.get("post").filter(|p| p.is_object()).unwrap_or(&this);
        if !is_post(post) {
            return Ok(());
        }
        let enabled = root_get(rc, ctx, "site.comments_enabled");
        if matches!(enabled.as_str(), None | Some("off")) {
            return Ok(());
        }

        let id = post.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let title = post.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        let count_label = hash_str(h, "count").unwrap_or_else(|| "true".into());
        let html = format!(
            r#"<div class="gh-comments" data-ghost-comments="{}" data-ghost-comments-title="{}" data-ghost-comments-count="{}"></div>"#,
            html_escape(id),
            html_escape(title),
            html_escape(&count_label),
        );
        write_out(rc, out, &html, false)
    }
}

/// `{{tiers}}` — the names of the tiers a post is available to.
pub struct TiersHelper;
impl HelperDef for TiersHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let names: Vec<String> = this
            .get("tiers")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                    .map(escape_expression)
                    .collect()
            })
            .unwrap_or_default();
        if names.is_empty() {
            return Ok(());
        }

        let separator = hash_str(h, "separator").unwrap_or_else(|| ", ".into());
        let last_separator = hash_str(h, "lastSeparator").unwrap_or_else(|| " and ".into());
        let prefix = hash_str(h, "prefix").unwrap_or_default();

        // the default suffix carries the singular/plural word
        let (body, default_suffix) = if names.len() == 1 {
            (names[0].clone(), " tier")
        } else {
            let (firsts, last) = names.split_at(names.len() - 1);
            (format!("{}{last_separator}{}", firsts.join(&separator), last[0]), " tiers")
        };
        let suffix = hash_str(h, "suffix").unwrap_or_else(|| default_suffix.to_string());
        write_out(rc, out, &format!("{prefix}{body}{suffix}"), false)
    }
}

/// `{{price 2100 currency="USD"}}` — an amount in the currency's major unit.
pub struct PriceHelper;
impl HelperDef for PriceHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        _ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
    ) -> Result<handlebars::ScopedJson<'rc>, handlebars::RenderError> {
        let arg = h.param(0).map(|p| p.value().clone()).unwrap_or(Value::Null);
        // either a bare amount or a plan/tier object carrying one
        let (amount, currency) = match &arg {
            Value::Number(n) => (n.as_f64(), hash_str(h, "currency")),
            Value::Object(_) => (
                arg.get("amount").and_then(|a| a.as_f64()),
                arg.get("currency")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
                    .or_else(|| hash_str(h, "currency")),
            ),
            _ => (None, hash_str(h, "currency")),
        };
        let Some(amount) = amount else {
            tracing::warn!("attribute is required e.g. {{{{price plan.amount}}}}");
            return Ok(handlebars::ScopedJson::Derived(json!("")));
        };
        // amounts are stored in the currency's minor unit
        let major = amount / 100.0;
        let symbol = currency.as_deref().map(currency_symbol).unwrap_or_default();
        let formatted = if (major.fract()).abs() < f64::EPSILON {
            format!("{}{}", symbol, major as i64)
        } else {
            format!("{symbol}{major:.2}")
        };
        Ok(handlebars::ScopedJson::Derived(json!(formatted)))
    }
}

fn currency_symbol(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "USD" | "AUD" | "CAD" | "NZD" | "SGD" | "HKD" | "MXN" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "INR" => "₹",
        "BRL" => "R$",
        "KRW" => "₩",
        "RUB" => "₽",
        "SEK" | "NOK" | "DKK" => "kr",
        "CHF" => "CHF ",
        _ => "",
    }
}

/// `{{total_members}}` / `{{total_paid_members}}` — Ghost rounds these so a
/// site never advertises an exact member count.
pub struct TotalMembers {
    pub paid: bool,
}
impl HelperDef for TotalMembers {
    fn call<'reg: 'rc, 'rc>(
        &self,
        _h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let key = if self.paid { "paid_members" } else { "total_members" };
        let total = root_get(rc, ctx, &format!("site.{key}")).as_i64().unwrap_or(0);
        write_out(rc, out, &member_count_rounding(total), false)
    }
}

/// Ghost's `memberCountRounding`: exact below 1,000, then rounded down to a
/// readable figure.
fn member_count_rounding(count: i64) -> String {
    match count {
        c if c < 1_000 => c.to_string(),
        c if c < 10_000 => format!("{:.1}k", (c as f64 / 100.0).floor() / 10.0),
        c if c < 1_000_000 => format!("{}k", c / 1_000),
        c => format!("{:.1}m", (c as f64 / 100_000.0).floor() / 10.0),
    }
}

/// `{{content_api_url}}` / `{{content_api_key}}`.
pub struct ContentApi {
    pub is_url: bool,
    pub key: String,
}
impl HelperDef for ContentApi {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        if !self.is_url {
            return write_out(rc, out, &self.key, false);
        }
        // the Content API URL is absolute unless the theme says otherwise
        let relative = h.hash_get("absolute").is_some() && !hash_flag(h, "absolute");
        let path = "/ghost/api/content/";
        if relative {
            return write_out(rc, out, path, false);
        }
        let origin = root_get(rc, ctx, "site.url").as_str().unwrap_or_default().to_string();
        write_out(rc, out, &format!("{}{path}", origin.trim_end_matches('/')), false)
    }
}

/// `{{cancel_link}}` — the subscription cancel/continue toggle.
pub struct CancelLink;
impl HelperDef for CancelLink {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let mut data = Map::new();
        data.insert("id".into(), this.get("id").cloned().unwrap_or(Value::Null));
        data.insert(
            "cancel_at_period_end".into(),
            this.get("cancel_at_period_end").cloned().unwrap_or(Value::Bool(false)),
        );
        data.insert("class".into(), json!(hash_str(h, "class").unwrap_or_else(|| "gh-cancel-link".into())));
        data.insert("errorClass".into(), json!(hash_str(h, "errorClass").unwrap_or_else(|| "gh-error".into())));
        data.insert(
            "cancelLabel".into(),
            json!(hash_str(h, "cancelLabel").unwrap_or_else(|| "Cancel subscription".into())),
        );
        data.insert(
            "continueLabel".into(),
            json!(hash_str(h, "continueLabel").unwrap_or_else(|| "Continue subscription".into())),
        );

        let mut block = handlebars::BlockContext::new();
        block.set_base_value(Value::Object(data));
        rc.push_block(block);
        let rendered = r.get_template("cancel_link").map(|tpl| tpl.render(r, ctx, rc, out));
        rc.pop_block();
        rendered.transpose()?;
        Ok(())
    }
}

/// `{{recommendations}}` — renders the recommendations partial over whatever
/// the route put on the scope.
pub struct Recommendations;
impl HelperDef for Recommendations {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let items = this
            .get("recommendations")
            .cloned()
            .or_else(|| Some(root_get(rc, ctx, "recommendations")))
            .filter(|v| v.is_array())
            .unwrap_or(Value::Array(vec![]));
        if items.as_array().is_some_and(|a| a.is_empty()) {
            return Ok(());
        }

        let mut data = Map::new();
        data.insert("recommendations".into(), items);
        for (k, v) in h.hash() {
            data.insert((*k).to_string(), v.value().clone());
        }
        let mut block = handlebars::BlockContext::new();
        block.set_base_value(Value::Object(data));
        rc.push_block(block);
        let rendered = r.get_template("recommendations").map(|tpl| tpl.render(r, ctx, rc, out));
        rc.pop_block();
        rendered.transpose()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// block / contentFor — express-hbs' template-hierarchy escape hatch
// ---------------------------------------------------------------------------

/// Shared store for `{{#contentFor "x"}}…{{/contentFor}}` → `{{block "x"}}`.
/// A render pass fills it, and the layout pass reads it back, which is why
/// it has to outlive a single template render.
pub type BlockStore = Arc<std::sync::Mutex<HashMap<String, String>>>;

pub struct ContentFor {
    pub blocks: BlockStore,
}
impl HelperDef for ContentFor {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        _out: &mut dyn Output,
    ) -> HelperResult {
        let Some(name) = h.param(0).and_then(|p| p.value().as_str()).map(str::to_string) else {
            return Ok(());
        };
        let mut buf = handlebars::StringOutput::new();
        if let Some(t) = h.template() {
            t.render(r, ctx, rc, &mut buf)?;
        }
        let body = buf.into_string().unwrap_or_default();
        if let Ok(mut blocks) = self.blocks.lock() {
            blocks.entry(name).or_default().push_str(&body);
        }
        Ok(())
    }
}

pub struct BlockHelper {
    pub blocks: BlockStore,
}
impl HelperDef for BlockHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let Some(name) = h.param(0).and_then(|p| p.value().as_str()) else {
            return Ok(());
        };
        let body = self.blocks.lock().ok().and_then(|b| b.get(name).cloned());
        match body {
            Some(body) if !body.is_empty() => out.write(&body)?,
            // no contentFor filled this block: fall back to the default body
            _ => {
                if let Some(t) = h.template() {
                    t.render(r, ctx, rc, out)?;
                }
            }
        }
        Ok(())
    }
}


// ---------------------------------------------------------------------------
// blockHelperMissing / helperMissing — handlebars.js semantics that
// handlebars-rust omits: `{{#post}}...{{/post}}` renders the block with the
// value of `post` as context when truthy, else the inverse branch.
// ---------------------------------------------------------------------------

pub struct BlockHelperMissing;
impl HelperDef for BlockHelperMissing {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let name = h.name().to_string();
        let val = rc
            .evaluate(ctx, &name)
            .map(|v| v.as_json().clone())
            .unwrap_or(Value::Null);
        let truthy = match &val {
            Value::Null | Value::Bool(false) => false,
            Value::String(s) => !s.is_empty() && s != "0",
            Value::Array(a) => !a.is_empty(),
            _ => true,
        };
        if truthy {
            let mut block = handlebars::BlockContext::new();
            block.set_base_value(val);
            rc.push_block(block);
            if let Some(t) = h.template() {
                t.render(r, ctx, rc, out)?;
            }
            rc.pop_block();
        } else if let Some(t) = h.inverse() {
            t.render(r, ctx, rc, out)?;
        }
        Ok(())
    }
}

pub struct HelperMissing;
impl HelperDef for HelperMissing {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        _r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // unknown helper with params: evaluate its name as a value path
        let name = h.name().to_string();
        let val = rc
            .evaluate(ctx, &name)
            .map(|v| v.as_json().clone())
            .unwrap_or(Value::Null);
        let s = match val {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        };
        out.write(&s)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------


/// Register the Ghost helper set onto a registry.
pub fn register_all(
    hb: &mut Handlebars<'_>,
    deps: &HelperDeps,
    translations: HashMap<String, String>,
    blocks: BlockStore,
) {
    hb.register_helper("blockHelperMissing", Box::new(BlockHelperMissing));
    hb.register_helper("helperMissing", Box::new(HelperMissing));

    // functional
    hb.register_helper("foreach", Box::new(Foreach));
    hb.register_helper("is", Box::new(Is));
    hb.register_helper("has", Box::new(Has));
    hb.register_helper("match", Box::new(Match));

    // data
    hb.register_helper("content", Box::new(Content));
    hb.register_helper("excerpt", Box::new(Excerpt));
    hb.register_helper("title", Box::new(Title));
    hb.register_helper("url", Box::new(UrlHelper));
    hb.register_helper("img_url", Box::new(ImgUrl));
    hb.register_helper("date", Box::new(DateHelper));
    hb.register_helper("tags", Box::new(TagsList { is_authors: false }));
    hb.register_helper("authors", Box::new(TagsList { is_authors: true }));
    hb.register_helper("navigation", Box::new(Navigation));
    hb.register_helper("social_accounts", Box::new(SocialAccounts));
    hb.register_helper("social_url", Box::new(SocialUrl { fixed_platform: None }));
    // deprecated single-platform aliases, still used by older themes
    hb.register_helper("facebook_url", Box::new(SocialUrl { fixed_platform: Some("facebook") }));
    hb.register_helper("twitter_url", Box::new(SocialUrl { fixed_platform: Some("twitter") }));
    hb.register_helper("link", Box::new(LinkHelper));
    hb.register_helper("readable_url", Box::new(ReadableUrl));

    // utility
    hb.register_helper("body_class", Box::new(BodyClass));
    hb.register_helper("post_class", Box::new(PostClass));
    hb.register_helper("link_class", Box::new(LinkClass));
    hb.register_helper("ghost_head", Box::new(GhostHead));
    hb.register_helper("ghost_foot", Box::new(GhostFoot));
    hb.register_helper("concat", Box::new(Concat));
    hb.register_helper("split", Box::new(SplitHelper));
    hb.register_helper("encode", Box::new(Encode));
    hb.register_helper("json", Box::new(JsonHelper));
    hb.register_helper("plural", Box::new(Plural));
    hb.register_helper("pagination", Box::new(Pagination));
    hb.register_helper("page_url", Box::new(PageUrl));
    hb.register_helper("reading_time", Box::new(ReadingTime));
    hb.register_helper("color_to_rgba", Box::new(ColorToRgba));
    hb.register_helper("contrast_text_color", Box::new(ContrastTextColor));
    hb.register_helper("raw", Box::new(RawHelper));
    hb.register_helper("log", Box::new(LogHelper));
    hb.register_helper("meta_title", Box::new(MetaTitle));
    hb.register_helper("meta_description", Box::new(MetaDescription));

    // i18n — `translate` is the long-form name of the same helper
    let translations = Arc::new(translations);
    hb.register_helper("t", Box::new(Translate { translations: translations.clone() }));
    hb.register_helper("translate", Box::new(Translate { translations }));

    // express-hbs block/contentFor pair
    hb.register_helper("block", Box::new(BlockHelper { blocks: blocks.clone() }));
    hb.register_helper("contentFor", Box::new(ContentFor { blocks }));

    // members-era
    hb.register_helper("search", Box::new(SearchHelper));
    hb.register_helper("comments", Box::new(CommentsHelper));
    hb.register_helper("comment_count", Box::new(CommentCount));
    hb.register_helper("tiers", Box::new(TiersHelper));
    hb.register_helper("price", Box::new(PriceHelper));
    hb.register_helper("total_members", Box::new(TotalMembers { paid: false }));
    hb.register_helper("total_paid_members", Box::new(TotalMembers { paid: true }));
    hb.register_helper("cancel_link", Box::new(CancelLink));
    hb.register_helper("recommendations", Box::new(Recommendations));
    hb.register_helper("content_api_url", Box::new(ContentApi { is_url: true, key: String::new() }));
    hb.register_helper(
        "content_api_key",
        Box::new(ContentApi { is_url: false, key: deps.content_api_key.clone() }),
    );

    // asset helper needs theme state
    hb.register_helper(
        "asset",
        Box::new(AssetHelper {
            hashes: deps.asset_hashes.clone(),
            site_origin: deps.site_origin.clone(),
            global_hash: deps.global_asset_hash.clone(),
        }),
    );

    // helpers with side deps
    hb.register_helper("get", Box::new(crate::get_helper::GetHelper::new(deps.clone())));
    hb.register_helper("prev_post", Box::new(crate::get_helper::PrevNextPost::new(deps.clone(), true)));
    hb.register_helper("next_post", Box::new(crate::get_helper::PrevNextPost::new(deps.clone(), false)));

    // not yet backed by data: render nothing rather than the helper name
    hb.register_helper("collection", Box::new(Noop));
}

pub struct Noop;
impl HelperDef for Noop {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        _rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let _ = (h, r, ctx, out);
        Ok(())
    }
}

#[cfg(test)]
mod social_tests {
    use super::*;

    #[test]
    fn builds_hrefs_from_handles_and_urls() {
        let site = json!({
            "facebook": "ghostorg",
            "twitter": "@ghost",
            "bluesky": "https://bsky.app/profile/ghost.org",
            "youtube": "@ghostcms",
            "instagram": "   ",
            "linkedin": "company/ghost",
        });
        let got: Vec<(String, String)> = SocialAccounts::accounts(&site)
            .iter()
            .map(|a| (a["type"].as_str().unwrap().into(), a["href"].as_str().unwrap().into()))
            .collect();

        // the order is the canonical one from Ghost's social_accounts helper
        assert_eq!(
            got,
            vec![
                ("x".to_string(), "https://x.com/ghost".to_string()),
                ("facebook".into(), "https://www.facebook.com/ghostorg".into()),
                ("linkedin".into(), "https://www.linkedin.com/company/ghost".into()),
                ("bluesky".into(), "https://bsky.app/profile/ghost.org".into()),
                ("youtube".into(), "https://www.youtube.com/@ghostcms".into()),
            ]
        );
    }

    #[test]
    fn no_accounts_configured_yields_nothing() {
        assert!(SocialAccounts::accounts(&json!({ "title": "A blog" })).is_empty());
        assert!(SocialAccounts::accounts(&Value::Null).is_empty());
    }
}
