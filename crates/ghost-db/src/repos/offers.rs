//! Offers: the discount codes Settings → Growth → Offers hands out.
//!
//! Ghost's shape, kept whole: an offer discounts one tier at one cadence, by a
//! percentage, a flat amount, or a free trial. The Stripe coupon that actually
//! applies the discount is minted alongside and its id stored here.
use ghost_core::{dates, ids, GhostError, Result};
use sqlx::SqlitePool;

use super::dberr;

/// `active` in Ghost's API is a `status` of `active` / `archived`. Archived
/// offers keep working for anyone already subscribed; they just stop being
/// redeemable.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Offer {
    pub id: String,
    pub active: bool,
    pub name: String,
    pub code: String,
    pub product_id: String,
    pub stripe_coupon_id: Option<String>,
    /// Ghost's `cadence`: `month` or `year`.
    pub interval: Option<String>,
    pub currency: Option<String>,
    /// Ghost's `type`: `percent`, `fixed` or `trial`.
    pub discount_type: Option<String>,
    pub discount_amount: Option<i64>,
    /// `once`, `repeating`, `forever` or `trial`.
    pub duration: Option<String>,
    pub duration_in_months: Option<i64>,
    pub portal_title: Option<String>,
    pub portal_description: Option<String>,
    pub redemption_type: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl Offer {
    /// How the discount reads on a row, e.g. "20% off forever".
    pub fn summary(&self) -> String {
        let amount = self.discount_amount.unwrap_or(0);
        match self.discount_type.as_deref() {
            Some("trial") => {
                format!("{amount} day{} free", if amount == 1 { "" } else { "s" })
            }
            Some("fixed") => {
                let currency = self.currency.clone().unwrap_or_default().to_uppercase();
                format!("{} {:.2} off {}", currency, amount as f64 / 100.0, self.duration_label())
            }
            _ => format!("{amount}% off {}", self.duration_label()),
        }
    }

    fn duration_label(&self) -> String {
        match self.duration.as_deref() {
            Some("once") => "once".into(),
            Some("repeating") => {
                let m = self.duration_in_months.unwrap_or(1);
                format!("for {m} month{}", if m == 1 { "" } else { "s" })
            }
            _ => "forever".into(),
        }
    }

    pub fn status(&self) -> &'static str {
        if self.active {
            "active"
        } else {
            "archived"
        }
    }
}

const COLS: &str = "id, active, name, code, product_id, stripe_coupon_id, interval, currency, \
     discount_type, discount_amount, duration, duration_in_months, portal_title, \
     portal_description, redemption_type, created_at, updated_at";

