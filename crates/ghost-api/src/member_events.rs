//! Member event history: what a member did, and what brought them in.
//!
//! Ghost keeps eleven of these tables and merges them into one feed, which is
//! what the member detail page shows and what `GET /members/events` serves.
//! This records the same events at the points where this port already changes
//! a member, and merges the same way.
//!
//! Two things are being answered here, and they are easy to confuse:
//!
//! * *attribution* — the resource a signup or a subscription is credited to.
//!   A post, a tag page, the homepage. This is what "which post brought them
//!   in" means, and it is resolved from the pages the visitor was on.
//! * *referrer* — where they were before the site at all: a search engine, a
//!   social network, a newsletter they clicked. Plus the `utm_*` tags
//!   whoever sent them chose to put on the URL.
use crate::{
    permissions::{self, Role},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use ghost_core::{dates, ids, GhostError, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/members/events", get(activity_feed))
        .route("/members/events/", get(activity_feed))
        .route("/members/{id}/events", get(member_activity_feed))
        .route("/members/{id}/events/", get(member_activity_feed))
}

// ---------------------------------------------------------------------------
// attribution
// ---------------------------------------------------------------------------

/// Where a member came from, as far as it could be worked out.
#[derive(Debug, Clone, Default)]
pub struct Attribution {
    /// The id of the post, page, tag or author the signup is credited to.
    pub id: Option<String>,
    /// `post`, `page`, `tag`, `author` or `url` — the last meaning a page
    /// that is not one of the others, such as the homepage.
    pub kind: Option<String>,
    pub url: Option<String>,
    pub referrer_source: Option<String>,
    pub referrer_medium: Option<String>,
    pub referrer_url: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_term: Option<String>,
    pub utm_content: Option<String>,
}

/// Resolve Portal's URL history into an attribution.
///
/// Portal keeps the last few pages a visitor was on in local storage and
/// posts them with the signup, each entry carrying the path and whatever it
/// could tell about the referrer. Ghost credits the *most recent* page that
/// resolves to a resource, and takes the referrer from the *earliest* entry
/// that names one — the last thing they read, and the thing that first sent
/// them. Both are deliberate, and swapping either makes the numbers mean
/// something else.
pub async fn attribution_from_history(state: &AppState, history: &Value) -> Attribution {
    let mut attribution = Attribution::default();
    let Some(entries) = history.as_array().filter(|entries| !entries.is_empty()) else {
        return attribution;
    };

    // Earliest entry with a referrer wins: it is the one outside the site.
    for entry in entries {
        let source = entry.get("referrerSource").and_then(Value::as_str).filter(|s| !s.is_empty());
        if source.is_some() && attribution.referrer_source.is_none() {
            attribution.referrer_source = source.map(str::to_owned);
            attribution.referrer_medium =
                entry.get("referrerMedium").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            attribution.referrer_url =
                entry.get("referrerUrl").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        }
    }

    for entry in entries {
        let Some(path) = entry.get("path").and_then(Value::as_str) else { continue };
        // Campaign tags ride on the URL, so the first entry carrying them is
        // the one the link had.
        if attribution.utm_source.is_none() {
            let utm = utm_from_path(path);
            if utm.iter().any(Option::is_some) {
                let [source, medium, campaign, term, content] = utm;
                attribution.utm_source = source;
                attribution.utm_medium = medium;
                attribution.utm_campaign = campaign;
                attribution.utm_term = term;
                attribution.utm_content = content;
            }
        }
    }

    // Most recent resolvable page, so a member who browsed three posts is
    // credited to the one they signed up from.
    for entry in entries.iter().rev() {
        let Some(path) = entry.get("path").and_then(Value::as_str) else { continue };
        let bare = path.split(['?', '#']).next().unwrap_or(path);
        if let Some((kind, id)) = resolve_path(state, bare).await {
            attribution.kind = Some(kind);
            attribution.id = id;
            attribution.url = Some(bare.to_string());
            return attribution;
        }
    }
    // Nothing resolvable: the most recent path is still worth keeping, since
    // "signed up from the homepage" is an answer.
    if let Some(path) = entries.last().and_then(|e| e.get("path")).and_then(Value::as_str) {
        attribution.kind = Some("url".into());
        attribution.url = Some(path.split(['?', '#']).next().unwrap_or(path).to_string());
    }
    attribution
}

fn utm_from_path(path: &str) -> [Option<String>; 5] {
    let Some((_, query)) = path.split_once('?') else { return Default::default() };
    let mut values: HashMap<String, String> = HashMap::new();
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key.starts_with("utm_") && !value.is_empty() {
            values.insert(key.to_string(), value.to_string());
        }
    }
    let take = |key: &str| values.get(key).cloned();
    [take("utm_source"), take("utm_medium"), take("utm_campaign"), take("utm_term"), take("utm_content")]
}

