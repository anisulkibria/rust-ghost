//! The revision queries behind Post history, on SQLite by default and on
//! MySQL when GHOST_TEST_MYSQL_URL is set.
//!
//! The ordering is the thing worth testing on both engines. `created_at_ts`
//! is milliseconds and the id breaks a same-millisecond tie — which is not
//! decoration: a test that saves three times in a row does it inside one
//! millisecond, and so does a real editor autosaving. An order that fell back
//! to SQLite's `rowid` would work there and silently scramble on MySQL.
use ghost_db::pool::connect_test;
use ghost_db::repos::{posts, publishing};

async fn a_post(db: &ghost_db::Db, title: &str) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
         VALUES (?, ?, ?, 'a-post', '<p>one</p>', 'one', 'post', 'published', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(title)
    .execute(db)
    .await
    .unwrap();
    id
}

/// One save by a person pressing Save — which is an *explicit* save in
/// Ghost's terms, so it always earns a version when something changed. The
/// autosave path is exercised separately, in
/// `a_run_of_autosaves_does_not_fill_the_history`.
async fn edit(db: &ghost_db::Db, id: &str, title: &str, editor: Option<&str>) {
    posts::update(
        db,
        id,
        &posts::PostChanges {
            title: Some(title.into()),
            html: Some(format!("<p>{title}</p>")),
            editor_id: editor.map(str::to_string),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn every_save_keeps_the_state_it_replaced_newest_first() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;

    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), 0);
    edit(&db, &id, "Second", None).await;
    edit(&db, &id, "Third", None).await;
    edit(&db, &id, "Fourth", None).await;

    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), 3);
    let listed = publishing::revisions(&db, &id, 100).await.unwrap();
    // Each snapshot is of the state being *replaced*, so the newest revision
    // holds the title from just before the last edit. That is what makes the
    // list restorable rather than a log of what is already there.
    let titles: Vec<&str> = listed.iter().map(|r| r.title.as_deref().unwrap_or("")).collect();
    assert_eq!(titles, ["Third", "Second", "First"], "newest first, each the state replaced");
}

#[tokio::test]
async fn the_limit_is_honoured_and_bounded() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;
    for n in 0..5 {
        edit(&db, &id, &format!("Edit {n}"), None).await;
    }
    assert_eq!(publishing::revisions(&db, &id, 2).await.unwrap().len(), 2);
    // Nonsense limits are clamped rather than passed through to the query.
    assert_eq!(publishing::revisions(&db, &id, 0).await.unwrap().len(), 1);
    assert_eq!(publishing::revisions(&db, &id, i64::MAX).await.unwrap().len(), 5);
}

#[tokio::test]
async fn a_revision_records_who_made_the_edit_and_tolerates_not_knowing() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Ada Lovelace", "ada@example.test", "unused").await.unwrap();
    let id = a_post(&db, "First").await;

    edit(&db, &id, "Second", Some(&owner.id)).await;
    // The scheduler has no human behind it, so it passes none.
    edit(&db, &id, "Third", None).await;

    let listed = publishing::revisions(&db, &id, 100).await.unwrap();
    assert_eq!(listed[0].author_name, None, "an edit with no actor has no author");
    assert_eq!(listed[1].author_name.as_deref(), Some("Ada Lovelace"));
}

#[tokio::test]
async fn a_snapshot_is_only_readable_through_the_post_it_belongs_to() {
    // The revision id arrives from a URL. Scoping by post as well is what
    // stops a revision of somebody else's post being read by guessing one.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let mine = a_post(&db, "Mine").await;
    edit(&db, &mine, "Mine, edited", None).await;
    let revision = publishing::revisions(&db, &mine, 1).await.unwrap()[0].id.clone();

    let theirs = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at) \
         VALUES (?, ?, 'Theirs', 'theirs', 'post', 'published', 'public', '2026-11-01 00:00:00')",
    )
    .bind(&theirs)
    .bind(ghost_core::ids::uuid_v4())
    .execute(&db)
    .await
    .unwrap();

    assert!(publishing::revision_snapshot(&db, &mine, &revision).await.unwrap().is_some());
    assert!(
        publishing::revision_snapshot(&db, &theirs, &revision).await.unwrap().is_none(),
        "a revision was readable through the wrong post"
    );
    // And restoring through the wrong post is refused, not just unreadable.
    assert!(publishing::restore(&db, &theirs, &revision).await.is_err());
}

