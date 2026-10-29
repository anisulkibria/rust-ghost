//! Ghost's `stats` family: the numbers behind the Analytics screens.
//!
//! Ghost serves these from Tinybird, a column store it ships events to, and
//! falls back to its own database for the member and revenue figures. There
//! is no Tinybird here, so every one of these is a query over this port's own
//! tables — `page_views` for traffic, the member event tables for growth,
//! `email_recipients` and `newsletter_clicks` for what a newsletter did.
//!
//! Two conventions throughout:
//!
//! * A date range is `date_from`/`date_to`, or `days` back from today.
//!   Ghost's admin sends both shapes.
//! * A count of members attributed to a post comes from the `attribution_id`
//!   on a signup or subscription event, never from "who was reading it at the
//!   time" — the latter is a correlation and would quietly flatter whichever
//!   post happened to be popular.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn admin_router() -> Router<AppState> {
    let mut router = Router::new();
    for suffix in ["", "/"] {
        router = router
            .route(&format!("/stats/member_count{suffix}"), get(member_count))
            .route(&format!("/stats/mrr{suffix}"), get(mrr))
            .route(&format!("/stats/subscriber-count{suffix}"), get(subscriber_count))
            .route(&format!("/stats/subscriptions{suffix}"), get(subscriptions))
            .route(&format!("/stats/top-posts{suffix}"), get(top_posts))
            .route(&format!("/stats/top-posts-views{suffix}"), get(top_posts_views))
            .route(&format!("/stats/top-content{suffix}"), get(top_content))
            .route(&format!("/stats/referrers{suffix}"), get(referrers))
            .route(&format!("/stats/top-sources-growth{suffix}"), get(top_sources_growth))
            .route(&format!("/stats/newsletter-stats{suffix}"), get(newsletter_stats))
            .route(&format!("/stats/newsletter-basic-stats{suffix}"), get(newsletter_basic_stats))
            .route(&format!("/stats/newsletter-click-stats{suffix}"), get(newsletter_click_stats))
            .route(&format!("/stats/locations{suffix}"), get(locations))
            .route(&format!("/stats/posts/{{id}}/stats{suffix}"), get(post_stats))
            .route(&format!("/stats/posts/{{id}}/growth{suffix}"), get(post_growth))
            .route(&format!("/stats/posts/{{id}}/top-referrers{suffix}"), get(post_top_referrers))
            .route(&format!("/stats/posts-member-counts{suffix}"), post(posts_member_counts))
            .route(&format!("/stats/posts-visitor-counts{suffix}"), post(posts_visitor_counts))
            // Ghost's older member-scoped aliases, which its admin still calls.
            .route(&format!("/members/stats/count{suffix}"), get(member_count))
            .route(&format!("/members/stats/mrr{suffix}"), get(mrr));
    }
    router
}

// ---------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------

/// The window a request asked for, as `YYYY-MM-DD` bounds.
struct Range {
    from: String,
    to: String,
}

impl Range {
    fn parse(q: &HashMap<String, String>) -> Self {
        let today = chrono::Utc::now().date_naive();
        let to = q
            .get("date_to")
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or(today);
        let from = q
            .get("date_from")
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or_else(|| {
                // `days` is Ghost's other shape. Bounded at five years: these
                // are grouped-by-day queries and an unbounded range is a way
                // to ask the server to build a very long list.
                let days = q.get("days").and_then(|d| d.parse::<i64>().ok()).unwrap_or(30).clamp(1, 1826);
                to - chrono::Duration::days(days - 1)
            });
        Range { from: from.to_string(), to: to.to_string() }
    }
}

