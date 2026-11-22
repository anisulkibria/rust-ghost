//! The Security module end to end: who is refused, what the report says about
//! a site, and whether the enforcement the settings promise actually happens.
//!
//! The negative tests are the point of this file, and there are two kinds:
//!
//! * **Refusals** — who cannot open the screens, and who cannot drive the
//!   revoke buttons from another origin. This is the most useful page in the
//!   admin to somebody who has got hold of a Contributor account, so the
//!   first test is the one that says they cannot have it.
//! * **Enforcement** — that the settings are not decoration. A session past
//!   its maximum age is actually refused; a suspended account's sessions are
//!   actually gone; the headers are actually on the response; a forwarded
//!   address is actually ignored when the site says to ignore it. Each of
//!   these was a real hole before the module existed, and a test that only
//!   checked the switch's stored value would have passed throughout.
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

async fn fixture() -> (ghost_db::TestDb, ghost_api::AppState, String) {
    fixture_with(vec![]).await
}

async fn fixture_with(
    extra_settings: Vec<(String, serde_json::Value)>,
) -> (ghost_db::TestDb, ghost_api::AppState, String) {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let mut values: Vec<(String, serde_json::Value)> = vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Security test")),
    ];
    values.extend(extra_settings);
    let settings = Arc::new(ghost_db::SettingsCache::from_values(values));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": std::env::temp_dir()
            .join(format!("ghost-security-test-{}", ghost_core::ids::new_id()))
            .to_string_lossy()}
    }))
    .unwrap();
    let routes = ghost_routing::routes_yaml::parse_routes_yaml(
        ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML,
    )
    .unwrap();
    let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
    let theme_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
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
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(
            ghost_routing::redirects::Redirects::default(),
        ))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(ghost_db::limits::Limits::default()),
        // Empty: a fixture firewall lets everybody through.
        firewall: ghost_db::repos::firewall::RuleCache::empty(),
    };
    (test_db, state, owner.id)
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
    headers: axum::http::HeaderMap,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

async fn admin(
    state: &ghost_api::AppState,
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
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let location =
        headers.get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body =
        String::from_utf8_lossy(&to_bytes(response.into_body(), 5_000_000).await.unwrap()).into_owned();
    Reply { status, location, body, headers }
}

/// A staff user with a role, for the role-refusal tests.
async fn staff(state: &ghost_api::AppState, name: &str, email: &str, role: &str) -> String {
    let id = ghost_core::ids::new_id();
    let now = ghost_core::dates::now_db();
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
        .bind(ghost_core::ids::new_id())
        .bind(&role_id)
        .bind(&id)
        .execute(&state.db)
        .await
        .unwrap();
    id
}

async fn session_for(state: &ghost_api::AppState, user: &str) -> String {
    ghost_db::repos::sessions::create(&state.db, user).await.unwrap()
}

/// Back-date a session's `created_at` and `updated_at`, to test the policy
/// without sleeping for a week.
async fn age_session(state: &ghost_api::AppState, sid: &str, secs: i64) {
    let then = ghost_core::dates::to_db_string(
        &(chrono::Utc::now() - chrono::Duration::seconds(secs)),
    );
    sqlx::query("UPDATE sessions SET created_at = ?, updated_at = ? WHERE session_id = ?")
        .bind(&then)
        .bind(&then)
        .bind(sid)
        .execute(&state.db)
        .await
        .unwrap();
}

fn cookie_for(sid: &str) -> String {
    format!("vuuut-admin-session={sid}")
}

const READ_SCREENS: &[&str] = &["/security/", "/security/sessions/", "/settings/security/"];
const WRITE_SCREENS: &[&str] = &[
    "/security/sessions/revoke/",
    "/security/sessions/revoke-all/",
    "/security/lockouts/clear/",
    "/security/lockouts/clear-all/",
    "/settings/security/",
];

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let (_db, state, _owner) = fixture().await;
    for uri in READ_SCREENS.iter().chain(WRITE_SCREENS) {
        let method = if WRITE_SCREENS.contains(uri) { "POST" } else { "GET" };
        for cookie in [None, Some("vuuut-admin-session=not-a-session")] {
            let r = admin(
                &state,
                method,
                uri,
                cookie,
                Some(ORIGIN),
                (method == "POST").then_some(""),
            )
            .await;
            assert!(
                r.status.is_redirection() && r.location.contains("signin")
                    || r.status == StatusCode::UNAUTHORIZED,
                "{method} {uri} answered {} to a stranger",
                r.status
            );
            // Nothing from the report leaks into a refusal page.
            assert!(!r.body.contains("sec-score"), "{uri} showed the posture to a stranger");
            assert!(!r.body.contains("Signed in now"), "{uri} showed the sessions to a stranger");
        }
    }
}

