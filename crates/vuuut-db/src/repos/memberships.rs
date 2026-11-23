//! The queries behind the Memberships module.
//!
//! **This module owns no table.** Members, tiers, offers, newsletters and
//! subscriptions all already exist and are all already written by somebody
//! else — the portal, the Stripe webhook, the members list, the tier editor.
//! What was missing was a way to read them *together*, and that is all this
//! is: eight reads, no writes, nothing cached. The same argument
//! [`super::seo`] and [`super::dashboard`] make, and the reason this module
//! has no migration either.
//!
//! The reads are deliberately shaped so the overview is a fixed number of
//! round trips rather than a number that grows with the number of tiers:
//!
//! 1. [`funnel`] — every count on the overview, as one query of scalar
//!    subqueries.
//! 2. [`mrr_cents`] — alone, for the reason spelled out on its doc comment.
//! 3. [`signups_by_day`] — the growth chart, one row per day per status.
//! 4. [`tier_stats`] — every tier with its member count and its share of MRR.
//! 5. [`offer_stats`] — every offer with its redemptions and the tier it
//!    discounts.
//! 6. [`newsletter_stats`] — every newsletter with its subscriber count.
//! 7. [`sources`] — where the window's signups came from.
//! 8. [`recent_signups`] — the latest arrivals, for the overview's activity
//!    card.
//!
//! Counting in SQL rather than in Rust matters here: `tier_stats` on a site
//! with eight tiers is one query, where asking each tier for its members
//! would be nine. The tier and offer lists are small enough to read whole.
//!
//! ## Personal data
//!
//! [`recent_signups`] returns member names and email addresses, which is
//! personal data about readers. It is read only by the Memberships overview,
//! which is Administrator-only for exactly that reason — the same level the
//! members list itself is gated at. Nothing else here identifies anybody.
//!
//! ## MySQL
//!
//! Every instant a query compares against is computed in Rust and bound.
//! SQLite would take `datetime('now', ?)` and MySQL's `INTERVAL` is syntax
//! rather than a value, so the whole project computes these in one place —
//! see [`vuuut_core::dialect`]. Nothing here reads a wide column: a
//! membership report has no use for a post body, and `MEDIUMTEXT` cannot be
//! decoded as a `String` through the `Any` driver at all.
use super::dberr;
use crate::Db;
use vuuut_core::Result;

/// Every count the overview shows, in one round trip.
///
/// A struct rather than a tuple of fourteen integers, for the reason
/// [`super::dashboard::Snapshot`] gives: nothing catches a transposed pair in
/// a tuple that wide, and `(free, paid)` the wrong way round compiles, runs
/// and quietly lies on the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct Counts {
    pub members: i64,
    /// Members on the `free` status.
    pub free: i64,
    /// Members on a paid subscription.
    pub paid: i64,
    /// Members given a paid tier by hand.
    pub comped: i64,
    /// Members whose email was switched off after delivery to them failed.
    pub email_disabled: i64,
    /// Members created inside the window.
    pub joined: i64,
    /// Members created inside the window immediately before this one.
    ///
    /// Counted directly rather than as a subtraction, the same care
    /// `dashboard::Snapshot::members_before` takes: a member who joined in
    /// the window and has since been deleted is in neither figure, and
    /// subtracting would report the earlier window as busier than it was.
    pub joined_before: i64,
    /// Status changes to `paid` inside the window.
    pub upgraded: i64,
    /// Subscription cancellations inside the window.
    pub cancelled: i64,
    /// Tiers of type `paid` that are active.
    pub paid_tiers: i64,
    /// Active paid tiers a visitor can reach: `visibility <> 'none'`.
    ///
    /// The visibility test is what makes this mean "for sale". A fresh
    /// install is seeded with an active paid tier at `visibility = 'none'`,
    /// which exists so there is something to configure — counting it would
    /// tell every new site its shop is open.
    pub visible_paid_tiers: i64,
    /// Every tier, including free and inactive ones.
    pub tiers: i64,
    /// Offers with `active = 1`.
    pub active_offers: i64,
    pub offers: i64,
    /// Newsletters that are sending.
    pub newsletters: i64,
}

