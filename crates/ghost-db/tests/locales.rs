//! Translation groups, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! The invariant under test is the one the whole module rests on: **one
//! language per group**. If two German posts could end up in one group, "the
//! German version of this post" stops having an answer and the editor's
//! switcher has to pick one arbitrarily. Every refusal below is there to keep
//! that from happening, and the merge cases are where it is easiest to break
//! by accident.
//!
//! The second thing worth testing is that a NULL locale and the default
//! language code are treated as the same language. They are the same
//! language, and the migration deliberately never backfills NULL — so every
//! query that compares locales has to handle both spellings or a site's
//! existing posts quietly stop matching anything.
use ghost_core::locales::Locales;
use ghost_db::pool::connect_test;
use ghost_db::repos::locales as repo;

async fn entry(
    db: &ghost_db::Db,
    slug: &str,
    title: &str,
    locale: Option<&str>,
    kind: &str,
) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, locale, created_at, updated_at, published_at) \
         VALUES (?, ?, ?, ?, '<p>x</p>', 'x', ?, 'published', 'public', ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(title)
    .bind(slug)
    .bind(kind)
    .bind(locale)
    .execute(db)
    .await
    .unwrap();
    id
}

async fn group_of(db: &ghost_db::Db, id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT translation_group FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn linking_two_entries_makes_each_reachable_from_the_other() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    // The English one has a NULL locale, which is what every existing post
    // on an upgraded site looks like.
    let en = entry(&db, "hello", "Hello", None, "post").await;
    let de = entry(&db, "hallo", "Hallo", Some("de"), "post").await;

    let group = repo::link(&db, &en, &de, "en").await.unwrap().expect("the link is accepted");
    assert_eq!(group_of(&db, &en).await.as_deref(), Some(group.as_str()));
    assert_eq!(group_of(&db, &de).await.as_deref(), Some(group.as_str()));

    // Each side sees the other, and only the other.
    let from_en = repo::siblings(&db, &en).await.unwrap();
    assert_eq!(from_en.len(), 1);
    assert_eq!(from_en[0].id, de);
    assert_eq!(from_en[0].locale.as_deref(), Some("de"));
    let from_de = repo::siblings(&db, &de).await.unwrap();
    assert_eq!(from_de.len(), 1);
    assert_eq!(from_de[0].id, en);

    // And the lookup the front end uses to answer "this page, in German".
    let found = repo::in_locale(&db, &group, "de", "en").await.unwrap().expect("German exists");
    assert_eq!(found.id, de);
    // A NULL locale answers to the default code, which is the whole reason
    // the migration does not backfill it.
    let back = repo::in_locale(&db, &group, "en", "en").await.unwrap().expect("English exists");
    assert_eq!(back.id, en);
    assert!(repo::in_locale(&db, &group, "fr", "en").await.unwrap().is_none());
}

#[tokio::test]
async fn one_language_per_group_is_enforced_rather_than_resolved() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let en = entry(&db, "hello", "Hello", None, "post").await;
    let de = entry(&db, "hallo", "Hallo", Some("de"), "post").await;
    let de2 = entry(&db, "hallo-again", "Hallo nochmal", Some("de"), "post").await;
    repo::link(&db, &en, &de, "en").await.unwrap().unwrap();

    // A second German one cannot join: "the German version" would stop
    // having an answer.
    let refused = repo::link(&db, &en, &de2, "en").await.unwrap().unwrap_err();
    assert_eq!(refused, repo::LinkRefusal::Occupied("de".into()));
    assert_eq!(group_of(&db, &de2).await, None, "nothing was written");

    // Two entries in the same language are refused outright, including when
    // one of them spells that language as NULL.
    let en2 = entry(&db, "hello-again", "Hello again", Some("en"), "post").await;
    assert_eq!(
        repo::link(&db, &en, &en2, "en").await.unwrap().unwrap_err(),
        repo::LinkRefusal::SameLocale("en".into()),
        "NULL and the default code are the same language"
    );

    // And the obvious degenerate cases.
    assert_eq!(repo::link(&db, &en, &en, "en").await.unwrap().unwrap_err(), repo::LinkRefusal::Same);
    assert_eq!(
        repo::link(&db, &en, "6553f7a1b2c3d4e5f6a7b8c9", "en").await.unwrap().unwrap_err(),
        repo::LinkRefusal::Missing
    );
}

