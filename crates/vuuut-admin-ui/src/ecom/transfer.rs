//! Products in and out, in Shopify's product CSV format
//! ([`vuuut_core::shopify_csv`]): the file Shopify's *Products → Export*
//! writes imports here, and this store's export imports into Shopify.
//!
//! Import matches products by handle: an existing handle is updated (when
//! asked to) keeping its variants' ids where the option values match, its
//! collections, its digital files and its download limits; a new handle
//! becomes a new product. Images can stay at their Shopify URLs or be copied
//! onto this site.

use super::*;
use vuuut_core::shopify_csv as csv_fmt;

const PRODUCTS: &str = "/ghost/ecom/products/";
const IMPORT: &str = "/ghost/ecom/products/import/";

pub(crate) async fn products_import_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, false).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let s = store_settings(&state);
    let count: i64 = repo::product_counts(&state.db).await.values().sum();
    let report = q.get("report").map(|r| {
        let lines: Vec<&str> = r.split('\n').filter(|l| !l.is_empty()).collect();
        format!(
            r#"<details class="card padded ec-card ec-report rise" open><summary><b>Rows that were skipped</b> <span class="muted">· {n}</span></summary><ul>{items}</ul></details>"#,
            n = lines.len(),
            items = lines.iter().map(|l| format!("<li>{}</li>", esc(l))).collect::<String>(),
        )
    });
    let body = format!(
        r#"{report}<div class="ec-split">
<form class="card padded ec-card rise" style="--i:2" method="post" action="{IMPORT}" enctype="multipart/form-data">
<h2 class="ec-card-title"><span class="ic tint-green">{up}</span>Import products</h2>
<p class="muted ec-p">Upload a product CSV in Shopify's format — the file Shopify gives you from <b>Products → Export</b> (“All products”, “CSV for Excel, Numbers, or other spreadsheet programs”). Products, variants and options, prices, compare-at prices, costs, SKUs, barcodes, weights, stock, images, tags, SEO and status all come across.</p>
<label class="ec-drop"><input type="file" name="file" accept=".csv,text/csv" required id="ec-csv-input">{up}<span><b>Choose a CSV</b> or drop it here</span><small>Up to 25 MB · {max} products</small></label>
<ul class="ec-picked" id="ec-csv-picked"></ul>
<div class="ec-sublabel">Options</div>
<label class="ec-check"><input type="checkbox" name="update" value="1" checked> Update products that already exist (matched by handle)</label>
<label class="ec-check"><input type="checkbox" name="draft" value="1"> Import new products as drafts, whatever the file says</label>
<label class="ec-check"><input type="checkbox" name="copy_images" value="1"> Copy images onto this site <span class="muted">— slower, but they keep working after the Shopify store closes</span></label>
<p class="muted ec-hint">Prices are read in this store's currency ({cur}). Collections, metafields and inventory locations are not part of Shopify's product CSV, so they are not imported.</p>
<div class="ec-actions"><button class="btn" type="submit">{up} Import</button></div>
</form>
<div class="card padded ec-card rise" style="--i:3"><h2 class="ec-card-title"><span class="ic tint-blue">{dl}</span>Export products</h2>
<p class="muted ec-p">All {count} products as a Shopify product CSV. Import it into Shopify with <b>Products → Import</b>, open it in a spreadsheet, or keep it as a backup and import it here again.</p>
<p class="muted ec-hint">Images on this site are written as full addresses so Shopify can fetch them.</p>
<div class="ec-actions" style="justify-content:flex-start"><a class="btn tonal" href="{PRODUCTS}export.csv">{dl} Download CSV</a></div></div>
</div>
<script src="{ASSETS}/ecom-editor.js" defer></script>"#,
        report = report.unwrap_or_default(),
        up = svg("upload"),
        dl = svg("import"),
        max = csv_fmt::MAX_PRODUCTS,
        cur = esc(&s.currency),
    );
    ecom_page(&actor, "Import and export products", "products", &q, "", Some(PRODUCTS), &body)
}

