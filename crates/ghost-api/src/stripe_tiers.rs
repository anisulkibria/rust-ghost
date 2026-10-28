//! Keep locally configured tiers backed by usable Stripe products and prices.
use crate::AppState;
use ghost_core::{GhostError, Result};
use ghost_stripe::StripeConfig;

fn db_error(error: sqlx::Error) -> GhostError { GhostError::Db(error.to_string()) }

/// Reuse matching prices; Stripe prices are immutable and must only be replaced
/// when their amount or currency changes. Shared by connection, tier edits,
/// and the checkout repair path for sites connected before tier sync existed.
#[allow(clippy::too_many_arguments)]
pub async fn sync_tier(
    state: &AppState, cfg: &StripeConfig, tier_id: &str, name: &str,
    currency: &str, monthly: Option<i64>, yearly: Option<i64>,
) -> Result<()> {
    static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SYNC.lock().await;
    let existing: Option<String> = sqlx::query_scalar("SELECT stripe_product_id FROM stripe_products WHERE product_id = ? LIMIT 1")
        .bind(tier_id).fetch_optional(&state.db).await.map_err(db_error)?;
    let product_id = match existing {
        Some(id) => id,
        None => ghost_stripe::ensure_product(cfg, name, None).await?,
    };
    let now = ghost_core::dates::now_db();
    let mapped: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stripe_products WHERE product_id = ? AND stripe_product_id = ?")
        .bind(tier_id).bind(&product_id).fetch_one(&state.db).await.map_err(db_error)?;
    if mapped == 0 {
        sqlx::query("INSERT INTO stripe_products (id, product_id, stripe_product_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(ghost_core::ids::new_id()).bind(tier_id).bind(&product_id).bind(&now).bind(&now)
            .execute(&state.db).await.map_err(db_error)?;
    }
    for (amount, interval, column) in [(monthly, "month", "monthly_price_id"), (yearly, "year", "yearly_price_id")] {
        let Some(amount) = amount.filter(|amount| *amount > 0) else { continue };
        let cached: Option<String> = sqlx::query_scalar("SELECT stripe_price_id FROM stripe_prices WHERE stripe_product_id = ? AND amount = ? AND lower(currency) = lower(?) AND interval = ? AND active = 1 LIMIT 1")
            .bind(&product_id).bind(amount).bind(currency).bind(interval)
            .fetch_optional(&state.db).await.map_err(db_error)?;
        let price_id = match cached {
            Some(id) => id,
            None => {
                let id = ghost_stripe::create_price(cfg, &product_id, currency, amount, interval).await?;
                sqlx::query("INSERT INTO stripe_prices (id, stripe_price_id, stripe_product_id, active, nickname, amount, currency, interval, type, created_at, updated_at) VALUES (?, ?, ?, 1, ?, ?, ?, ?, 'recurring', ?, ?)")
                    .bind(ghost_core::ids::new_id()).bind(&id).bind(&product_id).bind(interval)
                    .bind(amount).bind(currency).bind(interval).bind(&now).bind(&now)
                    .execute(&state.db).await.map_err(db_error)?;
                id
            }
        };
        // column is selected from the fixed list above, never from a request.
        sqlx::query(&format!("UPDATE products SET {column} = ? WHERE id = ?"))
            .bind(price_id).bind(tier_id).execute(&state.db).await.map_err(db_error)?;
    }
    Ok(())
}

pub async fn sync_active_tiers(state: &AppState, cfg: &StripeConfig) -> Result<()> {
    let tiers: Vec<(String, String, Option<String>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, name, currency, monthly_price, yearly_price FROM products WHERE type = 'paid' AND active = 1")
        .fetch_all(&state.db).await.map_err(db_error)?;
    for (id, name, currency, monthly, yearly) in tiers {
        sync_tier(state, cfg, &id, &name, currency.as_deref().unwrap_or("usd"), monthly, yearly).await?;
    }
    Ok(())
}
