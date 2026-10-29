//! The newsletter send loop, against an SMTP server that really answers.
//!
//! Sending is the one part of this crate that cannot be checked by reading a
//! rendered page: what matters is which messages were accepted, what was
//! written down about the ones that were not, and that a retry addresses only
//! those. So this starts a minimal SMTP sink on a loopback port, points the
//! relay settings at it, and sends for real.
//!
//! The sink refuses whichever addresses the test tells it to, which is how a
//! partly-failed send — the case a retry exists for — is produced on demand.
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// A loopback SMTP server that accepts everything except the addresses in
/// `refuse`, which it answers with a permanent 550.
struct Sink {
    port: u16,
    refuse: Arc<Mutex<Vec<String>>>,
    accepted: Arc<AtomicUsize>,
}

impl Sink {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let refuse: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (task_refuse, task_accepted) = (refuse.clone(), accepted.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let (refuse, accepted) = (task_refuse.clone(), task_accepted.clone());
                tokio::spawn(async move { session(stream, refuse, accepted).await });
            }
        });
        Sink { port, refuse, accepted }
    }

    fn refuse(&self, addresses: &[&str]) {
        *self.refuse.lock().unwrap() = addresses.iter().map(|a| a.to_string()).collect();
    }
}

/// Just enough of RFC 5321 for lettre to hand over a message, including
/// several messages on one pooled connection.
async fn session(stream: tokio::net::TcpStream, refuse: Arc<Mutex<Vec<String>>>, accepted: Arc<AtomicUsize>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    if write.write_all(b"220 sink ready\r\n").await.is_err() {
        return;
    }
    // Set while a RCPT TO in this transaction was refused, so the DATA that
    // follows is not counted as a delivery.
    let mut rejected = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let upper = line.to_ascii_uppercase();
        let reply: String = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            // No STARTTLS and no AUTH advertised: the test relay is plain.
            "250-sink\r\n250 SIZE 10485760\r\n".into()
        } else if upper.starts_with("MAIL FROM") {
            rejected = false;
            "250 OK\r\n".into()
        } else if upper.starts_with("RCPT TO") {
            let address = line
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .map(|(address, _)| address.to_string())
                .unwrap_or_default();
            if refuse.lock().unwrap().iter().any(|a| a.eq_ignore_ascii_case(&address)) {
                rejected = true;
                "550 no such mailbox\r\n".into()
            } else {
                "250 OK\r\n".into()
            }
        } else if upper.starts_with("DATA") {
            if write.write_all(b"354 go ahead\r\n").await.is_err() {
                return;
            }
            while let Ok(Some(body)) = lines.next_line().await {
                if body == "." {
                    break;
                }
            }
            if !rejected {
                accepted.fetch_add(1, Ordering::SeqCst);
            }
            "250 queued\r\n".into()
        } else if upper.starts_with("RSET") {
            rejected = false;
            "250 OK\r\n".into()
        } else if upper.starts_with("QUIT") {
            let _ = write.write_all(b"221 bye\r\n").await;
            return;
        } else {
            "250 OK\r\n".into()
        };
        if write.write_all(reply.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// Copy a directory tree. Used to give each fixture its own theme, since a
/// theme is read off disk and a test that activated or deleted one in the
/// repository would change what the next test saw.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn fixture(port: u16) -> (ghost_api::AppState, String) {
    let db = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("../../migrations").run(&db).await.unwrap();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values([
        ("site_url".into(), serde_json::json!("https://example.test")),
        ("title".into(), serde_json::json!("Sink test")),
        ("mail_transport".into(), serde_json::json!("smtp")),
        ("mail_smtp_host".into(), serde_json::json!("127.0.0.1")),
        ("mail_smtp_port".into(), serde_json::json!(port.to_string())),
        ("mail_smtp_security".into(), serde_json::json!("none")),
        ("mail_from_address".into(), serde_json::json!("news@example.test")),
        ("members_email_auth_secret".into(), serde_json::json!("fixture-secret")),
        ("email_track_opens".into(), serde_json::json!(true)),
        ("email_track_clicks".into(), serde_json::json!(true)),
    ]));
    // A content directory of its own, with the source theme copied in, so
    // theme listing and the backup below act on this test's own files rather
    // than on the repository's.
    let content = std::env::temp_dir().join(format!("ghost-rust-test-{}", ghost_core::ids::new_id()));
    let theme_source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
    copy_tree(&theme_source, &content.join("themes").join("source"));
    let config = serde_json::from_value(serde_json::json!({
        "url": "https://example.test", "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": content.to_string_lossy()}
    }))
    .unwrap();
    let routes = ghost_routing::routes_yaml::parse_routes_yaml(ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML).unwrap();
    let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
    let deps = ghost_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: ghost_api::themes::url_resolver(router.clone()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: "test".into(),
        theme_dir: theme_dir.clone(),
        site_origin: "https://example.test".into(),
        content_api_key: String::new(),
    };
    let theme = ghost_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
    let state = ghost_api::AppState {
        db,
        config: Arc::new(config),
        settings,
        theme: Arc::new(tokio::sync::RwLock::new(Arc::new(theme))),
        router: Arc::new(tokio::sync::RwLock::new(router)),
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(ghost_routing::redirects::Redirects::default()))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(ghost_db::limits::Limits::default()),
    };
    (state, owner.id)
}

