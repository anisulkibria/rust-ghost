//! The dashboard repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! Three things here are worth testing at this level rather than through the
//! admin, because each is decided in SQL and would be invisible in rendered
//! HTML if it were wrong:
//!
//! * the **ordering** `work_queue` imposes — a `CASE` expression over status
//!   and two different date columns, which is the fiddliest query in the
//!   module;
//! * the **overdue boundary**, which is a comparison against a bound instant
//!   and is the one figure on the home screen that reports a fault;
//! * **per-path view counting**, which must count distinct viewers and must
//!   not match a path it was not asked about. It is bound as data rather than
//!   concatenated, and that is also what makes it work on MySQL.
//!
//! Running on MySQL matters for all three: `substr`, `CASE … WHEN`, the
//! scalar-subquery select list and `path IN (?, …)` are each written once and
//! have to mean the same thing on both engines.
use vuuut_db::pool::connect_test;
use vuuut_db::repos::dashboard as repo;

const EPOCH: &str = "2026-11-01 00:00:00";

/// A post, attributed to nobody unless `author` says otherwise.
async fn post(
    db: &vuuut_db::Db,
    slug: &str,
    status: &str,
    updated_at: &str,
    published_at: &str,
    author: Option<&str>,
) -> String {
    let id = vuuut_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at, updated_at, published_at) \
         VALUES (?, ?, ?, ?, 'post', ?, 'public', ?, ?, ?)",
    )
    .bind(&id)
    .bind(vuuut_core::ids::uuid_v4())
    .bind(format!("Post {slug}"))
    .bind(slug)
    .bind(status)
    .bind(EPOCH)
    .bind(updated_at)
    .bind(published_at)
    .execute(db)
    .await
    .unwrap();
    if let Some(author) = author {
        sqlx::query("INSERT INTO posts_authors (id, post_id, author_id, sort_order) VALUES (?, ?, ?, 0)")
            .bind(vuuut_core::ids::new_id())
            .bind(&id)
            .bind(author)
            .execute(db)
            .await
            .unwrap();
    }
    id
}

async fn view(db: &vuuut_db::Db, path: &str, viewer: &str, at: &str) {
    sqlx::query(
        "INSERT INTO page_views (id, path, viewer, created_at, audience) VALUES (?, ?, ?, ?, 'public')",
    )
    .bind(vuuut_core::ids::new_id())
    .bind(path)
    .bind(viewer)
    .bind(at)
    .execute(db)
    .await
    .unwrap();
}

/// Scheduled entries lead, soonest first; drafts follow, most recently edited
/// first. A deadline outranks a recent edit.
#[tokio::test]
async fn the_work_queue_puts_deadlines_before_recent_edits() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    post(&db, "draft-old", "draft", "2026-11-05 00:00:00", "2026-11-05 00:00:00", None).await;
    post(&db, "draft-new", "draft", "2026-11-20 00:00:00", "2026-11-20 00:00:00", None).await;
    post(&db, "sched-late", "scheduled", EPOCH, "2026-12-25 09:00:00", None).await;
    post(&db, "sched-soon", "scheduled", EPOCH, "2026-12-01 09:00:00", None).await;
    // Neither of these belongs in a queue of unfinished work.
    post(&db, "live", "published", EPOCH, EPOCH, None).await;
    post(&db, "mailed", "sent", EPOCH, EPOCH, None).await;

    let queue = repo::work_queue(&db, None, 10).await.unwrap();
    let slugs: Vec<&str> = queue.iter().map(|entry| entry.slug.as_str()).collect();
    assert_eq!(slugs, vec!["sched-soon", "sched-late", "draft-new", "draft-old"]);
}

/// The limit is honoured and clamped, so a caller cannot ask the home screen
/// to load the whole site.
#[tokio::test]
async fn the_work_queue_is_bounded() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    for n in 0..8 {
        post(&db, &format!("d{n}"), "draft", EPOCH, EPOCH, None).await;
    }
    assert_eq!(repo::work_queue(&db, None, 5).await.unwrap().len(), 5);
    // Zero and negative are clamped up to one rather than returning nothing,
    // and a huge request is clamped down.
    assert_eq!(repo::work_queue(&db, None, 0).await.unwrap().len(), 1);
    assert_eq!(repo::work_queue(&db, None, 10_000).await.unwrap().len(), 8);
}