/// What a site path is, as far as the router and the database agree.
///
/// Returns `None` for a path that resolves to no resource at all, so the
/// caller can fall back; `Some((kind, None))` for one that is a known kind of
/// page but whose row has since gone.
async fn resolve_path(state: &AppState, path: &str) -> Option<(String, Option<String>)> {
    use ghost_routing::tree::{RouteMatch, TaxonomyKind};
    let router = state.router.read().await.clone();
    let matched = router.match_path(path)?;
    let (kind, slug) = match matched {
        RouteMatch::CollectionEntry { slug, .. } => ("post", slug),
        RouteMatch::StaticPage { slug } => ("page", slug),
        RouteMatch::Taxonomy { kind: TaxonomyKind::Tag, slug, .. } => ("tag", slug),
        RouteMatch::Taxonomy { kind: TaxonomyKind::Author, slug, .. } => ("author", slug),
        // A collection index, an RSS feed or a routes.yaml page is a real
        // page but not a resource to credit, so it is left to the caller's
        // `url` fallback.
        _ => return None,
    };
    let id: Option<String> = match kind {
        // A post permalink and a page permalink can both be `/{slug}/`, so
        // the type is checked as well as the slug.
        "post" => sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? AND type = 'post'"),
        "page" => sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? AND type = 'page'"),
        "tag" => sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?"),
        _ => sqlx::query_scalar("SELECT id FROM users WHERE slug = ?"),
    }
    .bind(&slug)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    Some((kind.to_string(), id))
}

// ---------------------------------------------------------------------------
// recording
// ---------------------------------------------------------------------------

/// The signup itself, with where it came from.
///
/// Replaces the bare `source`-only row this used to write: the source says
/// *how* (Portal, an import, the admin), and the attribution says *where*.
pub async fn record_signup(state: &AppState, member_id: &str, source: &str, attribution: &Attribution) -> Result<()> {
    let now = dates::now_db();
    sqlx::query(
        "INSERT INTO members_created_events \
         (id, member_id, source, attribution_id, attribution_type, attribution_url, \
          referrer_source, referrer_medium, referrer_url, \
          utm_source, utm_medium, utm_campaign, utm_term, utm_content, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(member_id)
    .bind(source)
    .bind(&attribution.id)
    .bind(&attribution.kind)
    .bind(&attribution.url)
    .bind(&attribution.referrer_source)
    .bind(&attribution.referrer_medium)
    .bind(&attribution.referrer_url)
    .bind(&attribution.utm_source)
    .bind(&attribution.utm_medium)
    .bind(&attribution.utm_campaign)
    .bind(&attribution.utm_term)
    .bind(&attribution.utm_content)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| GhostError::Db(e.to_string()))?;
    // A signup is also the first status event, which is what makes a status
    // timeline complete rather than starting at the first upgrade.
    record_status_change(state, member_id, None, "free").await
}

