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

        // options from hash
        let limit_raw = h.hash_get("limit").map(|v| crate::get_helper::util2::value_string(v.value())).unwrap_or_default();
        let page: i64 = h.hash_get("page").and_then(|v| v.value().as_i64()).unwrap_or(1);
        let order = h.hash_get("order").and_then(|v| v.value().as_str()).map(str::to_string);
        let include = h.hash_get("include").and_then(|v| v.value().as_str()).map(str::to_string);
        // filter supports {{path}} interpolation against the current scope
        let filter_raw = h.hash_get("filter").and_then(|v| v.value().as_str()).map(str::to_string);

        let deps = self.deps.clone();
        let this = current_value(rc, ctx);
        let filter = filter_raw.map(|f| interpolate_filter(&f, &this));
        let resource_for_query = resource.clone();

        let runtime = deps.runtime.clone();
        let result: Result<(Value, Value), String> = runtime.block_on(async move {
            run_get(&deps, &resource_for_query, &filter, &limit_raw, page, order.as_deref(), include.as_deref())
                .await
                .map_err(|e| e.message())
        });

        match result {
            Ok((items, pagination)) => {
                let mut data = Map::new();
                data.insert(resource.clone(), items);
                data.insert("pagination".into(), pagination);
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

/// Replace `{{path.to.value}}` inside a filter with the value from scope.
fn interpolate_filter(filter: &str, scope: &Value) -> String {
    let mut out = filter.to_string();
    // find {{...}} occurrences
    while let Some(start) = out.find("{{") {
        let Some(end_rel) = out[start..].find("}}") else { break };
        let end = start + end_rel;
        let path = &out[start + 2..end];
        let val = walk(scope, path);
        let replacement = match val {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        };
        out = format!("{}{}{}", &out[..start], replacement, &out[end + 2..]);
    }
    out
}

fn walk(v: &Value, path: &str) -> Value {
    let mut cur = v.clone();
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = if seg.contains('[') {
            // post.tags[0].slug style
            let (field, rest) = seg.split_once('[').unwrap_or((seg, ""));
            let idx: usize = rest.trim_end_matches(']').parse().unwrap_or(0);
            cur.get(field)
                .and_then(|f| f.as_array())
                .and_then(|a| a.get(idx))
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            cur.get(seg).cloned().unwrap_or(Value::Null)
        };
    }
    cur
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
) -> ghost_core::Result<(Value, Value)> {
    use ghost_db::repos;

    let limit: i64 = if limit_raw == "all" { -1 } else { limit_raw.parse().unwrap_or(15) };
    let filter_expr = match filter {
        Some(f) if !f.trim().is_empty() => Some(ghost_nql::parse(f)?),
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
            };
            let res = repos::posts::browse(&deps.pool, &q).await?;
            let mut items = Vec::new();
            for p in &res.posts {
                let rel = repos::posts::load_relations(&deps.pool, std::slice::from_ref(p)).await?;
                let authors = rel.authors_for(&p.id);
                let tags = rel.tags_for(&p.id);
                let mut j = ghost_core::models::post_to_json(p, &authors, &tags, "");
                j["url"] = json!(resolve(&j));
                crate::util::resolve_nested_urls(&mut j, resolve);
                items.push(j);
            }
            let pagination = ghost_core::models::pagination_json(res.page, res.limit.max(1), res.total);
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
            let items: Vec<Value> = tags
                .iter()
                .map(|t| {
                    let mut j = ghost_core::models::tag_to_json(t, None);
                    let url = resolve(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            let pagination = ghost_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "authors" | "users" => {
            let users = repos::users::list(&deps.pool, true).await?;
            let mut items: Vec<Value> = users
                .iter()
                .map(|u| {
                    let mut j = ghost_core::models::user_public_json(u, None);
                    let url = resolve(&j);
                    if let Value::Object(m) = &mut j {
                        m.insert("url".into(), json!(url));
                    }
                    j
                })
                .collect();
            if let Some(f) = filter_expr.as_ref() {
                items.retain(|i| ghost_nql::evaluate(f, i));
            }
            let total = items.len() as i64;
            let start = ((page.max(1) - 1) * if limit > 0 { limit } else { 15 }) as usize;
            let items: Vec<Value> = if limit < 0 { items } else { items.into_iter().skip(start).take(limit as usize).collect() };
            let pagination = ghost_core::models::pagination_json(page, if limit > 0 { limit } else { 15 }, total);
            Ok((Value::Array(items), pagination))
        }
        "members" => {
            // members are deferred: empty result renders {{else}}
            Ok((Value::Array(vec![]), ghost_core::models::pagination_json(1, limit.max(1), 0)))
        }
        _ => Ok((Value::Array(vec![]), ghost_core::models::pagination_json(1, limit.max(1), 0))),
    }
}

// ---------------------------------------------------------------------------
// prev_post / next_post
// ---------------------------------------------------------------------------

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
        let Some(post_id) = this.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            return Ok(());
        };
        let deps = self.deps.clone();
        let is_prev = self.is_prev;

        let found: Option<Value> = deps.runtime.block_on(async move {
            use ghost_db::repos;
            let Ok(Some(post)) = repos::posts::find_by_id(&deps.pool, &post_id).await else {
                return None;
            };
            let (prev, next) = repos::posts::neighbours(&deps.pool, &post).await.ok()?;
            let target = if is_prev { prev } else { next };
            let p = target?;
            let rel = repos::posts::load_relations(&deps.pool, std::slice::from_ref(&p)).await.ok()?;
            let authors = rel.authors_for(&p.id);
            let tags = rel.tags_for(&p.id);
            let mut j = ghost_core::models::post_to_json(&p, &authors, &tags, "");
            let url = (deps.url_resolver)(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            crate::util::resolve_nested_urls(&mut j, &*deps.url_resolver);
            Some(j)
        });

        if let Some(post) = found {
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
