//! Tools → E-commerce: products in Shopify's product CSV format, both ways.
//!
//! The file Shopify's admin writes from *Products → Export* and reads in
//! *Products → Import*: one row per variant, grouped by `Handle`. The first
//! row of a handle carries the product (title, body, vendor, tags, SEO,
//! status); later rows add variants, and rows with only `Image Src` add
//! images. A product with no options has one variant whose option is
//! `Title` / `Default Title`.
//!
//! Columns are found by their header, case-insensitively, so exports from
//! any period work — older files without `Product Category` or the
//! `Option… Linked To` columns, newer ones with Google Shopping and unit
//! price columns, which are ignored. Everything read is rebuilt through
//! `ghost_core::ecom` (prices to cents, tags, options, image URLs), never
//! stored as sent.

use crate::ecom::{self as rules, ProductOption};
use std::collections::HashMap;

/// A product as the CSV describes it, in this store's terms.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CsvProduct {
    pub handle: String,
    pub title: String,
    pub body_html: String,
    pub vendor: String,
    pub product_type: String,
    pub tags: Vec<String>,
    /// `active | draft | archived`.
    pub status: String,
    pub options: Vec<ProductOption>,
    pub variants: Vec<CsvVariant>,
    pub images: Vec<String>,
    pub seo_title: String,
    pub seo_description: String,
    pub requires_shipping: bool,
    pub taxable: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CsvVariant {
    pub values: Vec<String>,
    pub sku: String,
    pub barcode: String,
    pub price_cents: i64,
    pub compare_at_cents: i64,
    pub cost_cents: i64,
    pub grams: i64,
    pub track: bool,
    pub inventory: i64,
    pub backorder: bool,
    pub image: String,
}

pub const MAX_PRODUCTS: usize = 5_000;
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

// ---------------------------------------------------------------------------
// CSV, RFC 4180
// ---------------------------------------------------------------------------

/// Split CSV text into rows of fields: quoted fields, doubled quotes,
/// newlines inside quotes, CRLF or LF, a leading byte-order mark.
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// One CSV cell, quoted when it has to be. Text that a spreadsheet would
/// run as a formula is defused with a leading `'` (read back off on import).
fn cell(v: &str, text: bool) -> String {
    let v = if text { rules::csv_cell(v) } else { v.to_string() };
    if v.contains([',', '"', '\n', '\r']) || v.starts_with(' ') || v.ends_with(' ') {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

/// Undo [`cell`]'s formula guard.
fn undefuse(v: &str) -> String {
    match v.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@']) => rest.to_string(),
        _ => v.to_string(),
    }
}

// ---------------------------------------------------------------------------
// import
// ---------------------------------------------------------------------------

/// What reading a file found: the products, and what was wrong with rows
/// that were skipped (row number as a spreadsheet shows it).
#[derive(Debug, Default)]
pub struct Parsed {
    pub products: Vec<CsvProduct>,
    pub problems: Vec<String>,
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "y")
}

