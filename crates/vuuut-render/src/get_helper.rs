//! `{{#get}}` — in-template Content API queries — plus prev/next_post.
//!
//! Renders already run inside `spawn_blocking`, so blocking on the runtime
//! handle here is legal (mirrors Ghost's async helpers).
use handlebars::{
    Context, Handlebars, Helper, HelperDef, HelperResult, Output, RenderContext, Renderable,
};
use serde_json::{json, Map, Value};

use crate::helpers::HelperDeps;
use crate::util::current_value;

pub struct GetHelper {
    deps: HelperDeps,
}

impl GetHelper {
    pub fn new(deps: HelperDeps) -> Self {
        Self { deps }
    }
}

/// The resources `{{#get}}` accepts, from upstream `RESOURCES` in
/// `core/frontend/helpers/get.js`. Anything else sets `@data.error` and renders
/// `{{else}}` rather than quietly producing an empty list — a theme asking for
/// `{{#get "members"}}` is a theme bug, and Ghost says so.
///
/// Vuuut adds `products`: the online store's active products, so a theme can
/// show its catalogue anywhere (see [`browse_products`]).
const RESOURCES: &[&str] = &["posts", "pages", "tags", "authors", "tiers", "newsletters", "products"];

/// Ghost's `optimization:maxLimit`. Since 6.x `limit="all"` is capped here too
/// rather than fetching the whole table.
const MAX_LIMIT: i64 = 100;

/// Upstream `applyLimitCap`: `all` and anything above the cap become the cap,
/// and an unparseable limit is treated as out of range rather than as 15.
fn apply_limit_cap(raw: &str) -> i64 {
    if raw.is_empty() {
        return 15;
    }
    if raw.eq_ignore_ascii_case("all") {
        return MAX_LIMIT;
    }
    match raw.parse::<i64>() {
        Ok(n) if n > 0 && n <= MAX_LIMIT => n,
        // A non-positive limit is not a request for everything; the rest of
        // this port reads it as "unset" and so does this.
        Ok(n) if n <= 0 => 15,
        // Above the cap, or not a number at all, both land on the cap.
        _ => MAX_LIMIT,
    }
}