/// The paths a post is reachable at, for matching against `page_views`.
///
/// Built through the router rather than assumed to be `/{slug}/`, so a site
/// with a dated or prefixed permalink structure still reports on its posts.
/// Both the slashed and unslashed forms are returned because either can be
/// what a visitor's browser actually requested.
async fn post_paths(state: &AppState, post_id: &str) -> Vec<String> {
    let Ok(Some(post)) = ghost_db::repos::posts::find_by_id(&state.db, post_id).await else {
        return Vec::new();
    };
    let router = state.router.read().await.clone();
    let json = ghost_core::models::post_to_json(&post, &[], &[], "");
    let path = match router.permalink_for(&json) {
        Some((_, permalink)) => ghost_routing::tree::build_permalink(&permalink, &json),
        None => format!("/{}/", post.slug.clone().unwrap_or_default()),
    };
    let bare = path.trim_end_matches('/').to_string();
    if bare.is_empty() {
        vec!["/".to_string()]
    } else {
        vec![format!("{bare}/"), bare]
    }
}

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    permissions::require(state, headers, Role::Editor).await.map(|_| ())
}

/// These are all private to the signed-in staff member who asked for them.
fn private(body: Value) -> Response {
    ([("cache-control", "private, no-store")], Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// members and revenue
// ---------------------------------------------------------------------------

/// Members by day: how many joined, how many left, and the running total.
///
/// Built from the status timeline rather than from `members.created_at`, so a
/// member who upgraded shows as a paid member from the day they upgraded and
/// not from the day they signed up.
async fn member_count(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT substr(created_at,1,10) AS day, to_status, COUNT(*) \
         FROM members_status_events \
         WHERE substr(created_at,1,10) BETWEEN ? AND ? \
         GROUP BY day, to_status ORDER BY day",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // The totals as they stand now, which is what the tiles above the chart
    // show; the series beside them is the movement within the window.
    let totals = sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM members GROUP BY status")
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let mut by_status: HashMap<String, i64> = HashMap::new();
    for (status, count) in totals {
        by_status.insert(status, count);
    }

    let mut days: Vec<(String, HashMap<String, i64>)> = Vec::new();
    for (day, status, count) in rows {
        match days.last_mut() {
            Some((last, counts)) if *last == day => {
                *counts.entry(status).or_insert(0) += count;
            }
            _ => {
                days.push((day, HashMap::from([(status, count)])));
            }
        }
    }
    let stats: Vec<Value> = days
        .into_iter()
        .map(|(date, counts)| {
            let take = |key: &str| counts.get(key).copied().unwrap_or(0);
            json!({"date": date, "free": take("free"), "paid": take("paid"),
                   "comped": take("comped") + take("gift")})
        })
        .collect();
    private(json!({"stats": stats, "meta": {"totals": {
        "free": by_status.get("free").copied().unwrap_or(0),
        "paid": by_status.get("paid").copied().unwrap_or(0),
        "comped": by_status.get("comped").copied().unwrap_or(0) + by_status.get("gift").copied().unwrap_or(0),
    }}}))
}

/// Monthly recurring revenue by day, as a running total.
///
/// Each paid subscription event carries what it did to MRR, so the series is
/// a cumulative sum of deltas. That is the only way the number stays right
/// for a subscription that has since been cancelled: recomputing from today's
/// live subscriptions would show history that never happened.
async fn mrr(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    // Everything before the window is the opening balance.
    let opening: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(mrr_delta),0) FROM members_paid_subscription_events WHERE substr(created_at,1,10) < ?",
    )
    .bind(&range.from)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT substr(created_at,1,10) AS day, COALESCE(SUM(mrr_delta),0) \
         FROM members_paid_subscription_events \
         WHERE substr(created_at,1,10) BETWEEN ? AND ? GROUP BY day ORDER BY day",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let currency = sqlx::query_scalar::<_, String>(
        "SELECT currency FROM members_paid_subscription_events WHERE currency IS NOT NULL ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| state.settings.get_str("stripe_default_currency").unwrap_or_else(|| "usd".into()));

    let mut running = opening;
    let stats: Vec<Value> = rows
        .into_iter()
        .map(|(date, delta)| {
            running += delta;
            json!({"date": date, "mrr": running.max(0), "currency": currency})
        })
        .collect();
    private(json!({"stats": stats, "meta": {"totals": [{"currency": currency, "mrr": running.max(0)}]}}))
}

