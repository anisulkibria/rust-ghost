use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}, Router};
use ghost_api::AppState;
use ghost_db::repos;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::{Arc, atomic::AtomicBool}};
use tokio::sync::RwLock;
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
    cookie: String,
    content_key: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../../migrations").run(&db).await.unwrap();
        ghost_db::seed::seed_if_empty(&db).await.unwrap();
        let owner = repos::users::setup_owner(&db, "Test owner", "owner@example.test", "unused-test-hash").await.unwrap();
        let sid = repos::sessions::create(&db, &owner.id).await.unwrap();
        let content_key = repos::integrations::internal_content_key(&db).await.unwrap().unwrap();
        let settings = Arc::new(ghost_db::SettingsCache::from_values([
            ("site_url".into(), json!("https://example.test")),
            ("members_signup_access".into(), json!("all")),
        ]));
        let config = serde_json::from_value(json!({
            "url": "https://example.test", "server": {},
            "database": {"connection": {"filename": ":memory:"}}, "paths": {}
        })).unwrap();
        let routes = ghost_routing::routes_yaml::parse_routes_yaml(ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML).unwrap();
        let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
        let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/source");
        let deps = ghost_render::HelperDeps {
            pool: Arc::new(db.clone()), runtime: tokio::runtime::Handle::current(),
            url_resolver: ghost_api::themes::url_resolver(router.clone()),
            asset_hashes: Arc::new(HashMap::new()), global_asset_hash: "test".into(),
            theme_dir: theme_dir.clone(), site_origin: "https://example.test".into(), content_api_key: content_key.clone(),
        };
        let theme = ghost_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
        let state = AppState {
            db, config: Arc::new(config), settings,
            theme: Arc::new(RwLock::new(Arc::new(theme))), router: Arc::new(RwLock::new(router)),
            redirects: Arc::new(RwLock::new(Arc::new(ghost_routing::redirects::Redirects::default()))),
            setup_done: Arc::new(AtomicBool::new(true)), frontend_key: Arc::new(RwLock::new(Some(content_key.clone()))),
        };
        let app = Router::new().nest("/admin", ghost_api::admin::admin_router().merge(ghost_api::automations::automations_router()).merge(ghost_api::admin_comments::admin_comments_router()))
            .nest("/content", ghost_api::content::content_router())
            .merge(ghost_api::members::members_router()).with_state(state.clone());
        Self {state, app, cookie: format!("ghost-admin-api-session={sid}"), content_key}
    }

    async fn request(&self, method: &str, path: &str, body: Value, auth: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path).header("content-type", "application/json");
        if let Some(token) = auth { req = req.header("authorization", token); }
        else if path.starts_with("/admin") { req = req.header("cookie", &self.cookie); }
        let response = self.app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 10_000_000).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn staff(&self, role: &str, slug: &str) -> (String, String) {
        let id = ghost_core::ids::new_id();
        sqlx::query("INSERT INTO users (id, name, slug, email, password, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'fixture', 'active', datetime('now'), datetime('now'))")
            .bind(&id).bind(slug).bind(slug).bind(format!("{slug}@example.test")).execute(&self.state.db).await.unwrap();
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) SELECT ?, id, ? FROM roles WHERE name = ?")
            .bind(ghost_core::ids::new_id()).bind(&id).bind(role).execute(&self.state.db).await.unwrap();
        let session = repos::sessions::create(&self.state.db, &id).await.unwrap();
        (id, format!("ghost-admin-api-session={session}"))
    }

    async fn integration(&self) -> Value {
        let (status, body) = self.request("POST", "/admin/integrations/", json!({"integrations": [{"name": "Headless test", "description": "SDK compatibility"}]}), None).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["integrations"][0].clone()
    }

    fn jwt(integration: &Value, claims: Option<Value>) -> String {
        let key = integration["api_keys"].as_array().unwrap().iter().find(|k| k["type"] == "admin").unwrap();
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = key["id"].as_str().map(str::to_owned);
        let now = chrono::Utc::now().timestamp();
        let claims = claims.unwrap_or(json!({"iat": now, "exp": now + 300, "aud": "/admin/"}));
        let secret = hex::decode(key["secret"].as_str().unwrap()).unwrap();
        format!("Ghost {}", jsonwebtoken::encode(&header, &claims, &jsonwebtoken::EncodingKey::from_secret(&secret)).unwrap())
    }
}