#[tokio::test]
async fn a_post_and_a_page_are_not_translations_of_each_other() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let post = entry(&db, "hello", "Hello", None, "post").await;
    let page = entry(&db, "hallo", "Hallo", Some("de"), "page").await;
    assert_eq!(
        repo::link(&db, &post, &page, "en").await.unwrap().unwrap_err(),
        repo::LinkRefusal::KindMismatch
    );
}

#[tokio::test]
async fn linking_a_third_language_joins_the_group_rather_than_splitting_it() {
    // Linking A→B then B→C must leave one group of three. Minting a fresh id
    // on the second link would leave two groups that disagree about what the
    // set is, and the switcher would show different options depending on
    // which entry you opened.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let en = entry(&db, "hello", "Hello", None, "post").await;
    let de = entry(&db, "hallo", "Hallo", Some("de"), "post").await;
    let fr = entry(&db, "bonjour", "Bonjour", Some("fr"), "post").await;

    let first = repo::link(&db, &en, &de, "en").await.unwrap().unwrap();
    let second = repo::link(&db, &de, &fr, "en").await.unwrap().unwrap();
    assert_eq!(first, second, "the existing group was reused");
    for id in [&en, &de, &fr] {
        assert_eq!(group_of(&db, id).await.as_deref(), Some(first.as_str()));
    }
    assert_eq!(repo::siblings(&db, &en).await.unwrap().len(), 2);

    // Merging two groups that each already hold a language keeps the
    // invariant: a fourth entry in a language already present is refused.
    let de2 = entry(&db, "hallo-2", "Hallo 2", Some("de"), "post").await;
    assert_eq!(
        repo::link(&db, &fr, &de2, "en").await.unwrap().unwrap_err(),
        repo::LinkRefusal::Occupied("de".into())
    );
}

#[tokio::test]
async fn merging_two_existing_groups_moves_every_member() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let en = entry(&db, "a", "A", None, "post").await;
    let de = entry(&db, "b", "B", Some("de"), "post").await;
    let fr = entry(&db, "c", "C", Some("fr"), "post").await;
    let es = entry(&db, "d", "D", Some("es"), "post").await;
    // Two separate pairs...
    repo::link(&db, &en, &de, "en").await.unwrap().unwrap();
    repo::link(&db, &fr, &es, "en").await.unwrap().unwrap();
    // ...joined into one set of four.
    let merged = repo::link(&db, &de, &fr, "en").await.unwrap().unwrap();
    for id in [&en, &de, &fr, &es] {
        assert_eq!(group_of(&db, id).await.as_deref(), Some(merged.as_str()), "member left behind");
    }
    assert_eq!(repo::siblings(&db, &es).await.unwrap().len(), 3);
}

#[tokio::test]
async fn unlinking_leaves_the_others_grouped() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let en = entry(&db, "hello", "Hello", None, "post").await;
    let de = entry(&db, "hallo", "Hallo", Some("de"), "post").await;
    let fr = entry(&db, "bonjour", "Bonjour", Some("fr"), "post").await;
    let group = repo::link(&db, &en, &de, "en").await.unwrap().unwrap();
    repo::link(&db, &de, &fr, "en").await.unwrap().unwrap();

    repo::unlink(&db, &de).await.unwrap();
    assert_eq!(group_of(&db, &de).await, None);
    assert!(repo::siblings(&db, &de).await.unwrap().is_empty());
    // The other two are untouched, and the slot German vacated is free.
    assert_eq!(group_of(&db, &en).await.as_deref(), Some(group.as_str()));
    assert_eq!(repo::siblings(&db, &en).await.unwrap().len(), 1);
    let de2 = entry(&db, "hallo-2", "Hallo 2", Some("de"), "post").await;
    assert!(repo::link(&db, &en, &de2, "en").await.unwrap().is_ok(), "the slot was not freed");
}