/// Newsletter subscribers by day, and the current total.
async fn subscriber_count(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT substr(created_at,1,10) AS day, \
                COALESCE(SUM(subscribed),0), COALESCE(SUM(NOT subscribed),0) \
         FROM members_subscribe_events \
         WHERE substr(created_at,1,10) BETWEEN ? AND ? GROUP BY day ORDER BY day",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let total: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT member_id) FROM members_newsletters")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    private(json!({"stats": rows.into_iter().map(|(date, subscribed, unsubscribed)|
        json!({"date": date, "subscribed": subscribed, "unsubscribed": unsubscribed,
               "net": subscribed - unsubscribed})).collect::<Vec<_>>(),
        "meta": {"totals": {"subscribers": total}}}))
}

/// Paid subscriptions by tier and billing period.
async fn subscriptions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let rows = sqlx::query_as::<_, (Option<String>, Option<String>, String, i64)>(
        "SELECT p.name, s.plan_interval, s.status, COUNT(*) \
         FROM subscriptions s \
         LEFT JOIN products p ON p.monthly_price_id = s.stripe_price_id OR p.yearly_price_id = s.stripe_price_id \
         GROUP BY p.name, s.plan_interval, s.status ORDER BY COUNT(*) DESC",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(tier, cadence, status, count)|
        json!({"tier": tier, "cadence": cadence, "status": status, "count": count})).collect::<Vec<_>>()}))
}

// ---------------------------------------------------------------------------
// content
// ---------------------------------------------------------------------------

/// Posts ranked by the members they brought in.
///
/// This is the question the Analytics screen exists to answer, and it is only
/// answerable because a signup records which post it was attributed to.
async fn top_posts(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, i64, i64)>(
        "SELECT p.id, p.title, p.slug, \
                (SELECT COUNT(*) FROM members_created_events c \
                  WHERE c.attribution_id = p.id AND c.attribution_type = 'post' \
                    AND substr(c.created_at,1,10) BETWEEN ? AND ?), \
                (SELECT COUNT(*) FROM members_subscription_created_events s \
                  WHERE s.attribution_id = p.id AND s.attribution_type = 'post' \
                    AND substr(s.created_at,1,10) BETWEEN ? AND ?), \
                (SELECT COALESCE(SUM(e.mrr_delta),0) FROM members_subscription_created_events s \
                  JOIN members_paid_subscription_events e ON e.subscription_id = s.subscription_id \
                  WHERE s.attribution_id = p.id AND s.attribution_type = 'post' \
                    AND substr(s.created_at,1,10) BETWEEN ? AND ?) \
         FROM posts p WHERE p.status = 'published' AND p.type = 'post' \
         ORDER BY 4 + 5 DESC, p.published_at DESC LIMIT 20",
    )
    .bind(&range.from).bind(&range.to)
    .bind(&range.from).bind(&range.to)
    .bind(&range.from).bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(id, title, slug, free, paid, mrr)|
        json!({"post_id": id, "title": title, "slug": slug, "free_members": free,
               "paid_members": paid, "mrr": mrr})).collect::<Vec<_>>()}))
}

/// Posts ranked by how much they were read.
async fn top_posts_views(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    // Joined on the slug inside the path rather than on a rebuilt permalink,
    // because this ranks every post at once and rebuilding each one's URL
    // would mean a router pass per row.
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, i64)>(
        "SELECT p.id, p.title, p.slug, COUNT(v.id), COUNT(DISTINCT v.viewer) \
         FROM posts p JOIN page_views v ON v.path LIKE '%/' || p.slug || '/' OR v.path LIKE '%/' || p.slug \
         WHERE p.status = 'published' AND p.type = 'post' \
           AND substr(v.created_at,1,10) BETWEEN ? AND ? \
         GROUP BY p.id ORDER BY COUNT(v.id) DESC LIMIT 20",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(id, title, slug, views, visitors)|
        json!({"post_id": id, "title": title, "slug": slug, "views": views, "visitors": visitors})).collect::<Vec<_>>()}))
}

