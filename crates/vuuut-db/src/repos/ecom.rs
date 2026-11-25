//! Tools → E-commerce & Online Stores: every query.
//!
//! Nothing else reads or writes the `ecom_*` tables. Every value is bound;
//! search words go into `LIKE ?` placeholders. Flags and counters are read
//! as `i64`, and totals are added up in Rust rather than with `SUM()`,
//! which MySQL returns as a DECIMAL the `Any` driver cannot decode.
//!
//! Writes that change what the storefront shows call `content::touch()` so
//! the page cache empties.

use super::dberr;
use crate::Db;
use vuuut_core::ecom::{self as rules, Address, CartLine, ProductOption};
use vuuut_core::{dates, ids, GhostError, Result};
use std::collections::HashMap;

fn flag(b: bool) -> i64 {
    if b { 1 } else { 0 }
}

/// `?,?,?` for an `IN (…)`.
fn marks(n: usize) -> String {
    vec!["?"; n.max(1)].join(",")
}

/// 32 lowercase hex characters: a cart or order token.
pub fn new_token() -> String {
    ids::uuid_v4().replace('-', "").to_lowercase().chars().take(32).collect()
}

// ===========================================================================
// products
// ===========================================================================

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: String,
    pub handle: String,
    pub title: String,
    pub status: String,
    pub description_lexical: String,
    pub description_html: String,
    pub excerpt: String,
    pub vendor: String,
    pub product_type: String,
    pub tags: String,
    pub images: String,
    pub options: String,
    pub seo_title: String,
    pub seo_description: String,
    pub requires_shipping: i64,
    pub taxable: i64,
    pub featured: i64,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub download_limit: i64,
    pub download_days: i64,
    /// Live demo link, `http(s)` or empty (migration 0042).
    pub demo_url: String,
    /// Direct checkout elsewhere (Polar, Gumroad), `http(s)` or empty.
    pub purchase_url: String,
}

impl Product {
    pub fn image_list(&self) -> Vec<String> {
        rules::read_images(&self.images)
    }
    pub fn first_image(&self) -> Option<String> {
        self.image_list().into_iter().next()
    }
    pub fn option_list(&self) -> Vec<ProductOption> {
        rules::read_options(&self.options)
    }
    pub fn tag_list(&self) -> Vec<String> {
        rules::parse_tags(&self.tags)
    }
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
}

const P_COLS: &str = "id, handle, title, status, description_lexical, description_html, excerpt, vendor, product_type, tags, images, \
options, seo_title, seo_description, requires_shipping, taxable, featured, published_at, created_at, updated_at, download_limit, download_days, demo_url, purchase_url";

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct Variant {
    pub id: String,
    pub product_id: String,
    pub title: String,
    pub option1: String,
    pub option2: String,
    pub option3: String,
    pub sku: String,
    pub barcode: String,
    pub price_cents: i64,
    pub compare_at_cents: i64,
    pub cost_cents: i64,
    pub track_inventory: i64,
    pub inventory: i64,
    pub allow_backorder: i64,
    pub weight_grams: i64,
    pub image: String,
    pub position: i64,
}

impl Variant {
    pub fn tracked(&self) -> bool {
        self.track_inventory != 0
    }
    pub fn backorder(&self) -> bool {
        self.allow_backorder != 0
    }
    pub fn values(&self) -> Vec<String> {
        [&self.option1, &self.option2, &self.option3].iter().filter(|v| !v.is_empty()).map(|v| v.to_string()).collect()
    }
    pub fn available(&self, qty: i64) -> bool {
        rules::purchasable(self.tracked(), self.inventory, self.backorder(), qty)
    }
}

const V_COLS: &str = "id, product_id, title, option1, option2, option3, sku, barcode, price_cents, compare_at_cents, cost_cents, \
track_inventory, inventory, allow_backorder, weight_grams, image, position";

