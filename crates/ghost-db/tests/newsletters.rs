//! The Newsletters repository's reporting half, on SQLite by default and on
//! MySQL when GHOST_TEST_MYSQL_URL is set.
//!
//! Both engines matter here for three reasons, each the shape of thing that
//! works on one and silently does nothing on the other:
//!
//! * The window is a **string** comparison against a timestamp built in Rust,
//!   because `datetime('now', ?)` with a bound modifier is not portable.
//! * The edition reads select around five `MEDIUMTEXT` columns (`html`,
//!   `plaintext`, `source`, `recipient_filter`, `error_data`) that the `Any`
//!   driver cannot decode as a `String` at all. A select list that grew to
//!   include one would pass on SQLite and fail outright on MySQL.
//! * `MAX(created_at)` over an empty table comes back NULL, which is why
//!   `last_sent` is its own scalar rather than a column on `Counts`.
//!
//! What these tests are really about is the counting. A report that counts an
//! unconfirmed subscription as reach, or that loses a failed send because it
//! has no recipient rows, is wrong in a way nobody notices until somebody
//! acts on it.
use ghost_core::dates::to_db_string;
use ghost_db::pool::connect_test;
use ghost_db::repos::newsletters as repo;
use ghost_db::Db;

fn days_ago(n: i64) -> String {
    to_db_string(&(chrono::Utc::now() - chrono::Duration::days(n)))
}

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

async fn member(db: &Db, email: &str, email_disabled: i64) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, status, name, email_disabled, \
         created_at) VALUES (?, ?, ?, ?, 'free', ?, ?, ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(ghost_core::ids::new_id())
    .bind(email)
    .bind(email.split('@').next().unwrap_or("Reader"))
    .bind(email_disabled)
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

async fn post(db: &Db, title: &str) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at) \
         VALUES (?, ?, ?, ?, 'post', 'sent', 'public', ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(title)
    .bind(format!("{}-{}", ghost_core::slug::safe_string(title), &id[..6]))
    .bind(ghost_core::dates::now_db())
    .execute(db)
    .await
    .unwrap();
    id
}

/// An edition — one `emails` row — for `post`, created `days` ago.
async fn edition(
    db: &Db,
    post_id: &str,
    newsletter_id: Option<&str>,
    status: &str,
    subject: &str,
    days: i64,
) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO emails (id, post_id, status, subject, newsletter_id, error, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(post_id)
    .bind(status)
    .bind(subject)
    .bind(newsletter_id)
    .bind(if status == "failed" { Some("The provider refused it") } else { None })
    .bind(days_ago(days))
    .execute(db)
    .await
    .unwrap();
    id
}

/// A recipient row. `opened` writes an `opened_at`, which is what open
/// tracking does.
async fn recipient(db: &Db, email_id: &str, status: &str, opened: bool, days: i64) {
    let when = days_ago(days);
    sqlx::query(
        "INSERT INTO email_recipients (id, email_id, member_id, status, opened_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(email_id)
    .bind(ghost_core::ids::new_id())
    .bind(status)
    .bind(if opened { Some(when.clone()) } else { None })
    .bind(&when)
    .execute(db)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// the programme
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_site_counts_zero_rather_than_failing() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    assert_eq!(repo::programme(&db, &days_ago(30)).await.unwrap(), repo::Counts::default());
    // MAX over an empty table is NULL, which is why this is not a column on
    // Counts — folded in, it would make the whole struct fail to decode.
    assert_eq!(repo::last_sent(&db).await, None);
    assert!(repo::stats(&db).await.unwrap().is_empty());
    assert!(repo::editions(&db, 10, 0).await.unwrap().is_empty());
    assert_eq!(repo::edition_count(&db).await, 0);
    assert_eq!(repo::find_edition(&db, "0123456789abcdef01234567").await.unwrap(), None);
}

#[tokio::test]
async fn reach_counts_only_a_confirmed_subscription_to_somebody_who_can_receive_email() {
    // Both halves of `SUBSCRIBED`, and the reason it is one constant: this
    // admin briefly had two definitions, so one newsletter showed two
    // different subscriber counts on two screens.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let news = newsletter(&db, "Weekly", "active", 0).await;

    let good = member(&db, "good@example.com", 0).await;
    let unconfirmed = member(&db, "pending@example.com", 0).await;
    let bounced = member(&db, "bounced@example.com", 1).await;
    subscribe(&db, &good, &news, "confirmed").await;
    subscribe(&db, &unconfirmed, &news, "pending").await;
    subscribe(&db, &bounced, &news, "confirmed").await;

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.members, 3);
    assert_eq!(c.subscribers, 1, "an unconfirmed or undeliverable row counted as reach");
    assert_eq!(c.subscriptions, 1);
    // And the per-newsletter query agrees with the programme's total, which
    // is the whole point of sharing the fragment.
    let stats = repo::stats(&db).await.unwrap();
    assert_eq!(stats[0].subscribers, 1);
    assert_eq!(repo::subscriber_count(&db, &news).await, 1);
}

