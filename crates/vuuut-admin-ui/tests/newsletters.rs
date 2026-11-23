//! The Newsletters module end to end: who is refused, what the overview says
//! about a site, and whether the retry refuses what it should.
//!
//! The negative tests come first, and there are three kinds:
//!
//! * **Refusals** — who cannot open the screens, and who cannot drive the
//!   retry from another origin. The retry spends money per recipient and
//!   spends the site's sending reputation, which makes it the wrong button to
//!   leave resting on one browser behaviour.
//! * **Retries that must not happen** — a succeeded edition, a pending one, a
//!   forged id, an id for a row that has gone. Each of those, if it went
//!   through, would mail a list a second time.
//! * **Judgements that must not be flattering** — a seeded site has a
//!   newsletter and sending switched on and still cannot deliver a thing,
//!   because nothing is configured to carry it. That is the case the module
//!   exists for.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

async fn fixture() -> (vuuut_db::TestDb, vuuut_api::AppState, String) {
    fixture_with(vec![]).await
}

async fn fixture_with(
    extra_settings: Vec<(String, serde_json::Value)>,
) -> (vuuut_db::TestDb, vuuut_api::AppState, String) {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let mut values: Vec<(String, serde_json::Value)> = vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Newsletter test")),
    ];
    values.extend(extra_settings);
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(values));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": std::env::temp_dir()
            .join(format!("ghost-newsletters-test-{}", vuuut_core::ids::new_id()))
            .to_string_lossy()}
    }))
    .unwrap();
    let routes = vuuut_routing::routes_yaml::parse_routes_yaml(
        vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML,
    )
    .unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
    let deps = vuuut_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: vuuut_api::themes::url_resolver(router.clone()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: "test".into(),
        theme_dir: theme_dir.clone(),
        site_origin: ORIGIN.into(),
        content_api_key: String::new(),
    };
    let theme = vuuut_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
    let state = vuuut_api::AppState {
        db,
        config: Arc::new(config),
        settings,
        theme: Arc::new(tokio::sync::RwLock::new(Arc::new(theme))),
        router: Arc::new(tokio::sync::RwLock::new(router)),
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(
            vuuut_routing::redirects::Redirects::default(),
        ))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(vuuut_db::limits::Limits::default()),
        firewall: vuuut_db::repos::firewall::RuleCache::empty(),
    };
    (test_db, state, owner.id)
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

async fn admin(
    state: &vuuut_api::AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    form: Option<&str>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST)
        .header("accept", "text/html");
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
    let app = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = String::from_utf8_lossy(&to_bytes(response.into_body(), 5_000_000).await.unwrap())
        .into_owned();
    Reply { status, location, body }
}

async fn get(state: &vuuut_api::AppState, uri: &str, cookie: &str) -> Reply {
    admin(state, "GET", uri, Some(cookie), None, None).await
}

