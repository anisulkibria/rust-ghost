//! The Memberships repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! Both engines matter here for three reasons, each the shape of thing that
//! works on one and silently does nothing on the other:
//!
//! * Every window is a **string** comparison against a timestamp built in
//!   Rust, because `datetime('now', ?)` with a bound modifier is not portable
//!   — the repo-layer rule.
//! * `substr(created_at, 1, 10)` groups the growth chart by day.
//! * The booleans (`products.active`, `offers.active`, `members.email_disabled`)
//!   are 0/1 integer columns on both engines and are read as `i64`, because
//!   `Decode<Any> for bool` fails on both.
//!
//! The counting is what these tests are really about. A report that
//! double-counts, or that counts a hidden tier as for sale, is wrong in a way
//! nobody notices until somebody acts on it.
use ghost_core::dates::to_db_string;
use ghost_db::pool::connect_test;
use ghost_db::repos::memberships as repo;
use ghost_db::Db;

fn days_ago(n: i64) -> String {
    to_db_string(&(chrono::Utc::now() - chrono::Duration::days(n)))
}

/// A member, with the status they are on now and the day they arrived.
async fn member(db: &Db, email: &str, status: &str, created: &str) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, status, name, email_disabled, \
         created_at) VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(ghost_core::ids::new_id())
    .bind(email)
    .bind(status)
    .bind(email.split('@').next().unwrap_or("Reader"))
    .bind(created)
    .execute(db)
    .await
    .unwrap();
    id
}

/// A tier. `price` is the monthly price in minor units.
async fn tier(db: &Db, name: &str, kind: &str, active: i64, visibility: &str, price: Option<i64>) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO products (id, name, slug, active, visibility, type, currency, \
         monthly_price, yearly_price, trial_days, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'usd', ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(ghost_core::slug::safe_string(name))
    .bind(active)
    .bind(visibility)
    .bind(kind)
    .bind(price)
    .bind(price.map(|p| p * 10))
    .bind(ghost_core::dates::now_db())
    .execute(db)
    .await
    .unwrap();
    id
}

async fn hold(db: &Db, member_id: &str, tier_id: &str) {
    sqlx::query("INSERT INTO members_products (id, member_id, product_id, sort_order) VALUES (?, ?, ?, 0)")
        .bind(ghost_core::ids::new_id())
        .bind(member_id)
        .bind(tier_id)
        .execute(db)
        .await
        .unwrap();
}

async fn created_event(db: &Db, member_id: &str, source: &str, created: &str) {
    sqlx::query("INSERT INTO members_created_events (id, member_id, source, created_at) VALUES (?, ?, ?, ?)")
        .bind(ghost_core::ids::new_id())
        .bind(member_id)
        .bind(source)
        .bind(created)
        .execute(db)
        .await
        .unwrap();
}