/// Any path on the site, ranked by views — posts, pages, tag archives alike.
async fn top_content(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT path, COUNT(*), COUNT(DISTINCT viewer) FROM page_views \
         WHERE substr(created_at,1,10) BETWEEN ? AND ? \
         GROUP BY path ORDER BY COUNT(*) DESC LIMIT 25",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(path, views, visitors)|
        json!({"path": path, "views": views, "visitors": visitors})).collect::<Vec<_>>()}))
}

/// Where the traffic came from.
async fn referrers(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (Option<String>, Option<String>, i64, i64)>(
        "SELECT COALESCE(referrer_source, referrer) AS source, referrer_medium, COUNT(*), COUNT(DISTINCT viewer) \
         FROM page_views WHERE substr(created_at,1,10) BETWEEN ? AND ? \
         GROUP BY source, referrer_medium ORDER BY COUNT(*) DESC LIMIT 25",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(source, medium, views, visitors)|
        json!({"source": source.unwrap_or_else(|| "Direct".into()), "medium": medium,
               "views": views, "visitors": visitors})).collect::<Vec<_>>()}))
}

/// Which sources are actually producing members, as against traffic.
async fn top_sources_growth(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (Option<String>, i64, i64)>(
        "SELECT COALESCE(c.referrer_source, c.utm_source) AS source, COUNT(*), \
                (SELECT COUNT(*) FROM members_subscription_created_events s \
                  WHERE COALESCE(s.referrer_source, s.utm_source) IS COALESCE(c.referrer_source, c.utm_source) \
                    AND substr(s.created_at,1,10) BETWEEN ? AND ?) \
         FROM members_created_events c \
         WHERE substr(c.created_at,1,10) BETWEEN ? AND ? \
         GROUP BY source ORDER BY COUNT(*) DESC LIMIT 25",
    )
    .bind(&range.from).bind(&range.to)
    .bind(&range.from).bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(source, free, paid)|
        json!({"source": source.unwrap_or_else(|| "Direct".into()), "free_members": free,
               "paid_members": paid})).collect::<Vec<_>>()}))
}

/// Where in the world the readers are. Only as complete as the proxy in
/// front of the site: a view with no country header is counted as unknown
/// rather than guessed at.
async fn locations(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (Option<String>, i64, i64)>(
        "SELECT country, COUNT(*), COUNT(DISTINCT viewer) FROM page_views \
         WHERE substr(created_at,1,10) BETWEEN ? AND ? \
         GROUP BY country ORDER BY COUNT(DISTINCT viewer) DESC LIMIT 50",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(country, views, visitors)|
        json!({"country": country, "views": views, "visitors": visitors})).collect::<Vec<_>>()}))
}

// ---------------------------------------------------------------------------
// one post
// ---------------------------------------------------------------------------

/// Everything known about how one post did: on the web, in the inbox, and
/// in members gained.
async fn post_stats(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let Some(stats) = post_stats_value(&state, &id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    private(json!({"stats": [stats]}))
}

/// Shared with the admin's own post analytics page, so the page and the API
/// cannot disagree about what a post did.
pub async fn post_stats_value(state: &AppState, post_id: &str) -> Option<Value> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE id = ?)")
        .bind(post_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
    if !exists {
        return None;
    }
    let paths = post_paths(state, post_id).await;
    let (mut views, mut visitors) = (0i64, 0i64);
    for path in &paths {
        let row: (i64, i64) =
            sqlx::query_as("SELECT COUNT(*), COUNT(DISTINCT viewer) FROM page_views WHERE path = ?")
                .bind(path)
                .fetch_one(&state.db)
                .await
                .unwrap_or((0, 0));
        views += row.0;
        visitors += row.1;
    }

    // The email side, from the per-recipient records that sending writes.
    let email: Option<(String, String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, status, email_count, delivered_count, opened_count, failed_count \
         FROM emails WHERE post_id = ? LIMIT 1",
    )
    .bind(post_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let clicked: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT k.member_uuid) FROM newsletter_clicks k \
         JOIN newsletter_links l ON l.id = k.link_id WHERE l.post_id = ?",
    )
    .bind(post_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let free: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM members_created_events WHERE attribution_id = ? AND attribution_type = 'post'",
    )
    .bind(post_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    let paid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM members_subscription_created_events WHERE attribution_id = ? AND attribution_type = 'post'",
    )
    .bind(post_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let email_json = email.map(|(id, status, sent, delivered, opened, failed)| {
        // A rate over nothing is not zero, it is unknown — so it is null
        // until something has been delivered.
        let rate = |part: i64, whole: i64| (whole > 0).then(|| (part as f64 * 100.0 / whole as f64).round() as i64);
        json!({"id": id, "status": status, "sent": sent, "delivered": delivered,
               "opened": opened, "failed": failed, "clicked": clicked,
               "open_rate": rate(opened, delivered), "click_rate": rate(clicked, delivered)})
    });
    Some(json!({"post_id": post_id, "views": views, "visitors": visitors,
                "free_members": free, "paid_members": paid, "email": email_json}))
}