#[tokio::test]
async fn a_snapshot_carries_the_tags_and_authors_as_well_as_the_text() {
    // This is why history is worth having at all: Ghost's own revisions keep
    // the whole entry, so a restore is not just the body coming back.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Ada", "ada@example.test", "unused").await.unwrap();
    let id = a_post(&db, "First").await;

    let tag = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at) VALUES (?, 'Engineering', 'engineering', 'public', '2026-11-01 00:00:00')")
        .bind(&tag)
        .execute(&db)
        .await
        .unwrap();
    posts::update(
        &db,
        &id,
        &posts::PostChanges {
            tag_ids: Some(vec![tag.clone()]),
            author_ids: Some(vec![owner.id.clone()]),
            meta_description: Some("A description".into()),
            editor_id: Some(owner.id.clone()),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // A content change while the entry is tagged, so the version it banks is
    // the tagged one. Tags and SEO fields are not among the four Ghost
    // watches, so tagging alone would not have banked anything — which is
    // the behaviour `a_save_that_changes_nothing_a_reader_would_notice...`
    // pins from the other side.
    edit(&db, &id, "Second", Some(&owner.id)).await;
    // Now strip all of it away.
    posts::update(
        &db,
        &id,
        &posts::PostChanges {
            tag_ids: Some(vec![]),
            clearing: vec!["meta_description"],
            editor_id: Some(owner.id.clone()),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let revision = publishing::revisions(&db, &id, 1).await.unwrap()[0].id.clone();
    let snapshot = publishing::revision_snapshot(&db, &id, &revision).await.unwrap().unwrap();
    assert_eq!(snapshot.tag_ids, vec![tag.clone()]);
    assert_eq!(snapshot.author_ids, vec![owner.id.clone()]);
    assert_eq!(
        snapshot.post.meta.as_ref().and_then(|m| m.meta_description.clone()).as_deref(),
        Some("A description")
    );

    // And restoring puts them back.
    publishing::restore(&db, &id, &revision).await.unwrap();
    let tags: Vec<String> = sqlx::query_scalar("SELECT tag_id FROM posts_tags WHERE post_id = ?")
        .bind(&id)
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(tags, vec![tag]);
    let restored = posts::find_by_id(&db, &id).await.unwrap().unwrap();
    assert_eq!(
        restored.meta.and_then(|m| m.meta_description).as_deref(),
        Some("A description")
    );
}

// ---------------------------------------------------------------------------
// retention — Ghost's two numbers, against a real database
// ---------------------------------------------------------------------------

async fn save(db: &ghost_db::Db, id: &str, title: &str, explicit: bool) {
    posts::update(
        db,
        id,
        &posts::PostChanges {
            title: Some(title.into()),
            lexical: Some(format!(r#"{{"body":"{title}"}}"#)),
            explicit_save: explicit,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_post_keeps_exactly_the_number_of_versions_ghost_keeps() {
    // POST_REVISIONS_COUNT = 25, in core/server/models/post.js.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;

    for n in 0..40 {
        save(&db, &id, &format!("Edit {n}"), true).await;
    }
    let kept = publishing::revision_count(&db, &id).await.unwrap();
    assert_eq!(kept, ghost_core::revisions::MAX_REVISIONS, "retention is not Ghost's 25");
    assert_eq!(kept, 25);

    // The oldest are the ones dropped, so the newest 25 are what survived.
    let listed = publishing::revisions(&db, &id, 100).await.unwrap();
    assert_eq!(listed.len(), 25);
    assert_eq!(listed[0].title.as_deref(), Some("Edit 38"), "newest kept");
    assert_eq!(listed[24].title.as_deref(), Some("Edit 14"), "oldest kept");
}

#[tokio::test]
async fn a_run_of_autosaves_does_not_fill_the_history() {
    // The reason the cap needs the interval beside it: the editor autosaves
    // three seconds after typing stops, so twenty-five autosaves is about a
    // minute of work. Without this rule, one editing session would evict
    // every earlier version of the post.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;

    // The first save always keeps one (`initial_revision`).
    save(&db, &id, "Autosave 0", false).await;
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), 1);

    // Thirty more autosaves inside the ten-minute window add none.
    for n in 1..30 {
        save(&db, &id, &format!("Autosave {n}"), false).await;
    }
    assert_eq!(
        publishing::revision_count(&db, &id).await.unwrap(),
        1,
        "autosaves inside the interval must fold into the version already kept"
    );

    // An explicit save — Save, or cmd-S — always keeps one.
    save(&db, &id, "Asked for", true).await;
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), 2);

    // And once the interval has passed, the next background save keeps one.
    sqlx::query("UPDATE post_revisions SET created_at_ts = created_at_ts - ? WHERE post_id = ?")
        .bind(ghost_core::revisions::INTERVAL_MS + 1000)
        .bind(&id)
        .execute(&db)
        .await
        .unwrap();
    save(&db, &id, "Much later", false).await;
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), 3);
}

#[tokio::test]
async fn a_save_that_changes_nothing_a_reader_would_notice_keeps_no_version() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;
    save(&db, &id, "Settled", true).await;
    let before = publishing::revision_count(&db, &id).await.unwrap();

    // Re-saving the same title and body, explicitly, several times.
    for _ in 0..5 {
        save(&db, &id, "Settled", true).await;
    }
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), before);

    // An SEO field is not one of the four Ghost watches, so it does not
    // start a version of its own either — though the snapshot does carry it.
    posts::update(
        &db,
        &id,
        &posts::PostChanges {
            meta_description: Some("A description".into()),
            explicit_save: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), before);
}

