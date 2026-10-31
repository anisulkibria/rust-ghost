use ghost_db::repos::gift_links;

#[tokio::test]
async fn concurrent_ensures_on_separate_connections_create_one_live_token() {
    let file =
        std::env::temp_dir().join(format!("ghost-gifts-{}.sqlite", ghost_core::ids::new_id()));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&file)
                .create_if_missing(true)
                .busy_timeout(std::time::Duration::from_secs(10)),
        )
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO posts(id,uuid,title,slug,created_at)VALUES('gift-race','gift-race','Gift race','gift-race',datetime('now'))")
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
    std::fs::remove_file(file).unwrap();
}