/// What a product save writes, already rebuilt by the caller through
/// `vuuut_core::ecom`.
#[derive(Debug, Clone, Default)]
pub struct ProductInput {
    pub handle: String,
    pub title: String,
    pub status: String,
    pub description_lexical: String,
    pub description_html: String,
    pub excerpt: String,
    pub vendor: String,
    pub product_type: String,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub options: Vec<ProductOption>,
    pub seo_title: String,
    pub seo_description: String,
    pub requires_shipping: bool,
    pub taxable: bool,
    pub featured: bool,
    /// Downloads per purchase (0 = unlimited) and days they last (0 = forever).
    pub download_limit: i64,
    pub download_days: i64,
    /// Live demo link; written through [`rules::clean_web_url`].
    pub demo_url: String,
    /// Direct purchase link elsewhere; written through [`rules::clean_web_url`].
    pub purchase_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct VariantInput {
    /// An existing variant of this product to update; `None` inserts.
    pub id: Option<String>,
    pub values: Vec<String>,
    pub sku: String,
    pub barcode: String,
    pub price_cents: i64,
    pub compare_at_cents: i64,
    pub cost_cents: i64,
    pub track_inventory: bool,
    pub inventory: i64,
    pub allow_backorder: bool,
    pub weight_grams: i64,
    pub image: String,
}

pub async fn find_product(pool: &Db, id: &str) -> Result<Option<Product>> {
    sqlx::query_as::<_, Product>(&format!("SELECT {P_COLS} FROM ecom_products WHERE id = ?")).bind(id).fetch_optional(pool).await.map_err(dberr)
}

pub async fn product_by_handle(pool: &Db, handle: &str) -> Result<Option<Product>> {
    sqlx::query_as::<_, Product>(&format!("SELECT {P_COLS} FROM ecom_products WHERE handle = ?")).bind(handle).fetch_optional(pool).await.map_err(dberr)
}

pub async fn products_by_ids(pool: &Db, ids: &[String]) -> Vec<Product> {
    if ids.is_empty() {
        return Vec::new();
    }
    let sql = format!("SELECT {P_COLS} FROM ecom_products WHERE id IN ({})", marks(ids.len()));
    let mut q = sqlx::query_as::<_, Product>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.unwrap_or_default()
}

pub async fn variants(pool: &Db, product_id: &str) -> Vec<Variant> {
    read_variants(pool, product_id).await.unwrap_or_default()
}

pub(super) async fn read_variants(pool: &Db, product_id: &str) -> Result<Vec<Variant>> {
    sqlx::query_as::<_, Variant>(&format!("SELECT {V_COLS} FROM ecom_variants WHERE product_id = ? ORDER BY position, created_at, id"))
        .bind(product_id)
        .fetch_all(pool)
        .await
        .map_err(dberr)
}

pub async fn find_variant(pool: &Db, id: &str) -> Option<Variant> {
    sqlx::query_as::<_, Variant>(&format!("SELECT {V_COLS} FROM ecom_variants WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn variants_by_ids(pool: &Db, ids: &[String]) -> Vec<Variant> {
    if ids.is_empty() {
        return Vec::new();
    }
    let sql = format!("SELECT {V_COLS} FROM ecom_variants WHERE id IN ({})", marks(ids.len()));
    let mut q = sqlx::query_as::<_, Variant>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.unwrap_or_default()
}

/// Variants of several products at once, grouped by product.
pub async fn variants_for(pool: &Db, product_ids: &[String]) -> HashMap<String, Vec<Variant>> {
    let mut out: HashMap<String, Vec<Variant>> = HashMap::new();
    for chunk in product_ids.chunks(500) {
        let sql = format!("SELECT {V_COLS} FROM ecom_variants WHERE product_id IN ({}) ORDER BY position, created_at", marks(chunk.len()));
        let mut q = sqlx::query_as::<_, Variant>(&sql);
        for id in chunk {
            q = q.bind(id);
        }
        for v in q.fetch_all(pool).await.unwrap_or_default() {
            out.entry(v.product_id.clone()).or_default().push(v);
        }
    }
    out
}

pub async fn unique_handle(pool: &Db, wanted: &str, except: Option<&str>) -> String {
    let base = match wanted {
        "" => "product".to_string(),
        // With products directly in the store, these would shadow its pages.
        w if rules::RESERVED_HANDLES.contains(&w) => format!("{w}-product"),
        w => w.to_string(),
    };
    let mut candidate = base.clone();
    for n in 2..500 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_products WHERE handle = ? AND id <> ?")
            .bind(&candidate)
            .bind(except.unwrap_or(""))
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        if taken == 0 {
            return candidate;
        }
        candidate = format!("{base}-{n}");
    }
    format!("{base}-{}", ids::new_id())
}

/// Create or update a product with its variants and its manual collections,
/// in one transaction. Variants of this product not in `variants` (by id)
/// are deleted; a variant id that belongs to another product is treated as
/// new, so a crafted form cannot reach across products.
pub async fn save_product(pool: &Db, id: Option<&str>, p: &ProductInput, variants_in: &[VariantInput], collections: &[String]) -> Result<String> {
    crate::content::touch();
    let now = dates::now_db();
    let handle = unique_handle(pool, &p.handle, id).await;
    // Read before the transaction opens (a second pool connection inside it
    // can deadlock a one-connection pool).
    let existing_variants: Vec<String> = match id {
        Some(id) => variants(pool, id).await.into_iter().map(|v| v.id).collect(),
        None => Vec::new(),
    };
    let existing_links: Vec<String> = match id {
        Some(id) => sqlx::query_scalar::<_, String>("SELECT collection_id FROM ecom_collection_products WHERE product_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let manual: Vec<String> = sqlx::query_scalar::<_, String>("SELECT id FROM ecom_collections WHERE kind = 'manual'").fetch_all(pool).await.unwrap_or_default();
    let was_active = match id {
        Some(id) => find_product(pool, id).await.ok().flatten().map(|x| (x.status == "active", x.published_at)).unwrap_or((false, None)),
        None => (false, None),
    };
    let published_at = if p.status == "active" { was_active.1.clone().or_else(|| Some(now.clone())) } else { was_active.1.clone() };
    let images = serde_json::to_string(&p.images).unwrap_or_else(|_| "[]".into());
    let options = serde_json::to_string(&p.options).unwrap_or_else(|_| "[]".into());
    let tags = p.tags.join(", ");

    let mut tx = pool.begin().await.map_err(dberr)?;
    let pid = match id {
        Some(id) => {
            sqlx::query(
                "UPDATE ecom_products SET handle = ?, title = ?, status = ?, description_lexical = ?, description_html = ?, excerpt = ?, vendor = ?, \
                 product_type = ?, tags = ?, images = ?, options = ?, seo_title = ?, seo_description = ?, requires_shipping = ?, taxable = ?, featured = ?, \
                 published_at = ?, updated_at = ?, download_limit = ?, download_days = ?, demo_url = ?, purchase_url = ? WHERE id = ?",
            )
            .bind(&handle).bind(&p.title).bind(&p.status).bind(&p.description_lexical).bind(&p.description_html).bind(&p.excerpt)
            .bind(&p.vendor).bind(&p.product_type).bind(&tags).bind(&images).bind(&options).bind(&p.seo_title).bind(&p.seo_description)
            .bind(flag(p.requires_shipping)).bind(flag(p.taxable)).bind(flag(p.featured)).bind(&published_at).bind(&now).bind(p.download_limit).bind(p.download_days).bind(rules::clean_web_url(&p.demo_url)).bind(rules::clean_web_url(&p.purchase_url)).bind(id)
            .execute(&mut *tx).await.map_err(dberr)?;
            id.to_string()
        }
        None => {
            let id = ids::new_id();
            sqlx::query(&format!("INSERT INTO ecom_products ({P_COLS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"))
                .bind(&id).bind(&handle).bind(&p.title).bind(&p.status).bind(&p.description_lexical).bind(&p.description_html).bind(&p.excerpt)
                .bind(&p.vendor).bind(&p.product_type).bind(&tags).bind(&images).bind(&options).bind(&p.seo_title).bind(&p.seo_description)
                .bind(flag(p.requires_shipping)).bind(flag(p.taxable)).bind(flag(p.featured)).bind(&published_at).bind(&now).bind(&now)
                .bind(p.download_limit).bind(p.download_days).bind(rules::clean_web_url(&p.demo_url)).bind(rules::clean_web_url(&p.purchase_url))
                .execute(&mut *tx).await.map_err(dberr)?;
            id
        }
    };

    let mut kept: Vec<String> = Vec::new();
    let list: Vec<VariantInput> = if variants_in.is_empty() { vec![VariantInput { track_inventory: false, ..Default::default() }] } else { variants_in.to_vec() };
    for (pos, v) in list.iter().enumerate() {
        let get = |i: usize| v.values.get(i).cloned().unwrap_or_default();
        let title = rules::variant_title(&v.values);
        match v.id.as_ref().filter(|vid| existing_variants.contains(vid)) {
            Some(vid) => {
                sqlx::query(
                    "UPDATE ecom_variants SET title = ?, option1 = ?, option2 = ?, option3 = ?, sku = ?, barcode = ?, price_cents = ?, compare_at_cents = ?, \
                     cost_cents = ?, track_inventory = ?, inventory = ?, allow_backorder = ?, weight_grams = ?, image = ?, position = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&title).bind(get(0)).bind(get(1)).bind(get(2)).bind(&v.sku).bind(&v.barcode).bind(v.price_cents).bind(v.compare_at_cents)
                .bind(v.cost_cents).bind(flag(v.track_inventory)).bind(v.inventory).bind(flag(v.allow_backorder)).bind(v.weight_grams).bind(&v.image)
                .bind(pos as i64).bind(&now).bind(vid)
                .execute(&mut *tx).await.map_err(dberr)?;
                kept.push(vid.clone());
            }
            None => {
                let vid = ids::new_id();
                sqlx::query(
                    "INSERT INTO ecom_variants (id, product_id, title, option1, option2, option3, sku, barcode, price_cents, compare_at_cents, cost_cents, \
                     track_inventory, inventory, allow_backorder, weight_grams, image, position, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(&vid).bind(&pid).bind(&title).bind(get(0)).bind(get(1)).bind(get(2)).bind(&v.sku).bind(&v.barcode).bind(v.price_cents)
                .bind(v.compare_at_cents).bind(v.cost_cents).bind(flag(v.track_inventory)).bind(v.inventory).bind(flag(v.allow_backorder))
                .bind(v.weight_grams).bind(&v.image).bind(pos as i64).bind(&now).bind(&now)
                .execute(&mut *tx).await.map_err(dberr)?;
                kept.push(vid);
            }
        }
    }
    for old in existing_variants.iter().filter(|o| !kept.contains(o)) {
        sqlx::query("DELETE FROM ecom_variants WHERE id = ?").bind(old).execute(&mut *tx).await.map_err(dberr)?;
    }

    let wanted: Vec<&String> = collections.iter().filter(|c| manual.contains(c)).collect();
    for old in existing_links.iter().filter(|c| !wanted.contains(c)) {
        sqlx::query("DELETE FROM ecom_collection_products WHERE collection_id = ? AND product_id = ?").bind(old).bind(&pid).execute(&mut *tx).await.map_err(dberr)?;
    }
    for c in wanted.into_iter().filter(|c| !existing_links.contains(c)) {
        let next: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_collection_products WHERE collection_id = ?").bind(c).fetch_one(&mut *tx).await.unwrap_or(0);
        sqlx::query("INSERT INTO ecom_collection_products (collection_id, product_id, position) VALUES (?,?,?)").bind(c).bind(&pid).bind(next).execute(&mut *tx).await.map_err(dberr)?;
    }
    tx.commit().await.map_err(dberr)?;
    Ok(pid)
}

pub async fn set_product_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    crate::content::touch();
    let now = dates::now_db();
    sqlx::query("UPDATE ecom_products SET status = ?, published_at = CASE WHEN ? = 'active' AND published_at IS NULL THEN ? ELSE published_at END, updated_at = ? WHERE id = ?")
        .bind(status).bind(status).bind(&now).bind(&now).bind(id)
        .execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Deletes the product, its variants and its files' rows. Returns the
/// stored names of its files, for the caller to remove from disk.
pub async fn delete_product(pool: &Db, id: &str) -> Result<Vec<String>> {
    crate::content::touch();
    let stored: Vec<String> = sqlx::query_scalar("SELECT stored FROM ecom_files WHERE product_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default();
    let mut tx = pool.begin().await.map_err(dberr)?;
    for sql in [
        "DELETE FROM ecom_variants WHERE product_id = ?",
        "DELETE FROM ecom_collection_products WHERE product_id = ?",
        "DELETE FROM ecom_files WHERE product_id = ?",
        "DELETE FROM ecom_products WHERE id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await.map_err(dberr)?;
    }
    tx.commit().await.map_err(dberr)?;
    Ok(stored)
}

/// A draft copy, "Copy of …", with its variants (inventory reset to zero).
pub async fn duplicate_product(pool: &Db, id: &str) -> Result<String> {
    let Some(p) = find_product(pool, id).await? else { return Err(GhostError::BadRequest("No such product".into())) };
    let vs = variants(pool, id).await;
    let links: Vec<String> = sqlx::query_scalar("SELECT collection_id FROM ecom_collection_products WHERE product_id = ?").bind(id).fetch_all(pool).await.unwrap_or_default();
    let input = ProductInput {
        handle: format!("{}-copy", p.handle),
        title: format!("Copy of {}", p.title).chars().take(255).collect(),
        status: "draft".into(),
        description_lexical: p.description_lexical.clone(),
        description_html: p.description_html.clone(),
        excerpt: p.excerpt.clone(),
        vendor: p.vendor.clone(),
        product_type: p.product_type.clone(),
        tags: p.tag_list(),
        images: p.image_list(),
        options: p.option_list(),
        seo_title: p.seo_title.clone(),
        seo_description: p.seo_description.clone(),
        requires_shipping: p.requires_shipping != 0,
        taxable: p.taxable != 0,
        featured: false,
        download_limit: p.download_limit,
        download_days: p.download_days,
        demo_url: p.demo_url.clone(),
        purchase_url: p.purchase_url.clone(),
    };
    let vin: Vec<VariantInput> = vs
        .iter()
        .map(|v| VariantInput {
            id: None,
            values: v.values(),
            sku: String::new(),
            barcode: String::new(),
            price_cents: v.price_cents,
            compare_at_cents: v.compare_at_cents,
            cost_cents: v.cost_cents,
            track_inventory: v.tracked(),
            inventory: 0,
            allow_backorder: v.backorder(),
            weight_grams: v.weight_grams,
            image: v.image.clone(),
        })
        .collect();
    save_product(pool, None, &input, &vin, &links).await
}

#[derive(Debug, Clone, Default)]
pub struct ProductBrowse<'a> {
    pub terms: &'a [String],
    pub status: Option<&'a str>,
    pub vendor: Option<&'a str>,
    pub product_type: Option<&'a str>,
    /// Only these products (a collection's members).
    pub ids: Option<&'a [String]>,
    /// `title | price_asc | price_desc | newest | best | updated`
    pub sort: &'a str,
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse_products(pool: &Db, b: &ProductBrowse<'_>) -> Result<(Vec<Product>, i64)> {
    browse_products_with_limit(pool, b, b.limit.clamp(1, 1000)).await
}

/// The storefront uses the page size set by the store administrator.
pub async fn browse_storefront(pool: &Db, b: &ProductBrowse<'_>) -> Result<(Vec<Product>, i64)> {
    browse_products_with_limit(pool, b, b.limit.max(1)).await
}

async fn browse_products_with_limit(pool: &Db, b: &ProductBrowse<'_>, limit: i64) -> Result<(Vec<Product>, i64)> {
    if b.ids.is_some_and(|i| i.is_empty()) {
        return Ok((Vec::new(), 0));
    }
    let mut wh: Vec<String> = vec!["1 = 1".into()];
    let mut binds: Vec<String> = Vec::new();
    if let Some(s) = b.status {
        wh.push("status = ?".into());
        binds.push(s.into());
    }
    if let Some(v) = b.vendor {
        wh.push("vendor = ?".into());
        binds.push(v.into());
    }
    if let Some(t) = b.product_type {
        wh.push("product_type = ?".into());
        binds.push(t.into());
    }
    if let Some(ids) = b.ids {
        wh.push(format!("id IN ({})", marks(ids.len())));
        binds.extend(ids.iter().cloned());
    }
    for t in b.terms {
        wh.push("(LOWER(title) LIKE ? OR LOWER(tags) LIKE ? OR LOWER(vendor) LIKE ? OR LOWER(product_type) LIKE ? OR id IN (SELECT product_id FROM ecom_variants WHERE LOWER(sku) LIKE ?))".into());
        let like = format!("%{}%", t.to_lowercase());
        for _ in 0..5 {
            binds.push(like.clone());
        }
    }
    let wh = wh.join(" AND ");
    let order = match b.sort {
        "title" => "title, id",
        "price_asc" => "(SELECT MIN(price_cents) FROM ecom_variants v WHERE v.product_id = ecom_products.id), title",
        "price_desc" => "(SELECT MIN(price_cents) FROM ecom_variants v WHERE v.product_id = ecom_products.id) DESC, title",
        "best" => "(SELECT COALESCE(SUM(quantity), 0) FROM ecom_order_items i WHERE i.product_id = ecom_products.id) DESC, title",
        "updated" => "updated_at DESC",
        _ => "COALESCE(published_at, created_at) DESC, id",
    };
    let csql = format!("SELECT COUNT(*) FROM ecom_products WHERE {wh}");
    let mut cq = sqlx::query_scalar::<_, i64>(&csql);
    for v in &binds {
        cq = cq.bind(v.clone());
    }
    let total = cq.fetch_one(pool).await.map_err(dberr)?;
    let sql = format!("SELECT {P_COLS} FROM ecom_products WHERE {wh} ORDER BY {order} LIMIT ? OFFSET ?");
    let mut q = sqlx::query_as::<_, Product>(&sql);
    for v in &binds {
        q = q.bind(v.clone());
    }
    let rows = q.bind(limit).bind(b.offset.max(0)).fetch_all(pool).await.map_err(dberr)?;
    Ok((rows, total))
}

/// `(status, count)` across the catalogue.
pub async fn product_counts(pool: &Db) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM ecom_products GROUP BY status").fetch_all(pool).await.unwrap_or_default().into_iter().collect()
}

/// Distinct vendors and product types, for pickers and suggestions.
pub async fn vendors_and_types(pool: &Db) -> (Vec<String>, Vec<String>) {
    let v = sqlx::query_scalar::<_, String>("SELECT DISTINCT vendor FROM ecom_products WHERE vendor <> '' ORDER BY vendor").fetch_all(pool).await.unwrap_or_default();
    let t = sqlx::query_scalar::<_, String>("SELECT DISTINCT product_type FROM ecom_products WHERE product_type <> '' ORDER BY product_type").fetch_all(pool).await.unwrap_or_default();
    (v, t)
}

pub async fn all_tags(pool: &Db) -> Vec<String> {
    let rows = sqlx::query_scalar::<_, String>("SELECT tags FROM ecom_products WHERE tags <> ''").fetch_all(pool).await.unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        for t in rules::parse_tags(&r) {
            if !out.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                out.push(t);
            }
        }
    }
    out.sort_by_key(|t| t.to_lowercase());
    out
}

// ===========================================================================
// inventory
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StockRow {
    pub variant_id: String,
    pub product_id: String,
    pub product_title: String,
    pub variant_title: String,
    pub sku: String,
    pub inventory: i64,
    pub track_inventory: i64,
    pub allow_backorder: i64,
    pub images: String,
    pub status: String,
}

pub async fn stock_rows(pool: &Db, terms: &[String], only_low: Option<i64>, limit: i64) -> Vec<StockRow> {
    let mut wh = vec!["1 = 1".to_string()];
    let mut binds: Vec<String> = Vec::new();
    for t in terms {
        wh.push("(LOWER(p.title) LIKE ? OR LOWER(v.sku) LIKE ? OR LOWER(v.title) LIKE ?)".into());
        let like = format!("%{}%", t.to_lowercase());
        binds.extend([like.clone(), like.clone(), like]);
    }
    if let Some(low) = only_low {
        wh.push(format!("v.track_inventory = 1 AND v.inventory <= {}", low.max(0)));
    }
    let sql = format!(
        "SELECT v.id AS variant_id, p.id AS product_id, p.title AS product_title, v.title AS variant_title, v.sku, v.inventory, v.track_inventory, \
         v.allow_backorder, p.images, p.status FROM ecom_variants v JOIN ecom_products p ON p.id = v.product_id WHERE {} \
         ORDER BY p.title, v.position LIMIT ?",
        wh.join(" AND ")
    );
    let mut q = sqlx::query_as::<_, StockRow>(&sql);
    for b in binds {
        q = q.bind(b);
    }
    q.bind(limit).fetch_all(pool).await.unwrap_or_default()
}

pub async fn set_inventory(pool: &Db, variant_id: &str, qty: i64) -> Result<()> {
    crate::content::touch();
    sqlx::query("UPDATE ecom_variants SET inventory = ?, updated_at = ? WHERE id = ?").bind(qty.clamp(-1_000_000, 1_000_000_000)).bind(dates::now_db()).bind(variant_id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `(variants tracked, out of stock, low)`.
pub async fn stock_summary(pool: &Db, low: i64) -> (i64, i64, i64) {
    let one = |sql: String| async move { sqlx::query_scalar::<_, i64>(&sql).fetch_one(pool).await.unwrap_or(0) };
    let tracked = one("SELECT COUNT(*) FROM ecom_variants v JOIN ecom_products p ON p.id = v.product_id WHERE v.track_inventory = 1 AND p.status = 'active'".into()).await;
    let out = one("SELECT COUNT(*) FROM ecom_variants v JOIN ecom_products p ON p.id = v.product_id WHERE v.track_inventory = 1 AND v.inventory <= 0 AND p.status = 'active'".into()).await;
    let lowc = one(format!("SELECT COUNT(*) FROM ecom_variants v JOIN ecom_products p ON p.id = v.product_id WHERE v.track_inventory = 1 AND v.inventory > 0 AND v.inventory <= {} AND p.status = 'active'", low.max(0))).await;
    (tracked, out, lowc)
}

// ===========================================================================
// collections
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Collection {
    pub id: String,
    pub handle: String,
    pub title: String,
    pub description: String,
    pub image: String,
    pub kind: String,
    pub rules: String,
    pub match_all: i64,
    pub sort: String,
    pub published: i64,
    pub seo_title: String,
    pub seo_description: String,
    pub position: i64,
}

impl Collection {
    pub fn is_smart(&self) -> bool {
        self.kind == "smart"
    }
    pub fn rule_list(&self) -> Vec<rules::Rule> {
        rules::read_rules(&self.rules)
    }
}

const C_COLS: &str = "id, handle, title, description, image, kind, rules, match_all, sort, published, seo_title, seo_description, position";

#[derive(Debug, Clone, Default)]
pub struct CollectionInput {
    pub handle: String,
    pub title: String,
    pub description: String,
    pub image: String,
    pub kind: String,
    pub rules: Vec<rules::Rule>,
    pub match_all: bool,
    pub sort: String,
    pub published: bool,
    pub seo_title: String,
    pub seo_description: String,
}

pub async fn collections(pool: &Db) -> Vec<Collection> {
    sqlx::query_as::<_, Collection>(&format!("SELECT {C_COLS} FROM ecom_collections ORDER BY position, title")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_collection(pool: &Db, id: &str) -> Option<Collection> {
    sqlx::query_as::<_, Collection>(&format!("SELECT {C_COLS} FROM ecom_collections WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn collection_by_handle(pool: &Db, handle: &str) -> Option<Collection> {
    sqlx::query_as::<_, Collection>(&format!("SELECT {C_COLS} FROM ecom_collections WHERE handle = ?")).bind(handle).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_collection(pool: &Db, id: Option<&str>, c: &CollectionInput) -> Result<String> {
    crate::content::touch();
    let now = dates::now_db();
    let mut handle = if c.handle.is_empty() { "collection".to_string() } else { c.handle.clone() };
    for n in 2..200 {
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_collections WHERE handle = ? AND id <> ?").bind(&handle).bind(id.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
        if taken == 0 {
            break;
        }
        handle = format!("{}-{n}", if c.handle.is_empty() { "collection" } else { &c.handle });
    }
    let rules_json = serde_json::to_string(&c.rules).unwrap_or_else(|_| "[]".into());
    match id {
        Some(id) => {
            sqlx::query(
                "UPDATE ecom_collections SET handle = ?, title = ?, description = ?, image = ?, kind = ?, rules = ?, match_all = ?, sort = ?, published = ?, \
                 seo_title = ?, seo_description = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&handle).bind(&c.title).bind(&c.description).bind(&c.image).bind(&c.kind).bind(&rules_json).bind(flag(c.match_all)).bind(&c.sort)
            .bind(flag(c.published)).bind(&c.seo_title).bind(&c.seo_description).bind(&now).bind(id)
            .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            let pos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_collections").fetch_one(pool).await.unwrap_or(0);
            sqlx::query(&format!("INSERT INTO ecom_collections ({C_COLS}, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"))
                .bind(&id).bind(&handle).bind(&c.title).bind(&c.description).bind(&c.image).bind(&c.kind).bind(&rules_json).bind(flag(c.match_all))
                .bind(&c.sort).bind(flag(c.published)).bind(&c.seo_title).bind(&c.seo_description).bind(pos).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_collection(pool: &Db, id: &str) -> Result<()> {
    crate::content::touch();
    sqlx::query("DELETE FROM ecom_collection_products WHERE collection_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM ecom_collections WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// A manual collection's products in their order.
pub async fn manual_ids(pool: &Db, collection_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT product_id FROM ecom_collection_products WHERE collection_id = ? ORDER BY position, product_id")
        .bind(collection_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Rewrite a manual collection's order to exactly `ids`.
pub async fn set_manual_ids(pool: &Db, collection_id: &str, ids_in: &[String]) -> Result<()> {
    crate::content::touch();
    let mut tx = pool.begin().await.map_err(dberr)?;
    sqlx::query("DELETE FROM ecom_collection_products WHERE collection_id = ?").bind(collection_id).execute(&mut *tx).await.map_err(dberr)?;
    for (i, pid) in ids_in.iter().enumerate() {
        sqlx::query("INSERT INTO ecom_collection_products (collection_id, product_id, position) VALUES (?,?,?)").bind(collection_id).bind(pid).bind(i as i64).execute(&mut *tx).await.map_err(dberr)?;
    }
    tx.commit().await.map_err(dberr)?;
    Ok(())
}

/// Which collections a product is in by hand.
pub async fn product_collection_ids(pool: &Db, product_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT collection_id FROM ecom_collection_products WHERE product_id = ?").bind(product_id).fetch_all(pool).await.unwrap_or_default()
}

/// Every product's facts, for smart collections: `(product, facts)` built
/// from the product and its variants.
pub async fn facts(pool: &Db, only_active: bool) -> Vec<(Product, i64, i64)> {
    let sql = if only_active { format!("SELECT {P_COLS} FROM ecom_products WHERE status = 'active'") } else { format!("SELECT {P_COLS} FROM ecom_products") };
    let products = sqlx::query_as::<_, Product>(&sql).fetch_all(pool).await.unwrap_or_default();
    let ids: Vec<String> = products.iter().map(|p| p.id.clone()).collect();
    let vs = variants_for(pool, &ids).await;
    products
        .into_iter()
        .map(|p| {
            let v = vs.get(&p.id).cloned().unwrap_or_default();
            let price = v.iter().map(|v| v.price_cents).min().unwrap_or(0);
            let inv: i64 = v.iter().filter(|v| v.tracked()).map(|v| v.inventory).sum();
            (p, price, inv)
        })
        .collect()
}

/// The products in a collection, in its manual order when it has one.
pub async fn collection_product_ids(pool: &Db, c: &Collection, only_active: bool) -> Vec<String> {
    if !c.is_smart() {
        let ids = manual_ids(pool, &c.id).await;
        if !only_active {
            return ids;
        }
        let active: Vec<String> = products_by_ids(pool, &ids).await.into_iter().filter(|p| p.is_active()).map(|p| p.id).collect();
        return ids.into_iter().filter(|i| active.contains(i)).collect();
    }
    let rules_list = c.rule_list();
    facts(pool, only_active)
        .await
        .into_iter()
        .filter(|(p, price, inv)| {
            let f = rules::Facts { title: &p.title, product_type: &p.product_type, vendor: &p.vendor, tags: p.tag_list(), price: *price, inventory: *inv };
            rules::collection_matches(&rules_list, c.match_all != 0, &f)
        })
        .map(|(p, _, _)| p.id)
        .collect()
}

// ===========================================================================
// customers
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Customer {
    pub id: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub accepts_marketing: i64,
    pub note: String,
    pub tags: String,
    pub address: String,
    pub member_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Customer {
    pub fn name(&self) -> String {
        let n = format!("{} {}", self.first_name, self.last_name).trim().to_string();
        if n.is_empty() { self.email.clone() } else { n }
    }
    pub fn address(&self) -> Address {
        Address::read(&self.address)
    }
}

const CU_COLS: &str = "id, email, first_name, last_name, phone, accepts_marketing, note, tags, address, member_id, created_at, updated_at";

#[derive(Debug, Clone, Default)]
pub struct CustomerInput {
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub accepts_marketing: bool,
    pub note: String,
    pub tags: Vec<String>,
    pub address: Address,
}

pub async fn find_customer(pool: &Db, id: &str) -> Option<Customer> {
    sqlx::query_as::<_, Customer>(&format!("SELECT {CU_COLS} FROM ecom_customers WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn customer_by_email(pool: &Db, email: &str) -> Option<Customer> {
    sqlx::query_as::<_, Customer>(&format!("SELECT {CU_COLS} FROM ecom_customers WHERE email = ?")).bind(email.to_lowercase()).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_customer(pool: &Db, id: Option<&str>, c: &CustomerInput) -> Result<String> {
    let now = dates::now_db();
    let email = c.email.to_lowercase();
    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_customers WHERE email = ? AND id <> ?").bind(&email).bind(id.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
    if taken > 0 {
        return Err(GhostError::BadRequest("Another customer already has that email.".into()));
    }
    let member: Option<String> = sqlx::query_scalar("SELECT id FROM members WHERE email = ?").bind(&email).fetch_optional(pool).await.ok().flatten();
    match id {
        Some(id) => {
            sqlx::query("UPDATE ecom_customers SET email = ?, first_name = ?, last_name = ?, phone = ?, accepts_marketing = ?, note = ?, tags = ?, address = ?, member_id = ?, updated_at = ? WHERE id = ?")
                .bind(&email).bind(&c.first_name).bind(&c.last_name).bind(&c.phone).bind(flag(c.accepts_marketing)).bind(&c.note).bind(c.tags.join(", "))
                .bind(c.address.to_json()).bind(&member).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query(&format!("INSERT INTO ecom_customers ({CU_COLS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)"))
                .bind(&id).bind(&email).bind(&c.first_name).bind(&c.last_name).bind(&c.phone).bind(flag(c.accepts_marketing)).bind(&c.note)
                .bind(c.tags.join(", ")).bind(c.address.to_json()).bind(&member).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

/// Deletes the customer record; their orders stay, unlinked.
pub async fn delete_customer(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_orders SET customer_id = NULL WHERE customer_id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    sqlx::query("DELETE FROM ecom_customers WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn browse_customers(pool: &Db, terms: &[String], marketing: bool, limit: i64, offset: i64) -> (Vec<Customer>, i64) {
    let mut wh = vec!["1 = 1".to_string()];
    let mut binds: Vec<String> = Vec::new();
    for t in terms {
        wh.push("(LOWER(email) LIKE ? OR LOWER(first_name) LIKE ? OR LOWER(last_name) LIKE ? OR LOWER(tags) LIKE ? OR phone LIKE ?)".into());
        let like = format!("%{}%", t.to_lowercase());
        for _ in 0..5 {
            binds.push(like.clone());
        }
    }
    if marketing {
        wh.push("accepts_marketing = 1".into());
    }
    let wh = wh.join(" AND ");
    let csql = format!("SELECT COUNT(*) FROM ecom_customers WHERE {wh}");
    let mut cq = sqlx::query_scalar::<_, i64>(&csql);
    for b in &binds {
        cq = cq.bind(b.clone());
    }
    let total = cq.fetch_one(pool).await.unwrap_or(0);
    let sql = format!("SELECT {CU_COLS} FROM ecom_customers WHERE {wh} ORDER BY created_at DESC LIMIT ? OFFSET ?");
    let mut q = sqlx::query_as::<_, Customer>(&sql);
    for b in &binds {
        q = q.bind(b.clone());
    }
    (q.bind(limit).bind(offset).fetch_all(pool).await.unwrap_or_default(), total)
}

#[derive(Debug, Clone, Default)]
pub struct CustomerStats {
    pub orders: i64,
    pub last: String,
    pub spending: std::collections::BTreeMap<String, (i64, i64)>,
}

/// Totals net of refunds, grouped by the currency of the original order.
pub async fn customer_stats(pool: &Db, ids_in: &[String]) -> HashMap<String, CustomerStats> {
    let mut out: HashMap<String, CustomerStats> = HashMap::new();
    for chunk in ids_in.chunks(500) {
        let sql = format!(
            "SELECT customer_id, total_cents, refunded_cents, financial_status, created_at, currency FROM ecom_orders WHERE status <> 'cancelled' AND customer_id IN ({})",
            marks(chunk.len())
        );
        let mut q = sqlx::query_as::<_, (String, i64, i64, String, String, String)>(&sql);
        for id in chunk {
            q = q.bind(id);
        }
        for (cid, total, refunded, fin, created, currency) in q.fetch_all(pool).await.unwrap_or_default() {
            let e = out.entry(cid).or_default();
            e.orders += 1;
            if matches!(fin.as_str(), "paid" | "partially_refunded" | "refunded") {
                let (orders, spent) = e.spending.entry(currency).or_default();
                *orders += 1;
                *spent += total - refunded;
            }
            if created > e.last {
                e.last = created;
            }
        }
    }
    out
}

// ===========================================================================
// orders
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Order {
    pub id: String,
    pub number: i64,
    pub token: String,
    pub customer_id: Option<String>,
    pub email: String,
    pub phone: String,
    pub status: String,
    pub financial_status: String,
    pub fulfillment_status: String,
    pub currency: String,
    pub subtotal_cents: i64,
    pub discount_cents: i64,
    pub shipping_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub refunded_cents: i64,
    pub tax_included: i64,
    pub discount_code: String,
    pub shipping_method: String,
    pub shipping_address: String,
    pub payment_method: String,
    pub payment_ref: String,
    pub note: String,
    pub staff_note: String,
    pub tracking_company: String,
    pub tracking_number: String,
    pub tracking_url: String,
    pub source: String,
    pub restocked: i64,
    pub paid_at: Option<String>,
    pub fulfilled_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Order {
    pub fn address(&self) -> Address {
        Address::read(&self.shipping_address)
    }
    pub fn is_paid(&self) -> bool {
        matches!(self.financial_status.as_str(), "paid" | "partially_refunded" | "refunded")
    }
    pub fn is_cancelled(&self) -> bool {
        self.status == "cancelled"
    }
    pub fn net(&self) -> i64 {
        self.total_cents - self.refunded_cents
    }
}

const O_COLS: &str = "id, number, token, customer_id, email, phone, status, financial_status, fulfillment_status, currency, subtotal_cents, \
discount_cents, shipping_cents, tax_cents, total_cents, refunded_cents, tax_included, discount_code, shipping_method, shipping_address, \
payment_method, payment_ref, note, staff_note, tracking_company, tracking_number, tracking_url, source, restocked, paid_at, fulfilled_at, \
cancelled_at, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrderItem {
    pub id: String,
    pub order_id: String,
    pub product_id: Option<String>,
    pub variant_id: Option<String>,
    pub title: String,
    pub variant_title: String,
    pub sku: String,
    pub image: String,
    pub unit_cents: i64,
    pub quantity: i64,
    pub total_cents: i64,
    pub requires_shipping: i64,
    pub position: i64,
}

const I_COLS: &str = "id, order_id, product_id, variant_id, title, variant_title, sku, image, unit_cents, quantity, total_cents, requires_shipping, position";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrderEvent {
    pub id: String,
    pub order_id: String,
    pub kind: String,
    pub message: String,
    pub actor: String,
    pub created_at: String,
}

/// A line to put on a new order, priced by the caller.
#[derive(Debug, Clone)]
pub struct NewLine {
    pub product_id: String,
    pub variant_id: String,
    pub title: String,
    pub variant_title: String,
    pub sku: String,
    pub image: String,
    pub unit_cents: i64,
    pub quantity: i64,
    pub requires_shipping: bool,
    /// Take it out of stock (tracked and no backorder → must be available).
    pub track: bool,
    pub backorder: bool,
}

#[derive(Debug, Clone)]
pub struct NewOrder {
    pub email: String,
    pub phone: String,
    pub first_name: String,
    pub last_name: String,
    pub accepts_marketing: bool,
    pub currency: String,
    pub lines: Vec<NewLine>,
    pub totals: rules::Totals,
    pub tax_included: bool,
    pub discount_code: String,
    pub shipping_method: String,
    pub address: Address,
    pub payment_method: String,
    pub provider_product: Option<String>,
    pub financial_status: String,
    pub note: String,
    pub source: String,
    pub cart_id: Option<String>,
    pub actor: String,
}

pub async fn find_order(pool: &Db, id: &str) -> Option<Order> {
    sqlx::query_as::<_, Order>(&format!("SELECT {O_COLS} FROM ecom_orders WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn order_by_token(pool: &Db, token: &str) -> Option<Order> {
    sqlx::query_as::<_, Order>(&format!("SELECT {O_COLS} FROM ecom_orders WHERE token = ?")).bind(token).fetch_optional(pool).await.ok().flatten()
}

pub async fn order_items(pool: &Db, order_id: &str) -> Vec<OrderItem> {
    sqlx::query_as::<_, OrderItem>(&format!("SELECT {I_COLS} FROM ecom_order_items WHERE order_id = ? ORDER BY position")).bind(order_id).fetch_all(pool).await.unwrap_or_default()
}

pub async fn order_events(pool: &Db, order_id: &str) -> Vec<OrderEvent> {
    sqlx::query_as::<_, OrderEvent>("SELECT id, order_id, kind, message, actor, created_at FROM ecom_order_events WHERE order_id = ? ORDER BY created_at DESC, id DESC")
        .bind(order_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn add_event(pool: &Db, order_id: &str, kind: &str, message: &str, actor: &str) {
    let _ = sqlx::query("INSERT INTO ecom_order_events (id, order_id, kind, message, actor, created_at) VALUES (?,?,?,?,?,?)")
        .bind(ids::new_id()).bind(order_id).bind(kind).bind(message.chars().take(1000).collect::<String>()).bind(actor).bind(dates::now_db())
        .execute(pool)
        .await;
}

/// Place an order: number it, copy its lines, take stock, count the
/// discount, link or create the customer, close the cart — all or nothing.
/// A tracked line without enough stock fails the whole order with a
/// message naming it.
pub async fn place_order(pool: &Db, o: &NewOrder) -> Result<Order> {
    let now = dates::now_db();
    let email = o.email.to_lowercase();
    // Customer: found or created before the transaction (both are reads or
    // independent writes; a retry of the order does not duplicate it).
    let customer_id = match customer_by_email(pool, &email).await {
        Some(c) => {
            let addr = if o.address.is_empty() { c.address() } else { o.address.clone() };
            let _ = sqlx::query("UPDATE ecom_customers SET first_name = CASE WHEN first_name = '' THEN ? ELSE first_name END, last_name = CASE WHEN last_name = '' THEN ? ELSE last_name END, \
                 phone = CASE WHEN ? <> '' THEN ? ELSE phone END, accepts_marketing = CASE WHEN ? = 1 THEN 1 ELSE accepts_marketing END, address = ?, updated_at = ? WHERE id = ?")
                .bind(&o.first_name).bind(&o.last_name).bind(&o.phone).bind(&o.phone).bind(flag(o.accepts_marketing)).bind(addr.to_json()).bind(&now).bind(&c.id)
                .execute(pool).await;
            c.id
        }
        None if !email.is_empty() => save_customer(
            pool,
            None,
            &CustomerInput {
                email: email.clone(),
                first_name: o.first_name.clone(),
                last_name: o.last_name.clone(),
                phone: o.phone.clone(),
                accepts_marketing: o.accepts_marketing,
                address: o.address.clone(),
                ..Default::default()
            },
        )
        .await?,
        None => String::new(),
    };
    let customer = (!customer_id.is_empty()).then_some(customer_id);

    for attempt in 0..5 {
        let mut tx = pool.begin().await.map_err(dberr)?;
        let max: Option<i64> = sqlx::query_scalar("SELECT MAX(number) FROM ecom_orders").fetch_one(&mut *tx).await.map_err(dberr)?;
        let number = max.unwrap_or(1000).max(1000) + 1 + attempt;
        let id = ids::new_id();
        let token = new_token();
        let paid_at = (o.financial_status == "paid").then(|| now.clone());
        let inserted = sqlx::query(&format!("INSERT INTO ecom_orders ({O_COLS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"))
            .bind(&id).bind(number).bind(&token).bind(&customer).bind(&email).bind(&o.phone).bind("open").bind(&o.financial_status).bind("unfulfilled")
            .bind(&o.currency).bind(o.totals.subtotal).bind(o.totals.discount).bind(o.totals.shipping).bind(o.totals.tax).bind(o.totals.total).bind(0_i64)
            .bind(flag(o.tax_included)).bind(&o.discount_code).bind(&o.shipping_method).bind(o.address.to_json()).bind(&o.payment_method).bind("")
            .bind(&o.note).bind("").bind("").bind("").bind("").bind(&o.source).bind(0_i64).bind(&paid_at).bind(None::<String>).bind(None::<String>)
            .bind(&now).bind(&now)
            .execute(&mut *tx)
            .await;
        if inserted.is_err() {
            // Two checkouts took the same number; try the next one.
            let _ = tx.rollback().await;
            continue;
        }
        if let Some(product) = &o.provider_product {
            sqlx::query("INSERT INTO ecom_hosted_payments (order_id, provider, product_id) VALUES (?, ?, ?)")
                .bind(&id).bind(&o.payment_method).bind(product).execute(&mut *tx).await.map_err(dberr)?;
        }
        for (pos, l) in o.lines.iter().enumerate() {
            if l.track {
                let res = if l.backorder {
                    sqlx::query("UPDATE ecom_variants SET inventory = inventory - ? WHERE id = ?").bind(l.quantity).bind(&l.variant_id).execute(&mut *tx).await.map_err(dberr)?
                } else {
                    sqlx::query("UPDATE ecom_variants SET inventory = inventory - ? WHERE id = ? AND inventory >= ?").bind(l.quantity).bind(&l.variant_id).bind(l.quantity).execute(&mut *tx).await.map_err(dberr)?
                };
                if res.rows_affected() == 0 {
                    let _ = tx.rollback().await;
                    let name = if l.variant_title == rules::DEFAULT_VARIANT { l.title.clone() } else { format!("{} ({})", l.title, l.variant_title) };
                    return Err(GhostError::BadRequest(format!("Sorry — there is not enough of “{name}” left in stock.")));
                }
            }
            sqlx::query(&format!("INSERT INTO ecom_order_items ({I_COLS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)"))
                .bind(ids::new_id()).bind(&id).bind(&l.product_id).bind(&l.variant_id).bind(&l.title).bind(&l.variant_title).bind(&l.sku).bind(&l.image)
                .bind(l.unit_cents).bind(l.quantity).bind(l.unit_cents * l.quantity).bind(flag(l.requires_shipping)).bind(pos as i64)
                .execute(&mut *tx).await.map_err(dberr)?;
        }
        if !o.discount_code.is_empty() {
            sqlx::query("UPDATE ecom_discounts SET used_count = used_count + 1 WHERE code = ?").bind(&o.discount_code).execute(&mut *tx).await.map_err(dberr)?;
        }
        if let Some(cart) = &o.cart_id {
            let claimed = sqlx::query("UPDATE ecom_carts SET order_id = ?, updated_at = ? WHERE id = ? AND order_id IS NULL")
                .bind(&id).bind(&now).bind(cart).execute(&mut *tx).await.map_err(dberr)?;
            if claimed.rows_affected() == 0 {
                tx.rollback().await.map_err(dberr)?;
                return Err(GhostError::BadRequest("This cart has already been checked out.".into()));
            }
        }
        sqlx::query("INSERT INTO ecom_order_events (id, order_id, kind, message, actor, created_at) VALUES (?,?,?,?,?,?)")
            .bind(ids::new_id()).bind(&id).bind("placed")
            .bind(if o.source == "admin" { "Order created by staff." } else { "Order placed on the online store." })
            .bind(&o.actor).bind(&now)
            .execute(&mut *tx).await.map_err(dberr)?;
        tx.commit().await.map_err(dberr)?;
        crate::content::touch();
        return find_order(pool, &id).await.ok_or_else(|| GhostError::BadRequest("The order vanished".into()));
    }
    Err(GhostError::BadRequest("Could not number the order; please try again.".into()))
}

#[derive(Debug, Clone, Default)]
pub struct OrderBrowse<'a> {
    pub terms: &'a [String],
    /// `open | unfulfilled | unpaid | archived | cancelled | all`
    pub view: &'a str,
    pub customer_id: Option<&'a str>,
    pub email: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn browse_orders(pool: &Db, b: &OrderBrowse<'_>) -> (Vec<Order>, i64) {
    let mut wh = vec!["1 = 1".to_string()];
    let mut binds: Vec<String> = Vec::new();
    match b.view {
        "open" => wh.push("status = 'open'".into()),
        "unfulfilled" => wh.push("status = 'open' AND fulfillment_status = 'unfulfilled'".into()),
        "unpaid" => wh.push("status = 'open' AND financial_status = 'pending'".into()),
        "archived" => wh.push("status = 'archived'".into()),
        "cancelled" => wh.push("status = 'cancelled'".into()),
        _ => {}
    }
    if let Some(c) = b.customer_id {
        wh.push("customer_id = ?".into());
        binds.push(c.into());
    }
    if let Some(e) = b.email {
        wh.push("email = ?".into());
        binds.push(e.to_lowercase());
    }
    for t in b.terms {
        let digits = t.trim_start_matches('#');
        wh.push("(LOWER(email) LIKE ? OR LOWER(shipping_address) LIKE ? OR CAST(number AS CHAR) = ? OR id IN (SELECT order_id FROM ecom_order_items WHERE LOWER(title) LIKE ? OR LOWER(sku) LIKE ?))".into());
        let like = format!("%{}%", t.to_lowercase());
        binds.extend([like.clone(), like.clone(), digits.to_string(), like.clone(), like]);
    }
    let wh = wh.join(" AND ");
    let csql = format!("SELECT COUNT(*) FROM ecom_orders WHERE {wh}");
    let mut cq = sqlx::query_scalar::<_, i64>(&csql);
    for v in &binds {
        cq = cq.bind(v.clone());
    }
    let total = cq.fetch_one(pool).await.unwrap_or(0);
    let sql = format!("SELECT {O_COLS} FROM ecom_orders WHERE {wh} ORDER BY number DESC LIMIT ? OFFSET ?");
    let mut q = sqlx::query_as::<_, Order>(&sql);
    for v in &binds {
        q = q.bind(v.clone());
    }
    (q.bind(b.limit).bind(b.offset).fetch_all(pool).await.unwrap_or_default(), total)
}

/// How many orders sit in each tab.
pub async fn order_counts(pool: &Db) -> HashMap<&'static str, i64> {
    let one = |sql: &'static str| async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.unwrap_or(0) };
    let mut m = HashMap::new();
    m.insert("all", one("SELECT COUNT(*) FROM ecom_orders").await);
    m.insert("open", one("SELECT COUNT(*) FROM ecom_orders WHERE status = 'open'").await);
    m.insert("unfulfilled", one("SELECT COUNT(*) FROM ecom_orders WHERE status = 'open' AND fulfillment_status = 'unfulfilled'").await);
    m.insert("unpaid", one("SELECT COUNT(*) FROM ecom_orders WHERE status = 'open' AND financial_status = 'pending'").await);
    m.insert("archived", one("SELECT COUNT(*) FROM ecom_orders WHERE status = 'archived'").await);
    m.insert("cancelled", one("SELECT COUNT(*) FROM ecom_orders WHERE status = 'cancelled'").await);
    m
}

async fn touch_order(pool: &Db, id: &str, sql: &str, binds: &[String]) -> Result<()> {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b.clone());
    }
    q.bind(dates::now_db()).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn mark_paid(pool: &Db, id: &str, reference: &str) -> Result<()> {
    let now = dates::now_db();
    touch_order(pool, id, "UPDATE ecom_orders SET financial_status = 'paid', paid_at = ?, payment_ref = CASE WHEN ? <> '' THEN ? ELSE payment_ref END, updated_at = ? WHERE id = ?", &[now, reference.into(), reference.into()]).await
}

pub async fn fulfil(pool: &Db, id: &str, company: &str, number: &str, url: &str) -> Result<()> {
    let now = dates::now_db();
    touch_order(
        pool,
        id,
        "UPDATE ecom_orders SET fulfillment_status = 'fulfilled', fulfilled_at = ?, tracking_company = ?, tracking_number = ?, tracking_url = ?, updated_at = ? WHERE id = ?",
        &[now, company.into(), number.into(), url.into()],
    )
    .await
}

pub async fn unfulfil(pool: &Db, id: &str) -> Result<()> {
    touch_order(pool, id, "UPDATE ecom_orders SET fulfillment_status = 'unfulfilled', fulfilled_at = NULL, updated_at = ? WHERE id = ?", &[]).await
}

/// Put an order's tracked lines back in stock, once.
pub async fn restock(pool: &Db, o: &Order) -> Result<()> {
    if o.restocked != 0 {
        return Ok(());
    }
    let items = order_items(pool, &o.id).await;
    let mut tx = pool.begin().await.map_err(dberr)?;
    for i in items {
        if let Some(v) = &i.variant_id {
            sqlx::query("UPDATE ecom_variants SET inventory = inventory + ? WHERE id = ? AND track_inventory = 1").bind(i.quantity).bind(v).execute(&mut *tx).await.map_err(dberr)?;
        }
    }
    sqlx::query("UPDATE ecom_orders SET restocked = 1 WHERE id = ?").bind(&o.id).execute(&mut *tx).await.map_err(dberr)?;
    tx.commit().await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

pub async fn refund(pool: &Db, o: &Order, amount: i64) -> Result<()> {
    let refunded = (o.refunded_cents + amount).min(o.total_cents);
    let status = rules::refund_status(o.total_cents, refunded);
    touch_order(pool, &o.id, "UPDATE ecom_orders SET refunded_cents = ?, financial_status = ?, updated_at = ? WHERE id = ?", &[refunded.to_string(), status.into()]).await
}

pub async fn cancel(pool: &Db, o: &Order) -> Result<()> {
    let fin = if o.financial_status == "pending" { "voided".to_string() } else { o.financial_status.clone() };
    touch_order(pool, &o.id, "UPDATE ecom_orders SET status = 'cancelled', financial_status = ?, cancelled_at = ?, updated_at = ? WHERE id = ?", &[fin, dates::now_db()]).await
}

pub async fn set_order_status(pool: &Db, id: &str, status: &str) -> Result<()> {
    touch_order(pool, id, "UPDATE ecom_orders SET status = ?, updated_at = ? WHERE id = ?", &[status.into()]).await
}

pub async fn set_staff_note(pool: &Db, id: &str, note: &str) -> Result<()> {
    touch_order(pool, id, "UPDATE ecom_orders SET staff_note = ?, updated_at = ? WHERE id = ?", &[note.into()]).await
}

pub async fn set_order_contact(pool: &Db, id: &str, email: &str, phone: &str, address: &Address) -> Result<()> {
    touch_order(pool, id, "UPDATE ecom_orders SET email = ?, phone = ?, shipping_address = ?, updated_at = ? WHERE id = ?", &[email.to_lowercase(), phone.into(), address.to_json()]).await
}

pub async fn set_payment_ref(pool: &Db, id: &str, reference: &str) -> Result<()> {
    touch_order(pool, id, "UPDATE ecom_orders SET payment_ref = ?, updated_at = ? WHERE id = ?", &[reference.into()]).await
}

pub async fn delete_order(pool: &Db, id: &str) -> Result<()> {
    let mut tx = pool.begin().await.map_err(dberr)?;
    for sql in ["DELETE FROM ecom_order_items WHERE order_id = ?", "DELETE FROM ecom_order_events WHERE order_id = ?", "DELETE FROM ecom_orders WHERE id = ?"] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await.map_err(dberr)?;
    }
    tx.commit().await.map_err(dberr)?;
    Ok(())
}

/// Orders since `from` (a DB date string), lightly: for reports.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SaleRow {
    pub id: String,
    pub created_at: String,
    pub total_cents: i64,
    pub refunded_cents: i64,
    pub discount_cents: i64,
    pub tax_cents: i64,
    pub shipping_cents: i64,
    pub financial_status: String,
    pub customer_id: Option<String>,
}

pub async fn sales_since(pool: &Db, from: &str, currency: &str) -> Vec<SaleRow> {
    sqlx::query_as::<_, SaleRow>(
        "SELECT id, created_at, total_cents, refunded_cents, discount_cents, tax_cents, shipping_cents, financial_status, customer_id FROM ecom_orders \
         WHERE created_at >= ? AND currency = ? AND status <> 'cancelled' ORDER BY created_at",
    )
    .bind(from)
    .bind(currency)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// `(product_id, title, units, revenue)` best first, since `from`.
pub async fn top_products(pool: &Db, from: &str, currency: &str, limit: usize) -> Vec<(String, String, i64, i64)> {
    let rows = sqlx::query_as::<_, (Option<String>, String, i64, i64)>(
        "SELECT i.product_id, i.title, i.quantity, i.total_cents FROM ecom_order_items i JOIN ecom_orders o ON o.id = i.order_id \
         WHERE o.created_at >= ? AND o.currency = ? AND o.status <> 'cancelled'",
    )
    .bind(from)
    .bind(currency)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut agg: HashMap<String, (String, i64, i64)> = HashMap::new();
    for (pid, title, q, t) in rows {
        let key = pid.unwrap_or_else(|| format!("deleted:{title}"));
        let e = agg.entry(key).or_insert((title, 0, 0));
        e.1 += q;
        e.2 += t;
    }
    let mut v: Vec<(String, String, i64, i64)> = agg.into_iter().map(|(k, (t, q, r))| (k, t, q, r)).collect();
    v.sort_by(|a, b| b.3.cmp(&a.3).then(b.2.cmp(&a.2)));
    v.truncate(limit);
    v
}

/// `(units sold, revenue)` for one product, all time.
pub async fn product_sales(pool: &Db, product_id: &str) -> (i64, i64) {
    let rows = sqlx::query_as::<_, (i64, i64)>("SELECT i.quantity, i.total_cents FROM ecom_order_items i JOIN ecom_orders o ON o.id = i.order_id WHERE i.product_id = ? AND o.status <> 'cancelled'")
        .bind(product_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter().fold((0, 0), |a, r| (a.0 + r.0, a.1 + r.1))
}

// ===========================================================================
// discounts
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Discount {
    pub id: String,
    pub code: String,
    pub kind: String,
    pub value: i64,
    pub min_subtotal_cents: i64,
    pub usage_limit: i64,
    pub once_per_customer: i64,
    pub used_count: i64,
    pub active: i64,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub created_at: String,
}

const D_COLS: &str = "id, code, kind, value, min_subtotal_cents, usage_limit, once_per_customer, used_count, active, starts_at, ends_at, created_at";

#[derive(Debug, Clone, Default)]
pub struct DiscountInput {
    pub code: String,
    pub kind: String,
    pub value: i64,
    pub min_subtotal_cents: i64,
    pub usage_limit: i64,
    pub once_per_customer: bool,
    pub active: bool,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
}

pub async fn discounts(pool: &Db) -> Vec<Discount> {
    sqlx::query_as::<_, Discount>(&format!("SELECT {D_COLS} FROM ecom_discounts ORDER BY created_at DESC")).fetch_all(pool).await.unwrap_or_default()
}

pub async fn find_discount(pool: &Db, id: &str) -> Option<Discount> {
    sqlx::query_as::<_, Discount>(&format!("SELECT {D_COLS} FROM ecom_discounts WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

pub async fn discount_by_code(pool: &Db, code: &str) -> Option<Discount> {
    sqlx::query_as::<_, Discount>(&format!("SELECT {D_COLS} FROM ecom_discounts WHERE code = ?")).bind(code).fetch_optional(pool).await.ok().flatten()
}

pub async fn save_discount(pool: &Db, id: Option<&str>, d: &DiscountInput) -> Result<String> {
    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_discounts WHERE code = ? AND id <> ?").bind(&d.code).bind(id.unwrap_or("")).fetch_one(pool).await.unwrap_or(0);
    if taken > 0 {
        return Err(GhostError::BadRequest("Another discount already uses that code.".into()));
    }
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE ecom_discounts SET code = ?, kind = ?, value = ?, min_subtotal_cents = ?, usage_limit = ?, once_per_customer = ?, active = ?, starts_at = ?, ends_at = ?, updated_at = ? WHERE id = ?")
                .bind(&d.code).bind(&d.kind).bind(d.value).bind(d.min_subtotal_cents).bind(d.usage_limit).bind(flag(d.once_per_customer)).bind(flag(d.active))
                .bind(&d.starts_at).bind(&d.ends_at).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            sqlx::query(&format!("INSERT INTO ecom_discounts ({D_COLS}, updated_at) VALUES (?,?,?,?,?,?,?,0,?,?,?,?,?)"))
                .bind(&id).bind(&d.code).bind(&d.kind).bind(d.value).bind(d.min_subtotal_cents).bind(d.usage_limit).bind(flag(d.once_per_customer))
                .bind(flag(d.active)).bind(&d.starts_at).bind(&d.ends_at).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_discount(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM ecom_discounts WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn code_used_by(pool: &Db, code: &str, email: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ecom_orders WHERE discount_code = ? AND email = ? AND status <> 'cancelled'")
        .bind(code)
        .bind(email.to_lowercase())
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        > 0
}

// ===========================================================================
// shipping
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ShippingRate {
    pub id: String,
    pub name: String,
    pub price_cents: i64,
    pub min_subtotal_cents: i64,
    pub max_subtotal_cents: i64,
    pub countries: String,
    pub position: i64,
}

impl ShippingRate {
    pub fn applies(&self, country: &str, subtotal: i64) -> bool {
        let list = rules::parse_countries(&self.countries);
        (list.is_empty() || country.is_empty() || list.iter().any(|c| c == country))
            && subtotal >= self.min_subtotal_cents
            && (self.max_subtotal_cents == 0 || subtotal <= self.max_subtotal_cents)
    }
}

pub async fn shipping_rates(pool: &Db) -> Vec<ShippingRate> {
    sqlx::query_as::<_, ShippingRate>("SELECT id, name, price_cents, min_subtotal_cents, max_subtotal_cents, countries, position FROM ecom_shipping_rates ORDER BY position, price_cents")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn save_shipping_rate(pool: &Db, id: Option<&str>, name: &str, price: i64, min: i64, max: i64, countries: &str) -> Result<String> {
    let now = dates::now_db();
    match id {
        Some(id) => {
            sqlx::query("UPDATE ecom_shipping_rates SET name = ?, price_cents = ?, min_subtotal_cents = ?, max_subtotal_cents = ?, countries = ?, updated_at = ? WHERE id = ?")
                .bind(name).bind(price).bind(min).bind(max).bind(countries).bind(&now).bind(id)
                .execute(pool).await.map_err(dberr)?;
            Ok(id.to_string())
        }
        None => {
            let id = ids::new_id();
            let pos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_shipping_rates").fetch_one(pool).await.unwrap_or(0);
            sqlx::query("INSERT INTO ecom_shipping_rates (id, name, price_cents, min_subtotal_cents, max_subtotal_cents, countries, position, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(name).bind(price).bind(min).bind(max).bind(countries).bind(pos).bind(&now).bind(&now)
                .execute(pool).await.map_err(dberr)?;
            Ok(id)
        }
    }
}

pub async fn delete_shipping_rate(pool: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM ecom_shipping_rates WHERE id = ?").bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ===========================================================================
// carts
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Cart {
    pub id: String,
    pub email: String,
    pub items: String,
    pub discount_code: String,
    pub order_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Cart {
    pub fn lines(&self) -> Vec<CartLine> {
        rules::read_cart(&self.items)
    }
}

pub async fn find_cart(pool: &Db, token: &str) -> Option<Cart> {
    if !rules::is_cart_token(token) {
        return None;
    }
    sqlx::query_as::<_, Cart>("SELECT id, email, items, discount_code, order_id, created_at, updated_at FROM ecom_carts WHERE id = ?")
        .bind(token)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|c| c.order_id.is_none())
}

pub async fn create_cart(pool: &Db) -> Result<String> {
    let token = new_token();
    let now = dates::now_db();
    sqlx::query("INSERT INTO ecom_carts (id, email, items, discount_code, order_id, created_at, updated_at) VALUES (?, '', '[]', '', NULL, ?, ?)")
        .bind(&token).bind(&now).bind(&now)
        .execute(pool).await.map_err(dberr)?;
    Ok(token)
}

pub async fn save_cart_lines(pool: &Db, token: &str, lines: &[CartLine]) -> Result<()> {
    let json = serde_json::to_string(lines).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE ecom_carts SET items = ?, updated_at = ? WHERE id = ? AND order_id IS NULL").bind(json).bind(dates::now_db()).bind(token).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_cart_discount(pool: &Db, token: &str, code: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_carts SET discount_code = ?, updated_at = ? WHERE id = ? AND order_id IS NULL").bind(code).bind(dates::now_db()).bind(token).execute(pool).await.map_err(dberr)?;
    Ok(())
}

pub async fn set_cart_email(pool: &Db, token: &str, email: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_carts SET email = ?, updated_at = ? WHERE id = ? AND order_id IS NULL").bind(email.to_lowercase()).bind(dates::now_db()).bind(token).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Checkouts that got as far as an email, then stopped, before `cutoff`.
pub async fn abandoned(pool: &Db, cutoff: &str, limit: i64) -> Vec<Cart> {
    sqlx::query_as::<_, Cart>(
        "SELECT id, email, items, discount_code, order_id, created_at, updated_at FROM ecom_carts WHERE email <> '' AND order_id IS NULL AND items <> '[]' \
         AND updated_at < ? ORDER BY updated_at DESC LIMIT ?",
    )
    .bind(cutoff)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Forget carts nobody has touched since `before` (and that never became
/// an order — those are kept with the order).
pub async fn purge_carts(pool: &Db, before: &str) -> u64 {
    sqlx::query("DELETE FROM ecom_carts WHERE updated_at < ? AND order_id IS NULL").bind(before).execute(pool).await.map(|r| r.rows_affected()).unwrap_or(0)
}

pub async fn delete_cart(pool: &Db, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM ecom_carts WHERE id = ? AND order_id IS NULL").bind(token).execute(pool).await.map_err(dberr)?;
    Ok(())
}

// ===========================================================================
// digital downloads
// ===========================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EcomFile {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub filename: String,
    pub stored: String,
    pub size_bytes: i64,
    pub content_type: String,
    pub position: i64,
    pub created_at: String,
}

const F_COLS: &str = "id, product_id, name, filename, stored, size_bytes, content_type, position, created_at";

pub async fn files(pool: &Db, product_id: &str) -> Vec<EcomFile> {
    sqlx::query_as::<_, EcomFile>(&format!("SELECT {F_COLS} FROM ecom_files WHERE product_id = ? ORDER BY position, created_at"))
        .bind(product_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Files of several products, grouped by product.
pub async fn files_for(pool: &Db, product_ids: &[String]) -> HashMap<String, Vec<EcomFile>> {
    let mut out: HashMap<String, Vec<EcomFile>> = HashMap::new();
    for chunk in product_ids.chunks(500) {
        let sql = format!("SELECT {F_COLS} FROM ecom_files WHERE product_id IN ({}) ORDER BY position, created_at", marks(chunk.len()));
        let mut q = sqlx::query_as::<_, EcomFile>(&sql);
        for id in chunk {
            q = q.bind(id);
        }
        for f in q.fetch_all(pool).await.unwrap_or_default() {
            out.entry(f.product_id.clone()).or_default().push(f);
        }
    }
    out
}

pub async fn find_file(pool: &Db, id: &str) -> Option<EcomFile> {
    sqlx::query_as::<_, EcomFile>(&format!("SELECT {F_COLS} FROM ecom_files WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

/// Record a file already written to disk under `id` (its stored name).
pub async fn insert_file(pool: &Db, id: &str, product_id: &str, name: &str, filename: &str, size: i64, content_type: &str) -> Result<()> {
    let pos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_files WHERE product_id = ?").bind(product_id).fetch_one(pool).await.unwrap_or(0);
    sqlx::query(&format!("INSERT INTO ecom_files ({F_COLS}) VALUES (?,?,?,?,?,?,?,?,?)"))
        .bind(id).bind(product_id).bind(name).bind(filename).bind(id).bind(size).bind(content_type).bind(pos).bind(dates::now_db())
        .execute(pool).await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

pub async fn rename_file(pool: &Db, id: &str, name: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_files SET name = ? WHERE id = ?").bind(name).bind(id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Deletes the row; returns its stored name for the caller to remove.
pub async fn delete_file(pool: &Db, id: &str) -> Option<String> {
    let f = find_file(pool, id).await?;
    sqlx::query("DELETE FROM ecom_files WHERE id = ?").bind(id).execute(pool).await.ok()?;
    crate::content::touch();
    Some(f.stored)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Grant {
    pub id: String,
    pub order_id: String,
    pub order_item_id: String,
    pub product_id: String,
    pub title: String,
    pub email: String,
    pub member_id: Option<String>,
    pub download_count: i64,
    pub download_limit: i64,
    pub expires_at: Option<String>,
    pub revoked: i64,
    pub last_download_at: Option<String>,
    pub created_at: String,
}

const G_COLS: &str = "id, order_id, order_item_id, product_id, title, email, member_id, download_count, download_limit, expires_at, revoked, last_download_at, created_at";

/// Grant the order's digital lines, once each. Returns how many grants the
/// order has afterwards (new or old).
pub async fn grant_downloads(pool: &Db, order: &Order, member_id: Option<&str>) -> Result<usize> {
    let items = order_items(pool, &order.id).await;
    let pids: Vec<String> = items.iter().filter_map(|i| i.product_id.clone()).collect();
    let with_files = files_for(pool, &pids).await;
    let products = products_by_ids(pool, &pids).await;
    let now = dates::now_db();
    let mut n = 0;
    for i in &items {
        let Some(pid) = &i.product_id else { continue };
        if !with_files.contains_key(pid) {
            continue;
        }
        let p = products.iter().find(|p| &p.id == pid);
        let limit = p.map(|p| p.download_limit).unwrap_or(0).max(0) * i.quantity.max(1);
        let days = p.map(|p| p.download_days).unwrap_or(0);
        let expires = (days > 0).then(|| dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::days(days))));
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ecom_downloads WHERE order_item_id = ?").bind(&i.id).fetch_one(pool).await.unwrap_or(0);
        if exists == 0 {
            sqlx::query(&format!("INSERT INTO ecom_downloads ({G_COLS}) VALUES (?,?,?,?,?,?,?,0,?,?,0,NULL,?)"))
                .bind(ids::new_id()).bind(&order.id).bind(&i.id).bind(pid).bind(&i.title).bind(order.email.to_lowercase()).bind(member_id)
                .bind(limit).bind(&expires).bind(&now)
                .execute(pool).await.map_err(dberr)?;
        }
        n += 1;
    }
    if let Some(m) = member_id {
        sqlx::query("UPDATE ecom_downloads SET member_id = ? WHERE order_id = ? AND member_id IS NULL").bind(m).bind(&order.id).execute(pool).await.map_err(dberr)?;
    }
    Ok(n)
}

pub async fn grants_for_order(pool: &Db, order_id: &str) -> Vec<Grant> {
    sqlx::query_as::<_, Grant>(&format!("SELECT {G_COLS} FROM ecom_downloads WHERE order_id = ? ORDER BY created_at, id"))
        .bind(order_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Every grant for an email, newest first.
pub async fn grants_for_email(pool: &Db, email: &str) -> Vec<Grant> {
    sqlx::query_as::<_, Grant>(&format!("SELECT {G_COLS} FROM ecom_downloads WHERE email = ? ORDER BY created_at DESC, id"))
        .bind(email.to_lowercase())
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn find_grant(pool: &Db, id: &str) -> Option<Grant> {
    sqlx::query_as::<_, Grant>(&format!("SELECT {G_COLS} FROM ecom_downloads WHERE id = ?")).bind(id).fetch_optional(pool).await.ok().flatten()
}

/// Count one download, atomically against the limit. False when the limit
/// was reached between the check and now.
pub async fn record_download(pool: &Db, g: &Grant, file_id: &str, member_id: &str, ip: &str, user_agent: &str) -> bool {
    let now = dates::now_db();
    let done = sqlx::query("UPDATE ecom_downloads SET download_count = download_count + 1, last_download_at = ? WHERE id = ? AND revoked = 0 AND (download_limit = 0 OR download_count < download_limit)")
        .bind(&now)
        .bind(&g.id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() == 1)
        .unwrap_or(false);
    if done {
        let _ = sqlx::query("INSERT INTO ecom_download_log (id, grant_id, file_id, member_id, ip, user_agent, created_at) VALUES (?,?,?,?,?,?,?)")
            .bind(ids::new_id()).bind(&g.id).bind(file_id).bind(member_id).bind(ip.chars().take(64).collect::<String>())
            .bind(user_agent.chars().take(300).collect::<String>()).bind(&now)
            .execute(pool).await;
    }
    done
}

pub async fn set_order_downloads_revoked(pool: &Db, order_id: &str, revoked: bool) -> Result<()> {
    sqlx::query("UPDATE ecom_downloads SET revoked = ? WHERE order_id = ?").bind(flag(revoked)).bind(order_id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// Reset a grant's count (staff giving a buyer more downloads).
pub async fn reset_download_count(pool: &Db, grant_id: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_downloads SET download_count = 0 WHERE id = ?").bind(grant_id).execute(pool).await.map_err(dberr)?;
    Ok(())
}

/// `(when, file name, ip)` of the latest downloads of an order.
pub async fn download_log_for_order(pool: &Db, order_id: &str, limit: i64) -> Vec<(String, String, String)> {
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT l.created_at, COALESCE(f.name, '(deleted file)'), l.ip FROM ecom_download_log l JOIN ecom_downloads g ON g.id = l.grant_id \
         LEFT JOIN ecom_files f ON f.id = l.file_id WHERE g.order_id = ? ORDER BY l.created_at DESC LIMIT ?",
    )
    .bind(order_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Which of these products carry files.
pub async fn digital_ids(pool: &Db, product_ids: &[String]) -> std::collections::HashSet<String> {
    files_for(pool, product_ids).await.into_keys().collect()
}

// ---------------------------------------------------------------------------
// products as theme data
// ---------------------------------------------------------------------------

/// A product as a theme sees it: what `{{#get "products"}}` returns and what
/// the store hands its `shop/*.hbs` templates. `url` is the product's public
/// address (the caller knows the store's paths).
///
/// Tags written `key:value` are gathered into `meta` (`niche:Magazine` →
/// `meta.niche`) and left out of `tags`; the first link in the description
/// whose text mentions "demo" becomes `demo_url` when the product has no
/// demo link of its own.
pub fn card_json(p: &Product, vs: &[Variant], s: &rules::Settings, url: &str) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let cheapest = vs.iter().min_by_key(|v| v.price_cents);
    let min = cheapest.map(|v| v.price_cents).unwrap_or(0);
    let max = vs.iter().map(|v| v.price_cents).max().unwrap_or(0);
    let compare = cheapest.map(|v| v.compare_at_cents).unwrap_or(0);
    let amount = if min == 0 { "Free".to_string() } else { rules::format_money(min, &s.currency) };
    let price = if max > min { format!("From {amount}") } else { amount };
    let tags = p.tag_list();
    let mut meta = Map::new();
    for t in &tags {
        if let Some((k, v)) = t.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if !k.is_empty() && !v.is_empty() && !meta.contains_key(&k) {
                meta.insert(k, json!(v));
            }
        }
    }
    let plain: Vec<&String> = tags.iter().filter(|t| !t.contains(':')).collect();
    let images = p.image_list();
    // the values of a "Color"/"Colour" option, for swatches on a card
    let colors: Vec<String> = p
        .option_list()
        .into_iter()
        .find(|o| matches!(o.name.trim().to_ascii_lowercase().as_str(), "color" | "colour" | "finish"))
        .map(|o| o.values)
        .unwrap_or_default();
    json!({
        "id": p.id,
        "title": p.title,
        "slug": p.handle,
        "handle": p.handle,
        "url": url,
        "excerpt": p.excerpt,
        "vendor": p.vendor,
        "product_type": p.product_type,
        "tags": plain,
        "meta": meta,
        // Ghost's name for a lead image; gscan refuses a bare `image`
        "feature_image": images.first(),
        "image": images.first(),
        "images": images,
        "price": price,
        "price_cents": min,
        "compare_at": if compare > min { json!(rules::format_money(compare, &s.currency)) } else { Value::Null },
        "on_sale": compare > min,
        "free": max == 0,
        "sold_out": !vs.is_empty() && vs.iter().all(|v| !v.available(1)),
        "featured": p.featured != 0,
        // the product's own field first; older products named it with a
        // link in the description
        "demo_url": if p.demo_url.is_empty() { demo_link(&p.description_html) } else { Some(p.demo_url.clone()) },
        "purchase_url": if p.purchase_url.is_empty() { Value::Null } else { json!(p.purchase_url) },
        "published_at": p.published_at,
        "save_percent": if compare > min && compare > 0 { json!(((compare - min) * 100 + compare / 2) / compare) } else { Value::Null },
        "colors": colors,
        "variant_count": vs.len(),
        // one variant to add straight from a card: only when there is
        // nothing to choose and it can be bought
        "quick_add": if vs.len() == 1 && vs[0].available(1) { json!(vs[0].id) } else { Value::Null },
    })
}

/// The first `http(s)` link in `html` whose text says "demo".
pub fn demo_link(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<a").map(|i| i + from) {
        from = i + 2;
        if !lower[from..].starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let Some(tag_end) = lower[from..].find('>').map(|e| e + from) else { break };
        let Some(close) = lower[tag_end..].find("</a>").map(|e| e + tag_end) else { break };
        let attrs = &html[from..tag_end];
        let text = &lower[tag_end + 1..close];
        from = close;
        if !text.contains("demo") {
            continue;
        }
        let la = attrs.to_ascii_lowercase();
        let Some(h) = la.find("href") else { continue };
        let rest = attrs[h + 4..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(end) = rest[1..].find(q) else { continue };
        let href = rest[1..1 + end].replace("&amp;", "&");
        let hl = href.to_ascii_lowercase();
        if (hl.starts_with("https://") || hl.starts_with("http://")) && !href.contains(char::is_whitespace) {
            return Some(href);
        }
    }
    None
}

#[cfg(test)]
mod card_tests {
    use super::demo_link;

    #[test]
    fn a_demo_link_is_found_by_its_text() {
        let html = r#"<p>See <a href="https://x.test/docs">the docs</a> or the <a class="b" href="https://demo.test/?a=1&amp;b=2">Live <b>demo</b></a>.</p>"#;
        assert_eq!(demo_link(html).as_deref(), Some("https://demo.test/?a=1&b=2"));
        assert_eq!(demo_link(r#"<a href="javascript:demo()">demo</a>"#), None);
        assert_eq!(demo_link(r#"<abbr>demo</abbr><a href='http://d.test'>Demo</a>"#).as_deref(), Some("http://d.test"));
        assert_eq!(demo_link("<p>no links</p>"), None);
    }
}