/// Count everything the overview reports.
///
/// `since` opens the window and `before` opens the window in front of it;
/// both are `'YYYY-MM-DD HH:MM:SS'` UTC strings computed by the caller.
///
/// One statement, so one failure mode: if the read fails the screen shows
/// nothing rather than a page silently mixing fresh counts with missing ones.
pub async fn funnel(db: &Db, since: &str, before: &str) -> Result<Counts> {
    sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM members) AS members, \
         (SELECT COUNT(*) FROM members WHERE status = 'free') AS free, \
         (SELECT COUNT(*) FROM members WHERE status = 'paid') AS paid, \
         (SELECT COUNT(*) FROM members WHERE status = 'comped') AS comped, \
         (SELECT COUNT(*) FROM members WHERE email_disabled = 1) AS email_disabled, \
         (SELECT COUNT(*) FROM members WHERE created_at >= ?) AS joined, \
         (SELECT COUNT(*) FROM members WHERE created_at >= ? AND created_at < ?) AS joined_before, \
         (SELECT COUNT(*) FROM members_status_events WHERE to_status = 'paid' \
           AND created_at >= ?) AS upgraded, \
         (SELECT COUNT(*) FROM members_cancel_events WHERE created_at >= ?) AS cancelled, \
         (SELECT COUNT(*) FROM products WHERE type = 'paid' AND active = 1) AS paid_tiers, \
         (SELECT COUNT(*) FROM products WHERE type = 'paid' AND active = 1 \
           AND visibility <> 'none') AS visible_paid_tiers, \
         (SELECT COUNT(*) FROM products) AS tiers, \
         (SELECT COUNT(*) FROM offers WHERE active = 1) AS active_offers, \
         (SELECT COUNT(*) FROM offers) AS offers, \
         (SELECT COUNT(*) FROM newsletters WHERE status = 'active') AS newsletters",
    )
    .bind(since)
    .bind(before)
    .bind(since)
    .bind(since)
    .bind(since)
    .fetch_one(db)
    .await
    .map_err(dberr)
}

/// Monthly recurring revenue in minor units, from the live subscriptions.
///
/// Delegates to [`super::dashboard::mrr_minor`] rather than repeating the
/// query: the admin home screen and this module must not be able to disagree
/// about what the site earns, and two copies of a `SUM` over a `REAL` column
/// is precisely the kind of duplication that ends in two different numbers on
/// two screens.
///
/// It stays a separate round trip from [`funnel`] for the reason given there:
/// `subscriptions.mrr` is a `REAL`, `SUM` over it decodes as either an
/// integer or a float depending on the engine and on whether any rows
/// matched, and one undecodable value folded into the big query would zero
/// all fifteen counts beside it.
pub async fn mrr_cents(db: &Db) -> i64 {
    super::dashboard::mrr_minor(db).await
}

/// Signups per day per status since `since`, as `(day, status, count)`,
/// ascending by day.
///
/// `status` is the member's status **now**, not what it was on the day they
/// joined — the events needed for the latter are per-member and the chart
/// would cost a join per bar. Stated here because it changes what the chart
/// means: a bar is "the people who joined that day, coloured by what they
/// have become", which is the more useful of the two readings and the less
/// obvious one.
///
/// Days with no signups are absent. The caller fills the gaps, because only
/// it knows how wide its chart is.
pub async fn signups_by_day(db: &Db, since: &str) -> Vec<(String, String, i64)> {
    sqlx::query_as(
        "SELECT substr(created_at, 1, 10) AS day, status, COUNT(*) FROM members \
         WHERE created_at >= ? GROUP BY day, status ORDER BY day",
    )
    .bind(since)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// One tier, with what it is worth.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TierStat {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// `paid` or `free`.
    pub kind: String,
    /// `public` or `none` — see [`vuuut_core::memberships::visibility_label`].
    pub visibility: String,
    /// Read as an integer, not a `bool`: `Decode<Any> for bool` fails on both
    /// engines, where a boolean is a 0/1 column. See [`crate::types::flag`].
    pub active: i64,
    pub currency: Option<String>,
    pub monthly_price: Option<i64>,
    pub yearly_price: Option<i64>,
    pub trial_days: i64,
    /// Members holding this tier.
    pub members: i64,
}

impl TierStat {
    pub fn is_active(&self) -> bool {
        self.active != 0
    }

    pub fn is_paid(&self) -> bool {
        self.kind == "paid"
    }

    /// Whether a visitor is ever shown this tier.
    pub fn is_visible(&self) -> bool {
        self.visibility == "public"
    }
}

