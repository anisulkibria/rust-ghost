use ghost_db::repos::gift_links;

/// A shared database reached over several connections at once.
///
/// The race under test is between separate connections, which a
/// single-connection in-memory database cannot reproduce. SQLite gets a file —
/// `pool::connect` applies the busy timeout and WAL that make its contention
/// resolve rather than fail — and MySQL gets a database of its own from
/// `GHOST_TEST_MYSQL_URL`, where the pool's four connections provide the same
/// contention. The file is returned for the caller to remove; the `TestDb` is
/// returned so the MySQL database lives as long as the test does.
async fn shared_db() -> (ghost_db::Db, Option<std::path::PathBuf>, Option<ghost_db::TestDb>) {
    let mysql = std::env::var(ghost_db::pool::TEST_MYSQL_URL)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if mysql {
        let test_db = ghost_db::pool::connect_test().await.unwrap();
        (test_db.pool(), None, Some(test_db))
    } else {
        let file =
            std::env::temp_dir().join(format!("ghost-gifts-{}.sqlite", ghost_core::ids::new_id()));
        let pool = ghost_db::pool::connect(&file).await.unwrap();
        (pool, Some(file), None)
    }
}

#[tokio::test]
async fn concurrent_ensures_on_separate_connections_create_one_live_token() {
    let (pool, file, _test_db) = shared_db().await;
    sqlx::query(ghost_db::dialect::sql("INSERT INTO posts(id,uuid,title,slug,created_at)VALUES('gift-race','gift-race','Gift race','gift-race',datetime('now'))"))
        .execute(&pool).await.unwrap();
    let calls = (0..8).map(|i| {
        let pool = pool.clone();
        async move {
            gift_links::mint(&pool, "gift-race", &format!("candidate-{i}"), true)
                .await
                .unwrap()
        }
    });
    let links = futures::future::join_all(calls).await;
    assert_eq!(links.iter().filter(|(_, changed)| *changed).count(), 1);
    assert!(links.iter().all(|(link, _)| link.token == links[0].0.token));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gift_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
    if let Some(file) = file {
        std::fs::remove_file(file).unwrap();
    }
}
