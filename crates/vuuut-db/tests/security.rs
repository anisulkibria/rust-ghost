//! The security repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! Two things here are worth running on both engines rather than only on
//! SQLite. [`expire_sessions`] compares a text date column against a
//! timestamp built in Rust — the repo-layer rule, because
//! `datetime('now', ?)` with a bound modifier does not work on MySQL through
//! the Any driver — and [`lockouts`] does interval arithmetic on the `brute`
//! table's epoch-millisecond `BIGINT`s. Both are the shape of query that
//! works on one engine and silently returns nothing on the other.
use vuuut_db::pool::connect_test;
use vuuut_db::repos::security as repo;
use vuuut_db::Db;

async fn a_user(db: &Db, name: &str, email: &str, status: &str) -> String {
    let id = vuuut_core::ids::new_id();
    sqlx::query(
        "INSERT INTO users (id, name, slug, password, email, status, visibility, created_at) \
         VALUES (?, ?, ?, '', ?, ?, 'public', ?)",
    )
    .bind(&id)
    .bind(name)
    // Unique slug: the seed already ships a user called "Ghost", and a
    // `name.to_lowercase()` slug collides with it.
    .bind(format!("{}-{}", name.to_lowercase(), &id[..6]))
    .bind(email)
    .bind(status)
    .bind(vuuut_core::dates::now_db())
    .execute(db)
    .await
    .unwrap();
    id
}

async fn with_role(db: &Db, user: &str, role: &str) {
    let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_one(db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
        .bind(vuuut_core::ids::new_id())
        .bind(&role_id)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
}

/// Back-date a session so the policy can be tested without sleeping.
async fn age(db: &Db, sid: &str, created_secs_ago: i64, used_secs_ago: Option<i64>) {
    let at = |secs: i64| {
        vuuut_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::seconds(secs)))
    };
    sqlx::query("UPDATE sessions SET created_at = ?, updated_at = ? WHERE session_id = ?")
        .bind(at(created_secs_ago))
        .bind(used_secs_ago.map(at))
        .bind(sid)
        .execute(db)
        .await
        .unwrap();
}

async fn seeded() -> vuuut_db::TestDb {
    let test = connect_test().await.unwrap();
    vuuut_db::seed::seed_if_empty(&test.pool()).await.unwrap();
    test
}

#[tokio::test]
async fn the_session_list_names_the_person_their_role_and_their_status() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    with_role(&db, &ada, "Editor").await;
    let sid = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();

    let list = repo::sessions(&db, 50).await.unwrap();
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(s.session_id, sid);
    assert_eq!(s.user_name, "Ada");
    assert_eq!(s.user_email, "ada@example.test");
    assert_eq!(s.role, "Editor");
    assert_eq!(s.user_status, "active");
    assert!(s.verified);
    assert_eq!(s.short().len(), 6);
}

#[tokio::test]
async fn a_session_whose_user_was_deleted_is_not_listed_and_can_be_swept() {
    // `sessions.user_id` has no foreign key — it predates this port — so
    // deleting a staff member leaves their sessions behind. The list drops
    // them because "who is signed in" is not answered by a row that resolves
    // to nobody; the sweep removes them.
    let test = seeded().await;
    let db = test.pool();
    let ghost = a_user(&db, "Ghost", "ghost@example.test", "active").await;
    vuuut_db::repos::sessions::create(&db, &ghost).await.unwrap();
    sqlx::query("DELETE FROM users WHERE id = ?").bind(&ghost).execute(&db).await.unwrap();

    assert!(repo::sessions(&db, 50).await.unwrap().is_empty());
    let raw: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions").fetch_one(&db).await.unwrap();
    assert_eq!(raw, 1, "the row is still there, merely hidden");

    assert_eq!(repo::expire_orphan_sessions(&db).await.unwrap(), 1);
    let raw: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions").fetch_one(&db).await.unwrap();
    assert_eq!(raw, 0);
}