/// Read a Shopify product CSV.
pub fn parse(text: &str, currency: &str) -> Result<Parsed, String> {
    let rows = parse_rows(text);
    let Some(header) = rows.first() else { return Err("The file is empty.".into()) };
    let col: HashMap<String, usize> = header.iter().enumerate().map(|(i, h)| (h.trim().to_ascii_lowercase(), i)).collect();
    if !col.contains_key("handle") {
        return Err("This is not a Shopify product CSV: it has no Handle column.".into());
    }
    let mut out = Parsed::default();
    let mut order: Vec<String> = Vec::new();
    let mut by_handle: HashMap<String, CsvProduct> = HashMap::new();
    let mut option_names: HashMap<String, [String; 3]> = HashMap::new();
    // Handles with a variant row that could not be read.
    let mut refused: HashMap<String, usize> = HashMap::new();
    for (n, row) in rows.iter().enumerate().skip(1) {
        let get = |name: &str| col.get(name).and_then(|i| row.get(*i)).map(|v| undefuse(v.trim())).unwrap_or_default();
        let handle = rules::handleize(&get("handle"));
        if handle.is_empty() {
            if row.iter().any(|c| !c.trim().is_empty()) {
                out.problems.push(format!("Row {}: no handle, skipped.", n + 1));
            }
            continue;
        }
        if !by_handle.contains_key(&handle) {
            if by_handle.len() >= MAX_PRODUCTS {
                out.problems.push(format!("More than {MAX_PRODUCTS} products: the rest were not read."));
                break;
            }
            let title = rules::clean(&get("title"), 255);
            let status = match get("status").to_ascii_lowercase().as_str() {
                "active" => "active",
                "archived" => "archived",
                "draft" => "draft",
                // Older exports have no Status: Published says it.
                _ if get("published").is_empty() || truthy(&get("published")) => "active",
                _ => "draft",
            };
            by_handle.insert(
                handle.clone(),
                CsvProduct {
                    handle: handle.clone(),
                    title: if title.is_empty() { handle.replace('-', " ") } else { title },
                    body_html: get("body (html)"),
                    vendor: rules::clean(&get("vendor"), 120),
                    product_type: rules::clean(&{ let t = get("type"); if t.is_empty() { get("product type") } else { t } }, 120),
                    tags: rules::parse_tags(&get("tags")),
                    status: status.into(),
                    seo_title: rules::clean(&get("seo title"), 120),
                    seo_description: rules::clean(&get("seo description"), 320),
                    requires_shipping: true,
                    taxable: true,
                    ..Default::default()
                },
            );
            option_names.insert(handle.clone(), [get("option1 name"), get("option2 name"), get("option3 name")]);
            order.push(handle.clone());
        }
        let p = by_handle.get_mut(&handle).expect("inserted above");
        let names = option_names.get_mut(&handle).expect("inserted above");
        for (i, slot) in names.iter_mut().enumerate() {
            if slot.is_empty() {
                *slot = get(&format!("option{} name", i + 1));
            }
        }
        // An image on any row.
        let img = get("image src");
        if !img.is_empty() {
            for u in rules::clean_images(std::iter::once(img.as_str())) {
                if !p.images.contains(&u) {
                    p.images.push(u);
                }
            }
        }
        let values: Vec<String> = (1..=3).map(|i| rules::clean(&get(&format!("option{i} value")), 60)).collect();
        let is_variant = values.iter().any(|v| !v.is_empty()) || !get("variant price").is_empty() || !get("variant sku").is_empty();
        if !is_variant {
            continue;
        }
        let price = get("variant price");
        let Some(price_cents) = (if price.is_empty() { Some(0) } else { rules::parse_money(&price, currency) }) else {
            out.problems.push(format!("Row {}: “{}” is not a price, variant skipped.", n + 1, price));
            *refused.entry(handle.clone()).or_default() += 1;
            continue;
        };
        if p.variants.is_empty() {
            p.requires_shipping = get("variant requires shipping").is_empty() || truthy(&get("variant requires shipping"));
            p.taxable = get("variant taxable").is_empty() || truthy(&get("variant taxable"));
        }
        let vimg = rules::clean_images(std::iter::once(get("variant image").as_str())).into_iter().next().unwrap_or_default();
        if !vimg.is_empty() && !p.images.contains(&vimg) {
            p.images.push(vimg.clone());
        }
        let grams = get("variant grams").parse::<f64>().map(|g| g.round() as i64).unwrap_or(0).max(0);
        p.variants.push(CsvVariant {
            values,
            sku: rules::clean(&get("variant sku"), 80),
            barcode: rules::clean(&get("variant barcode"), 80),
            price_cents,
            compare_at_cents: rules::money_or_zero(&get("variant compare at price"), currency),
            cost_cents: rules::money_or_zero(&get("cost per item"), currency),
            grams,
            track: get("variant inventory tracker").eq_ignore_ascii_case("shopify"),
            inventory: get("variant inventory qty").parse::<f64>().map(|q| q as i64).unwrap_or(0),
            backorder: get("variant inventory policy").eq_ignore_ascii_case("continue"),
            image: vimg,
        });
    }
    for handle in order {
        let mut p = by_handle.remove(&handle).expect("ordered");
        // Every variant refused: there is nothing to sell, so no product.
        if p.variants.is_empty() && refused.get(&handle).is_some_and(|n| *n > 0) {
            out.problems.push(format!("{handle}: none of its variants could be read, product skipped."));
            continue;
        }
        let names = option_names.remove(&handle).unwrap_or_default();
        // `Title` / `Default Title` is Shopify's spelling of "no options".
        let single_default = p.variants.len() <= 1
            && names[0].eq_ignore_ascii_case("title")
            && p.variants.first().is_none_or(|v| v.values[0].eq_ignore_ascii_case(rules::DEFAULT_VARIANT) || v.values[0].is_empty());
        if single_default || names.iter().all(|n| n.is_empty()) {
            for v in p.variants.iter_mut() {
                v.values.clear();
            }
            p.variants.truncate(1);
        } else {
            let pairs: Vec<(String, String)> = (0..3)
                .filter(|i| !names[*i].is_empty())
                .map(|i| {
                    let mut vals: Vec<String> = Vec::new();
                    for v in &p.variants {
                        if let Some(x) = v.values.get(i).filter(|x| !x.is_empty()) {
                            if !vals.iter().any(|y| y.eq_ignore_ascii_case(x)) {
                                vals.push(x.clone());
                            }
                        }
                    }
                    (names[i].clone(), vals.join(","))
                })
                .collect();
            p.options = rules::build_options(&pairs);
            let n = p.options.len();
            for v in p.variants.iter_mut() {
                v.values.truncate(n);
            }
            // Duplicate combinations: the first wins.
            let mut seen: Vec<Vec<String>> = Vec::new();
            p.variants.retain(|v| {
                let k: Vec<String> = v.values.iter().map(|x| x.to_lowercase()).collect();
                if seen.contains(&k) {
                    false
                } else {
                    seen.push(k);
                    true
                }
            });
            p.variants.truncate(rules::MAX_VARIANTS);
        }
        if p.variants.is_empty() {
            p.variants.push(CsvVariant::default());
        }
        p.images.truncate(rules::MAX_IMAGES);
        out.products.push(p);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// The columns written, in Shopify's order. Shopify's importer reads any
/// subset of its columns by name, so this is the useful core of them.
pub const COLUMNS: &[&str] = &[
    "Handle", "Title", "Body (HTML)", "Vendor", "Product Category", "Type", "Tags", "Published",
    "Option1 Name", "Option1 Value", "Option2 Name", "Option2 Value", "Option3 Name", "Option3 Value",
    "Variant SKU", "Variant Grams", "Variant Inventory Tracker", "Variant Inventory Qty", "Variant Inventory Policy",
    "Variant Fulfillment Service", "Variant Price", "Variant Compare At Price", "Variant Requires Shipping", "Variant Taxable",
    "Variant Barcode", "Image Src", "Image Position", "Image Alt Text", "Gift Card", "SEO Title", "SEO Description",
    "Variant Image", "Variant Weight Unit", "Cost per item", "Status",
];

/// Write products as a Shopify product CSV. `site` turns this store's
/// relative image paths into absolute URLs, which Shopify needs to fetch them.
pub fn write(products: &[CsvProduct], currency: &str, site: &str) -> String {
    let abs = |u: &str| if u.starts_with('/') { format!("{}{u}", site.trim_end_matches('/')) } else { u.to_string() };
    let money = |c: i64| if c == 0 { String::new() } else { rules::decimal(c, currency) };
    let mut out = COLUMNS.join(",");
    out.push('\n');
    for p in products {
        let rows = p.variants.len().max(p.images.len()).max(1);
        for i in 0..rows {
            let first = i == 0;
            let v = p.variants.get(i);
            let mut r: Vec<String> = vec![String::new(); COLUMNS.len()];
            let mut set = |name: &str, value: String| {
                if let Some(j) = COLUMNS.iter().position(|c| *c == name) {
                    r[j] = value;
                }
            };
            set("Handle", p.handle.clone());
            if first {
                set("Title", p.title.clone());
                set("Body (HTML)", p.body_html.clone());
                set("Vendor", p.vendor.clone());
                set("Type", p.product_type.clone());
                set("Tags", p.tags.join(", "));
                set("Published", if p.status == "active" { "true".into() } else { "false".into() });
                set("Gift Card", "false".into());
                set("SEO Title", p.seo_title.clone());
                set("SEO Description", p.seo_description.clone());
                set("Status", p.status.clone());
            }
            if let Some(v) = v {
                if p.options.is_empty() {
                    if first {
                        set("Option1 Name", "Title".into());
                    }
                    set("Option1 Value", rules::DEFAULT_VARIANT.into());
                } else {
                    for (k, o) in p.options.iter().enumerate() {
                        if first {
                            set(&format!("Option{} Name", k + 1), o.name.clone());
                        }
                        set(&format!("Option{} Value", k + 1), v.values.get(k).cloned().unwrap_or_default());
                    }
                }
                set("Variant SKU", v.sku.clone());
                set("Variant Grams", v.grams.to_string());
                set("Variant Inventory Tracker", if v.track { "shopify".into() } else { String::new() });
                set("Variant Inventory Qty", v.inventory.to_string());
                set("Variant Inventory Policy", if v.backorder { "continue".into() } else { "deny".into() });
                set("Variant Fulfillment Service", "manual".into());
                set("Variant Price", rules::decimal(v.price_cents, currency));
                set("Variant Compare At Price", money(v.compare_at_cents));
                set("Variant Requires Shipping", p.requires_shipping.to_string());
                set("Variant Taxable", p.taxable.to_string());
                set("Variant Barcode", v.barcode.clone());
                set("Variant Image", if v.image.is_empty() { String::new() } else { abs(&v.image) });
                set("Variant Weight Unit", "g".into());
                set("Cost per item", money(v.cost_cents));
            }
            if let Some(img) = p.images.get(i) {
                set("Image Src", abs(img));
                set("Image Position", (i + 1).to_string());
            }
            let numeric = ["Variant Grams", "Variant Inventory Qty", "Variant Price", "Variant Compare At Price", "Cost per item", "Image Position"];
            let line: Vec<String> = r.iter().enumerate().map(|(j, value)| cell(value, !numeric.contains(&COLUMNS[j]))).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real Shopify export: a product with two options and
    /// three variants, an extra image row, and a product with no options.
    const SHOPIFY: &str = "\u{feff}Handle,Title,Body (HTML),Vendor,Product Category,Type,Tags,Published,Option1 Name,Option1 Value,Option1 Linked To,Option2 Name,Option2 Value,Option2 Linked To,Option3 Name,Option3 Value,Option3 Linked To,Variant SKU,Variant Grams,Variant Inventory Tracker,Variant Inventory Qty,Variant Inventory Policy,Variant Fulfillment Service,Variant Price,Variant Compare At Price,Variant Requires Shipping,Variant Taxable,Variant Barcode,Image Src,Image Position,Image Alt Text,Gift Card,SEO Title,SEO Description,Google Shopping / Gender,Variant Image,Variant Weight Unit,Variant Tax Code,Cost per item,Status\r
classic-tee,Classic Tee,\"<p>Soft, \"\"heavy\"\" cotton.</p>\nWashes well.\",Acme,Apparel & Accessories,Shirts,\"summer, cotton\",true,Size,S,,Color,Red,,,,,TEE-S-R,200,shopify,5,deny,manual,20.00,25.00,true,true,123,https://cdn.shopify.com/s/files/tee.jpg,1,,false,Tee title,Tee desc,,,g,,8.50,active\r
classic-tee,,,,,,,,,S,,,Blue,,,,,TEE-S-B,200,shopify,0,continue,manual,20.00,,true,true,,https://cdn.shopify.com/s/files/tee-2.jpg,2,,,,,,https://cdn.shopify.com/s/files/tee-blue.jpg,g,,,\r
classic-tee,,,,,,,,,M,,,Red,,,,,TEE-M-R,210,shopify,3,deny,manual,22.50,,true,true,,,,,,,,,,g,,,\r
classic-tee,,,,,,,,,,,,,,,,,,,,,,,,,,,,https://cdn.shopify.com/s/files/tee-3.jpg,3,,,,,,,,,,\r
ebook,The E-book,<p>Read it.</p>,Acme,,Books,,true,Title,Default Title,,,,,,,,EB-1,0,,0,deny,manual,0.00,,false,false,,,,,false,,,,,g,,,draft\r
";

    #[test]
    fn a_shopify_export_is_read_whole() {
        let p = parse(SHOPIFY, "USD").unwrap();
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(p.products.len(), 2);
        let tee = &p.products[0];
        assert_eq!(tee.handle, "classic-tee");
        assert_eq!(tee.body_html, "<p>Soft, \"heavy\" cotton.</p>\nWashes well.");
        assert_eq!(tee.tags, vec!["summer", "cotton"]);
        assert_eq!(tee.status, "active");
        assert_eq!(tee.options.iter().map(|o| (o.name.as_str(), o.values.join("|"))).collect::<Vec<_>>(), vec![("Size", "S|M".to_string()), ("Color", "Red|Blue".to_string())]);
        assert_eq!(tee.variants.len(), 3);
        assert_eq!(tee.variants[0].values, vec!["S", "Red"]);
        assert_eq!((tee.variants[0].price_cents, tee.variants[0].compare_at_cents, tee.variants[0].cost_cents), (2000, 2500, 850));
        assert!(tee.variants[0].track && !tee.variants[0].backorder);
        assert!(tee.variants[1].backorder);
        assert_eq!(tee.variants[1].image, "https://cdn.shopify.com/s/files/tee-blue.jpg");
        assert_eq!(tee.variants[2].price_cents, 2250);
        assert_eq!(tee.images.len(), 4, "three image rows and the variant image");
        assert_eq!(tee.seo_title, "Tee title");
        let book = &p.products[1];
        assert!(book.options.is_empty(), "Title / Default Title is no options");
        assert_eq!(book.variants.len(), 1);
        assert!(book.variants[0].values.is_empty());
        assert!(!book.requires_shipping && !book.taxable);
        assert_eq!(book.status, "draft");
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let p = parse(SHOPIFY, "USD").unwrap().products;
        let csv = write(&p, "USD", "https://example.test");
        assert!(csv.starts_with("Handle,Title,Body (HTML),Vendor"));
        let again = parse(&csv, "USD").unwrap();
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.products, p);
    }

    #[test]
    fn relative_images_are_made_absolute_and_formulas_defused() {
        let p = vec![CsvProduct {
            handle: "x".into(),
            title: "=HYPERLINK(\"evil\")".into(),
            status: "active".into(),
            images: vec!["/content/images/x.jpg".into()],
            variants: vec![CsvVariant { price_cents: 500, ..Default::default() }],
            ..Default::default()
        }];
        let csv = write(&p, "USD", "https://example.test/");
        assert!(csv.contains("https://example.test/content/images/x.jpg"));
        assert!(csv.contains("\"'=HYPERLINK(\"\"evil\"\")\""), "{csv}");
        let back = parse(&csv, "USD").unwrap();
        assert_eq!(back.products[0].title, "=HYPERLINK(\"evil\")", "the guard is taken off on import");
    }

    #[test]
    fn bad_rows_are_reported_not_fatal() {
        assert!(parse("Title,Price\nx,1\n", "USD").is_err());
        let p = parse("Handle,Title,Option1 Name,Option1 Value,Variant Price\nhat,Hat,Size,S,abc\nhat,,,M,9\n,,,,\n", "USD").unwrap();
        assert_eq!(p.problems.len(), 1);
        assert_eq!(p.products[0].variants.len(), 1);
        let none = parse("Handle,Title,Variant Price\nhat,Hat,abc\n", "USD").unwrap();
        assert!(none.products.is_empty(), "a product with no readable variant is not created at zero");
        assert_eq!(none.problems.len(), 2);
        assert_eq!(p.products[0].variants[0].values, vec!["M"]);
    }
}