async fn status_event(db: &Db, member_id: &str, to: &str, created: &str) {
    sqlx::query(
        "INSERT INTO members_status_events (id, member_id, from_status, to_status, created_at) \
         VALUES (?, ?, 'free', ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(member_id)
    .bind(to)
    .bind(created)
    .execute(db)
    .await
    .unwrap();
}

async fn cancel_event(db: &Db, member_id: &str, created: &str) {
    sqlx::query(
        "INSERT INTO members_cancel_events (id, member_id, subscription_id, from_plan, created_at) \
         VALUES (?, ?, NULL, 'monthly', ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(member_id)
    .bind(created)
    .execute(db)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// the funnel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_site_counts_zero_rather_than_failing() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let counts = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(counts, repo::Counts::default());
    assert_eq!(repo::mrr_cents(&db).await, 0);
    assert!(repo::tier_stats(&db).await.unwrap().is_empty());
    assert!(repo::offer_stats(&db).await.unwrap().is_empty());
    assert!(repo::newsletter_stats(&db).await.unwrap().is_empty());
    assert!(repo::recent_signups(&db, 5).await.unwrap().is_empty());
    assert!(repo::signups_by_day(&db, &days_ago(30)).await.is_empty());
    assert!(repo::sources(&db, &days_ago(30)).await.is_empty());
}

#[tokio::test]
async fn the_statuses_are_counted_separately_and_add_up_to_the_total() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    member(&db, "a@example.com", "free", &days_ago(1)).await;
    member(&db, "b@example.com", "free", &days_ago(2)).await;
    member(&db, "c@example.com", "paid", &days_ago(3)).await;
    member(&db, "d@example.com", "comped", &days_ago(4)).await;

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!((c.members, c.free, c.paid, c.comped), (4, 2, 1, 1));
    assert_eq!(c.free + c.paid + c.comped, c.members, "a status is being missed");
}

#[tokio::test]
async fn the_two_windows_do_not_overlap_so_a_member_is_in_one_of_them() {
    // The trend arrow is this window against the one before it. If the
    // earlier count were "everything before now" every member would be in
    // both and the arrow would always point down.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    member(&db, "now@example.com", "free", &days_ago(3)).await;
    member(&db, "then@example.com", "free", &days_ago(40)).await;
    member(&db, "ancient@example.com", "free", &days_ago(400)).await;

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(c.joined, 1);
    assert_eq!(c.joined_before, 1, "the previous window caught the wrong rows");
    assert_eq!(c.members, 3);
}

#[tokio::test]
async fn upgrades_and_cancellations_are_counted_inside_the_window_only() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let m = member(&db, "m@example.com", "paid", &days_ago(50)).await;
    status_event(&db, &m, "paid", &days_ago(5)).await;
    status_event(&db, &m, "paid", &days_ago(45)).await;
    // A status change to something else is not an upgrade.
    status_event(&db, &m, "free", &days_ago(5)).await;
    cancel_event(&db, &m, &days_ago(2)).await;
    cancel_event(&db, &m, &days_ago(44)).await;

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(c.upgraded, 1);
    assert_eq!(c.cancelled, 1);
}

#[tokio::test]
async fn a_hidden_paid_tier_is_active_but_is_not_for_sale() {
    // The distinction the whole report turns on. A fresh install ships an
    // active paid tier at visibility 'none' so there is something to
    // configure; counting it as for sale would tell every new site its shop
    // is open.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    tier(&db, "Default Product", "paid", 1, "none", Some(500)).await;
    tier(&db, "Supporter", "paid", 1, "public", Some(900)).await;
    tier(&db, "Retired", "paid", 0, "public", Some(900)).await;
    tier(&db, "Free", "free", 1, "public", None).await;

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(c.tiers, 4);
    assert_eq!(c.paid_tiers, 2, "an inactive tier was counted as active");
    assert_eq!(c.visible_paid_tiers, 1, "a hidden tier was counted as for sale");
}

#[tokio::test]
async fn email_disabled_members_are_counted() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let m = member(&db, "bounced@example.com", "free", &days_ago(1)).await;
    member(&db, "fine@example.com", "free", &days_ago(1)).await;
    sqlx::query("UPDATE members SET email_disabled = 1 WHERE id = ?")
        .bind(&m)
        .execute(&db)
        .await
        .unwrap();

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(c.email_disabled, 1);
}

#[tokio::test]
async fn an_archived_newsletter_is_not_counted_as_one_the_site_has() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    newsletter(&db, "Weekly", "active", 0).await;
    newsletter(&db, "Old news", "archived", 1).await;

    let c = repo::funnel(&db, &days_ago(30), &days_ago(60)).await.unwrap();
    assert_eq!(c.newsletters, 1);
}

// ---------------------------------------------------------------------------
// the growth chart
// ---------------------------------------------------------------------------

