//! The media repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! The library's contents come from the filesystem, so what is checked here is
//! the half the database owns: the words put beside a file, and which posts
//! point at one.
use ghost_core::media::{parse_location, Location};
use ghost_db::pool::connect_test;
use ghost_db::repos::media::{self as repo, MetaInput};

fn at(path: &str) -> Location {
    parse_location(path).expect("a valid stored path")
}

fn words<'a>(title: &'a str, alt: &'a str, caption: &'a str) -> MetaInput<'a> {
    MetaInput { title, alt, caption }
}

#[tokio::test]
async fn words_put_beside_a_file_are_saved_updated_and_read_back() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let photo = at("images/2026/11/harbour-ab12.jpg");

    // A file nobody has described has no row, not an empty one.
    assert_eq!(repo::find(&db, &photo).await.unwrap(), None);

    repo::save(&db, &photo, &words("Harbour", "Boats at dawn", "Taken from the east pier")).await.unwrap();
    let saved = repo::find(&db, &photo).await.unwrap().expect("a row");
    assert_eq!((saved.title.as_str(), saved.alt.as_str()), ("Harbour", "Boats at dawn"));
    assert_eq!(saved.caption, "Taken from the east pier");
    assert!(!saved.created_at.is_empty() && !saved.updated_at.is_empty());

    // Saving again updates in place rather than failing or duplicating.
    repo::save(&db, &photo, &words("The harbour", "Boats at dawn", "")).await.unwrap();
    let updated = repo::find(&db, &photo).await.unwrap().expect("still a row");
    assert_eq!(updated.title, "The harbour");
    assert_eq!(updated.caption, "");
    assert_eq!(updated.created_at, saved.created_at, "the first save is when it was created");

    // Clearing every field removes the row, so "has anything been written
    // about this file" has one answer and not two.
    repo::save(&db, &photo, &words("", "", "")).await.unwrap();
    assert_eq!(repo::find(&db, &photo).await.unwrap(), None);
}

#[tokio::test]
async fn every_description_in_the_library_is_read_in_one_query() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    repo::save(&db, &at("images/2026/11/a.jpg"), &words("A", "", "")).await.unwrap();
    repo::save(&db, &at("media/2026/11/b.mp4"), &words("B", "", "")).await.unwrap();
    repo::save(&db, &at("files/c.pdf"), &words("C", "", "")).await.unwrap();

    let all = repo::all(&db).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all["images/2026/11/a.jpg"].title, "A");
    assert_eq!(all["media/2026/11/b.mp4"].title, "B");
    assert_eq!(all["files/c.pdf"].title, "C");
}

#[tokio::test]
async fn a_rename_takes_the_description_with_it_and_leaves_nothing_behind() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let from = at("images/2026/11/dsc-0041-ab12.jpg");
    let to = at("images/2026/11/harbour-at-dawn-ab12.jpg");
    repo::save(&db, &from, &words("Harbour", "Boats at dawn", "East pier")).await.unwrap();

    repo::rename(&db, &from, &to).await.unwrap();
    assert_eq!(repo::find(&db, &from).await.unwrap(), None);
    let moved = repo::find(&db, &to).await.unwrap().expect("the row moved");
    assert_eq!((moved.title.as_str(), moved.alt.as_str(), moved.caption.as_str()), ("Harbour", "Boats at dawn", "East pier"));

    // A row left over at the destination by a file that was deleted does not
    // survive to be adopted by the file arriving there.
    let third = at("images/2026/11/stale-cd34.jpg");
    repo::save(&db, &third, &words("Stale", "", "")).await.unwrap();
    repo::rename(&db, &to, &third).await.unwrap();
    assert_eq!(repo::find(&db, &third).await.unwrap().unwrap().title, "Harbour");

    // Renaming a file nobody described is not an error; it just has nothing
    // to carry over.
    let plain = at("files/untouched.pdf");
    repo::rename(&db, &plain, &at("files/renamed.pdf")).await.unwrap();
    assert_eq!(repo::find(&db, &at("files/renamed.pdf")).await.unwrap(), None);
}

