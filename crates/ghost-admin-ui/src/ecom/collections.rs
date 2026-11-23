//! Collections: groups of products for the store to browse by.
//!
//! A manual collection is a hand-picked, hand-ordered list; a smart one
//! holds every product matching its conditions, re-evaluated whenever it is
//! read, so a new product with the right tag simply appears in it.

use super::*;

const COLLS: &str = "/ghost/ecom/collections/";

pub(crate) async fn collections_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let colls = repo::collections(&state.db).await;
    let mut rows = String::new();
    for c in &colls {
        let n = repo::collection_product_ids(&state.db, c, false).await.len() as i64;
        rows.push_str(&format!(
            r#"<tr><td><a class="ec-prod" href="{COLLS}{id}/">{thumb}<span><b>{t}</b><small class="muted">{public}</small></span></a></td>
<td>{kind}</td><td>{n}</td><td>{vis}</td></tr>"#,
            id = esc(&c.id),
            thumb = thumb(&c.image),
            t = esc(&c.title),
            public = esc(&ghost_api::ecom::paths(&state).public_path(&format!("/shop/collections/{}/", c.handle))),
            kind = if c.is_smart() { format!(r#"<span class="chip tonal">Smart · {}</span>"#, countable(c.rule_list().len() as i64, "condition")) } else { r#"<span class="chip">Manual</span>"#.into() },
            n = countable(n, "product"),
            vis = if c.published != 0 { r#"<span class="chip published">Visible</span>"# } else { r#"<span class="chip draft">Hidden</span>"# },
        ));
    }
    let body = format!(
        r#"<div class="card ec-table-card rise" style="--i:2">{}</div>"#,
        if colls.is_empty() {
            format!(
                r#"<div class="ec-empty">{ic}<p><b>Group your products into collections</b></p><p class="muted">Pick products by hand, or let a smart collection gather them by tag, type, vendor or price.</p><a class="btn" href="{COLLS}new/">{add} Create collection</a></div>"#,
                ic = svg("folder"),
                add = svg("add")
            )
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th>Collection</th><th>Type</th><th>Products</th><th>Store</th></tr></thead><tbody>{rows}</tbody></table>"#)
        }
    );
    ecom_page(&actor, "Collections", "collections", &q, &format!(r#"<a class="btn" href="{COLLS}new/">{} Create collection</a>"#, svg("add")), Some(BASE), &body)
}

pub(crate) async fn collection_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    collection_editor(&state, &actor, None, &q).await
}

pub(crate) async fn collection_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(c) = valid_id(&id).then_some(()).and(repo::find_collection(&state.db, &id).await) else {
        return not_found_page_for(actor.role, &actor.user, "That collection could not be found.");
    };
    collection_editor(&state, &actor, Some(c), &q).await
}

fn rule_row(r: Option<&rules::Rule>, s: &Settings) -> String {
    let fields: String = rules::RuleField::ALL.iter().map(|f| format!(r#"<option value="{}"{}>{}</option>"#, f.key(), if r.is_some_and(|r| r.field == *f) { " selected" } else { "" }, f.label())).collect();
    let ops: String = rules::RuleOp::ALL.iter().map(|o| format!(r#"<option value="{}"{}>{}</option>"#, o.key(), if r.is_some_and(|r| r.op == *o) { " selected" } else { "" }, o.label())).collect();
    let value = match r {
        Some(r) if r.field == rules::RuleField::Price => rules::money_input(r.value.parse().unwrap_or(0), &s.currency),
        Some(r) => r.value.clone(),
        None => String::new(),
    };
    format!(
        r#"<div class="ec-rule"><select class="ec-in" name="rule_field" aria-label="Field">{fields}</select><select class="ec-in" name="rule_op" aria-label="Condition">{ops}</select>
<input class="ec-in" name="rule_value" value="{v}" aria-label="Value" maxlength="80"><button class="icon-btn" type="button" data-remove-rule title="Remove condition">{del}</button></div>"#,
        v = esc(&value),
        del = svg("delete"),
    )
}

async fn collection_editor(state: &AppState, actor: &Actor, c: Option<repo::Collection>, q: &HashMap<String, String>) -> Response {
    let s = store_settings(state);
    let get = |f: fn(&repo::Collection) -> &str| c.as_ref().map(f).unwrap_or("").to_string();
    let smart = c.as_ref().map(|c| c.is_smart()).unwrap_or(q.get("kind").map(String::as_str) == Some("smart"));
    let rules_list = c.as_ref().map(|c| c.rule_list()).unwrap_or_default();
    let action = match &c {
        Some(c) => format!("{COLLS}{}/", esc(&c.id)),
        None => format!("{COLLS}new/"),
    };
    let sort_opts: String = rules::SORTS.iter().map(|(k, l)| format!(r#"<option value="{k}"{s}>{l}</option>"#, s = if get(|c| &c.sort) == *k { " selected" } else { "" })).collect();
    let members = match &c {
        Some(c) => repo::collection_product_ids(&state.db, c, false).await,
        None => Vec::new(),
    };
    let mut products = repo::products_by_ids(&state.db, &members).await;
    products.sort_by_key(|p| members.iter().position(|m| m == &p.id).unwrap_or(usize::MAX));
    let product_list: String = products
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let manual_tools = if c.as_ref().is_some_and(|c| !c.is_smart()) {
                format!(
                    r#"<form method="post" action="{action}products/"><input type="hidden" name="product" value="{id}"><button class="icon-btn" name="op" value="up" title="Move up"{d1}>{up}</button><button class="icon-btn" name="op" value="down" title="Move down"{d2}>{down}</button><button class="icon-btn" name="op" value="remove" title="Remove from collection">{x}</button></form>"#,
                    id = esc(&p.id),
                    d1 = if i == 0 { " disabled" } else { "" },
                    d2 = if i + 1 == products.len() { " disabled" } else { "" },
                    up = svg("up"),
                    down = svg("down"),
                    x = svg("close"),
                )
            } else {
                String::new()
            };
            format!(
                r#"<div class="row"><a class="ec-prod" href="/ghost/ecom/products/{id}/">{thumb}<span><b>{t}</b><small class="muted">{st}</small></span></a><div class="trailing ec-row-tools">{manual_tools}</div></div>"#,
                id = esc(&p.id),
                thumb = thumb(&p.first_image().unwrap_or_default()),
                t = esc(&p.title),
                st = esc(rules::ProductStatus::from_key(&p.status).label()),
            )
        })
        .collect();
    let add_box = match &c {
        Some(col) if !col.is_smart() => {
            let (all, _) = repo::browse_products(&state.db, &repo::ProductBrowse { sort: "title", limit: 1000, ..Default::default() }).await.unwrap_or_default();
            let opts: String = all.iter().filter(|p| !members.contains(&p.id)).map(|p| format!(r#"<option value="{}">{}</option>"#, esc(&p.id), esc(&p.title))).collect();
            if opts.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<form class="ec-row ec-addprod" method="post" action="{action}products/"><input type="hidden" name="op" value="add"><label class="field fixed" style="margin:0;flex:1"><select name="product">{opts}</select><span>Add a product</span>{SELECT_CARET}</label><button class="btn tonal" type="submit">{} Add</button></form>"#,
                    svg("add")
                )
            }
        }
        _ => String::new(),
    };
    let kind_picker = if c.is_some() {
        format!(r#"<input type="hidden" name="kind" value="{}">"#, if smart { "smart" } else { "manual" })
    } else {
        format!(
            r#"<div class="ec-kinds"><label class="ec-kind"><input type="radio" name="kind" value="manual"{m}><b>Manual</b><span>Add products one by one, in your order.</span></label>
<label class="ec-kind"><input type="radio" name="kind" value="smart"{sm}><b>Smart</b><span>Products matching conditions join automatically.</span></label></div>"#,
            m = if smart { "" } else { " checked" },
            sm = if smart { " checked" } else { "" },
        )
    };
    let conditions = format!(
        r#"<div class="ec-only-smart"><div class="ec-sublabel">Conditions</div>
<div class="ec-match"><label class="ec-check"><input type="radio" name="match_all" value="1"{all}> all conditions</label><label class="ec-check"><input type="radio" name="match_all" value="0"{any}> any condition</label></div>
<div id="ec-rules">{rows}</div><template id="ec-rule-template">{blank}</template>
<button class="btn text small" type="button" id="ec-add-rule">{add} Add another condition</button></div>"#,
        all = if c.as_ref().map(|c| c.match_all != 0).unwrap_or(true) { " checked" } else { "" },
        any = if c.as_ref().is_some_and(|c| c.match_all == 0) { " checked" } else { "" },
        rows = if rules_list.is_empty() { rule_row(None, &s) } else { rules_list.iter().map(|r| rule_row(Some(r), &s)).collect() },
        blank = rule_row(None, &s),
        add = svg("add"),
    );
    let products_card = if c.is_some() {
            format!(
                r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-pink">{bag}</span>Products <span class="muted">· {n}</span></h2>{add_box}<div class="rows ec-coll-products">{list}</div></section>"#,
                bag = svg("bag"),
                n = members.len(),
                list = if product_list.is_empty() { r#"<p class="muted">No products yet.</p>"#.to_string() } else { product_list },
            )
        } else {
            String::new()
        };
    let main = format!(
        r#"<section class="card padded ec-card">{title}<label class="field"><textarea name="description" rows="4" placeholder=" " maxlength="2000">{desc}</textarea><span>Description</span></label>
<div class="ec-sublabel">Collection type</div>{kind_picker}{conditions}</section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-slate">{search}</span>Search engine listing</h2>{seo_t}{seo_d}{handle}</section>"#,
        title = field("title", "Title", &get(|c| &c.title), r#" required maxlength="255""#),
        desc = esc(&get(|c| &c.description)),
        search = svg("search"),
        seo_t = field("seo_title", "Page title", &get(|c| &c.seo_title), r#" maxlength="120""#),
        seo_d = field("seo_description", "Meta description", &get(|c| &c.seo_description), r#" maxlength="320""#),
        handle = field("handle", "URL handle", &get(|c| &c.handle), r#" maxlength="120""#),
    );
    let side = format!(
        r#"<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-green">{eye}</span>Store</h2>{pub_}
{view}</section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-pink">{img}</span>Image</h2>{image}{preview}</section>
<section class="card padded ec-card"><h2 class="ec-card-title"><span class="ic tint-blue">{sort_ic}</span>Sort products</h2><label class="field fixed"><select name="sort">{sort_opts}</select><span>Order on the store</span>{SELECT_CARET}</label></section>
{delete}"#,
        eye = svg("eye"),
        pub_ = check_box("published", "Visible on the store", c.as_ref().map(|c| c.published != 0).unwrap_or(true)),
        view = c.as_ref().map(|c| format!(r#"<a class="btn text" href="{}" target="_blank" rel="noopener">{} View</a>"#, ghost_core::url::base(&ghost_api::ecom::paths(&state).public_path(&format!("/shop/collections/{}/", c.handle))), svg("open"))).unwrap_or_default(),
        img = svg("image"),
        image = field("image", "Image URL", &get(|c| &c.image), r#" maxlength="500""#),
        preview = if get(|c| &c.image).is_empty() { String::new() } else { format!(r#"<img class="ec-coll-img" src="{}" alt="">"#, esc(&get(|c| &c.image))) },
        sort_ic = svg("tune"),
        delete = c.as_ref().map(|_| format!(r#"<section class="card padded ec-card"><button class="btn text danger" type="submit" form="ec-del">{} Delete collection</button></section>"#, svg("delete"))).unwrap_or_default(),
    );
    let heading = c.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "Create collection".into());
    let body = format!(
        r#"{del_form}<form id="ec-collection" class="ec-edit{smart_cls}" method="post" action="{action}"><div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<div class="ec-below">{products_card}</div>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        del_form = c.as_ref().map(|c| format!(r#"<form id="ec-del" method="post" action="{COLLS}{}/delete/" data-confirm="Delete collection|The products stay; only the grouping goes."></form>"#, esc(&c.id))).unwrap_or_default(),
        smart_cls = if smart { " is-smart" } else { "" },
    );
    let head = crate::tools::tool_head_back("ecom", COLLS, Some("ec-collection"), "");
    page_for_role(
        actor.role,
        &heading,
        "tools",
        Some(&actor.user),
        &format!(r#"<div class="ec">{head}<h2 class="ec-subhead">{h}</h2>{banner}{body}</div>"#, h = esc(&heading), banner = flash_banner(q)),
        true,
    )
    .into_response()
}

fn field(name: &str, label: &str, value: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn check_box(name: &str, label: &str, on: bool) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="1"{c}> {l}</label>"#, c = if on { " checked" } else { "" }, l = esc(label))
}

fn read_collection(f: &[(String, String)], s: &Settings) -> Result<repo::CollectionInput, String> {
    let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let all = |k: &str| f.iter().filter(|(x, _)| x == k).map(|(_, v)| v.clone()).collect::<Vec<_>>();
    let title = rules::clean(&get("title"), 255);
    if title.is_empty() {
        return Err("Give the collection a title.".into());
    }
    let kind = if get("kind") == "smart" { "smart" } else { "manual" };
    let (fs, os, vs) = (all("rule_field"), all("rule_op"), all("rule_value"));
    let triples: Vec<(String, String, String)> = fs.iter().enumerate().map(|(i, f)| (f.clone(), os.get(i).cloned().unwrap_or_default(), vs.get(i).cloned().unwrap_or_default())).collect();
    let rules_list = if kind == "smart" { rules::build_rules(&triples, &s.currency) } else { Vec::new() };
    if kind == "smart" && rules_list.is_empty() {
        return Err("A smart collection needs at least one condition with a value.".into());
    }
    let handle_raw = get("handle");
    let image = rules::clean_images(std::iter::once(get("image").as_str())).into_iter().next().unwrap_or_default();
    Ok(repo::CollectionInput {
        handle: rules::handleize(if handle_raw.trim().is_empty() { &title } else { &handle_raw }),
        title,
        description: rules::clean_multiline(&get("description"), 2000),
        image,
        kind: kind.into(),
        rules: rules_list,
        match_all: get("match_all") != "0",
        sort: rules::sort_key(&get("sort")).into(),
        published: f.iter().any(|(k, _)| k == "published"),
        seo_title: rules::clean(&get("seo_title"), 120),
        seo_description: rules::clean(&get("seo_description"), 320),
    })
}

pub(crate) async fn collection_create(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let s = store_settings(&state);
    match read_collection(&f, &s) {
        Err(e) => flash_to(&format!("{COLLS}new/"), "error", &e),
        Ok(c) => match repo::save_collection(&state.db, None, &c).await {
            Ok(id) => flash_to(&format!("{COLLS}{id}/"), "ok", "Collection created."),
            Err(e) => flash_to(&format!("{COLLS}new/"), "error", &e.to_string()),
        },
    }
}

pub(crate) async fn collection_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<Vec<(String, String)>>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let Some(existing) = valid_id(&id).then_some(()).and(repo::find_collection(&state.db, &id).await) else {
        return flash_to(COLLS, "error", "That collection no longer exists.");
    };
    let s = store_settings(&state);
    let back = format!("{COLLS}{id}/");
    match read_collection(&f, &s) {
        Err(e) => flash_to(&back, "error", &e),
        Ok(mut c) => {
            // The type is chosen once; switching would silently drop a hand-made list.
            c.kind = existing.kind.clone();
            if existing.is_smart() && c.rules.is_empty() {
                return flash_to(&back, "error", "A smart collection needs at least one condition with a value.");
            }
            match repo::save_collection(&state.db, Some(&id), &c).await {
                Ok(_) => flash_to(&back, "ok", "Collection saved."),
                Err(e) => flash_to(&back, "error", &e.to_string()),
            }
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct MemberForm {
    #[serde(default)]
    product: String,
    #[serde(default)]
    op: String,
}

pub(crate) async fn collection_products(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<MemberForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let Some(c) = valid_id(&id).then_some(()).and(repo::find_collection(&state.db, &id).await).filter(|c| !c.is_smart()) else {
        return flash_to(COLLS, "error", "That collection cannot be edited by hand.");
    };
    let back = format!("{COLLS}{id}/");
    if !valid_id(&f.product) {
        return Redirect::to(&back).into_response();
    }
    let mut ids = repo::manual_ids(&state.db, &c.id).await;
    let pos = ids.iter().position(|x| x == &f.product);
    match (f.op.as_str(), pos) {
        ("add", None) => {
            if repo::find_product(&state.db, &f.product).await.ok().flatten().is_some() {
                ids.push(f.product.clone());
            }
        }
        ("remove", Some(i)) => {
            ids.remove(i);
        }
        ("up", Some(i)) if i > 0 => ids.swap(i, i - 1),
        ("down", Some(i)) if i + 1 < ids.len() => ids.swap(i, i + 1),
        _ => return Redirect::to(&back).into_response(),
    }
    match repo::set_manual_ids(&state.db, &c.id, &ids).await {
        Ok(_) => Redirect::to(&back).into_response(),
        Err(e) => flash_to(&back, "error", &e.to_string()),
    }
}

pub(crate) async fn collection_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if valid_id(&id) {
        let _ = repo::delete_collection(&state.db, &id).await;
    }
    flash_to(COLLS, "ok", "Collection deleted.")
}