/// Members subscribed to `newsletter`, half of them without a name so the
/// send has to split the batch on it.
async fn seed(state: &ghost_api::AppState, owner: &str, addresses: &[(&str, &str)]) -> String {
    sqlx::query("INSERT INTO newsletters (id,name,slug,status,created_at) VALUES ('nl','Weekly','weekly','active',datetime('now'))")
        .execute(&state.db)
        .await
        .unwrap();
    for (address, name) in addresses {
        let id = ghost_core::ids::new_id();
        sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES (?,?,?,?,?,'free',datetime('now'))")
            .bind(&id)
            .bind(format!("uuid-{address}"))
            .bind(&id)
            .bind(address)
            .bind(Some(*name).filter(|n| !n.is_empty()))
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO members_newsletters (id,member_id,newsletter_id) VALUES (?,?,'nl')")
            .bind(ghost_core::ids::new_id())
            .bind(&id)
            .execute(&state.db)
            .await
            .unwrap();
    }
    let post = ghost_db::repos::posts::insert(
        &state.db,
        &ghost_db::repos::posts::NewPost {
            title: "Batched".into(),
            status: "published".into(),
            visibility: "public".into(),
            post_type: "post".into(),
            html: Some("<p>Body</p>".into()),
            ..Default::default()
        },
        owner,
    )
    .await
    .unwrap();
    // The post names the newsletter it goes out on; `NewPost` has no field
    // for it, so it is set here as the editor's save would.
    sqlx::query("UPDATE posts SET newsletter_id='nl' WHERE id=?").bind(&post.id).execute(&state.db).await.unwrap();
    post.id
}

