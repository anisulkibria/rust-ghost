//! The forms repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
use ghost_core::forms::{default_fields, Answer};
use ghost_db::pool::connect_test;
use ghost_db::repos::forms::{self, Browse, Filed, FormInput, NewSubmission};

fn answers(text: &str) -> Vec<Answer> {
    vec![Answer { key: "f_00000001".into(), label: "Message".into(), kind: "textarea".into(), value: text.into() }]
}

async fn a_form(db: &ghost_db::Db, name: &str) -> String {
    let fields = default_fields();
    forms::create(
        db,
        &FormInput { name, status: "active", fields: &fields, submit_label: "Send", success_message: "Thanks", notify_emails: "", success_action: "message", success_url: "", success_link_label: "" },
    )
    .await
    .unwrap()
}

fn response<'a>(form_id: &'a str, answers: &'a [Answer], nonce: &'a str) -> NewSubmission<'a> {
    NewSubmission { form_id, form_name: "Contact", answers, nonce, ip: Some("203.0.113.7"), user_agent: None, page_path: Some("/contact/"), member_id: None }
}

#[tokio::test]
async fn a_token_files_exactly_one_response() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let form_id = a_form(&db, "Contact").await;
    let body = answers("hello");

    let first = forms::file(&db, &response(&form_id, &body, "nonce-one")).await.unwrap();
    assert!(matches!(first, Filed::Stored(_)));
    // the same nonce again is a replay, and writes nothing
    let second = forms::file(&db, &response(&form_id, &body, "nonce-one")).await.unwrap();
    assert_eq!(second, Filed::Replayed);

    let (rows, total) = forms::browse(&db, &Browse { limit: 50, page: 1, ..Default::default() }).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].answers, body);
    assert_eq!(rows[0].status, "new");
}

#[tokio::test]
async fn lists_filter_by_form_and_status_and_keep_spam_out_by_default() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let contact = a_form(&db, "Contact").await;
    let quote = a_form(&db, "Quote").await;
    let body = answers("hi");
    let mut ids = Vec::new();
    for (form, nonce) in [(&contact, "a"), (&contact, "b"), (&quote, "c")] {
        match forms::file(&db, &response(form, &body, nonce)).await.unwrap() {
            Filed::Stored(id) => ids.push(id),
            Filed::Replayed => panic!("fresh nonce was refused"),
        }
    }
    assert!(forms::set_submission_status(&db, &ids[1], "spam").await.unwrap());
    // a status outside the known three is never written
    assert!(!forms::set_submission_status(&db, &ids[0], "admin").await.unwrap());

    let all = Browse { limit: 50, page: 1, ..Default::default() };
    assert_eq!(forms::browse(&db, &all).await.unwrap().1, 2);
    let spam = Browse { status: Some("spam"), ..all.clone() };
    assert_eq!(forms::browse(&db, &spam).await.unwrap().1, 1);
    let contact_only = Browse { form_id: Some(&contact), ..all.clone() };
    assert_eq!(forms::browse(&db, &contact_only).await.unwrap().1, 1);
    // an injection attempt in the status is not a status: it reads as "all"
    let hostile = Browse { status: Some("new' OR '1'='1"), ..all.clone() };
    assert_eq!(forms::browse(&db, &hostile).await.unwrap().1, 2);

    assert_eq!(forms::totals(&db, None).await.unwrap(), (2, 0, 1));
    let listed = forms::list_with_counts(&db).await.unwrap();
    let contact_row = listed.iter().find(|f| f.form.id == contact).unwrap();
    assert_eq!((contact_row.total, contact_row.unread), (1, 1));

    // opening a spam response does not un-file it
    forms::mark_read_if_new(&db, &ids[1]).await.unwrap();
    assert_eq!(forms::find_submission(&db, &ids[1]).await.unwrap().unwrap().status, "spam");
    forms::mark_read_if_new(&db, &ids[0]).await.unwrap();
    assert_eq!(forms::find_submission(&db, &ids[0]).await.unwrap().unwrap().status, "read");
}

#[tokio::test]
async fn deleting_a_form_takes_its_responses_with_it() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let form_id = a_form(&db, "Contact").await;
    let body = answers("hello");
    forms::file(&db, &response(&form_id, &body, "n1")).await.unwrap();
    assert!(forms::delete(&db, &form_id).await.unwrap());
    assert!(forms::find(&db, &form_id).await.unwrap().is_none());
    assert_eq!(forms::totals(&db, None).await.unwrap(), (0, 0, 0));
    // not an id at all: answered without touching the database
    assert!(forms::find(&db, "../../etc/passwd").await.unwrap().is_none());
}

#[tokio::test]
async fn the_signing_secret_is_created_once_and_is_not_part_of_the_settings() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let first = forms::signing_secret(&db).await.unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(forms::signing_secret(&db).await.unwrap(), first);

    let mut config = forms::config(&db).await.unwrap();
    assert!(config.notify_enabled && config.store_ip);
    config.retention_days = 99_999;
    config.store_ip = false;
    forms::save_config(&db, &config).await.unwrap();
    let saved = forms::config(&db).await.unwrap();
    assert_eq!(saved.retention_days, 3650);
    assert!(!saved.store_ip);
    // saving the settings leaves the secret alone
    assert_eq!(forms::signing_secret(&db).await.unwrap(), first);
}

#[tokio::test]
async fn retention_removes_only_what_is_older_than_the_window() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    let form_id = a_form(&db, "Contact").await;
    let body = answers("hello");
    forms::file(&db, &response(&form_id, &body, "old")).await.unwrap();
    forms::file(&db, &response(&form_id, &body, "fresh")).await.unwrap();
    sqlx::query("UPDATE form_submissions SET created_at = '2001-01-01 00:00:00' WHERE nonce = 'old'").execute(&db).await.unwrap();
    assert_eq!(forms::purge_older_than(&db, 0).await.unwrap(), 0);
    assert_eq!(forms::purge_older_than(&db, 30).await.unwrap(), 1);
    assert_eq!(forms::totals(&db, None).await.unwrap(), (1, 0, 0));
}
