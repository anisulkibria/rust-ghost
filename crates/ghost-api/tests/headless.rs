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
        let app = Router::new().nest("/admin", ghost_api::admin::admin_router())
            .nest("/content", ghost_api::content::content_router()).with_state(state.clone());
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