#[tokio::test]
async fn signups_are_grouped_by_day_and_by_what_the_member_is_now() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let today = &days_ago(0);
    let day = today[..10].to_string();
    member(&db, "a@example.com", "free", today).await;
    member(&db, "b@example.com", "free", today).await;
    member(&db, "c@example.com", "paid", today).await;
    // Outside the window.
    member(&db, "old@example.com", "free", &days_ago(40)).await;

    let rows = repo::signups_by_day(&db, &days_ago(30)).await;
    let free = rows.iter().find(|(d, s, _)| *d == day && s == "free").map(|(_, _, n)| *n);
    let paid = rows.iter().find(|(d, s, _)| *d == day && s == "paid").map(|(_, _, n)| *n);
    assert_eq!(free, Some(2));
    assert_eq!(paid, Some(1));
    assert!(rows.iter().all(|(d, _, _)| d.len() == 10), "a day is not YYYY-MM-DD: {rows:?}");
    assert_eq!(rows.iter().map(|(_, _, n)| n).sum::<i64>(), 3, "the window leaked");
}

// ---------------------------------------------------------------------------
// tiers, offers, newsletters
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tier_carries_its_member_count_and_paid_tiers_are_listed_first() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let free = tier(&db, "Free", "free", 1, "public", None).await;
    let cheap = tier(&db, "Supporter", "paid", 1, "public", Some(500)).await;
    let dear = tier(&db, "Patron", "paid", 1, "public", Some(2_500)).await;

    let one = member(&db, "a@example.com", "paid", &days_ago(1)).await;
    let two = member(&db, "b@example.com", "paid", &days_ago(1)).await;
    hold(&db, &one, &dear).await;
    hold(&db, &two, &dear).await;
    hold(&db, &one, &cheap).await;

    let stats = repo::tier_stats(&db).await.unwrap();
    assert_eq!(stats.len(), 3);
    // Paid first, dearest first inside that.
    assert_eq!(stats[0].id, dear);
    assert_eq!(stats[1].id, cheap);
    assert_eq!(stats[2].id, free);

    assert_eq!(stats[0].members, 2);
    assert_eq!(stats[1].members, 1);
    assert_eq!(stats[2].members, 0, "a tier nobody holds should count zero, not nothing");

    assert!(stats[0].is_paid() && stats[0].is_active() && stats[0].is_visible());
    assert!(!stats[2].is_paid());
    assert_eq!(stats[0].monthly_price, Some(2_500));
    assert_eq!(stats[0].yearly_price, Some(25_000));
}

#[tokio::test]
async fn an_inactive_or_hidden_tier_still_appears_in_the_list_and_says_so() {
    // The list is the inventory, not the shop window: a tier that is off has
    // to be visible on the screen or there is no way to turn it back on.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    tier(&db, "Retired", "paid", 0, "none", Some(500)).await;

    let stats = repo::tier_stats(&db).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert!(!stats[0].is_active());
    assert!(!stats[0].is_visible());
}

#[tokio::test]
async fn an_offer_carries_its_tier_name_and_its_redemptions_active_first() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let t = tier(&db, "Supporter", "paid", 1, "public", Some(500)).await;
    let live = offer(&db, "Spring", "spring", &t, 1).await;
    offer(&db, "Dormant", "dormant", &t, 0).await;

    let m = member(&db, "a@example.com", "paid", &days_ago(1)).await;
    sqlx::query("UPDATE members SET offer_id = ? WHERE id = ?")
        .bind(&live)
        .bind(&m)
        .execute(&db)
        .await
        .unwrap();

    let stats = repo::offer_stats(&db).await.unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].id, live, "the active offer should lead");
    assert!(stats[0].is_active());
    assert!(!stats[1].is_active());
    assert_eq!(stats[0].tier_name.as_deref(), Some("Supporter"));
    assert_eq!(stats[0].redemptions, 1);
    assert_eq!(stats[1].redemptions, 0);
}

#[tokio::test]
async fn a_newsletter_counts_confirmed_subscribers_only() {
    // An unconfirmed row is somebody who has not agreed yet. Counting them
    // would overstate reach, which is the figure a sending decision is made
    // on.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let n = newsletter(&db, "Weekly", "active", 0).await;
    let archived = newsletter(&db, "Old news", "archived", 1).await;

    let a = member(&db, "a@example.com", "free", &days_ago(1)).await;
    let b = member(&db, "b@example.com", "free", &days_ago(1)).await;
    subscribe(&db, &a, &n, "confirmed").await;
    subscribe(&db, &b, &n, "pending").await;

    let stats = repo::newsletter_stats(&db).await.unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].id, n, "the sending newsletter should lead");
    assert_eq!(stats[0].subscribers, 1);
    assert_eq!(stats[1].id, archived);
    assert_eq!(stats[1].subscribers, 0);
}