#[tokio::test]
async fn the_module_is_closed_to_every_staff_role_below_administrator() {
    // This page is a list of the site's weaknesses with a link to each one,
    // plus the names and sign-in times of every staff member. It is the most
    // useful page in the admin to somebody who has got a Contributor account,
    // which is the whole reason for this test.
    let (_db, state, _owner) = fixture().await;
    for role in ["Contributor", "Author", "Editor"] {
        let id = staff(&state, role, &format!("{}@example.test", role.to_lowercase()), role).await;
        let cookie = cookie_for(&session_for(&state, &id).await);
        for uri in READ_SCREENS {
            let r = admin(&state, "GET", uri, Some(&cookie), None, None).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{role} could open {uri}");
            assert!(r.body.contains("Not allowed"), "{role} got something other than a refusal at {uri}");
            assert!(!r.body.contains("sec-score"), "{role} saw the posture at {uri}");
        }
        // And the module is not advertised to them either — rule 8's intent
        // is that no link leads to a refusal.
        let modules = admin(&state, "GET", "/tools/", Some(&cookie), None, None).await;
        assert!(
            !modules.body.contains(r#"href="/ghost/security/""#),
            "{role} was offered a link to a page they cannot open"
        );
    }
}

#[tokio::test]
async fn a_revoke_driven_from_another_site_is_refused_and_signs_nobody_out() {
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    let theirs = session_for(&state, &owner).await;

    for (uri, body) in [
        ("/security/sessions/revoke/", format!("session_id={theirs}")),
        ("/security/sessions/revoke-all/", String::new()),
        ("/security/lockouts/clear-all/", String::new()),
    ] {
        let forged = admin(
            &state,
            "POST",
            uri,
            Some(&cookie_for(&mine)),
            Some("https://evil.test"),
            Some(&body),
        )
        .await;
        assert_eq!(forged.status, StatusCode::FORBIDDEN, "{uri} accepted a cross-site POST");
    }
    // Both sessions survived.
    let live = ghost_db::repos::security::sessions(&state.db, 50).await.unwrap();
    assert_eq!(live.len(), 2, "a cross-site request signed somebody out");
}

// ---------------------------------------------------------------------------
// the report
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_module_is_on_the_modules_page_and_in_settings() {
    let (_db, state, owner) = fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let modules = admin(&state, "GET", "/tools/", Some(&cookie), None, None).await;
    assert!(
        modules.body.contains(r#"href="/ghost/security/""#),
        "Security is not on the Tools page"
    );
    let settings = admin(&state, "GET", "/settings/", Some(&cookie), None, None).await;
    assert!(
        settings.body.contains(r#"href="/ghost/settings/security/""#),
        "Security is not on the Settings page"
    );
}

#[tokio::test]
async fn the_report_names_the_two_findings_that_make_everything_else_advisory() {
    // An http site with forwarded addresses believed. Both are Critical and
    // both must be on the page with a link to the switch that fixes them.
    let (_db, state, owner) = fixture_with(vec![
        ("site_url".into(), serde_json::json!("http://example.test")),
        ("security_trust_proxy".into(), serde_json::json!(true)),
    ])
    .await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"id="finding-trust_proxy""#), "the forwarded-address finding is missing");
    assert!(r.body.contains(r#"href="/ghost/settings/security/""#));
    assert!(r.body.contains("Critical"));
    // The score ring is drawn and labelled.
    assert!(r.body.contains("Security posture"));
}

#[tokio::test]
async fn a_hardened_site_reports_a_strong_posture() {
    let (_db, state, owner) = fixture_with(vec![
        ("security_headers".into(), serde_json::json!(true)),
        ("security_hsts".into(), serde_json::json!(true)),
        ("security_trust_proxy".into(), serde_json::json!(false)),
        ("security_private_fetch".into(), serde_json::json!(true)),
        ("security_session_days".into(), serde_json::json!("7")),
        ("security_session_idle_minutes".into(), serde_json::json!("240")),
        ("security_min_password_length".into(), serde_json::json!("16")),
        ("require_email_mfa".into(), serde_json::json!(true)),
    ])
    .await;
    // Two administrators, so the single-administrator note does not fire.
    staff(&state, "Second", "second@example.test", "Administrator").await;
    // The seed ships the built-in integrations, each with an Admin API key,
    // and the report is right to note them — a key acts as a staff account
    // and no session policy or 2FA covers it. Cleared here so the
    // "nothing to report" state is reachable at all.
    sqlx::query("DELETE FROM api_keys").execute(&state.db).await.unwrap();
    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert!(r.body.contains("Nothing to report"), "a hardened site still had findings");
    assert!(!r.body.contains(r#"id="finding-trust_proxy""#));
}

#[tokio::test]
async fn a_lockout_key_holding_markup_is_escaped_on_the_page() {
    // A `brute` key is built from a client address and, for the reset
    // budgets, an email address a stranger typed. Both reach this page.
    let (_db, state, owner) = fixture().await;
    let hostile = r#"<img src=x onerror=alert(1)>@example.comreset"#;
    sqlx::query(
        "INSERT INTO brute (`key`, firstRequest, lastRequest, lifetime, count) VALUES (?, ?, ?, 3600, 9)",
    )
    .bind(hostile)
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&state.db)
    .await
    .unwrap();

    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(&state, "GET", "/security/sessions/", Some(&cookie), None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    // `esc` leaves `onerror=alert(1)` alone as text — it is only dangerous
    // inside a tag — so the assertion is about the tag.
    assert!(!r.body.contains("<img src=x"), "a hostile lockout key was rendered as markup");
    assert!(r.body.contains("&lt;img src=x"), "the key is not on the page at all");
    assert!(r.body.contains("Password reset"), "the key's kind was not read");
}

#[tokio::test]
async fn a_session_id_never_appears_on_the_page_except_in_the_form_that_revokes_it() {
    // The rule the sessions screen rests on: a screenshot of it must not be a
    // working login.
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    let other_user = staff(&state, "Ada", "ada@example.test", "Editor").await;
    let theirs = session_for(&state, &other_user).await;

    let r = admin(&state, "GET", "/security/sessions/", Some(&cookie_for(&mine)), None, None).await;
    assert_eq!(r.status, StatusCode::OK);

    // The only occurrence of the other session's full id is the hidden field
    // of its own revoke form.
    assert_eq!(r.body.matches(theirs.as_str()).count(), 1, "the id appears more than once");
    assert!(
        r.body.contains(&format!(r#"<input type="hidden" name="session_id" value="{theirs}">"#)),
        "the one occurrence is not the revoke form's hidden field"
    );
    // The reader's own id is not on the page at all: their row offers no
    // revoke button, so there is nothing to carry it.
    assert!(!r.body.contains(mine.as_str()), "the reader's own session id is on the page");
    // The short form is what is drawn.
    assert!(r.body.contains(&theirs[..6]));
}

// ---------------------------------------------------------------------------
// the actions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_is_revoked_and_that_browser_is_signed_out() {
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    let other_user = staff(&state, "Ada", "ada@example.test", "Editor").await;
    let theirs = session_for(&state, &other_user).await;

    // Ada can reach the admin before.
    let before = admin(&state, "GET", "/posts/", Some(&cookie_for(&theirs)), None, None).await;
    assert_eq!(before.status, StatusCode::OK);

    let revoked = admin(
        &state,
        "POST",
        "/security/sessions/revoke/",
        Some(&cookie_for(&mine)),
        Some(ORIGIN),
        Some(&format!("session_id={theirs}")),
    )
    .await;
    assert!(revoked.status.is_redirection(), "{}", revoked.status);
    assert!(revoked.location.contains("ok="), "{}", revoked.location);

    // And not after.
    let after = admin(&state, "GET", "/posts/", Some(&cookie_for(&theirs)), None, None).await;
    assert!(
        after.status.is_redirection() && after.location.contains("signin"),
        "a revoked session still reached the admin: {}",
        after.status
    );
}

#[tokio::test]
async fn revoking_your_own_session_is_refused_rather_than_signing_you_out() {
    // A button that signs you out of the page you are using reads as a bug
    // the moment it works, so the screen does not offer one and the handler
    // refuses it anyway.
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    let r = admin(
        &state,
        "POST",
        "/security/sessions/revoke/",
        Some(&cookie_for(&mine)),
        Some(ORIGIN),
        Some(&format!("session_id={mine}")),
    )
    .await;
    assert!(r.location.contains("error="), "{}", r.location);
    let still = admin(&state, "GET", "/posts/", Some(&cookie_for(&mine)), None, None).await;
    assert_eq!(still.status, StatusCode::OK, "the reader signed themselves out");
}

#[tokio::test]
async fn a_forged_session_id_never_reaches_a_delete() {
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    let other_user = staff(&state, "Ada", "ada@example.test", "Editor").await;
    session_for(&state, &other_user).await;

    for hostile in ["' OR 1=1 --", "%", "", "0123456789abcdef0123456789abcdeg", "../../etc/passwd"] {
        let r = admin(
            &state,
            "POST",
            "/security/sessions/revoke/",
            Some(&cookie_for(&mine)),
            Some(ORIGIN),
            Some(&format!("session_id={}", urlencoding(hostile))),
        )
        .await;
        assert!(r.status.is_redirection(), "{hostile} answered {}", r.status);
        assert!(r.location.contains("error="), "{hostile} was accepted: {}", r.location);
    }
    // Nothing was deleted along the way.
    assert_eq!(ghost_db::repos::security::sessions(&state.db, 50).await.unwrap().len(), 2);
}

fn urlencoding(s: &str) -> String {
    form_urlencoded::Serializer::new(String::new()).append_pair("x", s).finish()[2..].to_string()
}

#[tokio::test]
async fn signing_out_every_other_session_keeps_the_readers_own() {
    let (_db, state, owner) = fixture().await;
    let mine = session_for(&state, &owner).await;
    for i in 0..3 {
        let id = staff(&state, &format!("User{i}"), &format!("u{i}@example.test"), "Editor").await;
        session_for(&state, &id).await;
    }
    assert_eq!(ghost_db::repos::security::sessions(&state.db, 50).await.unwrap().len(), 4);

    let r = admin(
        &state,
        "POST",
        "/security/sessions/revoke-all/",
        Some(&cookie_for(&mine)),
        Some(ORIGIN),
        Some(""),
    )
    .await;
    assert!(r.location.contains("ok="), "{}", r.location);
    let left = ghost_db::repos::security::sessions(&state.db, 50).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].session_id, mine, "the reader was signed out of their own session");
}

#[tokio::test]
async fn a_lockout_is_cleared_so_somebody_locked_out_can_try_again() {
    let (_db, state, owner) = fixture().await;
    let key = "user_login:203.0.113.9";
    // Spend the login budget.
    for _ in 0..8 {
        let _ = ghost_api::brute::check(&state.db, key, &ghost_api::brute::USER_LOGIN).await;
    }
    assert!(
        ghost_api::brute::check(&state.db, key, &ghost_api::brute::USER_LOGIN).await.is_err(),
        "the budget was not spent"
    );

    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(
        &state,
        "POST",
        "/security/lockouts/clear/",
        Some(&cookie),
        Some(ORIGIN),
        Some(&format!("key={}", urlencoding(key))),
    )
    .await;
    assert!(r.location.contains("ok="), "{}", r.location);
    assert!(
        ghost_api::brute::check(&state.db, key, &ghost_api::brute::USER_LOGIN).await.is_ok(),
        "the lockout was not actually cleared"
    );
}

// ---------------------------------------------------------------------------
// enforcement — the part that would still pass if the settings were decoration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_session_past_its_maximum_age_is_refused_and_the_row_is_gone() {
    // Before this module the session cookie carried a seven-day `Max-Age` and
    // the row behind it lived forever, so an id read out of the database was a
    // login with no expiry date.
    let (_db, state, owner) =
        fixture_with(vec![("security_session_days".into(), serde_json::json!("7"))]).await;
    let sid = session_for(&state, &owner).await;

    // Six days old: still good.
    age_session(&state, &sid, 6 * 86_400).await;
    let ok = admin(&state, "GET", "/posts/", Some(&cookie_for(&sid)), None, None).await;
    assert_eq!(ok.status, StatusCode::OK, "a six-day-old session was refused under a seven-day policy");

    // Eight days old: refused, and swept on the way out.
    age_session(&state, &sid, 8 * 86_400).await;
    let expired = admin(&state, "GET", "/posts/", Some(&cookie_for(&sid)), None, None).await;
    assert!(
        expired.status.is_redirection() && expired.location.contains("signin"),
        "an expired session reached the admin: {}",
        expired.status
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE session_id = ?")
        .bind(&sid)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "the expired session was refused but left in the table");
}

#[tokio::test]
async fn an_idle_session_is_refused_once_an_idle_timeout_is_set() {
    let (_db, state, owner) = fixture_with(vec![
        ("security_session_days".into(), serde_json::json!("30")),
        ("security_session_idle_minutes".into(), serde_json::json!("60")),
    ])
    .await;
    let sid = session_for(&state, &owner).await;
    // Two hours unused, well inside the thirty-day maximum.
    age_session(&state, &sid, 2 * 3600).await;
    let r = admin(&state, "GET", "/posts/", Some(&cookie_for(&sid)), None, None).await;
    assert!(
        r.status.is_redirection() && r.location.contains("signin"),
        "an idle session reached the admin: {}",
        r.status
    );
}

#[tokio::test]
async fn an_ordinary_session_is_not_expired_by_an_idle_timeout_for_want_of_an_updated_at() {
    // `updated_at` is written only when the session data changes, which for a
    // session that never needed an emailed code is never. Reading a null as
    // "last used at the dawn of time" would expire every ordinary session the
    // moment an idle timeout was switched on — which is the shape of bug that
    // locks every administrator out of their own site at once.
    let (_db, state, owner) = fixture_with(vec![
        ("security_session_idle_minutes".into(), serde_json::json!("15")),
    ])
    .await;
    let sid = session_for(&state, &owner).await;
    sqlx::query("UPDATE sessions SET updated_at = NULL WHERE session_id = ?")
        .bind(&sid)
        .execute(&state.db)
        .await
        .unwrap();
    let r = admin(&state, "GET", "/posts/", Some(&cookie_for(&sid)), None, None).await;
    assert_eq!(r.status, StatusCode::OK, "a fresh session with no updated_at was expired");
}

#[tokio::test]
async fn suspending_an_account_signs_it_out_of_the_browser_it_is_already_in() {
    // A suspension has to reach a browser that is already signed in.
    // `verify_password` refuses a *new* sign-in from a non-active account, but
    // nothing in the session guard looked at `status` — so suspending
    // somebody sitting in the admin did nothing until they chose to sign out.
    let (_db, state, owner) = fixture().await;
    let mine = cookie_for(&session_for(&state, &owner).await);
    let ada = staff(&state, "Ada", "ada@example.test", "Editor").await;
    let theirs = cookie_for(&session_for(&state, &ada).await);

    let before = admin(&state, "GET", "/posts/", Some(&theirs), None, None).await;
    assert_eq!(before.status, StatusCode::OK);

    let suspended = admin(
        &state,
        "POST",
        "/staffs/ada/status/",
        Some(&mine),
        Some(ORIGIN),
        Some("status=inactive"),
    )
    .await;
    assert!(suspended.status.is_redirection(), "{} {}", suspended.status, suspended.body);

    let after = admin(&state, "GET", "/posts/", Some(&theirs), None, None).await;
    assert!(
        after.status.is_redirection() && after.location.contains("signin"),
        "a suspended account still reached the admin: {}",
        after.status
    );
    // And the report has nothing to flag, because the row is gone rather than
    // merely refused.
    assert_eq!(
        ghost_db::repos::security::suspended_with_sessions(&state.db).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn admin_responses_carry_the_security_headers_when_the_switch_is_on() {
    let (_db, state, owner) =
        fixture_with(vec![("security_headers".into(), serde_json::json!(true))]).await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert_eq!(r.header("x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(r.header("content-security-policy"), Some("frame-ancestors 'self'"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("referrer-policy"), Some("same-origin"));
    assert_eq!(r.header("cache-control"), Some("no-store, max-age=0"));
    // The site URL is https here, and HSTS is off by default, so it must not
    // be sent just because the other headers are on.
    assert_eq!(r.header("strict-transport-security"), None);
}

#[tokio::test]
async fn the_sign_in_screen_carries_them_too() {
    // The signed-out screens are the ones worth framing: a sign-in form
    // inside an invisible iframe on another site is the attack, and a header
    // that is only on the pages behind the login does not stop it.
    let (_db, state, _owner) =
        fixture_with(vec![("security_headers".into(), serde_json::json!(true))]).await;
    let r = admin(&state, "GET", "/signin/", None, None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(r.header("content-security-policy"), Some("frame-ancestors 'self'"));
}

#[tokio::test]
async fn the_public_site_gets_nosniff_but_is_not_told_it_cannot_be_framed() {
    // A publication's pages are embedded on purpose and its own Portal runs
    // in an iframe, so `X-Frame-Options` on the public site would break the
    // product. `nosniff` is the one that belongs there, and it belongs most
    // on this router: /content/files/ serves files somebody uploaded, from
    // the site's own origin.
    let (_db, state, _owner) =
        fixture_with(vec![("security_headers".into(), serde_json::json!(true))]).await;
    let app = ghost_front::site_router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri("/robots.txt")
        .header("host", HOST)
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let headers = response.headers().clone();
    assert_eq!(
        headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert!(headers.get("x-frame-options").is_none(), "the public site was told it cannot be framed");
    assert!(headers.get("content-security-policy").is_none());
}

#[tokio::test]
async fn turning_the_headers_off_actually_stops_sending_them() {
    let (_db, state, owner) =
        fixture_with(vec![("security_headers".into(), serde_json::json!(false))]).await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    let r = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert_eq!(r.header("x-frame-options"), None);
    assert_eq!(r.header("x-content-type-options"), None);
}

#[tokio::test]
async fn hsts_is_sent_on_an_https_site_and_never_on_an_http_one() {
    for (url, expected) in [
        (ORIGIN, Some("max-age=63072000; includeSubDomains")),
        ("http://example.test", None),
    ] {
        let (_db, state, owner) = fixture_with(vec![
            ("site_url".into(), serde_json::json!(url)),
            ("security_hsts".into(), serde_json::json!(true)),
        ])
        .await;
        let cookie = cookie_for(&session_for(&state, &owner).await);
        let r = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
        assert_eq!(r.header("strict-transport-security"), expected, "for {url}");
    }
}

#[tokio::test]
async fn a_forwarded_address_is_ignored_when_the_site_says_not_to_trust_one() {
    // The finding this module calls Critical. With forwarded addresses
    // believed, a caller sends a fresh header per request and every guess
    // lands on a budget key nobody has ever used — so sign-in, the emailed
    // code, password resets and the access code are all unlimited.
    let headers = |value: &str| {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", value.parse().unwrap());
        h
    };
    let socket: Option<std::net::IpAddr> = Some("198.51.100.7".parse().unwrap());

    let (_db, trusting, _) =
        fixture_with(vec![("security_trust_proxy".into(), serde_json::json!(true))]).await;
    let (_db2, strict, _) =
        fixture_with(vec![("security_trust_proxy".into(), serde_json::json!(false))]).await;

    // Trusting: each forged header is its own key, which is the hole.
    assert_eq!(
        ghost_api::brute::client_ip_for(&trusting, &headers("203.0.113.1"), socket),
        "203.0.113.1"
    );
    assert_eq!(
        ghost_api::brute::client_ip_for(&trusting, &headers("203.0.113.2"), socket),
        "203.0.113.2"
    );

    // Strict: every forged header lands on the same key — the real socket.
    for forged in ["203.0.113.1", "203.0.113.2", "127.0.0.1", "not-an-address"] {
        assert_eq!(
            ghost_api::brute::client_ip_for(&strict, &headers(forged), socket),
            "198.51.100.7",
            "a forged X-Forwarded-For of {forged} chose its own budget"
        );
    }
}

#[tokio::test]
async fn a_forged_setting_does_not_survive_a_save() {
    // The settings page offers five values for the session lifetime and four
    // for the password floor. A POST is not the page, so each is reduced to
    // something that was offered before it is stored — the password floor in
    // particular reaches every password path in the application.
    let (_db, state, owner) = fixture_with(vec![
        ("security_session_days".into(), serde_json::json!("7")),
        ("security_session_idle_minutes".into(), serde_json::json!("0")),
        ("security_min_password_length".into(), serde_json::json!("10")),
        ("security_admin_referrer_policy".into(), serde_json::json!("same-origin")),
        ("security_headers".into(), serde_json::json!(true)),
        ("security_hsts".into(), serde_json::json!(false)),
        ("security_trust_proxy".into(), serde_json::json!(true)),
        ("security_private_fetch".into(), serde_json::json!(true)),
        ("security_revoke_on_password_change".into(), serde_json::json!(true)),
        ("require_email_mfa".into(), serde_json::json!(false)),
    ])
    .await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let posted = form_urlencoded::Serializer::new(String::new())
        .append_pair("security_session_days", "99999")
        .append_pair("security_session_idle_minutes", "1")
        .append_pair("security_min_password_length", "1")
        .append_pair("security_admin_referrer_policy", "unsafe-url")
        .append_pair("security_headers", "1")
        .finish();
    let r = admin(
        &state,
        "POST",
        "/settings/security/",
        Some(&cookie),
        Some(ORIGIN),
        Some(&posted),
    )
    .await;
    assert!(r.status.is_redirection(), "{} {}", r.status, r.body);

    // Read the rows back out of the database, not out of the cache.
    let stored = |key: &'static str| {
        let db = state.db.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
                .bind(key)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(stored("security_session_days").await, "7", "a forged lifetime was stored");
    assert_eq!(stored("security_min_password_length").await, "10", "a forged password floor was stored");
    assert_eq!(
        stored("security_admin_referrer_policy").await,
        "same-origin",
        "a policy we never offered was stored"
    );
    // A value that *was* offered still saves.
    assert_eq!(stored("security_session_idle_minutes").await, "0");
}

#[tokio::test]
async fn a_saved_switch_takes_effect_without_a_restart() {
    // The settings cache is written through, so turning the headers off has
    // to change the next response rather than the next boot.
    let (_db, state, owner) = fixture_with(vec![
        ("security_headers".into(), serde_json::json!(true)),
        ("security_hsts".into(), serde_json::json!(false)),
        ("security_trust_proxy".into(), serde_json::json!(true)),
        ("security_private_fetch".into(), serde_json::json!(true)),
        ("security_revoke_on_password_change".into(), serde_json::json!(true)),
        ("security_session_days".into(), serde_json::json!("7")),
        ("security_session_idle_minutes".into(), serde_json::json!("0")),
        ("security_min_password_length".into(), serde_json::json!("10")),
        ("security_admin_referrer_policy".into(), serde_json::json!("same-origin")),
        ("require_email_mfa".into(), serde_json::json!(false)),
    ])
    .await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    let before = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert_eq!(before.header("x-frame-options"), Some("SAMEORIGIN"));

    // Save with the headers switch absent, which is how an unchecked
    // checkbox arrives.
    let posted = form_urlencoded::Serializer::new(String::new())
        .append_pair("security_session_days", "7")
        .append_pair("security_session_idle_minutes", "0")
        .append_pair("security_min_password_length", "10")
        .append_pair("security_admin_referrer_policy", "same-origin")
        .finish();
    admin(&state, "POST", "/settings/security/", Some(&cookie), Some(ORIGIN), Some(&posted)).await;

    let after = admin(&state, "GET", "/security/", Some(&cookie), None, None).await;
    assert_eq!(after.header("x-frame-options"), None, "the switch did not take effect");
}

#[tokio::test]
async fn the_password_floor_reaches_the_screen_that_sets_a_password() {
    let (_db, state, _owner) =
        fixture_with(vec![("security_min_password_length".into(), serde_json::json!("16"))]).await;
    // The setup screen is the one password form reachable without a session.
    state.setup_done.store(false, std::sync::atomic::Ordering::Relaxed);
    sqlx::query("DELETE FROM users").execute(&state.db).await.unwrap();
    let r = admin(&state, "GET", "/setup/", None, None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("min 16 characters"), "the setup form still promises ten");
    assert!(r.body.contains(r#"minlength="16""#));
}

// ---------------------------------------------------------------------------
// firewall
// ---------------------------------------------------------------------------

/// One request with a chosen client address.
///
/// The address is sent as `X-Forwarded-For`, which the fixture believes
/// because `security_trust_proxy` defaults on — so these tests are also an
/// assertion about the thing the Security report calls Critical: with
/// forwarded addresses trusted, the caller picks which firewall rule applies
/// to them.
async fn admin_from(
    state: &ghost_api::AppState,
    method: &str,
    uri: &str,
    ip: &str,
    cookie: Option<&str>,
    form: Option<&str>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST)
        .header("accept", "text/html")
        .header("x-forwarded-for", ip)
        .header("origin", ORIGIN);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let body = match form {
        Some(form) => {
            request = request.header("content-type", "application/x-www-form-urlencoded");
            Body::from(form.to_string())
        }
        None => Body::empty(),
    };
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let location = headers.get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body =
        String::from_utf8_lossy(&to_bytes(response.into_body(), 5_000_000).await.unwrap()).into_owned();
    Reply { status, location, body, headers }
}

async fn firewall_fixture() -> (ghost_db::TestDb, ghost_api::AppState, String) {
    fixture_with(vec![
        ("firewall_enabled".into(), serde_json::json!(true)),
        ("security_trust_proxy".into(), serde_json::json!(true)),
    ])
    .await
}

/// Add a rule straight to the table and refresh the cache, the way the
/// screen's handlers do.
async fn arm(state: &ghost_api::AppState, action: &str, scope: &str, pattern: &str) {
    ghost_db::repos::firewall::upsert(
        &state.db,
        &ghost_db::repos::firewall::NewRule {
            action: ghost_core::firewall::Action::parse(action),
            scope: ghost_core::firewall::Scope::parse(scope),
            pattern: ghost_core::firewall::Pattern::parse(pattern).unwrap(),
            note: "",
            created_by: None,
            expires_at: None,
        },
    )
    .await
    .unwrap();
    state.firewall.reload(&state.db).await;
}

#[tokio::test]
async fn the_firewall_screen_is_closed_to_anyone_below_administrator() {
    let (_db, state, _owner) = fixture().await;
    for role in ["Contributor", "Author", "Editor"] {
        let id = staff(&state, role, &format!("{}@example.test", role.to_lowercase()), role).await;
        let cookie = cookie_for(&session_for(&state, &id).await);
        let r = admin(&state, "GET", "/security/firewall/", Some(&cookie), None, None).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{role} could open the firewall");
    }
    // And a stranger cannot drive any of the four actions.
    for uri in [
        "/security/firewall/add/",
        "/security/firewall/delete/",
        "/security/firewall/clear/",
        "/security/firewall/toggle/",
    ] {
        let r = admin(&state, "POST", uri, None, Some(ORIGIN), Some("")).await;
        assert!(
            r.status.is_redirection() && r.location.contains("signin")
                || r.status == StatusCode::UNAUTHORIZED,
            "{uri} answered {} to a stranger",
            r.status,
        );
    }
}

#[tokio::test]
async fn a_blocked_address_is_refused_before_the_session_is_even_read() {
    // The whole point of mounting it outermost: a barred address is barred
    // whether or not it holds a valid login.
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    arm(&state, "block", "site", "203.0.113.0/24").await;

    let barred = admin_from(&state, "GET", "/posts/", "203.0.113.9", Some(&cookie), None).await;
    assert_eq!(barred.status, StatusCode::FORBIDDEN);
    assert_eq!(barred.header("cache-control"), Some("no-store"));

    // Somebody else, with the same cookie, is unaffected.
    let fine = admin_from(&state, "GET", "/posts/", "198.51.100.1", Some(&cookie), None).await;
    assert_eq!(fine.status, StatusCode::OK);
}

#[tokio::test]
async fn an_admin_scoped_rule_leaves_the_publication_readable() {
    let (_db, state, _owner) = firewall_fixture().await;
    arm(&state, "block", "admin", "203.0.113.4").await;

    let barred = admin_from(&state, "GET", "/signin/", "203.0.113.4", None, None).await;
    assert_eq!(barred.status, StatusCode::FORBIDDEN);

    // The same address on the public site.
    let app = ghost_front::site_router(state.clone());
    let request = Request::builder()
        .method("GET")
        .uri("/robots.txt")
        .header("host", HOST)
        .header("x-forwarded-for", "203.0.113.4")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "an admin-scoped rule took the publication away from a reader",
    );
}

#[tokio::test]
async fn an_allow_rule_punches_a_hole_in_a_range_block() {
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    arm(&state, "block", "site", "203.0.113.0/24").await;
    arm(&state, "allow", "site", "203.0.113.4").await;

    assert_eq!(
        admin_from(&state, "GET", "/posts/", "203.0.113.4", Some(&cookie), None).await.status,
        StatusCode::OK,
    );
    assert_eq!(
        admin_from(&state, "GET", "/posts/", "203.0.113.5", Some(&cookie), None).await.status,
        StatusCode::FORBIDDEN,
    );
}

#[tokio::test]
async fn the_switch_being_off_means_no_rule_is_enforced() {
    let (_db, state, owner) =
        fixture_with(vec![("firewall_enabled".into(), serde_json::json!(false))]).await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    arm(&state, "block", "site", "203.0.113.0/24").await;
    assert_eq!(
        admin_from(&state, "GET", "/posts/", "203.0.113.9", Some(&cookie), None).await.status,
        StatusCode::OK,
        "a rule was enforced while the firewall was off",
    );
}

#[tokio::test]
async fn a_rule_that_would_lock_the_author_out_is_refused_and_nothing_is_saved() {
    // The guard that matters most: three of the four actions on this screen
    // can lock somebody out of the page they would undo it from.
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let r = admin_from(
        &state,
        "POST",
        "/security/firewall/add/",
        "203.0.113.4",
        Some(&cookie),
        Some("pattern=203.0.113.0%2F24&action=block&scope=admin&hours=0&note="),
    )
    .await;
    assert!(r.location.contains("error="), "{}", r.location);
    assert!(r.location.contains("lock+you+out") || r.location.contains("lock%20you%20out"), "{}", r.location);
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 0, "the rule was saved anyway");
}

#[tokio::test]
async fn the_first_admin_allow_rule_is_refused_when_it_does_not_cover_the_author() {
    // The nastiest lockout, and the reason the guard simulates rather than
    // asking "is this my address": the rule names somebody *else's* address,
    // matches nobody relevant, and locks the author out by switching
    // allowlist mode on.
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let r = admin_from(
        &state,
        "POST",
        "/security/firewall/add/",
        "203.0.113.4",
        Some(&cookie),
        Some("pattern=198.51.100.0%2F24&action=allow&scope=admin&hours=0&note="),
    )
    .await;
    assert!(r.location.contains("error="), "{}", r.location);
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 0);

    // Naming the author's own range is what makes it safe.
    let ok = admin_from(
        &state,
        "POST",
        "/security/firewall/add/",
        "203.0.113.4",
        Some(&cookie),
        Some("pattern=203.0.113.0%2F24&action=allow&scope=admin&hours=0&note=the+office"),
    )
    .await;
    assert!(ok.location.contains("ok="), "{}", ok.location);
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 1);
}

#[tokio::test]
async fn removing_the_allow_rule_that_covers_you_is_refused() {
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    arm(&state, "allow", "admin", "203.0.113.0/24").await;
    arm(&state, "allow", "admin", "198.51.100.0/24").await;

    let mine = ghost_db::repos::firewall::all(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.pattern == "203.0.113.0/24")
        .unwrap();
    let r = admin_from(
        &state,
        "POST",
        "/security/firewall/delete/",
        "203.0.113.4",
        Some(&cookie),
        Some(&format!("id={}", mine.id)),
    )
    .await;
    assert!(r.location.contains("error="), "{}", r.location);
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 2);
}

#[tokio::test]
async fn adding_the_first_rule_turns_the_firewall_on() {
    // A stored rule that is quietly not enforced is worse than no rule,
    // because the screen says it is there.
    let (_db, state, owner) =
        fixture_with(vec![("firewall_enabled".into(), serde_json::json!(false))]).await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    assert!(!state.settings.get_bool("firewall_enabled"));

    let r = admin_from(
        &state,
        "POST",
        "/security/firewall/add/",
        "203.0.113.4",
        Some(&cookie),
        Some("pattern=198.51.100.0%2F24&action=block&scope=admin&hours=0&note="),
    )
    .await;
    assert!(r.location.contains("ok="), "{}", r.location);
    assert!(state.settings.get_bool("firewall_enabled"), "the firewall was left off");
}

#[tokio::test]
async fn a_pattern_that_is_not_an_address_is_refused() {
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    for bad in ["example.com", "nonsense", "203.0.113.4%2F33", "", "203.0.113.999"] {
        let r = admin_from(
            &state,
            "POST",
            "/security/firewall/add/",
            "203.0.113.4",
            Some(&cookie),
            Some(&format!("pattern={bad}&action=block&scope=admin&hours=0&note=")),
        )
        .await;
        assert!(r.location.contains("error="), "{bad:?} was accepted: {}", r.location);
    }
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 0);
}

#[tokio::test]
async fn clearing_every_rule_is_always_allowed_because_it_is_the_undo() {
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    arm(&state, "allow", "admin", "198.51.100.0/24").await;

    // The author is locked out by allowlist mode right now — and clearing is
    // still allowed, because an undo that can refuse is not one. They reach
    // the handler from an address the rule does admit.
    let r = admin_from(
        &state,
        "POST",
        "/security/firewall/clear/",
        "198.51.100.1",
        Some(&cookie),
        Some(""),
    )
    .await;
    assert!(r.location.contains("ok="), "{}", r.location);
    assert_eq!(ghost_db::repos::firewall::count(&state.db).await.unwrap(), 0);
}

#[tokio::test]
async fn a_blocked_user_agent_is_refused_and_an_absent_one_is_not() {
    let (_db, state, owner) = fixture_with(vec![
        ("spam_block_user_agents".into(), serde_json::json!(true)),
        ("spam_blocked_user_agents".into(), serde_json::json!("badbot\n# a comment\nscrapey")),
    ])
    .await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let ua = |value: &str| {
        let state = state.clone();
        let cookie = cookie.clone();
        let value = value.to_string();
        async move {
            let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
            let request = Request::builder()
                .method("GET")
                .uri("/posts/")
                .header("host", HOST)
                .header("accept", "text/html")
                .header("cookie", cookie)
                .header("user-agent", value)
                .body(Body::empty())
                .unwrap();
            app.oneshot(request).await.unwrap().status()
        }
    };
    assert_eq!(ua("Mozilla/5.0 (compatible; BadBot/1.0)").await, StatusCode::FORBIDDEN);
    assert_eq!(ua("scrapey-mcscrapeface").await, StatusCode::FORBIDDEN);
    assert_eq!(ua("Mozilla/5.0 (Macintosh)").await, StatusCode::OK);
    // An empty one is not blocked: feed readers and monitoring send none.
    assert_eq!(ua("").await, StatusCode::OK);
}

#[tokio::test]
async fn the_firewall_screen_shows_a_rule_with_its_note_escaped() {
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);
    ghost_db::repos::firewall::upsert(
        &state.db,
        &ghost_db::repos::firewall::NewRule {
            action: ghost_core::firewall::Action::Block,
            scope: ghost_core::firewall::Scope::Admin,
            pattern: ghost_core::firewall::Pattern::parse("198.51.100.0/24").unwrap(),
            note: &ghost_core::firewall::clean_note(r#"<img src=x onerror=alert(1)>"#),
            created_by: None,
            expires_at: None,
        },
    )
    .await
    .unwrap();

    let r = admin(&state, "GET", "/security/firewall/", Some(&cookie), None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains("<img src=x"), "a note was rendered as markup");
    assert!(r.body.contains("&lt;img src=x"));
    assert!(r.body.contains("198.51.100.0/24"));
    assert!(r.body.contains("256 addresses"));
}

#[tokio::test]
async fn the_screen_warns_in_words_when_allowlist_mode_is_on() {
    // The one piece of behaviour that is not "a rule matched", so it is
    // stated rather than left to be inferred from the list.
    let (_db, state, owner) = firewall_fixture().await;
    let cookie = cookie_for(&session_for(&state, &owner).await);

    let before = admin(&state, "GET", "/security/firewall/", Some(&cookie), None, None).await;
    assert!(!before.body.contains("Allowlist mode is on"));

    arm(&state, "allow", "admin", "203.0.113.0/24").await;
    let after = admin(&state, "GET", "/security/firewall/", Some(&cookie), None, None).await;
    assert!(after.body.contains("Allowlist mode is on"), "the mode change was not announced");
}

#[tokio::test]
async fn a_disposable_signup_address_is_refused_only_when_the_switch_is_on() {
    for (on, expected) in [(false, true), (true, false)] {
        let (_db, state, _owner) = fixture_with(vec![
            ("spam_block_disposable_email".into(), serde_json::json!(on)),
            ("members_signup_access".into(), serde_json::json!("all")),
        ])
        .await;
        let refused =
            ghost_api::firewall::signup_email_refused(&state, "someone@mailinator.com").is_none();
        assert_eq!(refused, expected, "with the switch {on}");
        // A real address is never caught either way.
        assert!(ghost_api::firewall::signup_email_refused(&state, "a@example.com").is_none());
    }
}
