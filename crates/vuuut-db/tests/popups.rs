//! The popups repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! The round trip below exists because of a real bug: `surface` was added to
//! the UPDATE's column list and not to its bind list, which left 22
//! placeholders against 21 values. Every column after `audience` then took
//! its neighbour's value, the `WHERE id = ?` had nothing left to bind, and
//! **every save silently changed nothing** while the screen said "saved".
//! Nothing in the type system catches that; a save-then-read does.

use vuuut_db::pool::connect_test;
use vuuut_db::repos::popups::{self, PopupInput};

fn input(name: &str, status: &str) -> PopupInput {
    PopupInput {
        name: name.into(),
        status: status.into(),
        kind: "modal".into(),
        heading: "Heading".into(),
        body: "Body".into(),
        button_label: "Subscribe".into(),
        button_url: String::new(),
        success_message: "Thanks".into(),
        capture_email: 1,
        capture_name: 0,
        trigger_kind: "delay".into(),
        trigger_value: 5,
        frequency: "once".into(),
        frequency_days: 0,
        audience: "logged_out".into(),
        surface: "any".into(),
        targeting: "everywhere".into(),
        paths: String::new(),
        starts_at: None,
        ends_at: None,
    }
}

#[tokio::test]
async fn a_popup_survives_a_save_with_every_field_changed() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = popups::insert(&db, &input("First", "draft")).await.unwrap();

    let stored = popups::find(&db, &id).await.unwrap().expect("inserted popup");
    assert_eq!(stored.name, "First");
    assert_eq!(stored.status, "draft");
    assert_eq!(stored.surface, "any");
    assert_eq!(stored.capture_email, 1);
    assert_eq!(stored.impressions, 0);

    // Every field different from the insert, so a shifted bind shows up as a
    // value in the wrong column rather than as a value that happens to match.
    let mut changed = input("Second", "live");
    changed.kind = "bar".into();
    changed.heading = "New heading".into();
    changed.body = "New body".into();
    changed.button_label = "Join".into();
    changed.button_url = "/about/".into();
    changed.success_message = "Done".into();
    changed.capture_email = 0;
    changed.capture_name = 1;
    changed.trigger_kind = "scroll".into();
    changed.trigger_value = 40;
    changed.frequency = "days".into();
    changed.frequency_days = 14;
    changed.audience = "members".into();
    changed.surface = "posts".into();
    changed.targeting = "only".into();
    changed.paths = "/tag/news".into();
    changed.starts_at = Some("2026-01-01 00:00:00".into());
    changed.ends_at = Some("2026-12-31 00:00:00".into());
    popups::update(&db, &id, &changed).await.unwrap();

    let after = popups::find(&db, &id).await.unwrap().expect("popup after update");
    assert_eq!(after.name, "Second");
    assert_eq!(after.status, "live", "publishing has to survive the save");
    assert_eq!(after.kind, "bar");
    assert_eq!(after.heading, "New heading");
    assert_eq!(after.body, "New body");
    assert_eq!(after.button_label, "Join");
    assert_eq!(after.button_url, "/about/");
    assert_eq!(after.success_message, "Done");
    assert_eq!(after.capture_email, 0);
    assert_eq!(after.capture_name, 1);
    assert_eq!(after.trigger_kind, "scroll");
    assert_eq!(after.trigger_value, 40);
    assert_eq!(after.frequency, "days");
    assert_eq!(after.frequency_days, 14);
    assert_eq!(after.audience, "members");
    assert_eq!(after.surface, "posts");
    assert_eq!(after.targeting, "only");
    assert_eq!(after.paths, "/tag/news");
    assert_eq!(after.starts_at.as_deref(), Some("2026-01-01 00:00:00"));
    assert_eq!(after.ends_at.as_deref(), Some("2026-12-31 00:00:00"));
    // The counters belong to the public endpoint and a save must not touch
    // them.
    assert_eq!(after.impressions, 0);
    assert_eq!(after.submissions, 0);
}

#[tokio::test]
async fn only_live_popups_are_served_and_status_can_be_flipped() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let draft = popups::insert(&db, &input("Draft one", "draft")).await.unwrap();
    let live = popups::insert(&db, &input("Live one", "live")).await.unwrap();

    assert!(popups::any_live(&db).await);
    let serving: Vec<String> = popups::live(&db).await.unwrap().into_iter().map(|p| p.id).collect();
    assert!(serving.contains(&live) && !serving.contains(&draft));

    popups::set_status(&db, &draft, "live").await.unwrap();
    popups::set_status(&db, &live, "draft").await.unwrap();
    let serving: Vec<String> = popups::live(&db).await.unwrap().into_iter().map(|p| p.id).collect();
    assert!(serving.contains(&draft) && !serving.contains(&live));
}

#[tokio::test]
async fn a_lead_counts_once_and_goes_with_its_popup() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let id = popups::insert(&db, &input("Capture", "live")).await.unwrap();

    popups::record_impression(&db, &id).await;
    popups::record_impression(&db, &id).await;
    assert!(!popups::lead_exists(&db, &id, "reader@example.com").await);

    popups::record_lead(&db, &id, "reader@example.com", "Reader", "/about/", Some("member-1"))
        .await
        .unwrap();
    assert!(popups::lead_exists(&db, &id, "reader@example.com").await);
    // ...and not for a different popup, which is why the index is on the pair.
    let other = popups::insert(&db, &input("Other", "live")).await.unwrap();
    assert!(!popups::lead_exists(&db, &other, "reader@example.com").await);

    let counted = popups::find(&db, &id).await.unwrap().unwrap();
    assert_eq!(counted.impressions, 2);
    assert_eq!(counted.submissions, 1);

    let leads = popups::leads(&db, &id, 50).await.unwrap();
    assert_eq!(leads.len(), 1);
    assert_eq!(leads[0].email, "reader@example.com");
    assert_eq!(leads[0].member_id.as_deref(), Some("member-1"));

    // Deleting the popup takes its leads with it.
    popups::destroy(&db, &id).await.unwrap();
    assert!(popups::find(&db, &id).await.unwrap().is_none());
    assert!(popups::leads(&db, &id, 50).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_kind_of_a_path_is_worked_out_without_the_router() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    assert_eq!(popups::kind_of_path(&db, "/").await, "home");
    assert_eq!(popups::kind_of_path(&db, "").await, "home");
    assert_eq!(popups::kind_of_path(&db, "/tag/news/").await, "tag");
    assert_eq!(popups::kind_of_path(&db, "/author/ada/").await, "author");
    // Nothing in the database by that slug.
    assert_eq!(popups::kind_of_path(&db, "/nothing-here/").await, "other");
}