async fn staff(state: &vuuut_api::AppState, name: &str, email: &str, role: &str) -> String {
    let id = vuuut_core::ids::new_id();
    let now = vuuut_core::dates::now_db();
    sqlx::query(
        "INSERT INTO users (id, name, slug, password, email, status, visibility, created_at) \
         VALUES (?, ?, ?, '', ?, 'active', 'public', ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(name.to_lowercase())
    .bind(email)
    .bind(&now)
    .execute(&state.db)
    .await
    .unwrap();
    let role_id: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
        .bind(vuuut_core::ids::new_id())
        .bind(&role_id)
        .bind(&id)
        .execute(&state.db)
        .await
        .unwrap();
    id
}

fn cookie_for(sid: &str) -> String {
    format!("vuuut-admin-session={sid}")
}

async fn owner_cookie(state: &vuuut_api::AppState, owner: &str) -> String {
    cookie_for(&vuuut_db::repos::sessions::create(&state.db, owner).await.unwrap())
}

async fn post(state: &vuuut_api::AppState, title: &str) -> String {
    let id = vuuut_core::ids::new_id();
    sqlx::query(
        "INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at) \
         VALUES (?, ?, ?, ?, 'post', 'sent', 'public', ?)",
    )
    .bind(&id)
    .bind(vuuut_core::ids::uuid_v4())
    .bind(title)
    .bind(format!("{}-{}", vuuut_core::slug::safe_string(title), &id[..6]))
    .bind(vuuut_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();
    id
}

async fn edition(state: &vuuut_api::AppState, post_id: &str, status: &str, subject: &str) -> String {
    let id = vuuut_core::ids::new_id();
    let news: Option<String> = sqlx::query_scalar("SELECT id FROM newsletters LIMIT 1")
        .fetch_optional(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO emails (id, post_id, status, subject, newsletter_id, error, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(post_id)
    .bind(status)
    .bind(subject)
    .bind(news)
    .bind(if status == "failed" { Some("The provider refused the domain") } else { None })
    .bind(vuuut_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();
    id
}

async fn member_on_the_newsletter(state: &vuuut_api::AppState, email: &str) -> String {
    let id = vuuut_core::ids::new_id();
    sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, status, name, email_disabled, \
         created_at) VALUES (?, ?, ?, ?, 'free', ?, 0, ?)",
    )
    .bind(&id)
    .bind(vuuut_core::ids::uuid_v4())
    .bind(vuuut_core::ids::new_id())
    .bind(email)
    .bind(email.split('@').next().unwrap_or("Reader"))
    .bind(vuuut_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();
    let news: String = sqlx::query_scalar("SELECT id FROM newsletters LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO members_newsletters (id, member_id, newsletter_id, status) \
         VALUES (?, ?, ?, 'confirmed')",
    )
    .bind(vuuut_core::ids::new_id())
    .bind(&id)
    .bind(&news)
    .execute(&state.db)
    .await
    .unwrap();
    id
}

/// How many recipient rows an edition has, to prove a refused retry sent
/// nothing.
async fn recipient_rows(state: &vuuut_api::AppState, email_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM email_recipients WHERE email_id = ?")
        .bind(email_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(-1)
}

const READ_SCREENS: &[&str] = &["/newsletters/", "/newsletters/editions/"];
const RETRY: &str = "/newsletters/editions/retry/";

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let (_db, state, _owner) = fixture().await;
    for uri in READ_SCREENS {
        for cookie in [None, Some("vuuut-admin-session=not-a-session")] {
            let r = admin(&state, "GET", uri, cookie, Some(ORIGIN), None).await;
            assert!(
                r.status.is_redirection() && r.location.contains("signin")
                    || r.status == StatusCode::UNAUTHORIZED,
                "GET {uri} answered {} to a stranger",
                r.status
            );
            assert!(!r.body.contains("nl-tile-value"), "{uri} showed the figures to a stranger");
            assert!(!r.body.contains("Delivery &amp; measurement"), "{uri} leaked the setup");
        }
    }
    // And the retry, with an empty body — the extractor must not answer
    // before the guard does.
    let r = admin(&state, "POST", RETRY, None, Some(ORIGIN), Some("")).await;
    assert!(
        r.status.is_redirection() && r.location.contains("signin")
            || r.status == StatusCode::UNAUTHORIZED,
        "the retry answered {} to a stranger rather than sending them to signin",
        r.status
    );
}

#[tokio::test]
async fn a_contributor_an_author_and_an_editor_are_all_refused() {
    let (_db, state, _owner) = fixture().await;
    for (i, role) in ["Contributor", "Author", "Editor"].iter().enumerate() {
        let user = staff(&state, &format!("Staff{i}"), &format!("staff{i}@example.test"), role).await;
        let cookie = cookie_for(&vuuut_db::repos::sessions::create(&state.db, &user).await.unwrap());
        for uri in READ_SCREENS {
            let r = get(&state, uri, &cookie).await;
            assert!(
                r.status.is_redirection() || r.status == StatusCode::UNAUTHORIZED
                    || r.body.contains("Not allowed"),
                "{role} opened {uri} and got {}",
                r.status
            );
            assert!(!r.body.contains("nl-tile-value"), "{role} saw the figures on {uri}");
        }
    }
}

#[tokio::test]
async fn a_retry_from_another_origin_is_refused_and_sends_nothing() {
    // Sending email costs money per recipient and spends the site's sending
    // reputation. A cross-site POST that re-sent a newsletter would be
    // somebody else spending both.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let p = post(&state, "The letter").await;
    let e = edition(&state, &p, "failed", "The letter").await;

    // A forged Origin, and `Origin: null` as a sandboxed frame sends. A
    // *missing* Origin is deliberately accepted — see
    // `vuuut_api::forms::same_origin`, which treats it as same-origin because
    // a non-browser client omits it, and which the whole admin relies on.
    for origin in ["https://evil.test", "null"] {
        let r = admin(&state, "POST", RETRY, Some(&cookie), Some(origin), Some(&format!("id={e}")))
            .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "origin {origin} was accepted");
        assert_eq!(recipient_rows(&state, &e).await, 0, "a refused retry still sent");
    }
}

// ---------------------------------------------------------------------------
// the card and the rail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_modules_hub_carries_a_newsletters_card() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/tools/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"href="/ghost/newsletters/""#), "no Newsletters card on the hub");
    assert!(r.body.contains("<h2>Newsletters</h2>"));
}

#[tokio::test]
async fn every_screen_of_the_module_lights_modules_in_the_rail() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    for uri in READ_SCREENS {
        let r = get(&state, uri, &cookie).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}");
        assert!(
            r.body.contains(r#"class="nav-item active" href="/ghost/tools/""#),
            "{uri} does not light Tools in the rail"
        );
    }
}

#[tokio::test]
async fn the_third_tab_points_at_the_settings_screen_that_owns_the_newsletters() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(r.body.contains(r#"class="seg" href="/ghost/newsletters/settings/""#));
    // …and that screen still works, with its subscriber counts intact after
    // the move to one query.
    member_on_the_newsletter(&state, "reader@example.test").await;
    let settings = get(&state, "/newsletters/settings/", &cookie).await;
    assert_eq!(settings.status, StatusCode::OK);
    assert!(settings.body.contains("1 subscriber"), "the settings list lost its counts");
}

// ---------------------------------------------------------------------------
// the overview
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_seeded_site_is_told_it_cannot_deliver_rather_than_told_it_is_fine() {
    // A fresh install has a newsletter, has sending on, and cannot send a
    // thing: nothing is configured to carry the mail. Both facts live on
    // different screens and neither screen says the other's half.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("gap-no-transport"), "the missing transport went unreported");
    assert!(r.body.contains("Blocking"));
    assert!(!r.body.contains("Nothing is in the way"));
    // The reason comes from the transport resolver itself, not from a guess.
    assert!(r.body.contains("Amazon SES or Mailgun"), "{}", r.body);
}

#[tokio::test]
async fn the_overview_counts_reach_and_refuses_to_invent_an_open_rate() {
    // The trap the module exists for: with open tracking off, every
    // `opened_at` is NULL, so a 0% open rate would be a measurement nobody
    // took reported as a result.
    let (_db, state, owner) = fixture().await;
    member_on_the_newsletter(&state, "a@example.test").await;
    member_on_the_newsletter(&state, "b@example.test").await;
    let cookie = owner_cookie(&state, &owner).await;

    let r = get(&state, "/newsletters/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("2 members subscribed"), "{}", r.body);
    assert!(r.body.contains("gap-opens-untracked"));
    assert!(r.body.contains("Open tracking is off"));
    // Nothing has been sent, so the summary says that rather than quoting
    // rates it has no messages to compute.
    assert!(r.body.contains("nothing emailed yet"), "{}", r.body);

    // Once something has gone out, the summary reports the attempt and still
    // refuses to turn an untracked open into a percentage.
    let p = post(&state, "The letter").await;
    edition(&state, &p, "submitted", "The letter").await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(r.body.contains("opens not measured"), "{}", r.body);
    assert!(!r.body.contains("0.0% of them opened"));
}

#[tokio::test]
async fn switching_tracking_on_turns_the_dashes_into_numbers() {
    let (_db, state, owner) = fixture_with(vec![
        ("email_track_opens".into(), serde_json::json!(true)),
        ("email_track_clicks".into(), serde_json::json!(true)),
    ])
    .await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(!r.body.contains("gap-opens-untracked"));
    assert!(!r.body.contains("gap-clicks-untracked"));
    assert!(r.body.contains(r#"<span class="chip published">On<"#), "{}", r.body);
}

#[tokio::test]
async fn switching_sending_off_is_reported_as_blocking() {
    let (_db, state, owner) = fixture_with(vec![(
        "editor_default_email_recipients".into(),
        serde_json::json!("disabled"),
    )])
    .await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(r.body.contains("gap-sending-off"));
    assert!(r.body.contains("Publishing a post emails nobody"));
}

#[tokio::test]
async fn an_archived_newsletter_is_reported_and_the_site_told_it_cannot_send() {
    let (_db, state, owner) = fixture().await;
    sqlx::query("UPDATE newsletters SET status = 'archived'").execute(&state.db).await.unwrap();
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(r.body.contains("gap-all-archived"));
    assert!(r.body.contains("Archived"));
}

#[tokio::test]
async fn a_failed_edition_is_reported_on_the_overview_with_the_way_to_fix_it() {
    let (_db, state, owner) = fixture().await;
    let p = post(&state, "The letter").await;
    edition(&state, &p, "failed", "The letter").await;
    let cookie = owner_cookie(&state, &owner).await;

    let r = get(&state, "/newsletters/", &cookie).await;
    assert!(r.body.contains("gap-failed-editions"));
    assert!(r.body.contains("without mailing anybody twice"));
    assert!(r.body.contains(r#"href="/ghost/newsletters/editions/""#));
    // The latest-editions card shows it, and offers no button there.
    assert!(r.body.contains("Latest editions"));
    assert!(!r.body.contains("Send again"), "the overview card grew a button");
}

#[tokio::test]
async fn the_window_is_reduced_to_one_the_screen_offers() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    for (asked, shown) in [("7", "Sent in 7 days"), ("90", "Sent in 90 days")] {
        let r = get(&state, &format!("/newsletters/?days={asked}"), &cookie).await;
        assert_eq!(r.status, StatusCode::OK);
        assert!(r.body.contains(shown), "{asked} was not honoured");
    }
    for forged in ["4000", "-1", "%27%3B+DROP+TABLE+emails%3B+--", ""] {
        let r = get(&state, &format!("/newsletters/?days={forged}"), &cookie).await;
        assert_eq!(r.status, StatusCode::OK, "{forged}");
        assert!(r.body.contains("Sent in 30 days"), "{forged} was not reduced to the default");
    }
}

// ---------------------------------------------------------------------------
// editions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_editions_screen_is_empty_before_anything_is_sent_and_says_why() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = get(&state, "/newsletters/editions/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Nothing emailed yet"));
    assert!(!r.body.contains("Send again"));
}

#[tokio::test]
async fn an_edition_is_listed_with_what_became_of_it_and_a_failure_offers_a_retry() {
    let (_db, state, owner) = fixture().await;
    let good = post(&state, "It worked").await;
    let bad = post(&state, "It did not").await;
    edition(&state, &good, "submitted", "It worked").await;
    let failed = edition(&state, &bad, "failed", "It did not").await;
    let cookie = owner_cookie(&state, &owner).await;

    let r = get(&state, "/newsletters/editions/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("It worked"));
    assert!(r.body.contains("It did not"));
    assert!(r.body.contains(">Sent<"));
    assert!(r.body.contains(">Failed<"));
    // The provider's own words, which is what somebody needs to act.
    assert!(r.body.contains("The provider refused the domain"));
    // Exactly one retry form, for the failed one. Counted by its action
    // rather than by its label, which also appears in the button's tooltip.
    assert_eq!(r.body.matches(r#"action="/ghost/newsletters/editions/retry/""#).count(), 1);
    assert!(r.body.contains(&format!(r#"value="{failed}""#)));
}

#[tokio::test]
async fn a_retry_of_an_edition_that_did_not_fail_is_refused_and_sends_nothing() {
    // The refusal that matters most: a retry of a succeeded send would mail
    // the whole list a second time. The handler re-reads the row rather than
    // trusting the page that drew the button, because the send loop may have
    // finished it in between.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    for status in ["submitted", "pending"] {
        let p = post(&state, &format!("A {status} edition")).await;
        let e = edition(&state, &p, status, "Subject").await;
        let r = admin(&state, "POST", RETRY, Some(&cookie), Some(ORIGIN), Some(&format!("id={e}")))
            .await;
        assert!(r.status.is_redirection(), "{status} answered {}", r.status);
        assert!(r.location.contains("error="), "{status} was not refused: {}", r.location);
        assert!(
            r.location.contains("nothing+to+send+again"),
            "{status} gave the wrong reason: {}",
            r.location
        );
        assert_eq!(recipient_rows(&state, &e).await, 0, "a refused retry still sent");
    }
}

#[tokio::test]
async fn a_retry_of_something_that_is_not_an_edition_is_refused() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    // A forged id, an id of the right shape that names nothing, and no id at
    // all. None of them may reach the send path.
    for (body, expected) in [
        ("id=../../etc/passwd", "not+an+edition"),
        ("id=0123456789abcdef01234567", "no+longer+exists"),
        ("", "not+an+edition"),
    ] {
        let r = admin(&state, "POST", RETRY, Some(&cookie), Some(ORIGIN), Some(body)).await;
        assert!(r.status.is_redirection(), "{body:?} answered {}", r.status);
        assert!(r.location.contains(expected), "{body:?} gave: {}", r.location);
    }
}

#[tokio::test]
async fn a_retry_with_nobody_left_to_reach_says_so_rather_than_claiming_success() {
    // A failed edition with no `failed` recipient rows has nothing to retry.
    // The message comes from the send path and names the real obstacle.
    let (_db, state, owner) = fixture().await;
    let p = post(&state, "The letter").await;
    let e = edition(&state, &p, "failed", "The letter").await;
    let cookie = owner_cookie(&state, &owner).await;

    let r = admin(&state, "POST", RETRY, Some(&cookie), Some(ORIGIN), Some(&format!("id={e}"))).await;
    assert!(r.status.is_redirection());
    assert!(r.location.contains("error="), "a retry with nothing to do reported success: {}", r.location);
    assert_eq!(recipient_rows(&state, &e).await, 0);
}

#[tokio::test]
async fn a_page_number_outside_the_data_reads_as_the_first_page() {
    let (_db, state, owner) = fixture().await;
    let p = post(&state, "The only one").await;
    edition(&state, &p, "submitted", "The only one").await;
    let cookie = owner_cookie(&state, &owner).await;

    for forged in ["2", "0", "-1", "99999", "two", ""] {
        let r = get(&state, &format!("/newsletters/editions/?page={forged}"), &cookie).await;
        assert_eq!(r.status, StatusCode::OK, "{forged}");
        assert!(r.body.contains("The only one"), "{forged} showed an empty page");
    }
    // One page of data draws no pager at all.
    let r = get(&state, "/newsletters/editions/", &cookie).await;
    assert!(!r.body.contains("nl-pager"));
}

#[tokio::test]
async fn a_providers_error_cannot_put_markup_on_the_page() {
    // The only genuinely remote text on either screen: whatever a mail
    // provider chose to return.
    let (_db, state, owner) = fixture().await;
    let p = post(&state, "The letter").await;
    let e = edition(&state, &p, "failed", "The letter").await;
    sqlx::query("UPDATE emails SET error = ? WHERE id = ?")
        .bind(r#"<img src=x onerror=alert(1)>"#)
        .bind(&e)
        .execute(&state.db)
        .await
        .unwrap();
    let cookie = owner_cookie(&state, &owner).await;

    let r = get(&state, "/newsletters/editions/", &cookie).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains("<img src=x"), "a provider's error reached the page as markup");
    assert!(r.body.contains("&lt;img src=x"));
}
