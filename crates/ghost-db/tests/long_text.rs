//! Proof that a long `TEXT` column round-trips on both engines.
//!
//! The whole dual-engine plan for this project rests on this: Ghost's schema
//! keeps post bodies, theme sources and email bodies in `TEXT`, and MySQL
//! reports those as blobs. If [`ghost_db::types::Text`] did not bridge that,
//! every one of those columns would have to become a `VARCHAR` — and a
//! post's HTML is far longer than the ~16,000 characters a readable MySQL
//! `VARCHAR` can hold, so there would be no schema that works.
//!
//! The MySQL half is skipped unless `GHOST_TEST_MYSQL_URL` names a server:
//!
//! ```sh
//! GHOST_TEST_MYSQL_URL='mysql://root:rootpw@127.0.0.1:13306' \
//!   cargo test -p ghost-db --test long_text
//! ```
use ghost_db::types::Text;
use sqlx::any::{install_default_drivers, AnyPoolOptions};
use sqlx::AnyPool;

/// Bigger than anything MySQL will report as text, and realistic: a long post
/// with images is this size.
const BIG: usize = 200_000;

fn body() -> String {
    // Multi-byte on purpose: the decode path goes through `from_utf8_lossy`,
    // and a naive byte-slice anywhere would corrupt this.
    "héllo wörld 🎉 ".repeat(BIG / 16)
}

/// `long_type` is the engine's spelling for "unbounded text".
///
/// SQLite's `TEXT` has no length limit at all. MySQL's `TEXT` is 65,535
/// *bytes* — which a long post exceeds, and multi-byte content exceeds sooner
/// — so the real schema uses `MEDIUMTEXT` there, as upstream Ghost's own knex
/// schema does for `mobiledoc`, `lexical` and `html`.
async fn exercise(pool: &AnyPool, label: &str, long_type: &str) {
    let text = body();

    sqlx::query(&format!("CREATE TABLE posts (id VARCHAR(24) PRIMARY KEY, html {long_type})"))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{label}: create: {e}"));

    sqlx::query("INSERT INTO posts (id, html) VALUES ('1', ?)")
        .bind(&text)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{label}: insert: {e}"));

    // The read that fails without `Text`.
    let (got,): (Text,) = sqlx::query_as("SELECT html FROM posts WHERE id = '1'")
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("{label}: a TEXT column did not decode: {e}"));
    assert_eq!(got.0.len(), text.len(), "{label}: length changed");
    assert_eq!(got.0, text, "{label}: content changed");

    // A plain `String` must still work for a short VARCHAR, or the schema
    // would have to use `Text` for everything and lose the distinction.
    let (id,): (String,) =
        sqlx::query_as("SELECT id FROM posts WHERE id = '1'").fetch_one(pool).await.unwrap();
    assert_eq!(id, "1");

    // NULL has to survive too — most of Ghost's long columns are nullable.
    sqlx::query("INSERT INTO posts (id, html) VALUES ('2', NULL)")
        .execute(pool)
        .await
        .unwrap();
    let (none,): (Option<Text>,) = sqlx::query_as("SELECT html FROM posts WHERE id = '2'")
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("{label}: a NULL TEXT column did not decode: {e}"));
    assert!(none.is_none(), "{label}: NULL came back as a value");
}

/// SQLite, where `TEXT` arrives as text. This is the control.
#[tokio::test]
async fn a_long_text_column_round_trips_on_sqlite() {
    install_default_drivers();
    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    exercise(&pool, "sqlite", "TEXT").await;
}

/// MySQL, where the same column arrives as a blob. This is the case the
/// `Text` type exists for.
#[tokio::test]
async fn a_long_text_column_round_trips_on_mysql() {
    let Some(server) = std::env::var("GHOST_TEST_MYSQL_URL").ok().filter(|s| !s.trim().is_empty())
    else {
        eprintln!("skipping: GHOST_TEST_MYSQL_URL is not set");
        return;
    };
    install_default_drivers();

    let name = format!("ghost_text_{}", ghost_core::ids::new_id());
    let server = server.trim_end_matches('/');
    let admin = AnyPool::connect(&format!("{server}/mysql")).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    ))
    .execute(&admin)
    .await
    .unwrap();

    let pool = AnyPoolOptions::new()
        .max_connections(2)
        .connect(&format!("{server}/{name}"))
        .await
        .unwrap();
    exercise(&pool, "mysql", "MEDIUMTEXT").await;
    pool.close().await;

    sqlx::query(&format!("DROP DATABASE `{name}`")).execute(&admin).await.unwrap();
    admin.close().await;
}
