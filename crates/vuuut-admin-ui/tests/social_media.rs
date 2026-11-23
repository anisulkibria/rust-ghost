//! Tools → Social → Social media end to end, without talking to any
//! platform: credentials are validated and never shown back, shares are
//! composed against each network's limit and scheduled, publishing queues
//! exactly one automatic share per account, and the share buttons reach the
//! post. Refusals first. The platform calls themselves are covered by the
//! unit tests in `vuuut_api::social_media` (the OAuth 1.0a signature against
//! X's published example).
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

const HOST: &str = "example.test";
const ORIGIN: &str = "https://example.test";

struct Fixture {
    _db: vuuut_db::TestDb,
    state: vuuut_api::AppState,
    owner: String,
}

impl Fixture {
    async fn session(&self) -> String {
        let id = vuuut_db::repos::sessions::create(&self.state.db, &self.owner).await.unwrap();
        format!("vuuut-admin-session={id}")
    }

    /// A session for somebody who is not an administrator.
    async fn author_session(&self) -> String {
        let id = vuuut_core::ids::new_id();
        sqlx::query(
            "INSERT INTO users (id, name, slug, password, email, status, created_at) \
             VALUES (?, 'An author', ?, 'unused', ?, 'active', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(format!("an-author-{id}"))
        .bind(format!("author-{id}@example.test"))
        .execute(&self.state.db)
        .await
        .unwrap();
        let role: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Author' LIMIT 1")
            .fetch_one(&self.state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)")
            .bind(vuuut_core::ids::new_id())
            .bind(&role)
            .bind(&id)
            .execute(&self.state.db)
            .await
            .unwrap();
        let session = vuuut_db::repos::sessions::create(&self.state.db, &id).await.unwrap();
        format!("vuuut-admin-session={session}")
    }

    /// A published post or page with a body, a visibility and tags.
    async fn post(&self, slug: &str, title: &str, body: &str, kind: &str, visibility: &str, tags: &[&str]) {
        let id = vuuut_core::ids::new_id();
        sqlx::query(
            "INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'published', ?, '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')",
        )
        .bind(&id)
        .bind(vuuut_core::ids::uuid_v4())
        .bind(title)
        .bind(slug)
        .bind(format!("<p>{body}</p>"))
        .bind(body)
        .bind(kind)
        .bind(visibility)
        .execute(&self.state.db)
        .await
        .unwrap();
        for (i, tag) in tags.iter().enumerate() {
            let tag_id: Option<String> = sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?")
                .bind(tag)
                .fetch_optional(&self.state.db)
                .await
                .unwrap();
            let tag_id = match tag_id {
                Some(t) => t,
                None => {
                    let t = vuuut_core::ids::new_id();
                    sqlx::query("INSERT INTO tags (id, name, slug, visibility, created_at, updated_at) VALUES (?, ?, ?, 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
                        .bind(&t)
                        .bind(tag)
                        .bind(tag)
                        .execute(&self.state.db)
                        .await
                        .unwrap();
                    t
                }
            };
            sqlx::query("INSERT INTO posts_tags (id, post_id, tag_id, sort_order) VALUES (?, ?, ?, ?)")
                .bind(vuuut_core::ids::new_id())
                .bind(&id)
                .bind(&tag_id)
                .bind(i as i64)
                .execute(&self.state.db)
                .await
                .unwrap();
        }
    }
}

async fn fixture() -> Fixture {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Social test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}}, "paths": {}
    }))
    .unwrap();
    let routes =
        vuuut_routing::routes_yaml::parse_routes_yaml(vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML)
            .unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
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
        // Empty: a fixture firewall lets everybody through.
        firewall: vuuut_db::repos::firewall::RuleCache::empty(),
    };
    Fixture { _db: test_db, state, owner: owner.id }
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