#[tokio::test]
async fn publishing_and_unpublishing_always_keep_a_version() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = a_post(&db, "First").await;
    // Start from a draft with a version already banked, so "initial" is not
    // what is being observed.
    sqlx::query("UPDATE posts SET status = 'draft' WHERE id = ?").bind(&id).execute(&db).await.unwrap();
    save(&db, &id, "Draft text", true).await;
    let before = publishing::revision_count(&db, &id).await.unwrap();

    // Publishing, with no change to the text at all.
    posts::update(
        &db,
        &id,
        &posts::PostChanges { status: Some("published".into()), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), before + 1);

    // And pulling it back to a draft.
    posts::update(
        &db,
        &id,
        &posts::PostChanges { status: Some("draft".into()), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(publishing::revision_count(&db, &id).await.unwrap(), before + 2);

    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT reason FROM post_revisions WHERE post_id = ? ORDER BY created_at_ts DESC, id DESC")
            .bind(&id)
            .fetch_all(&db)
            .await
            .unwrap();
    // Ghost's own strings, so a database read by either implementation says
    // the same thing about why each version exists.
    assert_eq!(reasons[0], "unpublished");
    assert_eq!(reasons[1], "published");
    assert!(reasons.contains(&"initial_revision".to_string()), "{reasons:?}");
}

#[tokio::test]
async fn pruning_only_touches_the_post_being_saved() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let mine = a_post(&db, "Mine").await;
    let theirs = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, html, type, status, visibility, created_at) \
         VALUES (?, ?, 'Theirs', 'theirs', '<p>x</p>', 'post', 'published', 'public', '2026-11-01 00:00:00')",
    )
    .bind(&theirs)
    .bind(ghost_core::ids::uuid_v4())
    .execute(&db)
    .await
    .unwrap();

    save(&db, &theirs, "Theirs, edited", true).await;
    let theirs_before = publishing::revision_count(&db, &theirs).await.unwrap();

    // Push one post well past the cap.
    for n in 0..40 {
        save(&db, &mine, &format!("Edit {n}"), true).await;
    }
    assert_eq!(publishing::revision_count(&db, &mine).await.unwrap(), 25);
    assert_eq!(
        publishing::revision_count(&db, &theirs).await.unwrap(),
        theirs_before,
        "pruning one post's history removed another post's"
    );
}