#[tokio::test]
async fn a_member_on_two_newsletters_is_one_subscriber_and_two_subscriptions() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let one = newsletter(&db, "Weekly", "active", 0).await;
    let two = newsletter(&db, "Monthly", "active", 1).await;
    let m = member(&db, "both@example.com", 0).await;
    subscribe(&db, &m, &one, "confirmed").await;
    subscribe(&db, &m, &two, "confirmed").await;

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.subscribers, 1, "reach double-counted one person");
    assert_eq!(c.subscriptions, 2);
}

#[tokio::test]
async fn an_archived_newsletter_is_counted_but_not_as_one_a_post_can_go_to() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    newsletter(&db, "Weekly", "active", 0).await;
    newsletter(&db, "Old news", "archived", 1).await;

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.newsletters, 2);
    assert_eq!(c.sending_newsletters, 1);

    // And the list leads with the sending one, which is the order somebody
    // reads it in.
    let stats = repo::stats(&db).await.unwrap();
    assert!(stats[0].is_sending());
    assert!(!stats[1].is_sending());
}

#[tokio::test]
async fn editions_are_counted_by_lifetime_and_by_window_and_failures_separately() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p1 = post(&db, "Recent").await;
    let p2 = post(&db, "Older").await;
    let p3 = post(&db, "Broken").await;
    let p4 = post(&db, "Waiting").await;
    edition(&db, &p1, None, "submitted", "Recent", 2).await;
    edition(&db, &p2, None, "submitted", "Older", 90).await;
    edition(&db, &p3, None, "failed", "Broken", 1).await;
    edition(&db, &p4, None, "pending", "Waiting", 0).await;

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.editions, 4, "the lifetime count should not be windowed");
    assert_eq!(c.editions_in_window, 3);
    assert_eq!(c.failed_editions, 1);
    assert_eq!(c.pending_editions, 1);
}

#[tokio::test]
async fn recipients_are_counted_inside_the_window_only_and_split_by_outcome() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p = post(&db, "The letter").await;
    let e = edition(&db, &p, None, "submitted", "The letter", 1).await;
    recipient(&db, &e, "sent", true, 1).await;
    recipient(&db, &e, "sent", true, 1).await;
    recipient(&db, &e, "sent", false, 1).await;
    recipient(&db, &e, "failed", false, 1).await;
    // Outside the window.
    recipient(&db, &e, "sent", true, 90).await;

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.attempted, 4, "the window leaked");
    assert_eq!(c.delivered, 3);
    assert_eq!(c.opened, 2);
    assert_eq!(c.failed, 1);
}

#[tokio::test]
async fn the_last_send_is_the_newest_one_whatever_order_the_rows_went_in() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p1 = post(&db, "First").await;
    let p2 = post(&db, "Second").await;
    edition(&db, &p1, None, "submitted", "First", 30).await;
    edition(&db, &p2, None, "submitted", "Second", 3).await;

    let last = repo::last_sent(&db).await.expect("nothing reported");
    assert!(last.starts_with(&days_ago(3)[..10]), "{last}");
}

#[tokio::test]
async fn clicks_are_counted_once_per_member_inside_the_window() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p = post(&db, "The letter").await;
    let link = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO newsletter_links (id, post_id, from_url, to_url, created_at, updated_at) \
         VALUES (?, ?, 'https://example.test/a', 'https://example.test/b', ?, ?)",
    )
    .bind(&link)
    .bind(&p)
    .bind(ghost_core::dates::now_db())
    .bind(ghost_core::dates::now_db())
    .execute(&db)
    .await
    .unwrap();
    let who = ghost_core::ids::uuid_v4();
    for (uuid, days) in [(&who, 1), (&who, 2), (&ghost_core::ids::uuid_v4(), 1)] {
        sqlx::query(
            "INSERT INTO newsletter_clicks (id, link_id, member_uuid, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(ghost_core::ids::new_id())
        .bind(&link)
        .bind(uuid)
        .bind(days_ago(days))
        .execute(&db)
        .await
        .unwrap();
    }
    // One click outside the window, by a third person.
    sqlx::query("INSERT INTO newsletter_clicks (id, link_id, member_uuid, created_at) VALUES (?, ?, ?, ?)")
        .bind(ghost_core::ids::new_id())
        .bind(&link)
        .bind(ghost_core::ids::uuid_v4())
        .bind(days_ago(90))
        .execute(&db)
        .await
        .unwrap();

    let c = repo::programme(&db, &days_ago(30)).await.unwrap();
    assert_eq!(c.clicked, 2, "one member clicking twice should count once");
}