/// What this post added to the membership and to revenue.
async fn post_growth(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let free: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM members_created_events WHERE attribution_id = ? AND attribution_type = 'post'",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);
    let (paid, mrr): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(e.mrr_delta),0) \
         FROM members_subscription_created_events s \
         LEFT JOIN members_paid_subscription_events e ON e.subscription_id = s.subscription_id \
         WHERE s.attribution_id = ? AND s.attribution_type = 'post'",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap_or((0, 0));
    private(json!({"stats": [{"post_id": id, "free_members": free, "paid_members": paid, "mrr": mrr}]}))
}

/// The sources that sent readers to this post.
async fn post_top_referrers(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let paths = post_paths(&state, &id).await;
    if paths.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut totals: HashMap<String, (i64, i64)> = HashMap::new();
    for path in &paths {
        let rows = sqlx::query_as::<_, (Option<String>, i64, i64)>(
            "SELECT COALESCE(referrer_source, referrer), COUNT(*), COUNT(DISTINCT viewer) \
             FROM page_views WHERE path = ? GROUP BY 1",
        )
        .bind(path)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        for (source, views, visitors) in rows {
            let entry = totals.entry(source.unwrap_or_else(|| "Direct".into())).or_insert((0, 0));
            entry.0 += views;
            entry.1 += visitors;
        }
    }
    let mut stats: Vec<Value> = totals
        .into_iter()
        .map(|(source, (views, visitors))| json!({"source": source, "views": views, "visitors": visitors}))
        .collect();
    stats.sort_by_key(|s| -s["views"].as_i64().unwrap_or(0));
    private(json!({"stats": stats}))
}

/// `POST` with `{"ids": [...]}`, answering with a count per post. Ghost's
/// admin asks for a whole list's figures in one request rather than one at a
/// time, which is also why these two are POSTs: the list is the body.
async fn posts_member_counts(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let mut counts = serde_json::Map::new();
    for id in requested_ids(&body) {
        let free: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM members_created_events WHERE attribution_id = ? AND attribution_type = 'post'",
        )
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
        let paid: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM members_subscription_created_events WHERE attribution_id = ? AND attribution_type = 'post'",
        )
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
        counts.insert(id, json!({"free": free, "paid": paid, "total": free + paid}));
    }
    private(json!({"stats": [counts]}))
}

async fn posts_visitor_counts(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let mut counts = serde_json::Map::new();
    for id in requested_ids(&body) {
        let mut visitors = 0i64;
        for path in post_paths(&state, &id).await {
            visitors += sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT viewer) FROM page_views WHERE path = ?")
                .bind(&path)
                .fetch_one(&state.db)
                .await
                .unwrap_or(0);
        }
        counts.insert(id, json!(visitors));
    }
    private(json!({"stats": [counts]}))
}

/// Bounded at a hundred: this is one query per id, and the body is whatever
/// the client sent.
fn requested_ids(body: &Value) -> Vec<String> {
    body.get("ids")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).take(100).collect())
        .unwrap_or_default()
}

