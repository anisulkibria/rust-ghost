//! Inventory: every variant's stock on one screen, editable in place.
//!
//! A quantity can be set outright or adjusted by an amount (`+5`, `-2`),
//! which is what a person counting a delivery wants.

use super::*;

const INVENTORY: &str = "/ghost/ecom/inventory/";

pub(crate) async fn inventory_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let query = q.get("q").map(|v| v.chars().take(100).collect::<String>()).unwrap_or_default();
    let terms = rules::search_terms(&query);
    let low = q.get("low").is_some();
    let rows = repo::stock_rows(&state.db, &terms, low.then_some(s.low_stock), 1000).await;
    let (tracked, out, lowc) = repo::stock_summary(&state.db, s.low_stock).await;
    let body_rows: String = rows
        .iter()
        .map(|r| {
            let state_chip = match rules::stock(r.track_inventory != 0, r.inventory, s.low_stock) {
                rules::Stock::Out => r#"<span class="chip error">Out of stock</span>"#,
                rules::Stock::Low => r#"<span class="chip scheduled">Low</span>"#,
                rules::Stock::InStock => r#"<span class="chip published">In stock</span>"#,
                rules::Stock::Untracked => r#"<span class="chip">Not tracked</span>"#,
            };
            format!(
                r#"<tr><td><a class="ec-prod" href="/ghost/ecom/products/{pid}/">{thumb}<span><b>{t}</b><small class="muted">{v}</small></span></a></td><td>{sku}</td><td>{chip}</td>
<td>{edit}</td></tr>"#,
                pid = esc(&r.product_id),
                thumb = thumb(&rules::read_images(&r.images).into_iter().next().unwrap_or_default()),
                t = esc(&r.product_title),
                v = if r.variant_title == rules::DEFAULT_VARIANT { String::new() } else { esc(&r.variant_title) },
                sku = esc(&r.sku),
                chip = state_chip,
                edit = if r.track_inventory != 0 {
                    format!(
                        r#"<form method="post" action="{INVENTORY}{vid}/" class="ec-stock-form"><input class="ec-in ec-in-qty" name="qty" value="{q}" aria-label="Quantity" title="A number sets the stock; +5 or -2 adjusts it"><button class="btn tonal small" type="submit">Save</button></form>"#,
                        vid = esc(&r.variant_id),
                        q = r.inventory
                    )
                } else {
                    r#"<span class="muted">—</span>"#.to_string()
                },
            )
        })
        .collect();
    let body = format!(
        r#"<div class="ec-tiles rise" style="--i:2">{a}{b}{c}</div>
<form class="ec-filter rise" style="--i:3" method="get" action="{INVENTORY}">
<label class="field" style="margin:0;flex:1"><input type="search" name="q" value="{qv}" placeholder=" " maxlength="100"><span>Search product, variant or SKU</span></label>
<label class="ec-check"><input type="checkbox" name="low" value="1"{lowon}> Low and out of stock only</label>
<button class="btn tonal" type="submit">{search} Filter</button></form>
<p class="muted ec-lead">Type a number to set the stock, or +5 / −2 to adjust it.</p>
<div class="card ec-table-card rise" style="--i:3">{table}</div>"#,
        a = tile("Tracked", &tracked.to_string(), "Variants of active products", None, "box", "blue"),
        b = tile("Low stock", &lowc.to_string(), &format!("At or under {}", s.low_stock), Some("?low=1"), "warning", if lowc > 0 { "amber" } else { "slate" }),
        c = tile("Out of stock", &out.to_string(), "Cannot be bought unless backorders are on", Some("?low=1"), "close", if out > 0 { "red" } else { "slate" }),
        qv = esc(&query),
        lowon = if low { " checked" } else { "" },
        search = svg("search"),
        table = if rows.is_empty() {
            format!(r#"<div class="ec-empty">{}<p><b>Nothing to show</b></p></div>"#, svg("box"))
        } else {
            format!(r#"<table class="ec-table ec-list"><thead><tr><th>Product</th><th>SKU</th><th>State</th><th>Available</th></tr></thead><tbody>{body_rows}</tbody></table>"#)
        },
    );
    ecom_page(&actor, "Inventory", "inventory", &q, "", Some(BASE), &body)
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct StockForm {
    qty: String,
}

pub(crate) async fn inventory_set(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<StockForm>) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, true).await {
        return r;
    }
    let Some(v) = valid_id(&id).then_some(()).and(repo::find_variant(&state.db, &id).await) else {
        return flash_to(INVENTORY, "error", "That variant no longer exists.");
    };
    let raw = f.qty.trim().replace('−', "-");
    let parsed = if raw.starts_with('+') || (raw.starts_with('-') && raw.len() > 1) {
        raw.parse::<i64>().ok().map(|d| v.inventory + d)
    } else {
        raw.parse::<i64>().ok()
    };
    match parsed {
        Some(n) => {
            let _ = repo::set_inventory(&state.db, &v.id, n).await;
            flash_to(INVENTORY, "ok", &format!("Stock set to {n}."))
        }
        None => flash_to(INVENTORY, "error", "Enter a number, or +n / -n to adjust."),
    }
}