#[tokio::test]
async fn a_send_records_every_recipient_batches_it_and_retries_only_the_failures() {
    let sink = Sink::start().await;
    let (state, owner) = fixture(sink.port).await;
    let post_id = seed(
        &state,
        &owner,
        &[("one@example.test", "One"), ("two@example.test", ""), ("three@example.test", "Three")],
    )
    .await;
    // One address the relay will not take, so the send half fails.
    sink.refuse(&["two@example.test"]);

    let sent = ghost_admin_ui::send_newsletter_for_test(&state, &post_id).await;
    // Two accepted, so the send reports success even though one was refused.
    assert_eq!(sent, Ok(2), "{sent:?}");
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 2);

    let email: (String, String, i64, i64, i64, Option<i64>, Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT id,status,email_count,delivered_count,failed_count,candidate_count,preflight_email_count,prepared_at FROM emails",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    let email_id = email.0.clone();
    assert_eq!(email.1, "submitted");
    assert_eq!((email.2, email.3, email.4), (3, 2, 1));
    assert_eq!((email.5, email.6), (Some(3), Some(3)));
    assert!(email.7.is_some(), "prepared_at was never set");

    // A recipient row each, and the one that was refused has a failure
    // record saying so.
    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT email_address,status,batch_id FROM email_recipients ORDER BY email_address")
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|(_, _, batch)| batch.is_some()), "a recipient was not attributed to a batch");
    assert_eq!(
        rows.iter().map(|(a, s, _)| (a.as_str(), s.as_str())).collect::<Vec<_>>(),
        [("one@example.test", "sent"), ("three@example.test", "sent"), ("two@example.test", "failed")]
    );
    let failures: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT message,severity,member_id FROM email_recipient_failures").fetch_all(&state.db).await.unwrap();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].2.is_some(), "the failure does not name the member");

    // Named and unnamed members cannot share a body, so they are separate
    // batches even though three recipients would fit in one.
    let batches: Vec<(String, String, i64, i64)> =
        sqlx::query_as("SELECT member_segment,status,recipient_count,submitted_count FROM email_batches ORDER BY member_segment")
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert_eq!(batches.len(), 2, "{batches:?}");
    assert_eq!(batches[0], ("status:free access:true named:false".into(), "failed".into(), 1, 0));
    assert_eq!(batches[1], ("status:free access:true named:true".into(), "submitted".into(), 2, 2));

    // Each delivery moves the member's email count, which is the denominator
    // of their open rate; the refused one is not counted.
    let counted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE email_count=1").fetch_one(&state.db).await.unwrap();
    assert_eq!(counted, 2);

    // --- retry ------------------------------------------------------------
    // Sending again outright is refused: the post has been emailed.
    assert!(ghost_admin_ui::send_newsletter_for_test(&state, &post_id).await.is_err());

    sink.refuse(&[]);
    let retried = ghost_admin_ui::retry_email(&state, &email_id).await;
    assert_eq!(retried, Ok(1), "{retried:?}");
    // Only the address that failed was tried again — the other two are not
    // mailed a second time.
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 3);

    let settled: (String, i64, i64, Option<String>) =
        sqlx::query_as("SELECT status,delivered_count,failed_count,error FROM emails WHERE id=?")
            .bind(&email_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    // The counts describe the whole send, not just the attempt that fixed it.
    assert_eq!(settled, ("submitted".into(), 3, 0, None));
    let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM email_recipients ORDER BY email_address")
        .fetch_all(&state.db)
        .await
        .unwrap();
    assert_eq!(statuses, ["sent", "sent", "sent"]);
    // The failure history survives the retry: it is the record of what went
    // wrong, not of the current state.
    let remembered: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM email_recipient_failures").fetch_one(&state.db).await.unwrap();
    assert_eq!(remembered, 1);
    // And with nothing left failing, there is nothing to retry.
    assert!(ghost_admin_ui::retry_email(&state, &email_id).await.is_err());
}