#[tokio::test]
async fn an_unverified_session_is_listed_as_awaiting_its_code() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    vuuut_db::repos::sessions::create_unverified(&db, &ada).await.unwrap();
    let list = repo::sessions(&db, 50).await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(!list[0].verified, "a pending sign-in reads as a completed one");
}

#[tokio::test]
async fn the_maximum_age_expires_a_session_and_leaves_a_younger_one() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let old = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    let fresh = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    age(&db, &old, 8 * 86_400, Some(8 * 86_400)).await;

    // Seven days, no idle limit.
    assert_eq!(repo::expire_sessions(&db, 7 * 86_400, 0).await.unwrap(), 1);
    let left: Vec<String> =
        repo::sessions(&db, 50).await.unwrap().into_iter().map(|s| s.session_id).collect();
    assert_eq!(left, vec![fresh]);
}

#[tokio::test]
async fn the_idle_limit_expires_an_unused_session_inside_its_maximum_age() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let idle = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    // Created a day ago, last used two hours ago: well inside thirty days.
    age(&db, &idle, 86_400, Some(2 * 3600)).await;
    assert_eq!(repo::expire_sessions(&db, 30 * 86_400, 3600).await.unwrap(), 1);
    assert!(repo::sessions(&db, 50).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_session_with_no_updated_at_is_measured_from_when_it_was_created() {
    // `updated_at` is written only when the session data changes, so an
    // ordinary session has none for its whole life. Reading null as "never
    // used" would expire every session on the site the moment an idle timeout
    // was switched on — which is the bug that locks every administrator out
    // at once, so it is worth a test of its own on both engines.
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let sid = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    age(&db, &sid, 60, None).await;

    // A minute old, with a fifteen-minute idle limit: survives.
    assert_eq!(repo::expire_sessions(&db, 30 * 86_400, 15 * 60).await.unwrap(), 0);
    assert_eq!(repo::sessions(&db, 50).await.unwrap().len(), 1);

    // An hour old, same limit: goes, measured from `created_at`.
    age(&db, &sid, 3600, None).await;
    assert_eq!(repo::expire_sessions(&db, 30 * 86_400, 15 * 60).await.unwrap(), 1);
}

#[tokio::test]
async fn a_zero_limit_expires_nothing() {
    // `0` is how "no idle timeout" is stored, and a maximum age of zero is
    // what a corrupted row reads as. Neither may be read as "expire
    // everything", which is the failure mode that signs out the whole site.
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let sid = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    age(&db, &sid, 365 * 86_400, Some(365 * 86_400)).await;
    assert_eq!(repo::expire_sessions(&db, 0, 0).await.unwrap(), 0);
    assert_eq!(repo::sessions(&db, 50).await.unwrap().len(), 1);
}

#[tokio::test]
async fn revoking_for_one_user_spares_the_session_it_is_told_to_keep() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let bob = a_user(&db, "Bob", "bob@example.test", "active").await;
    let keep = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    let bobs = vuuut_db::repos::sessions::create(&db, &bob).await.unwrap();

    assert_eq!(repo::revoke_sessions_for_user(&db, &ada, Some(&keep)).await.unwrap(), 2);
    let left: Vec<String> =
        repo::sessions(&db, 50).await.unwrap().into_iter().map(|s| s.session_id).collect();
    assert_eq!(left.len(), 2);
    assert!(left.contains(&keep));
    assert!(left.contains(&bobs), "another user's session was revoked");
}

#[tokio::test]
async fn revoking_everything_spares_only_the_caller() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let bob = a_user(&db, "Bob", "bob@example.test", "active").await;
    let mine = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &bob).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &bob).await.unwrap();

    assert_eq!(repo::revoke_all_sessions(&db, Some(&mine)).await.unwrap(), 2);
    let left = repo::sessions(&db, 50).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].session_id, mine);
}

