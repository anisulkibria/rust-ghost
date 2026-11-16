//! The firewall repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! Two things here need both engines rather than just SQLite. Expiry is a
//! **string** comparison against a timestamp built in Rust — the repo-layer
//! rule, because `datetime('now', ?)` with a bound modifier does not work
//! through the Any driver — and the upsert is a delete-then-insert against a
//! unique index that is declared differently in the two schemas. Both are the
//! shape of thing that works on one engine and silently does nothing on the
//! other.
use ghost_core::firewall::{Action, Pattern, Scope};
use ghost_db::pool::connect_test;
use ghost_db::repos::firewall::{self as repo, NewRule};
use ghost_db::Db;

async fn add(db: &Db, action: Action, scope: Scope, pattern: &str, expires: Option<String>) -> String {
    repo::upsert(
        db,
        &NewRule {
            action,
            scope,
            pattern: Pattern::parse(pattern).unwrap(),
            note: "",
            created_by: None,
            expires_at: expires,
        },
    )
    .await
    .unwrap()
}

fn in_hours(h: i64) -> String {
    ghost_core::dates::to_db_string(&(chrono::Utc::now() + chrono::Duration::hours(h)))
}

fn hours_ago(h: i64) -> String {
    ghost_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::hours(h)))
}

#[tokio::test]
async fn a_rule_is_stored_normalized_so_one_range_is_one_rule() {
    // `10.1.2.3/8` and `10.0.0.0/8` match identically. Stored as typed they
    // would be two rows the unique index cannot tell apart, and two rules in
    // the list that cannot both be right.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Site, "10.1.2.3/8", None).await;
    let all = repo::all(&db).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].pattern, "10.0.0.0/8");

    // The same range in a different spelling replaces it rather than adding.
    add(&db, Action::Block, Scope::Site, "10.255.255.255/8", None).await;
    assert_eq!(repo::all(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn changing_your_mind_about_a_range_replaces_the_rule_rather_than_contradicting_it() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Admin, "203.0.113.0/24", None).await;
    add(&db, Action::Allow, Scope::Admin, "203.0.113.0/24", None).await;

    let all = repo::all(&db).await.unwrap();
    assert_eq!(all.len(), 1, "two contradictory rules for one range");
    assert_eq!(all[0].action, "allow");
}

#[tokio::test]
async fn the_same_address_can_be_barred_from_the_admin_and_allowed_on_the_site() {
    // Which is why the unique index is on (scope, pattern) and not on
    // pattern alone.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Admin, "203.0.113.4", None).await;
    add(&db, Action::Allow, Scope::Site, "203.0.113.4", None).await;
    assert_eq!(repo::all(&db).await.unwrap().len(), 2);
}

#[tokio::test]
async fn an_expired_rule_is_not_live_and_is_swept() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Site, "203.0.113.4", Some(hours_ago(1))).await;
    add(&db, Action::Block, Scope::Site, "198.51.100.4", Some(in_hours(1))).await;
    add(&db, Action::Block, Scope::Site, "192.0.2.4", None).await;

    // `all` shows everything, so the screen can show an expired rule and say
    // it is expired.
    assert_eq!(repo::all(&db).await.unwrap().len(), 3);
    // `live` is what the middleware reads, and the expired one is not in it.
    let live = repo::live(&db).await.unwrap();
    assert_eq!(live.len(), 2);
    assert!(!live.iter().any(|r| r.pattern.to_string() == "203.0.113.4"));

    assert_eq!(repo::sweep_expired(&db).await.unwrap(), 1);
    assert_eq!(repo::all(&db).await.unwrap().len(), 2);
    // Twice is not an error and removes nothing more.
    assert_eq!(repo::sweep_expired(&db).await.unwrap(), 0);
}

#[tokio::test]
async fn a_rule_with_no_expiry_is_never_swept() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Site, "203.0.113.4", None).await;
    assert_eq!(repo::sweep_expired(&db).await.unwrap(), 0);
    assert_eq!(repo::live(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_row_whose_pattern_cannot_be_read_is_left_out_of_the_live_rules() {
    // Only reachable by editing the table by hand — the screen will not
    // store one — but a row like this must not become "matches everything".
    let test = connect_test().await.unwrap();
    let db = test.pool();
    sqlx::query(
        "INSERT INTO firewall_rules (id, action, scope, pattern, note, created_at, hits) \
         VALUES (?, 'block', 'site', 'not-an-address', '', ?, 0)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(ghost_core::dates::now_db())
    .execute(&db)
    .await
    .unwrap();
    add(&db, Action::Block, Scope::Site, "203.0.113.4", None).await;

    assert!(repo::live(&db).await.unwrap().len() == 1, "an unreadable rule reached the matcher");
    // And it is still on the screen, so the thing that is not working is
    // visible rather than merely absent.
    let all = repo::all(&db).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all.iter().filter(|r| !r.is_valid()).count(), 1);
}

#[tokio::test]
async fn a_hit_is_counted_against_the_rule_that_refused_somebody() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = add(&db, Action::Block, Scope::Site, "203.0.113.4", None).await;
    assert_eq!(repo::find(&db, &id).await.unwrap().unwrap().hits, 0);

    repo::record_hit(&db, &id).await;
    repo::record_hit(&db, &id).await;
    let found = repo::find(&db, &id).await.unwrap().unwrap();
    assert_eq!(found.hits, 2);
    assert!(found.last_hit_at.is_some());

    // A hit against a rule that no longer exists is not an error: the rule
    // may have been removed between the decision and the write.
    repo::record_hit(&db, "0123456789abcdef01234567").await;
}

#[tokio::test]
async fn rules_are_listed_allow_first_which_is_the_order_they_are_evaluated_in() {
    // So a reader working out why an address gets through sees the allow
    // that let it before the block that would have stopped it.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    add(&db, Action::Block, Scope::Site, "203.0.113.0/24", None).await;
    add(&db, Action::Allow, Scope::Site, "203.0.113.4", None).await;
    let all = repo::all(&db).await.unwrap();
    assert_eq!(all[0].action, "allow");
    assert_eq!(all[1].action, "block");
}

#[tokio::test]
async fn removing_one_rule_and_removing_them_all() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let one = add(&db, Action::Block, Scope::Site, "203.0.113.4", None).await;
    add(&db, Action::Block, Scope::Site, "198.51.100.4", None).await;

    assert!(repo::delete(&db, &one).await.unwrap());
    assert!(!repo::delete(&db, &one).await.unwrap(), "a second delete reported a removal");
    assert_eq!(repo::count(&db).await.unwrap(), 1);

    assert_eq!(repo::delete_all(&db).await.unwrap(), 1);
    assert_eq!(repo::count(&db).await.unwrap(), 0);
    assert!(repo::live(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_cache_reflects_the_table_after_a_reload_and_not_before() {
    // The honest trade stated in the repo's module doc: the cache is
    // reloaded on every write from the admin screen, and a rule written
    // straight into the database waits for a restart.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let cache = repo::RuleCache::load(&db).await.unwrap();
    assert!(cache.is_empty());

    add(&db, Action::Block, Scope::Site, "203.0.113.4", None).await;
    assert!(cache.is_empty(), "the cache changed without a reload");

    cache.reload(&db).await;
    assert_eq!(cache.rules().len(), 1);
    assert!(!cache.is_empty());

    repo::delete_all(&db).await.unwrap();
    cache.reload(&db).await;
    assert!(cache.is_empty());
}