pub async fn record_login(state: &AppState, member_id: &str) -> Result<()> {
    sqlx::query("INSERT INTO members_login_events (id, member_id, created_at) VALUES (?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

pub async fn record_status_change(
    state: &AppState,
    member_id: &str,
    from: Option<&str>,
    to: &str,
) -> Result<()> {
    if from == Some(to) {
        return Ok(());
    }
    sqlx::query("INSERT INTO members_status_events (id, member_id, from_status, to_status, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(from)
        .bind(to)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

pub async fn record_email_change(state: &AppState, member_id: &str, from: &str, to: &str) -> Result<()> {
    if from.eq_ignore_ascii_case(to) {
        return Ok(());
    }
    sqlx::query("INSERT INTO members_email_change_events (id, member_id, from_email, to_email, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(from)
        .bind(to)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

/// Subscribing or unsubscribing from one newsletter.
pub async fn record_newsletter_change(
    state: &AppState,
    member_id: &str,
    newsletter_id: Option<&str>,
    subscribed: bool,
    source: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO members_subscribe_events (id, member_id, newsletter_id, subscribed, source, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(newsletter_id)
        .bind(subscribed)
        .bind(source)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

/// A paid subscription starting, credited the same way a signup is.
pub async fn record_subscription_created(
    state: &AppState,
    member_id: &str,
    subscription_id: Option<&str>,
    attribution: &Attribution,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO members_subscription_created_events \
         (id, member_id, subscription_id, attribution_id, attribution_type, attribution_url, \
          referrer_source, referrer_medium, referrer_url, \
          utm_source, utm_medium, utm_campaign, utm_term, utm_content, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(member_id)
    .bind(subscription_id)
    .bind(&attribution.id)
    .bind(&attribution.kind)
    .bind(&attribution.url)
    .bind(&attribution.referrer_source)
    .bind(&attribution.referrer_medium)
    .bind(&attribution.referrer_url)
    .bind(&attribution.utm_source)
    .bind(&attribution.utm_medium)
    .bind(&attribution.utm_campaign)
    .bind(&attribution.utm_term)
    .bind(&attribution.utm_content)
    .bind(dates::now_db())
    .execute(&state.db)
    .await
    .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

/// What a subscription change did, including to monthly recurring revenue.
pub struct PaidChange<'a> {
    pub member_id: &'a str,
    /// `created`, `updated`, `canceled`, `expired` or `reactivated`.
    pub kind: &'a str,
    pub subscription_id: Option<&'a str>,
    pub from_plan: Option<&'a str>,
    pub to_plan: Option<&'a str>,
    pub from_status: Option<&'a str>,
    pub to_status: Option<&'a str>,
    pub currency: Option<&'a str>,
    pub source: &'a str,
    /// The change in monthly recurring revenue, in the currency's smallest
    /// unit. Stored as a delta so an MRR series is a running total rather
    /// than something to recompute from every subscription each time.
    pub mrr_delta: i64,
}

pub async fn record_paid_change(state: &AppState, change: PaidChange<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO members_paid_subscription_events \
         (id, member_id, type, subscription_id, from_plan, to_plan, from_status, to_status, \
          currency, source, mrr_delta, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(change.member_id)
    .bind(change.kind)
    .bind(change.subscription_id)
    .bind(change.from_plan)
    .bind(change.to_plan)
    .bind(change.from_status)
    .bind(change.to_status)
    .bind(change.currency)
    .bind(change.source)
    .bind(change.mrr_delta)
    .bind(dates::now_db())
    .execute(&state.db)
    .await
    .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

pub async fn record_payment(
    state: &AppState,
    member_id: &str,
    amount: i64,
    currency: Option<&str>,
    source: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO members_payment_events (id, member_id, amount, currency, source, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(amount)
        .bind(currency)
        .bind(source)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

/// A tier being granted or taken away. `action` is `added` or `removed`.
pub async fn record_product_change(
    state: &AppState,
    member_id: &str,
    product_id: &str,
    action: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO members_product_events (id, member_id, product_id, action, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(product_id)
        .bind(action)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

pub async fn record_cancellation(
    state: &AppState,
    member_id: &str,
    subscription_id: Option<&str>,
    from_plan: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO members_cancel_events (id, member_id, subscription_id, from_plan, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(ids::new_id())
        .bind(member_id)
        .bind(subscription_id)
        .bind(from_plan)
        .bind(dates::now_db())
        .execute(&state.db)
        .await
        .map_err(|e| GhostError::Db(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the feed
// ---------------------------------------------------------------------------

// Row shapes, named rather than written out at each query. `sqlx` builds a
// row into a flat tuple, so these cannot be composed out of smaller ones —
// which is also why they are this wide.
type WhoRow = (String, String, Option<String>, Option<String>, Option<String>);
type SubscriptionRow = (String, String, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<i64>);
type PostRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);
type FeedbackRow = (String, String, Option<String>, Option<String>, Option<String>, i64, Option<String>, Option<String>);
type RecipientRow =
    (String, String, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<String>, Option<String>, Option<String>);

/// One query per event kind, merged and sorted, because that is what the
/// shapes allow: these rows have nothing in common but a member and a time,
/// and a `UNION` wide enough to hold all of them would be a column soup that
/// no reader could check. The merge is bounded by `limit` at both ends — each
/// query takes at most `limit`, and the merged list is cut to `limit` — so
/// the work does not grow with the size of the history.
async fn events_for(state: &AppState, member: Option<&str>, limit: i64) -> Vec<Value> {
    let mut events: Vec<(String, Value)> = Vec::new();

    // Rows of (created_at, member_id, name, email, ...extra) per kind.
    macro_rules! collect {
        ($sql:expr, $kind:expr, $extra:expr) => {{
            let rows = sqlx::query_as::<_, WhoRow>(&format!(
                "SELECT {} FROM {} ORDER BY e.created_at DESC LIMIT ?",
                "e.created_at, m.id, m.name, m.email, m.uuid", $sql
            ))
            .bind(member)
            .bind(member)
            .bind(limit)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
            for (created_at, id, name, email, uuid) in rows {
                let member = json!({"id": id, "uuid": uuid, "name": name, "email": email});
                events.push((created_at.clone(), json!({
                    "type": $kind,
                    "data": {"created_at": created_at, "member": member, "extra": $extra},
                })));
            }
        }};
    }

    // Each of these is "the event table joined to its member, optionally
    // narrowed to one member". The `? IS NULL OR` pair is how one query
    // serves both the whole-site feed and a single member's.
    let scoped = |table: &str| {
        format!("{table} e JOIN members m ON m.id = e.member_id WHERE (? IS NULL OR e.member_id = ?)")
    };

    collect!(scoped("members_created_events"), "signup_event", Value::Null);
    collect!(scoped("members_login_events"), "login_event", Value::Null);
    collect!(scoped("members_subscription_created_events"), "subscription_event", Value::Null);
    collect!(scoped("members_payment_events"), "payment_event", Value::Null);
    collect!(scoped("members_subscribe_events"), "newsletter_event", Value::Null);
    collect!(scoped("members_email_change_events"), "email_change_event", Value::Null);
    collect!(scoped("members_cancel_events"), "cancel_event", Value::Null);

    // The ones whose detail is worth carrying get their own query rather than
    // the macro, because what the page shows is the detail.
    let detailed: Vec<SubscriptionRow> = sqlx::query_as(
            "SELECT e.created_at, m.id, m.name, m.email, m.uuid, e.type, e.currency, e.mrr_delta \
             FROM members_paid_subscription_events e JOIN members m ON m.id = e.member_id \
             WHERE (? IS NULL OR e.member_id = ?) ORDER BY e.created_at DESC LIMIT ?",
        )
        .bind(member)
        .bind(member)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for (created_at, id, name, email, uuid, kind, currency, mrr_delta) in detailed {
        events.push((
            created_at.clone(),
            json!({"type": "subscription_event", "data": {
                "created_at": created_at,
                "member": {"id": id, "uuid": uuid, "name": name, "email": email},
                "extra": {"type": kind, "currency": currency, "mrr_delta": mrr_delta},
            }}),
        ));
    }

    // Comments, clicks and feedback are not in the member event tables —
    // they are the content tables, read as events.
    let comments: Vec<PostRow> = sqlx::query_as(
            "SELECT c.created_at, m.id, m.name, m.email, m.uuid, p.title, p.id \
             FROM comments c JOIN members m ON m.id = c.member_id LEFT JOIN posts p ON p.id = c.post_id \
             WHERE (? IS NULL OR c.member_id = ?) AND c.status = 'published' ORDER BY c.created_at DESC LIMIT ?",
        )
        .bind(member)
        .bind(member)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for (created_at, id, name, email, uuid, title, post_id) in comments {
        events.push((
            created_at.clone(),
            json!({"type": "comment_event", "data": {
                "created_at": created_at,
                "member": {"id": id, "uuid": uuid, "name": name, "email": email},
                "extra": {"post": {"id": post_id, "title": title}},
            }}),
        ));
    }

    let clicks: Vec<PostRow> = sqlx::query_as(
            "SELECT k.created_at, m.id, m.name, m.email, m.uuid, l.to_url, l.post_id \
             FROM newsletter_clicks k JOIN members m ON m.uuid = k.member_uuid \
             LEFT JOIN newsletter_links l ON l.id = k.link_id \
             WHERE (? IS NULL OR m.id = ?) ORDER BY k.created_at DESC LIMIT ?",
        )
        .bind(member)
        .bind(member)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for (created_at, id, name, email, uuid, url, post_id) in clicks {
        events.push((
            created_at.clone(),
            json!({"type": "click_event", "data": {
                "created_at": created_at,
                "member": {"id": id, "uuid": uuid, "name": name, "email": email},
                "extra": {"link": {"to": url}, "post": {"id": post_id}},
            }}),
        ));
    }

    let feedback: Vec<FeedbackRow> = sqlx::query_as(
            "SELECT f.created_at, m.id, m.name, m.email, m.uuid, f.score, p.title, p.id \
             FROM members_feedback f JOIN members m ON m.id = f.member_id \
             LEFT JOIN posts p ON p.id = f.post_id \
             WHERE (? IS NULL OR f.member_id = ?) ORDER BY f.created_at DESC LIMIT ?",
        )
        .bind(member)
        .bind(member)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for (created_at, id, name, email, uuid, score, title, post_id) in feedback {
        events.push((
            created_at.clone(),
            json!({"type": "feedback_event", "data": {
                "created_at": created_at,
                "member": {"id": id, "uuid": uuid, "name": name, "email": email},
                "extra": {"score": score, "post": {"id": post_id, "title": title}},
            }}),
        ));
    }

    // What became of the emails this member was sent. `email_recipients` only
    // started carrying these when sending began writing to it.
    let emails: Vec<RecipientRow> = sqlx::query_as(
            "SELECT r.created_at, m.id, m.name, m.email, m.uuid, r.status, r.delivered_at, r.opened_at, r.failed_at, p.title \
             FROM email_recipients r JOIN members m ON m.id = r.member_id \
             LEFT JOIN emails e ON e.id = r.email_id LEFT JOIN posts p ON p.id = e.post_id \
             WHERE (? IS NULL OR r.member_id = ?) ORDER BY r.created_at DESC LIMIT ?",
        )
        .bind(member)
        .bind(member)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    for (created_at, id, name, email, uuid, status, delivered_at, opened_at, failed_at, title) in emails {
        let who = json!({"id": id, "uuid": uuid, "name": name, "email": email});
        let subject = json!({"title": title});
        // One recipient row is up to four events: sent, then delivered,
        // opened or failed, each at its own time.
        let mut push = |kind: &str, at: Option<String>| {
            if let Some(at) = at {
                events.push((
                    at.clone(),
                    json!({"type": kind, "data": {"created_at": at, "member": who, "extra": {"email": subject}}}),
                ));
            }
        };
        push("email_sent_event", Some(created_at));
        push("email_delivered_event", delivered_at);
        push("email_opened_event", opened_at);
        if status == "failed" {
            push("email_failed_event", failed_at);
        }
    }

    events.sort_by(|a, b| b.0.cmp(&a.0));
    events.truncate(limit as usize);
    events.into_iter().map(|(_, event)| event).collect()
}

/// One member's activity, for the admin's own member page.
///
/// The same merge the API serves, so the page and the API cannot disagree
/// about what a member did.
pub async fn events_for_member(state: &AppState, member_id: &str, limit: i64) -> Vec<Value> {
    events_for(state, Some(member_id), limit).await
}

/// `GET /members/events` — the whole publication's activity.
async fn activity_feed(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(50).clamp(1, 500);
    let filtered = filter_types(events_for(&state, None, limit).await, q.get("filter").map(String::as_str));
    Json(json!({"events": filtered})).into_response()
}

/// `GET /members/{id}/events` — one member's activity, for their detail page.
async fn member_activity_feed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = permissions::require(&state, &headers, Role::Editor).await {
        return r;
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM members WHERE id = ?)")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
    if !exists {
        return StatusCode::NOT_FOUND.into_response();
    }
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(50).clamp(1, 500);
    let filtered = filter_types(events_for(&state, Some(&id), limit).await, q.get("filter").map(String::as_str));
    Json(json!({"events": filtered})).into_response()
}

/// Ghost filters this feed with `filter=type:[a,b]`, which is the one NQL
/// shape its own admin sends here.
fn filter_types(events: Vec<Value>, filter: Option<&str>) -> Vec<Value> {
    let Some(filter) = filter else { return events };
    let Some(list) = filter.strip_prefix("type:") else { return events };
    let wanted: Vec<&str> =
        list.trim_matches(|c| c == '[' || c == ']').split(',').map(str::trim).filter(|t| !t.is_empty()).collect();
    if wanted.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|event| event.get("type").and_then(Value::as_str).is_some_and(|t| wanted.contains(&t)))
        .collect()
}