fn apply_context_access(post: &mut Value, ctx: &Context) {
    let member = ctx.data().get("member").unwrap_or(&Value::Null);
    let status = member.get("status").and_then(Value::as_str);
    let tiers = member.get("tiers").and_then(Value::as_array).map(|tiers| tiers.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
    vuuut_core::models::apply_post_access(post, status, &tiers);
}

impl HelperDef for GetHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let resource = h
            .param(0)
            .and_then(|p| p.value().as_str())
            .unwrap_or("posts")
            .to_string();

        // Ghost warns and renders {{else}} for a resource it has no public
        // controller for, rather than returning an empty collection that looks
        // like "no results".
        if !RESOURCES.contains(&resource.as_str()) {
            if let Some(t) = h.inverse() {
                t.render(r, ctx, rc, out)?;
            }
            return Ok(());
        }

        // options from hash
        let limit_raw = h.hash_get("limit").map(|v| crate::get_helper::util2::value_string(v.value())).unwrap_or_default();
        let page: i64 = h.hash_get("page").and_then(|v| v.value().as_i64()).unwrap_or(1);
        let order = h.hash_get("order").and_then(|v| v.value().as_str()).map(str::to_string);
        let include = h.hash_get("include").and_then(|v| v.value().as_str()).map(str::to_string);
        // filter supports {{path}} interpolation against the current scope
        let filter_raw = h.hash_get("filter").and_then(|v| v.value().as_str()).map(str::to_string);
        let fields = h.hash_get("fields").and_then(|v| v.value().as_str()).map(str::to_string);
        let formats = h.hash_get("formats").and_then(|v| v.value().as_str()).map(str::to_string);
        // `id`/`slug` make this a read rather than a browse
        let read_by = h
            .hash_get("id")
            .map(|v| ("id", crate::get_helper::util2::value_string(v.value())))
            .or_else(|| {
                h.hash_get("slug").map(|v| ("slug", crate::get_helper::util2::value_string(v.value())))
            })
            .filter(|(_, v)| !v.is_empty());

        let deps = self.deps.clone();
        let this = current_value(rc, ctx);
        let globals = handlebars_globals(ctx);
        let filter = match (&read_by, filter_raw) {
            // A read is addressed by its own key; upstream ignores `filter`
            // for it because the controller takes none.
            (Some((key, value)), _) => Some(format!("{key}:'{}'", value.replace('\'', "\\'"))),
            (None, Some(f)) => Some(interpolate_filter(&f, &this, &globals)),
            (None, None) => None,
        };
        let resource_for_query = resource.clone();
        let is_read = read_by.is_some();
        let limit_for_query = if is_read { "1".to_string() } else { limit_raw };

        // The page's language, unless the entry is addressed by id or slug
        // (a read names exactly what it wants) or the theme's own filter
        // already chooses a language.
        let lang = page_language(rc, ctx).filter(|_| {
            !is_read && !filter.as_deref().is_some_and(|f| f.contains("locale"))
        });
        let runtime = deps.runtime.clone();
        let result: Result<(Value, Value), String> = runtime.block_on(async move {
            run_get(&deps, &resource_for_query, &filter, &limit_for_query, page, order.as_deref(), include.as_deref(), lang.as_ref())
                .await
                .map_err(|e| e.message())
        });

        match result {
            Ok((mut items, pagination)) => {
                if matches!(resource.as_str(), "posts" | "pages") {
                    if let Some(items) = items.as_array_mut() {
                        for item in items { apply_context_access(item, ctx); }
                    }
                }
                if let Some(items) = items.as_array_mut() {
                    for item in items.iter_mut() {
                        apply_formats(item, formats.as_deref());
                        apply_fields(item, fields.as_deref(), &resource);
                    }
                }
                let mut data = Map::new();
                data.insert(resource.clone(), items);
                // Ghost's read controllers return no pagination meta, so a
                // theme cannot page through a single-record result.
                if !is_read {
                    data.insert("pagination".into(), pagination);
                }
                let mut block = handlebars::BlockContext::new();
                block.set_base_value(Value::Object(data.clone()));
                // `as |result pageInfo|` binds the items themselves, not the
                // wrapper — `{{#foreach result}}` has to see an array
                let items = data.get(&resource).cloned().unwrap_or(Value::Array(vec![]));
                let mut params = handlebars::BlockParams::new();
                if let Some((p1, p2)) = h.block_param_pair() {
                    params.add_value(p1, items)?;
                    params.add_value(p2, data.get("pagination").cloned().unwrap_or(Value::Null))?;
                } else if let Some(p1) = h.block_param() {
                    params.add_value(p1, items)?;
                }
                block.set_block_params(params);
                rc.push_block(block);
                if let Some(t) = h.template() {
                    t.render(r, ctx, rc, out)?;
                }
                rc.pop_block();
            }
            Err(_e) => {
                // Ghost renders {{else}} and sets data.error
                if let Some(t) = h.inverse() {
                    t.render(r, ctx, rc, out)?;
                }
            }
        }
        Ok(())
    }
}

/// The Handlebars globals an `@`-prefixed filter path reads.
///
/// They live on the root data object here (`site`, `config`, `member`,
/// `custom`, `labs`), which is exactly upstream's frame minus `_parent`/`root`.
fn handlebars_globals(ctx: &Context) -> Value {
    ctx.data().clone()
}

/// The content formats `{{#get}}` can select between.
///
/// Dropping the formats a theme did not ask for is the point of the option:
/// `formats="plaintext"` on a list of 100 posts is how a theme avoids shipping
/// 100 rendered post bodies it will not use.
const CONTENT_FORMATS: &[&str] = &["html", "plaintext"];

fn apply_formats(item: &mut Value, formats: Option<&str>) {
    let Some(formats) = formats else { return };
    let wanted: Vec<&str> = formats.split(',').map(str::trim).collect();
    let Value::Object(m) = item else { return };

    // `plaintext` is not part of the serialized post here — it is a column the
    // excerpt is derived from, not a field the serializer emits — so a theme
    // asking for it would otherwise get nothing at all: the html removed below
    // and no plaintext to replace it. Derived from the html the record does
    // carry, which is what the stored column holds anyway.
    if wanted.contains(&"plaintext") && !m.contains_key("plaintext") {
        if let Some(html) = m.get("html").and_then(Value::as_str) {
            let plain = crate::markdown::html_to_plaintext(html);
            m.insert("plaintext".into(), Value::String(plain));
        }
    }

    for format in CONTENT_FORMATS {
        if !wanted.contains(format) {
            m.remove(*format);
        }
    }
}