#[tokio::test]
async fn deleting_a_file_forgets_what_was_written_about_it() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let doc = at("files/2026/11/terms-ab12.pdf");
    repo::save(&db, &doc, &words("Terms", "", "")).await.unwrap();
    repo::delete(&db, &doc).await.unwrap();
    assert_eq!(repo::find(&db, &doc).await.unwrap(), None);
    // and deleting what is not there is a no-op, not a failure — a second
    // submit of the same form must not 500
    repo::delete(&db, &doc).await.unwrap();
}

/// A post, written straight in: the usage query reads `html`, `lexical` and
/// `feature_image`, and this test is about the reading.
async fn a_post(db: &ghost_db::Db, id: &str, title: &str, html: &str, lexical: &str, feature_image: Option<&str>) {
    sqlx::query(
        "INSERT INTO posts (id, title, slug, html, lexical, feature_image, type, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'post', 'published', '2026-11-14 10:00:00', '2026-11-14 10:00:00')",
    )
    .bind(id)
    .bind(title)
    .bind(title.to_lowercase().replace(' ', "-"))
    .bind(html)
    .bind(lexical)
    .bind(feature_image)
    .execute(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_file_knows_which_posts_point_at_it_however_the_url_was_stored() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let url = "/content/images/2026/11/harbour-ab12.jpg";

    // in rendered HTML
    a_post(&db, "000000000000000000000001", "In the body", &format!(r#"<img src="{url}">"#), "", None).await;
    // in the editor's own document, where a URL's slashes arrive escaped
    let escaped = url.replace('/', "\\/");
    a_post(&db, "000000000000000000000002", "In the editor", "", &format!(r#"{{"src":"{escaped}"}}"#), None).await;
    // as the feature image, which is stored as a plain column
    a_post(&db, "000000000000000000000003", "As the cover", "", "", Some(url)).await;
    // and a post that merely mentions a similar name
    a_post(&db, "000000000000000000000004", "Unrelated", r#"<img src="/content/images/2026/11/other.jpg">"#, "", None).await;

    let (used, total) = repo::usage(&db, url, 10).await.unwrap();
    assert_eq!(total, 3, "three posts use it: {used:?}");
    let mut titles: Vec<&str> = used.iter().map(|u| u.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["As the cover", "In the body", "In the editor"]);
    assert!(used.iter().all(|u| u.post_type == "post" && u.status == "published"));

    // A file nothing points at reports nothing.
    let (none, zero) = repo::usage(&db, "/content/images/2026/11/never-used.jpg", 10).await.unwrap();
    assert!(none.is_empty() && zero == 0);

    // The list is capped while the count stays true, which is what lets the
    // delete warning say "and 2 more".
    let (one, still_three) = repo::usage(&db, url, 1).await.unwrap();
    assert_eq!((one.len(), still_three), (1, 3));
}

#[tokio::test]
async fn a_wildcard_in_a_file_name_is_matched_literally() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    // `_` matches any single character in LIKE. Without escaping, a post
    // using `plan-a.pdf` would be reported as using `plan_a.pdf`.
    a_post(
        &db,
        "000000000000000000000011",
        "Decoy",
        r#"<a href="/content/files/plan-a.pdf">plan</a>"#,
        "",
        None,
    )
    .await;
    let (used, total) = repo::usage(&db, "/content/files/plan_a.pdf", 10).await.unwrap();
    assert!(used.is_empty() && total == 0, "`_` was treated as a wildcard: {used:?}");

    // …and the real file is still found.
    a_post(
        &db,
        "000000000000000000000012",
        "Real",
        r#"<a href="/content/files/plan_a.pdf">plan</a>"#,
        "",
        None,
    )
    .await;
    let (used, total) = repo::usage(&db, "/content/files/plan_a.pdf", 10).await.unwrap();
    assert_eq!((used.len(), total), (1, 1));
    assert_eq!(used[0].title, "Real");
}