/// One row per newsletter that went out, with how it was received.
async fn newsletter_stats(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, String, i64, i64, i64, i64)>(
        "SELECT e.id, p.title, e.subject, substr(e.created_at,1,10), \
                e.email_count, e.delivered_count, e.opened_count, \
                (SELECT COUNT(DISTINCT k.member_uuid) FROM newsletter_clicks k \
                  JOIN newsletter_links l ON l.id = k.link_id WHERE l.post_id = e.post_id) \
         FROM emails e LEFT JOIN posts p ON p.id = e.post_id \
         WHERE substr(e.created_at,1,10) BETWEEN ? AND ? \
         ORDER BY e.created_at DESC LIMIT 50",
    )
    .bind(&range.from)
    .bind(&range.to)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(id, title, subject, date, sent, delivered, opened, clicked)| {
        let rate = |part: i64, whole: i64| (whole > 0).then(|| (part as f64 * 100.0 / whole as f64).round() as i64);
        json!({"email_id": id, "title": title.or(subject), "date": date, "sent": sent,
               "delivered": delivered, "opened": opened, "clicked": clicked,
               "open_rate": rate(opened, delivered), "click_rate": rate(clicked, delivered)})
    }).collect::<Vec<_>>()}))
}

/// One row per post a newsletter sent, which is what the Newsletters screen
/// sorts to answer "which issue landed".
///
/// Ghost's `newsletter-basic-stats`. Rates come back as fractions rather than
/// percentages, as Ghost's do, because the caller formats them; and they are
/// over `sent_to` rather than over deliveries, so a newsletter is measured
/// against the list it went to.
async fn newsletter_basic_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let range = Range::parse(&q);
    // Ghost validates the order field and refuses anything else rather than
    // silently sorting by something the caller did not ask for — and since
    // this interpolates into SQL, an allowlist is the only safe way to do it.
    let order = q.get("order").map(String::as_str).unwrap_or("date desc");
    let (field, direction) = order.split_once(' ').unwrap_or((order, "desc"));
    let column = match field {
        "date" => "p.published_at",
        "open_rate" => "open_rate",
        "sent_to" => "sent_to",
        "click_rate" => "click_rate",
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": [{
                    "message": format!("Invalid order field: {other}. Must be one of: date, open_rate, sent_to, click_rate"),
                    "errorType": "BadRequestError"
                }]})),
            )
                .into_response()
        }
    };
    let direction = match direction.to_ascii_lowercase().as_str() {
        "asc" => "ASC",
        "desc" => "DESC",
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": [{
                    "message": format!("Invalid order direction: {other}"),
                    "errorType": "BadRequestError"
                }]})),
            )
                .into_response()
        }
    };
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).filter(|l| *l > 0).unwrap_or(20).min(500);

    // `newsletter_id` is optional: without it this reports on every
    // newsletter, which is what the single-newsletter sites that are most of
    // them actually want.
    let newsletter = q.get("newsletter_id").cloned();
    let sql = format!(
        "SELECT p.id, p.title, p.published_at, \
                COALESCE(e.email_count, 0) AS sent_to, \
                COALESCE(e.opened_count, 0) AS total_opens, \
                (SELECT COUNT(DISTINCT k.member_uuid) FROM newsletter_clicks k \
                   JOIN newsletter_links l ON l.id = k.link_id WHERE l.post_id = p.id) AS total_clicks, \
                CASE WHEN COALESCE(e.email_count,0) > 0 \
                     THEN CAST(COALESCE(e.opened_count,0) AS REAL) / e.email_count ELSE 0 END AS open_rate, \
                CASE WHEN COALESCE(e.email_count,0) > 0 \
                     THEN CAST((SELECT COUNT(DISTINCT k.member_uuid) FROM newsletter_clicks k \
                                  JOIN newsletter_links l ON l.id = k.link_id WHERE l.post_id = p.id) AS REAL) \
                          / e.email_count ELSE 0 END AS click_rate \
         FROM posts p LEFT JOIN emails e ON e.post_id = p.id \
         WHERE p.status IN ('sent','published') \
           AND substr(p.published_at,1,10) BETWEEN ? AND ? \
           AND (? IS NULL OR p.newsletter_id = ?) \
         ORDER BY {column} {direction} LIMIT ?"
    );
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, i64, i64, f64, f64)>(&sql)
        .bind(&range.from)
        .bind(&range.to)
        .bind(&newsletter)
        .bind(&newsletter)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(
        |(post_id, post_title, send_date, sent_to, total_opens, total_clicks, open_rate, click_rate)| json!({
            "post_id": post_id,
            "post_title": post_title,
            "send_date": send_date,
            "sent_to": sent_to,
            "total_opens": total_opens,
            "total_clicks": total_clicks,
            "open_rate": open_rate,
            "click_rate": click_rate,
        })
    ).collect::<Vec<_>>()}))
}