pub async fn list(pool: &SqlitePool) -> Result<Vec<Offer>> {
    let rows = sqlx::query_as::<_, Offer>(&format!(
        "SELECT {COLS} FROM offers ORDER BY active DESC, created_at DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(dberr)?;
    Ok(rows)
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Offer>> {
    let row = sqlx::query_as::<_, Offer>(&format!("SELECT {COLS} FROM offers WHERE id = ? LIMIT 1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(dberr)?;
    Ok(row)
}

/// The lookup the portal does when a visitor arrives on an offer URL. Only
/// active offers resolve — an archived code must stop working.
pub async fn find_redeemable_by_code(pool: &SqlitePool, code: &str) -> Result<Option<Offer>> {
    let row = sqlx::query_as::<_, Offer>(&format!(
        "SELECT {COLS} FROM offers WHERE code = ? AND active = 1 LIMIT 1"
    ))
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(dberr)?;
    Ok(row)
}

/// How many members arrived through this offer.
pub async fn redemption_count(pool: &SqlitePool, offer_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE offer_id = ?")
        .bind(offer_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// The editable fields. `name` and `code` are unique across all offers, which
/// the caller checks first so the clash can be explained rather than surfacing
/// as a constraint error.
#[derive(Debug, Default, Clone)]
pub struct OfferInput {
    pub name: String,
    pub code: String,
    pub product_id: String,
    pub interval: String,
    pub discount_type: String,
    pub discount_amount: i64,
    pub duration: String,
    pub duration_in_months: Option<i64>,
    pub currency: Option<String>,
    pub portal_title: String,
    pub portal_description: String,
    pub stripe_coupon_id: Option<String>,
    pub active: bool,
}

/// Whether `name` or `code` is already taken by a *different* offer.
pub async fn conflict(
    pool: &SqlitePool,
    name: &str,
    code: &str,
    self_id: Option<&str>,
) -> Result<Option<&'static str>> {
    let id = self_id.unwrap_or("");
    let name_taken: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM offers WHERE name = ? AND id != ?")
            .bind(name)
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(dberr)?;
    if name_taken > 0 {
        return Ok(Some("name"));
    }
    let code_taken: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM offers WHERE code = ? AND id != ?")
            .bind(code)
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(dberr)?;
    if code_taken > 0 {
        return Ok(Some("code"));
    }
    Ok(None)
}

pub async fn create(pool: &SqlitePool, input: &OfferInput) -> Result<Offer> {
    let id = ids::new_id();
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO offers (id, active, name, code, product_id, stripe_coupon_id, interval, \
         currency, discount_type, discount_amount, duration, duration_in_months, portal_title, \
         portal_description, redemption_type, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'signup', ?, ?)",
    )
    .bind(&id)
    .bind(input.active)
    .bind(&input.name)
    .bind(&input.code)
    .bind(&input.product_id)
    .bind(&input.stripe_coupon_id)
    .bind(&input.interval)
    .bind(&input.currency)
    .bind(&input.discount_type)
    .bind(input.discount_amount)
    .bind(&input.duration)
    .bind(input.duration_in_months)
    .bind(&input.portal_title)
    .bind(&input.portal_description)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(dberr)?;
    find_by_id(pool, &id).await?.ok_or_else(|| GhostError::Internal("offer vanished".into()))
}

/// Update an offer.
///
/// Ghost only lets the *presentation* of a live offer change — name, code and
/// the portal copy — because the discount itself is a Stripe coupon that
/// existing subscriptions are already attached to. `coupon` is therefore
/// written only when a fresh one was minted.
pub async fn update(pool: &SqlitePool, id: &str, input: &OfferInput) -> Result<Offer> {
    let now = dates::now_db();
    sqlx::query(
        "UPDATE offers SET name = ?, code = ?, portal_title = ?, portal_description = ?, \
         active = ?, stripe_coupon_id = COALESCE(?, stripe_coupon_id), updated_at = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.code)
    .bind(&input.portal_title)
    .bind(&input.portal_description)
    .bind(input.active)
    .bind(&input.stripe_coupon_id)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(dberr)?;
    find_by_id(pool, id).await?.ok_or_else(|| GhostError::Internal("offer vanished".into()))
}

pub async fn set_active(pool: &SqlitePool, id: &str, active: bool) -> Result<()> {
    sqlx::query("UPDATE offers SET active = ?, updated_at = ? WHERE id = ?")
        .bind(active)
        .bind(dates::now_db())
        .bind(id)
        .execute(pool)
        .await
        .map_err(dberr)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(discount_type: &str, amount: i64, duration: &str, months: Option<i64>) -> Offer {
        Offer {
            id: "1".into(),
            active: true,
            name: "Test".into(),
            code: "test".into(),
            product_id: "p".into(),
            stripe_coupon_id: None,
            interval: Some("month".into()),
            currency: Some("usd".into()),
            discount_type: Some(discount_type.into()),
            discount_amount: Some(amount),
            duration: Some(duration.into()),
            duration_in_months: months,
            portal_title: None,
            portal_description: None,
            redemption_type: Some("signup".into()),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn summaries_read_the_way_ghost_describes_an_offer() {
        assert_eq!(offer("percent", 20, "forever", None).summary(), "20% off forever");
        assert_eq!(offer("percent", 50, "once", None).summary(), "50% off once");
        assert_eq!(
            offer("percent", 10, "repeating", Some(3)).summary(),
            "10% off for 3 months"
        );
        // a single month is not "1 months"
        assert_eq!(offer("percent", 10, "repeating", Some(1)).summary(), "10% off for 1 month");
        assert_eq!(offer("fixed", 500, "once", None).summary(), "USD 5.00 off once");
        assert_eq!(offer("trial", 14, "trial", None).summary(), "14 days free");
        assert_eq!(offer("trial", 1, "trial", None).summary(), "1 day free");
    }

    #[test]
    fn status_maps_to_ghosts_two_values() {
        let mut o = offer("percent", 20, "forever", None);
        assert_eq!(o.status(), "active");
        o.active = false;
        assert_eq!(o.status(), "archived");
    }
}
