//! The Forms module end to end: build a form in the admin, send a response
//! through the public endpoint the way a browser would, and read it back.
//!
//! The checks that matter most here are the negative ones — who is refused,
//! and that what a visitor wrote is never rendered as markup.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

async fn fixture() -> (ghost_db::TestDb, ghost_api::AppState, String) {
    let (db, state, owner) = fixture_with(vec![]).await;
    // No mail is sent from the tests that use this.
    sqlx::query("UPDATE forms_config SET notify_enabled = 0 WHERE id = 1").execute(&state.db).await.unwrap();
    (db, state, owner)
}

async fn fixture_with(extra_settings: Vec<(String, serde_json::Value)>) -> (ghost_db::TestDb, ghost_api::AppState, String) {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let mut values: Vec<(String, serde_json::Value)> =
        vec![("site_url".into(), serde_json::json!(ORIGIN)), ("title".into(), serde_json::json!("Forms test"))];
    values.extend(extra_settings);
    let settings = Arc::new(ghost_db::SettingsCache::from_values(values));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": std::env::temp_dir().join(format!("ghost-forms-test-{}", ghost_core::ids::new_id())).to_string_lossy()}
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
        site_origin: ORIGIN.into(),
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
    (test_db, state, owner.id)
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

/// One request to the admin, as a browser on the admin's own pages sends it
/// unless `origin` says otherwise.
async fn admin(state: &ghost_api::AppState, method: &str, uri: &str, cookie: Option<&str>, origin: Option<&str>, form: Option<&str>) -> Reply {
    let mut request = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let body = match form {
        Some(form) => {
            request = request.header("content-type", "application/x-www-form-urlencoded");
            Body::from(form.to_string())
        }
        None => Body::empty(),
    };
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    reply(app.oneshot(request.body(body).unwrap()).await.unwrap()).await
}

async fn reply(response: axum::response::Response) -> Reply {
    let status = response.status();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body = String::from_utf8_lossy(&to_bytes(response.into_body(), 5_000_000).await.unwrap()).into_owned();
    Reply { status, location, body }
}

/// A visitor's post to the public endpoint.
async fn visitor(state: &ghost_api::AppState, form_id: &str, body: &str, origin: &str) -> Reply {
    let app = ghost_api::forms::public_router().with_state(state.clone());
    let request = Request::builder()
        .method("POST")
        .uri(format!("/forms/api/{form_id}/submit/"))
        .header("host", HOST)
        .header("origin", origin)
        .header("accept", "application/json")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("referer", "https://example.test/contact/?utm=1")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(body.to_string()))
        .unwrap();
    reply(app.oneshot(request).await.unwrap()).await
}

fn encode(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text.find(start).unwrap_or_else(|| panic!("{start:?} not found")) + start.len();
    let len = text[from..].find(end).unwrap();
    &text[from..from + len]
}