/// Click counts for a named set of posts — Ghost's `newsletter-click-stats`,
/// which its Newsletters screen calls once it knows which rows are on screen
/// rather than counting clicks for every issue ever sent.
///
/// No post ids means no work, as in Ghost: an empty list is a caller that has
/// nothing to ask about, not a request for everything.
async fn newsletter_click_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers).await {
        return r;
    }
    let ids: Vec<String> = q
        .get("post_ids")
        .map(|raw| raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return private(json!({"stats": []}));
    }
    let newsletter = q.get("newsletter_id").cloned();
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT p.id, \
                (SELECT COUNT(DISTINCT k.member_uuid) FROM newsletter_clicks k \
                   JOIN newsletter_links l ON l.id = k.link_id WHERE l.post_id = p.id), \
                COALESCE(e.email_count, 0) \
         FROM posts p LEFT JOIN emails e ON e.post_id = p.id WHERE p.id IN (",
    );
    let mut separated = query.separated(", ");
    for id in &ids {
        separated.push_bind(id);
    }
    query.push(")");
    if let Some(id) = &newsletter {
        query.push(" AND p.newsletter_id = ").push_bind(id);
    }
    let rows = query
        .build_query_as::<(String, i64, i64)>()
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    private(json!({"stats": rows.into_iter().map(|(post_id, total_clicks, email_count)| json!({
        "post_id": post_id,
        "total_clicks": total_clicks,
        "email_count": email_count,
        "click_rate": if email_count > 0 { total_clicks as f64 / email_count as f64 } else { 0.0 },
    })).collect::<Vec<_>>()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_takes_explicit_bounds_or_counts_back_from_today() {
        let explicit =
            Range::parse(&HashMap::from([("date_from".into(), "2026-01-01".into()), ("date_to".into(), "2026-01-31".into())]));
        assert_eq!((explicit.from.as_str(), explicit.to.as_str()), ("2026-01-01", "2026-01-31"));

        // `days` counts inclusively, so one day is today twice over.
        let single = Range::parse(&HashMap::from([("days".into(), "1".into())]));
        assert_eq!(single.from, single.to);

        let week = Range::parse(&HashMap::from([("days".into(), "7".into()), ("date_to".into(), "2026-01-08".into())]));
        assert_eq!((week.from.as_str(), week.to.as_str()), ("2026-01-02", "2026-01-08"));

        // Nonsense falls back rather than failing, and an unbounded range is
        // not something a caller gets to ask for.
        let junk = Range::parse(&HashMap::from([("days".into(), "-5".into())]));
        assert_eq!(junk.from, junk.to);
        let huge = Range::parse(&HashMap::from([("days".into(), "999999".into()), ("date_to".into(), "2026-01-01".into())]));
        assert_eq!(huge.from.as_str(), "2021-01-02");
        assert_eq!(Range::parse(&HashMap::new()).to, chrono::Utc::now().date_naive().to_string());
    }

    #[test]
    fn only_a_bounded_list_of_ids_is_accepted() {
        let many: Vec<Value> = (0..500).map(|n| json!(format!("id{n}"))).collect();
        assert_eq!(requested_ids(&json!({"ids": many})).len(), 100);
        assert_eq!(requested_ids(&json!({"ids": ["a", 3, "b"]})), ["a", "b"]);
        assert!(requested_ids(&json!({})).is_empty());
        assert!(requested_ids(&json!({"ids": "not-a-list"})).is_empty());
    }
}