#[tokio::test]
async fn a_suspended_account_holding_a_session_is_counted() {
    // The one number the report expects to be zero. Suspending now revokes,
    // so anything here is a suspension that happened before that was true or
    // a bug — which is exactly why it is counted rather than assumed.
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    assert_eq!(repo::suspended_with_sessions(&db).await.unwrap(), 0);

    sqlx::query("UPDATE users SET status = 'inactive' WHERE id = ?")
        .bind(&ada)
        .execute(&db)
        .await
        .unwrap();
    // Counted per *account*, not per session: "one suspended account still
    // has access" is the fact, and two sessions do not make it two facts.
    assert_eq!(repo::suspended_with_sessions(&db).await.unwrap(), 1);
}

#[tokio::test]
async fn stale_sessions_are_counted_against_the_policy_they_are_stale_under() {
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    let sid = vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    age(&db, &sid, 10 * 86_400, Some(10 * 86_400)).await;

    assert_eq!(repo::stale_session_count(&db, 7 * 86_400).await.unwrap(), 1);
    assert_eq!(repo::stale_session_count(&db, 30 * 86_400).await.unwrap(), 0);
    // No policy, nothing stale.
    assert_eq!(repo::stale_session_count(&db, 0).await.unwrap(), 0);
}

// ---------------------------------------------------------------------------
// lockouts
// ---------------------------------------------------------------------------

