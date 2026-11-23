//! Products: the catalogue list and the product editor.
//!
//! The editor is the post editor's layout made for a product: the title and
//! a Koenig canvas for the description on the left (the same vendored
//! bundle, mounted by `assets/ecom-editor.js`), then media, pricing,
//! inventory, shipping, options and variants, and search listing; status,
//! organisation and collections on the right.
//!
//! The form posts every field, so it saves without JavaScript too — the
//! description then falls back to a plain textarea. Variants are reconciled
//! on the server: the options decide which combinations exist; each keeps
//! the row submitted for it, else the stored variant with the same values,
//! else the product's default price and stock.

use super::*;
use ghost_core::ecom::ProductStatus;

const PRODUCTS: &str = "/ghost/ecom/products/";

pub(crate) async fn products_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let status = q.get("status").map(String::as_str).filter(|v| matches!(*v, "active" | "draft" | "archived"));
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let (vendors, types) = repo::vendors_and_types(&state.db).await;
    let vendor = q.get("vendor").filter(|v| vendors.contains(v)).cloned();
    let ptype = q.get("type").filter(|v| types.contains(v)).cloned();
    let sort = q.get("sort").map(String::as_str).filter(|v| matches!(*v, "title" | "price_asc" | "price_desc" | "best" | "updated" | "newest")).unwrap_or("updated");
    let page_no: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1).clamp(1, 10_000);
    let per = 50;
    let (items, total) = repo::browse_products(
        &state.db,
        &repo::ProductBrowse { terms: &terms, status, vendor: vendor.as_deref(), product_type: ptype.as_deref(), sort, limit: per, offset: (page_no - 1) * per, ..Default::default() },
    )
    .await
    .unwrap_or_default();
    let counts = repo::product_counts(&state.db).await;
    let ids: Vec<String> = items.iter().map(|p| p.id.clone()).collect();
    let vs = repo::variants_for(&state.db, &ids).await;
    let digital = repo::digital_ids(&state.db, &ids).await;
    let rows: String = items
        .iter()
        .map(|p| {
            let v = vs.get(&p.id).cloned().unwrap_or_default();
            let tracked: Vec<&repo::Variant> = v.iter().filter(|x| x.tracked()).collect();
            let stock: i64 = tracked.iter().map(|x| x.inventory).sum();
            let inv = if tracked.is_empty() {
                r#"<span class="muted">Not tracked</span>"#.to_string()
            } else {
                let cls = if stock <= 0 { "ec-neg" } else if stock <= s.low_stock { "ec-low" } else { "" };
                format!(r#"<span class="{cls}">{stock} in stock</span>{}"#, if v.len() > 1 { format!(r#" <span class="muted">for {} variants</span>"#, v.len()) } else { String::new() })
            };
            let min = v.iter().map(|x| x.price_cents).min().unwrap_or(0);
            let max = v.iter().map(|x| x.price_cents).max().unwrap_or(0);
            let st = ProductStatus::from_key(&p.status);
            format!(
                r#"<tr><td class="ec-cb"><input type="checkbox" name="id" value="{id}" form="ec-bulk" aria-label="Select {t}"></td>
<td><a class="ec-prod" href="{PRODUCTS}{id}/">{thumb}<span><b>{t}</b>{feat}</span></a></td>
<td><span class="chip {chip}">{st}</span></td><td>{inv}</td><td class="ec-num">{price}</td><td>{ty}</td><td>{ve}</td></tr>"#,
                id = esc(&p.id),
                t = esc(&p.title),
                thumb = thumb(&p.first_image().unwrap_or_default()),
                feat = format!("{}{}", if p.featured != 0 { r#" <span class="chip tonal">Featured</span>"# } else { "" }, if digital.contains(&p.id) { r#" <span class="chip">Digital</span>"# } else { "" }),
                chip = match st { ProductStatus::Active => "published", ProductStatus::Draft => "scheduled", ProductStatus::Archived => "draft" },
                st = st.label(),
                price = if max > min { format!("{} – {}", money(min, &s), money(max, &s)) } else { money(min, &s) },
                ty = esc(&p.product_type),
                ve = esc(&p.vendor),
            )
        })
        .collect();
    let status_tab = |key: &str, label: &str, n: i64| {
        let on = status.unwrap_or("") == key;
        let href = if key.is_empty() { PRODUCTS.to_string() } else { format!("{PRODUCTS}?status={key}") };
        format!(r#"<a class="ec-subtab{}" href="{href}">{label} <span>{n}</span></a>"#, if on { " on" } else { "" })
    };
    let opt = |list: &[String], cur: &Option<String>, all: &str| -> String {
        std::iter::once(format!(r#"<option value="">{all}</option>"#))
            .chain(list.iter().map(|v| format!(r#"<option value="{e}"{s}>{e}</option>"#, e = esc(v), s = if cur.as_deref() == Some(v.as_str()) { " selected" } else { "" })))
            .collect()
    };
    let sorts = [("updated", "Recently updated"), ("newest", "Newest"), ("title", "Title A–Z"), ("price_asc", "Lowest price"), ("price_desc", "Highest price"), ("best", "Best selling")]
        .iter()
        .map(|(k, l)| format!(r#"<option value="{k}"{s}>{l}</option>"#, s = if *k == sort { " selected" } else { "" }))
        .collect::<String>();
    let pages = (total + per - 1) / per;
    let pager = if pages > 1 {
        let mk = |n: i64| {
            let mut p: Vec<String> = q.iter().filter(|(k, _)| k.as_str() != "page" && k.as_str() != "ok" && k.as_str() != "error").map(|(k, v)| format!("{}={}", esc(k), ghost_core::social_media::encode(v))).collect();
            p.push(format!("page={n}"));
            format!("?{}", p.join("&amp;"))
        };
        format!(
            r#"<nav class="ec-pager">{}<span>Page {page_no} of {pages}</span>{}</nav>"#,
            if page_no > 1 { format!(r#"<a class="btn text" href="{}">Previous</a>"#, mk(page_no - 1)) } else { String::new() },
            if page_no < pages { format!(r#"<a class="btn text" href="{}">Next</a>"#, mk(page_no + 1)) } else { String::new() }
        )
    } else {
        String::new()
    };
    let all: i64 = counts.values().sum();
    let body = format!(
        r#"<div class="ec-subtabs rise" style="--i:2">{t_all}{t_active}{t_draft}{t_arch}</div>
<form class="ec-filter rise" style="--i:2" method="get" action="{PRODUCTS}">{keep}
<label class="field" style="margin:0;flex:2"><input type="search" name="q" value="{q}" placeholder=" " maxlength="100"><span>Search title, SKU, tag, vendor</span></label>
<label class="field fixed" style="margin:0"><select name="vendor">{vo}</select><span>Vendor</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="type">{to}</select><span>Type</span>{caret}</label>
<label class="field fixed" style="margin:0"><select name="sort">{sorts}</select><span>Sort</span>{caret}</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<form id="ec-bulk" method="post" action="{PRODUCTS}bulk/" data-confirm-delete="Delete products|The selected products will be permanently deleted. Past orders keep their copies."></form>
<div class="ec-bulkbar rise" style="--i:3"><span class="muted">{count}</span>
<select name="action" form="ec-bulk" aria-label="Bulk action"><option value="">With selected…</option><option value="active">Set as active</option><option value="draft">Set as draft</option><option value="archived">Archive</option><option value="delete">Delete</option></select>
<button class="btn text small" type="submit" form="ec-bulk">Apply</button></div>
<div class="card ec-table-card rise" style="--i:3">{table}</div>{pager}"#,
        t_all = status_tab("", "All", all),
        t_active = status_tab("active", "Active", counts.get("active").copied().unwrap_or(0)),
        t_draft = status_tab("draft", "Draft", counts.get("draft").copied().unwrap_or(0)),
        t_arch = status_tab("archived", "Archived", counts.get("archived").copied().unwrap_or(0)),
        keep = status.map(|s| format!(r#"<input type="hidden" name="status" value="{s}">"#)).unwrap_or_default(),
        q = esc(&query),
        vo = opt(&vendors, &vendor, "All vendors"),
        to = opt(&types, &ptype, "All types"),
        caret = SELECT_CARET,
        search = svg("search"),
        count = esc(&countable(total, "product")),
        table = if items.is_empty() {
            format!(
                r#"<div class="ec-empty">{ic}<p><b>{h}</b></p><p class="muted">{d}</p><a class="btn" href="{PRODUCTS}new/">{add} Add product</a></div>"#,
                ic = svg("bag"),
                h = if all == 0 { "Add your first product" } else { "No products match" },
                d = if all == 0 { "Products are what you sell: give each a title, photos, a description and a price." } else { "Try another search or filter." },
                add = svg("add"),
            )
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th class="ec-cb"><input type="checkbox" data-check-all aria-label="Select all"></th><th>Product</th><th>Status</th><th>Inventory</th><th class="ec-num">Price</th><th>Type</th><th>Vendor</th></tr></thead><tbody>{rows}</tbody></table>"#)
        },
    );
    ecom_page(
        &actor,
        "Products",
        "products",
        &q,
        &format!(r#"<a class="btn text" href="{PRODUCTS}import/">{} Import</a><a class="btn text" href="{PRODUCTS}export.csv">{} Export</a><a class="btn" href="{PRODUCTS}new/">{} Add product</a>"#, svg("upload"), svg("import"), svg("add")),
        Some(BASE),
        &format!(r#"{body}<script src="{ASSETS}/ecom-editor.js" defer></script>"#),
    )
}

pub(crate) async fn products_bulk(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<Vec<(String, String)>>) -> Response {
    let _actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let action = f.iter().find(|(k, _)| k == "action").map(|(_, v)| v.as_str()).unwrap_or("");
    let ids: Vec<&String> = f.iter().filter(|(k, v)| k == "id" && valid_id(v)).map(|(_, v)| v).collect();
    if ids.is_empty() {
        return flash_to(PRODUCTS, "error", "Select at least one product.");
    }
    let mut n = 0;
    for id in &ids {
        let ok = match action {
            "active" | "draft" | "archived" => repo::set_product_status(&state.db, id, action).await.is_ok(),
            "delete" => match repo::delete_product(&state.db, id).await {
                Ok(stored) => {
                    remove_stored(&state, &stored).await;
                    true
                }
                Err(_) => false,
            },
            _ => return flash_to(PRODUCTS, "error", "Choose what to do with the selected products."),
        };
        if ok {
            n += 1;
        }
    }
    flash_to(PRODUCTS, "ok", &format!("{} updated.", countable(n, "product")))
}

// ---------------------------------------------------------------------------
// the editor
// ---------------------------------------------------------------------------

pub(crate) async fn product_new(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    product_editor(&state, &actor, None, &q).await
}

pub(crate) async fn product_edit(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !valid_id(&id) {
        return not_found_page_for(actor.role, &actor.user, "That product could not be found.");
    }
    let Ok(Some(p)) = repo::find_product(&state.db, &id).await else {
        return not_found_page_for(actor.role, &actor.user, "That product could not be found.");
    };
    product_editor(&state, &actor, Some(p), &q).await
}

fn field(name: &str, label: &str, value: &str, kind: &str, extra: &str) -> String {
    format!(r#"<label class="field"><input type="{kind}" name="{name}" value="{v}" placeholder=" "{extra}><span>{l}</span></label>"#, v = esc(value), l = esc(label))
}

fn check(name: &str, label: &str, on: bool, extra: &str) -> String {
    format!(r#"<label class="ec-check"><input type="checkbox" name="{name}" value="1"{c}{extra}> {l}</label>"#, c = if on { " checked" } else { "" }, l = esc(label))
}

fn card(title: &str, icon: &str, tint: &str, body: &str, id: &str) -> String {
    format!(r#"<section class="card padded ec-card" id="{id}"><h2 class="ec-card-title"><span class="ic tint-{tint}">{ic}</span>{t}</h2>{body}</section>"#, ic = svg(icon), t = esc(title))
}

/// One row of the variants table.
fn variant_row(key: &str, v: Option<&repo::Variant>, base: &repo::Variant, s: &Settings, images: &[String]) -> String {
    let src = v.unwrap_or(base);
    let img_opts: String = std::iter::once(r#"<option value="">—</option>"#.to_string())
        .chain(images.iter().enumerate().map(|(i, u)| format!(r#"<option value="{u}"{s}>Image {n}</option>"#, u = esc(u), s = if src.image == *u { " selected" } else { "" }, n = i + 1)))
        .collect();
    format!(
        r#"<tr data-key="{k}"><td><input type="hidden" name="v_key" value="{k}"><input type="hidden" name="v_id" value="{id}"><b>{k}</b></td>
<td><input class="ec-in" name="v_price" value="{p}" inputmode="decimal" aria-label="Price" placeholder="0.00"></td>
<td><input class="ec-in" name="v_compare" value="{c}" inputmode="decimal" aria-label="Compare-at price"></td>
<td><input class="ec-in" name="v_sku" value="{sku}" aria-label="SKU" maxlength="80"></td>
<td><input class="ec-in ec-in-qty" name="v_qty" value="{q}" inputmode="numeric" aria-label="Quantity"></td>
<td><select class="ec-in" name="v_image" aria-label="Image">{img_opts}</select></td></tr>"#,
        k = esc(key),
        id = esc(v.map(|v| v.id.as_str()).unwrap_or("")),
        p = esc(&rules::money_input(src.price_cents, &s.currency)),
        c = esc(&rules::money_input(src.compare_at_cents, &s.currency)),
        sku = esc(if v.is_some() { &src.sku } else { "" }),
        q = src.inventory,
    )
}

async fn product_editor(state: &AppState, actor: &Actor, p: Option<repo::Product>, q: &HashMap<String, String>) -> Response {
    let s = store_settings(state);
    let vs = match &p {
        Some(p) => repo::variants(&state.db, &p.id).await,
        None => Vec::new(),
    };
    let collections = repo::collections(&state.db).await;
    let in_colls = match &p {
        Some(p) => repo::product_collection_ids(&state.db, &p.id).await,
        None => Vec::new(),
    };
    let (vendors, types) = repo::vendors_and_types(&state.db).await;
    let tags = repo::all_tags(&state.db).await;
    let options = p.as_ref().map(|p| p.option_list()).unwrap_or_default();
    let images = p.as_ref().map(|p| p.image_list()).unwrap_or_default();
    let get = |f: fn(&repo::Product) -> &str| p.as_ref().map(f).unwrap_or("").to_string();
    let base = vs.first().cloned().unwrap_or(repo::Variant {
        id: String::new(),
        product_id: String::new(),
        title: rules::DEFAULT_VARIANT.into(),
        option1: String::new(),
        option2: String::new(),
        option3: String::new(),
        sku: String::new(),
        barcode: String::new(),
        price_cents: 0,
        compare_at_cents: 0,
        cost_cents: 0,
        track_inventory: 1,
        inventory: 0,
        allow_backorder: 0,
        weight_grams: 0,
        image: String::new(),
        position: 0,
    });
    let status = ProductStatus::from_key(p.as_ref().map(|p| p.status.as_str()).unwrap_or("draft"));
    let action = match &p {
        Some(p) => format!("{PRODUCTS}{}/", esc(&p.id)),
        None => format!("{PRODUCTS}new/"),
    };

    // Koenig: the description as Lexical, opened exactly as the post editor
    // opens a post (an old HTML-only description is wrapped into a card).
    let lexical = get(|p| &p.description_lexical);
    let initial = if !lexical.trim().is_empty() { Some(lexical.clone()) } else { ghost_render::lexical::wrap_existing(Some(""), Some(&get(|p| &p.description_html))) };
    let initial = initial.map(|l| ghost_core::url::base_assets(&ghost_core::url::unbase_assets(&l)));
    let site_title = state.settings.get_str("title").unwrap_or_default();
    let config = serde_json::json!({
        "lexical": initial,
        "placeholder": "Describe this product — what it is, what it is made of, why someone wants it…",
        "siteUrl": state.site_url(),
        "siteTitle": site_title,
        "siteDescription": state.settings.get_str("description").unwrap_or_default(),
        "membersEnabled": false,
        "stripeEnabled": false,
        "basePath": ghost_core::url::subdir(),
        "unsplash": serde_json::Value::Null,
        "pinturaConfig": serde_json::Value::Null,
        "currency": s.currency,
        "symbol": rules::symbol(&s.currency),
    });

    let media = format!(
        r#"<div class="ec-media" id="ec-media">{items}<label class="ec-media-add" title="Upload images"><input type="file" accept="image/*" multiple id="ec-upload">{plus}<span>Add images</span></label></div>
<details class="ec-more"><summary>Add from URL</summary><div class="ec-row"><input class="ec-in" id="ec-media-url" placeholder="https://… or /content/images/…" maxlength="500"><button class="btn tonal small" type="button" id="ec-media-url-add">Add</button></div></details>
<p class="muted ec-hint">The first image is the main one. Drag to reorder, or use the arrows.</p>"#,
        items = images.iter().map(|u| media_item(u)).collect::<String>(),
        plus = svg("add"),
    );
    let pricing = format!(
        r#"<div class="ec-grid2">{price}{compare}</div>
<div class="ec-grid3">{cost}<div class="ec-calc"><span class="muted">Profit</span><b id="ec-profit">—</b></div><div class="ec-calc"><span class="muted">Margin</span><b id="ec-margin">—</b></div></div>
{taxable}<p class="muted ec-hint ec-only-variants">With variants, these are the defaults for new ones; set each variant's price in the table below.</p>"#,
        price = field("price", &format!("Price ({})", s.currency), &rules::money_input(base.price_cents, &s.currency), "text", r#" inputmode="decimal" id="ec-price""#),
        compare = field("compare_at", "Compare-at price", &rules::money_input(base.compare_at_cents, &s.currency), "text", r#" inputmode="decimal""#),
        cost = field("cost", "Cost per item", &rules::money_input(base.cost_cents, &s.currency), "text", r#" inputmode="decimal" id="ec-cost""#),
        taxable = check("taxable", "Charge tax on this product", p.as_ref().map(|p| p.taxable != 0).unwrap_or(true), ""),
    );
    let inventory = format!(
        r#"<div class="ec-grid2">{sku}{barcode}</div>{track}<div class="ec-tracked"><div class="ec-grid2">{qty}<div></div></div>{back}</div>"#,
        track = check("track", "Track quantity", base.tracked(), ""),
        qty = field("qty", "Quantity", &base.inventory.to_string(), "number", ""),
        sku = field("sku", "SKU (stock keeping unit)", if vs.len() <= 1 { &base.sku } else { "" }, "text", r#" maxlength="80""#),
        barcode = field("barcode", "Barcode (ISBN, UPC, GTIN…)", &base.barcode, "text", r#" maxlength="80""#),
        back = check("backorder", "Continue selling when out of stock", base.backorder(), ""),
    );
    let shipping = format!(
        r#"{phys}<div class="ec-physical"><div class="ec-grid2">{w}</div></div><p class="muted ec-hint ec-nonphysical">Not shipped: checkout asks for no address. Use this for digital downloads and services.</p>"#,
        phys = check("requires_shipping", "This is a physical product", p.as_ref().map(|p| p.requires_shipping != 0).unwrap_or(true), ""),
        w = field("weight", "Weight (grams)", &if base.weight_grams > 0 { base.weight_grams.to_string() } else { String::new() }, "number", r#" min="0""#),
    );
    let option_rows: String = (0..rules::MAX_OPTIONS)
        .map(|i| {
            let o = options.get(i);
            format!(
                r#"<div class="ec-option"{hide} data-option="{i}"><label class="field"><input name="opt_name_{i}" value="{n}" placeholder=" " maxlength="40" list="ec-option-names"><span>Option name</span></label>
<label class="field" style="flex:2"><input name="opt_values_{i}" value="{v}" placeholder=" " maxlength="1500"><span>Values, separated by commas</span></label>
<button class="icon-btn" type="button" data-remove-option title="Remove option">{del}</button></div>"#,
                // No options yet: every row waits behind the button.
                hide = if o.is_none() && (i > 0 || options.is_empty()) { " hidden" } else { "" },
                n = esc(o.map(|o| o.name.as_str()).unwrap_or("")),
                v = esc(&o.map(|o| o.values.join(", ")).unwrap_or_default()),
                del = svg("delete"),
            )
        })
        .collect();
    let combos = rules::combinations(&options);
    let variant_rows: String = if options.is_empty() {
        String::new()
    } else {
        combos
            .iter()
            .map(|c| {
                let key = rules::variant_title(c);
                let found = vs.iter().find(|v| v.values() == *c);
                variant_row(&key, found, &base, &s, &images)
            })
            .collect()
    };
    let variants = format!(
        r#"<datalist id="ec-option-names"><option value="Size"><option value="Color"><option value="Material"><option value="Style"></datalist>
<div id="ec-options">{option_rows}</div>
<p class="muted ec-hint ec-no-options">This product has one version. Add options if it comes in sizes, colours or other choices.</p>
<button class="btn text small" type="button" id="ec-add-option">{add} <span data-label>Add options like size or colour</span></button>
<div class="ec-variants-wrap"{vhide} id="ec-variants-wrap"><table class="ec-table ec-variants"><thead><tr><th>Variant</th><th>Price</th><th>Compare at</th><th>SKU</th><th>Quantity</th><th>Image</th></tr></thead>
<tbody id="ec-variants">{variant_rows}</tbody></table>
<p class="muted ec-hint">{n_variants} · Saving creates, keeps or removes variants to match the options.</p></div>"#,
        add = svg("add"),
        vhide = if options.is_empty() { " hidden" } else { "" },
        n_variants = countable(combos.len() as i64, "variant"),
    );
    let seo = format!(
        r#"<div class="ec-serp"><span class="ec-serp-url" id="ec-serp-url">{site}/{ppath}/<b>{handle}</b>/</span><span class="ec-serp-title" id="ec-serp-title">{st}</span><span class="ec-serp-desc" id="ec-serp-desc">{sd}</span></div>
{t}{d}{h}"#,
        site = esc(state.site_url().trim_end_matches('/')),
        handle = esc(&get(|p| &p.handle)),
        ppath = esc(&ghost_api::ecom::paths(state).product),
        st = esc(&{ let t = get(|p| &p.seo_title); if t.is_empty() { get(|p| &p.title) } else { t } }),
        sd = esc(&get(|p| &p.seo_description)),
        t = field("seo_title", "Page title", &get(|p| &p.seo_title), "text", r#" maxlength="70" id="ec-seo-title""#),
        d = format!(r#"<label class="field"><textarea name="seo_description" rows="2" placeholder=" " maxlength="320" id="ec-seo-desc">{}</textarea><span>Meta description</span></label>"#, esc(&get(|p| &p.seo_description))),
        h = field("handle", "URL handle", &get(|p| &p.handle), "text", r#" maxlength="120" id="ec-handle""#),
    );

    let manual: Vec<&repo::Collection> = collections.iter().filter(|c| !c.is_smart()).collect();
    let smart_in: Vec<String> = match &p {
        Some(prod) => {
            let mut out = Vec::new();
            for c in collections.iter().filter(|c| c.is_smart()) {
                if repo::collection_product_ids(&state.db, c, false).await.contains(&prod.id) {
                    out.push(c.title.clone());
                }
            }
            out
        }
        None => Vec::new(),
    };
    let coll_html = if collections.is_empty() {
        r#"<p class="muted ec-hint">No collections yet. <a href="/ghost/ecom/collections/new/">Create one</a>.</p>"#.to_string()
    } else {
        format!(
            "{}{}",
            manual.iter().map(|c| check("collection", &c.title, in_colls.contains(&c.id), &format!(r#" data-value="{}""#, esc(&c.id))).replace(r#"value="1""#, &format!(r#"value="{}""#, esc(&c.id)))).collect::<String>(),
            if smart_in.is_empty() { String::new() } else { format!(r#"<p class="muted ec-hint">Also in (automatically): {}</p>"#, esc(&smart_in.join(", "))) }
        )
    };
    let datalist = |id: &str, list: &[String]| format!(r#"<datalist id="{id}">{}</datalist>"#, list.iter().map(|v| format!(r#"<option value="{}">"#, esc(v))).collect::<String>());
    let status_opts: String = ProductStatus::ALL.iter().map(|x| format!(r#"<option value="{}"{}>{}</option>"#, x.key(), if *x == status { " selected" } else { "" }, x.label())).collect();
    let sales = match &p {
        Some(prod) => {
            let (units, rev) = repo::product_sales(&state.db, &prod.id).await;
            card("Sales", "chart", "green", &format!(r#"<div class="ec-kpis"><div><b>{units}</b><span class="muted">units sold</span></div><div><b>{}</b><span class="muted">revenue</span></div></div>"#, money(rev, &s)), "ec-sales")
        }
        None => String::new(),
    };
    let actions = match &p {
        Some(prod) => format!(
            r#"<section class="card padded ec-card"><div class="ec-side-actions">
{view}<button class="btn text" type="submit" form="ec-dup">{copy} Duplicate</button>
<button class="btn text danger" type="submit" form="ec-del">{del} Delete product</button></div></section>"#,
            view = if prod.is_active() { format!(r#"<a class="btn text" href="{}" target="_blank" rel="noopener">{} View in store</a>"#, ghost_core::url::base(&ghost_api::ecom::paths(&state).public_path(&format!("/shop/products/{}/", prod.handle))), svg("open")) } else { String::new() },
            copy = svg("copy"),
            del = svg("delete"),
        ),
        None => String::new(),
    };
    let side = format!(
        r#"{status}{org}{sales}{actions}"#,
        status = card("Status", "eye", "green", &format!(r#"<label class="field fixed"><select name="status">{status_opts}</select><span>Status</span>{SELECT_CARET}</label>
<p class="muted ec-hint">Only active products are in the store. Drafts are saved but hidden; archived ones are kept for your records.</p>{feat}"#, feat = check("featured", "Feature on the store's home", p.as_ref().is_some_and(|p| p.featured != 0), "")), "ec-status"),
        org = card("Product organization", "folder", "blue", &format!(
            r#"{ty}{ve}{dl_t}{dl_v}<label class="field"><input name="tags" value="{tags}" placeholder=" " list="ec-tag-list" maxlength="500"><span>Tags, separated by commas</span></label>{dl_tags}
<div class="ec-sublabel">Collections</div>{coll_html}"#,
            ty = field("product_type", "Product type", &get(|p| &p.product_type), "text", r#" list="ec-types" maxlength="120""#),
            ve = field("vendor", "Vendor", &get(|p| &p.vendor), "text", r#" list="ec-vendors" maxlength="120""#),
            dl_t = datalist("ec-types", &types),
            dl_v = datalist("ec-vendors", &vendors),
            tags = esc(&get(|p| &p.tags)),
            dl_tags = datalist("ec-tag-list", &tags),
        ), "ec-org"),
    );
    let main = format!(
        r#"<section class="card padded ec-card ec-titlecard">{title}
<div class="ec-sublabel">Description</div>
<div class="ec-koenig"><div id="koenig-mount" data-config="{config}"></div></div>
<input type="hidden" name="description_lexical" id="ec-lexical" value="">
<div id="ec-desc-fallback"><label class="field"><textarea name="description_plain" rows="8" placeholder=" ">{plain}</textarea><span>Description (plain text)</span></label></div>
{excerpt}</section>
{media}{pricing}{inventory}{shipping}{variants}{digital}{seo}"#,
        title = field("title", "Title", &get(|p| &p.title), "text", r#" required maxlength="255" id="ec-title" placeholder="Short sleeve t-shirt""#).replace(r#"placeholder=" " required"#, r#"placeholder=" " required"#),
        config = esc(&config.to_string()),
        plain = esc(&ghost_render::markdown::html_to_plaintext(&get(|p| &p.description_html))),
        excerpt = field("excerpt", "Short summary (shown in lists and shares)", &get(|p| &p.excerpt), "text", r#" maxlength="500""#),
        media = card("Media", "image", "pink", &media, "ec-media-card"),
        pricing = card("Pricing", "sell", "green", &pricing, "ec-pricing"),
        inventory = card("Inventory", "box", "amber", &inventory, "ec-inventory"),
        shipping = card("Shipping", "truck", "blue", &shipping, "ec-shipping"),
        variants = card("Variants", "tune", "purple", &variants, "ec-variants-card"),
        seo = card("Search engine listing", "search", "slate", &seo, "ec-seo"),
        digital = digital_card(state, p.as_ref()).await,
    );
    let forms = match &p {
        Some(prod) => format!(
            r#"<form id="ec-dup" method="post" action="{PRODUCTS}{id}/duplicate/"></form><form id="ec-del" method="post" action="{PRODUCTS}{id}/delete/" data-confirm="Delete product|&#8220;{t}&#8221; will be permanently deleted. Past orders keep their copy, but buyers lose access to its downloadable files."></form>"#,
            id = esc(&prod.id),
            t = esc(&prod.title)
        ),
        None => String::new(),
    };
    let heading = p.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| "Add product".into());
    let body = format!(
        r#"<style>html {{ font-size: 62.5%; }}</style>
<header class="page-head rise ec-edit-head"><a class="icon-btn" style="transform:rotate(180deg)" href="{PRODUCTS}" title="Back to products">{back}</a>
<div><h1>{h}</h1><p class="sub">{sub}</p></div>
<div class="actions"><span class="save-state" id="ec-save-state" aria-live="polite"></span><button class="btn" type="submit" form="ec-product">{check} Save</button></div></header>
{banner}{forms}
<form id="ec-product" class="ec-edit" method="post" action="{action}" enctype="multipart/form-data">
<div class="ec-edit-main">{main}</div><aside class="ec-edit-side">{side}</aside></form>
<script src="{ASSETS}/react.js" defer></script>
<script src="{ASSETS}/react-jsx.js" defer></script>
<script src="{ASSETS}/react-dom.js" defer></script>
<script src="{ASSETS}/koenig-lexical.js" defer></script>
<script src="{ASSETS}/koenig-integrations.js" defer></script>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        back = svg("chevron"),
        h = esc(&heading),
        sub = if p.is_some() { format!("{} · {}", status.label(), countable(vs.len() as i64, "variant")) } else { "Products are what customers buy in your store.".into() },
        check = svg("check"),
        banner = flash_banner(q),
    );
    page_for_role(actor.role, &heading, "tools", Some(&actor.user), &format!(r#"<div class="ec">{body}</div>"#), true).into_response()
}

pub(crate) fn media_item(u: &str) -> String {
    format!(
        r#"<div class="ec-media-item" draggable="true"><img src="{u}" alt=""><input type="hidden" name="image" value="{u}">
<div class="ec-media-tools"><button type="button" data-move="-1" title="Move earlier">‹</button><button type="button" data-move="1" title="Move later">›</button><button type="button" data-remove title="Remove">×</button></div></div>"#,
        u = esc(u)
    )
}

/// Parse the editor's form into what the repository writes.
fn read_form(f: &[(String, String)], s: &Settings, existing: &[repo::Variant]) -> Result<(repo::ProductInput, Vec<repo::VariantInput>, Vec<String>), String> {
    let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let all = |k: &str| f.iter().filter(|(x, _)| x == k).map(|(_, v)| v.clone()).collect::<Vec<_>>();
    // A checkbox counts only when it says so: a crafted `track=0` is off.
    let on = |k: &str| f.iter().any(|(x, v)| x == k && matches!(v.as_str(), "1" | "on" | "true"));
    let title = rules::clean(&get("title"), 255);
    if title.is_empty() {
        return Err("Give the product a title.".into());
    }
    let lexical = get("description_lexical");
    let (lexical, html) = if lexical.trim().starts_with('{') {
        let l = ghost_core::url::unbase_assets(&lexical);
        let html = ghost_render::lexical::to_html(&l);
        (l, html)
    } else {
        let plain = rules::clean_multiline(&get("description_plain"), 20_000);
        (String::new(), rules::paragraphs(&plain))
    };
    let options = rules::build_options(&(0..rules::MAX_OPTIONS).map(|i| (get(&format!("opt_name_{i}")), get(&format!("opt_values_{i}")))).collect::<Vec<_>>());
    let handle_raw = get("handle");
    let handle = rules::handleize(if handle_raw.trim().is_empty() { &title } else { &handle_raw });
    let images = rules::clean_images(all("image").iter().map(String::as_str));
    let price = rules::money_or_zero(&get("price"), &s.currency);
    let compare = rules::money_or_zero(&get("compare_at"), &s.currency);
    let cost = rules::money_or_zero(&get("cost"), &s.currency);
    let track = on("track");
    let backorder = on("backorder");
    let qty: i64 = get("qty").trim().parse().unwrap_or(0);
    let weight: i64 = get("weight").trim().parse::<i64>().unwrap_or(0).clamp(0, 10_000_000);
    let input = repo::ProductInput {
        handle,
        title,
        status: ProductStatus::from_key(&get("status")).key().into(),
        description_lexical: lexical,
        description_html: html,
        excerpt: rules::clean(&get("excerpt"), 500),
        vendor: rules::clean(&get("vendor"), 120),
        product_type: rules::clean(&get("product_type"), 120),
        tags: rules::parse_tags(&get("tags")),
        images: images.clone(),
        options: options.clone(),
        seo_title: rules::clean(&get("seo_title"), 120),
        seo_description: rules::clean(&get("seo_description"), 320),
        requires_shipping: on("requires_shipping"),
        taxable: on("taxable"),
        featured: on("featured"),
        download_limit: get("download_limit").trim().parse::<i64>().unwrap_or(0).clamp(0, 10_000),
        download_days: get("download_days").trim().parse::<i64>().unwrap_or(0).clamp(0, 36_500),
    };
    let base = repo::VariantInput {
        id: None,
        values: Vec::new(),
        sku: rules::clean(&get("sku"), 80),
        barcode: rules::clean(&get("barcode"), 80),
        price_cents: price,
        compare_at_cents: compare,
        cost_cents: cost,
        track_inventory: track,
        inventory: qty,
        allow_backorder: backorder,
        weight_grams: weight,
        image: String::new(),
    };
    let variants = if options.is_empty() {
        vec![repo::VariantInput { id: existing.first().map(|v| v.id.clone()), ..base }]
    } else {
        let keys = all("v_key");
        let col = |k: &str| all(k);
        let (ids, prices, compares, skus, qtys, imgs) = (col("v_id"), col("v_price"), col("v_compare"), col("v_sku"), col("v_qty"), col("v_image"));
        rules::combinations(&options)
            .into_iter()
            .map(|values| {
                let key = rules::variant_title(&values);
                let stored = existing.iter().find(|v| v.values() == values);
                match keys.iter().position(|k| *k == key) {
                    Some(i) => {
                        let at = |v: &Vec<String>| v.get(i).cloned().unwrap_or_default();
                        let image = at(&imgs);
                        repo::VariantInput {
                            id: Some(at(&ids)).filter(|x| valid_id(x)).or_else(|| stored.map(|v| v.id.clone())),
                            values,
                            sku: rules::clean(&at(&skus), 80),
                            barcode: stored.map(|v| v.barcode.clone()).unwrap_or_default(),
                            price_cents: rules::parse_money(&at(&prices), &s.currency).unwrap_or(price),
                            compare_at_cents: rules::money_or_zero(&at(&compares), &s.currency),
                            cost_cents: stored.map(|v| v.cost_cents).unwrap_or(cost),
                            track_inventory: track,
                            inventory: at(&qtys).trim().parse().unwrap_or(0),
                            allow_backorder: backorder,
                            weight_grams: weight,
                            image: if images.contains(&image) { image } else { String::new() },
                        }
                    }
                    None => match stored {
                        Some(v) => repo::VariantInput {
                            id: Some(v.id.clone()),
                            values,
                            sku: v.sku.clone(),
                            barcode: v.barcode.clone(),
                            price_cents: v.price_cents,
                            compare_at_cents: v.compare_at_cents,
                            cost_cents: v.cost_cents,
                            track_inventory: track,
                            inventory: v.inventory,
                            allow_backorder: backorder,
                            weight_grams: weight,
                            image: v.image.clone(),
                        },
                        None => repo::VariantInput { values, sku: String::new(), barcode: String::new(), inventory: if track { 0 } else { qty }, ..base.clone() },
                    },
                }
            })
            .collect()
    };
    let colls: Vec<String> = all("collection").into_iter().filter(|c| valid_id(c)).collect();
    Ok((input, variants, colls))
}

pub(crate) async fn product_create(State(state): State<AppState>, headers: HeaderMap, req: axum::extract::Request) -> Response {
    let _actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let back = format!("{PRODUCTS}new/");
    let (f, staged) = match read_product_request(&state, req).await {
        Ok(x) => x,
        Err(e) => return flash_to(&back, "error", &e),
    };
    let s = store_settings(&state);
    let (p, v, c) = match read_form(&f, &s, &[]) {
        Ok(x) => x,
        Err(e) => {
            discard(&state, &staged).await;
            return flash_to(&back, "error", &e);
        }
    };
    match repo::save_product(&state.db, None, &p, &v, &c).await {
        Ok(id) => match attach(&state, &id, staged).await {
            Ok(n) => flash_to(&format!("{PRODUCTS}{id}/"), "ok", &saved_message("Product created.", n)),
            Err(e) => flash_to(&format!("{PRODUCTS}{id}/#ec-digital"), "error", &format!("Product created, but its files were not: {e}")),
        },
        Err(e) => {
            discard(&state, &staged).await;
            flash_to(&back, "error", &e.to_string())
        }
    }
}

pub(crate) async fn product_save(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, req: axum::extract::Request) -> Response {
    let _actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !valid_id(&id) || repo::find_product(&state.db, &id).await.ok().flatten().is_none() {
        return flash_to(PRODUCTS, "error", "That product no longer exists.");
    }
    let back = format!("{PRODUCTS}{id}/");
    let (f, staged) = match read_product_request(&state, req).await {
        Ok(x) => x,
        Err(e) => return flash_to(&back, "error", &e),
    };
    let s = store_settings(&state);
    let existing = repo::variants(&state.db, &id).await;
    let (p, v, c) = match read_form(&f, &s, &existing) {
        Ok(x) => x,
        Err(e) => {
            discard(&state, &staged).await;
            return flash_to(&back, "error", &e);
        }
    };
    match repo::save_product(&state.db, Some(&id), &p, &v, &c).await {
        Ok(_) => match attach(&state, &id, staged).await {
            Ok(n) => flash_to(&back, "ok", &saved_message("Product saved.", n)),
            Err(e) => flash_to(&format!("{back}#ec-digital"), "error", &format!("Product saved, but its files were not: {e}")),
        },
        Err(e) => {
            discard(&state, &staged).await;
            flash_to(&back, "error", &e.to_string())
        }
    }
}

pub(crate) async fn product_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if valid_id(&id) {
        if let Ok(stored) = repo::delete_product(&state.db, &id).await {
            remove_stored(&state, &stored).await;
        }
    }
    flash_to(PRODUCTS, "ok", "Product deleted.")
}

pub(crate) async fn product_duplicate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    if !valid_id(&id) {
        return flash_to(PRODUCTS, "error", "That product no longer exists.");
    }
    match repo::duplicate_product(&state.db, &id).await {
        Ok(new_id) => flash_to(&format!("{PRODUCTS}{new_id}/"), "ok", "Duplicated as a draft. Stock starts at zero."),
        Err(e) => flash_to(PRODUCTS, "error", &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// digital downloads
// ---------------------------------------------------------------------------

async fn remove_stored(state: &AppState, stored: &[String]) {
    for name in stored {
        if let Some(path) = ghost_api::ecom::stored_path(state, name) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

/// The "Digital downloads" card, part of the product form on both Add and
/// Edit: files picked here upload with Save, so a product is created with
/// its files in one step. Existing files show with a staff download link and
/// a Remove button; Remove submits to its own address (`formaction`) rather
/// than nesting a form.
async fn digital_card(state: &AppState, p: Option<&repo::Product>) -> String {
    let files = match p {
        Some(p) => repo::files(&state.db, &p.id).await,
        None => Vec::new(),
    };
    let rows: String = files
        .iter()
        .map(|f| {
            let base = format!("{PRODUCTS}{}/files/", esc(&f.product_id));
            format!(
                r#"<div class="row ec-file"><span class="ic tint-purple">{ic}</span><div class="body"><span class="title">{n}</span><span class="detail">{fname} · {size} · added {d}</span></div>
<div class="trailing ec-row-tools"><a class="icon-btn" href="{base}{id}/" title="Download (staff)">{dl}</a>
<button class="icon-btn" type="submit" formaction="{base}{id}/delete/" formnovalidate data-confirm-click="Buyers will no longer be able to download &#8220;{n}&#8221;. Unsaved changes on this page are discarded." title="Remove">{del}</button></div></div>"#,
                ic = svg("file"),
                n = esc(&f.name),
                fname = esc(&f.filename),
                size = esc(&rules::format_bytes(f.size_bytes.max(0) as u64)),
                d = esc(f.created_at.get(..10).unwrap_or(&f.created_at)),
                id = esc(&f.id),
                dl = svg("import"),
                del = svg("delete"),
            )
        })
        .collect();
    format!(
        r#"<section class="card padded ec-card" id="ec-digital"><h2 class="ec-card-title"><span class="ic tint-purple">{ic}</span>Digital downloads</h2>
<p class="muted ec-hint" style="margin-bottom:10px">Attach the files buyers get — PDF, ZIP, audio, video, anything up to 2 GB each. They upload when you save, are stored privately, and are only served to the signed-in member who bought them: after payment, or straight away when the product is free.</p>
<div class="rows">{rows}</div>
<label class="ec-drop"><input type="file" name="digital_file" multiple id="ec-digital-input">{up}<span><b>Choose files</b> or drop them here</span><small>They upload with Save · up to {max} files</small></label>
<ul class="ec-picked" id="ec-digital-picked"></ul>
<div class="ec-grid2" style="margin-top:12px">
<label class="field"><input name="download_limit" type="number" min="0" value="{limit}" placeholder=" "><span>Downloads per purchase (0 = unlimited)</span></label>
<label class="field"><input name="download_days" type="number" min="0" value="{days}" placeholder=" "><span>Days available after payment (0 = forever)</span></label></div>
<p class="muted ec-hint">For a digital-only product, untick "This is a physical product" so checkout asks only for an email.</p></section>"#,
        ic = svg("file"),
        rows = rows,
        up = svg("upload"),
        max = rules::MAX_FILES,
        limit = p.map(|p| p.download_limit).unwrap_or(0),
        days = p.map(|p| p.download_days).unwrap_or(0),
    )
}

/// A file streamed to a temporary name while the form is read.
struct Staged {
    id: String,
    filename: String,
    size: u64,
}

async fn discard(state: &AppState, staged: &[Staged]) {
    for f in staged {
        let _ = tokio::fs::remove_file(ghost_api::ecom::files_dir(state).join(format!("{}.part", f.id))).await;
    }
}

/// Read the product form, url-encoded or multipart. Text fields come back
/// as pairs; each `digital_file` is streamed to `<id>.part` in the private
/// directory, counted against the size limit as it arrives.
async fn read_product_request(state: &AppState, req: axum::extract::Request) -> Result<(Vec<(String, String)>, Vec<Staged>), String> {
    use axum::extract::FromRequest;
    let multipart = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("multipart/form-data"));
    if !multipart {
        let Form(pairs) = Form::<Vec<(String, String)>>::from_request(req, state).await.map_err(|_| "The form could not be read.".to_string())?;
        return Ok((pairs, Vec::new()));
    }
    let mut mp = Multipart::from_request(req, state).await.map_err(|_| "The form could not be read.".to_string())?;
    let mut pairs = Vec::new();
    let mut staged: Vec<Staged> = Vec::new();
    let dir = ghost_api::ecom::files_dir(state);
    loop {
        let field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => {
                discard(state, &staged).await;
                return Err("The upload was interrupted.".into());
            }
        };
        let name = field.name().unwrap_or("").to_string();
        if name != "digital_file" {
            let text = field.text().await.unwrap_or_default();
            pairs.push((name, text));
            continue;
        }
        let raw_name = field.file_name().unwrap_or("").to_string();
        if raw_name.is_empty() {
            continue; // an empty file input
        }
        if staged.len() >= rules::MAX_FILES {
            discard(state, &staged).await;
            return Err(format!("A product can have at most {} files.", rules::MAX_FILES));
        }
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            tracing::error!("ecom: cannot create {}: {e}", dir.display());
            discard(state, &staged).await;
            return Err("The server could not store the file.".into());
        }
        let id = ghost_core::ids::new_id();
        let tmp = dir.join(format!("{id}.part"));
        let mut out = match tokio::fs::File::create(&tmp).await {
            Ok(f) => f,
            Err(_) => {
                discard(state, &staged).await;
                return Err("The server could not store the file.".into());
            }
        };
        staged.push(Staged { id, filename: rules::safe_filename(&raw_name), size: 0 });
        use tokio::io::AsyncWriteExt;
        let mut field = field;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    let st = staged.last_mut().expect("just pushed");
                    st.size += chunk.len() as u64;
                    if st.size > rules::MAX_FILE_BYTES || out.write_all(&chunk).await.is_err() {
                        let too_big = st.size > rules::MAX_FILE_BYTES;
                        let fname = st.filename.clone();
                        drop(out);
                        discard(state, &staged).await;
                        return Err(if too_big { format!("{fname} is over 2 GB.") } else { "The server could not store the file.".into() });
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    drop(out);
                    discard(state, &staged).await;
                    return Err("The upload was interrupted.".into());
                }
            }
        }
        let _ = out.flush().await;
        if staged.last().is_some_and(|s| s.size == 0) {
            let f = staged.pop().expect("just pushed");
            let _ = tokio::fs::remove_file(dir.join(format!("{}.part", f.id))).await;
        }
    }
    Ok((pairs, staged))
}

/// Move staged files into place and record them on the product.
async fn attach(state: &AppState, product_id: &str, staged: Vec<Staged>) -> Result<usize, String> {
    let existing = repo::files(&state.db, product_id).await.len();
    if existing + staged.len() > rules::MAX_FILES {
        discard(state, &staged).await;
        return Err(format!("A product can have at most {} files.", rules::MAX_FILES));
    }
    let dir = ghost_api::ecom::files_dir(state);
    let n = staged.len();
    for f in staged {
        let Some(final_path) = ghost_api::ecom::stored_path(state, &f.id) else { continue };
        if tokio::fs::rename(dir.join(format!("{}.part", f.id)), &final_path).await.is_err() {
            return Err("The server could not store the file.".into());
        }
        let name = f.filename.rsplit_once('.').map(|(a, _)| a.to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| f.filename.clone());
        if let Err(e) = repo::insert_file(&state.db, &f.id, product_id, &rules::clean(&name, 200), &f.filename, f.size as i64, rules::content_type_for(&f.filename)).await {
            let _ = tokio::fs::remove_file(&final_path).await;
            return Err(e.to_string());
        }
    }
    Ok(n)
}

fn saved_message(base: &str, files: usize) -> String {
    match files {
        0 => base.to_string(),
        1 => format!("{base} 1 file uploaded."),
        n => format!("{base} {n} files uploaded."),
    }
}

/// Upload one file. Streamed to a temporary name in the private directory,
/// counted against the size limit as it arrives, and renamed into place
/// only once complete — a cut-off upload leaves nothing behind.
pub(crate) async fn product_file_upload(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, mut multipart: Multipart) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{PRODUCTS}{id}/#ec-digital");
    if !valid_id(&id) || repo::find_product(&state.db, &id).await.ok().flatten().is_none() {
        return flash_to(PRODUCTS, "error", "That product no longer exists.");
    }
    if repo::files(&state.db, &id).await.len() >= rules::MAX_FILES {
        return flash_to(&back, "error", &format!("A product can have at most {} files.", rules::MAX_FILES));
    }
    let dir = ghost_api::ecom::files_dir(&state);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::error!("ecom: cannot create {}: {e}", dir.display());
        return flash_to(&back, "error", "The server could not store the file.");
    }
    while let Ok(Some(mut field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let filename = rules::safe_filename(field.file_name().unwrap_or(""));
        if field.file_name().unwrap_or("").is_empty() {
            return flash_to(&back, "error", "Choose a file to upload.");
        }
        let file_id = ghost_core::ids::new_id();
        let tmp = dir.join(format!("{file_id}.part"));
        let mut out = match tokio::fs::File::create(&tmp).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("ecom: cannot write {}: {e}", tmp.display());
                return flash_to(&back, "error", "The server could not store the file.");
            }
        };
        use tokio::io::AsyncWriteExt;
        let mut size: u64 = 0;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    size += chunk.len() as u64;
                    if size > rules::MAX_FILE_BYTES || out.write_all(&chunk).await.is_err() {
                        drop(out);
                        let _ = tokio::fs::remove_file(&tmp).await;
                        return flash_to(&back, "error", if size > rules::MAX_FILE_BYTES { "That file is over 2 GB." } else { "The server could not store the file." });
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    drop(out);
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return flash_to(&back, "error", "The upload was interrupted.");
                }
            }
        }
        if out.flush().await.is_err() || size == 0 {
            drop(out);
            let _ = tokio::fs::remove_file(&tmp).await;
            return flash_to(&back, "error", "That file is empty.");
        }
        drop(out);
        let Some(final_path) = ghost_api::ecom::stored_path(&state, &file_id) else { return flash_to(&back, "error", "Bad file id.") };
        if tokio::fs::rename(&tmp, &final_path).await.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
            return flash_to(&back, "error", "The server could not store the file.");
        }
        let name = filename.rsplit_once('.').map(|(a, _)| a.to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| filename.clone());
        if let Err(e) = repo::insert_file(&state.db, &file_id, &id, &rules::clean(&name, 200), &filename, size as i64, rules::content_type_for(&filename)).await {
            let _ = tokio::fs::remove_file(&final_path).await;
            return flash_to(&back, "error", &e.to_string());
        }
        return flash_to(&back, "ok", &format!("{filename} uploaded ({}).", rules::format_bytes(size)));
    }
    flash_to(&back, "error", "Choose a file to upload.")
}

pub(crate) async fn product_file_delete(State(state): State<AppState>, headers: HeaderMap, Path((id, fid)): Path<(String, String)>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let back = format!("{PRODUCTS}{id}/#ec-digital");
    if !valid_id(&id) || !valid_id(&fid) || repo::find_file(&state.db, &fid).await.is_none_or(|f| f.product_id != id) {
        return flash_to(&back, "error", "That file no longer exists.");
    }
    if let Some(stored) = repo::delete_file(&state.db, &fid).await {
        remove_stored(&state, &[stored]).await;
    }
    flash_to(&back, "ok", "File removed.")
}

/// Staff can download any product file, to check what buyers get.
pub(crate) async fn product_file_download(State(state): State<AppState>, headers: HeaderMap, Path((id, fid)): Path<(String, String)>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let Some(f) = (valid_id(&fid)).then_some(()).and(repo::find_file(&state.db, &fid).await).filter(|f| f.product_id == id) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let Some(path) = ghost_api::ecom::stored_path(&state, &f.stored) else { return (StatusCode::NOT_FOUND, "Not found").into_response() };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_DISPOSITION, rules::content_disposition(&f.filename)),
                (header::CACHE_CONTROL, "private, no-store".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "The file is missing on disk.").into_response(),
    }
}