// ---------------------------------------------------------------------------
// the editions list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_edition_carries_its_post_its_newsletter_and_its_counts_newest_first() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let news = newsletter(&db, "Weekly", "active", 0).await;
    let old_post = post(&db, "Older").await;
    let new_post = post(&db, "Newer").await;
    let old = edition(&db, &old_post, Some(&news), "submitted", "Older", 10).await;
    let new = edition(&db, &new_post, Some(&news), "submitted", "Newer", 1).await;
    recipient(&db, &new, "sent", true, 1).await;
    recipient(&db, &new, "sent", false, 1).await;
    recipient(&db, &new, "failed", false, 1).await;

    let list = repo::editions(&db, 10, 0).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, new, "the list is not newest-first");
    assert_eq!(list[1].id, old);

    assert_eq!(list[0].post_title.as_deref(), Some("Newer"));
    assert_eq!(list[0].newsletter_name.as_deref(), Some("Weekly"));
    assert_eq!((list[0].attempted, list[0].delivered, list[0].opened, list[0].failed), (3, 2, 1, 1));
    assert_eq!(list[0].title(), "Newer");
    // An edition nobody was sent is a row of zeroes, not a missing row.
    assert_eq!((list[1].attempted, list[1].delivered), (0, 0));
}

#[tokio::test]
async fn a_failed_edition_with_no_recipients_is_still_in_the_list() {
    // The case a `JOIN … GROUP BY` would silently drop — and the one
    // somebody opens this screen to find.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p = post(&db, "Broken").await;
    let e = edition(&db, &p, None, "failed", "Broken", 1).await;

    let list = repo::editions(&db, 10, 0).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, e);
    assert_eq!(list[0].status, "failed");
    assert_eq!(list[0].attempted, 0);
    assert_eq!(list[0].error.as_deref(), Some("The provider refused it"));
    assert!(ghost_core::newsletters::is_retryable(&list[0].status));
}

#[tokio::test]
async fn an_edition_outlives_the_post_it_was_made_from() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p = post(&db, "Deleted later").await;
    edition(&db, &p, None, "submitted", "The subject it was sent under", 1).await;
    sqlx::query("DELETE FROM posts WHERE id = ?").bind(&p).execute(&db).await.unwrap();

    let list = repo::editions(&db, 10, 0).await.unwrap();
    assert_eq!(list.len(), 1, "the record of a send went with the post");
    assert_eq!(list[0].post_title, None);
    // The subject stands in, so the row still has clickable text.
    assert_eq!(list[0].title(), "The subject it was sent under");
}

#[tokio::test]
async fn an_edition_can_be_found_by_id_which_is_what_the_retry_re_reads() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let p = post(&db, "Broken").await;
    let e = edition(&db, &p, None, "failed", "Broken", 1).await;

    let found = repo::find_edition(&db, &e).await.unwrap().expect("not found");
    assert_eq!(found.id, e);
    assert_eq!(found.status, "failed");
    assert_eq!(repo::find_edition(&db, "0123456789abcdef01234567").await.unwrap(), None);
}

#[tokio::test]
async fn the_pages_of_the_list_do_not_overlap_and_the_limit_is_clamped() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let mut ids = Vec::new();
    for i in 0..5 {
        let p = post(&db, &format!("Post {i}")).await;
        // Newest last, so the list comes back in reverse.
        ids.push(edition(&db, &p, None, "submitted", &format!("Post {i}"), 5 - i).await);
    }
    assert_eq!(repo::edition_count(&db).await, 5);

    let first = repo::editions(&db, 2, 0).await.unwrap();
    let second = repo::editions(&db, 2, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(
        first.iter().all(|a| second.iter().all(|b| a.id != b.id)),
        "two pages returned the same edition"
    );
    // Newest first: the last one inserted is 1 day old.
    assert_eq!(first[0].id, ids[4]);

    // Clamped at both ends rather than trusted.
    assert_eq!(repo::editions(&db, 0, 0).await.unwrap().len(), 1);
    assert_eq!(repo::editions(&db, -5, 0).await.unwrap().len(), 1);
    assert_eq!(repo::editions(&db, 10_000, 0).await.unwrap().len(), 5);
    // A negative offset must not wrap round to the end of the table.
    assert_eq!(repo::editions(&db, 2, -10).await.unwrap()[0].id, ids[4]);
}
