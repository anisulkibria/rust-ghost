//! The cookie consent repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! The consent log is read back through sqlx `Any`, which cannot decode a
//! MySQL `bool` — so `gpc` is a SMALLINT read as i64, and this round trip is
//! what proves it on the engine where it matters.

use ghost_core::cookies::{Choice, Declared, Category};
use ghost_db::pool::connect_test;
use ghost_db::repos::cookies as repo;

#[tokio::test]
async fn a_decision_is_logged_exactly_as_rebuilt_and_found_by_its_id_prefix() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let choice = Choice::parse("custom", "analytics,marketing").unwrap();
    repo::record(&pool, id, &choice, 3, true, "/about/").await.unwrap();
    repo::record(&pool, "11111111-2222-4333-8444-555555555555", &Choice::parse("reject_all", "").unwrap(), 3, false, "").await.unwrap();

    let found = repo::log(&pool, "0f8fad5b", 10).await.unwrap();
    assert_eq!(found.len(), 1);
    let row = &found[0];
    assert_eq!(row.consent_id, id);
    assert_eq!(row.action, "custom");
    assert_eq!(row.categories, "necessary,analytics,marketing");
    assert_eq!((row.revision, row.gpc, row.page.as_str()), (3, 1, "/about/"));

    assert_eq!(repo::log(&pool, "", 10).await.unwrap().len(), 2);
    assert_eq!(repo::count(&pool).await, 2);

    let tally = repo::tally(&pool, "2000-01-01 00:00:00").await.unwrap();
    let summary = ghost_core::cookies::Summary::from_rows(&tally);
    assert_eq!((summary.total, summary.custom, summary.reject_all), (2, 1, 1));
}

#[tokio::test]
async fn old_records_are_purged_and_recent_ones_kept() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let choice = Choice::parse("accept_all", "").unwrap();
    repo::record(&pool, "0f8fad5b-d9cb-469f-a165-70867728950e", &choice, 1, false, "").await.unwrap();
    sqlx::query("INSERT INTO cookie_consents (id, consent_id, action, categories, revision, gpc, page, created_at) VALUES ('old1', 'x', 'accept_all', 'necessary', 1, 0, '', '2020-01-01 00:00:00')")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(repo::purge_older_than(&pool, 365).await.unwrap(), 1);
    assert_eq!(repo::count(&pool).await, 1);
}

#[tokio::test]
async fn a_declaration_round_trips_in_reading_order() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let d = |name: &str, category: Category| Declared {
        name: name.into(),
        provider: "Google".into(),
        category,
        duration: "2 years".into(),
        purpose: "Counts visits".into(),
    };
    repo::declare(&pool, &d("zz_ads", Category::Marketing), "").await.unwrap();
    let id = repo::declare(&pool, &d("_ga", Category::Analytics), "google_analytics").await.unwrap();
    assert!(repo::is_declared(&pool, "_ga").await);
    let all = repo::declarations(&pool).await.unwrap();
    assert_eq!(all.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["_ga", "zz_ads"]);
    assert_eq!(all[0].tracker_key, "google_analytics");
    repo::undeclare(&pool, &id).await.unwrap();
    assert!(!repo::is_declared(&pool, "_ga").await);
}

#[tokio::test]
async fn the_scan_reads_only_published_rows_with_something_to_find() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    for (slug, status, html, head) in [
        ("tracked", "published", "<p>x</p>", "<script src=\"https://connect.facebook.net/x.js\"></script>"),
        ("video", "published", "<iframe src=\"https://www.youtube.com/embed/a\"></iframe>", ""),
        ("plain", "published", "<p>nothing</p>", ""),
        ("draft", "draft", "<script>gtag('config','G-1')</script>", ""),
    ] {
        sqlx::query("INSERT INTO posts (id, uuid, title, slug, html, codeinjection_head, type, status, visibility, created_at, updated_at, published_at) VALUES (?, ?, ?, ?, ?, ?, 'post', ?, 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
            .bind(ghost_core::ids::new_id())
            .bind(ghost_core::ids::uuid_v4())
            .bind(slug)
            .bind(slug)
            .bind(html)
            .bind(head)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
    }
    let rows = repo::scannable(&pool, 100).await.unwrap();
    let mut titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["tracked", "video"]);
    assert!(rows.iter().any(|r| r.markup.contains("connect.facebook.net")));
}
