//! Membership tier visibility shared by the portal settings and tier editor.
use ghost_core::Result;
use crate::Db;

/// Only current grants count. Expired gifts/complimentary grants must not
/// unlock a tier even if the cached member status has not been swept yet.
pub async fn member_tier_ids(pool: &Db, member_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar(crate::dialect::sql("SELECT DISTINCT product_id FROM members_products WHERE member_id = ? AND (expiry_at IS NULL OR datetime(expiry_at) > datetime('now'))"))
        .bind(member_id).fetch_all(pool).await.map_err(super::dberr)
}

pub async fn post_tier_ids(pool: &Db, post_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT product_id FROM posts_products WHERE post_id = ? ORDER BY sort_order")
        .bind(post_id).fetch_all(pool).await.map_err(super::dberr)
}

pub async fn validate_post_tiers(pool: &Db, visibility: &str, ids: &[String]) -> Result<()> {
    let invalid = |message: &str| ghost_core::GhostError::ValidationError { message: message.into(), context: Some("tiers".into()) };
    if !matches!(visibility, "public" | "members" | "paid" | "tiers") { return Err(invalid("Invalid post visibility.")); }
    for id in ids {
        let found: bool = sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM products WHERE id = ? AND type = 'paid')")
            .bind(id).fetch_one(pool).await.map_err(super::dberr)? != 0;
        if !found { return Err(invalid("A selected paid tier does not exist.")); }
    }
    Ok(())
}

pub async fn set_free_signup_visibility(pool: &Db, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE products SET visibility = ?, active = CASE WHEN ? THEN 1 ELSE active END, updated_at = ? WHERE type = 'free'")
        .bind(if enabled { "public" } else { "none" }).bind(enabled)
        .bind(ghost_core::dates::now_db()).execute(pool).await.map_err(super::dberr)?;
    Ok(())
}

/// Every active tier in the Content API's `tiers` shape, as one browse.
///
/// Shared so `{{#get "tiers"}}` and `GET /content/tiers` cannot drift: the
/// helper and the endpoint were separately hand-rolling this projection, and
/// only the endpoint had it. Free tiers carry no price, so `currency` and the
/// two price fields are dropped for them rather than sent as null — Portal
/// reads a present `monthly_price` as "this plan is purchasable".
pub async fn browse_public(
    pool: &Db,
    filter: Option<&ghost_nql::Expr>,
    order: Option<&str>,
    limit: i64,
    page: i64,
) -> Result<(Vec<serde_json::Value>, i64)> {
    use serde_json::json;

    type Row = (
        String,         // id
        String,         // name
        String,         // slug
        Option<String>, // description
        String,         // type
        String,         // visibility
        Option<String>, // currency
        Option<i64>,    // monthly_price
        Option<i64>,    // yearly_price
        i64,            // trial_days
        Option<String>, // welcome_page_url
        Option<String>, // created_at
        Option<String>, // updated_at
    );
    let rows = sqlx::query_as::<_, Row>(&format!(
        "SELECT id, name, slug, description, type, visibility, currency, monthly_price, \
         yearly_price, trial_days, welcome_page_url, created_at, updated_at \
         FROM products WHERE active = 1 ORDER BY {}",
        tier_order_clause(order),
    ))
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;

    let mut items: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
    for r in rows {
        let benefits: Vec<String> = sqlx::query_scalar(
            "SELECT b.name FROM products_benefits pb JOIN benefits b ON b.id = pb.benefit_id \
             WHERE pb.product_id = ? ORDER BY pb.sort_order",
        )
        .bind(&r.0)
        .fetch_all(pool)
        .await
        .map_err(super::dberr)?;
        let mut tier = json!({
            "id": r.0,
            "name": r.1,
            "slug": r.2,
            "description": r.3,
            "active": true,
            "type": r.4,
            "visibility": r.5,
            "trial_days": r.9,
            "welcome_page_url": r.10,
            "created_at": r.11,
            "updated_at": r.12,
            "benefits": benefits,
        });
        if r.4 != "free" {
            if let serde_json::Value::Object(m) = &mut tier {
                m.insert("currency".into(), json!(r.6));
                m.insert("monthly_price".into(), json!(r.7));
                m.insert("yearly_price".into(), json!(r.8));
            }
        }
        items.push(tier);
    }

    // Filtering after projection, so a filter can name `type` or `visibility`
    // as the Content API documents them rather than the column names.
    if let Some(f) = filter {
        items.retain(|i| ghost_nql::evaluate(f, i));
    }
    let total = items.len() as i64;
    Ok((paginate(items, limit, page), total))
}

