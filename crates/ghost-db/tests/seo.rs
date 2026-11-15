//! The SEO audit's one query, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! What is worth testing here is not the SQL's shape but the joins: the
//! report's two most common findings are "no meta description" and "no tags",
//! and both are read through a `LEFT JOIN` and a subquery that would be
//! trivially easy to write as an inner join — which would hide every entry
//! that has no `posts_meta` row at all, i.e. exactly the ones the report
//! exists to list.
use ghost_db::pool::connect_test;
use ghost_db::repos::seo;

async fn entry(
    db: &ghost_db::Db,
    slug: &str,
    title: &str,
    status: &str,
    plaintext: &str,
) -> String {
    let id = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
         VALUES (?, ?, ?, ?, '<p>x</p>', ?, 'post', ?, 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(title)
    .bind(slug)
    .bind(plaintext)
    .bind(status)
    .execute(db)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn an_entry_with_no_meta_row_is_still_reported_on() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    entry(&db, "bare", "A bare post", "published", "one two three").await;

    let entries = seo::entries(&db, 100).await.unwrap();
    let bare = entries.iter().find(|e| e.slug == "bare").expect("the entry is listed");
    assert_eq!(bare.title, "A bare post");
    assert_eq!(bare.meta_description, None);
    assert_eq!(bare.tag_count, 0);
    assert_eq!(bare.word_count, 3);
    assert_eq!(bare.kind, "post");
    // and the audit finds the two things the report is mostly about
    let findings = ghost_core::seo::audit(bare);
    let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
    assert!(codes.contains(&"no_description"), "{codes:?}");
    assert!(codes.contains(&"no_tags"), "{codes:?}");
}

#[tokio::test]
async fn the_meta_row_and_the_tag_count_are_read_when_they_exist() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = entry(&db, "described", "A described post", "published", "one two").await;
    sqlx::query(
        "INSERT INTO posts_meta (id, post_id, meta_title, meta_description, feature_image_alt, og_image) \
         VALUES (?, ?, 'SEO title', 'A description', 'Alt text', '/content/images/og.png')",
    )
    .bind(ghost_core::ids::new_id())
    .bind(&id)
    .execute(&db)
    .await
    .unwrap();
    let tag_id = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at) VALUES (?, 'Engineering', 'engineering', 'public', '2026-11-01 00:00:00')")
        .bind(&tag_id)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, 0)")
        .bind(ghost_core::ids::new_id())
        .bind(&id)
        .bind(&tag_id)
        .execute(&db)
        .await
        .unwrap();

    let entries = seo::entries(&db, 100).await.unwrap();
    let e = entries.iter().find(|e| e.slug == "described").unwrap();
    assert_eq!(e.effective_title(), "SEO title");
    assert_eq!(e.effective_description(), Some("A description"));
    assert_eq!(e.feature_image_alt.as_deref(), Some("Alt text"));
    assert_eq!(e.og_image.as_deref(), Some("/content/images/og.png"));
    assert_eq!(e.tag_count, 1);
    assert!(e.has_image());
}

#[tokio::test]
async fn the_audit_is_bounded_and_says_how_much_there_was() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    for i in 0..5 {
        entry(&db, &format!("p{i}"), &format!("Post {i}"), "published", "words here").await;
    }
    assert_eq!(seo::total(&db).await.unwrap(), 5);
    assert_eq!(seo::entries(&db, 2).await.unwrap().len(), 2);
    // A limit beyond the ceiling is clamped rather than honoured, and a
    // nonsensical one still returns something.
    assert_eq!(seo::entries(&db, i64::MAX).await.unwrap().len(), 5);
    assert_eq!(seo::entries(&db, 0).await.unwrap().len(), 1);
}
