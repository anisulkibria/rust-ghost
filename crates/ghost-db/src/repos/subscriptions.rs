//! Atomic Stripe subscription synchronization without deleting unrelated grants.
use super::dberr;
use ghost_core::{dates, ids, Result};
use serde_json::Value;
use sqlx::SqlitePool;
pub async fn sync(pool: &SqlitePool, sub: &Value) -> Result<Option<String>> {
    let id = sub["id"].as_str().unwrap_or("");
    let customer = sub["customer"]
        .as_str()
        .or_else(|| sub.pointer("/customer/id").and_then(Value::as_str))
        .unwrap_or("");
    if id.is_empty() || customer.is_empty() {
        return Ok(None);
    }
    let member: Option<String> = sqlx::query_scalar(
        "SELECT member_id FROM members_stripe_customers WHERE customer_id=? LIMIT 1",
    )
    .bind(customer)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    let Some(member) = member else {
        return Ok(None);
    };
    let price = sub
        .pointer("/items/data/0/price")
        .or_else(|| sub.get("plan"))
        .unwrap_or(&Value::Null);
    let price_id = price["id"].as_str().unwrap_or("");
    let status = sub["status"].as_str().unwrap_or("incomplete");
    let active = matches!(status, "active" | "trialing");
    let end = sub["current_period_end"]
        .as_i64()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string());
    let started = sub["start_date"]
        .as_i64()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(dates::now_db);
    let cancelled = sub["cancel_at_period_end"].as_bool().unwrap_or(false);
    let now = dates::now_db();
    let mut tx = pool.begin().await.map_err(dberr)?;
    sqlx::query("INSERT INTO subscriptions (id,customer_id,subscription_id,status,cancel_at_period_end,current_period_end,start_date,stripe_price_id,plan_interval,plan_currency,plan_amount,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(subscription_id) DO UPDATE SET status=excluded.status,cancel_at_period_end=excluded.cancel_at_period_end,current_period_end=excluded.current_period_end,stripe_price_id=excluded.stripe_price_id,plan_interval=excluded.plan_interval,plan_currency=excluded.plan_currency,plan_amount=excluded.plan_amount,updated_at=excluded.updated_at")
        .bind(ids::new_id()).bind(customer).bind(id).bind(status).bind(cancelled).bind(&end).bind(started).bind(price_id).bind(price.pointer("/recurring/interval").or_else(||price.get("interval")).and_then(Value::as_str).unwrap_or("month"))
        .bind(price["currency"].as_str().unwrap_or("usd")).bind(price.get("unit_amount").or_else(||price.get("amount")).and_then(Value::as_i64)).bind(&now).bind(&now).execute(&mut *tx).await.map_err(dberr)?;
    sqlx::query("DELETE FROM members_products WHERE member_id=? AND subscription_id=?")
        .bind(&member)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr)?;
    if active {
        let tier: Option<String> = sqlx::query_scalar(
            "SELECT id FROM products WHERE monthly_price_id=? OR yearly_price_id=? LIMIT 1",
        )
        .bind(price_id)
        .bind(price_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr)?;
        if let Some(tier) = tier {
            sqlx::query("INSERT INTO members_products (id,member_id,product_id,subscription_id,expiry_at) VALUES (?,?,?,?,?)")
            .bind(ids::new_id()).bind(&member).bind(tier).bind(id).bind(&end).execute(&mut *tx).await.map_err(dberr)?;
        }
    }
    sqlx::query("UPDATE members SET status=CASE WHEN EXISTS(SELECT 1 FROM members_stripe_customers c JOIN subscriptions s ON s.customer_id=c.customer_id WHERE c.member_id=members.id AND s.status IN ('active','trialing')) THEN 'paid' WHEN EXISTS(SELECT 1 FROM members_products mp JOIN products p ON p.id=mp.product_id WHERE mp.member_id=members.id AND p.type='paid' AND mp.subscription_id IS NULL AND (mp.expiry_at IS NULL OR datetime(mp.expiry_at)>datetime('now'))) THEN CASE WHEN status='gift' THEN 'gift' ELSE 'comped' END ELSE 'free' END, updated_at=? WHERE id=?")
        .bind(now).bind(&member).execute(&mut *tx).await.map_err(dberr)?;
    tx.commit().await.map_err(dberr)?;
    Ok(Some(member))
}
