//! Recommended publications, in the shape every reader of them expects.
//!
//! Four callers used to query this table with four different column lists —
//! the theme helper, the Content API, the Admin API and the settings screen —
//! and only one of them ever selected the display metadata. They all come
//! through here now, so a recommendation renders the same everywhere.
use ghost_core::Result;
use serde_json::{json, Value};
use sqlx::SqlitePool;

/// Ghost's default: newest first. `featured` leads within that, which is the
/// port's own column and the order the settings list is kept in.
const DEFAULT_ORDER: &str = "featured DESC, created_at DESC";

/// The browse options `{{recommendations}}` and the Content API both accept.
#[derive(Debug, Clone)]
pub struct BrowseRecommendations {
    pub filter: Option<ghost_nql::Expr>,
    pub order: Option<String>,
    pub limit: i64,
    pub page: i64,
}

impl Default for BrowseRecommendations {
    /// The helper's documented defaults: five records, newest first, page one.
    fn default() -> Self {
        Self { filter: None, order: None, limit: 5, page: 1 }
    }
}

type Row = (
    String,         // id
    String,         // url
    String,         // title
    Option<String>, // description
    Option<String>, // excerpt
    Option<String>, // featured_image
    Option<String>, // favicon
    bool,           // one_click_subscribe
    bool,           // featured
    Option<String>, // created_at
    Option<String>, // updated_at
);

fn to_json((id, url, title, description, excerpt, featured_image, favicon, one_click, featured, created_at, updated_at): Row) -> Value {
    json!({
        "id": id,
        "url": url,
        "title": title,
        "description": description,
        "excerpt": excerpt,
        "featured_image": featured_image,
        "favicon": favicon,
        "one_click_subscribe": one_click,
        "featured": featured,
        "created_at": created_at,
        "updated_at": updated_at,
    })
}

const COLS: &str = "id, url, title, description, excerpt, featured_image, favicon, \
                    one_click_subscribe, featured, created_at, updated_at";

/// Browse recommendations with the helper/Content API option set applied.
///
/// Returns the page plus the total number of matching records, so a caller can
/// build real pagination metadata instead of an empty meta object.
pub async fn browse(pool: &SqlitePool, q: &BrowseRecommendations) -> Result<(Vec<Value>, i64)> {
    let rows = sqlx::query_as::<_, Row>(&format!(
        "SELECT {COLS} FROM recommendations ORDER BY {}",
        order_clause(q.order.as_deref()),
    ))
    .fetch_all(pool)
    .await
    .map_err(super::dberr)?;
    let mut items: Vec<Value> = rows.into_iter().map(to_json).collect();

    // Filtering happens on the projection so a filter can name the API's
    // fields; the table is small enough that this costs nothing.
    if let Some(f) = &q.filter {
        items.retain(|i| ghost_nql::evaluate(f, i));
    }
    let total = items.len() as i64;

    let items = if q.limit < 0 {
        items
    } else {
        let limit = q.limit.max(1) as usize;
        let start = (q.page.max(1) as usize - 1).saturating_mul(limit);
        items.into_iter().skip(start).take(limit).collect()
    };
    Ok((items, total))
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<Value>> {
    let row = sqlx::query_as::<_, Row>(&format!("SELECT {COLS} FROM recommendations WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(super::dberr)?;
    Ok(row.map(to_json))
}

/// Only known columns reach the ORDER BY, so an `order` a theme supplies
/// cannot become SQL.
fn order_clause(order: Option<&str>) -> String {
    let Some(order) = order.map(str::trim).filter(|o| !o.is_empty()) else {
        return DEFAULT_ORDER.to_string();
    };
    const ALLOWED: &[&str] =
        &["id", "url", "title", "description", "excerpt", "featured", "one_click_subscribe", "created_at", "updated_at"];
    let mut parts = Vec::new();
    for term in order.split(',') {
        let mut it = term.trim().split_whitespace();
        let Some(field) = it.next() else { continue };
        if !ALLOWED.contains(&field) {
            continue;
        }
        let dir = match it.next().map(|d| d.to_ascii_lowercase()) {
            Some(d) if d == "desc" => "DESC",
            _ => "ASC",
        };
        parts.push(format!("{field} {dir}"));
    }
    if parts.is_empty() { DEFAULT_ORDER.to_string() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_falls_back_rather_than_interpolating_unknown_input() {
        assert_eq!(order_clause(None), DEFAULT_ORDER);
        assert_eq!(order_clause(Some("")), DEFAULT_ORDER);
        // a theme's typo, and an injection attempt, both land on the default
        assert_eq!(order_clause(Some("nonsense desc")), DEFAULT_ORDER);
        assert_eq!(order_clause(Some("title; DROP TABLE recommendations")), DEFAULT_ORDER);
    }

    #[test]
    fn order_accepts_the_documented_fields() {
        assert_eq!(order_clause(Some("title asc")), "title ASC");
        assert_eq!(order_clause(Some("created_at desc")), "created_at DESC");
        // Ghost accepts a comma-separated list; a bare field defaults to ASC
        assert_eq!(order_clause(Some("featured desc, title")), "featured DESC, title ASC");
    }
}