/// Narrowing to an author returns that author's entries and nobody else's —
/// including entries with no author at all, which must not leak in.
#[tokio::test]
async fn narrowing_to_an_author_returns_only_their_entries() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "x")
        .await
        .unwrap();
    // The seed ships a published welcome post attributed to the owner, which
    // would land in `recent_published` and make these counts about the seed
    // rather than about the narrowing.
    sqlx::query("DELETE FROM posts").execute(&db).await.unwrap();

    post(&db, "theirs", "draft", EPOCH, EPOCH, Some(&owner.id)).await;
    post(&db, "orphan", "draft", EPOCH, EPOCH, None).await;
    post(&db, "live-theirs", "published", EPOCH, EPOCH, Some(&owner.id)).await;
    post(&db, "live-orphan", "published", EPOCH, EPOCH, None).await;

    let mine = repo::work_queue(&db, Some(&owner.id), 10).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].slug, "theirs");
    assert_eq!(repo::work_queue(&db, None, 10).await.unwrap().len(), 2);

    let mine = repo::recent_published(&db, Some(&owner.id), 10).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].slug, "live-theirs");
    assert_eq!(repo::recent_published(&db, None, 10).await.unwrap().len(), 2);
}

/// A scheduled post counts as overdue once its time is behind the instant it
/// is compared against, and not before. The boundary is the whole point: this
/// is the one count on the home screen that claims the site is broken.
#[tokio::test]
async fn overdue_means_past_the_instant_it_is_measured_against() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    post(&db, "past", "scheduled", EPOCH, "2026-11-10 09:00:00", None).await;
    post(&db, "future", "scheduled", EPOCH, "2026-12-10 09:00:00", None).await;
    // A draft with a stale `published_at` is not scheduled and never overdue.
    post(&db, "draft", "draft", EPOCH, "2020-01-01 00:00:00", None).await;

    let before = repo::snapshot(&db, "2026-11-10 08:59:59", EPOCH).await.unwrap();
    assert_eq!(before.scheduled, 2);
    assert_eq!(before.overdue_scheduled, 0);

    let after = repo::snapshot(&db, "2026-11-10 09:00:01", EPOCH).await.unwrap();
    assert_eq!(after.scheduled, 2, "an overdue post is still scheduled");
    assert_eq!(after.overdue_scheduled, 1);

    let much_later = repo::snapshot(&db, "2027-01-01 00:00:00", EPOCH).await.unwrap();
    assert_eq!(much_later.overdue_scheduled, 2);
}

/// The snapshot counts each status into the right column. Eleven subqueries in
/// one statement is exactly where a transposition hides.
#[tokio::test]
async fn the_snapshot_counts_content_by_status_and_type() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    post(&db, "d1", "draft", EPOCH, EPOCH, None).await;
    post(&db, "d2", "draft", EPOCH, EPOCH, None).await;
    post(&db, "s1", "scheduled", EPOCH, "2027-01-01 00:00:00", None).await;
    post(&db, "p1", "published", EPOCH, EPOCH, None).await;
    post(&db, "p2", "sent", EPOCH, EPOCH, None).await;
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at, updated_at, published_at) \
         VALUES (?, ?, 'About', 'about', 'page', 'published', 'public', ?, ?, ?)",
    )
    .bind(vuuut_core::ids::new_id())
    .bind(vuuut_core::ids::uuid_v4())
    .bind(EPOCH)
    .bind(EPOCH)
    .bind(EPOCH)
    .execute(&db)
    .await
    .unwrap();

    let snapshot = repo::snapshot(&db, "2026-11-15 00:00:00", EPOCH).await.unwrap();
    assert_eq!(snapshot.drafts, 2);
    assert_eq!(snapshot.scheduled, 1);
    // `sent` is published too — it went to the web as well as to the inbox.
    assert_eq!(snapshot.published, 2);
    // A page is never counted as a post, in either direction.
    assert_eq!(snapshot.pages, 1);
}

/// A fresh install has an active paid tier at `visibility = 'none'`, which is
/// a tier that exists rather than one that is for sale. Counting it would tell
/// every new site to go and connect Stripe.
#[tokio::test]
async fn a_hidden_paid_tier_is_not_counted_as_being_for_sale() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();

    let seeded = repo::snapshot(&db, EPOCH, EPOCH).await.unwrap();
    assert_eq!(seeded.paid_tiers, 0, "the seeded hidden tier was counted as for sale");

    sqlx::query("UPDATE products SET visibility = 'public' WHERE type = 'paid'")
        .execute(&db)
        .await
        .unwrap();
    let offered = repo::snapshot(&db, EPOCH, EPOCH).await.unwrap();
    assert_eq!(offered.paid_tiers, 1);

    // Deactivated is not for sale either, however visible it is.
    sqlx::query("UPDATE products SET active = 0 WHERE type = 'paid'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(repo::snapshot(&db, EPOCH, EPOCH).await.unwrap().paid_tiers, 0);
}