async fn a_lockout(db: &Db, key: &str, count: i64, last_secs_ago: i64, lifetime: i64) {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO brute (`key`, firstRequest, lastRequest, lifetime, count) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(key)
    .bind(now - last_secs_ago * 1000)
    .bind(now - last_secs_ago * 1000)
    .bind(lifetime)
    .bind(count)
    .execute(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn the_lockout_list_shows_live_records_busiest_first_and_hides_expired_ones() {
    let test = seeded().await;
    let db = test.pool();
    a_lockout(&db, "user_login:203.0.113.1", 3, 60, 3600).await;
    a_lockout(&db, "user_login:203.0.113.2", 40, 60, 3600).await;
    // Last touched two hours ago on a one-hour lifetime: expired, and not a
    // lockout — just a row the sweep has not reached.
    a_lockout(&db, "user_login:203.0.113.3", 99, 7200, 3600).await;

    let list = repo::lockouts(&db, 50).await.unwrap();
    assert_eq!(list.len(), 2, "an expired record was listed as a lockout");
    assert_eq!(list[0].key, "user_login:203.0.113.2", "not ordered busiest first");
    assert_eq!(list[0].kind(), "Staff sign-in");

    // `count > 4` is the smallest free allowance any authentication budget
    // has, so this is an upper bound on "addresses being made to wait".
    assert_eq!(repo::lockout_count(&db).await.unwrap(), 1);
}

#[tokio::test]
async fn clearing_a_lockout_removes_exactly_one_and_clearing_all_removes_the_rest() {
    let test = seeded().await;
    let db = test.pool();
    a_lockout(&db, "user_login:203.0.113.1", 9, 60, 3600).await;
    a_lockout(&db, "someone@example.com reset", 9, 60, 3600).await;

    assert!(repo::clear_lockout(&db, "user_login:203.0.113.1").await.unwrap());
    // A key that is not there is not an error — it had already expired.
    assert!(!repo::clear_lockout(&db, "user_login:203.0.113.1").await.unwrap());
    assert_eq!(repo::lockouts(&db, 50).await.unwrap().len(), 1);

    assert_eq!(repo::clear_all_lockouts(&db).await.unwrap(), 1);
    assert!(repo::lockouts(&db, 50).await.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// the counts behind the posture
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_counts_measure_what_the_posture_is_judged_on() {
    let test = seeded().await;
    let db = test.pool();
    let owner = a_user(&db, "Owner", "owner@example.test", "active").await;
    with_role(&db, &owner, "Owner").await;
    let admin = a_user(&db, "Admin", "admin@example.test", "active").await;
    with_role(&db, &admin, "Administrator").await;
    let editor = a_user(&db, "Editor", "editor@example.test", "active").await;
    with_role(&db, &editor, "Editor").await;
    // A suspended administrator does not count towards "who can change
    // settings": they cannot sign in.
    let gone = a_user(&db, "Gone", "gone@example.test", "inactive").await;
    with_role(&db, &gone, "Administrator").await;

    vuuut_db::repos::sessions::create(&db, &owner).await.unwrap();
    vuuut_db::repos::sessions::create(&db, &editor).await.unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis();
    for (email, expires) in
        [("open@example.test", now_ms + 86_400_000), ("stale@example.test", now_ms - 86_400_000)]
    {
        let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Author'")
            .fetch_one(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO invites (id, role_id, token, expires, status, email, created_at) \
             VALUES (?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(vuuut_core::ids::new_id())
        .bind(&role_id)
        .bind(vuuut_core::ids::random_hex(16))
        .bind(expires)
        .bind(email)
        .bind(vuuut_core::dates::now_db())
        .execute(&db)
        .await
        .unwrap();
    }

    let counts = repo::counts(&db, 7 * 86_400).await.unwrap();
    assert_eq!(counts.admins, 2, "a suspended administrator was counted");
    assert_eq!(counts.sessions, 2);
    assert_eq!(counts.open_invites, 1);
    assert_eq!(counts.expired_invites, 1);
    assert_eq!(counts.suspended_with_sessions, 0);
    assert_eq!(counts.stale_sessions, 0);
    // The seed ships the built-in integrations, each with an Admin API key,
    // so this is non-zero on any real site and the report notes it.
    assert!(counts.admin_api_keys > 0, "the seeded integrations' keys were not counted");
}

#[tokio::test]
async fn the_activity_list_shows_only_the_events_that_change_access() {
    // The `actions` table is the whole site's audit log. A security screen
    // that scrolls past forty post edits to reach one role change is a screen
    // nobody reads, so the resource types are an allowlist.
    let test = seeded().await;
    let db = test.pool();
    let actor = a_user(&db, "Ada", "ada@example.test", "active").await;
    for (kind, event) in [
        ("user", "added"),
        ("invite", "added"),
        ("api_key", "edited"),
        ("post", "published"),
        ("tag", "added"),
    ] {
        vuuut_db::repos::actions::record(&db, kind, &vuuut_core::ids::new_id(), &actor, event)
            .await
            .unwrap();
    }

    let events = repo::events(&db, 50).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.resource_type.as_str()).collect();
    assert!(kinds.contains(&"user"));
    assert!(kinds.contains(&"invite"));
    assert!(kinds.contains(&"api_key"));
    assert!(!kinds.contains(&"post"), "a post edit is on the security activity list");
    assert!(!kinds.contains(&"tag"));
    assert_eq!(events[0].actor_name.as_deref(), Some("Ada"));
}

#[tokio::test]
async fn every_list_is_bounded_however_much_is_asked_for() {
    // A site with ten thousand lockouts has a problem the report should name
    // as a number, not paint as a table.
    let test = seeded().await;
    let db = test.pool();
    let ada = a_user(&db, "Ada", "ada@example.test", "active").await;
    for _ in 0..3 {
        vuuut_db::repos::sessions::create(&db, &ada).await.unwrap();
    }
    // Asking for more than the ceiling gets the ceiling, not an error; asking
    // for zero or a negative gets one row rather than none, because a limit
    // of zero in a `LIMIT ?` is a list that is silently always empty.
    assert_eq!(repo::sessions(&db, i64::MAX).await.unwrap().len(), 3);
    assert_eq!(repo::sessions(&db, 0).await.unwrap().len(), 1);
    assert_eq!(repo::sessions(&db, -5).await.unwrap().len(), 1);
}