impl Reply {
    fn flash(&self) -> String {
        let Some(query) = self.location.split_once('?').map(|(_, q)| q) else { return String::new() };
        form_urlencoded::parse(query.as_bytes())
            .filter(|(key, _)| key == "ok" || key == "error")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

async fn send(
    state: &vuuut_api::AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    form: Option<&[(&str, &str)]>,
) -> Reply {
    let mut request =
        Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    let body = match form {
        Some(pairs) => {
            request = request.header("content-type", "application/x-www-form-urlencoded");
            let mut s = form_urlencoded::Serializer::new(String::new());
            for (k, v) in pairs {
                s.append_pair(k, v);
            }
            Body::from(s.finish())
        }
        None => Body::empty(),
    };
    let app = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location =
        response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &vuuut_api::AppState, uri: &str, cookie: &str) -> Reply {
    send(state, "GET", uri, Some(cookie), None, None).await
}

async fn post(
    state: &vuuut_api::AppState,
    uri: &str,
    cookie: &str,
    pairs: &[(&str, &str)],
) -> Reply {
    send(state, "POST", uri, Some(cookie), Some(ORIGIN), Some(pairs)).await
}




const SECRET: &str = "SECRET-TOKEN-do-not-show-123";

async fn account(f: &Fixture, network: &str, auto: bool) -> String {
    let creds = match network {
        "mastodon" => serde_json::json!({"instance": "https://mastodon.example", "access_token": SECRET}),
        _ => serde_json::json!({"webhook_url": format!("https://discord.com/api/webhooks/1/{SECRET}")}),
    };
    vuuut_db::repos::social_media::add_account(&f.state.db, network, "Our account", creds.as_object().unwrap(), auto).await.unwrap()
}

async fn published(f: &Fixture, title: &str) -> String {
    let id = vuuut_core::ids::new_id();
    sqlx::query("INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at, published_at) VALUES (?, ?, ?, ?, '<p>x</p>', 'Body words here.', 'post', 'published', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
        .bind(&id).bind(vuuut_core::ids::uuid_v4()).bind(title).bind(format!("p-{id}"))
        .execute(&f.state.db).await.unwrap();
    id
}

#[tokio::test]
async fn accounts_are_for_administrators_and_composing_for_editors() {
    let f = fixture().await;
    for uri in ["/social/media/", "/social/media/compose/", "/social/media/history/", "/social/media/settings/"] {
        let anon = send(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        assert_ne!(get(&f.state, uri, &f.author_session().await).await.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    let forged = send(&f.state, "POST", "/social/media/accounts/", Some(&cookie), Some("https://elsewhere.example"), Some(&[("network", "discord")])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn both_halves_of_social_say_what_they_are() {
    let f = fixture().await;
    let cookie = f.session().await;
    for (uri, current) in [("/social/", "/ghost/social/"), ("/social/media/", "/ghost/social/media/")] {
        let page = get(&f.state, uri, &cookie).await;
        assert_eq!(page.status, StatusCode::OK, "{uri}");
        assert!(page.body.contains("Social web") && page.body.contains("Social media"), "{uri}");
        assert!(page.body.contains(&format!(r#"class="sm-section on" href="{current}""#)), "{uri}: {}", page.body);
    }
}

#[tokio::test]
async fn bad_credentials_are_refused_and_stored_secrets_are_never_shown() {
    let f = fixture().await;
    let cookie = f.session().await;
    for bad in [
        vec![("network", "discord"), ("webhook_url", "https://evil.example/api/webhooks/1/x")],
        vec![("network", "mastodon"), ("instance", "http://localhost"), ("access_token", "t")],
        vec![("network", "nonsense")],
    ] {
        let r = post(&f.state, "/social/media/accounts/", &cookie, &bad).await;
        assert!(r.location.contains("error="), "{bad:?} → {}", r.location);
    }
    assert!(vuuut_db::repos::social_media::accounts(&f.state.db).await.unwrap().is_empty());

    let id = account(&f, "mastodon", false).await;
    for uri in ["/social/media/".to_string(), format!("/social/media/accounts/{id}/")] {
        let page = get(&f.state, &uri, &cookie).await;
        assert_eq!(page.status, StatusCode::OK, "{uri}");
        assert!(!page.body.contains(SECRET), "{uri} showed a stored secret");
    }
    // Toggling auto-share keeps the credentials.
    post(&f.state, &format!("/social/media/accounts/{id}/auto/"), &cookie, &[("on", "1")]).await;
    let a = vuuut_db::repos::social_media::account(&f.state.db, &id).await.unwrap().unwrap();
    assert!(a.shares_automatically());
    assert_eq!(a.creds()["access_token"], SECRET);
}

#[tokio::test]
async fn a_share_is_composed_within_the_limit_scheduled_and_cancelled() {
    let f = fixture().await;
    let cookie = f.session().await;
    let acc = account(&f, "mastodon", false).await;
    let post_id = published(&f, "A post worth sharing").await;
    let page = get(&f.state, &format!("/social/media/compose/?post={post_id}"), &cookie).await;
    assert!(page.body.contains("A post worth sharing"), "{}", page.body);
    assert!(page.body.contains("utm_source=mastodon"), "campaign tags on by default: {}", page.body);

    let field = format!("text_{acc}");
    let too_long = "word ".repeat(200);
    let r = post(&f.state, "/social/media/compose/", &cookie, &[("post", &post_id), ("account", &acc), (&field, &too_long), ("send_at", "2099-01-01T09:00")]).await;
    assert!(r.flash().contains("over Mastodon's 500"), "{}", r.flash());

    let r = post(&f.state, "/social/media/compose/", &cookie, &[("post", &post_id), ("account", &acc), (&field, "Read this https://example.test/p/"), ("send_at", "2099-01-01T09:00")]).await;
    assert!(r.flash().contains("scheduled for 2099-01-01 09:00:00"), "{}", r.flash());
    let shares = vuuut_db::repos::social_media::history(&f.state.db, Some("scheduled"), 10).await.unwrap();
    assert_eq!(shares.len(), 1);
    let history = get(&f.state, "/social/media/history/?status=scheduled", &cookie).await;
    assert!(history.body.contains("Read this https://example.test/p/"));
    post(&f.state, &format!("/social/media/shares/{}/cancel/", shares[0].id), &cookie, &[]).await;
    assert!(vuuut_db::repos::social_media::history(&f.state.db, None, 10).await.unwrap().is_empty());

    // A draft cannot be shared; a time in the past is refused.
    let draft = vuuut_core::ids::new_id();
    sqlx::query("INSERT INTO posts (id, uuid, title, slug, type, status, visibility, created_at, updated_at) VALUES (?, ?, 'Draft', ?, 'post', 'draft', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
        .bind(&draft).bind(vuuut_core::ids::uuid_v4()).bind(format!("d-{draft}")).execute(&f.state.db).await.unwrap();
    let r = post(&f.state, "/social/media/compose/", &cookie, &[("post", &draft), ("account", &acc), (&field, "x")]).await;
    assert!(r.flash().contains("Only a published post"), "{}", r.flash());
    let r = post(&f.state, "/social/media/compose/", &cookie, &[("post", &post_id), ("account", &acc), (&field, "x"), ("send_at", "2001-01-01T09:00")]).await;
    assert!(r.flash().contains("passed"), "{}", r.flash());
}

#[tokio::test]
async fn publishing_queues_one_automatic_share_per_account_and_never_twice() {
    let f = fixture().await;
    let cookie = f.session().await;
    let auto = account(&f, "discord", true).await;
    let _manual = account(&f, "mastodon", false).await;
    let id = vuuut_core::ids::new_id();
    sqlx::query("INSERT INTO posts (id, uuid, title, slug, html, plaintext, type, status, visibility, created_at, updated_at) VALUES (?, ?, 'Fresh news', ?, '<p>x</p>', 'x', 'post', 'draft', 'public', '2026-11-01 00:00:00', '2026-11-01 00:00:00')")
        .bind(&id).bind(vuuut_core::ids::uuid_v4()).bind(format!("n-{id}")).execute(&f.state.db).await.unwrap();
    let r = post(&f.state, &format!("/posts/{id}/"), &cookie, &[("title", "Fresh news"), ("status", "published"), ("markdown", "x")]).await;
    assert_ne!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.body);
    let shares = vuuut_db::repos::social_media::of_post(&f.state.db, &id).await;
    assert_eq!(shares.len(), 1, "only the auto-sharing account");
    assert_eq!((shares[0].account_id.as_str(), shares[0].trigger_kind.as_str()), (auto.as_str(), "auto"));
    assert!(shares[0].body.contains("Fresh news"));
    // Unpublish and publish again: not announced a second time.
    post(&f.state, &format!("/posts/{id}/"), &cookie, &[("title", "Fresh news"), ("status", "draft"), ("markdown", "x")]).await;
    post(&f.state, &format!("/posts/{id}/"), &cookie, &[("title", "Fresh news"), ("status", "published"), ("markdown", "x")]).await;
    assert_eq!(vuuut_db::repos::social_media::of_post(&f.state.db, &id).await.len(), 1);
}

#[tokio::test]
async fn share_buttons_reach_the_post_only_when_switched_on() {
    let f = fixture().await;
    let cookie = f.session().await;
    let mut data = serde_json::json!({"post": {"html": "<p>Body</p>", "url": "/a-post/", "title": "A & B"}});
    assert!(!vuuut_api::social_media::apply_share_buttons(&f.state, &mut data, true), "off by default");
    post(&f.state, "/social/media/settings/", &cookie, &[
        ("social_share_buttons", "1"), ("button", "x"), ("button", "copy"), ("button", "bogus"),
        ("social_share_position", "bottom"), ("social_utm", "1"), ("social_default_template", "{title} {url}"),
    ]).await;
    assert!(!vuuut_api::social_media::apply_share_buttons(&f.state, &mut data.clone(), false), "pages get none");
    assert!(vuuut_api::social_media::apply_share_buttons(&f.state, &mut data, true));
    let html = data["post"]["html"].as_str().unwrap();
    assert!(html.starts_with("<p>Body</p><nav class=\"vsb\""), "{html}");
    assert!(html.contains("x.com/intent/post?text=A%20%26%20B"));
    assert!(html.contains(r#"data-copy="https://example.test/a-post/""#), "{html}");
    assert!(!html.contains("bogus"));
}