pub(crate) async fn products_export(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = guard(&state, &headers, Role::Editor, false).await {
        return r;
    }
    let s = store_settings(&state);
    let (items, _) = repo::browse_products(&state.db, &repo::ProductBrowse { sort: "title", limit: 100_000, ..Default::default() }).await.unwrap_or_default();
    let ids: Vec<String> = items.iter().map(|p| p.id.clone()).collect();
    let vs = repo::variants_for(&state.db, &ids).await;
    let products: Vec<csv_fmt::CsvProduct> = items
        .iter()
        .map(|p| csv_fmt::CsvProduct {
            handle: p.handle.clone(),
            title: p.title.clone(),
            body_html: p.description_html.clone(),
            vendor: p.vendor.clone(),
            product_type: p.product_type.clone(),
            tags: p.tag_list(),
            status: p.status.clone(),
            options: p.option_list(),
            variants: vs
                .get(&p.id)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|v| csv_fmt::CsvVariant {
                    values: v.values(),
                    sku: v.sku.clone(),
                    barcode: v.barcode.clone(),
                    price_cents: v.price_cents,
                    compare_at_cents: v.compare_at_cents,
                    cost_cents: v.cost_cents,
                    grams: v.weight_grams,
                    track: v.tracked(),
                    inventory: v.inventory,
                    backorder: v.backorder(),
                    image: v.image.clone(),
                })
                .collect(),
            images: p.image_list(),
            seo_title: p.seo_title.clone(),
            seo_description: p.seo_description.clone(),
            requires_shipping: p.requires_shipping != 0,
            taxable: p.taxable != 0,
        })
        .collect();
    let body = csv_fmt::write(&products, &s.currency, &state.site_url());
    let name = format!("products_export_{}.csv", chrono::Utc::now().format("%Y-%m-%d"));
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn products_import(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    let actor = match guard(&state, &headers, Role::Editor, true).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let mut text: Option<String> = None;
    let (mut update, mut draft, mut copy_images) = (false, false, false);
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name().unwrap_or("") {
            "file" => match field.bytes().await {
                Ok(b) if b.len() > csv_fmt::MAX_BYTES => return flash_to(IMPORT, "error", "That file is over 25 MB."),
                Ok(b) => text = Some(String::from_utf8_lossy(&b).into_owned()),
                Err(_) => return flash_to(IMPORT, "error", "The upload was interrupted."),
            },
            "update" => update = true,
            "draft" => draft = true,
            "copy_images" => copy_images = true,
            _ => {}
        }
    }
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return flash_to(IMPORT, "error", "Choose a CSV file to import.");
    };
    let s = store_settings(&state);
    let parsed = match csv_fmt::parse(&text, &s.currency) {
        Ok(p) => p,
        Err(e) => return flash_to(IMPORT, "error", &e),
    };
    let mut problems = parsed.problems;
    let (mut created, mut updated, mut skipped) = (0, 0, 0);
    let mut copied: HashMap<String, String> = HashMap::new();
    for mut p in parsed.products {
        let existing = repo::product_by_handle(&state.db, &p.handle).await.ok().flatten();
        if existing.is_some() && !update {
            skipped += 1;
            continue;
        }
        if copy_images {
            for img in p.images.iter_mut().chain(p.variants.iter_mut().map(|v| &mut v.image)) {
                if !img.starts_with("http") {
                    continue;
                }
                if let Some(local) = copied.get(img.as_str()) {
                    *img = local.clone();
                } else if let Some(local) = vuuut_api::ecom::copy_image(&state, img).await {
                    copied.insert(img.clone(), local.clone());
                    *img = local;
                } else {
                    problems.push(format!("{}: could not copy {} — kept the original address.", p.handle, img));
                }
            }
        }
        let old_variants = match &existing {
            Some(e) => repo::variants(&state.db, &e.id).await,
            None => Vec::new(),
        };
        let collections = match &existing {
            Some(e) => repo::product_collection_ids(&state.db, &e.id).await,
            None => Vec::new(),
        };
        let input = repo::ProductInput {
            handle: p.handle.clone(),
            title: p.title.clone(),
            status: if draft && existing.is_none() { "draft".into() } else { p.status.clone() },
            description_lexical: vuuut_render::lexical::wrap_existing(Some(""), Some(&p.body_html)).unwrap_or_default(),
            description_html: p.body_html.clone(),
            excerpt: existing.as_ref().map(|e| e.excerpt.clone()).unwrap_or_default(),
            vendor: p.vendor.clone(),
            product_type: p.product_type.clone(),
            tags: p.tags.clone(),
            images: p.images.clone(),
            options: p.options.clone(),
            seo_title: p.seo_title.clone(),
            seo_description: p.seo_description.clone(),
            requires_shipping: p.requires_shipping,
            taxable: p.taxable,
            featured: existing.as_ref().is_some_and(|e| e.featured != 0),
            download_limit: existing.as_ref().map(|e| e.download_limit).unwrap_or(0),
            download_days: existing.as_ref().map(|e| e.download_days).unwrap_or(0),
            // Shopify's CSV has no column for these; an import keeps what is set here
            demo_url: existing.as_ref().map(|e| e.demo_url.clone()).unwrap_or_default(),
            purchase_url: existing.as_ref().map(|e| e.purchase_url.clone()).unwrap_or_default(),
        };
        let variants: Vec<repo::VariantInput> = p
            .variants
            .iter()
            .map(|v| repo::VariantInput {
                id: old_variants.iter().find(|o| o.values() == v.values).map(|o| o.id.clone()),
                values: v.values.clone(),
                sku: v.sku.clone(),
                barcode: v.barcode.clone(),
                price_cents: v.price_cents,
                compare_at_cents: v.compare_at_cents,
                cost_cents: v.cost_cents,
                track_inventory: v.track,
                inventory: v.inventory,
                allow_backorder: v.backorder,
                weight_grams: v.grams,
                image: if p.images.contains(&v.image) { v.image.clone() } else { String::new() },
            })
            .collect();
        match repo::save_product(&state.db, existing.as_ref().map(|e| e.id.as_str()), &input, &variants, &collections).await {
            Ok(_) if existing.is_some() => updated += 1,
            Ok(_) => created += 1,
            Err(e) => problems.push(format!("{}: {e}", p.handle)),
        }
    }
    tracing::info!("ecom: CSV import by {}: {created} created, {updated} updated, {skipped} skipped", actor.user.email.as_deref().unwrap_or(""));
    let summary = format!(
        "Import finished: {} created, {} updated{}.",
        created,
        updated,
        if skipped > 0 { format!(", {skipped} skipped because they already exist") } else { String::new() }
    );
    let kind = if created + updated == 0 && !problems.is_empty() { "error" } else { "ok" };
    let mut encoded = form_urlencoded::Serializer::new(String::new());
    encoded.append_pair(kind, &summary);
    if !problems.is_empty() {
        problems.truncate(50);
        encoded.append_pair("report", &problems.join("\n"));
    }
    let target = if problems.is_empty() { format!("{PRODUCTS}?{}", encoded.finish()) } else { format!("{IMPORT}?{}", encoded.finish()) };
    Redirect::to(&target).into_response()
}