/// Narrow a record to the requested `fields`.
///
/// `url` is added back for posts, pages and tags the way upstream does: it is
/// attached by the serializer rather than being a column, and `{{url}}` in the
/// block would otherwise render empty for a theme that listed only
/// `fields="title,url"`-adjacent columns.
fn apply_fields(item: &mut Value, fields: Option<&str>, resource: &str) {
    let Some(fields) = fields else { return };
    let mut wanted: Vec<&str> = fields.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
    if wanted.is_empty() {
        return;
    }
    if matches!(resource, "posts" | "pages" | "tags") && !wanted.contains(&"url") {
        wanted.push("url");
    }
    if let Value::Object(m) = item {
        m.retain(|k, _| wanted.contains(&k.as_str()));
        // A listed field with no value still has to exist, or `{{#if}}` on it
        // reads as "key missing" instead of "empty".
        for field in wanted {
            m.entry(field.to_string()).or_insert(Value::Null);
        }
    }
}

/// The short path forms Ghost understands, from upstream `pathAliases`.
///
/// `filter="tags:[{{post.tags}}]"` is the common one: on its own `post.tags`
/// is an array of tag objects, which is useless in a filter, so Ghost expands
/// it to every tag slug before interpolating.
fn path_alias(path: &str) -> &str {
    match path {
        "post.tags" => "post.tags[*].slug",
        "post.author" => "post.author.slug",
        other => other,
    }
}

/// Replace `{{path.to.value}}` inside a filter with the value(s) from scope.
///
/// Ported from upstream `resolvePaths`. Every lookup yields a *list* of
/// matches joined with commas, which is what makes `tags:[{{post.tags}}]`
/// expand to `tags:[one,two,three]`. `@`-prefixed paths read the Handlebars
/// globals (`@site`, `@custom`, …) instead of the current scope.
fn interpolate_filter(filter: &str, scope: &Value, globals: &Value) -> String {
    let mut out = String::with_capacity(filter.len());
    let mut rest = filter;
    while let Some(start) = rest.find("{{") {
        let Some(end_rel) = rest[start + 2..].find("}}") else { break };
        let end = start + 2 + end_rel;
        out.push_str(&rest[..start]);
        let raw = rest[start + 2..end].trim();
        // Handlebars writes literal segments as `.[my-key]`; the query form is
        // the same path without the leading dot.
        let path = path_alias(raw).replace(".[", "[");
        let matches = if let Some(global_path) = path.strip_prefix('@') {
            // Preprocessing rewrites `@site` to `@root.site` before the
            // template is parsed, so the path arrives either way.
            query_simple_path(globals, global_path.strip_prefix("root.").unwrap_or(global_path))
        } else {
            query_simple_path(scope, &path)
        };
        let joined: Vec<String> = matches
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            })
            .collect();
        out.push_str(&joined.join(","));
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Resolve a path like `post.tags[*].slug` to every value it matches.
///
/// Ported from upstream `querySimplePath`. The wildcard is the reason this
/// returns a list: `tags[*]` flattens the array into the working set so the
/// following `.slug` applies to each element. `tags[0]` keeps the single
/// element, and a path that matches nothing yields an empty list rather than
/// a null — a filter interpolating it gets an empty string, not "null".
fn query_simple_path(root: &Value, path: &str) -> Vec<Value> {
    let mut current = vec![root.clone()];
    for part in path.split('.') {
        if current.is_empty() || part.is_empty() {
            break;
        }
        let (key, bracket) = match (part.find('['), part.strip_suffix(']')) {
            (Some(open), Some(_)) => (&part[..open], Some(&part[open + 1..part.len() - 1])),
            _ => (part, None),
        };
        // Upstream validates each segment against `^\w+(\[(\*|\d+)\])?$` and
        // raises on anything else. Resolving to nothing is the same outcome for
        // a filter — an empty interpolation — without taking the render down.
        let valid_key = !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_');
        let valid_bracket =
            bracket.is_none_or(|b| b == "*" || (!b.is_empty() && b.chars().all(|c| c.is_ascii_digit())));
        if !valid_key || !valid_bracket {
            return Vec::new();
        }
        let next: Vec<Value> =
            current.iter().filter_map(|item| item.get(key).cloned()).collect();
        current = match bracket {
            Some("*") => next
                .into_iter()
                .flat_map(|item| match item {
                    Value::Array(a) => a,
                    _ => Vec::new(),
                })
                .collect(),
            Some(index) => {
                let index: usize = index.parse().unwrap_or(0);
                next.into_iter().filter_map(|item| item.get(index).cloned()).collect()
            }
            None => next,
        };
    }
    current
}

/// The language a page is being rendered in, on a multilingual site.
pub(crate) struct PageLanguage {
    current: String,
    default: String,
}

