//! Transactional capture is tested on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set. Only opaque row identifiers leave the worker.
use ghost_db::pool::connect_test;
#[tokio::test]
async fn captures_only_committed_facts_and_deduplicates_newsletter_activity() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let now = ghost_core::dates::now_db();
    let member = |id: &str| {
        sqlx::query("INSERT INTO members(id,uuid,transient_id,email,created_at) VALUES(?,?,?,?,?)")
            .bind(id.to_string())
            .bind(id.to_string())
            .bind(id.to_string())
            .bind(format!("{id}@example.test"))
            .bind(now.clone())
    };
    member("before").execute(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE click_connection SET capture=1 WHERE id=1")
        .execute(&db)
        .await
        .unwrap();
    let mut tx = db.begin().await.unwrap();
    member("rolled-back").execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    member("reader").execute(&db).await.unwrap();
    sqlx::query("INSERT INTO subscriptions(id,customer_id,subscription_id,status,created_at) VALUES('subscription','customer','sub','active',?)").bind(&now).execute(&db).await.unwrap();
    for _ in 0..2 {
        sqlx::query(
            "UPDATE subscriptions SET status='canceled',updated_at=? WHERE id='subscription'",
        )
        .bind(&now)
        .execute(&db)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO members_payment_events(id,member_id,amount,currency,created_at) VALUES('payment','reader',2000,'usd',?)").bind(&now).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO members_payment_events(id,member_id,amount,currency,created_at) VALUES('refund','reader',-500,'usd',?)").bind(&now).execute(&db).await.unwrap();
    sqlx::query(
        "INSERT INTO emails(id,post_id,status,created_at) VALUES('email','post','submitted',?)",
    )
    .bind(&now)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO email_recipients(id,email_id,member_id,status,created_at) VALUES('recipient','email','reader','sent',?)").bind(&now).execute(&db).await.unwrap();
    for _ in 0..2 {
        sqlx::query("UPDATE email_recipients SET opened_at=? WHERE id='recipient'")
            .bind(&now)
            .execute(&db)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO posts(id,uuid,title,slug,status,type,created_at) VALUES('post','post','A','post','published','post',?)").bind(&now).execute(&db).await.unwrap();
    sqlx::query("INSERT INTO newsletter_links(id,post_id,from_url,to_url,created_at,updated_at) VALUES('link','post','https://source.test/?secret=private','https://destination.test/?token=private',?,?)").bind(&now).bind(&now).execute(&db).await.unwrap();
    for id in ["click-1", "click-2"] {
        sqlx::query("INSERT INTO newsletter_clicks(id,link_id,member_uuid,created_at) VALUES(?,'link','reader',?)").bind(id).bind(&now).execute(&db).await.unwrap();
    }
    let rows: Vec<(String, Option<i64>, Option<String>)> =
        sqlx::query_as("SELECT kind,amount_minor,currency FROM click_outbox ORDER BY kind")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(
        rows.iter().find(|r| r.0 == "payment").unwrap().1,
        Some(2000)
    );
    assert_eq!(rows.iter().find(|r| r.0 == "refund").unwrap().1, Some(500));
    assert_eq!(
        rows.iter().find(|r| r.0 == "payment").unwrap().2.as_deref(),
        Some("USD")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM click_outbox WHERE kind='newsletter_click'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        1
    );
    // The queue has no email, referrer or URL column; identifiers are hashed
    // by the sender before they can enter Click's accepted event payload.
    sqlx::query("UPDATE click_connection SET capture=0 WHERE id=1")
        .execute(&db)
        .await
        .unwrap();
    member("after").execute(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&db)
            .await
            .unwrap(),
        8
    );
}