#[tokio::test]
async fn editor_snippets_round_trip_validate_and_delete() {
    let f = Fixture::new().await;
    let lexical = json!({"namespace":"KoenigEditor","nodes":[{"type":"paragraph","version":1,"children":[{"type":"text","version":1,"text":"Reusable content","format":1,"mode":"normal","style":"","detail":0}],"direction":"ltr","format":"","indent":0}]}).to_string();
    let body = json!({"snippets":[{"name":"Reusable greeting", "lexical":lexical, "mobiledoc":"{}"}]});
    assert_eq!(f.request("POST", "/admin/snippets/", body.clone(), Some("Ghost invalid")).await.0, StatusCode::UNAUTHORIZED);
    let (status, added) = f.request("POST", "/admin/snippets/?formats=lexical,mobiledoc", body.clone(), None).await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let id = added["snippets"][0]["id"].as_str().unwrap();
    assert_eq!(added["snippets"][0]["lexical"], lexical);
    assert_eq!(f.request("POST", "/admin/snippets/", body, None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let stored: String = sqlx::query_scalar("SELECT lexical FROM snippets WHERE id = ?").bind(id).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(stored, lexical);
    let path = format!("/admin/snippets/{id}/?formats=lexical");
    let (_, read) = f.request("GET", &path, Value::Null, None).await;
    assert_eq!(read["snippets"][0]["name"], "Reusable greeting");
    assert!(read["snippets"][0].get("mobiledoc").is_none());
    let (_, updated) = f.request("PUT", &path, json!({"snippets":[{"name":"Renamed greeting"}]}), None).await;
    assert_eq!(updated["snippets"][0]["name"], "Renamed greeting");
    assert_eq!(updated["snippets"][0]["lexical"], lexical);
    for (name, content) in [("", lexical.as_str()), ("Invalid", "bad JSON"), ("Empty", "{\"nodes\":[]}")] {
        let (status, error) = f.request("POST", "/admin/snippets/", json!({"snippets":[{"name":name,"lexical":content}]}), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    }
    let (_, list) = f.request("GET", "/admin/snippets?limit=all&formats=lexical,mobiledoc", Value::Null, None).await;
    assert_eq!(list["snippets"].as_array().unwrap().len(), 1);
    assert_eq!(list["meta"]["can_manage"], true);
    assert_eq!(list["meta"]["pagination"]["total"], 1);
    assert_eq!(f.request("DELETE", &path, Value::Null, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(f.request("GET", &path, Value::Null, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.request("DELETE", &path, Value::Null, None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn authors_can_use_snippets_but_only_editors_and_admins_can_manage_them() {
    let f = Fixture::new().await;
    let body = json!({"snippets":[{"name":"Shared card", "lexical":json!({"namespace":"KoenigEditor","nodes":[{"type":"horizontalrule","version":1}]}).to_string()}]});
    let (_, added) = f.request("POST", "/admin/snippets/?formats=lexical", body.clone(), None).await;
    let id = added["snippets"][0]["id"].as_str().unwrap();
    let path = format!("/admin/snippets/{id}/");
    for role in ["Author", "Contributor", "Editor"] {
        sqlx::query("UPDATE roles_users SET role_id = (SELECT id FROM roles WHERE name = ?) WHERE user_id = (SELECT id FROM users WHERE email = 'owner@example.test')")
            .bind(role).execute(&f.state.db).await.unwrap();
        let (status, list) = f.request("GET", "/admin/snippets/?formats=lexical", Value::Null, None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert_eq!(list["meta"]["can_manage"], role == "Editor");
        if role != "Editor" {
            assert_eq!(f.request("POST", "/admin/snippets/", body.clone(), None).await.0, StatusCode::FORBIDDEN);
            assert_eq!(f.request("PUT", &path, body.clone(), None).await.0, StatusCode::FORBIDDEN);
            assert_eq!(f.request("DELETE", &path, Value::Null, None).await.0, StatusCode::FORBIDDEN);
        } else {
            assert_eq!(f.request("DELETE", &path, Value::Null, None).await.0, StatusCode::NO_CONTENT);
        }
    }
}

#[tokio::test]
async fn publication_name_persists_and_reaches_content_settings() {
    let f = Fixture::new().await;
    let (status, body) = f.request("PUT", "/admin/settings/", json!({"settings": [
        {"key": "name", "value": "Daily News"},
        {"key": "title", "value": "Independent news and analysis"}
    ]}), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reloaded = ghost_db::SettingsCache::load(&f.state.db).await.unwrap();
    assert_eq!(reloaded.site_name(), "Daily News");
    let (status, body) = f.request("GET", &format!("/content/settings/?key={}", f.content_key), Value::Null, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["settings"]["name"], "Daily News");
    assert_eq!(body["settings"]["title"], "Independent news and analysis");
}

#[tokio::test]
async fn sdk_content_paths_authentication_and_read_fields() {
    let f = Fixture::new().await;
    for path in ["/content/posts/", "/content/posts", "/content/tags/", "/content/authors/", "/content/settings/"] {
        assert_eq!(f.request("GET", path, Value::Null, None).await.0, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(f.request("GET", &format!("{path}?key={}", f.content_key), Value::Null, None).await.0, StatusCode::OK, "{path}");
    }
    let (_, list) = f.request("GET", &format!("/content/posts/?key={}&limit=all", f.content_key), Value::Null, None).await;
    assert_eq!(list["meta"]["pagination"]["pages"], 1);
    let post = &list["posts"][0];
    assert!(post["url"].as_str().unwrap().starts_with("https://example.test/"));
    assert!(post.get("authors").is_none());
    for path in [format!("/content/posts/{}/", post["id"].as_str().unwrap()), format!("/content/posts/slug/{}/", post["slug"].as_str().unwrap())] {
        let (status, body) = f.request("GET", &format!("{path}?key={}&fields=title", f.content_key), Value::Null, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["posts"][0].as_object().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn integration_crud_creates_both_keys_and_revokes_them() {
    let f = Fixture::new().await;
    let i = f.integration().await;
    assert_eq!(i["api_keys"].as_array().unwrap().len(), 2);
    assert_eq!(i["description"], "SDK compatibility");
    let id = i["id"].as_str().unwrap();
    let path = format!("/admin/integrations/{id}/");
    let (status, edited) = f.request("PUT", &path, json!({"integrations": [{"name": "Updated", "description": null, "icon_image": "https://example.test/icon.png"}]}), None).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["integrations"][0]["name"], "Updated");
    assert_eq!(edited["integrations"][0]["description"], Value::Null);
    let token = Fixture::jwt(&i, None);
    assert_eq!(f.request("GET", "/admin/posts/", Value::Null, Some(&token)).await.0, StatusCode::OK);
    assert_eq!(f.request("POST", "/admin/integrations/", json!({"integrations": [{"name": "Forbidden"}]}), Some(&token)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("DELETE", &path, Value::Null, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(f.request("GET", "/admin/posts/", Value::Null, Some(&token)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn content_api_honors_formats_includes_and_member_previews() {
    let f = Fixture::new().await;
    let (_, body) = f.request("POST", "/admin/posts/", json!({"posts": [{
        "title": "Gated test", "status": "published", "visibility": "members",
        "html": "<p>Public preview</p><!--members-only--><p>Private body</p>",
        "feature_image": "/content/images/test.jpg",
        "tags": [{"name": "Public tag"}, {"name": "#private-tag"}]
    }]}), None).await;
    let id = body["posts"][0]["id"].as_str().unwrap();
    let (_, body) = f.request("GET", &format!("/content/posts/{id}/?key={}&formats=html,plaintext&include=authors,tags", f.content_key), Value::Null, None).await;
    let post = &body["posts"][0];
    assert_eq!(post["access"], false);
    assert!(!post["html"].as_str().unwrap().contains("Private body"));
    assert!(!post["plaintext"].as_str().unwrap().contains("Private body"));
    assert!(post["authors"][0]["url"].as_str().unwrap().starts_with("https://example.test/author/"));
    assert_eq!(post["tags"].as_array().unwrap().len(), 1);
    assert_eq!(post["feature_image"], "https://example.test/content/images/test.jpg");
    assert!(post.get("status").is_none());
}

#[tokio::test]
async fn webhook_ownership_secret_updates_and_url_validation() {
    let f = Fixture::new().await;
    let first = f.integration().await;
    let second = f.integration().await;
    let token = Fixture::jwt(&first, None);
    let token2 = Fixture::jwt(&second, None);
    let (status, body) = f.request("POST", "/admin/webhooks/", json!({"webhooks": [{"event": "post.published", "target_url": "https://example.test/hook", "secret": "old"}]}), Some(&token)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let path = format!("/admin/webhooks/{}/", body["webhooks"][0]["id"].as_str().unwrap());
    assert_eq!(f.request("PUT", &path, json!({"webhooks": [{"secret": "hijack"}]}), Some(&token2)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("DELETE", &path, Value::Null, Some(&token2)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("PUT", &path, json!({"webhooks": [{"target_url": "file:///etc/passwd"}]}), Some(&token)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = f.request("PUT", &path, json!({"webhooks": [{"secret": "new"}]}), Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["webhooks"][0]["secret"], "new");
    let (_, body) = f.request("GET", &format!("/admin/integrations/{}/", first["id"].as_str().unwrap()), Value::Null, None).await;
    assert_eq!(body["integrations"][0]["webhooks"].as_array().unwrap().len(), 1);
    f.request("DELETE", &format!("/admin/integrations/{}/", first["id"].as_str().unwrap()), Value::Null, None).await;
    assert!(repos::webhooks::list_for_integration(&f.state.db, first["id"].as_str().unwrap()).await.unwrap().is_empty());
}

#[tokio::test]
async fn jwt_requires_iat_honors_nbf_and_accepts_audience_arrays() {
    let f = Fixture::new().await;
    let i = f.integration().await;
    let now = chrono::Utc::now().timestamp();
    for claims in [json!({"exp": now + 600, "aud": "/admin/"}),
        json!({"iat": now - 300, "exp": now + 600, "aud": "/admin/"}),
        json!({"iat": now, "exp": now, "aud": "/admin/"}),
        json!({"iat": now, "nbf": now + 300, "aud": "/admin/"})] {
        let token = Fixture::jwt(&i, Some(claims));
        assert_eq!(f.request("GET", "/admin/posts/", Value::Null, Some(&token)).await.0, StatusCode::UNAUTHORIZED);
    }
    let token = Fixture::jwt(&i, Some(json!({"iat": now, "aud": ["/content/", "/admin/"]})));
    assert_eq!(f.request("GET", "/admin/posts/", Value::Null, Some(&token)).await.0, StatusCode::OK);
    let request = Request::builder().uri("/admin/posts/").header("authorization", "Ghost invalid")
        .header("cookie", &f.cookie).body(Body::empty()).unwrap();
    assert_eq!(f.app.clone().oneshot(request).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn content_api_cors_preflights_and_version_headers() {
    let f = Fixture::new().await;
    let request = Request::builder().method("OPTIONS").uri("/content/posts/")
        .header("origin", "https://frontend.example.test")
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "accept-version")
        .body(Body::empty()).unwrap();
    let response = f.app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    let response = f.app.clone().oneshot(Request::builder()
        .uri(format!("/content/posts/?key={}", f.content_key))
        .header("origin", "https://frontend.example.test").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.headers()["content-version"], "v6.73");
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn api_source_formats_dates_tag_edits_and_null_clearing() {
    let f = Fixture::new().await;
    let lexical = r#"{"root":{"type":"root","children":[{"type":"paragraph","children":[{"type":"text","text":"Round trip","format":0}]}]}}"#;
    let (status, body) = f.request("POST", "/admin/posts/?formats=lexical", json!({"posts": [{
        "title": "Source round trip", "lexical": lexical, "status": "published",
        "published_at": "2025-01-02T03:04:05.000Z", "feature_image_caption": "Old caption"
    }]}), None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["posts"][0]["lexical"], lexical);
    assert_eq!(body["posts"][0]["published_at"], "2025-01-02T03:04:05.000Z");
    let id = body["posts"][0]["id"].as_str().unwrap();
    let path = format!("/admin/posts/{id}/?formats=lexical,html,plaintext");
    let (status, body) = f.request("PUT", &path, json!({"posts": [{"tags": [{"name": "Updated tag"}], "feature_image_caption": null}]}), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["posts"][0]["lexical"], lexical);
    assert_eq!(body["posts"][0]["feature_image_caption"], Value::Null);
    assert_eq!(body["posts"][0]["tags"][0]["name"], "Updated tag");
    let (_, body) = f.request("GET", &format!("/admin/posts/{id}/?fields=lexical"), Value::Null, None).await;
    assert_eq!(body["posts"][0]["lexical"], lexical);
    assert_eq!(body["posts"][0].as_object().unwrap().len(), 1);
    for post in [json!({"title": "Invalid", "published_at": "bad-date"}), json!({"title": "Invalid", "status": "scheduled"}), json!({"title": "Invalid", "status": "unknown"})] {
        assert_eq!(f.request("POST", "/admin/posts/", json!({"posts": [post]}), None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn api_publishing_delivers_signed_webhooks_with_previous_values() {
    let f = Fixture::new().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let receiver = Router::new().route("/hook", axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
        let tx = tx.clone();
        async move { tx.send((headers, body)).unwrap(); StatusCode::OK }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/hook", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, receiver).await.unwrap(); });
    let integration = f.integration().await;
    for event in ["post.published", "post.published.edited", "post.unpublished"] {
        repos::webhooks::create(&f.state.db, integration["id"].as_str().unwrap(), event, &target, "test", "test-secret").await.unwrap();
    }
    let token = Fixture::jwt(&integration, None);
    let (_, body) = f.request("POST", "/admin/posts/", json!({"posts": [{"title": "Before", "status": "draft", "html": "<p>Body</p>"}]}), Some(&token)).await;
    let path = format!("/admin/posts/{}/", body["posts"][0]["id"].as_str().unwrap());
    for (edit, event, old_title) in [
        (json!({"status": "published"}), "post.published", None),
        (json!({"title": "After"}), "post.published.edited", Some("Before")),
        (json!({"status": "draft"}), "post.unpublished", None),
    ] {
        let (status, body) = f.request("PUT", &path, json!({"posts": [edit]}), Some(&token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (headers, body) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert!(headers["x-ghost-signature"].to_str().unwrap().starts_with("sha256="));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["event"], event);
        if let Some(title) = old_title { assert_eq!(body["post"]["previous"]["title"], title); }
    }
    task.abort();
}

#[tokio::test]
async fn integration_creation_rolls_back_if_a_key_insert_fails() {
    let f = Fixture::new().await;
    sqlx::query("CREATE TRIGGER fail_admin_key BEFORE INSERT ON api_keys WHEN NEW.type = 'admin' BEGIN SELECT RAISE(ABORT, 'simulated failure'); END").execute(&f.state.db).await.unwrap();
    let result = repos::integrations::create_custom(&f.state.db, "Must roll back", None).await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM integrations WHERE name = 'Must roll back'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(count, 0);
    let orphan_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE integration_id NOT IN (SELECT id FROM integrations)").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(orphan_count, 0);
}

#[tokio::test]
async fn public_authors_and_tags_include_counts_on_reads() {
    let f = Fixture::new().await;
    let (_, body) = f.request("GET", &format!("/content/authors/?key={}&include=count.posts&order=name%20desc", f.content_key), Value::Null, None).await;
    let author = &body["authors"][0];
    assert!(author["count"]["posts"].as_i64().unwrap() > 0);
    let (_, body) = f.request("GET", &format!("/content/authors/{}/?key={}&include=count.posts&fields=name,count", author["id"].as_str().unwrap(), f.content_key), Value::Null, None).await;
    assert!(body["authors"][0]["count"]["posts"].as_i64().unwrap() > 0);
    let (_, body) = f.request("POST", "/admin/posts/", json!({"posts": [{"title": "Tag count", "status": "published", "html": "body", "tags": [{"name": "Count this"}]}]}), None).await;
    let tag = &body["posts"][0]["tags"][0];
    let (_, body) = f.request("GET", &format!("/content/tags/{}/?key={}&include=count.posts&fields=name,count", tag["id"].as_str().unwrap(), f.content_key), Value::Null, None).await;
    assert_eq!(body["tags"][0]["count"]["posts"], 1);
}


#[tokio::test]
async fn free_signup_defaults_are_visible_and_membership_controls_update_visibility() {
    let f = Fixture::new().await;
    let free: (String, String) = sqlx::query_as("SELECT id, visibility FROM products WHERE type = 'free'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(free.1, "public");
    let (_, body) = f.request("GET", &format!("/content/tiers/?key={}", f.content_key), Value::Null, None).await;
    let tier = body["tiers"].as_array().unwrap().iter().find(|tier| tier["type"] == "free").unwrap();
    assert_eq!(tier["visibility"], "public");
    repos::tiers::set_free_signup_visibility(&f.state.db, false).await.unwrap();
    let hidden: String = sqlx::query_scalar("SELECT visibility FROM products WHERE id = ?").bind(&free.0).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(hidden, "none");
    repos::tiers::set_free_signup_visibility(&f.state.db, true).await.unwrap();
    let visible: String = sqlx::query_scalar("SELECT visibility FROM products WHERE id = ?").bind(&free.0).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(visible, "public");
}

#[tokio::test]
async fn stripe_sync_repairs_missing_tier_ids_and_reuses_unchanged_prices() {
    let f = Fixture::new().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1").fetch_one(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO stripe_products (id, product_id, stripe_product_id, created_at, updated_at) VALUES ('product-map', ?, 'prod_fixture', datetime('now'), datetime('now'))").bind(&tier).execute(&f.state.db).await.unwrap();
    for (id, amount, interval) in [("price_month_fixture", 500, "month"), ("price_year_fixture", 5000, "year")] {
        sqlx::query("INSERT INTO stripe_prices (id, stripe_price_id, stripe_product_id, active, amount, currency, interval, type, created_at, updated_at) VALUES (?, ?, 'prod_fixture', 1, ?, 'usd', ?, 'recurring', datetime('now'), datetime('now'))")
            .bind(id).bind(id).bind(amount).bind(interval).execute(&f.state.db).await.unwrap();
    }
    let cfg = ghost_stripe::StripeConfig {secret_key: "sk_test_never_sent".into(), publishable_key: String::new(), webhook_secret: String::new(), checkout_design: None};
    for _ in 0..2 {
        ghost_api::stripe_tiers::sync_tier(&f.state, &cfg, &tier, "Fixture", "USD", Some(500), Some(5000)).await.unwrap();
    }
    let ids: (Option<String>, Option<String>) = sqlx::query_as("SELECT monthly_price_id, yearly_price_id FROM products WHERE id = ?").bind(&tier).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(ids.0.as_deref(), Some("price_month_fixture"));
    assert_eq!(ids.1.as_deref(), Some("price_year_fixture"));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stripe_prices").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn checkout_configuration_errors_use_portal_messages_and_persist_owner_diagnostics() {
    let f = Fixture::new().await;
    let (status, body) = f.request("POST", "/members/api/create-stripe-checkout-session/", json!({"tierId": "missing", "cadence": "month"}), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["errors"][0]["message"], "This site is not accepting payments at the moment.");
    f.state.settings.set(&f.state.db, "stripe_checkout_setup_error", &json!("Complete the Stripe business name.")).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'stripe_checkout_setup_error'").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(stored, "Complete the Stripe business name.");
}

#[tokio::test]
async fn api_staff_permissions_and_post_ownership_are_enforced() {
    let mut f = Fixture::new().await;
    let owner_cookie = f.cookie.clone();
    let owner = repos::users::find_owner(&f.state.db).await.unwrap().unwrap();
    let (status, post) = f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Owner draft","html":"<p>owner secret</p>"}]}), None).await;
    assert_eq!(status, StatusCode::CREATED, "{post}");
    let owner_post = post["posts"][0]["id"].as_str().unwrap().to_string();
    let (contributor_id, contributor_cookie) = f.staff("Contributor", "contributor").await;
    f.cookie = contributor_cookie.clone();
    assert_eq!(f.request("GET", &format!("/admin/posts/{owner_post}/"), Value::Null, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("PUT", &format!("/admin/posts/{owner_post}/"), json!({"posts":[{"title":"hijacked"}]}), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("DELETE", &format!("/admin/posts/{owner_post}/"), Value::Null, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Cannot publish","status":"published"}]}), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Cannot impersonate","authors":[{"id":owner.id}]}]}), None).await.0, StatusCode::FORBIDDEN);
    let (status, mine) = f.request("POST", "/admin/posts/", json!({"posts":[{"title":"My draft"}]}), None).await;
    assert_eq!(status, StatusCode::CREATED, "{mine}");
    let mine = mine["posts"][0]["id"].as_str().unwrap().to_string();
    let (_, list) = f.request("GET", "/admin/posts/?limit=all", Value::Null, None).await;
    assert_eq!(list["posts"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list["posts"][0]["id"], mine);
    assert_eq!(f.request("PUT", &format!("/admin/posts/{mine}/"), json!({"posts":[{"status":"published"}]}), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("PUT", &format!("/admin/posts/{mine}/"), json!({"posts":[{"title":"Updated draft"}]}), None).await.0, StatusCode::OK);
    assert_eq!(f.request("PUT", "/admin/settings/", json!({"settings":[{"key":"title","value":"forbidden"}]}), None).await.0, StatusCode::FORBIDDEN);
    for path in ["/admin/newsletters/", "/admin/automations/", "/admin/comments/", "/admin/integrations/"] {
        assert_eq!(f.request("GET", path, Value::Null, None).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    let (_, settings) = f.request("GET", "/admin/settings/", Value::Null, None).await;
    assert!(settings["settings"].as_array().unwrap().iter().all(|item| item["key"] != "stripe_secret_key" && item["key"] != "theme_session_secret"));
    assert_eq!(f.request("PUT", &format!("/admin/users/{}/", owner.id), json!({"users":[{"email":"takeover@example.test"}]}), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("PUT", "/admin/users/me/", json!({"users":[{"roles":[{"name":"Owner"}]}]}), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.request("PUT", "/admin/users/me/", json!({"users":[{"name":"My own profile"}]}), None).await.0, StatusCode::OK);
    let (_, author_cookie) = f.staff("Author", "author").await;
    f.cookie = author_cookie;
    assert_eq!(f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Author publication","status":"published"}]}), None).await.0, StatusCode::CREATED);
    assert_eq!(f.request("PUT", &format!("/admin/posts/{owner_post}/"), json!({"posts":[{"title":"Not mine"}]}), None).await.0, StatusCode::FORBIDDEN);
    let (_, editor_cookie) = f.staff("Editor", "editor").await;
    f.cookie = editor_cookie;
    assert_eq!(f.request("PUT", &format!("/admin/posts/{owner_post}/"), json!({"posts":[{"title":"Editor revision"}]}), None).await.0, StatusCode::OK);
    assert_eq!(f.request("POST", "/admin/tags/", json!({"tags":[{"name":"Editor tag"}]}), None).await.0, StatusCode::CREATED);
    assert_eq!(f.request("PUT", "/admin/settings/", json!({"settings":[{"key":"title","value":"forbidden"}]}), None).await.0, StatusCode::FORBIDDEN);
    sqlx::query("UPDATE users SET status = 'locked' WHERE id = ?").bind(contributor_id).execute(&f.state.db).await.unwrap();
    f.cookie = contributor_cookie;
    assert_eq!(f.request("GET", "/admin/posts/", Value::Null, None).await.0, StatusCode::UNAUTHORIZED);
    f.cookie = owner_cookie;
    assert_eq!(f.request("PUT", "/admin/settings/", json!({"settings":[{"key":"title","value":"Owner update"}]}), None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn tier_assignments_round_trip_and_invalid_updates_are_atomic() {
    let f = Fixture::new().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1").fetch_one(&f.state.db).await.unwrap();
    let (status, added) = f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Tier exclusive","visibility":"tiers","status":"published","tiers":[{"id":tier}],"html":"<p>EXCLUSIVE CONTENT</p>"}]}), None).await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let id = added["posts"][0]["id"].as_str().unwrap();
    assert_eq!(added["posts"][0]["tiers"][0]["id"], tier);
    let stored = repos::posts::find_by_id(&f.state.db, id).await.unwrap().unwrap();
    assert_eq!(stored.tier_ids, vec![tier.clone()]);
    let path = format!("/admin/posts/{id}/");
    let (status, error) = f.request("PUT", &path, json!({"posts":[{"title":"Must not persist","tiers":[{"id":"missing-tier"}]}]}), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    let stored = repos::posts::find_by_id(&f.state.db, id).await.unwrap().unwrap();
    assert_eq!(stored.title.as_deref(), Some("Tier exclusive"));
    assert_eq!(stored.tier_ids, vec![tier]);
    assert_eq!(f.request("PUT", &path, json!({"posts":[{"tiers":[]}]}), None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, public) = f.request("GET", &format!("/content/posts/{id}/?key={}&formats=html,plaintext", f.content_key), Value::Null, None).await;
    assert!(!public.to_string().contains("EXCLUSIVE CONTENT"), "{public}");
    let (status, draft) = f.request("POST", "/admin/posts/", json!({"posts":[{"title":"Choose tier later","visibility":"tiers","status":"draft"}]}), None).await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    let draft_id = draft["posts"][0]["id"].as_str().unwrap();
    assert_eq!(f.request("PUT", &format!("/admin/posts/{draft_id}/"), json!({"posts":[{"status":"published"}]}), None).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn publishing_copies_and_revisions_preserve_content_and_assignments() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Original revision","html":"<p>First body</p>","status":"draft","feature_image_caption":"Original caption"}]}),None).await;
    let id=created["posts"][0]["id"].as_str().unwrap();
    let path=format!("/admin/posts/{id}/");
    assert_eq!(f.request("PUT",&path,json!({"posts":[{"title":"Changed","html":"<p>Changed body</p>"}]}),None).await.0,StatusCode::OK);
    let (status,revisions)=f.request("GET",&format!("{path}revisions/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{revisions}");
    let revision=revisions["revisions"][0]["id"].as_str().unwrap();
    assert_eq!(revisions["revisions"][0]["post"]["title"],"Original revision");
    let (status,restored)=f.request("PUT",&format!("{path}revisions/{revision}/"),json!({}),None).await;
    assert_eq!(status,StatusCode::OK,"{restored}");assert_eq!(restored["posts"][0]["title"],"Original revision");
    let (status,copied)=f.request("POST",&format!("{path}copy/"),json!({}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{copied}");assert_ne!(copied["posts"][0]["id"],id);
    assert_eq!(copied["posts"][0]["title"],"Original revision (Copy)");assert_eq!(copied["posts"][0]["status"],"draft");
    assert_eq!(copied["posts"][0]["html"],"<p>First body</p>");
    let (status,_)=f.request("POST","/admin/views/",json!({"resource":"posts","name":"Drafts","filter":"status:draft"}),None).await;
    assert_eq!(status,StatusCode::CREATED);
    let (_,views)=f.request("GET","/admin/views/?resource=posts",Value::Null,None).await;
    assert_eq!(views["views"][0]["name"],"Drafts");
    let (_,cookie)=f.staff("Contributor","revision-contributor").await;
    let req=Request::builder().method("GET").uri(format!("/admin/posts/{id}/revisions/")).header("cookie",cookie);
    let response=f.app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn members_filters_bulk_updates_and_impersonation_are_persistent() {
    let f=Fixture::new().await;
    let (status,created)=f.request("POST","/admin/members/",json!({"members":[{"email":"reader@example.test","name":"Reader"}]}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{created}");let id=created["members"][0]["id"].as_str().unwrap();
    let (_,empty)=f.request("GET","/admin/members/?filter=status:paid",Value::Null,None).await;assert_eq!(empty["members"].as_array().unwrap().len(),0);
    let (_,matching)=f.request("GET","/admin/members/?filter=status:free%2Bemail:reader@example.test",Value::Null,None).await;assert_eq!(matching["members"].as_array().unwrap().len(),1);
    assert_eq!(f.request("GET","/admin/members/?filter=unknown:bad",Value::Null,None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    sqlx::query("INSERT INTO labels (id,name,slug,created_at) VALUES ('vip','VIP','vip',datetime('now'))").execute(&f.state.db).await.unwrap();
    assert_eq!(f.request("POST","/admin/members/bulk/",json!({"ids":[id],"action":"label","label_id":"vip"}),None).await.0,StatusCode::OK);
    let (_,matching)=f.request("GET","/admin/members/?filter=labels:vip",Value::Null,None).await;assert_eq!(matching["members"][0]["id"],id);
    let (status,link)=f.request("GET",&format!("/admin/members/{id}/signin_urls/"),Value::Null,None).await;assert_eq!(status,StatusCode::OK,"{link}");
    let url=reqwest::Url::parse(link["member_signin_urls"][0]["url"].as_str().unwrap()).unwrap();
    let path=format!("{}?{}",url.path(),url.query().unwrap());
    let response=f.app.clone().oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::FOUND);assert!(response.headers().contains_key("set-cookie"));
    let response=f.app.clone().oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap()).await.unwrap();
    assert!(!response.headers().contains_key("set-cookie"));
}

#[tokio::test]
async fn newsletter_links_can_be_edited_and_suppression_events_require_valid_signatures() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Links test","html":"<p>Content</p>"}]}),None).await;
    let id=created["posts"][0]["id"].as_str().unwrap();
    let now=ghost_core::dates::now_db();
    sqlx::query("INSERT INTO newsletter_links (id,post_id,from_url,to_url,created_at,updated_at) VALUES ('link',?,'https://old.example','https://old.example',?,?)")
        .bind(id).bind(&now).bind(&now).execute(&f.state.db).await.unwrap();
    assert_eq!(f.request("PUT","/admin/links/link/",json!({"links":[{"to":"javascript:alert(1)"}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (status,links)=f.request("PUT","/admin/links/link/",json!({"links":[{"to":"https://new.example/article"}]}),None).await;
    assert_eq!(status,StatusCode::OK,"{links}");assert_eq!(links["links"][0]["to"],"https://new.example/article");
    let events=ghost_api::newsletter_events::public_router().with_state(f.state.clone());
    let response=events.oneshot(Request::builder().method("POST").uri("/members/webhooks/mailgun/").header("content-type","application/json").body(Body::from(json!({"signature":{"timestamp":"0","token":"bad","signature":"bad"},"event-data":{"id":"event","event":"complained","recipient":"victim@example.test"}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    let total:i64=sqlx::query_scalar("SELECT COUNT(*) FROM email_suppressions").fetch_one(&f.state.db).await.unwrap();assert_eq!(total,0);
}

#[tokio::test]
async fn native_social_identity_notes_and_gated_exports_are_persistent() {
    let f=Fixture::new().await;
    let public=ghost_api::federation::public_router().with_state(f.state.clone());
    let response=public.clone().oneshot(Request::builder().uri("/.ghost/activitypub/actor").body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::NOT_FOUND);
    let (status,settings)=f.request("PUT","/admin/social/settings/",json!({"enabled":true}),None).await;
    assert_eq!(status,StatusCode::OK,"{settings}");assert_eq!(settings["social"]["enabled"],true);
    let response=public.clone().oneshot(Request::builder().uri("/.well-known/webfinger?resource=acct:publication@example.test").body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
    let response=public.clone().oneshot(Request::builder().uri("/.ghost/activitypub/actor").body(Body::empty()).unwrap()).await.unwrap();let actor:Value=serde_json::from_slice(&to_bytes(response.into_body(),1_000_000).await.unwrap()).unwrap();assert!(actor["publicKey"]["publicKeyPem"].as_str().unwrap().contains("BEGIN PUBLIC KEY"));assert!(actor.get("private_pem").is_none());
    let (status,created)=f.request("POST","/admin/social/notes/",json!({"text":"A native **publication** note"}),None).await;assert_eq!(status,StatusCode::CREATED,"{created}");
    let id=created["notes"][0]["id"].as_str().unwrap().rsplit('/').next().unwrap();
    let response=public.clone().oneshot(Request::builder().uri(format!("/.ghost/activitypub/objects/{id}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let object:Value=serde_json::from_slice(&to_bytes(response.into_body(),1_000_000).await.unwrap()).unwrap();assert_eq!(object["type"],"Note");assert!(object["content"].as_str().unwrap().contains("<strong>publication</strong>"));
    let (_,reader)=f.request("GET","/admin/social/reader/",Value::Null,None).await;assert!(reader["notes"][0]["text"].as_str().unwrap().contains("publication"));
    assert_eq!(f.request("DELETE",&format!("/admin/social/notes/{id}/"),Value::Null,None).await.0,StatusCode::NO_CONTENT);
    let response=public.clone().oneshot(Request::builder().uri(format!("/.ghost/activitypub/objects/{id}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::GONE);
    let (_,post)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Private federation body","html":"<p>PRIVATE-FEDERATION-SECRET</p>","visibility":"paid","status":"published"}]}),None).await;
    let post_id=post["posts"][0]["id"].as_str().unwrap();
    let response=public.clone().oneshot(Request::builder().uri(format!("/.ghost/activitypub/objects/{post_id}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let body=to_bytes(response.into_body(),1_000_000).await.unwrap();assert!(!String::from_utf8_lossy(&body).contains("PRIVATE-FEDERATION-SECRET"));
    let (_,cookie)=f.staff("Author","social-author").await;
    let response=f.app.clone().oneshot(Request::builder().method("POST").uri("/admin/social/notes/").header("cookie",cookie).header("content-type","application/json").body(Body::from(json!({"text":"Unauthorized note"}).to_string())).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::FORBIDDEN);
    let response=public.clone().oneshot(Request::builder().method("POST").uri("/.ghost/activitypub/inbox").header("content-type","application/json").body(Body::from(json!({"id":"https://remote.example.test/follow","actor":"https://remote.example.test/actor","type":"Follow","object":actor["id"]}).to_string())).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    let followers:i64=sqlx::query_scalar("SELECT COUNT(*) FROM federation_followers").fetch_one(&f.state.db).await.unwrap();assert_eq!(followers,0);
}

#[tokio::test]
async fn staff_password_reset_is_expiring_single_use_and_revokes_sessions() {
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    let owner=repos::users::find_owner(&f.state.db).await.unwrap().unwrap();let token="a".repeat(64);let token_hash=hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("INSERT INTO staff_password_resets (token_hash,user_id,expires_at,created_at) VALUES (?,?,datetime('now','+10 minutes'),datetime('now'))").bind(token_hash).bind(&owner.id).execute(&f.state.db).await.unwrap();
    let request=json!({"password_reset":[{"token":token,"newPassword":"A secure fixture password"}]});
    assert_eq!(f.request("PUT","/admin/authentication/password_reset/",request.clone(),None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.request("PUT","/admin/authentication/password_reset/",request,None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(f.request("GET","/admin/posts/",Value::Null,None).await.0,StatusCode::UNAUTHORIZED);
    let owner=repos::users::find_owner(&f.state.db).await.unwrap().unwrap();assert!(bcrypt::verify("A secure fixture password",owner.password.as_deref().unwrap()).unwrap());
}

#[tokio::test]
async fn new_management_resources_and_audiences_validate_before_writing() {
    let f=Fixture::new().await;
    let (status,label)=f.request("POST","/admin/labels/",json!({"labels":[{"name":"VIP readers"}]}),None).await;assert_eq!(status,StatusCode::CREATED,"{label}");assert_eq!(label["labels"][0]["slug"],"vip-readers");
    let (status,tier)=f.request("POST","/admin/tiers/",json!({"tiers":[{"name":"Premium API tier","monthly_price":500,"yearly_price":5000}]}),None).await;assert_eq!(status,StatusCode::CREATED,"{tier}");let tier_id=tier["tiers"][0]["id"].as_str().unwrap();
    assert_eq!(f.request("PUT",&format!("/admin/tiers/{tier_id}/"),json!({"tiers":[{"monthly_price":-10}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (status,offer)=f.request("POST","/admin/offers/",json!({"offers":[{"name":"Half price","code":"half-price","tier":{"id":tier_id},"amount":50}]}),None).await;assert_eq!(status,StatusCode::CREATED,"{offer}");
    let (_,post)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Audience test"}]}),None).await;let id=post["posts"][0]["id"].as_str().unwrap();let path=format!("/admin/posts/{id}/audience/");
    assert_eq!(f.request("PUT",&path,json!({"filter":"unknown:unsafe","include_preview":true}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (status,audience)=f.request("PUT",&path,json!({"filter":"labels:vip-readers+status:free","include_preview":true}),None).await;assert_eq!(status,StatusCode::OK,"{audience}");
    let (_,read)=f.request("GET",&path,Value::Null,None).await;assert_eq!(read["audience"]["filter"],"labels:vip-readers+status:free");assert_eq!(read["audience"]["include_preview"],true);
}

#[tokio::test]
async fn machine_payment_challenges_are_article_bound_and_never_grant_membership() {
    use axum::http::{HeaderMap,Uri};
    let f=Fixture::new().await;
    let owner=repos::users::find_owner(&f.state.db).await.unwrap().unwrap();
    let post=repos::posts::insert(&f.state.db,&repos::posts::NewPost{title:"Paid machine article".into(),status:"published".into(),visibility:"paid".into(),html:Some("<p>PRIVATE-MACHINE-CONTENT</p>".into()),..Default::default()},&owner.id).await.unwrap();
    let mut state=f.state.clone();state.settings=Arc::new(state.settings.with_overrides([
        ("stripe_secret_key".into(),json!("sk_test_fixture")),("machine_payments_enabled".into(),json!(true)),
        ("machine_payments_amount".into(),json!(100)),("machine_payments_currency".into(),json!("USD")),
        ("machine_payments_secret".into(),json!("a sufficiently long fixture secret for signed challenges")),
        ("machine_payments_stripe_profile_id".into(),json!("fixture_profile")),("llms_enabled".into(),json!(true)),
        ("labs".into(),json!({"machinePayments":true}))
    ]));
    let uri:Uri="/paid-machine-article.md".parse().unwrap();
    let response=ghost_api::machine_payments::serve(&state,&post,&uri,&HeaderMap::new()).await.unwrap();
    assert_eq!(response.status(),StatusCode::PAYMENT_REQUIRED);assert_eq!(response.headers()["cache-control"],"private, no-store");assert!(!response.headers().contains_key("set-cookie"));
    let challenge=mpp::PaymentChallenge::from_header(response.headers()["www-authenticate"].to_str().unwrap()).unwrap();
    let body=to_bytes(response.into_body(),1_000_000).await.unwrap();assert!(!String::from_utf8_lossy(&body).contains("PRIVATE-MACHINE-CONTENT"));
    let credential=mpp::PaymentCredential::new(challenge.to_echo(),json!({"spt":"spt_never_sent_to_stripe","externalId":post.id}));
    let mut headers=HeaderMap::new();headers.insert("authorization",mpp::format_authorization(&credential).unwrap().parse().unwrap());
    // A validly signed challenge for another URL must fail before contacting Stripe.
    let other:Uri="/another-article.md".parse().unwrap();
    let response=tokio::time::timeout(std::time::Duration::from_secs(2),ghost_api::machine_payments::serve(&state,&post,&other,&headers)).await.unwrap().unwrap();assert_eq!(response.status(),StatusCode::PAYMENT_REQUIRED);
    let attempts:i64=sqlx::query_scalar("SELECT COUNT(*) FROM machine_payment_attempts").fetch_one(&state.db).await.unwrap();assert_eq!(attempts,0);
    let members:i64=sqlx::query_scalar("SELECT COUNT(*) FROM members").fetch_one(&state.db).await.unwrap();assert_eq!(members,0);
    let members_post=ghost_core::models::Post{visibility:"members".into(),..post.clone()};
    assert!(ghost_api::machine_payments::serve(&state,&members_post,&uri,&headers).await.is_none());
}

#[tokio::test]
async fn subscription_sync_preserves_other_active_subscriptions_and_nonrecurring_grants() {
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('subscriber','subscriber-uuid','subscriber-transient','subscriber@example.test','paid',datetime('now'))").execute(&f.state.db).await.unwrap();
    for(id,price)in [("tier-a","price_a"),("tier-b","price_b"),("gift-tier","price_gift")]{sqlx::query("INSERT INTO products (id,name,slug,monthly_price_id,created_at) VALUES (?,?,?,?,datetime('now'))").bind(id).bind(id).bind(id).bind(price).execute(&f.state.db).await.unwrap();}
    sqlx::query("INSERT INTO members_stripe_customers (id,member_id,customer_id,created_at) VALUES ('customer','subscriber','cus_fixture',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO members_products (id,member_id,product_id,expiry_at) VALUES ('gift-grant','subscriber','gift-tier',datetime('now','+10 days'))").execute(&f.state.db).await.unwrap();
    let sub=|id:&str,price:&str,status:&str|json!({"id":id,"customer":"cus_fixture","status":status,"current_period_end":chrono::Utc::now().timestamp()+86400,"items":{"data":[{"price":{"id":price,"currency":"usd","unit_amount":500,"recurring":{"interval":"month"}}}]}});
    repos::subscriptions::sync(&f.state.db,&sub("sub_a","price_a","active")).await.unwrap();
    repos::subscriptions::sync(&f.state.db,&sub("sub_b","price_b","active")).await.unwrap();
    repos::subscriptions::sync(&f.state.db,&sub("sub_a","price_a","canceled")).await.unwrap();
    let tiers=repos::tiers::member_tier_ids(&f.state.db,"subscriber").await.unwrap();assert!(tiers.contains(&"tier-b".into()));assert!(tiers.contains(&"gift-tier".into()));assert!(!tiers.contains(&"tier-a".into()));
    let status:String=sqlx::query_scalar("SELECT status FROM members WHERE id='subscriber'").fetch_one(&f.state.db).await.unwrap();assert_eq!(status,"paid");
}

#[tokio::test]
async fn checkout_design_validates_and_saved_view_updates_keep_their_id() {
    let f=Fixture::new().await;
    let path="/admin/stripe/checkout/config/";
    let (status,_)=f.request("PUT",path,json!({"design":{"customize":true,"button_color":"url(javascript:bad)","background_color":"#ffffff","font_family":"inter","border_style":"rounded"}}),None).await;
    assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY);
    let design=json!({"customize":true,"button_color":"#1144aa","background_color":"#ffffff","font_family":"inter","border_style":"pill"});
    let (status,value)=f.request("PUT",path,json!({"design":design}),None).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["checkout_config"][0]["design"],design);
    assert_eq!(f.state.settings.get("stripe_checkout_config").unwrap()["design"],design);
    let (_,cookie)=f.staff("Author","design-author").await;
    let response=f.app.clone().oneshot(Request::builder().uri(path).header("cookie",cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::FORBIDDEN);
    let (_,first)=f.request("POST","/admin/views/",json!({"name":"My drafts","resource":"posts","filter":"status:draft"}),None).await;
    let (status,second)=f.request("POST","/admin/views/",json!({"name":"My drafts","resource":"posts","filter":"status:published"}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{second}");assert_eq!(first["views"][0]["id"],second["views"][0]["id"]);
    let id=second["views"][0]["id"].as_str().unwrap();
    assert_eq!(f.request("DELETE",&format!("/admin/views/{id}/"),Value::Null,None).await.0,StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn automation_designs_are_immutable_and_body_revisions_retain_the_design() {
    use ghost_core::automations::{Action,ActionKind,Status};
    let f=Fixture::new().await;
    let actions=[Action{id:"design-action".into(),kind:ActionKind::SendEmail{subject:"Welcome".into(),lexical:"{}".into()}}];
    let id=repos::automations::create(&f.state.db,&repos::automations::AutomationInput{name:"Design regression",description:"",status:Status::Inactive,trigger_tier_scope:None,trigger_tier_ids:&[],actions:&actions,edges:&[]}).await.unwrap();
    let path=format!("/admin/automations/{id}/actions/design-action/design/");
    let (status,first)=f.request("PUT",&path,json!({"settings":{"button_color":"#1144aa","body_font_category":"serif"}}),None).await;
    assert_eq!(status,StatusCode::OK,"{first}");let original=first["design"]["id"].as_str().unwrap();
    let (status,second)=f.request("PUT",&path,json!({"settings":{"button_color":"#cc4400"}}),None).await;
    assert_eq!(status,StatusCode::OK,"{second}");assert_ne!(original,second["design"]["id"].as_str().unwrap());
    let (_,saved)=f.request("GET",&format!("/admin/email-design-settings/{original}/"),Value::Null,None).await;
    assert_eq!(saved["email_design_settings"][0]["settings"]["button_color"],"#1144aa");
    let actions=[Action{id:"design-action".into(),kind:ActionKind::SendEmail{subject:"Updated welcome".into(),lexical:"{}".into()}}];
    repos::automations::update(&f.state.db,&id,&repos::automations::AutomationInput{name:"Design regression",description:"",status:Status::Inactive,trigger_tier_scope:None,trigger_tier_ids:&[],actions:&actions,edges:&[]}).await.unwrap();
    let latest:String=sqlx::query_scalar("SELECT email_design_setting_id FROM automation_action_revisions WHERE action_id='design-action' ORDER BY created_at DESC LIMIT 1").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(latest,second["design"]["id"].as_str().unwrap());
    let (_,read)=f.request("GET",&path,Value::Null,None).await;
    assert_eq!(read["design"]["settings"]["body_font_category"],"serif");
}

#[tokio::test]
async fn signed_mailgun_events_are_idempotent_after_suppression_is_removed() {
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    let mut state=f.state.clone();state.settings=Arc::new(state.settings.with_overrides([("mailgun_webhook_signing_key".into(),json!("fixture-signing-key"))]));
    let app=ghost_api::newsletter_events::public_router().with_state(state.clone());
    let timestamp=chrono::Utc::now().timestamp().to_string();let token="fixture-token";
    // Independent RFC 2104 construction: no dependency on the webhook's signer.
    let mut key=[0u8;64];key[..19].copy_from_slice(b"fixture-signing-key");
    let mut inner=Sha256::new();inner.update(key.map(|b|b^0x36));inner.update(format!("{timestamp}{token}"));
    let mut outer=Sha256::new();outer.update(key.map(|b|b^0x5c));outer.update(inner.finalize());let signature=hex::encode(outer.finalize());
    let event=json!({"signature":{"timestamp":timestamp,"token":token,"signature":signature},"event-data":{"id":"fixture-complaint","event":"complained","recipient":"reader@example.test"}});
    for replay in [false,true] {
        let response=app.clone().oneshot(Request::builder().method("POST").uri("/members/webhooks/mailgun/").header("content-type","application/json").body(Body::from(event.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(ghost_api::newsletter_events::suppression(&state,"reader@example.test").await["suppressed"],!replay);
        ghost_api::newsletter_events::remove_suppression(&state,"reader@example.test").await.unwrap();
    }
}

#[tokio::test]
async fn stripe_retries_events_when_subscription_persistence_fails() {
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('retry-member','retry-uuid','retry-transient','retry@example.test','free',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO members_stripe_customers (id,member_id,customer_id,created_at) VALUES ('retry-customer','retry-member','cus_retry',datetime('now'))").execute(&f.state.db).await.unwrap();
    // Force a write failure after ownership resolution, without any provider calls.
    sqlx::query("CREATE TRIGGER fail_subscription_write BEFORE INSERT ON subscriptions BEGIN SELECT RAISE(ABORT,'fixture write failure'); END").execute(&f.state.db).await.unwrap();
    let mut state=f.state.clone();state.settings=Arc::new(state.settings.with_overrides([("stripe_secret_key".into(),json!("sk_test_fixture")),("members_stripe_webhook_secret".into(),json!("whsec_fixture"))]));
    let app=ghost_api::members::members_router().with_state(state);
    let event=json!({"type":"customer.subscription.updated","data":{"object":{"id":"sub_retry","customer":"cus_retry","status":"active"}}}).to_string();
    let now=chrono::Utc::now().timestamp();let mut key=[0u8;64];key[..13].copy_from_slice(b"whsec_fixture");
    let mut inner=Sha256::new();inner.update(key.map(|b|b^0x36));inner.update(format!("{now}.{event}"));
    let mut outer=Sha256::new();outer.update(key.map(|b|b^0x5c));outer.update(inner.finalize());let signature=hex::encode(outer.finalize());
    let response=app.oneshot(Request::builder().method("POST").uri("/members/webhooks/stripe/").header("stripe-signature",format!("t={now},v1={signature}")).body(Body::from(event)).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::SERVICE_UNAVAILABLE);
    let status:String=sqlx::query_scalar("SELECT status FROM members WHERE id='retry-member'").fetch_one(&f.state.db).await.unwrap();assert_eq!(status,"free");
}
