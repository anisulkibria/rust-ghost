//! Store product links and settlement records for hosted payment providers.
use super::{dberr, ecom::Order};
use crate::Db;
use vuuut_core::{Result, dates, ids};

#[derive(Clone, Default, sqlx::FromRow)]
pub struct ProductMapping {
    pub polar_product: String,
    pub gumroad_product: String,
}

pub async fn digital_products(db: &Db) -> Result<Vec<(String, String)>> {
    sqlx::query_as("SELECT id, title FROM ecom_products WHERE requires_shipping = 0 ORDER BY title, id")
        .fetch_all(db).await.map_err(dberr)
}

pub async fn mapping(db: &Db, product: &str) -> Result<ProductMapping> {
    sqlx::query_as("SELECT polar_product, gumroad_product FROM ecom_payment_products WHERE product_id = ?")
        .bind(product).fetch_optional(db).await.map(|row| row.unwrap_or_default()).map_err(dberr)
}

pub async fn save_mapping(db: &Db, product: &str, mapping: &ProductMapping) -> Result<()> {
    sqlx::query(crate::dialect::sql(
        "INSERT INTO ecom_payment_products (product_id, polar_product, gumroad_product) VALUES (?, ?, ?) \
         ON CONFLICT(product_id) DO UPDATE SET polar_product = excluded.polar_product, gumroad_product = excluded.gumroad_product",
    ))
    .bind(product)
    .bind(&mapping.polar_product)
    .bind(&mapping.gumroad_product)
    .execute(db).await.map_err(dberr)?;
    crate::content::touch();
    Ok(())
}

#[derive(sqlx::FromRow)]
pub struct HostedPayment {
    pub provider: String,
    pub product_id: String,
    pub checkout_id: String,
    pub checkout_url: String,
}

pub async fn hosted(db: &Db, order_id: &str) -> Result<Option<HostedPayment>> {
    sqlx::query_as("SELECT provider, product_id, checkout_id, checkout_url FROM ecom_hosted_payments WHERE order_id = ?")
        .bind(order_id).fetch_optional(db).await.map_err(dberr)
}

/// Only the first stored checkout URL is shared with the customer.
pub async fn register_checkout(db: &Db, order_id: &str, checkout_id: &str, url: &str) -> Result<()> {
    sqlx::query("UPDATE ecom_hosted_payments SET checkout_id = ?, checkout_url = ? WHERE order_id = ? AND checkout_url = ''")
        .bind(checkout_id).bind(url).bind(order_id).execute(db).await.map_err(dberr)?;
    Ok(())
}

/// Claim a provider receipt and update the order together. The return value
/// identifies the delivery responsible for the receipt and notifications.
pub async fn settle(db: &Db, order: &Order, receipt: &str, message: &str) -> Result<bool> {
    let mut tx = db.begin().await.map_err(dberr)?;
    let claim = sqlx::query(
        "UPDATE ecom_hosted_payments SET receipt_id = ? WHERE order_id = ? AND provider = ? AND receipt_id IS NULL",
    )
    .bind(receipt).bind(&order.id).bind(&order.payment_method)
    .execute(&mut *tx).await.map_err(dberr)?;
    if claim.rows_affected() == 0 {
        tx.rollback().await.map_err(dberr)?;
        return Ok(false);
    }
    let now = dates::now_db();
    let paid = sqlx::query(
        "UPDATE ecom_orders SET financial_status = 'paid', paid_at = ?, payment_ref = ?, updated_at = ? \
         WHERE id = ? AND payment_method = ? AND financial_status = 'pending' AND cancelled_at IS NULL",
    )
    .bind(&now).bind(receipt).bind(&now).bind(&order.id).bind(&order.payment_method)
    .execute(&mut *tx).await.map_err(dberr)?;
    if paid.rows_affected() == 0 {
        tx.rollback().await.map_err(dberr)?;
        return Ok(false);
    }
    sqlx::query("INSERT INTO ecom_order_events (id, order_id, kind, message, actor, created_at) VALUES (?, ?, 'paid', ?, '', ?)")
        .bind(ids::new_id()).bind(&order.id).bind(message).bind(&now).execute(&mut *tx).await.map_err(dberr)?;
    tx.commit().await.map_err(dberr)?;
    crate::content::touch();
    Ok(true)
}