const HOSTILE: &str = r#"<img src=x onerror=alert(1)>"><script>alert(2)</script>"#;

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let (_db, state, _owner) = fixture().await;
    for (method, uri) in [
        ("GET", "/forms/"),
        ("GET", "/forms/responses/"),
        ("GET", "/forms/responses/export/"),
        ("GET", "/forms/0123456789abcdef01234567/"),
        ("GET", "/forms/responses/0123456789abcdef01234567/"),
        ("GET", "/settings/forms/"),
        ("POST", "/forms/new/"),
        ("POST", "/forms/0123456789abcdef01234567/delete/"),
        ("POST", "/forms/responses/0123456789abcdef01234567/delete/"),
        ("POST", "/settings/forms/"),
    ] {
        // no cookie, and a cookie that names no session
        for cookie in [None, Some("ghost-admin-api-session=not-a-session")] {
            let r = admin(&state, method, uri, cookie, Some(ORIGIN), (method == "POST").then_some("")).await;
            assert!(
                r.status.is_redirection() && r.location.contains("signin") || r.status == StatusCode::UNAUTHORIZED,
                "{method} {uri} answered {} to a stranger",
                r.status
            );
            assert!(!r.body.contains("form-builder"), "{uri} showed the builder to a stranger");
        }
    }
    assert!(ghost_db::repos::forms::list(&state.db).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_form_is_built_sent_read_exported_and_deleted() {
    let (_db, state, owner) = fixture().await;
    let session = ghost_db::repos::sessions::create(&state.db, &owner).await.unwrap();
    let cookie = format!("ghost-admin-api-session={session}");
    let cookie = Some(cookie.as_str());

    // The module is on the Modules page and in Settings.
    let modules = admin(&state, "GET", "/modules/", cookie, None, None).await;
    assert!(modules.body.contains(r#"href="/ghost/forms/""#), "Forms is not on the Modules page");
    let settings = admin(&state, "GET", "/settings/", cookie, None, None).await;
    assert!(settings.body.contains(r#"href="/ghost/settings/forms/""#), "Forms is not on the Settings page");

    // --- a state-changing request from another site is refused, signed in or not
    let forged = admin(&state, "POST", "/forms/new/", cookie, Some("https://evil.test"), Some("")).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert!(ghost_db::repos::forms::list(&state.db).await.unwrap().is_empty(), "a cross-site request created a form");

    // --- build
    let created = admin(&state, "POST", "/forms/new/", cookie, Some(ORIGIN), Some("")).await;
    assert!(created.status.is_redirection(), "{}", created.status);
    let form_id = between(&created.location, "/ghost/forms/", "/").to_string();
    assert!(ghost_core::ids::is_objectid(&form_id), "{}", created.location);

    let builder = admin(&state, "GET", &format!("/forms/{form_id}/"), cookie, None, None).await;
    assert_eq!(builder.status, StatusCode::OK);
    assert!(builder.body.contains("forms-builder.js?v=") && builder.body.contains("data-builder="));
    assert!(builder.body.contains(r#"id="fb-upload""#) && builder.body.contains("/ghost/api/admin/files/upload/"), "the upload control is not on the builder");

    // Save a definition the way the builder posts it — with a hostile label,
    // a forged field key and a property the builder never sends.
    let fields = serde_json::json!([
        {"key": "", "kind": "text", "label": HOSTILE, "required": true},
        {"key": "_token", "kind": "email", "label": "Email", "required": true},
        {"key": "", "kind": "select", "label": "Topic", "options": ["Sales", "Support"], "html": "<script>"},
        {"key": "", "kind": "textarea", "label": "Message", "required": true},
    ])
    .to_string();
    let saved = admin(
        &state,
        "POST",
        &format!("/forms/{form_id}/"),
        cookie,
        Some(ORIGIN),
        Some(&encode(&[
            ("name", "Contact us"),
            ("status", "active"),
            ("fields", &fields),
            ("submit_label", "Send it"),
            ("success_message", "Thanks <b>friend</b>"),
            ("notify_emails", "team@example.test"),
        ])),
    )
    .await;
    assert!(saved.location.contains("ok="), "save was refused: {}", saved.location);

    // a bad recipient refuses the whole save rather than storing it
    let refused = admin(
        &state,
        "POST",
        &format!("/forms/{form_id}/"),
        cookie,
        Some(ORIGIN),
        Some(&encode(&[("name", "Changed"), ("fields", &fields), ("notify_emails", "a@b.co\r\nBcc: x@y.zz")])),
    )
    .await;
    assert!(refused.location.contains("error="), "{}", refused.location);

    let form = ghost_db::repos::forms::find(&state.db, &form_id).await.unwrap().unwrap();
    assert_eq!(form.name, "Contact us");
    assert_eq!(form.notify_emails, "team@example.test");
    assert_eq!(form.fields.len(), 4);
    assert!(form.fields.iter().all(|f| ghost_core::forms::is_field_key(&f.key)), "a forged key was kept");

    // the builder shows the hostile label as data, never as markup
    let builder = admin(&state, "GET", &format!("/forms/{form_id}/"), cookie, None, None).await;
    assert!(!builder.body.contains("<script>alert(2)"), "the builder page carries a raw script from a label");

    // --- the editor is told about the form, and loads the card
    let editor = admin(&state, "GET", "/posts/new/", cookie, None, None).await;
    assert_eq!(editor.status, StatusCode::OK);
    assert!(editor.body.contains("koenig-forms.js"), "the form card script is not loaded by the editor");
    assert!(editor.body.contains(&form_id), "the editor was not given the form to offer");

    // --- render it into a page
    let placeholder = ghost_api::forms::placeholder(&form_id).unwrap();
    let page = ghost_api::forms::expand_html(&state, format!("<p>before</p>{placeholder}<p>after</p>")).await;
    assert!(page.contains("<form class=\"vf-form\""), "{page}");
    assert!(!page.contains("<script>alert(2)") && !page.contains("<img src=x"), "a label reached the page unescaped");
    let token = between(&page, r#"name="_token" value=""#, "\"").to_string();
    let key = |i: usize| form.fields[i].key.clone();
    let answers = |name: &str, token: &str| {
        encode(&[
            ("_token", token),
            (&key(0), name),
            (&key(1), "ada@example.com"),
            (&key(2), "Support"),
            (&key(3), "=HYPERLINK(\"http://evil.test\")\r\nsecond line"),
            // none of these are fields of the form
            ("status", "read"),
            ("form_id", "0123456789abcdef01234567"),
            ("member_id", "0123456789abcdef01234567"),
        ])
    };

    // --- send. Too soon first: nothing human answers a form this fast.
    let hasty = visitor(&state, &form_id, &answers(HOSTILE, &token), ORIGIN).await;
    assert_eq!(hasty.status, StatusCode::BAD_REQUEST, "{}", hasty.body);
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;

    // from another site
    let foreign = visitor(&state, &form_id, &answers(HOSTILE, &token), "https://evil.test").await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
    // the decoy filled in: answered as a success, stored nowhere
    let bot = visitor(&state, &form_id, &format!("{}&fax_number=1", answers(HOSTILE, &token)), ORIGIN).await;
    assert_eq!(bot.status, StatusCode::OK);
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap(), (0, 0, 0));

    // the real one
    let sent = visitor(&state, &form_id, &answers(HOSTILE, &token), ORIGIN).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    // and the same token again
    let replay = visitor(&state, &form_id, &answers("Second", &token), ORIGIN).await;
    assert_eq!(replay.status, StatusCode::CONFLICT, "{}", replay.body);
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap(), (1, 0, 0));

    let (rows, _) = ghost_db::repos::forms::browse(&state.db, &ghost_db::repos::forms::Browse { page: 1, limit: 10, ..Default::default() })
        .await
        .unwrap();
    let stored = &rows[0];
    assert_eq!(stored.answers.len(), 4, "something other than the form's fields was stored");
    assert_eq!(stored.form_id, form_id, "a posted form_id overrode the real one");
    assert_eq!(stored.member_id, None, "a posted member_id was believed");
    assert_eq!(stored.status, "new");
    assert_eq!(stored.page_path.as_deref(), Some("/contact/"));
    assert_eq!(stored.ip.as_deref(), Some("203.0.113.9"));

    // --- read it in the admin: what the visitor wrote is text, not markup
    let list = admin(&state, "GET", "/forms/responses/", cookie, None, None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert!(list.body.contains("&lt;img src=x"), "the response is not in the list");
    assert!(!list.body.contains("<img src=x") && !list.body.contains("<script>alert(2)"), "the list renders a visitor's markup");

    let detail = admin(&state, "GET", &format!("/forms/responses/{}/", stored.id), cookie, None, None).await;
    assert_eq!(detail.status, StatusCode::OK);
    assert!(!detail.body.contains("<img src=x") && !detail.body.contains("<script>alert(2)"), "the detail page renders a visitor's markup");
    assert!(detail.body.contains("mailto:ada@example.com"));
    assert!(detail.body.contains("203.0.113.9"));
    // opening it marked it read
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap(), (0, 1, 0));

    // --- export: the formula a visitor typed is defused
    let export = admin(&state, "GET", &format!("/forms/responses/export/?form={form_id}"), cookie, None, None).await;
    assert_eq!(export.status, StatusCode::OK);
    assert!(export.body.contains("'=HYPERLINK"), "{}", export.body);
    assert!(!export.body.contains(",=HYPERLINK") && !export.body.contains("\"=HYPERLINK"), "{}", export.body);

    // --- file as spam, from the admin only
    let forged = admin(&state, "POST", &format!("/forms/responses/{}/delete/", stored.id), cookie, Some("https://evil.test"), Some("")).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    let spam = admin(&state, "POST", &format!("/forms/responses/{}/status/", stored.id), cookie, Some(ORIGIN), Some("status=spam")).await;
    assert!(spam.location.contains("ok="), "{}", spam.location);
    let bogus = admin(&state, "POST", &format!("/forms/responses/{}/status/", stored.id), cookie, Some(ORIGIN), Some("status=owner")).await;
    assert!(bogus.location.contains("error="), "{}", bogus.location);
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap(), (0, 0, 1));

    // --- settings
    let page = admin(&state, "GET", "/settings/forms/", cookie, None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    let saved = admin(&state, "POST", "/settings/forms/", cookie, Some(ORIGIN), Some("notify_emails=ops%40example.test&retention_days=90")).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let config = ghost_db::repos::forms::config(&state.db).await.unwrap();
    assert_eq!((config.notify_enabled, config.store_ip, config.retention_days), (false, false, 90));
    assert_eq!(config.notify_emails, "ops@example.test");
    // a window that is not on offer keeps everything rather than being trusted
    admin(&state, "POST", "/settings/forms/", cookie, Some(ORIGIN), Some("retention_days=1")).await;
    assert_eq!(ghost_db::repos::forms::config(&state.db).await.unwrap().retention_days, 0);

    // --- what follows a response is configurable, and only to somewhere safe
    let stored_fields = serde_json::to_string(&form.fields).unwrap();
    let save_with = |action: &'static str, url: &'static str| {
        let fields = stored_fields.clone();
        let state = state.clone();
        let form_id = form_id.clone();
        async move {
            admin(
                &state,
                "POST",
                &format!("/forms/{form_id}/"),
                cookie,
                Some(ORIGIN),
                Some(&encode(&[
                    ("name", "Contact us"),
                    ("status", "active"),
                    ("fields", &fields),
                    ("success_action", action),
                    ("success_url", url),
                    ("success_link_label", "Download the guide"),
                ])),
            )
            .await
        }
    };
    for bad in ["javascript:alert(1)", "//evil.test/x", "data:text/html,x", "evil.test"] {
        let refused = save_with("redirect", bad).await;
        assert!(refused.location.contains("error="), "{bad} was accepted as a redirect target");
    }
    // a link or redirect with nowhere to go is refused rather than saved half-set
    assert!(save_with("link", "").await.location.contains("error="));
    assert_eq!(ghost_db::repos::forms::find(&state.db, &form_id).await.unwrap().unwrap().success_action, "message");

    let saved = save_with("link", "/content/files/2026/11/guide.pdf").await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    // the link is not in the page a visitor is served…
    let page = ghost_api::forms::expand_html(&state, placeholder.clone()).await;
    assert!(!page.contains("guide.pdf"), "the gated link is in the page before any response");
    // …and arrives only with an accepted response
    let token = between(&page, r#"name="_token" value=""#, "\"").to_string();
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    let sent = visitor(&state, &form_id, &answers("Grace", &token), ORIGIN).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    let json: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(json["link"]["url"], "/content/files/2026/11/guide.pdf");
    assert_eq!(json["link"]["label"], "Download the guide");
    // a refused response gets no link
    let again = visitor(&state, &form_id, &answers("Grace", &token), ORIGIN).await;
    assert!(!again.body.contains("guide.pdf"), "{}", again.body);

    let saved = save_with("redirect", "https://example.test/thanks/").await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let page = ghost_api::forms::expand_html(&state, placeholder.clone()).await;
    let token = between(&page, r#"name="_token" value=""#, "\"").to_string();
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    let sent = visitor(&state, &form_id, &answers("Hedy", &token), ORIGIN).await;
    let json: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(json["redirect"], "https://example.test/thanks/");
    // these two responses arrived; clear them so the counts below are unchanged
    sqlx::query("DELETE FROM form_submissions WHERE status = 'new'").execute(&state.db).await.unwrap();

    // --- a file uploaded the way the builder's button uploads it becomes a
    // link the form will accept
    let boundary = "forms-test-boundary";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"guide.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4 test\r\n--{boundary}--\r\n"
    );
    let upload = axum::Router::new().nest("/files", ghost_api::images::files_router()).with_state(state.clone());
    let uploaded = reply(
        upload
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/files/upload/")
                    .header("host", HOST)
                    .header("cookie", cookie.unwrap())
                    .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                    .body(Body::from(multipart))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.body);
    let uploaded: serde_json::Value = serde_json::from_str(&uploaded.body).unwrap();
    // the shape the builder script reads
    let file_url = uploaded["files"][0]["url"].as_str().expect("files[0].url").to_string();
    assert!(file_url.ends_with(".pdf"), "{file_url}");
    assert_eq!(ghost_core::forms::normalize_success_url(&file_url).as_deref(), Ok(file_url.as_str()));
    // and a stranger cannot upload
    let upload = axum::Router::new().nest("/files", ghost_api::images::files_router()).with_state(state.clone());
    let stranger = upload
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/files/upload/")
                .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                .body(Body::from("x"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stranger.status(), StatusCode::UNAUTHORIZED);

    // --- close the form: it stops being offered and stops accepting
    let closed = admin(
        &state,
        "POST",
        &format!("/forms/{form_id}/"),
        cookie,
        Some(ORIGIN),
        Some(&encode(&[("name", "Contact us"), ("status", "closed"), ("fields", &fields)])),
    )
    .await;
    assert!(closed.location.contains("ok="), "{}", closed.location);
    let page = ghost_api::forms::expand_html(&state, placeholder.clone()).await;
    assert!(!page.contains("<form") && page.contains("no longer accepting"), "{page}");
    let late = visitor(&state, &form_id, &answers("Late", &token), ORIGIN).await;
    assert_eq!(late.status, StatusCode::NOT_FOUND);

    // --- delete: the form, its responses, and its place on the page
    let deleted = admin(&state, "POST", &format!("/forms/{form_id}/delete/"), cookie, Some(ORIGIN), Some("")).await;
    assert!(deleted.location.contains("ok="), "{}", deleted.location);
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap(), (0, 0, 0));
    assert_eq!(ghost_api::forms::expand_html(&state, format!("<p>a</p>{placeholder}<p>b</p>")).await, "<p>a</p><p>b</p>");
}

/// A loopback SMTP server that records who each message was addressed to.
async fn smtp_sink() -> (u16, Arc<std::sync::Mutex<Vec<(String, String)>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received: Arc<std::sync::Mutex<Vec<(String, String)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = received.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let store = store.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                if write.write_all(b"220 sink ready\r\n").await.is_err() {
                    return;
                }
                let mut recipient = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    let upper = line.to_ascii_uppercase();
                    let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        b"250-sink\r\n250 SIZE 10485760\r\n"
                    } else if upper.starts_with("RCPT TO") {
                        recipient = line.split_once('<').and_then(|(_, rest)| rest.split_once('>')).map(|(a, _)| a.to_string()).unwrap_or_default();
                        b"250 OK\r\n"
                    } else if upper.starts_with("DATA") {
                        if write.write_all(b"354 go ahead\r\n").await.is_err() {
                            return;
                        }
                        let mut message = String::new();
                        while let Ok(Some(body)) = lines.next_line().await {
                            if body == "." {
                                break;
                            }
                            message.push_str(&body);
                            message.push_str("\r\n");
                        }
                        store.lock().unwrap().push((recipient.clone(), message));
                        b"250 queued\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        return;
                    } else {
                        b"250 OK\r\n"
                    };
                    if write.write_all(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, received)
}

/// Send one valid response to `form_id` and wait for `expected` messages.
async fn respond_and_collect(
    state: &ghost_api::AppState,
    form_id: &str,
    received: &Arc<std::sync::Mutex<Vec<(String, String)>>>,
    expected: usize,
) -> Vec<(String, String)> {
    let form = ghost_db::repos::forms::find(&state.db, form_id).await.unwrap().unwrap();
    let page = ghost_api::forms::expand_html(state, ghost_api::forms::placeholder(form_id).unwrap()).await;
    let token = between(&page, r#"name="_token" value=""#, "\"").to_string();
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    let body = encode(&[
        ("_token", &token),
        (&form.fields[0].key, "Ada Lovelace"),
        (&form.fields[1].key, "ada@example.com"),
        (&form.fields[2].key, "Hello <b>there</b>\r\nSubject: injected"),
    ]);
    let sent = visitor(state, form_id, &body, ORIGIN).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    // The notification is sent after the visitor has their answer.
    for _ in 0..100 {
        if received.lock().unwrap().len() >= expected {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    std::mem::take(&mut *received.lock().unwrap())
}

#[tokio::test]
async fn a_response_is_emailed_to_the_owner_by_default_through_the_sites_mail_settings() {
    let (port, received) = smtp_sink().await;
    // Settings → Email: the transactional transport every other system
    // message uses. Nothing about forms is configured.
    let (_db, state, owner) = fixture_with(vec![
        ("mail_transport".into(), serde_json::json!("smtp")),
        ("mail_smtp_host".into(), serde_json::json!("127.0.0.1")),
        ("mail_smtp_port".into(), serde_json::json!(port.to_string())),
        ("mail_smtp_security".into(), serde_json::json!("none")),
        ("mail_from_address".into(), serde_json::json!("site@example.test")),
    ])
    .await;
    let session = ghost_db::repos::sessions::create(&state.db, &owner).await.unwrap();
    let cookie = format!("ghost-admin-api-session={session}");
    let created = admin(&state, "POST", "/forms/new/", Some(&cookie), Some(ORIGIN), Some("")).await;
    let form_id = between(&created.location, "/ghost/forms/", "/").to_string();

    // --- nobody assigned anywhere: the owner hears about it
    let mail = respond_and_collect(&state, &form_id, &received, 1).await;
    assert_eq!(mail.len(), 1, "expected exactly one notification");
    let (to, message) = &mail[0];
    assert_eq!(to, "owner@example.test");
    // sent as the address set in Settings → Email
    assert!(message.contains("site@example.test"), "the message did not come from the configured sender:\n{message}");
    assert!(message.contains("Subject: New response: Contact form"), "{message}");
    // an answer cannot add a header, and its markup is not markup in the mail
    let headers = message.split("\r\n\r\n").next().unwrap_or("");
    assert!(!headers.contains("injected"), "an answer reached the headers:\n{headers}");
    // The plain-text part carries the answer as typed; the HTML part must
    // carry it escaped. (Soft line breaks are undone before looking.)
    let unfolded = message.replace("=\r\n", "");
    let html_part = unfolded.split("text/html").nth(1).expect("the message has an HTML part");
    assert!(!html_part.contains("<b>there</b>"), "an answer's markup is live in the HTML part:\n{html_part}");
    assert!(html_part.contains("&lt;b&gt;there&lt;/b&gt;"), "the answer is missing from the HTML part:\n{html_part}");

    // --- the site-wide list replaces the owner
    let saved = admin(&state, "POST", "/settings/forms/", Some(&cookie), Some(ORIGIN), Some("notify_enabled=1&notify_emails=ops%40example.test")).await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let mail = respond_and_collect(&state, &form_id, &received, 1).await;
    assert_eq!(mail.iter().map(|(to, _)| to.as_str()).collect::<Vec<_>>(), ["ops@example.test"]);

    // --- and a form's own list replaces that
    let form = ghost_db::repos::forms::find(&state.db, &form_id).await.unwrap().unwrap();
    let saved = admin(
        &state,
        "POST",
        &format!("/forms/{form_id}/"),
        Some(&cookie),
        Some(ORIGIN),
        Some(&encode(&[
            ("name", "Contact form"),
            ("status", "active"),
            ("fields", &serde_json::to_string(&form.fields).unwrap()),
            ("notify_emails", "sales@example.test, lead@example.test"),
        ])),
    )
    .await;
    assert!(saved.location.contains("ok="), "{}", saved.location);
    let mail = respond_and_collect(&state, &form_id, &received, 2).await;
    let mut to: Vec<&str> = mail.iter().map(|(to, _)| to.as_str()).collect();
    to.sort();
    assert_eq!(to, ["lead@example.test", "sales@example.test"]);

    // --- switched off: stored, and nobody is mailed
    admin(&state, "POST", "/settings/forms/", Some(&cookie), Some(ORIGIN), Some("notify_emails=ops%40example.test")).await;
    let mail = respond_and_collect(&state, &form_id, &received, 1).await;
    assert!(mail.is_empty(), "a notification was sent with notifications off");
    assert_eq!(ghost_db::repos::forms::totals(&state.db, None).await.unwrap().0, 4);
}