/// Views are counted per path, by distinct viewer, and only for the paths
/// asked about.
#[tokio::test]
async fn views_are_counted_by_viewer_and_only_for_the_paths_asked_for() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    let recent = "2026-11-14 12:00:00";
    view(&db, "/wanted/", "reader-a", recent).await;
    view(&db, "/wanted/", "reader-a", recent).await;
    view(&db, "/wanted/", "reader-b", recent).await;
    view(&db, "/ignored/", "reader-c", recent).await;
    // Outside the window.
    view(&db, "/wanted/", "reader-d", "2026-01-01 00:00:00").await;

    let rows = repo::views_for_paths(&db, &["/wanted/".to_string()], "2026-11-01 00:00:00").await;
    assert_eq!(rows.len(), 1, "a path that was not asked for came back: {rows:?}");
    let (path, visitors, views) = &rows[0];
    assert_eq!(path, "/wanted/");
    // Two people, three visits — the figure on the card is readers, not loads.
    assert_eq!(*visitors, 2);
    assert_eq!(*views, 3);

    // No paths means no query and no rows, rather than `path IN ()`, which is
    // a syntax error on both engines.
    assert!(repo::views_for_paths(&db, &[], "2026-11-01 00:00:00").await.is_empty());
}

/// The path a view is filed under is the one `Entry::path` builds, so the card
/// and the counter cannot disagree about what a post's URL is.
#[tokio::test]
async fn an_entrys_path_is_the_path_its_views_are_filed_under() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    post(&db, "hello-world", "published", EPOCH, EPOCH, None).await;
    view(&db, "/hello-world/", "reader-a", "2026-11-14 12:00:00").await;

    let entries = repo::recent_published(&db, None, 5).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path(), "/hello-world/");

    let paths: Vec<String> = entries.iter().map(repo::Entry::path).collect();
    let rows = repo::views_for_paths(&db, &paths, "2026-11-01 00:00:00").await;
    assert_eq!(rows.len(), 1, "the entry's own path matched nothing: {rows:?}");
}

/// Daily buckets are whole days, ascending, with nothing invented for a quiet
/// day — the caller fills those, because only it knows how wide its chart is.
#[tokio::test]
async fn daily_visitors_buckets_by_day_and_skips_the_quiet_ones() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    view(&db, "/a/", "one", "2026-11-10 01:00:00").await;
    view(&db, "/b/", "one", "2026-11-10 23:00:00").await;
    view(&db, "/a/", "two", "2026-11-10 12:00:00").await;
    // Nothing at all on the 11th.
    view(&db, "/a/", "one", "2026-11-12 08:00:00").await;

    let days = repo::daily_visitors(&db, "2026-11-01 00:00:00").await;
    assert_eq!(
        days,
        vec![("2026-11-10".to_string(), 2), ("2026-11-12".to_string(), 1)],
        "a viewer on two paths in one day is one visitor, and the 11th is absent"
    );
}

/// Members are split at the window the caller names, both sides counted
/// directly so a deletion cannot make them disagree.
#[tokio::test]
async fn members_are_counted_either_side_of_the_window() {
    let test = connect_test().await.unwrap();
    let db = test.pool();

    let member = |email: &'static str, status: &'static str, at: &'static str| {
        let db = db.clone();
        async move {
            let id = vuuut_core::ids::new_id();
            sqlx::query(
                "INSERT INTO members (id, uuid, transient_id, email, status, email_count, email_opened_count, created_at) \
                 VALUES (?, ?, ?, ?, ?, 0, 0, ?)",
            )
            .bind(&id)
            .bind(vuuut_core::ids::uuid_v4())
            .bind(vuuut_core::ids::uuid_v4())
            .bind(email)
            .bind(status)
            .bind(at)
            .execute(&db)
            .await
            .unwrap();
        }
    };
    member("old@example.test", "free", "2026-09-01 00:00:00").await;
    member("new@example.test", "free", "2026-11-14 00:00:00").await;
    member("paying@example.test", "paid", "2026-11-14 00:00:00").await;

    let snapshot = repo::snapshot(&db, "2026-11-15 00:00:00", "2026-11-01 00:00:00").await.unwrap();
    assert_eq!(snapshot.members, 3);
    assert_eq!(snapshot.paid_members, 1);
    assert_eq!(snapshot.new_members, 2);
    assert_eq!(snapshot.members_before, 1);
}

/// With no subscriptions at all the figure is zero rather than an error — a
/// `SUM` over no rows is NULL on both engines, and the home screen must not
/// fall over on a site that sells nothing.
#[tokio::test]
async fn mrr_is_zero_on_a_site_with_no_subscriptions() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    assert_eq!(repo::mrr_minor(&db).await, 0);
}