/// Every tier with its member count, paid tiers first and the highest priced
/// of those first.
///
/// One query with a correlated subquery rather than a query per tier: a site
/// with eight tiers would otherwise cost nine round trips to draw one card.
///
/// `members_products` is the holding, which is what a tier's size means here —
/// a member can hold a tier through a subscription, a comp or an import, and
/// all three are somebody who has it.
pub async fn tier_stats(db: &Db) -> Result<Vec<TierStat>> {
    sqlx::query_as(
        "SELECT p.id, p.name, p.slug, p.type AS kind, p.visibility, p.active, p.currency, \
         p.monthly_price, p.yearly_price, p.trial_days, \
         (SELECT COUNT(*) FROM members_products mp WHERE mp.product_id = p.id) AS members \
         FROM products p \
         ORDER BY CASE p.type WHEN 'paid' THEN 0 ELSE 1 END, \
                  COALESCE(p.monthly_price, 0) DESC, p.name",
    )
    .fetch_all(db)
    .await
    .map_err(dberr)
}

/// One offer, with how often it has been taken up.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct OfferStat {
    pub id: String,
    pub name: String,
    pub code: String,
    /// Read as an integer rather than a `bool` — see [`TierStat::active`].
    pub active: i64,
    /// `percent` or `fixed`.
    pub discount_type: Option<String>,
    pub discount_amount: Option<i64>,
    /// `once`, `forever` or `repeating`.
    pub duration: Option<String>,
    /// `month` or `year`.
    pub interval: Option<String>,
    pub currency: Option<String>,
    /// The tier being discounted. `None` only if the tier was deleted under
    /// it, which the schema's `ON DELETE CASCADE` should prevent.
    pub tier_name: Option<String>,
    /// Members who arrived through this offer.
    pub redemptions: i64,
}

impl OfferStat {
    pub fn is_active(&self) -> bool {
        self.active != 0
    }
}

/// Every offer with its tier and its redemption count, active first.
///
/// Redemptions are counted from `members.offer_id`, the same column
/// [`super::offers::redemption_count`] reads, so the two figures cannot
/// disagree.
pub async fn offer_stats(db: &Db) -> Result<Vec<OfferStat>> {
    sqlx::query_as(
        "SELECT o.id, o.name, o.code, o.active, o.discount_type, o.discount_amount, \
         o.duration, o.interval, o.currency, p.name AS tier_name, \
         (SELECT COUNT(*) FROM members m WHERE m.offer_id = o.id) AS redemptions \
         FROM offers o LEFT JOIN products p ON p.id = o.product_id \
         ORDER BY o.active DESC, o.created_at DESC",
    )
    .fetch_all(db)
    .await
    .map_err(dberr)
}

/// Every newsletter with its subscriber count, under this module's own name.
///
/// The query itself belongs to [`super::newsletters`], which owns the
/// `newsletters` and `members_newsletters` tables. It is re-exported rather
/// than reimplemented because the Memberships overview and the Newsletters
/// module must not be able to report different reach for one newsletter —
/// which they briefly did, each applying half of
/// [`super::newsletters::SUBSCRIBED`].
pub use super::newsletters::{stats as newsletter_stats, NewsletterStat};

/// Where the window's signups came from, as `(source, count)`, busiest first.
///
/// From `members_created_events`, which is the only record of how somebody
/// arrived — the member row itself does not keep it. A member whose event row
/// is missing (an import that predates the table, a hand-inserted row) is
/// absent rather than counted as `admin`, so these counts can legitimately
/// add up to less than the signups in the window. The screen says so.
pub async fn sources(db: &Db, since: &str) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT source, COUNT(*) AS n FROM members_created_events \
         WHERE created_at >= ? GROUP BY source ORDER BY n DESC",
    )
    .bind(since)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}

/// One recent arrival.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Arrival {
    pub id: String,
    pub name: Option<String>,
    pub email: String,
    pub status: String,
    pub created_at: String,
    /// How they arrived, from `members_created_events`. `None` when there is
    /// no event row — see [`sources`].
    pub source: Option<String>,
}

/// The latest members to arrive, newest first.
///
/// `limit` is clamped, as every list query in this project clamps it: a limit
/// that came from a request and was passed through would let one page ask for
/// the whole table.
pub async fn recent_signups(db: &Db, limit: i64) -> Result<Vec<Arrival>> {
    sqlx::query_as(
        "SELECT m.id, m.name, m.email, m.status, m.created_at, \
         (SELECT e.source FROM members_created_events e WHERE e.member_id = m.id \
           ORDER BY e.created_at LIMIT 1) AS source \
         FROM members m ORDER BY m.created_at DESC, m.id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 50))
    .fetch_all(db)
    .await
    .map_err(dberr)
}