/// Read from `@site`, which `context::build_site_for_page` fills from the
/// request's URL prefix and the *current* settings — so a theme's
/// `{{#get "posts"}}` on the English home page lists English posts, on
/// `/bn/` Bengali ones, and a change of default language is followed on the
/// next render without anything else being told. `None` on a
/// single-language site, where the query stays exactly what it always was.
fn page_language(rc: &RenderContext<'_, '_>, ctx: &Context) -> Option<PageLanguage> {
    let site = crate::util::root_get(rc, ctx, "site");
    if site.get("is_multilingual").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let current = site.get("locale").and_then(Value::as_str)?.to_string();
    // `@site.locales` lists the enabled languages with the default first.
    let default = site.get("locales").and_then(|l| l.get(0)).and_then(Value::as_str)?.to_string();
    Some(PageLanguage { current, default })
}

/// Query the DB the way the Content API would for a get helper.
async fn run_get(
    deps: &HelperDeps,
    resource: &str,
    filter: &Option<String>,
    limit_raw: &str,
    page: i64,
    order: Option<&str>,
    include: Option<&str>,
    lang: Option<&PageLanguage>,
) -> vuuut_core::Result<(Value, Value)> {
    use vuuut_db::repos;

    let limit: i64 = apply_limit_cap(limit_raw);
    let filter_expr = match filter {
        Some(f) if !f.trim().is_empty() => Some(vuuut_nql::parse(f)?),
        _ => None,
    };
    let includes_tags = include.map(|i| i.contains("tags")).unwrap_or(true);
    let includes_authors = include.map(|i| i.contains("authors")).unwrap_or(true);
    let includes_count = include.map(|i| i.contains("count")).unwrap_or(false);

    let resolve = &*deps.url_resolver;

    match resource {
        "posts" | "pages" => {
            let q = repos::posts::BrowsePosts {
                filter: filter_expr,
                order: order.map(str::to_string),
                limit,
                page,
                post_type: Some(resource.trim_end_matches('s').to_string()),
                public: true,
                include_authors: includes_authors,
                include_tags: includes_tags,
                locale: lang.map(|l| l.current.clone()),
                default_locale: lang.map(|l| l.default.clone()),
            };
            let res = repos::posts::browse(&deps.pool, &q).await?;
            let mut items = Vec::new();
            for p in &res.posts {
                let rel = repos::posts::load_relations(&deps.pool, std::slice::from_ref(p)).await?;
                let authors = rel.authors_for(&p.id);
                let tags = rel.tags_for(&p.id);
                let mut j = vuuut_core::models::post_to_json(p, &authors, &tags, "");
                j["url"] = json!(resolve(&j));
                crate::util::resolve_nested_urls(&mut j, resolve);
                items.push(j);
            }
            let pagination = vuuut_core::models::pagination_json(res.page, res.limit.max(1), res.total);
            Ok((Value::Array(items), pagination))
        }
        "tags" => {
            let (tags, total) = repos::tags::browse(
                &deps.pool,
                filter_expr.as_ref(),
                order,
                limit,
                page,
                true,
                includes_count,
            )
            .await?;
            // A tag with a language of its own is listed only in that
            // language; one with none belongs to every language.
            let items: Vec<Value> = tags
                .iter()
                .filter(|t| match (lang, t.locale.as_deref().filter(|c| !c.is_empty())) {
                    (Some(l), Some(code)) => code.eq_ignore_ascii_case(&l.current),
                    _ => true,
                })
                .map(|t| {
                    let mut j = vuuut_core::models::tag_to_json(t, None);
                    let url = crate::util::resolve_for_page(&j, lang.map(|l| l.current.as_str()), true, resolve);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            let pagination = vuuut_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "authors" | "users" => {
            let users = repos::users::list(&deps.pool, true).await?;
            let mut items: Vec<Value> = users
                .iter()
                .map(|u| {
                    let mut j = vuuut_core::models::user_public_json(u, None);
                    let url = crate::util::resolve_for_page(&j, lang.map(|l| l.current.as_str()), false, resolve);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            if let Some(f) = filter_expr.as_ref() {
                items.retain(|i| vuuut_nql::evaluate(f, i));
            }
            let total = items.len() as i64;
            let start = ((page.max(1) - 1) * if limit > 0 { limit } else { 15 }) as usize;
            let items: Vec<Value> = if limit < 0 { items } else { items.into_iter().skip(start).take(limit as usize).collect() };
            let pagination = vuuut_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "tiers" => {
            let (items, total) =
                repos::tiers::browse_public(&deps.pool, filter_expr.as_ref(), order, limit, page).await?;
            let pagination =
                vuuut_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "newsletters" => {
            let (items, total) =
                repos::tiers::browse_newsletters_public(&deps.pool, filter_expr.as_ref(), order, limit, page)
                    .await?;
            let pagination =
                vuuut_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "products" => browse_products(deps, filter_expr.as_ref(), order, limit, page).await,
        _ => Ok((Value::Array(vec![]), vuuut_core::models::pagination_json(1, limit.max(1), 0))),
    }
}

/// `{{#get "products"}}`: the store's active products, newest first, as the
/// storefront would list them. Nothing at all while the store is switched
/// off, so a theme's `{{else}}` shows instead.
///
/// Each product is [`vuuut_db::repos::ecom::card_json`]: `title`, `url`,
/// `excerpt`, `vendor`, `product_type`, `tags`, `meta`, `image`, `price`
/// (formatted, `"Free"` at zero), `on_sale`, `sold_out`, `demo_url`, …,
/// plus `collections`: the handles of the published collections it is in.
///
/// `filter` is NQL over those fields; `order` takes `price asc|desc`,
/// `title asc`, `featured` or (default) newest.
async fn browse_products(
    deps: &HelperDeps,
    filter: Option<&vuuut_nql::Expr>,
    order: Option<&str>,
    limit: i64,
    page: i64,
) -> vuuut_core::Result<(Value, Value)> {
    use vuuut_core::ecom as rules;
    use vuuut_db::repos::ecom as repo;

    let per_page = if limit > 0 { limit } else { 15 };
    let empty = || (Value::Array(vec![]), vuuut_core::models::pagination_json(1, per_page, 0));
    let stored: std::collections::HashMap<String, String> =
        vuuut_db::repos::settings::all(&deps.pool).await?.into_iter().map(|s| (s.key, s.value)).collect();
    let settings = rules::Settings::read(|k| stored.get(k).cloned());
    if !settings.enabled {
        return Ok(empty());
    }
    let paths = rules::Paths::from_settings(&settings);
    let order = order.unwrap_or("").to_ascii_lowercase();
    let sort = match order.trim() {
        "price asc" | "price" => "price_asc",
        "price desc" => "price_desc",
        "title asc" | "title" => "title",
        "featured" | "featured desc" => "newest",
        _ => "newest",
    };
    let browse = repo::ProductBrowse { status: Some("active"), sort, limit: 1000, ..Default::default() };
    let (products, _) = repo::browse_products(&deps.pool, &browse).await?;
    let ids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let variants = repo::variants_for(&deps.pool, &ids).await;
    // product id → handles of the published collections it is in, so a
    // theme can ask for `filter="collections:themes"`
    let mut member_of: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for c in repo::collections(&deps.pool).await.into_iter().filter(|c| c.published != 0) {
        for id in repo::collection_product_ids(&deps.pool, &c, true).await {
            member_of.entry(id).or_default().push(c.handle.clone());
        }
    }

    let mut items: Vec<Value> = products
        .iter()
        .map(|p| {
            let vs = variants.get(&p.id).map(Vec::as_slice).unwrap_or_default();
            let url = vuuut_core::url::base(&paths.public_path(&format!("/shop/products/{}/", p.handle)));
            let mut j = repo::card_json(p, vs, &settings, &url);
            j["collections"] = json!(member_of.get(&p.id).cloned().unwrap_or_default());
            j
        })
        .collect();
    if order.starts_with("featured") {
        // stable: featured first, newest order kept within each group
        items.sort_by_key(|i| !i["featured"].as_bool().unwrap_or(false));
    }
    if let Some(f) = filter {
        items.retain(|i| vuuut_nql::evaluate(f, i));
    }
    let total = items.len() as i64;
    let items: Vec<Value> = if limit < 0 {
        items
    } else {
        items.into_iter().skip(((page.max(1) - 1) * per_page) as usize).take(per_page as usize).collect()
    };
    Ok((Value::Array(items), vuuut_core::models::pagination_json(page, per_page, total)))
}


// ---------------------------------------------------------------------------
// prev_post / next_post
// ---------------------------------------------------------------------------

/// The NQL filter Ghost's `prev_post`/`next_post` browse with.
///
/// Ported from `core/frontend/helpers/prev_post.js`, including its asymmetry:
/// the previous post is looked for at `published_at:<=` and the next at `>`,
/// with the current post kept out by slug either way. That is what makes two
/// posts sharing a timestamp still resolve to each other instead of to
/// nothing.
///
/// `in` scopes the search the way Ghost does — `primary_tag`, `primary_author`
/// or `author` — and is ignored when the post has no such relation.
fn neighbour_filter(post: &Value, is_prev: bool, scope: Option<&str>) -> Option<String> {
    let slug = post.get("slug")?.as_str()?;
    let published = post.get("published_at")?.as_str()?;
    // Serialized as `2026-10-24T09:30:58.000Z`, stored as `2026-10-24 09:30:58`
    // — NQL compares them as text, so the two have to be written the same way.
    let published = published.replace('T', " ");
    let published = published.split('.').next().unwrap_or(&published).trim_end_matches('Z');
    let op = if is_prev { "<=" } else { ">" };
    let mut filter = format!("slug:-{slug}+published_at:{op}'{published}'");

    let related = |key: &str| post.get(key).and_then(|v| v.get("slug")).and_then(Value::as_str);
    match scope {
        Some("primary_tag") => {
            if let Some(tag) = related("primary_tag") {
                filter.push_str(&format!("+primary_tag:{tag}"));
            }
        }
        Some("primary_author") => {
            if let Some(author) = related("primary_author") {
                filter.push_str(&format!("+primary_author:{author}"));
            }
        }
        Some("author") => {
            if let Some(author) = related("author") {
                filter.push_str(&format!("+author:{author}"));
            }
        }
        _ => {}
    }
    Some(filter)
}

pub struct PrevNextPost {
    deps: HelperDeps,
    is_prev: bool,
}

impl PrevNextPost {
    pub fn new(deps: HelperDeps, is_prev: bool) -> Self {
        Self { deps, is_prev }
    }
}

impl HelperDef for PrevNextPost {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        let this = current_value(rc, ctx);
        let scope = h.hash_get("in").and_then(|v| v.value().as_str()).map(str::to_string);

        // Ghost renders the inverse for anything that is not a published post
        // — a page, a preview, the home page — rather than nothing at all.
        let is_post = this.get("id").and_then(|v| v.as_str()).is_some()
            && this.get("title").is_some()
            && this.get("page").and_then(Value::as_bool) != Some(true);
        let filter = match (is_post, neighbour_filter(&this, self.is_prev, scope.as_deref())) {
            (true, Some(f)) => f,
            _ => {
                if let Some(t) = h.inverse() {
                    t.render(r, ctx, rc, out)?;
                }
                return Ok(());
            }
        };

        let deps = self.deps.clone();
        let runtime = deps.runtime.clone();
        let order = if self.is_prev { "published_at desc" } else { "published_at asc" };
        // The neighbour in the same language: "next" on a Bengali post is
        // the next Bengali post.
        let lang = page_language(rc, ctx);
        let found: Option<Value> = runtime.block_on(async move {
            let (items, _) =
                run_get(&deps, "posts", &Some(filter), "1", 1, Some(order), Some("authors,tags"), lang.as_ref()).await.ok()?;
            items.as_array().and_then(|a| a.first().cloned())
        });

        if let Some(mut post) = found {
            apply_context_access(&mut post, ctx);
            let mut block = handlebars::BlockContext::new();
            block.set_base_value(post);
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

// ---------------------------------------------------------------------------
// collection
// ---------------------------------------------------------------------------

/// `{{#collection "slug"}}` — a posts browse scoped to a routing collection.
///
/// Ported from `core/frontend/helpers/collection.js`, and faithful to a
/// surprise in it: upstream's `parseOptions` returns **only** a limit, and the
/// collection slug is forwarded to the posts controller as an option that
/// nothing in the model layer reads (it reaches only the cache key). So in
/// Ghost 6.73 this helper renders the publication's latest posts, defaulting
/// to three, and `filter`/`order`/`include` passed to it are discarded. It is
/// kept that way deliberately: a theme written against Ghost gets the same
/// output here. This is separate from YAML routing collections, which select
/// posts per route and work independently of this helper.
pub struct CollectionHelper {
    deps: HelperDeps,
}

impl CollectionHelper {
    pub fn new(deps: HelperDeps) -> Self {
        Self { deps }
    }
}

impl HelperDef for CollectionHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Handlebars<'reg>,
        ctx: &'rc Context,
        rc: &mut RenderContext<'reg, 'rc>,
        out: &mut dyn Output,
    ) -> HelperResult {
        // `limit="all"` and a missing limit both mean three here, not
        // everything — upstream's parseOptions, not the get helper's cap.
        let limit = h
            .hash_get("limit")
            .map(|v| util2::value_string(v.value()))
            .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("all"))
            .map(|l| apply_limit_cap(&l))
            .unwrap_or(3);

        let deps = self.deps.clone();
        let runtime = deps.runtime.clone();
        let lang = page_language(rc, ctx);
        let result: Result<(Value, Value), String> = runtime.block_on(async move {
            run_get(&deps, "posts", &None, &limit.to_string(), 1, None, Some("authors,tags"), lang.as_ref())
                .await
                .map_err(|e| e.message())
        });

        match result {
            Ok((mut items, pagination)) => {
                if let Some(items) = items.as_array_mut() {
                    for item in items {
                        apply_context_access(item, ctx);
                    }
                }
                let mut data = Map::new();
                data.insert("posts".into(), items);
                data.insert("pagination".into(), pagination);
                let mut block = handlebars::BlockContext::new();
                block.set_base_value(Value::Object(data.clone()));
                let items = data.get("posts").cloned().unwrap_or(Value::Array(vec![]));
                let mut params = handlebars::BlockParams::new();
                if let Some((p1, p2)) = h.block_param_pair() {
                    params.add_value(p1, items)?;
                    params.add_value(p2, data.get("pagination").cloned().unwrap_or(Value::Null))?;
                } else if let Some(p1) = h.block_param() {
                    params.add_value(p1, items)?;
                }
                block.set_block_params(params);
                rc.push_block(block);
                if let Some(t) = h.template() {
                    t.render(r, ctx, rc, out)?;
                }
                rc.pop_block();
            }
            Err(_) => {
                if let Some(t) = h.inverse() {
                    t.render(r, ctx, rc, out)?;
                }
            }
        }
        Ok(())
    }
}

// small util for hash values that may be string or number
pub mod util2 {
    pub fn value_string(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn post_with_tags() -> Value {
        json!({
            "post": {
                "title": "One",
                "tags": [{ "slug": "news" }, { "slug": "sport" }],
                "author": { "slug": "jane" },
                "primary_tag": { "slug": "news" },
            }
        })
    }

    #[test]
    fn wildcard_expands_every_match_rather_than_the_first() {
        // The bug this replaces: `[*]` fell back to index 0, so a related-posts
        // block filtered on one tag instead of all of them.
        let found = query_simple_path(&post_with_tags(), "post.tags[*].slug");
        assert_eq!(found, vec![json!("news"), json!("sport")]);
    }

    #[test]
    fn numeric_index_still_selects_one_element() {
        assert_eq!(query_simple_path(&post_with_tags(), "post.tags[1].slug"), vec![json!("sport")]);
    }

    #[test]
    fn a_path_that_matches_nothing_yields_no_values() {
        assert!(query_simple_path(&post_with_tags(), "post.missing.slug").is_empty());
        assert!(query_simple_path(&post_with_tags(), "post.tags[9].slug").is_empty());
    }

    #[test]
    fn tag_alias_expands_to_every_slug_in_a_filter() {
        // `{{post.tags}}` on its own is an array of objects; Ghost's alias is
        // what turns it into something a filter can use.
        let filter = interpolate_filter(
            "tags:[{{post.tags}}]+id:-{{post.id}}",
            &post_with_tags(),
            &Value::Null,
        );
        assert_eq!(filter, "tags:[news,sport]+id:-");
    }

    #[test]
    fn author_alias_resolves_to_the_slug() {
        let filter = interpolate_filter("authors:{{post.author}}", &post_with_tags(), &Value::Null);
        assert_eq!(filter, "authors:jane");
    }

    #[test]
    fn globals_are_read_from_the_root_data_either_spelling() {
        let globals = json!({ "site": { "title": "Flux" } });
        let scope = json!({});
        assert_eq!(interpolate_filter("title:{{@site.title}}", &scope, &globals), "title:Flux");
        // preprocessing rewrites @site to @root.site before parsing
        assert_eq!(interpolate_filter("title:{{@root.site.title}}", &scope, &globals), "title:Flux");
    }

    #[test]
    fn handlebars_bracket_indexing_is_normalised() {
        // Handlebars writes array indexing as `.[0]`; the query form drops the
        // dot. Upstream's `path.replace(/\.\[/g, '[')` does exactly this.
        let scope = json!({ "post": { "tags": [{ "slug": "news" }] } });
        assert_eq!(interpolate_filter("x:{{post.tags.[0].slug}}", &scope, &Value::Null), "x:news");
    }

    #[test]
    fn an_unsupported_path_segment_resolves_to_nothing() {
        // Upstream raises IncorrectUsageError on a segment outside
        // `^\w+(\[(\*|\d+)\])?$`. Either way the theme gets no value rather
        // than a silently wrong one — the old code took element 0 here.
        let scope = json!({ "post": { "my-key": "value" } });
        assert!(query_simple_path(&scope, "post[my-key]").is_empty());
        assert_eq!(interpolate_filter("x:{{post.[my-key]}}", &scope, &Value::Null), "x:");
    }

    #[test]
    fn text_around_and_without_interpolation_is_preserved() {
        assert_eq!(interpolate_filter("featured:true", &json!({}), &Value::Null), "featured:true");
        // an unterminated mustache must not eat the rest of the filter
        assert_eq!(interpolate_filter("a{{b", &json!({}), &Value::Null), "a{{b");
    }

    #[test]
    fn limit_is_capped_at_ghosts_maximum() {
        assert_eq!(apply_limit_cap("5"), 5);
        assert_eq!(apply_limit_cap("100"), 100);
        assert_eq!(apply_limit_cap("500"), MAX_LIMIT);
        // `all` is no longer a whole-table fetch in Ghost 6
        assert_eq!(apply_limit_cap("all"), MAX_LIMIT);
        assert_eq!(apply_limit_cap("ALL"), MAX_LIMIT);
        assert_eq!(apply_limit_cap("nonsense"), MAX_LIMIT);
        assert_eq!(apply_limit_cap(""), 15);
    }

    #[test]
    fn fields_narrows_the_record_but_keeps_url() {
        let mut post = json!({ "id": "1", "title": "T", "html": "<p>x</p>", "url": "/t/" });
        apply_fields(&mut post, Some("title"), "posts");
        let obj = post.as_object().unwrap();
        assert_eq!(obj.len(), 2, "title and the serializer-attached url");
        assert_eq!(obj["title"], json!("T"));
        assert_eq!(obj["url"], json!("/t/"), "{{url}} in the block must still resolve");
        assert!(!obj.contains_key("html"), "the body was not asked for");
    }

    #[test]
    fn a_requested_field_with_no_value_stays_present() {
        let mut tier = json!({ "id": "1", "name": "Gold" });
        apply_fields(&mut tier, Some("name,description"), "tiers");
        assert_eq!(tier["description"], Value::Null);
        assert!(!tier.as_object().unwrap().contains_key("url"), "tiers have no url");
    }

    #[test]
    fn formats_drops_the_bodies_a_theme_did_not_ask_for() {
        let mut post = json!({ "title": "T", "html": "<p>x</p>", "plaintext": "x" });
        apply_formats(&mut post, Some("plaintext"));
        let obj = post.as_object().unwrap();
        assert!(!obj.contains_key("html"));
        assert_eq!(obj["plaintext"], json!("x"));
        assert_eq!(obj["title"], json!("T"), "formats only governs content formats");
    }

    #[test]
    fn plaintext_is_derived_when_the_record_carries_only_html() {
        // The serialized post has no `plaintext` key — it is a column the
        // excerpt comes from, not a serialized field — so asking for it alone
        // used to drop the html and supply nothing in its place.
        let mut post = json!({ "title": "T", "html": "<p>Hello <b>world</b></p>" });
        apply_formats(&mut post, Some("plaintext"));
        let obj = post.as_object().unwrap();
        assert!(!obj.contains_key("html"), "not asked for");
        assert!(
            obj["plaintext"].as_str().is_some_and(|p| p.contains("Hello") && p.contains("world")),
            "got {:?}",
            obj["plaintext"]
        );
    }

    #[test]
    fn asking_for_both_formats_keeps_both() {
        let mut post = json!({ "html": "<p>x</p>" });
        apply_formats(&mut post, Some("html,plaintext"));
        let obj = post.as_object().unwrap();
        assert_eq!(obj["html"], json!("<p>x</p>"));
        assert!(obj.contains_key("plaintext"));
    }

    #[test]
    fn a_record_with_no_html_is_not_given_an_empty_plaintext() {
        let mut tier = json!({ "name": "Gold" });
        apply_formats(&mut tier, Some("plaintext"));
        assert!(!tier.as_object().unwrap().contains_key("plaintext"), "nothing to derive it from");
    }

    #[test]
    fn no_formats_option_leaves_the_record_alone() {
        let mut post = json!({ "html": "<p>x</p>", "plaintext": "x" });
        apply_formats(&mut post, None);
        assert_eq!(post.as_object().unwrap().len(), 2);
    }

    #[test]
    fn the_resource_list_matches_upstream() {
        for resource in ["posts", "pages", "tags", "authors", "tiers", "newsletters"] {
            assert!(RESOURCES.contains(&resource), "{resource} is a public Ghost resource");
        }
        // Ghost has no public members controller; asking renders {{else}}
        assert!(!RESOURCES.contains(&"members"));
    }
}