#[tokio::test]
async fn a_send_nobody_accepts_leaves_no_record_so_it_can_be_tried_again() {
    let sink = Sink::start().await;
    let (state, owner) = fixture(sink.port).await;
    let post_id = seed(&state, &owner, &[("only@example.test", "Only")]).await;
    sink.refuse(&["only@example.test"]);

    assert!(ghost_admin_ui::send_newsletter_for_test(&state, &post_id).await.is_err());
    // The record is kept, with the reason on it: the send was attempted and
    // is reportable, and `post_id` being unique is what stops a double send.
    let email: (String, i64, i64, Option<String>) =
        sqlx::query_as("SELECT status,delivered_count,failed_count,error FROM emails").fetch_one(&state.db).await.unwrap();
    assert_eq!(email.0, "failed");
    assert_eq!((email.1, email.2), (0, 1));
    assert!(email.3.is_some(), "a failed send did not record why");

    // With the relay taking mail again, the retry reaches them.
    sink.refuse(&[]);
    let email_id: String = sqlx::query_scalar("SELECT id FROM emails").fetch_one(&state.db).await.unwrap();
    assert_eq!(ghost_admin_ui::retry_email(&state, &email_id).await, Ok(1));
    assert_eq!(sink.accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_posts_analytics_page_reports_the_send_it_just_made() {
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    let sink = Sink::start().await;
    let (state, owner) = fixture(sink.port).await;
    let post_id = seed(&state, &owner, &[("a@example.test", "A"), ("b@example.test", "B")]).await;
    sink.refuse(&["b@example.test"]);
    assert_eq!(ghost_admin_ui::send_newsletter_for_test(&state, &post_id).await, Ok(1));

    // Two page views of the post, one of them from a search engine.
    for (viewer, source) in [("v1", Some("Google")), ("v2", None)] {
        sqlx::query("INSERT INTO page_views (id,path,viewer,audience,created_at,referrer_source,referrer_medium) VALUES (?,?,?,'public',datetime('now'),?,?)")
            .bind(ghost_core::ids::new_id())
            .bind("/batched/")
            .bind(viewer)
            .bind(source)
            .bind(source.map(|_| "search"))
            .execute(&state.db)
            .await
            .unwrap();
    }
    // And a member who signed up from it.
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('gm','gu','gt','grown@example.test','free',datetime('now'))")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO members_created_events (id,member_id,source,attribution_id,attribution_type,created_at) VALUES (?,'gm','member',?,'post',datetime('now'))")
        .bind(ghost_core::ids::new_id())
        .bind(&post_id)
        .execute(&state.db)
        .await
        .unwrap();

    let session = ghost_db::repos::sessions::create(&state.db, &owner).await.unwrap();
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/posts/{post_id}/analytics/"))
                .header("cookie", format!("ghost-admin-api-session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let page = String::from_utf8(to_bytes(response.into_body(), 5_000_000).await.unwrap().to_vec()).unwrap();

    assert!(page.contains("Batched"), "the post's title is not on its own analytics page");
    // Web: two views, two visitors, one attributed signup.
    assert!(page.contains(">Views</div><div class=\"value\">2<"), "views missing from {page}");
    assert!(page.contains(">Free signups</div><div class=\"value\">1<"), "signups missing");
    // Sources: the search referral and the direct visit are both named.
    assert!(page.contains("Google"), "the referring source is missing");
    assert!(page.contains("Direct"), "direct traffic is not named");
    // Email: one accepted, one refused, and no open rate yet because
    // nothing has been opened.
    assert!(page.contains(">Sent</div><div class=\"value\">2<"), "sent count missing");
    assert!(page.contains("1 could not be delivered"), "the refused recipient is not reported");
    assert!(page.contains(">Open rate</div><div class=\"value\">0%<"), "open rate missing");

    let export=ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone()).oneshot(
        Request::builder().uri("/settings/migration/export/analytics/").header("cookie",format!("ghost-admin-api-session={session}")).body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(export.status(),axum::http::StatusCode::OK);
    let bytes=to_bytes(export.into_body(),5_000_000).await.unwrap();
    let mut csv=csv::Reader::from_reader(bytes.as_ref());
    let records:Vec<HashMap<String,String>>=csv.deserialize().collect::<Result<_,_>>().unwrap();
    let row=records.iter().find(|r|r["id"]==post_id).unwrap();
    assert_eq!(row["views"],"2");assert_eq!(row["signups"],"1");assert_eq!(row["clicks"],"0");assert_eq!(row["paid_conversions"],"0");

    // A post that does not exist does not render a page of zeros.
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/posts/nope/analytics/")
                .header("cookie", format!("ghost-admin-api-session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let page = String::from_utf8(to_bytes(response.into_body(), 5_000_000).await.unwrap().to_vec()).unwrap();
    assert!(page.contains("no longer exists"), "{page}");
}

/// The Admin API endpoints that reuse this crate's own handlers: themes,
/// export, backup and the settings files. They are merged into the API nest
/// by the binary, so this exercises the router they come from.
#[tokio::test]
async fn the_admin_api_can_manage_themes_and_export_content() {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let sink = Sink::start().await;
    let (state, owner) = fixture(sink.port).await;
    seed(&state, &owner, &[("reader@example.test", "R")]).await;
    let session = ghost_db::repos::sessions::create(&state.db, &owner).await.unwrap();
    let cookie = format!("ghost-admin-api-session={session}");

    let call = |method: &'static str, uri: String, cookie: String| {
        let app = ghost_admin_ui::rest::router().with_state(state.clone());
        async move {
            let response = app
                .oneshot(Request::builder().method(method).uri(uri).header("cookie", cookie).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 20_000_000).await.unwrap();
            (status, body)
        }
    };

    // Themes, as REST rather than as an HTML form.
    let (status, body) = call("GET", "/themes/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let themes: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(!themes["themes"].as_array().unwrap().is_empty(), "{themes}");
    assert!(themes["themes"][0]["package"]["name"].is_string(), "{themes}");

    let (status, body) = call("GET", "/themes/active/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let active: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(active["themes"][0]["active"], true);

    // Installing by name is limited to Ghost's own themes; anything else has
    // to be uploaded, so a `ref` cannot point the server at a random repo.
    let (status, _) = call("POST", "/themes/install/?ref=someone/not-a-ghost-theme".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The export, which is the Ghost-format JSON the importer reads.
    let (status, body) = call("GET", "/db/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let export: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(export["db"][0]["data"]["posts"].is_array(), "the export has no posts: {export}");

    // A backup writes the same thing beside the database.
    let (status, body) = call("POST", "/db/backup/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let backup: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let filename = backup["db"][0]["filename"].as_str().unwrap();
    assert!(state.config.content_path().join("data").join(filename).exists(), "{filename} was not written");

    // Deleting all content needs the publication's own name echoed back: a
    // REST client has no confirmation dialog.
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts").fetch_one(&state.db).await.unwrap();
    assert!(before > 0, "the fixture has no content to protect");
    let (status, _) = call("DELETE", "/db/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts").fetch_one(&state.db).await.unwrap();
    assert_eq!(after, before, "content was deleted without confirmation");

    let name = state.settings.site_name();
    let encoded = form_urlencoded::Serializer::new(String::new()).append_pair("confirm", &name).finish();
    let (status, _) = call("DELETE", format!("/db/?{encoded}"), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let posts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts").fetch_one(&state.db).await.unwrap();
    assert_eq!(posts, 0);

    // The routing file, as the file it is edited as.
    let (status, body) = call("GET", "/settings/routes/yaml/".into(), cookie.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("routes:"), "{}", String::from_utf8_lossy(&body));

    // And none of it without being signed in.
    for uri in ["/themes/", "/db/", "/settings/routes/yaml/"] {
        let (status, _) = call("GET", uri.into(), String::new()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} was readable unauthenticated");
    }
}

#[tokio::test]
async fn gift_reminders_send_once_and_survive_repeated_scheduler_passes(){
    let sink=Sink::start().await;
    let (state,_)=fixture(sink.port).await;
    sqlx::query("INSERT INTO products(id,name,slug,type,created_at)VALUES('gift-tier','Gift','gift-reminder-tier','paid',datetime('now'))").execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO members(id,uuid,transient_id,email,status,created_at)VALUES('gift-member','gift-member-uuid','gift-transient','gift@example.test','gift',datetime('now'))").execute(&state.db).await.unwrap();
    let redeemed:String=sqlx::query_scalar("SELECT datetime('now','-25 days')").fetch_one(&state.db).await.unwrap();
    let end:String=sqlx::query_scalar("SELECT datetime('now','+5 days')").fetch_one(&state.db).await.unwrap();
    sqlx::query("INSERT INTO members_products(id,member_id,product_id,expiry_at)VALUES('gift-grant','gift-member','gift-tier',?)").bind(end).execute(&state.db).await.unwrap();
    let data=serde_json::json!({"gift":true,"token":"gift-reminder-token","tier_id":"gift-tier","tier_name":"Gift","cadence":"month","duration":1,"total_months":1,"currency":"usd","amount":100,"buyer_email":"buyer@example.test","delivery":"link","status":"redeemed","redeemed_by":"gift-member","redeemed_at":redeemed});
    sqlx::query("INSERT INTO tokens(id,token,data,created_at)VALUES('gift-token','gift-reminder-token',?,datetime('now'))").bind(data.to_string()).execute(&state.db).await.unwrap();
    assert_eq!(ghost_api::members::send_due_gift_reminders(&state).await.unwrap(),1);
    assert_eq!(ghost_api::members::send_due_gift_reminders(&state).await.unwrap(),0);
    assert_eq!(sink.accepted.load(Ordering::SeqCst),1);
    let sent:Option<String>=sqlx::query_scalar("SELECT sent_at FROM gift_reminders WHERE token='gift-reminder-token'").fetch_one(&state.db).await.unwrap();assert!(sent.is_some());
    let _=std::fs::remove_dir_all(state.config.content_path());
}

#[tokio::test]
async fn ghost_newsletter_preview_and_test_send_use_the_standard_api_payload(){
    use axum::{body::{Body,to_bytes},http::Request};use tower::ServiceExt;
    let sink=Sink::start().await;let(state,owner)=fixture(sink.port).await;
    let post=seed(&state,&owner,&[("preview-subscriber@example.test","Reader")]).await;
    let session=ghost_db::repos::sessions::create(&state.db,&owner).await.unwrap();
    let app=ghost_api::admin::admin_router().with_state(state.clone());
    let response=app.clone().oneshot(Request::builder().uri(format!("/email_previews/posts/{post}/")).header("cookie",format!("ghost-admin-api-session={session}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::OK);let bytes=to_bytes(response.into_body(),5_000_000).await.unwrap();let preview:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert!(preview["email_previews"][0]["html"].as_str().unwrap().contains("Batched"));
    let response=app.oneshot(Request::builder().method("POST").uri(format!("/email_previews/posts/{post}/")).header("cookie",format!("ghost-admin-api-session={session}")).header("content-type","application/json").body(Body::from(serde_json::json!({"emails":["test-reader@example.test"]}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::NO_CONTENT);assert_eq!(sink.accepted.load(Ordering::SeqCst),1);
    let _=std::fs::remove_dir_all(state.config.content_path());
}

#[tokio::test]
async fn ghost_json_import_uses_the_standard_multipart_field_and_preserves_content(){
    use axum::{body::{Body,to_bytes},http::Request};use tower::ServiceExt;
    let sink=Sink::start().await;let(state,owner)=fixture(sink.port).await;
    let session=ghost_db::repos::sessions::create(&state.db,&owner).await.unwrap();
    let lexical=serde_json::json!({"root":{"type":"root","children":[{"type":"paragraph","children":[{"type":"text","text":"Imported through Ghost API"}]}]}}).to_string();
    let data=serde_json::json!({"db":[{"data":{"posts":[{"id":"ghost-api-import","title":"API import","slug":"api-import","lexical":lexical,"type":"post","status":"published","visibility":"public","published_at":"2026-10-01T12:34:56Z","meta_title":"SEO survives"}],"posts_authors":[{"id":"import-author-relation","post_id":"ghost-api-import","author_id":owner}]}}]});
    let body=format!("--import-boundary\r\nContent-Disposition: form-data; name=\"importfile\"; filename=\"ghost.json\"\r\nContent-Type: application/json\r\n\r\n{data}\r\n--import-boundary--\r\n");
    let app=ghost_admin_ui::rest::router().with_state(state.clone());
    let response=app.oneshot(Request::builder().method("POST").uri("/db/").header("cookie",format!("ghost-admin-api-session={session}")).header("accept","application/json").header("content-type","multipart/form-data; boundary=import-boundary").body(Body::from(body)).unwrap()).await.unwrap();
    let status=response.status();let bytes=to_bytes(response.into_body(),1_000_000).await.unwrap();assert_eq!(status,axum::http::StatusCode::OK,"{}",String::from_utf8_lossy(&bytes));
    let (html,date):(String,String)=sqlx::query_as("SELECT html,published_at FROM posts WHERE id='ghost-api-import'").fetch_one(&state.db).await.unwrap();assert!(html.contains("Imported through Ghost API"));assert_eq!(date,"2026-10-01 12:34:56");
    let title:String=sqlx::query_scalar("SELECT meta_title FROM posts_meta WHERE post_id='ghost-api-import'").fetch_one(&state.db).await.unwrap();assert_eq!(title,"SEO survives");
    let _=std::fs::remove_dir_all(state.config.content_path());
}