#[tokio::test]
async fn an_entry_with_no_group_gets_one_minted_once() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let en = entry(&db, "hello", "Hello", None, "post").await;
    assert_eq!(group_of(&db, &en).await, None);
    let first = repo::group_for(&db, &en).await.unwrap();
    let second = repo::group_for(&db, &en).await.unwrap();
    assert_eq!(first, second, "a second call must not mint a new group");
    assert_eq!(group_of(&db, &en).await.as_deref(), Some(first.as_str()));
}

#[tokio::test]
async fn coverage_counts_each_language_and_reports_null_as_the_default() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    entry(&db, "a", "A", None, "post").await;
    entry(&db, "b", "B", Some("en"), "post").await;
    entry(&db, "c", "C", Some("de"), "post").await;
    entry(&db, "d", "D", Some("de"), "page").await;
    let draft = entry(&db, "e", "E", Some("de"), "post").await;
    sqlx::query("UPDATE posts SET status = 'draft' WHERE id = ?").bind(&draft).execute(&db).await.unwrap();

    let coverage: std::collections::HashMap<String, repo::Coverage> =
        repo::coverage(&db, "en").await.unwrap().into_iter().collect();
    // The NULL-locale post and the explicitly-English one are both English.
    assert_eq!(coverage["en"].posts, 2);
    assert_eq!(coverage["de"].posts, 1);
    assert_eq!(coverage["de"].pages, 1);
    assert_eq!(coverage["de"].drafts, 1);
}

#[tokio::test]
async fn untranslated_lists_what_a_language_is_missing() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let translated = entry(&db, "hello", "Hello", None, "post").await;
    let de = entry(&db, "hallo", "Hallo", Some("de"), "post").await;
    repo::link(&db, &translated, &de, "en").await.unwrap().unwrap();
    let alone = entry(&db, "lonely", "Lonely", None, "post").await;
    let draft = entry(&db, "draft", "Draft", None, "post").await;
    sqlx::query("UPDATE posts SET status = 'draft' WHERE id = ?").bind(&draft).execute(&db).await.unwrap();
    // A German post is not "missing a German translation".
    entry(&db, "nur-deutsch", "Nur Deutsch", Some("de"), "post").await;

    let missing = repo::untranslated(&db, "de", "en", 100).await.unwrap();
    let ids: Vec<&str> = missing.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&alone.as_str()), "the untranslated post is missing: {ids:?}");
    assert!(!ids.contains(&translated.as_str()), "the translated post should not be listed");
    assert!(!ids.contains(&draft.as_str()), "a draft is not yet missing anything");
    assert_eq!(ids.len(), 1, "{ids:?}");
}

#[tokio::test]
async fn a_slug_shadowed_by_a_language_prefix_is_found() {
    // `/de/` is the German home page, so a post slugged `de` cannot be
    // opened at all. Reported rather than left to be noticed as a 404.
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let shadowed = entry(&db, "de", "About German", None, "post").await;
    entry(&db, "deutsch", "Deutsch", None, "post").await;
    entry(&db, "en", "About English", None, "post").await;

    let found = repo::shadowed_slugs(&db, &Locales::new("en", &["de", "fr"])).await.unwrap();
    let ids: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [shadowed.as_str()], "only the shadowed slug: {ids:?}");

    // Nothing is shadowed on a single-language site, so enabling the module
    // cannot retroactively break a slug.
    assert!(repo::shadowed_slugs(&db, &Locales::default()).await.unwrap().is_empty());
}