// ---------------------------------------------------------------------------
// sources and arrivals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sources_come_back_busiest_first_and_only_from_inside_the_window() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let a = member(&db, "a@example.com", "free", &days_ago(1)).await;
    let b = member(&db, "b@example.com", "free", &days_ago(1)).await;
    let c = member(&db, "c@example.com", "free", &days_ago(1)).await;
    let old = member(&db, "old@example.com", "free", &days_ago(40)).await;
    created_event(&db, &a, "member", &days_ago(1)).await;
    created_event(&db, &b, "member", &days_ago(2)).await;
    created_event(&db, &c, "admin", &days_ago(3)).await;
    created_event(&db, &old, "import", &days_ago(40)).await;

    let rows = repo::sources(&db, &days_ago(30)).await;
    assert_eq!(rows, vec![("member".to_string(), 2), ("admin".to_string(), 1)]);
}

#[tokio::test]
async fn the_recent_arrivals_are_newest_first_and_carry_how_they_arrived() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let first = member(&db, "first@example.com", "free", &days_ago(9)).await;
    let last = member(&db, "last@example.com", "paid", &days_ago(1)).await;
    created_event(&db, &last, "member", &days_ago(1)).await;
    // `first` has no event row: a hand-inserted or long-imported member.

    let rows = repo::recent_signups(&db, 10).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, last);
    assert_eq!(rows[0].source.as_deref(), Some("member"));
    assert_eq!(rows[0].status, "paid");
    assert_eq!(rows[1].id, first);
    assert_eq!(rows[1].source, None, "a missing event must not be invented as 'admin'");
}

#[tokio::test]
async fn a_limit_from_a_request_cannot_ask_for_the_whole_table() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    for i in 0..4 {
        member(&db, &format!("m{i}@example.com"), "free", &days_ago(i + 1)).await;
    }
    assert_eq!(repo::recent_signups(&db, 2).await.unwrap().len(), 2);
    // Clamped at both ends rather than trusted.
    assert_eq!(repo::recent_signups(&db, 0).await.unwrap().len(), 1);
    assert_eq!(repo::recent_signups(&db, -5).await.unwrap().len(), 1);
    assert_eq!(repo::recent_signups(&db, 10_000).await.unwrap().len(), 4);
}

// ---------------------------------------------------------------------------
// fixtures that need more than one line
// ---------------------------------------------------------------------------

async fn newsletter(db: &Db, name: &str, status: &str, sort: i64) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO newsletters (id, uuid, name, slug, status, visibility, sort_order, \
         created_at) VALUES (?, ?, ?, ?, ?, 'members', ?, ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(name)
    .bind(ghost_core::slug::safe_string(name))
    .bind(status)
    .bind(sort)
    .bind(ghost_core::dates::now_db())
    .execute(db)
    .await
    .unwrap();
    id
}

async fn subscribe(db: &Db, member_id: &str, newsletter_id: &str, status: &str) {
    sqlx::query(
        "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(member_id)
    .bind(newsletter_id)
    .bind(status)
    .execute(db)
    .await
    .unwrap();
}

async fn offer(db: &Db, name: &str, code: &str, tier_id: &str, active: i64) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO offers (id, active, name, code, product_id, interval, currency, \
         discount_type, discount_amount, duration, portal_title, redemption_type, created_at) \
         VALUES (?, ?, ?, ?, ?, 'month', 'usd', 'percent', 20, 'once', ?, 'multiple', ?)",
    )
    .bind(&id)
    .bind(active)
    .bind(name)
    .bind(code)
    .bind(tier_id)
    .bind(name)
    .bind(ghost_core::dates::now_db())
    .execute(db)
    .await
    .unwrap();
    id
}
