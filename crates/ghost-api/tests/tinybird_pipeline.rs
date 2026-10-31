//! The Tinybird sync against a real HTTP receiver.
//!
//! The unit tests cover the query shape and the chunking; this covers the
//! part that only shows up end to end — that the watermark advances, that a
//! second run ships nothing, that a new row is picked up, and that the
//! safety lag holds back rows too fresh to trust.
use std::sync::{Arc, Mutex};

/// A stand-in for the ingestion endpoint that records what it was sent.
async fn receiver(received: Arc<Mutex<Vec<String>>>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/events",
        axum::routing::post(move |body: String| {
            let received = received.clone();
            async move {
                for line in body.lines().filter(|l| !l.trim().is_empty()) {
                    received.lock().unwrap().push(line.to_string());
                }
                axum::http::StatusCode::OK
            }
        }),
    );
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://127.0.0.1:{port}/events"), handle)
}

/// The parent rows the foreign keys require.
async fn seed_automations(pool: &sqlx::SqlitePool) {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for id in ["a1", "a2"] {
        sqlx::query(
            "INSERT INTO automations (id, status, name, description, created_at) VALUES (?,?,?,?,?)",
        )
        .bind(id).bind("paused").bind(id).bind("")
        .bind(&now)
        .execute(pool).await.unwrap();
    }
}

#[tokio::test]
async fn the_sync_is_incremental_resumable_and_lagged() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let (endpoint, server) = receiver(received.clone()).await;

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    seed_automations(&pool).await;

    let config = ghost_api::tinybird_sync::Config {
        endpoint,
        token: "writer".into(),
        site_uuid: "site-1".into(),
    };
    let target = &ghost_api::tinybird_sync::TARGETS[0];

    // Three rows old enough to be past the safety lag, and one too fresh.
    let old = |mins: i64| {
        (chrono::Utc::now() - chrono::Duration::minutes(mins))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    for (id, when) in [("r1", old(30)), ("r2", old(20)), ("r3", old(10))] {
        sqlx::query(
            "INSERT INTO automation_runs (id, automation_id, member_email, created_at, updated_at) VALUES (?,?,?,?,?)",
        )
        .bind(id).bind("a1").bind("m@b.test").bind(&when).bind(&when)
        .execute(&pool).await.unwrap();
    }
    let fresh = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query("INSERT INTO automation_runs (id, automation_id, member_email, created_at, updated_at) VALUES (?,?,?,?,?)")
        .bind("r4-too-fresh").bind("a1").bind("m@b.test").bind(&fresh).bind(&fresh)
        .execute(&pool).await.unwrap();

    // --- first run -------------------------------------------------------
    let sent = ghost_api::tinybird_sync::sync_table(&pool, &config, target).await.unwrap();
    assert_eq!(sent, 3, "the row inside the safety lag is held back");
    let lines = received.lock().unwrap().clone();
    assert_eq!(lines.len(), 3);
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(first["type"], "automation_runs");
    assert_eq!(first["id"], "r1");
    assert_eq!(first["site_uuid"], "site-1");
    assert_eq!(first["payload"]["automation_id"], "a1");
    assert!(
        !lines.iter().any(|l| l.contains("r4-too-fresh")),
        "a row whose transaction may still be open must not move the watermark past it"
    );

    // --- the watermark was recorded --------------------------------------
    let mark: (String, String) = sqlx::query_as(
        "SELECT last_synced_id, last_synced_updated_at FROM tinybird_syncs WHERE table_name='automation_runs'",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(mark.0, "r3");

    // --- second run ships nothing ----------------------------------------
    received.lock().unwrap().clear();
    let sent = ghost_api::tinybird_sync::sync_table(&pool, &config, target).await.unwrap();
    assert_eq!(sent, 0, "nothing new, so nothing resent");
    assert!(received.lock().unwrap().is_empty());

    // --- a new old-enough row is picked up -------------------------------
    let when = old(5);
    sqlx::query("INSERT INTO automation_runs (id, automation_id, member_email, created_at, updated_at) VALUES (?,?,?,?,?)")
        .bind("r5").bind("a2").bind("m@b.test").bind(&when).bind(&when)
        .execute(&pool).await.unwrap();
    let sent = ghost_api::tinybird_sync::sync_table(&pool, &config, target).await.unwrap();
    assert_eq!(sent, 1);
    let lines = received.lock().unwrap().clone();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("r5"));

    server.abort();
}

#[tokio::test]
async fn two_rows_sharing_a_timestamp_are_both_shipped_exactly_once() {
    // The tie-break on id. Without it one of these is skipped forever or
    // both are resent on every run.
    let received = Arc::new(Mutex::new(Vec::new()));
    let (endpoint, server) = receiver(received.clone()).await;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    seed_automations(&pool).await;

    let when = (chrono::Utc::now() - chrono::Duration::minutes(10))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    for id in ["same-a", "same-b", "same-c"] {
        sqlx::query("INSERT INTO automation_runs (id, automation_id, member_email, created_at, updated_at) VALUES (?,?,?,?,?)")
            .bind(id).bind("a1").bind("m@b.test").bind(&when).bind(&when)
            .execute(&pool).await.unwrap();
    }
    let config = ghost_api::tinybird_sync::Config {
        endpoint, token: "writer".into(), site_uuid: "s".into(),
    };
    let target = &ghost_api::tinybird_sync::TARGETS[0];

    assert_eq!(ghost_api::tinybird_sync::sync_table(&pool, &config, target).await.unwrap(), 3);
    assert_eq!(received.lock().unwrap().len(), 3);
    // And a second run resends none of them.
    assert_eq!(ghost_api::tinybird_sync::sync_table(&pool, &config, target).await.unwrap(), 0);
    assert_eq!(received.lock().unwrap().len(), 3);

    server.abort();
}

#[tokio::test]
async fn a_refused_batch_does_not_advance_the_watermark() {
    // At-least-once: the next run has to resend what was not accepted.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/events",
        axum::routing::post(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let server = tokio::spawn(async move { let _ = axum::serve(listener, app).await; });

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    seed_automations(&pool).await;
    let when = (chrono::Utc::now() - chrono::Duration::minutes(10))
        .format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query("INSERT INTO automation_runs (id, automation_id, member_email, created_at, updated_at) VALUES (?,?,?,?,?)")
        .bind("r1").bind("a1").bind("m@b.test").bind(&when).bind(&when)
        .execute(&pool).await.unwrap();

    let config = ghost_api::tinybird_sync::Config {
        endpoint: format!("http://127.0.0.1:{port}/events"),
        token: "writer".into(),
        site_uuid: "s".into(),
    };
    let result = ghost_api::tinybird_sync::sync_table(
        &pool, &config, &ghost_api::tinybird_sync::TARGETS[0],
    ).await;
    assert!(result.is_err(), "a refused POST is an error, not a silent success");

    let marks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tinybird_syncs")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(marks, 0, "nothing was confirmed delivered, so nothing was marked");

    server.abort();
}