/// `type desc, created_at` is the default: paid tiers before the free one, so
/// Portal's plan list leads with what the publisher sells.
fn tier_order_clause(order: Option<&str>) -> String {
    sort_clause(order, &["name", "slug", "type", "visibility", "created_at", "updated_at", "monthly_price", "yearly_price"], "type DESC, created_at")
}

/// Active newsletters in the Content API's `newsletters` shape.
pub async fn browse_newsletters_public(
    pool: &Db,
    filter: Option<&ghost_nql::Expr>,
    order: Option<&str>,
    limit: i64,
    page: i64,
) -> Result<(Vec<serde_json::Value>, i64)> {
    use serde_json::json;

    type Row = (
        String,         // id
        Option<String>, // uuid
        String,         // name
        Option<String>, // description
        String,         // slug
        Option<String>, // sender_name
        Option<String>, // sender_reply_to
        String,         // visibility
        i64,            // subscribe_on_signup (0/1; `Any` gives no bool)
        i64,            // sort_order
    );
    let rows = sqlx::query_as::<_, Row>(&format!(
        "SELECT id, uuid, name, description, slug, sender_name, sender_reply_to, visibility, \
         subscribe_on_signup, sort_order FROM newsletters WHERE status = 'active' ORDER BY {}",
        sort_clause(order, &["name", "slug", "sort_order", "visibility", "created_at", "updated_at"], "sort_order, created_at"),
    ))
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;
    let mut items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0,
                "uuid": r.1,
                "name": r.2,
                "description": r.3,
                "slug": r.4,
                "sender_name": r.5,
                "sender_reply_to": r.6,
                "status": "active",
                "visibility": r.7,
                "subscribe_on_signup": r.8,
                "sort_order": r.9,
            })
        })
        .collect();
    if let Some(f) = filter {
        items.retain(|i| ghost_nql::evaluate(f, i));
    }
    let total = items.len() as i64;
    Ok((paginate(items, limit, page), total))
}

/// Translate an `order` string into SQL, accepting only known columns.
///
/// An unrecognised field falls back to the default rather than reaching the
/// query, so `order="; DROP TABLE"` from a theme is inert.
fn sort_clause(order: Option<&str>, allowed: &[&str], default: &str) -> String {
    let Some(order) = order.map(str::trim).filter(|o| !o.is_empty()) else {
        return default.to_string();
    };
    let mut parts = Vec::new();
    for term in order.split(',') {
        let mut it = term.trim().split_whitespace();
        let Some(field) = it.next() else { continue };
        if !allowed.contains(&field) {
            continue;
        }
        let dir = match it.next().map(|d| d.to_ascii_lowercase()) {
            Some(d) if d == "desc" => "DESC",
            _ => "ASC",
        };
        parts.push(format!("{field} {dir}"));
    }
    if parts.is_empty() { default.to_string() } else { parts.join(", ") }
}

/// `limit` of -1 means every row; otherwise take the requested page.
fn paginate(items: Vec<serde_json::Value>, limit: i64, page: i64) -> Vec<serde_json::Value> {
    if limit < 0 {
        return items;
    }
    let limit = limit.max(1) as usize;
    let start = (page.max(1) as usize - 1).saturating_mul(limit);
    items.into_iter().skip(start).take(limit).collect()
}
